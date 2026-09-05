//! Application state and the event loop.
//!
//! The UI thread never blocks on I/O: every fetch runs on a worker thread and
//! reports back by channel, the player is driven by messages, and the analyser
//! publishes frames the renderer samples at its own pace.

use anyhow::Result;
use arc_swap::ArcSwap;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ytm_api::{MusicBackend, SearchFilter};
use ytm_core::{PlayState, Rating, Row, Track, VideoId};
use ytm_player::engine::Command as PCmd;
use ytm_player::{PlayerHandle, Tap, CHANNELS, SAMPLE_RATE};
use ytm_viz::{Analyser, SpectrumFrame};

use crate::nav::{sidebar, Dest, Page, View};
use crate::spectrum::VizStyle;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Content,
    Queue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
}

/// Rating and library state for the playing track, fetched from the account so
/// the UI shows what is actually true rather than a guess (FR-R7).
#[derive(Clone, Default)]
pub struct TrackState {
    pub id: Option<VideoId>,
    pub rating: Rating,
    pub in_library: bool,
    pub token_add: Option<String>,
    pub token_remove: Option<String>,
}

pub enum AppEvent {
    /// Content for a view, tagged with the request that asked for it.
    Loaded { generation: u64, append: bool, result: Result<ytm_api::RowPage, String> },
    Radio(Result<Vec<Track>, String>),
    RadioFrom(Track, Result<Vec<Track>, String>),
    TrackState(VideoId, TrackState),
    Toast(String),
}

pub struct App {
    pub backend: Arc<dyn MusicBackend>,
    pub player: PlayerHandle,
    pub theme: Theme,

    pub mode: Mode,
    pub focus: Focus,
    pub query: String,
    /// Navigation stack; the last entry is what is on screen.
    pub stack: Vec<Page>,
    pub sidebar: Vec<Dest>,
    pub sidebar_sel: usize,
    pub queue_sel: usize,

    pub viz_style: VizStyle,
    pub viz_fullscreen: bool,
    pub show_help: bool,
    pub spectrum: Arc<ArcSwap<SpectrumFrame>>,
    pub n_bands: Arc<AtomicU64>,

    pub now: TrackState,
    pub toast: Option<(String, Instant)>,
    pub should_quit: bool,
    pub autoplay: bool,

    prev_state: PlayState,
    prev_track: Option<VideoId>,
    radio_pending: bool,
    generation: u64,
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

        let authed = backend.is_authenticated();
        let bar = sidebar(authed);
        let first = bar.iter().position(|d| d.is_selectable()).unwrap_or(0);

