//! Every action a key can be bound to.
//!
//! Keeping this as one enum is what makes the keymap remappable (FR-U3) and
//! the command palette possible (FR-U5): both are just different ways of
//! choosing an `Action`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    // navigation
    Search,
    Activate,
    Back,
    NextPane,
    PrevPane,
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    NextTab,
    PrevTab,
    GoToArtist,
    GoToAlbum,
    // playback
    TogglePause,
    Next,
    Prev,
    SeekForward,
    SeekBackward,
    SeekForwardLong,
    SeekBackwardLong,
    VolumeUp,
    VolumeDown,
    ToggleShuffle,
    SpeedUp,
    SpeedDown,
    SpeedReset,
    ToggleNormalize,
    CycleRepeat,
    StartRadio,
    ToggleAutoplay,
    // queue
    PlayNext,
    Enqueue,
    RemoveFromQueue,
    // account
    ThumbsUp,
    ThumbsDown,
    ToggleLibrary,
    AddToPlaylist,
    NewPlaylist,
    RenamePlaylist,
    DeletePlaylist,
    RemoveFromPlaylist,
    ToggleSubscribe,
    CopyLink,
    // interface
    CycleVisualizer,
    ToggleVisualizerFullscreen,
    ToggleLyrics,
    ToggleArt,
    CommandPalette,
    Help,
    Quit,
}

impl Action {
    /// Human-readable label, used by the help overlay and command palette.
    pub fn label(self) -> &'static str {
        use Action::*;
        match self {
            Search => "Search",
            Activate => "Open / play selection",
            Back => "Back",
            NextPane => "Next pane",
            PrevPane => "Previous pane",
            Up => "Up",
            Down => "Down",
            PageUp => "Page up",
            PageDown => "Page down",
            Top => "Top",
            Bottom => "Bottom",
            NextTab => "Next search tab",
            PrevTab => "Previous search tab",
            GoToArtist => "Go to artist",
            GoToAlbum => "Go to album",
            TogglePause => "Play / pause",
            Next => "Next track",
            Prev => "Previous track",
            SeekForward => "Seek forward 5s",
            SeekBackward => "Seek back 5s",
            SeekForwardLong => "Seek forward 30s",
            SeekBackwardLong => "Seek back 30s",
            VolumeUp => "Volume up",
            VolumeDown => "Volume down",
            ToggleShuffle => "Toggle shuffle",
            SpeedUp => "Speed up",
            SpeedDown => "Slow down",
            SpeedReset => "Reset speed to 1x",
            ToggleNormalize => "Toggle loudness levelling",
            CycleRepeat => "Cycle repeat mode",
            StartRadio => "Start radio from selection",
            ToggleAutoplay => "Toggle autoplay",
            PlayNext => "Play next",
            Enqueue => "Add to end of queue",
            RemoveFromQueue => "Remove from queue",
            ThumbsUp => "Thumbs up",
            ThumbsDown => "Thumbs down",
            ToggleLibrary => "Add to / remove from library",
            AddToPlaylist => "Add to playlist",
            NewPlaylist => "New playlist",
            RenamePlaylist => "Rename this playlist",
            DeletePlaylist => "Delete this playlist",
            RemoveFromPlaylist => "Remove from this playlist",
            ToggleSubscribe => "Subscribe / unsubscribe to artist",
            CopyLink => "Copy link",
            CycleVisualizer => "Cycle visualiser style",
            ToggleVisualizerFullscreen => "Fullscreen visualiser",
            ToggleLyrics => "Toggle lyrics",
            ToggleArt => "Toggle album art",
            CommandPalette => "Command palette",
            Help => "Help",
            Quit => "Quit",
        }
    }

    /// The exact name accepted in the config file. Derived from the same
    /// snake_case shape as the parser, and a test round-trips every one
    /// through it - deriving from Debug instead printed "togglepause", which
    /// the config would reject.
    pub fn name(self) -> String {
        let debug = format!("{self:?}");
        let mut out = String::with_capacity(debug.len() + 4);
        for (i, c) in debug.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Every action, in the order the command palette offers them.
    ///
    /// This must list every variant: it is what the palette and
    /// `--list-actions` enumerate, and a variant left out is simply
    /// unreachable from either. A test checks that everything bound by the
    /// default keymap appears here.
    pub const ALL: &'static [Action] = {
        use Action::*;
        &[
            Search, Activate, Back,
            TogglePause, Next, Prev,
            SeekForward, SeekBackward, SeekForwardLong, SeekBackwardLong,
            VolumeUp, VolumeDown,
            ToggleShuffle, CycleRepeat, StartRadio, ToggleAutoplay,
            SpeedUp, SpeedDown, SpeedReset, ToggleNormalize,
            PlayNext, Enqueue, RemoveFromQueue,
            ThumbsUp, ThumbsDown, ToggleLibrary,
            AddToPlaylist, NewPlaylist, RenamePlaylist, DeletePlaylist,
            RemoveFromPlaylist, ToggleSubscribe, CopyLink,
            GoToArtist, GoToAlbum, NextTab, PrevTab,
            NextPane, PrevPane, Up, Down, PageUp, PageDown, Top, Bottom,
            CycleVisualizer, ToggleVisualizerFullscreen, ToggleLyrics, ToggleArt,
            CommandPalette, Help, Quit,
        ]
    };
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::Action;
    use std::str::FromStr;

    /// Whatever `--list-actions` prints must be parseable by `[keys]`.
    /// Deriving the name from Debug instead gave "togglepause", which the
    /// config would then reject.
    /// A variant missing from ALL is unreachable from the command palette and
    /// from --list-actions. This caught SpeedUp and friends being added to the
    /// enum but not to the catalogue.
    #[test]
    fn every_bound_action_is_in_the_catalogue() {
        for (chord, action) in crate::default_keymap() {
            assert!(
                Action::ALL.contains(&action),
                "{action:?} (bound to {chord}) is missing from Action::ALL"
            );
        }
    }

    #[test]
    fn catalogue_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for a in Action::ALL {
            assert!(seen.insert(*a), "{a:?} listed twice in Action::ALL");
        }
    }

    #[test]
    fn every_action_name_round_trips() {
        for a in Action::ALL {
            let n = a.name();
            assert!(!n.is_empty(), "{a:?} has no name");
            assert!(n.contains(|c: char| c.is_ascii_lowercase()));
            assert_eq!(Action::from_str(&n).unwrap(), *a, "name {n} did not round-trip");
        }
    }
}

/// Parse the snake_case name used in the config file.
impl FromStr for Action {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde::Deserialize::deserialize(serde::de::value::StrDeserializer::<
            serde::de::value::Error,
        >::new(s))
        .map_err(|_| format!("unknown action '{s}'"))
    }
}
