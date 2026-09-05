//! ffmpeg-piped PCM decoding with a lock-free tap for the visualiser.
//!
//! ffmpeg handles every codec YouTube serves (Opus, AAC, ...) while the decoded
//! PCM stays *in our own process*, which is what makes an exact,
//! perfectly-synchronised spectrum possible.
//!
//! Reading from the pipe happens on a dedicated thread, never in `next()`:
//! `next()` is called from the audio callback, and a blocking read there causes
//! buffer underruns the moment the network hiccups (NFR-4, R5).

use anyhow::{Context, Result};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use rodio::{ChannelCount, SampleRate, Source};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

/// Decoded audio held ahead of the sound card: ~4 s of stereo f32.
const AUDIO_RING: usize = SAMPLE_RATE as usize * CHANNELS as usize * 4;
/// How much to buffer before letting playback begin.
const PREBUFFER: usize = SAMPLE_RATE as usize * CHANNELS as usize / 2;
/// Give up on a stalled stream rather than emitting silence forever.
const STARVE_TIMEOUT: Duration = Duration::from_secs(15);

/// Producer side of the visualiser tap, shared by whichever decoder is running.
/// Created once for the app's lifetime so the analyser is never rewired.
#[derive(Clone)]
pub struct TapSink(Arc<Mutex<HeapProd<f32>>>);

/// Consumer side of the tap. The analyser owns this.
pub struct Tap {
    cons: HeapCons<f32>,
    errors: Arc<Mutex<Vec<String>>>,
}

pub fn tap(capacity: usize) -> (TapSink, Tap) {
    let (prod, cons) = HeapRb::<f32>::new(capacity).split();
    (
        TapSink(Arc::new(Mutex::new(prod))),
        Tap { cons, errors: Arc::new(Mutex::new(Vec::new())) },
    )
}

impl Tap {
    pub fn drain(&mut self, out: &mut Vec<f32>) -> usize {
        let mut n = 0;
        while let Some(s) = self.cons.try_pop() {
            out.push(s);
            n += 1;
        }
        n
    }
    pub fn errors(&self) -> Arc<Mutex<Vec<String>>> {
        self.errors.clone()
    }
}

/// Playback position of the currently running decoder.
#[derive(Clone, Default)]
pub struct Progress {
    frames: Arc<AtomicU64>,
    offset_secs: Arc<Mutex<f64>>,
}

impl Progress {
    pub fn seconds(&self) -> f64 {
        self.frames.load(Ordering::Relaxed) as f64 / SAMPLE_RATE as f64
            + *self.offset_secs.lock().unwrap()
    }
    fn reset(&self, offset: f64) {
        self.frames.store(0, Ordering::Relaxed);
        *self.offset_secs.lock().unwrap() = offset;
    }
}

/// A rodio `Source` serving PCM that a background thread pulls from ffmpeg.
pub struct FfmpegPcm {
    child: Arc<Mutex<Child>>,
    cons: HeapCons<f32>,
    eof: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    sink: TapSink,
    progress: Progress,
    frame_parity: usize,
    /// When the ring first ran dry; used to fail a permanently stalled stream.
    starving_since: Option<Instant>,
    pub dropped: u64,
}

