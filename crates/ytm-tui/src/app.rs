//! Application state and the event loop.
//!
//! The UI thread never blocks on I/O: search runs on a worker thread and
//! reports back by channel, the player is driven by messages, and the analyser
//! publishes frames the renderer samples at its own pace.

use anyhow::Result;
use arc_swap::ArcSwap;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ytm_api::MusicBackend;
use ytm_core::{Rating, Track};
use ytm_player::engine::Command as PCmd;
use ytm_player::{PlayerHandle, Tap, CHANNELS, SAMPLE_RATE};
use ytm_viz::{Analyser, SpectrumFrame};

use crate::spectrum::VizStyle;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Results,
    Queue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
}

pub enum AppEvent {
    Search(Result<Vec<Track>, String>),
    /// An autoplay continuation, ready to append to the running queue.
    Radio(Result<Vec<Track>, String>),
    /// A radio started deliberately from a selection, replacing the queue.
    RadioFrom(Track, Result<Vec<Track>, String>),
    Toast(String),
}

pub struct App {
    pub backend: Arc<dyn MusicBackend>,
    pub player: PlayerHandle,
    pub theme: Theme,

    pub mode: Mode,
    pub focus: Focus,
    pub query: String,
    pub results: Vec<Track>,
    pub results_sel: usize,
    pub queue_sel: usize,
    pub searching: bool,
    pub results_title: String,

    pub viz_style: VizStyle,
    pub viz_fullscreen: bool,
    pub show_help: bool,
    pub spectrum: Arc<ArcSwap<SpectrumFrame>>,
    /// Written by the renderer once the spectrum area's width is known.
    pub n_bands: Arc<AtomicU64>,

    pub toast: Option<(String, Instant)>,
    pub should_quit: bool,
    /// Continue with a station when the queue runs dry, as the official
    /// player does (FR-Q3).
    pub autoplay: bool,
    prev_state: ytm_core::PlayState,
    radio_pending: bool,

    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    quit_flag: Arc<AtomicBool>,
}

impl App {
    pub fn new(backend: Arc<dyn MusicBackend>, player: PlayerHandle, tap: Tap) -> Self {
        let (tx, rx) = mpsc::channel();
        let spectrum = Arc::new(ArcSwap::from_pointee(SpectrumFrame::default()));
        let n_bands = Arc::new(AtomicU64::new(96));
        let quit_flag = Arc::new(AtomicBool::new(false));

        spawn_analyser(tap, spectrum.clone(), n_bands.clone(), quit_flag.clone());

        let mut app = Self {
            backend,
            player,
            theme: Theme::default(),
            mode: Mode::Normal,
            focus: Focus::Results,
            query: String::new(),
            results: Vec::new(),
            results_sel: 0,
            queue_sel: 0,
            searching: false,
            results_title: "Results".into(),
            viz_style: VizStyle::Mirrored,
            viz_fullscreen: false,
            show_help: false,
            spectrum,
            n_bands,
            toast: None,
            should_quit: false,
            autoplay: true,
            prev_state: ytm_core::PlayState::Stopped,
            radio_pending: false,
            tx,
            rx,
            quit_flag,
        };
        // Signed-in users get something to play immediately.
        if app.backend.is_authenticated() {
            app.load_liked();
        } else {
            app.results_title = "Search results".into();
            app.toast("anonymous mode - press / to search, likes disabled".into());
        }
        app
    }

    pub fn toast(&mut self, msg: String) {
        self.toast = Some((msg, Instant::now()));
    }

