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

use crate::pcm::{self, FfmpegPcm, Progress, Tap, TapSink};
use crate::resolver::ResolverCache;

#[derive(Debug, Clone)]
pub enum Command {
    /// Replace the queue and start at `index`.
    PlayQueue { tracks: Vec<Track>, index: usize },
    Enqueue(Track),
    /// Append an autoplay continuation, and resume if the queue had run dry.
    AppendRadio(Vec<Track>),
    /// Reinstate a saved queue, paused at `position`. Restoring a session
    /// should put things back, not start making noise.
    RestoreQueue { tracks: Vec<Track>, index: usize, position: f64 },
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
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(ArcSwap::from_pointee(PlayerStatus {
        volume: 1.0,
        ..Default::default()
    }));
    // One tap for the app's lifetime: the analyser is never rewired on track
    // change. ~1s of stereo audio is ample headroom.
    let (sink, tap) = pcm::tap(pcm::SAMPLE_RATE as usize * pcm::CHANNELS as usize);
    let errors = tap.errors();

    let handle = PlayerHandle { tx, status: status.clone() };
    std::thread::Builder::new()
        .name("ytm-player".into())
        .spawn(move || {
            match Engine::new(resolver, sink, errors, status.clone()) {
                Ok(mut e) => e.run(rx),
                Err(err) => {
                    status.store(Arc::new(PlayerStatus {
                        error: Some(format!("audio device unavailable: {err}")),
                        ..Default::default()
                    }));
                }
            }
        })?;

    Ok((handle, tap))
}

struct Engine {
    _device: rodio::MixerDeviceSink,
    player: Player,
    resolver: Arc<ResolverCache>,
    sink: TapSink,
    errors: Arc<Mutex<Vec<String>>>,
    status: Arc<ArcSwap<PlayerStatus>>,

    queue: Vec<Track>,
    index: usize,
    state: PlayState,
    repeat: RepeatMode,
    shuffle: bool,
    volume: f32,
    progress: Progress,
    /// The decoder for the *next* track, opened early and queued behind the
    /// current one so the handover has no gap (FR-P5).
    armed: Option<(usize, Progress)>,
    arming: Option<Receiver<ArmResult>>,
    /// Guards against reading `player.empty()` before the source is running.
    started: Option<Instant>,
    error: Option<String>,
    rng: u64,
}

impl Engine {
    fn new(
        resolver: Arc<ResolverCache>,
        sink: TapSink,
        errors: Arc<Mutex<Vec<String>>>,
        status: Arc<ArcSwap<PlayerStatus>>,
    ) -> Result<Self> {
        let device = DeviceSinkBuilder::open_default_sink()?;
        let player = Player::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
            resolver,
            sink,
            errors,
            status,
            queue: Vec::new(),
            index: 0,
            state: PlayState::Stopped,
            repeat: RepeatMode::Off,
            shuffle: false,
            volume: 1.0,
            progress: Progress::default(),
            armed: None,
            arming: None,
            started: None,
            error: None,
            rng: 0x2545F4914F6CDD1D,
        })
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
            self.poll_gapless();
            self.poll_track_end();
            self.drain_decoder_errors();
            self.publish();
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn handle(&mut self, c: Command) {
        match c {
            Command::PlayQueue { tracks, index } => {
                self.queue = tracks;
                self.index = index.min(self.queue.len().saturating_sub(1));
                self.start(0.0);
            }
            Command::Enqueue(t) => {
                let was_last = self.index + 1 >= self.queue.len();
                self.queue.push(t);
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
                self.queue.extend(tracks);
                if self.state == PlayState::Stopped && was_empty_ahead {
                    self.index += 1;
                    self.start(0.0);
                }
            }
            Command::PlayNext(t) => {
                let at = (self.index + 1).min(self.queue.len());
                self.queue.insert(at, t);
                // The armed decoder is for whatever used to be next.
                self.disarm_and_correct();
            }
            Command::RestoreQueue { tracks, index, position } => {
                if tracks.is_empty() {
                    return;
                }
                self.queue = tracks;
                self.index = index.min(self.queue.len() - 1);
                self.start(position);
                self.player.pause();
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
                    self.queue.remove(i);
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
                    self.player.pause();
                    self.state = PlayState::Paused;
                }
                PlayState::Paused => {
                    self.player.play();
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
                self.player.set_volume(self.volume);
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
                        self.player.append(src);
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
        let Some(remaining) = self.remaining_secs() else { return };
        if remaining > 12.0 {
            return;
        }
        let next_index = self.index + 1;
        let Some(track) = self.queue.get(next_index).cloned() else { return };

        let (tx, rx) = mpsc::channel();
        self.arming = Some(rx);
        let resolver = self.resolver.clone();
        let sink = self.sink.clone();
        let errors = self.errors.clone();
        std::thread::spawn(move || {
            let progress = Progress::default();
            let r = resolver.resolve(&track.id).and_then(|fmt| {
                FfmpegPcm::open(&fmt.url, 0.0, sink, progress.clone(), errors)
                    .map(|src| (next_index, src, progress))
            });
            let _ = tx.send(r.map_err(|e| e.to_string()));
        });
    }

    fn remaining_secs(&self) -> Option<f64> {
        let total = self.current_duration()?;
        Some((total - self.progress.seconds()).max(0.0))
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
        self.player.clear();
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

        match FfmpegPcm::open(
            &fmt.url,
            from,
            self.sink.clone(),
            self.progress.clone(),
            self.errors.clone(),
        ) {
            Ok(src) => {
                self.player.append(src);
                self.player.set_volume(self.volume);
                self.player.play();
                self.state = PlayState::Playing;
                self.started = Some(Instant::now());
                self.prefetch_next();
            }
            Err(e) => {
                self.error = Some(format!("decode failed: {e}"));
                self.state = PlayState::Stopped;
            }
        }
    }

    fn stop(&mut self) {
        self.player.clear();
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
        if self.state != PlayState::Playing {
            return;
        }
        // A queued next source means rodio handles the handover itself; the
        // queue length dropping back to one is how we learn it happened.
        if let Some((index, progress)) = self.armed.clone() {
            if self.player.len() <= 1 {
                self.index = index;
                self.progress = progress;
                self.armed = None;
                self.started = Some(Instant::now());
                self.prefetch_next();
            }
            return;
        }
        // Ignore `empty()` briefly after start: the source is not yet running.
        let Some(t0) = self.started else { return };
        if t0.elapsed() < Duration::from_millis(400) {
            return;
        }
        if self.player.empty() {
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
            self.queue.swap(i, j);
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
            queue: self.queue.clone(),
            queue_index: self.index,
            error: self.error.clone(),
        }));
    }
}
