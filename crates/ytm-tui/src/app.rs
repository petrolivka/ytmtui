//! Application state and the event loop.
//!
//! The UI thread never blocks on I/O: every fetch runs on a worker thread and
//! reports back by channel, the player is driven by messages, and the analyser
//! publishes frames the renderer samples at its own pace.

use anyhow::Result;
use arc_swap::ArcSwap;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use ytm_api::{MusicBackend, SearchFilter};
use ytm_config::{Action, Chord, Config};
use ytm_core::{PlayState, PlaylistRef, Rating, Row, Track, VideoId};
use ytm_player::engine::Command as PCmd;
use ytm_player::{PlayerHandle, Tap, CHANNELS, SAMPLE_RATE};
use ytm_viz::{Analyser, SpectrumFrame};

use crate::keymap::chord_of;
use crate::modal::{filter_actions, Confirm, Modal, Prompt};
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
    Loaded {
        generation: u64,
        append: bool,
        result: Result<ytm_api::RowPage, String>,
    },
    Radio(Result<Vec<Track>, String>),
    RadioFrom(Track, Result<Vec<Track>, String>),
    TrackState(VideoId, TrackState),
    Lyrics(VideoId, Result<Option<String>, String>),
    SyncedLyrics(VideoId, Vec<ytm_api::lrclib::Line>),
    Playlists(Result<Vec<PlaylistRef>, String>),
    Suggestions(String, Vec<String>),
    ArtReady,
    /// A write finished; refresh whatever view is showing.
    Wrote(String, bool),
    Toast(String),
}

/// Where the panes ended up on the last frame, so a click can be mapped back
/// to what is under it. Filled in by the renderer.
#[derive(Default)]
pub struct Hitboxes {
    pub sidebar: Cell<Rect>,
    pub content: Cell<Rect>,
    pub queue: Cell<Rect>,
    pub progress: Cell<Rect>,
    /// Inner area of the cover pane, for fetching at the right size and for
    /// positioning graphics escapes.
    pub cover: Cell<Rect>,
    /// Inner area of the visualiser pane. The fire needs it to size its
    /// simulation, and to position its own graphics escapes.
    pub viz: Cell<Rect>,
    /// Where the fire was last painted through a graphics protocol, so it can
    /// be wiped when the style changes. Ratatui will not do it: those cells
    /// were marked skipped, so its diff has no record of anything being there.
    pub viz_painted: Cell<Option<Rect>>,
    /// What the graphics backend last painted, so an unchanged image is not
    /// re-encoded and re-sent every frame. At 60 fps that is a lot of bytes
    /// down a terminal, and it flickers over a slow link.
    pub painted: RefCell<Option<(String, Rect)>>,
}

/// List scroll offsets, owned here rather than rebuilt per frame, so a click
/// can be mapped to the row actually under the pointer. Estimating the offset
/// instead picks the wrong row as soon as a list is scrolled.
#[derive(Default)]
pub struct ListStates {
    pub content: RefCell<ratatui::widgets::ListState>,
    pub queue: RefCell<ratatui::widgets::ListState>,
}