    pub fn tick(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AppEvent::Search(Ok(tracks)) => {
                    self.searching = false;
                    self.results_sel = 0;
                    if tracks.is_empty() {
                        self.toast("no results".into());
                    }
                    self.results = tracks;
                }
                AppEvent::Search(Err(e)) => {
                    self.searching = false;
                    self.toast(format!("search failed: {e}"));
                }
                AppEvent::Radio(r) => {
                    self.radio_pending = false;
                    match r {
                        Ok(t) if !t.is_empty() => {
                            self.toast(format!("radio: {} more tracks", t.len()));
                            self.player.send(PCmd::AppendRadio(t));
                        }
                        Ok(_) => self.toast("radio returned nothing".into()),
                        Err(e) => self.toast(format!("radio failed: {e}")),
                    }
                }
                AppEvent::RadioFrom(seed, r) => {
                    self.radio_pending = false;
                    match r {
                        Ok(more) => {
                            let title = seed.title.clone();
                            let mut q = vec![seed];
                            q.extend(more);
                            self.toast(format!("radio from {title} ({} tracks)", q.len()));
                            self.player.send(PCmd::PlayQueue { tracks: q, index: 0 });
                        }
                        Err(e) => self.toast(format!("radio failed: {e}")),
                    }
                }
                AppEvent::Toast(m) => self.toast(m),
            }
        }

        self.maybe_autoplay();
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            }
        }
    }

    /// When the queue runs dry, continue with a station seeded from the track
    /// that just finished - matching the official player rather than falling
    /// silent.
    fn maybe_autoplay(&mut self) {
        use ytm_core::PlayState;
        let st = self.player.status();
        let stopped_just_now = self.prev_state == PlayState::Playing && st.state == PlayState::Stopped;
        self.prev_state = st.state;

        if !stopped_just_now || !self.autoplay || self.radio_pending {
            return;
        }
        // Only when we actually ran off the end of the queue.
        if st.queue.is_empty() || st.queue_index + 1 < st.queue.len() {
            return;
        }
        let Some(seed) = st.queue.get(st.queue_index).cloned() else { return };

        self.radio_pending = true;
        self.toast("queue finished - starting radio\u{2026}".into());
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b.radio(&seed.id).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Radio(r));
        });
    }

    /// Start a station from the highlighted track (FR-Q4).
    fn start_radio_from_selection(&mut self) {
        let Some(seed) = self.selected_track() else {
            self.toast("nothing selected".into());
            return;
        };
        if self.radio_pending {
            return;
        }
        self.radio_pending = true;
        self.toast(format!("starting radio from {}\u{2026}", seed.title));
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b.radio(&seed.id).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::RadioFrom(seed, r));
        });
    }

    fn load_liked(&mut self) {
        self.results_title = "Liked songs".into();
        self.searching = true;
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b.liked_songs().map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Search(r));
        });
    }

    fn run_search(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.results_title = format!("Search: {q}");
        self.searching = true;
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b.search_songs(&q).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Search(r));
        });
    }

    fn rate_current(&mut self, want: Rating) {
        let st = self.player.status();
        let Some(track) = st.current.clone() else {
            self.toast("nothing playing".into());
            return;
        };
        if !self.backend.is_authenticated() {
            self.toast("not signed in - see docs/M0-FINDINGS.md section 8".into());
            return;
        }
        // Thumbs-up and thumbs-down both toggle off when pressed again.
        let new = if want == Rating::Like {
            track.rating.toggled_like()
        } else {
            track.rating.toggled_dislike()
        };
        let b = self.backend.clone();
        let tx = self.tx.clone();
        let id = track.id.clone();
        let title = track.title.clone();
        std::thread::spawn(move || {
            let msg = match b.rate(&id, new) {
                Ok(()) => match new {
                    Rating::Like => format!("liked: {title}"),
                    Rating::Dislike => format!("disliked: {title}"),
                    Rating::Indifferent => format!("rating cleared: {title}"),
                },
                Err(e) => format!("rating failed: {e}"),
            };
            let _ = tx.send(AppEvent::Toast(msg));
        });
        // Thumbs-down skips, mirroring the official player (FR-R2).
        if new == Rating::Dislike {
            self.player.send(PCmd::Next);
        }
    }

    fn selected_track(&self) -> Option<Track> {
        match self.focus {
            Focus::Results => self.results.get(self.results_sel).cloned(),
            Focus::Queue => self.player.status().queue.get(self.queue_sel).cloned(),
        }
    }

    fn move_sel(&mut self, delta: isize) {
        let len = match self.focus {
            Focus::Results => self.results.len(),
            Focus::Queue => self.player.status().queue.len(),
        };
        if len == 0 {
            return;
        }
        let sel = match self.focus {
            Focus::Results => &mut self.results_sel,
            Focus::Queue => &mut self.queue_sel,
        };
        let n = (*sel as isize + delta).clamp(0, len as isize - 1);
        *sel = n as usize;
    }

    pub fn handle_key(&mut self, k: KeyEvent) {
        if self.mode == Mode::Search {
            match k.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    self.run_search();
                }
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            return;
        }

        if self.show_help {
            self.show_help = false;
            return;
        }

        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.query.clear();
            }
            KeyCode::Tab => {
                self.focus = if self.focus == Focus::Results { Focus::Queue } else { Focus::Results }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Home => self.move_sel(-(i32::MAX as isize)),
            KeyCode::End => self.move_sel(i32::MAX as isize),

            KeyCode::Enter => match self.focus {
                Focus::Results => {
                    if !self.results.is_empty() {
                        self.player.send(PCmd::PlayQueue {
                            tracks: self.results.clone(),
                            index: self.results_sel,
                        });
                        self.queue_sel = self.results_sel;
                    }
                }
                Focus::Queue => self.player.send(PCmd::JumpTo(self.queue_sel)),
            },
            KeyCode::Char('o') => {
                if let Some(t) = self.selected_track() {
                    let title = t.title.clone();
                    self.player.send(PCmd::PlayNext(t));
                    self.toast(format!("playing next: {title}"));
                }
            }
            KeyCode::Char('e') => {
                if let Some(t) = self.selected_track() {
                    let title = t.title.clone();
                    self.player.send(PCmd::Enqueue(t));
                    self.toast(format!("queued: {title}"));
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if self.focus == Focus::Queue {
                    self.player.send(PCmd::RemoveAt(self.queue_sel));
                }
            }

            KeyCode::Char(' ') => self.player.send(PCmd::TogglePause),
            KeyCode::Char('n') => self.player.send(PCmd::Next),
            KeyCode::Char('p') => self.player.send(PCmd::Prev),
            KeyCode::Right => self.player.send(PCmd::SeekRelative(if shift { 30.0 } else { 5.0 })),
            KeyCode::Left => self.player.send(PCmd::SeekRelative(if shift { -30.0 } else { -5.0 })),
            KeyCode::Char('R') => self.start_radio_from_selection(),
            KeyCode::Char('A') => {
                self.autoplay = !self.autoplay;
                let on = self.autoplay;
                self.toast(format!("autoplay {}", if on { "on" } else { "off" }));
            }
            KeyCode::Char('s') => self.player.send(PCmd::ToggleShuffle),
            KeyCode::Char('r') => self.player.send(PCmd::CycleRepeat),
            KeyCode::Char('0') => {
                let v = (self.player.status().volume + 0.05).min(1.5);
                self.player.send(PCmd::SetVolume(v));
            }
            KeyCode::Char('9') => {
                let v = (self.player.status().volume - 0.05).max(0.0);
                self.player.send(PCmd::SetVolume(v));
            }

            // Ratings pulled forward from M2: the API layer already supports them.
            KeyCode::Char('+') | KeyCode::Char('l') => self.rate_current(Rating::Like),
            KeyCode::Char('-') | KeyCode::Char('d') => self.rate_current(Rating::Dislike),

            KeyCode::Char('v') => self.viz_style = self.viz_style.next(),
            KeyCode::Char('z') => self.viz_fullscreen = !self.viz_fullscreen,
            KeyCode::Esc => {
                if self.viz_fullscreen {
                    self.viz_fullscreen = false;
                }
            }
            _ => {}
        }
    }

    /// Block for at most `timeout`, returning true if anything happened.
    pub fn poll_input(&mut self, timeout: Duration) -> Result<bool> {
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(k) if k.is_press() => self.handle_key(k),
                Event::Resize(_, _) => {}
                _ => {}
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub fn shutdown(&self) {
        self.quit_flag.store(true, Ordering::Relaxed);
        self.player.send(PCmd::Shutdown);
    }
}

