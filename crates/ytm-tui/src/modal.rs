//! Overlays that take over input: the command palette, pickers and prompts.
//!
//! Modelling them as one enum keeps input handling in a single place; each new
//! overlay is a variant rather than another flag to check in the key handler.

use ytm_config::Action;
use ytm_core::{PlaylistRef, Track};

/// What a text prompt does with its answer.
#[derive(Debug, Clone)]
pub enum Prompt {
    NewPlaylist { then_add: Option<Track> },
    RenamePlaylist { id: ytm_core::PlaylistId },
}

/// What a confirmation, if accepted, will do.
#[derive(Debug, Clone)]
pub enum Confirm {
    DeletePlaylist { id: ytm_core::PlaylistId, title: String },
}

pub enum Modal {
    Palette {
        query: String,
        sel: usize,
    },
    PlaylistPicker {
        track: Track,
        playlists: Vec<PlaylistRef>,
        sel: usize,
        loading: bool,
    },
    Text {
        title: String,
        value: String,
        prompt: Prompt,
    },
    Confirm {
        message: String,
        confirm: Confirm,
    },
}

impl Modal {
    pub fn title(&self) -> &str {
        match self {
            Modal::Palette { .. } => "command",
            Modal::PlaylistPicker { .. } => "add to playlist",
            Modal::Text { title, .. } => title,
            Modal::Confirm { .. } => "confirm",
        }
    }
}

/// Actions matching a filter, in the order they should be offered.
///
/// Matching is subsequence-based so "adpl" finds "Add to playlist" - typing
/// initials is how these are actually used.
pub fn filter_actions(query: &str) -> Vec<Action> {
    let q: String = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Action::ALL.to_vec();
    }
    let mut scored: Vec<(usize, Action)> = Action::ALL
        .iter()
        .filter_map(|a| subsequence_score(&a.label().to_lowercase(), &q).map(|s| (s, *a)))
        .collect();
    scored.sort_by_key(|(s, a)| (*s, a.label()));
    scored.into_iter().map(|(_, a)| a).collect()
}

/// Lower is better: the span consumed matching the query.
fn subsequence_score(haystack: &str, needle: &str) -> Option<usize> {
    let hay: Vec<char> = haystack.chars().collect();
    let mut first = None;
    let mut i = 0usize;
    for c in needle.chars() {
        let found = hay[i..].iter().position(|h| *h == c)? + i;
        first.get_or_insert(found);
        i = found + 1;
    }
    Some(i - first.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_offers_everything() {
        assert_eq!(filter_actions("").len(), Action::ALL.len());
    }

    #[test]
    fn matches_initials_and_substrings() {
        let r = filter_actions("addpl");
        assert!(r.contains(&Action::AddToPlaylist), "got {:?}", &r[..r.len().min(3)]);
        let r = filter_actions("shuffle");
        assert_eq!(r.first(), Some(&Action::ToggleShuffle));
    }

    #[test]
    fn nonsense_matches_nothing() {
        assert!(filter_actions("zzzqqq").is_empty());
    }
}