pub struct App {
    pub hit: Hitboxes,
    pub lists: ListStates,
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
    pub show_art: bool,
    pub art_backend: ytm_art::Backend,
    pub art_cache: Arc<ytm_art::ArtCache>,
    /// Rendered cells for the current cover, keyed by the size they were made
    /// for so a resize re-renders rather than stretching.
    pub art_cells: Vec<Vec<ytm_art::Cell>>,
    pub art_for: Option<(String, u16, u16)>,
    art_fetching: bool,
    pub modal: Option<Modal>,
    pub search_history: Vec<String>,
    pub suggestions: Vec<String>,
    /// When the query last changed, for debouncing (FR-S2).
    suggest_after: Option<Instant>,
    suggest_for: String,
    /// Last-seen config mtime, for hot-reload (FR-C1).
    config_mtime: Option<std::time::SystemTime>,
    config_checked: Instant,
    scrobbler: Option<Arc<ytm_api::listenbrainz::Scrobbler>>,
    /// Which track the pending scrobble is for, when it started, and whether
    /// the listen has been submitted yet.
    scrobble: Option<(VideoId, u64, bool)>,
    history_pos: Option<usize>,
    /// Lyrics for the track they belong to, so a stale fetch is never shown
    /// against the wrong song.
    pub lyrics: Option<(VideoId, Option<String>)>,
    /// Time-synced lyrics, when a provider has them for this track.
    pub lyrics_synced: Option<(VideoId, Vec<ytm_api::lrclib::Line>)>,
    pub lyrics_scroll: u16,
    pub lyrics_loading: bool,
    pub spectrum: Arc<ArcSwap<SpectrumFrame>>,
    /// Rolling band history for the spectrogram, newest last.
    pub history: std::collections::VecDeque<Vec<f32>>,
    /// Rolling pitch-class history for the chroma strip, newest last.
    pub chroma: std::collections::VecDeque<[f32; ytm_viz::N_CHROMA]>,
    /// The pixel visualisers, live only while one of those styles is on.
    pub pixels: crate::pixel::Pixels,
    /// The current pixel frame as half-block cells, for terminals without a
    /// graphics protocol. Empty when the graphics path is drawing it instead.
    pub pixel_cells: Vec<Vec<ytm_art::Cell>>,
    /// Fades after an onset, so the accent pulses rather than flickers.
    pub beat_glow: f32,
    last_seq: u64,
    /// When the last pixel frame was rendered. The ink advects by time rather
    /// than by frame, so it flows at the same speed whatever the frame rate.
    pixel_at: Instant,
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
        let style = VizStyle::parse(&cfg.visualizer.style);
        ytm_art::set_cell_aspect(cfg.art.cell_aspect);
        let mut app = Self {
            hit: Hitboxes::default(),
            lists: ListStates::default(),
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
            show_art: cfg.art.enabled,
            art_backend: ytm_art::Backend::parse(&cfg.art.backend)
                .unwrap_or_else(ytm_art::Backend::detect),
            art_cache: Arc::new(
                ytm_art::ArtCache::new().unwrap_or_else(|_| unreachable!("client builder")),
            ),
            art_cells: Vec::new(),
            art_for: None,
            art_fetching: false,
            modal: None,
            search_history: crate::session::load_search_history(),
            suggestions: Vec::new(),
            suggest_after: None,
            suggest_for: String::new(),
            config_mtime: config_mtime(),
            config_checked: Instant::now(),
            scrobbler: None,
            scrobble: None,
            history_pos: None,
            lyrics: None,
            lyrics_synced: None,
            lyrics_scroll: 0,
            lyrics_loading: false,
            history: std::collections::VecDeque::with_capacity(512),
            chroma: std::collections::VecDeque::with_capacity(512),
            pixels: crate::pixel::Pixels::new(),
            pixel_cells: Vec::new(),
            beat_glow: 0.0,
            last_seq: 0,
            pixel_at: Instant::now(),
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
        let Some(s) = crate::session::load() else {
            return;
        };
        if s.queue.is_empty() {
            return;
        }
        let n = s.queue.len();
        self.player
            .send(PCmd::SetVolume(if s.volume > 0.0 { s.volume } else { 1.0 }));
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
            queue: st.queue.as_ref().clone(),
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
        let Some(p) = self.stack.last_mut() else {
            return;
        };
        if p.loading || p.loading_more || p.rows.is_empty() {
            return;
        }
        let Some(token) = p.continuation.clone() else {
            return;
        };
        if p.sel + 12 < p.rows.len() {
            return;
        }
        p.loading_more = true;
        let generation = p.generation;
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b.continue_rows(&token).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Loaded {
                generation,
                append: true,
                result: r,
            });
        });
    }

    /// Open whatever the cursor is on.
    fn activate(&mut self) {
        let Some(page) = self.stack.last() else {
            return;
        };
        let Some(row) = page.selected().cloned() else {
            return;
        };
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
        let target = if artist {
            t.artist_id.clone()
        } else {
            t.album_id.clone()
        };
        match target {
            Some(id) if artist => self.push(View::Artist(id, t.artist.clone())),
            Some(id) => self.push(View::Album(
                id,
                t.album.clone().unwrap_or_else(|| t.title.clone()),
            )),
            None => self.toast(
                if artist {
                    "no artist link on this track"
                } else {
                    "no album link on this track"
                }
                .into(),
            ),
        }
    }

    // ---- data --------------------------------------------------------------

    pub fn tick(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AppEvent::Loaded {
                    generation,
                    append,
                    result,
                } => {
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
                            self.player.send(PCmd::PlayQueue {
                                tracks: q,
                                index: 0,
                            });
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
                AppEvent::Playlists(r) => {
                    if let Some(Modal::PlaylistPicker {
                        playlists, loading, ..
                    }) = &mut self.modal
                    {
                        *loading = false;
                        match r {
                            Ok(p) => *playlists = p,
                            Err(e) => {
                                self.modal = None;
                                self.toast(format!("playlists failed: {e}"));
                            }
                        }
                    }
                }
                AppEvent::Wrote(msg, reload) => {
                    self.toast(msg);
                    if reload {
                        self.reload_current();
                    }
                }
                AppEvent::Suggestions(q, list) => {
                    // Discard anything that arrived for an older query.
                    if q == self.query.trim() {
                        self.suggestions = list;
                    }
                }
                AppEvent::SyncedLyrics(id, lines) => {
                    if self.player.status().current.as_ref().map(|t| &t.id) == Some(&id) {
                        self.lyrics_synced = Some((id, lines));
                    }
                }
                AppEvent::ArtReady => self.art_fetching = false,
                AppEvent::Toast(m) => self.toast(m),
            }
        }
        let c = self.hit.cover.get();
        self.ensure_art(c.width, c.height);
        self.sample_spectrum();
        self.step_pixels();
        self.poll_scrobble();
        self.poll_config();
        self.poll_suggestions();
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
        self.now = TrackState {
            id: cur.clone(),
            ..Default::default()
        };
        self.lyrics = None;
        self.lyrics_synced = None;
        self.lyrics_scroll = 0;
        self.notify_track_change();
        self.start_scrobble();
        self.art_cells.clear();
        self.art_for = None;
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
                    TrackState {
                        id: Some(id),
                        rating,
                        in_library,
                        token_add: add,
                        token_remove: remove,
                    },
                ));
            }
        });
    }

    /// Tell ListenBrainz what is on, and arm the listen for later.
    fn start_scrobble(&mut self) {
        let Some(sc) = self.scrobbler.clone() else {
            return;
        };
        let Some(t) = self.player.status().current.clone() else {
            self.scrobble = None;
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.scrobble = Some((t.id.clone(), now, false));
        std::thread::spawn(move || {
            if let Err(e) = sc.playing_now(&t.artist, &t.title, t.album.as_deref()) {
                tracing::warn!("playing_now failed: {e}");
            }
        });
    }

    /// Submit the listen once enough of the track has played.
    fn poll_scrobble(&mut self) {
        let Some(sc) = self.scrobbler.clone() else {
            return;
        };
        let st = self.player.status();
        let Some(t) = st.current.clone() else { return };
        let Some((id, started, submitted)) = self.scrobble.clone() else {
            return;
        };
        if submitted || id != t.id {
            return;
        }
        if !ytm_api::listenbrainz::should_submit(st.position, t.duration) {
            return;
        }
        self.scrobble = Some((id, started, true));
        std::thread::spawn(move || {
            if let Err(e) = sc.listen(&t.artist, &t.title, t.album.as_deref(), started) {
                tracing::warn!("scrobble failed: {e}");
            } else {
                tracing::info!("scrobbled {} - {}", t.artist, t.title);
            }
        });
    }

    fn notify_track_change(&self) {
        if !self.config.general.notifications {
            return;
        }
        let Some(t) = self.player.status().current.clone() else {
            return;
        };
        // Off the UI thread: spawning a notifier can block for a moment.
        std::thread::spawn(move || {
            let body = if t.album.is_some() {
                format!(
                    "{}  \u{2022}  {}",
                    t.artist,
                    t.album.clone().unwrap_or_default()
                )
            } else {
                t.artist.clone()
            };
            crate::notify::track_changed(&t.title, &body);
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
        let t = track.clone();
        std::thread::spawn(move || {
            let r = b.lyrics(&id).map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Lyrics(id.clone(), r));
            // Synced lyrics come from a different provider and often miss;
            // they are an upgrade on the plain ones, never a prerequisite.
            if let Ok(Some(lines)) = b.synced_lyrics(&t) {
                let _ = tx.send(AppEvent::SyncedLyrics(id, lines));
            }
        });
    }

    fn maybe_autoplay(&mut self) {
        let st = self.player.status();
        let stopped_just_now =
            self.prev_state == PlayState::Playing && st.state == PlayState::Stopped;
        self.prev_state = st.state;
        if !stopped_just_now || !self.autoplay || self.radio_pending {
            return;
        }
        if st.queue.is_empty() || st.queue_index + 1 < st.queue.len() {
            return;
        }
        let Some(seed) = st.queue.get(st.queue_index).cloned() else {
            return;
        };
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
        let Some(seed) = self
            .selected_track()
            .or_else(|| self.player.status().current.clone())
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
        self.history_pos = None;
        self.search_history.retain(|h| h != &q);
        self.search_history.insert(0, q.clone());
        self.search_history.truncate(50);
        crate::session::save_search_history(&self.search_history);
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
        let next = if forward {
            (i + 1) % all.len()
        } else {
            (i + all.len() - 1) % all.len()
        };
        let view = View::Search {
            query: query.clone(),
            filter: all[next],
        };
        self.stack.pop();
        self.push(view);
    }

    // ---- input -------------------------------------------------------------

    pub fn handle_key(&mut self, k: KeyEvent) {
        // Overlays own the keyboard while they are up.
        if self.modal.is_some() {
            self.modal_key(k);
            return;
        }
        // Text entry swallows keys before the keymap sees them, otherwise typing
        // a query would trigger bindings.
        if self.mode == Mode::Search {
            match k.code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.suggestions.clear();
                }
                // Accept the first suggestion rather than the raw text.
                KeyCode::Tab => {
                    if let Some(first) = self.suggestions.first().cloned() {
                        self.query = first;
                        self.suggestions.clear();
                    }
                }
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    self.suggestions.clear();
                    self.run_search(SearchFilter::Songs);
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.schedule_suggestions();
                }
                // Recall previous searches (FR-S3).
                KeyCode::Up => self.recall_search(1),
                KeyCode::Down => self.recall_search(-1),
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.history_pos = None;
                    self.schedule_suggestions();
                }
                _ => {}
            }
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        let Some(chord) = chord_of(k) else { return };
        let Some(action) = self.keymap.get(&chord).copied() else {
            return;
        };
        self.do_action(action);
    }

    /// Make sure the cover for the playing track is fetched and rendered at
    /// the size the pane currently is.
    pub fn ensure_art(&mut self, cols: u16, rows: u16) {
        if !self.show_art || cols == 0 || rows == 0 {
            return;
        }
        let Some(url) = self
            .player
            .status()
            .current
            .as_ref()
            .and_then(|t| t.thumbnail.clone())
        else {
            return;
        };
        // Ask the image host for roughly what will be drawn; upscaling a 60px
        // thumbnail looks far worse than requesting the right size.
        let want = ytm_art::at_size(&url, (cols as u32).max(rows as u32 * 2).clamp(64, 544));
        if self.art_for.as_ref() == Some(&(want.clone(), cols, rows)) {
            return;
        }
        if let Some(img) = self.art_cache.get(&want) {
            self.art_cells = ytm_art::to_half_blocks(&img, cols, rows);
            self.art_for = Some((want, cols, rows));
            return;
        }
        if self.art_cache.has(&want) || self.art_fetching {
            return; // already tried, or a fetch is in flight
        }
        self.art_fetching = true;
        let cache = self.art_cache.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = cache.fetch(&want);
            let _ = tx.send(AppEvent::ArtReady);
        });
    }

    /// Render one frame of whichever pixel visualiser is showing.
    ///
    /// Driven from the tick rather than from the widget because two of the
    /// three are simulations with state, not functions of the current spectrum
    /// frame - and because on a graphics terminal nothing is drawn into the
    /// ratatui buffer at all.
    fn step_pixels(&mut self) {
        let area = self.hit.viz.get();
        if !self.viz_style.is_pixel() || area.width == 0 || area.height == 0 {
            self.pixels.clear();
            self.pixel_cells = Vec::new();
            return;
        }
        let graphics = crate::cover::is_graphics(self.art_backend);
        let frame = self.spectrum.load_full();
        let dt = self.pixel_at.elapsed().as_secs_f32().clamp(1e-3, 0.1);
        self.pixel_at = Instant::now();
        self.pixels.step(
            self.viz_style,
            viz_pixels(self.viz_style, graphics, area),
            &frame,
            self.beat_glow,
            dt,
            &self.theme,
        );
        if !graphics {
            if let Some(img) = self.pixels.image() {
                self.pixel_cells = ytm_art::to_half_blocks(img, area.width, area.height);
            }
        }
    }

    /// Take one column of history per rendered frame, and decay the beat glow.
    fn sample_spectrum(&mut self) {
        let f = self.spectrum.load();
        if f.seq == self.last_seq {
            return;
        }
        self.last_seq = f.seq;
        if !f.bands.is_empty() {
            self.history.push_back(f.bands.clone());
            // Bounded: only what a very wide terminal could show.
            while self.history.len() > 512 {
                self.history.pop_front();
            }
        }
        self.chroma.push_back(f.chroma);
        while self.chroma.len() > 512 {
            self.chroma.pop_front();
        }
        if f.beat {
            self.beat_glow = self.beat_glow.max(0.35 + f.beat_strength * 0.65);
        } else {
            self.beat_glow = (self.beat_glow - 0.06).max(0.0);
        }
    }

    /// Pick up edits to config.toml without a restart. Polling the mtime a few
    /// times a minute avoids a file-watcher dependency for something this small.
    fn poll_config(&mut self) {
        if self.config_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.config_checked = Instant::now();
        let now = config_mtime();
        if now == self.config_mtime {
            return;
        }
        self.config_mtime = now;
        let loaded = ytm_config::load();
        self.theme = Theme::from_config(&loaded.config);
        ytm_art::set_cell_aspect(loaded.config.art.cell_aspect);
        self.keymap = loaded.keymap;
        self.autoplay = loaded.config.general.autoplay;
        self.config = loaded.config;
        if let Some(w) = loaded.warnings.first() {
            self.toast(format!("config reloaded with problems: {w}"));
        } else {
            self.toast("config reloaded".into());
        }
    }

    /// Debounce: only ask after typing pauses, so a fast typist makes one
    /// request rather than one per keystroke (FR-S2, and FR-N2).
    fn schedule_suggestions(&mut self) {
        self.suggest_after = Some(Instant::now() + Duration::from_millis(250));
    }

    fn poll_suggestions(&mut self) {
        let Some(at) = self.suggest_after else { return };
        if Instant::now() < at {
            return;
        }
        self.suggest_after = None;
        let q = self.query.trim().to_string();
        if q.is_empty() {
            self.suggestions.clear();
            return;
        }
        if q == self.suggest_for {
            return;
        }
        self.suggest_for = q.clone();
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Ok(s) = b.search_suggestions(&q) {
                let _ = tx.send(AppEvent::Suggestions(q, s));
            }
        });
    }

    fn recall_search(&mut self, delta: isize) {
        if self.search_history.is_empty() {
            return;
        }
        let n = self.search_history.len() as isize;
        let next = match self.history_pos {
            None if delta > 0 => 0,
            None => return,
            Some(p) => (p as isize + delta).clamp(-1, n - 1),
        };
        if next < 0 {
            self.history_pos = None;
            self.query.clear();
            return;
        }
        self.history_pos = Some(next as usize);
        self.query = self.search_history[next as usize].clone();
    }

    fn modal_key(&mut self, k: KeyEvent) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        match modal {
            Modal::Palette { query, sel } => match k.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Up => *sel = sel.saturating_sub(1),
                KeyCode::Down => *sel += 1,
                KeyCode::Backspace => {
                    query.pop();
                    *sel = 0;
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    *sel = 0;
                }
                KeyCode::Enter => {
                    let matches = filter_actions(query);
                    let chosen = matches.get(*sel).copied();
                    self.modal = None;
                    if let Some(a) = chosen {
                        self.do_action(a);
                    }
                }
                _ => {}
            },
            Modal::PlaylistPicker {
                playlists,
                sel,
                track,
                ..
            } => match k.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Up => *sel = sel.saturating_sub(1),
                KeyCode::Down => *sel = (*sel + 1).min(playlists.len()),
                KeyCode::Enter => {
                    let t = (**track).clone();
                    // Index 0 is always "new playlist"; the rest are existing ones.
                    if *sel == 0 {
                        self.modal = Some(Modal::Text {
                            title: "new playlist".into(),
                            value: String::new(),
                            prompt: Prompt::NewPlaylist {
                                then_add: Some(Box::new(t)),
                            },
                        });
                    } else {
                        let target = playlists.get(*sel - 1).cloned();
                        self.modal = None;
                        if let Some(p) = target {
                            self.add_to_playlist(p, t);
                        }
                    }
                }
                _ => {}
            },
            Modal::Text { value, prompt, .. } => match k.code {
                KeyCode::Esc => self.modal = None,
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(c) => value.push(c),
                KeyCode::Enter => {
                    let (v, p) = (value.clone(), prompt.clone());
                    self.modal = None;
                    if !v.trim().is_empty() {
                        self.submit_prompt(p, v);
                    }
                }
                _ => {}
            },
            Modal::Confirm { confirm, .. } => match k.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let c = confirm.clone();
                    self.modal = None;
                    self.submit_confirm(c);
                }
                _ => self.modal = None,
            },
        }
    }

    fn submit_prompt(&mut self, prompt: Prompt, value: String) {
        let b = self.backend.clone();
        let tx = self.tx.clone();
        match prompt {
            Prompt::NewPlaylist { then_add } => {
                std::thread::spawn(move || {
                    let msg = match b.create_playlist(&value, "") {
                        Ok(id) => match then_add {
                            Some(t) => match b.playlist_add(&id, &t.id) {
                                Ok(()) => format!("created '{value}' and added {}", t.title),
                                Err(e) => format!("created '{value}' but adding failed: {e}"),
                            },
                            None => format!("created playlist '{value}'"),
                        },
                        Err(e) => format!("create failed: {e}"),
                    };
                    let _ = tx.send(AppEvent::Wrote(msg, true));
                });
            }
            Prompt::RenamePlaylist { id } => {
                std::thread::spawn(move || {
                    let msg = match b.rename_playlist(&id, &value) {
                        Ok(()) => format!("renamed to '{value}'"),
                        Err(e) => format!("rename failed: {e}"),
                    };
                    let _ = tx.send(AppEvent::Wrote(msg, true));
                });
            }
        }
    }

    fn submit_confirm(&mut self, confirm: Confirm) {
        let b = self.backend.clone();
        let tx = self.tx.clone();
        match confirm {
            Confirm::DeletePlaylist { id, title } => {
                std::thread::spawn(move || {
                    let msg = match b.delete_playlist(&id) {
                        Ok(()) => format!("deleted '{title}'"),
                        Err(e) => format!("delete failed: {e}"),
                    };
                    let _ = tx.send(AppEvent::Wrote(msg, true));
                });
            }
        }
    }

    fn add_to_playlist(&mut self, p: PlaylistRef, t: Track) {
        let Some(pid) = p.playlist_id.clone().or_else(|| {
            p.id.0
                .strip_prefix("VL")
                .map(|x| ytm_core::PlaylistId(x.to_string()))
        }) else {
            self.toast("that playlist has no usable id".into());
            return;
        };
        let b = self.backend.clone();
        let tx = self.tx.clone();
        let (title, ptitle) = (t.title.clone(), p.title.clone());
        std::thread::spawn(move || {
            let msg = match b.playlist_add(&pid, &t.id) {
                Ok(()) => format!("added {title} to {ptitle}"),
                Err(e) => format!("add failed: {e}"),
            };
            let _ = tx.send(AppEvent::Wrote(msg, false));
        });
    }

    fn open_playlist_picker(&mut self) {
        let Some(track) = self
            .selected_track()
            .or_else(|| self.player.status().current.clone())
        else {
            self.toast("nothing selected".into());
            return;
        };
        if !self.backend.is_authenticated() {
            self.toast("not signed in".into());
            return;
        }
        self.modal = Some(Modal::PlaylistPicker {
            track: Box::new(track),
            playlists: Vec::new(),
            sel: 0,
            loading: true,
        });
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = b
                .library(ytm_api::LibrarySection::Playlists)
                .map(|p| {
                    p.rows
                        .into_iter()
                        .filter_map(|r| match r {
                            Row::Playlist(p) => Some(p),
                            _ => None,
                        })
                        .collect()
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Playlists(r));
        });
    }

    /// The playlist this view is showing, if any - the target for rename,
    /// delete and remove-from.
    fn current_playlist(&self) -> Option<(ytm_core::PlaylistId, String)> {
        let p = self.page()?;
        match &p.view {
            View::Playlist(id, title) => {
                let pid =
                    id.0.strip_prefix("VL")
                        .map(|x| ytm_core::PlaylistId(x.to_string()))
                        .unwrap_or_else(|| ytm_core::PlaylistId(id.0.clone()));
                Some((pid, title.clone()))
            }
            _ => None,
        }
    }

    fn remove_from_playlist(&mut self) {
        let Some((pid, _)) = self.current_playlist() else {
            self.toast("not viewing a playlist".into());
            return;
        };
        let Some(t) = self.selected_track() else {
            self.toast("no track selected".into());
            return;
        };
        let Some(svid) = t.set_video_id.clone() else {
            // Without the per-playlist item id the API cannot identify the row.
            self.toast("this row cannot be removed (no playlist item id)".into());
            return;
        };
        let b = self.backend.clone();
        let tx = self.tx.clone();
        let title = t.title.clone();
        std::thread::spawn(move || {
            let msg = match b.playlist_remove(&pid, &t.id, &svid) {
                Ok(()) => format!("removed {title}"),
                Err(e) => format!("remove failed: {e}"),
            };
            let _ = tx.send(AppEvent::Wrote(msg, true));
        });
    }

    fn toggle_subscribe(&mut self) {
        // The artist is whatever the cursor is on, else the current page.
        let target = match self.page().and_then(|p| p.selected()) {
            Some(Row::Artist(a)) => Some((a.id.clone(), a.name.clone())),
            _ => match self.page().map(|p| &p.view) {
                Some(View::Artist(id, name)) => Some((id.clone(), name.clone())),
                _ => None,
            },
        };
        let Some((id, name)) = target else {
            self.toast("select an artist first".into());
            return;
        };
        let b = self.backend.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // Read the current state first: a "toggle" that cannot read it can
            // only ever subscribe.
            let msg = match b.is_subscribed(&id) {
                Ok(now) => match b.set_subscribed(&id, !now) {
                    Ok(()) if now => format!("unsubscribed from {name}"),
                    Ok(()) => format!("subscribed to {name}"),
                    Err(e) => format!("subscribe failed: {e}"),
                },
                Err(e) => format!("could not read subscription state: {e}"),
            };
            let _ = tx.send(AppEvent::Wrote(msg, true));
        });
    }

    fn reload_current(&mut self) {
        let Some(p) = self.stack.last() else { return };
        let view = p.view.clone();
        self.generation += 1;
        let gen = self.generation;
        if let Some(p) = self.stack.last_mut() {
            p.generation = gen;
            p.loading = true;
        }
        self.fetch(view, gen);
    }

    /// Every binding, and the command palette, funnels through here.
    pub fn do_action(&mut self, action: Action) {
        use Action::*;
        match action {
            Quit => self.should_quit = true,
            Help => self.show_help = true,
            Search => {
                self.mode = Mode::Search;
                self.query.clear();
            }
            CommandPalette => {
                self.modal = Some(Modal::Palette {
                    query: String::new(),
                    sel: 0,
                })
            }
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
            SpeedUp => {
                let v = (self.player.status().speed + 0.1).min(2.0);
                self.player.send(PCmd::SetSpeed(v));
                self.toast(format!("speed {v:.2}x"));
            }
            SpeedDown => {
                let v = (self.player.status().speed - 0.1).max(0.5);
                self.player.send(PCmd::SetSpeed(v));
                self.toast(format!("speed {v:.2}x"));
            }
            SpeedReset => {
                self.player.send(PCmd::SetSpeed(1.0));
                self.toast("speed 1.00x".into());
            }
            ToggleNormalize => {
                let on = !self.player.status().normalize;
                self.player.send(PCmd::ToggleNormalize);
                self.toast(format!(
                    "loudness levelling {}",
                    if on { "on" } else { "off" }
                ));
            }
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
            AddToPlaylist => self.open_playlist_picker(),
            NewPlaylist => {
                self.modal = Some(Modal::Text {
                    title: "new playlist".into(),
                    value: String::new(),
                    prompt: Prompt::NewPlaylist { then_add: None },
                })
            }
            RenamePlaylist => match self.current_playlist() {
                Some((id, title)) => {
                    self.modal = Some(Modal::Text {
                        title: "rename playlist".into(),
                        value: title,
                        prompt: Prompt::RenamePlaylist { id },
                    })
                }
                None => self.toast("open a playlist first".into()),
            },
            DeletePlaylist => match self.current_playlist() {
                Some((id, title)) => {
                    self.modal = Some(Modal::Confirm {
                        message: format!("Delete playlist '{title}'?  y / n"),
                        confirm: Confirm::DeletePlaylist { id, title },
                    })
                }
                None => self.toast("open a playlist first".into()),
            },
            RemoveFromPlaylist => self.remove_from_playlist(),
            ToggleSubscribe => self.toggle_subscribe(),
            CopyLink => self.copy_link(),

            CycleVisualizer => self.viz_style = self.viz_style.next(),
            ToggleVisualizerFullscreen => self.viz_fullscreen = !self.viz_fullscreen,
            ToggleArt => {
                self.show_art = !self.show_art;
                let on = self.show_art;
                if !on {
                    self.art_cells.clear();
                    self.art_for = None;
                }
                self.toast(format!(
                    "album art {} ({})",
                    if on { "on" } else { "off" },
                    self.art_backend.name()
                ));
            }
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
        let Some(t) = self
            .selected_track()
            .or_else(|| self.player.status().current.clone())
        else {
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
            match event::read()? {
                Event::Key(k) if k.is_press() => self.handle_key(k),
                Event::Mouse(m) => self.handle_mouse(m),
                _ => {}
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn handle_mouse(&mut self, m: MouseEvent) {
        // Overlays own the pointer too, and there is nothing useful to click.
        if self.modal.is_some() || self.show_help {
            return;
        }
        let inside = |r: Rect| {
            m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
        };
        let (sidebar, content, queue, progress) = (
            self.hit.sidebar.get(),
            self.hit.content.get(),
            self.hit.queue.get(),
            self.hit.progress.get(),
        );

        match m.kind {
            MouseEventKind::ScrollDown => {
                let target = if inside(sidebar) {
                    Focus::Sidebar
                } else if inside(queue) {
                    Focus::Queue
                } else {
                    Focus::Content
                };
                self.focus = target;
                self.move_sel(3);
            }
            MouseEventKind::ScrollUp => {
                let target = if inside(sidebar) {
                    Focus::Sidebar
                } else if inside(queue) {
                    Focus::Queue
                } else {
                    Focus::Content
                };
                self.focus = target;
                self.move_sel(-3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if inside(progress) && progress.width > 0 {
                    self.seek_to_fraction(
                        (m.column.saturating_sub(progress.x)) as f64 / progress.width as f64,
                    );
                } else if inside(sidebar) {
                    self.focus = Focus::Sidebar;
                    let i = (m.row - sidebar.y) as usize;
                    if self.sidebar.get(i).map(|d| d.is_selectable()) == Some(true) {
                        self.sidebar_sel = i;
                    }
                } else if inside(queue) {
                    self.focus = Focus::Queue;
                    let idx = self.lists.queue.borrow().offset() + (m.row - queue.y) as usize;
                    if idx < self.player.status().queue.len() {
                        self.queue_sel = idx;
                    }
                } else if inside(content) {
                    self.focus = Focus::Content;
                    let idx = self.lists.content.borrow().offset() + (m.row - content.y) as usize;
                    if let Some(p) = self.stack.last_mut() {
                        if p.rows.get(idx).map(|r| r.is_selectable()) == Some(true) {
                            p.sel = idx;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn seek_to_fraction(&mut self, f: f64) {
        let st = self.player.status();
        let Some(total) = st.current.as_ref().and_then(|t| t.duration) else {
            return;
        };
        let target = total.as_secs_f64() * f.clamp(0.0, 1.0);
        self.player
            .send(PCmd::SeekRelative(target - st.position.as_secs_f64()));
    }

    pub fn shutdown(&self) {
        self.save_session();
        self.quit_flag.store(true, Ordering::Relaxed);
        self.player.send(PCmd::Shutdown);
    }
}

fn config_mtime() -> Option<std::time::SystemTime> {
    let p = ytm_config::config_path()?;
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Pixels a style draws into for a pane of `area` cells.
///
/// Half blocks give exactly two pixels per cell, so the picture is drawn
/// one-to-one and never resampled - which matters most for the scope, whose
/// one-pixel trace would be filtered away by a downscale.
///
/// With a graphics backend the width is capped, and by how much depends on
/// what the style is. The simulations want a *low* resolution: a full pane of
/// doom fire is a large simulation and a lot of bytes down a pty sixty times a
/// second, and chunky pixels are the look anyway. The scope wants the
/// opposite, because resolution is the entire reason it draws pixels at all,
/// and drawing a line costs nothing whatever the size.
fn viz_pixels(style: VizStyle, graphics: bool, area: Rect) -> (usize, usize) {
    if !graphics {
        return (area.width as usize, area.height as usize * 2);
    }
    let max_w = if style == VizStyle::Scope { 1600 } else { 360 };
    let cell = ytm_art::cell_px();
    let w = area.width as usize * cell.w as usize;
    let h = area.height as usize * cell.h as usize;
    if w <= max_w {
        return (w.max(1), h.max(1));
    }
    (max_w, (h * max_w / w).max(1))
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
