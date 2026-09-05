//! Application state and the event loop.
//!
//! The UI thread never blocks on I/O: every fetch runs on a worker thread and
//! reports back by channel, the player is driven by messages, and the analyser
//! publishes frames the renderer samples at its own pace.

use anyhow::Result;
use arc_swap::ArcSwap;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ytm_api::{MusicBackend, SearchFilter};
use ytm_config::{Action, Chord, Config};
use ytm_core::{PlayState, Rating, Row, Track, VideoId};
use ytm_player::engine::Command as PCmd;
use ytm_player::{PlayerHandle, Tap, CHANNELS, SAMPLE_RATE};
use ytm_viz::{Analyser, SpectrumFrame};

use crate::keymap::chord_of;
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
    Lyrics(VideoId, Result<Option<String>, String>),
    Toast(String),
}

pub struct App {
    pub backend: Arc<dyn MusicBackend>,
    pub player: PlayerHandle,
    pub theme: Theme,
    pub config: Config,
    pub keymap: HashMap<Chord, Action>,

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
    pub show_lyrics: bool,
    /// Lyrics for the track they belong to, so a stale fetch is never shown
    /// against the wrong song.
    pub lyrics: Option<(VideoId, Option<String>)>,
    pub lyrics_scroll: u16,
    pub lyrics_loading: bool,
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
    pub fn new(
        backend: Arc<dyn MusicBackend>,
        player: PlayerHandle,
        tap: Tap,
        loaded: ytm_config::Loaded,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let spectrum = Arc::new(ArcSwap::from_pointee(SpectrumFrame::default()));
        let n_bands = Arc::new(AtomicU64::new(96));
        let quit_flag = Arc::new(AtomicBool::new(false));
        spawn_analyser(tap, spectrum.clone(), n_bands.clone(), quit_flag.clone());

        let authed = backend.is_authenticated();
        let bar = sidebar(authed);
        let first = bar.iter().position(|d| d.is_selectable()).unwrap_or(0);

        let cfg = loaded.config;
        let style = match cfg.visualizer.style.as_str() {
            "bars" => VizStyle::Bars,
            "scope" => VizStyle::Scope,
            _ => VizStyle::Mirrored,
        };
        let mut app = Self {
            backend,
            player,
            theme: Theme::from_config(&cfg),
            keymap: loaded.keymap,
            mode: Mode::Normal,
            focus: Focus::Content,
            query: String::new(),
            stack: Vec::new(),
            sidebar: bar,
            sidebar_sel: first,
            queue_sel: 0,
            viz_style: style,
            viz_fullscreen: false,
            show_help: false,
            show_lyrics: false,
            lyrics: None,
            lyrics_scroll: 0,
            lyrics_loading: false,
            spectrum,
            n_bands,
            now: TrackState::default(),
            toast: None,
            should_quit: false,
            autoplay: cfg.general.autoplay,
            config: cfg,
            prev_state: PlayState::Stopped,
            prev_track: None,
            radio_pending: false,
            generation: 0,
            tx,
            rx,
            quit_flag,
        };
        app.go(View::Home);
        app.restore_session();
        for w in loaded.warnings {
            app.toast(w);
        }
        if !authed {
            app.toast("anonymous mode - account features disabled".into());
        }
        app
    }

    /// Put back the queue and position from the previous run, paused (FR-C5).
    fn restore_session(&mut self) {
        if !self.config.general.restore_session {
            return;
        }
        let Some(s) = crate::session::load() else { return };
        if s.queue.is_empty() {
            return;
        }
        let n = s.queue.len();
        self.player.send(PCmd::SetVolume(if s.volume > 0.0 { s.volume } else { 1.0 }));
        self.player.send(PCmd::RestoreQueue {
            tracks: s.queue,
            index: s.index,
            position: s.position,
        });
        self.toast(format!("restored {n} tracks (paused)"));
    }

