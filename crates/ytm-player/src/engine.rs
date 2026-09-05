//! The playback engine. Owns the audio device, the queue and the decoder, and
//! communicates with the rest of the app only by messages: commands in, an
//! immutable status snapshot out. Nothing else shares its state.

use anyhow::Result;
use arc_swap::ArcSwap;
use rodio::{DeviceSinkBuilder, Player};
use std::sync::mpsc::{self, Receiver, Sender};

/// A decoder opened ahead of time, with the queue index it belongs to.
type ArmResult = Result<(usize, FfmpegPcm, Progress), String>;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ytm_core::{PlayState, PlayerStatus, RepeatMode, Track};

use crate::pcm::{self, FfmpegPcm, Filters, Progress, Tap, TapSink};
use crate::resolver::ResolverCache;

/// Open a named output device, or the default. A configured device that has
/// been unplugged falls back rather than refusing to start.
fn open_device(name: &str) -> Result<rodio::MixerDeviceSink> {
    if name.is_empty() || name == "default" {
        return Ok(DeviceSinkBuilder::open_default_sink()?);
    }
    use rodio::cpal::traits::HostTrait;
    let host = rodio::cpal::default_host();
    if let Ok(mut devices) = host.output_devices() {
        if let Some(d) = devices.find(|d| crate::device_name(d).as_deref() == Some(name)) {
            if let Ok(sink) = DeviceSinkBuilder::from_device(d).and_then(|b| b.open_stream()) {
                return Ok(sink);
            }
        }
    }
    tracing::warn!("audio device '{name}' not available; using the default");
    Ok(DeviceSinkBuilder::open_default_sink()?)
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Replace the queue and start at `index`.
    PlayQueue {
        tracks: Vec<Track>,
        index: usize,
    },
    Enqueue(Track),
    /// Append an autoplay continuation, and resume if the queue had run dry.
    AppendRadio(Vec<Track>),
    /// Reinstate a saved queue, paused at `position`. Restoring a session
    /// should put things back, not start making noise.
    RestoreQueue {
        tracks: Vec<Track>,
        index: usize,
        position: f64,
    },
    PlayNext(Track),
    JumpTo(usize),
    RemoveAt(usize),
    TogglePause,
    Next,
    Prev,
    SeekRelative(f64),
    SetVolume(f32),
    CycleRepeat,
    ToggleShuffle,
    Stop,
    /// Overlap between tracks, in seconds. Zero restores gapless handover.
    SetCrossfade(f32),
    /// Playback speed, 0.5 to 2.0, pitch preserved.
    SetSpeed(f32),
    ToggleNormalize,
    Shutdown,
}

#[derive(Clone)]
pub struct PlayerHandle {
    tx: Sender<Command>,
    status: Arc<ArcSwap<PlayerStatus>>,
}

impl PlayerHandle {
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
    pub fn status(&self) -> Arc<PlayerStatus> {
        self.status.load_full()
    }
}

/// Spawn the engine on its own thread. Returns the handle and the analyser tap.
pub fn spawn(resolver: Arc<ResolverCache>) -> Result<(PlayerHandle, Tap)> {
    spawn_on_device(resolver, "default")
}

/// Spawn the engine on a named output device, falling back to the default when
/// the named one is gone - a device disappearing should not stop playback from
/// starting (FR-P9).
pub fn spawn_on_device(resolver: Arc<ResolverCache>, device: &str) -> Result<(PlayerHandle, Tap)> {
    let device = device.to_string();
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(ArcSwap::from_pointee(PlayerStatus {
        volume: 1.0,
        // Not Default: a speed of 0.0 would render as "0.00x" before the
        // engine publishes its first real status.
        speed: 1.0,
        ..Default::default()
    }));
    // One tap for the app's lifetime: the analyser is never rewired on track
    // change. ~1s of stereo audio is ample headroom.
    let (sink, tap) = pcm::tap(pcm::SAMPLE_RATE as usize * pcm::CHANNELS as usize);
    let errors = tap.errors();

    let handle = PlayerHandle {
        tx,
        status: status.clone(),
    };
    std::thread::Builder::new()
        .name("ytm-player".into())
        .spawn(
            move || match Engine::new(resolver, sink, errors, status.clone(), &device) {
                Ok(mut e) => e.run(rx),
                Err(err) => {
                    status.store(Arc::new(PlayerStatus {
                        error: Some(format!("audio device unavailable: {err}")),
                        speed: 1.0,
                        ..Default::default()
                    }));
                }
            },
        )?;

    Ok((handle, tap))
}