/// The analyser thread. Rebuilds itself when the terminal width changes the
/// band count, and publishes an immutable frame the UI samples at its own pace.
fn spawn_analyser(
    mut tap: Tap,
    out: Arc<ArcSwap<SpectrumFrame>>,
    n_bands: Arc<AtomicU64>,
    quit: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("ytm-viz".into())
        .spawn(move || {
            let mut cur = n_bands.load(Ordering::Relaxed).max(1) as usize;
            let mut an = Analyser::new(cur, SAMPLE_RATE);
            let mut buf: Vec<f32> = Vec::with_capacity(1 << 16);
            let mut last = Instant::now();
            while !quit.load(Ordering::Relaxed) {
                let want = n_bands.load(Ordering::Relaxed).max(1) as usize;
                if want != cur {
                    cur = want;
                    an = Analyser::new(cur, SAMPLE_RATE);
                }
                let dt = last.elapsed().as_secs_f32().max(1e-4);
                last = Instant::now();

                buf.clear();
                if tap.drain(&mut buf) > 0 {
                    an.feed_interleaved(&buf, CHANNELS as usize);
                } else {
                    // Paused, stopped or starved: shift silence in so the bars
                    // fall to zero. Re-analysing a stale buffer makes them
                    // slowly climb as the gain ceiling decays toward it.
                    an.feed_silence((dt * SAMPLE_RATE as f32) as usize);
                }
                out.store(Arc::new(an.analyse(dt)));
                std::thread::sleep(Duration::from_millis(16));
            }
        })
        .expect("spawn analyser");
}
