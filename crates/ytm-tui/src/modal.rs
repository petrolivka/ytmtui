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

#[cfg(test)]
mod more_tests {
    use super::*;

    /// Every action must be reachable by typing its own label - otherwise the
    /// palette silently cannot run it.
    #[test]
    fn every_action_is_findable_by_its_label() {
        for a in Action::ALL {
            let hits = filter_actions(a.label());
            assert!(
                hits.contains(a),
                "{:?} ({}) is not findable by its own label; got {:?}",
                a,
                a.label(),
                hits.iter().take(3).map(|x| x.label()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn speed_actions_are_in_the_catalogue() {
        assert!(Action::ALL.contains(&Action::SpeedUp), "SpeedUp missing from ALL");
        assert!(Action::ALL.contains(&Action::ToggleNormalize), "ToggleNormalize missing");
        assert!(Action::ALL.contains(&Action::ToggleArt), "ToggleArt missing");
    }
}
