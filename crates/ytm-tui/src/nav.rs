//! Where the user is, and how they got there.
//!
//! Every surface renders as a list of `Row`, so one view type and one
//! navigation stack serve Home, Explore, the library and every entity page.

use ytm_api::{ExploreSection, LibrarySection, PageFilter, SearchFilter};
use ytm_core::{BrowseId, Row};

/// A header chip the user has applied, kept on the view so that going back
/// returns to the feed as it was rather than to the unfiltered one.
///
/// Only the applied chip is stored; the "All" chip is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub label: String,
    pub params: String,
}

impl Chip {
    /// `None` for the "All" chip, which is the unfiltered page.
    pub fn from_filter(f: &PageFilter) -> Option<Self> {
        f.params.as_ref().map(|p| Chip {
            label: f.label.clone(),
            params: p.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Home(Option<Chip>),
    Explore(ExploreSection),
    Library(LibrarySection),
    Search {
        query: String,
        filter: SearchFilter,
    },
    Artist(BrowseId, String),
    Album(BrowseId, String),
    Playlist(BrowseId, String),
    /// A mood or genre page: browse id, its opaque params, and the tile's label.
    Category(BrowseId, Option<String>, String),
}

impl View {
    pub fn title(&self) -> String {
        match self {
            View::Home(None) => "Home".into(),
            View::Home(Some(c)) => format!("Home \u{2022} {}", c.label),
            View::Explore(s) => s.label().into(),
            View::Library(s) => s.label().into(),
            View::Search { query, filter } => {
                format!("Search: {query} \u{2022} {}", filter.label())
            }
            View::Artist(_, name) => name.clone(),
            View::Album(_, name) => name.clone(),
            View::Playlist(_, name) => name.clone(),
            View::Category(_, _, name) => name.clone(),
        }
    }

    /// Search is the only view with tabs across the top.
    pub fn filter(&self) -> Option<SearchFilter> {
        match self {
            View::Search { filter, .. } => Some(*filter),
            _ => None,
        }
    }
}

/// One entry on the navigation stack, with its own content and cursor so going
/// back restores the previous position rather than resetting it.
pub struct Page {
    pub view: View,
    pub rows: Vec<Row>,
    pub sel: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// Identifies the in-flight request, so a superseded load is discarded
    /// instead of overwriting newer content.
    pub generation: u64,
    /// Token for the next page, when the list has more (FR-B4).
    pub continuation: Option<String>,
    pub loading_more: bool,
    /// The page's header chips, when it has any. Empty for most surfaces.
    pub filters: Vec<PageFilter>,
}

impl Page {
    pub fn new(view: View, generation: u64) -> Self {
        Self {
            view,
            rows: Vec::new(),
            sel: 0,
            loading: true,
            error: None,
            generation,
            continuation: None,
            loading_more: false,
            filters: Vec::new(),
        }
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.sel)
    }

    /// Move the cursor, skipping non-selectable section headers.
    pub fn move_sel(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let step = if delta >= 0 { 1 } else { -1 };
        let mut i = self.sel as isize;
        let mut remaining = delta.abs();
        while remaining > 0 {
            let mut moved = false;
            let mut probe = i;
            for _ in 0..n {
                probe += step;
                if probe < 0 || probe >= n {
                    break;
                }
                if self.rows[probe as usize].is_selectable() {
                    i = probe;
                    moved = true;
                    break;
                }
            }
            if !moved {
                break;
            }
            remaining -= 1;
        }
        self.sel = i.clamp(0, n - 1) as usize;
        // A header may still be selected if the list starts with one.
        if !self.rows[self.sel].is_selectable() {
            if let Some(next) = self.rows.iter().position(|r| r.is_selectable()) {
                self.sel = next;
            }
        }
    }

    pub fn snap_to_selectable(&mut self) {
        if self.rows.get(self.sel).map(|r| r.is_selectable()) == Some(true) {
            return;
        }
        self.sel = self
            .rows
            .iter()
            .position(|r| r.is_selectable())
            .unwrap_or(0);
    }

    /// Every playable row, and where the given index sits among them - so
    /// pressing Enter plays the whole view starting from the highlighted track.
    pub fn tracks_from(&self, index: usize) -> Option<(Vec<ytm_core::Track>, usize)> {
        let mut tracks = Vec::new();
        let mut start = None;
        for (i, r) in self.rows.iter().enumerate() {
            if let Row::Track(t) = r {
                if i == index {
                    start = Some(tracks.len());
                }
                tracks.push(t.clone());
            }
        }
        start.map(|s| (tracks, s))
    }
}

/// A destination in the sidebar.
#[derive(Debug, Clone, PartialEq)]
pub enum Dest {
    Separator(&'static str),
    Go(View),
}

pub fn sidebar(authenticated: bool) -> Vec<Dest> {
    let mut d = vec![
        Dest::Separator("browse"),
        Dest::Go(View::Home(None)),
        Dest::Go(View::Explore(ExploreSection::Explore)),
        Dest::Go(View::Explore(ExploreSection::NewReleases)),
        Dest::Go(View::Explore(ExploreSection::Charts)),
    ];
    if authenticated {
        d.push(Dest::Separator("library"));
        for s in LibrarySection::ALL {
            d.push(Dest::Go(View::Library(s)));
        }
    }
    d
}

impl Dest {
    pub fn label(&self) -> String {
        match self {
            Dest::Separator(s) => (*s).to_string(),
            Dest::Go(v) => v.title(),
        }
    }
    pub fn is_selectable(&self) -> bool {
        matches!(self, Dest::Go(_))
    }
}