impl FfmpegPcm {
    /// Open `input` (local path or HTTP(S) URL), seeking to `start` seconds.
    /// Blocks briefly to prebuffer, so call this from the engine thread.
    pub fn open(
        input: &str,
        start: f64,
        sink: TapSink,
        progress: Progress,
        errors: Arc<Mutex<Vec<String>>>,
    ) -> Result<Self> {
        progress.reset(start);

        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        // `-reconnect*` are HTTP-protocol options; passing them for a local file
        // makes ffmpeg abort with "Option reconnect not found".
        if is_network_input(input) {
            cmd.args([
                "-reconnect", "1",
                "-reconnect_streamed", "1",
                "-reconnect_delay_max", "5",
            ]);
        }
        if start > 0.05 {
            cmd.args(["-ss", &format!("{start:.3}")]);
        }

        let mut child = cmd
            .args([
                "-i", input,
                "-f", "f32le",
                "-acodec", "pcm_f32le",
                "-ar", &SAMPLE_RATE.to_string(),
                "-ac", &CHANNELS.to_string(),
                "-",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn ffmpeg - is it on PATH?")?;

        let stdout = child.stdout.take().context("ffmpeg stdout missing")?;

        // NFR-13: never discard a subprocess's stderr.
        if let Some(err) = child.stderr.take() {
            let log = errors.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    let mut g = log.lock().unwrap();
                    g.push(line);
                    if g.len() > 32 {
                        g.remove(0);
                    }
                }
            });
        }

        let (mut prod, cons) = HeapRb::<f32>::new(AUDIO_RING).split();
        let eof = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));

        // Reader thread: the only place that blocks on the pipe.
        {
            let eof = eof.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("ytm-decode".into())
                .spawn(move || {
                    let mut r = BufReader::with_capacity(64 * 1024, stdout);
                    let mut raw = [0u8; 16 * 1024];
                    let mut tail: Vec<u8> = Vec::with_capacity(4);
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        let carry = tail.len();
                        raw[..carry].copy_from_slice(&tail);
                        tail.clear();
                        let n = match r.read(&mut raw[carry..]) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        let filled = carry + n;
                        let whole = filled / 4;
                        tail.extend_from_slice(&raw[whole * 4..filled]);

                        let mut i = 0;
                        while i < whole {
                            if stop.load(Ordering::Relaxed) {
                                break;
                            }
                            let s = f32::from_le_bytes([
                                raw[i * 4], raw[i * 4 + 1], raw[i * 4 + 2], raw[i * 4 + 3],
                            ]);
                            if prod.try_push(s).is_ok() {
                                i += 1;
                            } else {
                                // Ring full: playback is ahead of us. Wait.
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                    eof.store(true, Ordering::Relaxed);
                })?;
        }

        let this = Self {
            child: Arc::new(Mutex::new(child)),
            cons,
            eof,
            stop,
            sink,
            progress,
            frame_parity: 0,
            starving_since: None,
            dropped: 0,
        };

        // Prebuffer so playback does not start into an empty ring.
        let t0 = Instant::now();
        while this.cons.occupied_len() < PREBUFFER
            && !this.eof.load(Ordering::Relaxed)
            && t0.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(this)
    }
}

impl Iterator for FfmpegPcm {
    type Item = f32;

    /// Called on the audio thread. Never blocks, never allocates.
    fn next(&mut self) -> Option<f32> {
        let s = match self.cons.try_pop() {
            Some(s) => {
                self.starving_since = None;
                s
            }
            None => {
                if self.eof.load(Ordering::Relaxed) {
                    return None; // genuine end of track
                }
                // Underrun: emit silence rather than stalling the device, but
                // do not do so indefinitely.
                let since = *self.starving_since.get_or_insert_with(Instant::now);
                if since.elapsed() > STARVE_TIMEOUT {
                    return None;
                }
                0.0
            }
        };

        self.frame_parity += 1;
        if self.frame_parity >= CHANNELS as usize {
            self.frame_parity = 0;
            self.progress.frames.fetch_add(1, Ordering::Relaxed);
        }

        // Tap: never block. try_lock is effectively uncontended (one decoder at
        // a time) and dropping a sample beats stalling the audio thread.
        if let Ok(mut p) = self.sink.0.try_lock() {
            if p.try_push(s).is_err() {
                self.dropped = self.dropped.wrapping_add(1);
            }
        }
        Some(s)
    }
}

impl Source for FfmpegPcm {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(CHANNELS).expect("nonzero")
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(SAMPLE_RATE).expect("nonzero")
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Drop for FfmpegPcm {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn is_network_input(input: &str) -> bool {
    let l = input.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}