        let mut app = Self {
            backend,
            player,
            theme: Theme::default(),
            mode: Mode::Normal,
            focus: Focus::Content,
            query: String::new(),
            stack: Vec::new(),
            sidebar: bar,
            sidebar_sel: first,
            queue_sel: 0,
            viz_style: VizStyle::Mirrored,
            viz_fullscreen: false,
            show_help: false,
            spectrum,
            n_bands,
            now: TrackState::default(),
            toast: None,
            should_quit: false,
            autoplay: true,
            prev_state: PlayState::Stopped,
            prev_track: None,
            radio_pending: false,
            generation: 0,
            tx,
            rx,
            quit_flag,
        };
        app.go(View::Home);
        if !authed {
            app.toast("anonymous mode - account features disabled".into());
        }
        app
    }

    pub fn page(&self) -> Option<&Page> {
        self.stack.last()
    }
    fn page_mut(&mut self) -> Option<&mut Page> {
        self.stack.last_mut()
    }

    pub fn toast(&mut self, msg: String) {
        self.toast = Some((msg, Instant::now()));
    }

    // ---- navigation --------------------------------------------------------

    /// Replace the stack with a top-level destination.
    fn go(&mut self, view: View) {
        self.stack.clear();
        self.push(view);
    }

    /// Descend into a view, keeping the current one to come back to.
    fn push(&mut self, view: View) {
        self.generation += 1;
        let gen = self.generation;
        self.stack.push(Page::new(view.clone(), gen));
        self.focus = Focus::Content;
        self.fetch(view, gen);
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Kick off the network fetch for a view, on a worker thread.
    fn fetch(&self, view: View, generation: u64) {
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = match &view {
                View::Home => b.home(),
                View::Explore(s) => b.explore(*s),
                View::Library(s) => b.library(*s),
                View::Search { query, filter } => b.search(query, *filter),
                View::Artist(id, _) => b.artist(id),
                View::Album(id, _) => b.album(id),
                View::Playlist(id, _) => b.playlist(id),
            };
            let _ = tx.send(AppEvent::Loaded {
                generation,
                append: false,
                result: result.map_err(|e| e.to_string()),
            });
        });
    }

    /// Pull the next page in when the cursor nears the end of the list (FR-B4).
    fn maybe_load_more(&mut self) {
        let Some(p) = self.stack.last_mut() else { return };
        if p.loading || p.loading_more || p.rows.is_empty() {
            return;
        }
        let Some(token) = p.continuation.clone() else { return };
        if p.sel + 12 < p.rows.len() {
            return;
        }
        p.loading_more = true;
        let generation = p.generation;
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b.continue_rows(&token).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Loaded { generation, append: true, result: r });
        });
    }

    /// Open whatever the cursor is on.
    fn activate(&mut self) {
        let Some(page) = self.stack.last() else { return };
        let Some(row) = page.selected().cloned() else { return };
        match row {
            Row::Header(_) => {}
            Row::Track(_) => {
                if let Some((tracks, index)) = page.tracks_from(page.sel) {
                    self.player.send(PCmd::PlayQueue { tracks, index });
                    self.queue_sel = index;
                }
            }
            Row::Album(a) => self.push(View::Album(a.id, a.title)),
            Row::Artist(a) => self.push(View::Artist(a.id, a.name)),
            Row::Playlist(p) => self.push(View::Playlist(p.id, p.title)),
        }
    }

    /// "Go to artist" / "go to album" from the selected or playing track (FR-B5).
    fn goto_related(&mut self, artist: bool) {
        let track = self
            .selected_track()
            .or_else(|| self.player.status().current.clone());
        let Some(t) = track else {
            self.toast("no track selected".into());
            return;
        };
        let target = if artist { t.artist_id.clone() } else { t.album_id.clone() };
        match target {
            Some(id) if artist => self.push(View::Artist(id, t.artist.clone())),
            Some(id) => self.push(View::Album(id, t.album.clone().unwrap_or_else(|| t.title.clone()))),
            None => self.toast(
                if artist { "no artist link on this track" } else { "no album link on this track" }
                    .into(),
            ),
        }
    }

    // ---- data --------------------------------------------------------------

    pub fn tick(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AppEvent::Loaded { generation, append, result } => {
                    // Ignore anything a newer navigation has superseded.
                    let Some(p) = self.stack.iter_mut().find(|p| p.generation == generation) else {
                        continue;
                    };
                    if append {
                        p.loading_more = false;
                    } else {
                        p.loading = false;
                    }
                    match result {
                        Ok(page) => {
                            p.continuation = page.continuation;
                            if append {
                                p.rows.extend(page.rows);
                            } else {
                                p.rows = page.rows;
                                p.sel = 0;
                                p.snap_to_selectable();
                                if let Some(t) = page.title.filter(|t| !t.trim().is_empty()) {
                                    match &mut p.view {
                                        View::Artist(_, n)
                                        | View::Album(_, n)
                                        | View::Playlist(_, n) => *n = t,
                                        _ => {}
                                    }
                                }
                                if p.rows.is_empty() {
                                    p.error = Some("nothing here".into());
                                }
                            }
                        }
                        Err(e) => {
                            if !append {
                                p.error = Some(e);
                            }
                        }
                    }
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
                AppEvent::TrackState(id, st) => {
                    // Only apply if it is still the playing track.
                    if self.player.status().current.as_ref().map(|t| &t.id) == Some(&id) {
                        self.now = st;
                    }
                }
                AppEvent::Toast(m) => self.toast(m),
            }
        }
        self.maybe_autoplay();
        self.refresh_track_state();
        self.maybe_load_more();
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            }
        }
    }

    /// Fetch the true rating/library state whenever the playing track changes.
    fn refresh_track_state(&mut self) {
        let cur = self.player.status().current.as_ref().map(|t| t.id.clone());
        if cur == self.prev_track {
            return;
        }
        self.prev_track = cur.clone();
        self.now = TrackState { id: cur.clone(), ..Default::default() };
        let Some(id) = cur else { return };
        if !self.backend.is_authenticated() {
            return;
        }
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Ok((rating, add, remove, in_library)) = b.track_state(&id) {
                let _ = tx.send(AppEvent::TrackState(
                    id.clone(),
                    TrackState { id: Some(id), rating, in_library, token_add: add, token_remove: remove },
                ));
            }
        });
    }

    fn maybe_autoplay(&mut self) {
        let st = self.player.status();
        let stopped_just_now = self.prev_state == PlayState::Playing && st.state == PlayState::Stopped;
        self.prev_state = st.state;
        if !stopped_just_now || !self.autoplay || self.radio_pending {
            return;
        }
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

    fn start_radio_from_selection(&mut self) {
        let Some(seed) = self.selected_track().or_else(|| self.player.status().current.clone())
        else {
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

    // ---- account actions ---------------------------------------------------

    fn rate_current(&mut self, want: Rating) {
        let Some(track) = self.player.status().current.clone() else {
            self.toast("nothing playing".into());
            return;
        };
        if !self.backend.is_authenticated() {
            self.toast("not signed in - ratings unavailable".into());
            return;
        }
        let new = if want == Rating::Like {
            self.now.rating.toggled_like()
        } else {
            self.now.rating.toggled_dislike()
        };
        // Optimistic: the UI reflects the intent immediately and is corrected
        // if the request fails.
        let previous = self.now.rating;
        self.now.rating = new;

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
                Err(e) => format!("rating failed ({e}) - was {previous:?}"),
            };
            let _ = tx.send(AppEvent::Toast(msg));
        });
        if new == Rating::Dislike {
            self.player.send(PCmd::Next);
        }
    }

    /// Add to / remove from the library. Deliberately separate from thumbs-up:
    /// liking a song puts it in Liked Songs, not in the library (FR-R4).
    fn toggle_library(&mut self) {
        if !self.backend.is_authenticated() {
            self.toast("not signed in - library unavailable".into());
            return;
        }
        let Some(track) = self.player.status().current.clone() else {
            self.toast("nothing playing".into());
            return;
        };
        let token = if self.now.in_library {
            self.now.token_remove.clone()
        } else {
            self.now.token_add.clone()
        };
        let Some(token) = token else {
            self.toast("no library token for this track".into());
            return;
        };
        let adding = !self.now.in_library;
        self.now.in_library = adding;
        let b = self.backend.clone();
        let tx = self.tx.clone();
        let title = track.title.clone();
        std::thread::spawn(move || {
            let msg = match b.set_library(&token) {
                Ok(()) if adding => format!("added to library: {title}"),
                Ok(()) => format!("removed from library: {title}"),
                Err(e) => format!("library update failed: {e}"),
            };
            let _ = tx.send(AppEvent::Toast(msg));
        });
    }

    // ---- selection ---------------------------------------------------------

    pub fn selected_track(&self) -> Option<Track> {
        match self.focus {
            Focus::Queue => self.player.status().queue.get(self.queue_sel).cloned(),
            _ => self.page()?.selected().and_then(|r| r.as_track()).cloned(),
        }
    }

    fn move_sel(&mut self, delta: isize) {
        match self.focus {
            Focus::Sidebar => {
                let n = self.sidebar.len() as isize;
                let step = if delta >= 0 { 1 } else { -1 };
                let mut i = self.sidebar_sel as isize;
                for _ in 0..delta.abs() {
                    let mut probe = i;
                    loop {
                        probe += step;
                        if probe < 0 || probe >= n {
                            break;
                        }
                        if self.sidebar[probe as usize].is_selectable() {
                            i = probe;
                            break;
                        }
                    }
                }
                self.sidebar_sel = i.clamp(0, n - 1) as usize;
            }
            Focus::Content => {
                if let Some(p) = self.page_mut() {
                    p.move_sel(delta);
                }
            }
            Focus::Queue => {
                let len = self.player.status().queue.len();
                if len > 0 {
                    let n = (self.queue_sel as isize + delta).clamp(0, len as isize - 1);
                    self.queue_sel = n as usize;
                }
            }
        }
    }

    fn run_search(&mut self, filter: SearchFilter) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.go(View::Search { query: q, filter });
    }

    fn cycle_search_tab(&mut self, forward: bool) {
        let Some(page) = self.page() else { return };
        let View::Search { query, filter } = &page.view else {
            self.toast("tabs apply to search results".into());
            return;
        };
        let all = SearchFilter::ALL;
        let i = all.iter().position(|f| f == filter).unwrap_or(0);
        let next = if forward { (i + 1) % all.len() } else { (i + all.len() - 1) % all.len() };
        let view = View::Search { query: query.clone(), filter: all[next] };
        self.stack.pop();
        self.push(view);
    }

    // ---- input -------------------------------------------------------------

    pub fn handle_key(&mut self, k: KeyEvent) {
        if self.mode == Mode::Search {
            match k.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    self.run_search(SearchFilter::Songs);
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
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Content,
                    Focus::Content => Focus::Queue,
                    Focus::Queue => Focus::Sidebar,
                }
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Queue,
                    Focus::Content => Focus::Sidebar,
                    Focus::Queue => Focus::Content,
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Home => self.move_sel(-9999),
            KeyCode::End => self.move_sel(9999),

            KeyCode::Enter => match self.focus {
                Focus::Sidebar => {
                    if let Some(Dest::Go(v)) = self.sidebar.get(self.sidebar_sel).cloned() {
                        self.go(v);
                    }
                }
                Focus::Content => self.activate(),
                Focus::Queue => self.player.send(PCmd::JumpTo(self.queue_sel)),
            },
            KeyCode::Esc | KeyCode::Backspace => {
                if self.viz_fullscreen {
                    self.viz_fullscreen = false;
                } else {
                    self.back();
                }
            }
            KeyCode::Char('[') => self.cycle_search_tab(false),
            KeyCode::Char(']') => self.cycle_search_tab(true),
            KeyCode::Char('g') => self.goto_related(true),
            KeyCode::Char('G') => self.goto_related(false),

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
            KeyCode::Char('s') => self.player.send(PCmd::ToggleShuffle),
            KeyCode::Char('r') => self.player.send(PCmd::CycleRepeat),
            KeyCode::Char('R') => self.start_radio_from_selection(),
            KeyCode::Char('A') => {
                self.autoplay = !self.autoplay;
                let on = self.autoplay;
                self.toast(format!("autoplay {}", if on { "on" } else { "off" }));
            }
            KeyCode::Char('0') => {
                let v = (self.player.status().volume + 0.05).min(1.5);
                self.player.send(PCmd::SetVolume(v));
            }
            KeyCode::Char('9') => {
                let v = (self.player.status().volume - 0.05).max(0.0);
                self.player.send(PCmd::SetVolume(v));
            }

            KeyCode::Char('+') | KeyCode::Char('l') => self.rate_current(Rating::Like),
            KeyCode::Char('-') | KeyCode::Char('d') => self.rate_current(Rating::Dislike),
            KeyCode::Char('a') => self.toggle_library(),

            KeyCode::Char('v') => self.viz_style = self.viz_style.next(),
            KeyCode::Char('z') => self.viz_fullscreen = !self.viz_fullscreen,
            _ => {}
        }
    }

    pub fn poll_input(&mut self, timeout: Duration) -> Result<bool> {
        if event::poll(timeout)? {
            if let Event::Key(k) = event::read()? {
                if k.is_press() {
                    self.handle_key(k);
                }
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
                }
                out.store(Arc::new(an.analyse(dt)));
                std::thread::sleep(Duration::from_millis(16));
            }
        })
        .expect("spawn analyser");
}