    fn save_session(&self) {
        if !self.config.general.restore_session {
            return;
        }
        let st = self.player.status();
        crate::session::save(&crate::session::Session {
            queue: st.queue.clone(),
            index: st.queue_index,
            position: st.position.as_secs_f64(),
            volume: st.volume,
            saved_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
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
                AppEvent::Lyrics(id, r) => {
                    self.lyrics_loading = false;
                    match r {
                        Ok(text) => self.lyrics = Some((id, text)),
                        Err(e) => {
                            self.lyrics = Some((id, None));
                            self.toast(format!("lyrics failed: {e}"));
                        }
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
        self.lyrics = None;
        self.lyrics_scroll = 0;
        if self.show_lyrics {
            self.fetch_lyrics();
        }
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

    /// Fetch lyrics for whatever is playing, if the panel is open and we do
    /// not already have them for that exact track.
    fn fetch_lyrics(&mut self) {
        let Some(track) = self.player.status().current.clone() else {
            self.lyrics = None;
            return;
        };
        if self.lyrics.as_ref().map(|(id, _)| id) == Some(&track.id) || self.lyrics_loading {
            return;
        }
        self.lyrics_loading = true;
        self.lyrics_scroll = 0;
        let b = self.backend.clone();
        let tx = self.tx.clone();
        let id = track.id.clone();
        std::thread::spawn(move || {
            let r = b.lyrics(&id).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Lyrics(id, r));
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
            Focus::Queue if self.show_lyrics => {
                let n = self.lyrics_scroll as isize + delta;
                self.lyrics_scroll = n.max(0) as u16;
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
        // Text entry swallows keys before the keymap sees them, otherwise typing
        // a query would trigger bindings.
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
        let Some(chord) = chord_of(k) else { return };
        let Some(action) = self.keymap.get(&chord).copied() else { return };
        self.do_action(action);
    }

    /// Every binding, and later the command palette, funnels through here.
    pub fn do_action(&mut self, action: Action) {
        use Action::*;
        match action {
            Quit => self.should_quit = true,
            Help => self.show_help = true,
            Search => {
                self.mode = Mode::Search;
                self.query.clear();
            }
            CommandPalette => self.toast("command palette: not yet".into()),
            NextPane => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Content,
                    Focus::Content => Focus::Queue,
                    Focus::Queue => Focus::Sidebar,
                }
            }
            PrevPane => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Queue,
                    Focus::Content => Focus::Sidebar,
                    Focus::Queue => Focus::Content,
                }
            }
            Down => self.move_sel(1),
            Up => self.move_sel(-1),
            PageDown => self.move_sel(10),
            PageUp => self.move_sel(-10),
            Top => self.move_sel(-9999),
            Bottom => self.move_sel(9999),
            Activate => match self.focus {
                Focus::Sidebar => {
                    if let Some(Dest::Go(v)) = self.sidebar.get(self.sidebar_sel).cloned() {
                        self.go(v);
                    }
                }
                Focus::Content => self.activate(),
                Focus::Queue => self.player.send(PCmd::JumpTo(self.queue_sel)),
            },
            Back => {
                if self.viz_fullscreen {
                    self.viz_fullscreen = false;
                } else {
                    self.back();
                }
            }
            NextTab => self.cycle_search_tab(true),
            PrevTab => self.cycle_search_tab(false),
            GoToArtist => self.goto_related(true),
            GoToAlbum => self.goto_related(false),

            TogglePause => self.player.send(PCmd::TogglePause),
            Next => self.player.send(PCmd::Next),
            Prev => self.player.send(PCmd::Prev),
            SeekForward => self.player.send(PCmd::SeekRelative(5.0)),
            SeekBackward => self.player.send(PCmd::SeekRelative(-5.0)),
            SeekForwardLong => self.player.send(PCmd::SeekRelative(30.0)),
            SeekBackwardLong => self.player.send(PCmd::SeekRelative(-30.0)),
            VolumeUp => {
                let v = (self.player.status().volume + 0.05).min(1.5);
                self.player.send(PCmd::SetVolume(v));
            }
            VolumeDown => {
                let v = (self.player.status().volume - 0.05).max(0.0);
                self.player.send(PCmd::SetVolume(v));
            }
            ToggleShuffle => self.player.send(PCmd::ToggleShuffle),
            CycleRepeat => self.player.send(PCmd::CycleRepeat),
            StartRadio => self.start_radio_from_selection(),
            ToggleAutoplay => {
                self.autoplay = !self.autoplay;
                let on = self.autoplay;
                self.toast(format!("autoplay {}", if on { "on" } else { "off" }));
            }

            PlayNext => {
                if let Some(t) = self.selected_track() {
                    let title = t.title.clone();
                    self.player.send(PCmd::PlayNext(t));
                    self.toast(format!("playing next: {title}"));
                }
            }
            Enqueue => {
                if let Some(t) = self.selected_track() {
                    let title = t.title.clone();
                    self.player.send(PCmd::Enqueue(t));
                    self.toast(format!("queued: {title}"));
                }
            }
            RemoveFromQueue => {
                if self.focus == Focus::Queue {
                    self.player.send(PCmd::RemoveAt(self.queue_sel));
                }
            }

            ThumbsUp => self.rate_current(Rating::Like),
            ThumbsDown => self.rate_current(Rating::Dislike),
            ToggleLibrary => self.toggle_library(),
            AddToPlaylist => self.toast("add to playlist: not yet".into()),
            ToggleSubscribe => self.toast("subscribe: not yet".into()),
            CopyLink => self.copy_link(),

            CycleVisualizer => self.viz_style = self.viz_style.next(),
            ToggleVisualizerFullscreen => self.viz_fullscreen = !self.viz_fullscreen,
            ToggleLyrics => {
                self.show_lyrics = !self.show_lyrics;
                if self.show_lyrics {
                    self.fetch_lyrics();
                }
            }
        }
    }

    /// Put a share link on the clipboard, falling back to showing it if no
    /// clipboard tool is available (headless, or over plain SSH).
    fn copy_link(&mut self) {
        let Some(t) = self.selected_track().or_else(|| self.player.status().current.clone()) else {
            self.toast("nothing selected".into());
            return;
        };
        let url = format!("https://music.youtube.com/watch?v={}", t.id);
        match crate::clipboard::copy(&url) {
            Ok(via) => self.toast(format!("link copied ({via})")),
            Err(_) => self.toast(url),
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
        self.save_session();
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