/// Hand pages freed inside glibc's arenas back to the operating system.
///
/// A track handover frees a couple of megabytes at once - the previous
/// decoder's ring buffer, chiefly - but glibc keeps an arena at its high-water
/// mark, so RSS ratchets upwards over a long listening session and never comes
/// back down. Trimming at track boundaries is cheap and puts it back.
///
/// glibc only. musl has no arenas to trim, and no `malloc_trim`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim_heap() {
    extern "C" {
        fn malloc_trim(pad: usize) -> std::os::raw::c_int;
    }
    // Safety: `malloc_trim` takes no pointers and only walks the allocator's
    // own free lists.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim_heap() {}

/// Don't trim more often than this. Holding "next" would otherwise walk the
/// free lists on every keypress for no benefit.
const TRIM_INTERVAL: Duration = Duration::from_secs(30);

struct Engine {
    _device: rodio::MixerDeviceSink,
    /// Two players on one mixer. Crossfading needs both playing at once, which
    /// a single queue cannot do; with crossfade off only `active` is ever used
    /// and the behaviour is exactly the gapless path.
    players: [Player; 2],
    active: usize,
    /// Seconds of overlap between tracks. Zero keeps gapless handover.
    crossfade: f32,
    /// The fade in progress: the queue index being faded in, its progress, and
    /// when the fade started.
    fading: Option<(usize, Progress, Instant)>,
    resolver: Arc<ResolverCache>,
    sink: TapSink,
    errors: Arc<Mutex<Vec<String>>>,
    status: Arc<ArcSwap<PlayerStatus>>,

    /// Copy-on-write, so `publish` can hand the UI a snapshot 25 times a
    /// second without deep-copying every track. Mutating it clones the vector
    /// once, which is fine: the queue changes a handful of times an hour.
    queue: Arc<Vec<Track>>,
    index: usize,
    state: PlayState,
    repeat: RepeatMode,
    shuffle: bool,
    volume: f32,
    filters: Filters,
    progress: Progress,
    /// The decoder for the *next* track, opened early and queued behind the
    /// current one so the handover has no gap (FR-P5).
    armed: Option<(usize, Progress)>,
    arming: Option<Receiver<ArmResult>>,
    /// Guards against reading `player.empty()` before the source is running.
    started: Option<Instant>,
    error: Option<String>,
    rng: u64,
    /// When the heap was last trimmed, so a burst of skips does not.
    last_trim: Instant,
}

impl Engine {
    fn new(
        resolver: Arc<ResolverCache>,
        sink: TapSink,
        errors: Arc<Mutex<Vec<String>>>,
        status: Arc<ArcSwap<PlayerStatus>>,
        device_name: &str,
    ) -> Result<Self> {
        let device = open_device(device_name)?;
        let players = [
            Player::connect_new(device.mixer()),
            Player::connect_new(device.mixer()),
        ];
        Ok(Self {
            _device: device,
            players,
            active: 0,
            crossfade: 0.0,
            fading: None,
            resolver,
            sink,
            errors,
            status,
            queue: Arc::new(Vec::new()),
            index: 0,
            state: PlayState::Stopped,
            repeat: RepeatMode::Off,
            shuffle: false,
            volume: 1.0,
            filters: Filters::default(),
            progress: Progress::default(),
            armed: None,
            arming: None,
            started: None,
            error: None,
            rng: 0x2545F4914F6CDD1D,
            last_trim: Instant::now(),
        })
    }

