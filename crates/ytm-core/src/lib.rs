//! Domain types shared by every other crate. No I/O, no dependencies on the
//! backend or the UI.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// An 11-character YouTube video id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VideoId(pub String);

impl fmt::Display for VideoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl VideoId {
    pub fn is_valid(&self) -> bool {
        self.0.len() == 11
            && self
                .0
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }
}

/// An InnerTube `browseId`, addressing an artist, album or playlist page.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BrowseId(pub String);

impl fmt::Display for BrowseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A playlist id, which is not interchangeable with a browse id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlaylistId(pub String);

impl fmt::Display for PlaylistId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A playable track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: VideoId,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    /// Present only in authenticated responses; required for library writes.
    pub feedback_token_add: Option<String>,
    pub feedback_token_remove: Option<String>,
    pub rating: Rating,
    /// Whether the track is in the user's library. Distinct from `rating`:
    /// liking a song adds it to Liked Songs, not to the library.
    pub in_library: bool,
    /// Targets for "go to album" / "go to artist" navigation.
    pub album_id: Option<BrowseId>,
    pub artist_id: Option<BrowseId>,
}

impl Track {
    pub fn new(id: impl Into<String>, title: impl Into<String>, artist: impl Into<String>) -> Self {
        Self {
            id: VideoId(id.into()),
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration: None,
            feedback_token_add: None,
            feedback_token_remove: None,
            rating: Rating::Indifferent,
            in_library: false,
            album_id: None,
            artist_id: None,
        }
    }

    pub fn duration_str(&self) -> String {
        match self.duration {
            Some(d) => fmt_duration(d),
            None => "--:--".into(),
        }
    }
}

/// Thumbs up / down. Deliberately distinct from library membership: liking a
/// song adds it to Liked Songs and biases radio, but does *not* add it to the
/// library. Conflating the two is the classic third-party-client bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Rating {
    Like,
    Dislike,
    #[default]
    Indifferent,
}

impl Rating {
    /// What pressing thumbs-up should produce, given the current state.
    pub fn toggled_like(self) -> Self {
        if self == Rating::Like { Rating::Indifferent } else { Rating::Like }
    }
    pub fn toggled_dislike(self) -> Self {
        if self == Rating::Dislike { Rating::Indifferent } else { Rating::Dislike }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            Rating::Like => "\u{1F44D}",
            Rating::Dislike => "\u{1F44E}",
            Rating::Indifferent => " ",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            RepeatMode::Off => "repeat:off",
            RepeatMode::All => "repeat:all",
            RepeatMode::One => "repeat:one",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayState {
    #[default]
    Stopped,
    Buffering,
    Playing,
    Paused,
}

impl PlayState {
    pub fn glyph(self) -> &'static str {
        match self {
            PlayState::Stopped => "\u{25A0}",
            PlayState::Buffering => "\u{25CC}",
            PlayState::Playing => "\u{25B6}",
            PlayState::Paused => "\u{23F8}",
        }
    }
}

/// Snapshot of the engine, published for the UI to render.
#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    pub state: PlayState,
    pub current: Option<Track>,
    pub position: Duration,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub queue: Vec<Track>,
    pub queue_index: usize,
    /// Last error, surfaced in the status bar rather than swallowed.
    pub error: Option<String>,
}

pub fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A card pointing at an album page.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumRef {
    pub id: BrowseId,
    pub title: String,
    pub artist: String,
    pub year: Option<String>,
}

/// A card pointing at an artist page.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtistRef {
    pub id: BrowseId,
    pub name: String,
    pub subtitle: String,
}

/// A card pointing at a playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistRef {
    pub id: BrowseId,
    pub playlist_id: Option<PlaylistId>,
    pub title: String,
    pub subtitle: String,
}

/// One line in the main pane.
///
/// Home, search, library and every entity page render through this single
/// shape, so navigation and selection are written once rather than per view.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A non-selectable section heading, e.g. a Home shelf title.
    Header(String),
    Track(Track),
    Album(AlbumRef),
    Artist(ArtistRef),
    Playlist(PlaylistRef),
}

impl Row {
    pub fn is_selectable(&self) -> bool {
        !matches!(self, Row::Header(_))
    }

    pub fn title(&self) -> &str {
        match self {
            Row::Header(t) => t,
            Row::Track(t) => &t.title,
            Row::Album(a) => &a.title,
            Row::Artist(a) => &a.name,
            Row::Playlist(p) => &p.title,
        }
    }

    pub fn subtitle(&self) -> String {
        match self {
            Row::Header(_) => String::new(),
            Row::Track(t) => t.artist.clone(),
            Row::Album(a) => match &a.year {
                Some(y) => format!("{} \u{2022} {}", a.artist, y),
                None => a.artist.clone(),
            },
            Row::Artist(a) => a.subtitle.clone(),
            Row::Playlist(p) => p.subtitle.clone(),
        }
    }

    /// Short tag shown in the right-hand column, so the kind of a row is
    /// obvious without relying on colour alone (NFR-8).
    pub fn tag(&self) -> String {
        match self {
            Row::Header(_) => String::new(),
            Row::Track(t) => t.duration_str(),
            Row::Album(_) => "album".into(),
            Row::Artist(_) => "artist".into(),
            Row::Playlist(_) => "playlist".into(),
        }
    }

    pub fn as_track(&self) -> Option<&Track> {
        match self {
            Row::Track(t) => Some(t),
            _ => None,
        }
    }
}
