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