    /// The player currently carrying the track being listened to.
    fn player(&self) -> &Player {
        &self.players[self.active]
    }

    /// The other player, used for the incoming track during a crossfade.
    fn other(&self) -> &Player {
        &self.players[1 - self.active]
    }

    fn run(&mut self, rx: Receiver<Command>) {
        loop {
            // Drain every pending command before doing any work.
            loop {
                match rx.try_recv() {
                    Ok(Command::Shutdown) => return,
                    Ok(c) => self.handle(c),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if self.crossfade > 0.0 {
                self.poll_crossfade();
            } else {
                self.poll_gapless();
            }
            self.poll_track_end();
            self.drain_decoder_errors();
            self.publish();
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn handle(&mut self, c: Command) {
        match c {
            Command::PlayQueue { tracks, index } => {
                self.queue = Arc::new(tracks);
                self.index = index.min(self.queue.len().saturating_sub(1));
                self.start(0.0);
            }
            Command::Enqueue(t) => {
                let was_last = self.index + 1 >= self.queue.len();
                Arc::make_mut(&mut self.queue).push(t);
                // Appending only changes what plays next if nothing followed.
                if was_last {
                    self.disarm_and_correct();
                }
            }
            Command::AppendRadio(tracks) => {
                if tracks.is_empty() {
                    return;
                }
                let was_empty_ahead = self.index + 1 >= self.queue.len();
                Arc::make_mut(&mut self.queue).extend(tracks);
                if self.state == PlayState::Stopped && was_empty_ahead {
                    self.index += 1;
                    self.start(0.0);
                }
            }
            Command::PlayNext(t) => {
                let at = (self.index + 1).min(self.queue.len());
                Arc::make_mut(&mut self.queue).insert(at, t);
                // The armed decoder is for whatever used to be next.
                self.disarm_and_correct();
            }
            Command::RestoreQueue {
                tracks,
                index,
                position,
            } => {
                if tracks.is_empty() {
                    return;
                }
                self.queue = Arc::new(tracks);
                self.index = index.min(self.queue.len() - 1);
                self.start(position);
                self.player().pause();
                self.state = PlayState::Paused;
            }
            Command::JumpTo(i) => {
                if i < self.queue.len() {
                    self.index = i;
                    self.start(0.0);
                }
            }
            Command::RemoveAt(i) => {
                if i < self.queue.len() {
                    if i == self.index + 1 {
                        self.disarm_and_correct();
                    }
                    Arc::make_mut(&mut self.queue).remove(i);
                    if i < self.index {
                        self.index -= 1;
                    } else if i == self.index {
                        // Removing the playing track advances to what took its place.
                        if self.index >= self.queue.len() {
                            self.stop();
                        } else {
                            self.start(0.0);
                        }
                    }
                }
            }
            Command::TogglePause => match self.state {
                PlayState::Playing => {
                    // Both, or a fading-in track would keep playing.
                    self.players[0].pause();
                    self.players[1].pause();
                    self.state = PlayState::Paused;
                }
                PlayState::Paused => {
                    self.player().play();
                    if self.fading.is_some() {
                        self.other().play();
                    }
                    self.state = PlayState::Playing;
                }
                _ => {}
            },
            Command::Next => self.advance(true),
            Command::Prev => {
                // Matches the official player: restart if we are past 3 seconds.
                if self.progress.seconds() > 3.0 {
                    self.start(0.0);
                } else if self.index > 0 {
                    self.index -= 1;
                    self.start(0.0);
                } else {
                    self.start(0.0);
                }
            }
            Command::SeekRelative(d) => {
                if self.state != PlayState::Stopped {
                    let target = (self.progress.seconds() + d).max(0.0);
                    let dur = self.current_duration();
                    let target = match dur {
                        Some(t) if target >= t - 0.5 => {
                            self.advance(true);
                            return;
                        }
                        _ => target,
                    };
                    self.start(target);
                }
            }
            Command::SetVolume(v) => {
                self.volume = v.clamp(0.0, 1.5);
                // During a fade the ramp owns the volumes; it will pick this up.
                if self.fading.is_none() {
                    self.player().set_volume(self.volume);
                }
            }
            Command::CycleRepeat => {
                self.repeat = self.repeat.next();
                if self.repeat == RepeatMode::One {
                    self.disarm_and_correct();
                }
            }
            Command::ToggleShuffle => {
                self.shuffle = !self.shuffle;
                if self.shuffle {
                    self.shuffle_tail();
                }
                self.disarm_and_correct();
            }
            Command::SetCrossfade(v) => {
                self.crossfade = v.clamp(0.0, 12.0);
                // Switching modes mid-track would leave one path half armed.
                self.disarm();
                self.cancel_fade();
            }
            Command::SetSpeed(v) => {
                let v = v.clamp(0.5, 2.0);
                if (v - self.filters.speed).abs() > 0.001 {
                    self.filters.speed = v;
                    self.restart_in_place();
                }
            }
            Command::ToggleNormalize => {
                self.filters.normalize = !self.filters.normalize;
                self.restart_in_place();
            }
            Command::Stop => self.stop(),
            Command::Shutdown => {}
        }
    }

    /// Prepare the next track and queue it behind the current one, so playback
    /// runs straight on rather than pausing to spawn a decoder.
    ///
    /// rodio plays queued sources back to back, so the handover costs nothing;
    /// the gap in the naive version is entirely the ffmpeg spawn and prebuffer,
    /// which this moves off the critical path.
    fn poll_gapless(&mut self) {
        // Collect a decoder that finished opening.
        if let Some(rx) = &self.arming {
            match rx.try_recv() {
                Ok(Ok((index, src, progress))) => {
                    self.arming = None;
                    // Still valid? A skip or seek may have moved on meanwhile.
                    if self.state == PlayState::Playing && self.index + 1 == index {
                        self.player().append(src);
                        self.armed = Some((index, progress));
                    }
                }
                Ok(Err(_)) => self.arming = None,
                Err(mpsc::TryRecvError::Disconnected) => self.arming = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if self.armed.is_some() || self.arming.is_some() || self.state != PlayState::Playing {
            return;
        }
        // Repeat-one replays the same source; queueing the next would be wrong.
        if self.repeat == RepeatMode::One {
            return;
        }
        let Some(remaining) = self.remaining_secs() else {
            return;
        };
        if remaining > 12.0 {
            return;
        }
        let next_index = self.index + 1;
        let Some(track) = self.queue.get(next_index).cloned() else {
            return;
        };

        let (tx, rx) = mpsc::channel();
        self.arming = Some(rx);
        let resolver = self.resolver.clone();
        let sink = self.sink.clone();
        let errors = self.errors.clone();
        let filters = self.filters;
        std::thread::spawn(move || {
            let progress = Progress::default();
            let r = resolver.resolve(&track.id).and_then(|fmt| {
                FfmpegPcm::open_with(&fmt.url, 0.0, sink, progress.clone(), errors, filters)
                    .map(|src| (next_index, src, progress))
            });
            let _ = tx.send(r.map_err(|e| e.to_string()));
        });
    }

    /// Start the next track on the other player and ramp between them.
    ///
    /// Two players are needed because a crossfade means both tracks audible at
    /// once, which a single sequential queue cannot express.
    fn poll_crossfade(&mut self) {
        // A decoder finished opening: begin the ramp.
        if self.fading.is_none() {
            if let Some(rx) = &self.arming {
                match rx.try_recv() {
                    Ok(Ok((index, src, progress))) => {
                        self.arming = None;
                        if self.state == PlayState::Playing && self.index + 1 == index {
                            self.other().set_volume(0.0);
                            self.other().append(src);
                            self.other().play();
                            self.fading = Some((index, progress, Instant::now()));
                        }
                    }
                    Ok(Err(_)) => self.arming = None,
                    Err(mpsc::TryRecvError::Disconnected) => self.arming = None,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
        }

        // A fade in flight: drive the ramp.
        if let Some((index, progress, started)) = self.fading.clone() {
            let t = (started.elapsed().as_secs_f32() / self.crossfade).clamp(0.0, 1.0);
            self.players[self.active].set_volume(self.volume * (1.0 - t));
            self.players[1 - self.active].set_volume(self.volume * t);
            if t >= 1.0 {
                self.players[self.active].clear();
                self.players[self.active].set_volume(self.volume);
                self.active = 1 - self.active;
                self.index = index;
                self.progress = progress;
                self.fading = None;
                self.started = Some(Instant::now());
                self.prefetch_next();
                self.maybe_trim();
            }
            return;
        }

        if self.state != PlayState::Playing
            || self.repeat == RepeatMode::One
            || self.arming.is_some()
        {
            return;
        }
        let Some(remaining) = self.remaining_secs() else {
            return;
        };
        // Start opening early enough that the decoder is ready when the fade
        // should begin, rather than after it.
        if remaining > self.crossfade as f64 + 4.0 {
            return;
        }
        let next_index = self.index + 1;
        let Some(track) = self.queue.get(next_index).cloned() else {
            return;
        };

        // Resolving and opening take a second or more. Doing that here would
        // stall the engine thread, and with it the ramp itself - the fade would
        // jump rather than glide. Open in the background and start the ramp
        // when it lands, exactly as the gapless path does.
        let (tx, rx) = mpsc::channel();
        self.arming = Some(rx);
        let resolver = self.resolver.clone();
        let sink = self.sink.clone();
        let errors = self.errors.clone();
        let filters = self.filters;
        std::thread::spawn(move || {
            let progress = Progress::default();
            let r = resolver.resolve(&track.id).and_then(|fmt| {
                FfmpegPcm::open_with(&fmt.url, 0.0, sink, progress.clone(), errors, filters)
                    .map(|src| (next_index, src, progress))
            });
            let _ = tx.send(r.map_err(|e| e.to_string()));
        });
    }

    fn cancel_fade(&mut self) {
        if self.fading.take().is_some() {
            self.other().clear();
            self.other().set_volume(self.volume);
            self.players[self.active].set_volume(self.volume);
        }
    }

    fn remaining_secs(&self) -> Option<f64> {
        let total = self.current_duration()?;
        Some((total - self.progress.seconds()).max(0.0))
    }

    /// Re-open the decoder at the current position, e.g. after changing a
    /// filter. ffmpeg applies filters at open, so there is no way to change
    /// them mid-stream.
    fn restart_in_place(&mut self) {
        if self.state == PlayState::Stopped {
            return;
        }
        let at = self.progress.seconds();
        let was_paused = self.state == PlayState::Paused;
        self.start(at);
        if was_paused {
            self.player().pause();
            self.state = PlayState::Paused;
        }
    }

    /// Throw away a queued next decoder, e.g. after a skip or a seek.
    fn disarm(&mut self) {
        self.armed = None;
        self.arming = None;
    }

    /// Invalidate a queued next decoder when what comes next has changed.
    ///
    /// rodio can only clear the whole queue, not one entry behind the playing
    /// source, so if a decoder is already queued the current track has to be
    /// restarted at its position. That only happens inside the last few
    /// seconds of a track, where these actions are rare.
    fn disarm_and_correct(&mut self) {
        let was_armed = self.armed.is_some();
        self.disarm();
        if was_armed && self.state == PlayState::Playing {
            let at = self.progress.seconds();
            self.start(at);
        }
    }

    /// Begin (or restart) the current track at `from` seconds.
    fn start(&mut self, from: f64) {
        self.disarm();
        self.cancel_fade();
        self.player().clear();
        self.started = None;
        self.error = None;

        let Some(track) = self.queue.get(self.index).cloned() else {
            self.state = PlayState::Stopped;
            return;
        };

        self.state = PlayState::Buffering;
        self.publish();

        let fmt = match self.resolver.resolve(&track.id) {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(format!("resolve failed: {e}"));
                self.state = PlayState::Stopped;
                return;
            }
        };

        match FfmpegPcm::open_with(
            &fmt.url,
            from,
            self.sink.clone(),
            self.progress.clone(),
            self.errors.clone(),
            self.filters,
        ) {
            Ok(src) => {
                self.player().append(src);
                self.player().set_volume(self.volume);
                self.player().play();
                self.state = PlayState::Playing;
                self.started = Some(Instant::now());
                self.prefetch_next();
                // The previous decoder is gone by now, so this is the moment
                // its buffers can actually be given back.
                self.maybe_trim();
            }
            Err(e) => {
                self.error = Some(format!("decode failed: {e}"));
                self.state = PlayState::Stopped;
            }
        }
    }

    fn maybe_trim(&mut self) {
        if self.last_trim.elapsed() >= TRIM_INTERVAL {
            self.last_trim = Instant::now();
            trim_heap();
        }
    }

    fn stop(&mut self) {
        self.cancel_fade();
        self.player().clear();
        self.state = PlayState::Stopped;
        self.started = None;
    }

    /// Move to the next track. `manual` distinguishes a keypress from a track
    /// ending naturally, which matters for repeat-one.
    fn advance(&mut self, manual: bool) {
        self.disarm();
        if !manual && self.repeat == RepeatMode::One {
            self.start(0.0);
            return;
        }
        if self.index + 1 < self.queue.len() {
            self.index += 1;
            self.start(0.0);
        } else if self.repeat == RepeatMode::All && !self.queue.is_empty() {
            self.index = 0;
            self.start(0.0);
        } else {
            self.stop();
        }
    }

    fn poll_track_end(&mut self) {
        if self.state != PlayState::Playing || self.fading.is_some() {
            return;
        }
        // A queued next source means rodio handles the handover itself; the
        // queue length dropping back to one is how we learn it happened.
        if let Some((index, progress)) = self.armed.clone() {
            if self.player().len() <= 1 {
                self.index = index;
                self.progress = progress;
                self.armed = None;
                self.started = Some(Instant::now());
                self.prefetch_next();
                // rodio has dropped the finished source, so its ring buffer is
                // free and can go back to the OS.
                self.maybe_trim();
            }
            return;
        }
        // Ignore `empty()` briefly after start: the source is not yet running.
        let Some(t0) = self.started else { return };
        if t0.elapsed() < Duration::from_millis(400) {
            return;
        }
        if self.player().empty() {
            self.advance(false);
        }
    }

    fn drain_decoder_errors(&mut self) {
        let mut g = self.errors.lock().unwrap();
        if let Some(last) = g.last() {
            self.error = Some(format!("ffmpeg: {last}"));
        }
        g.clear();
    }

    fn prefetch_next(&self) {
        if let Some(next) = self.queue.get(self.index + 1) {
            self.resolver.prefetch(&next.id);
        }
    }

    fn current_duration(&self) -> Option<f64> {
        self.queue
            .get(self.index)
            .and_then(|t| t.duration)
            .map(|d| d.as_secs_f64())
    }

    /// Shuffle only what has not been played yet, preserving the current track.
    fn shuffle_tail(&mut self) {
        let start = self.index + 1;
        if start >= self.queue.len() {
            return;
        }
        for i in (start + 1..self.queue.len()).rev() {
            let j = start + (self.next_rand() as usize % (i - start + 1));
            Arc::make_mut(&mut self.queue).swap(i, j);
        }
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64*, plenty for shuffling a play queue.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn publish(&self) {
        self.status.store(Arc::new(PlayerStatus {
            state: self.state,
            current: self.queue.get(self.index).cloned(),
            position: Duration::from_secs_f64(self.progress.seconds().max(0.0)),
            volume: self.volume,
            repeat: self.repeat,
            shuffle: self.shuffle,
            speed: self.filters.speed,
            normalize: self.filters.normalize,
            queue: self.queue.clone(),
            queue_index: self.index,
            error: self.error.clone(),
        }));
    }
}
