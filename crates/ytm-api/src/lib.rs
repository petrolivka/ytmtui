//! YouTube Music access. Everything the app needs from the network lives behind
//! `MusicBackend`, so an InnerTube change or a library swap is a contained fix.

pub mod innertube;
pub mod json;
pub mod parse;

use anyhow::Result;
use ytm_core::{BrowseId, PlaylistId, Rating, Track, VideoId};

pub use innertube::{ExploreSection, Innertube, LibrarySection, RowPage, SearchFilter};

/// Everything the app needs from the network. One trait so an InnerTube change
/// or a library swap stays a contained fix.
pub trait MusicBackend: Send + Sync {
    fn search(&self, query: &str, filter: SearchFilter) -> Result<RowPage>;
    fn search_songs(&self, query: &str) -> Result<Vec<Track>>;
    fn home(&self) -> Result<RowPage>;
    fn library(&self, section: LibrarySection) -> Result<RowPage>;
    fn explore(&self, section: ExploreSection) -> Result<RowPage>;
    /// Next page of a list, addressed by the token the previous page returned.
    fn continue_rows(&self, token: &str) -> Result<RowPage>;
    /// Lyrics for a track, or None when YouTube Music has none.
    fn lyrics(&self, id: &VideoId) -> Result<Option<String>>;
    fn search_suggestions(&self, input: &str) -> Result<Vec<String>>;
    fn create_playlist(&self, title: &str, description: &str) -> Result<PlaylistId>;
    fn delete_playlist(&self, id: &PlaylistId) -> Result<()>;
    fn rename_playlist(&self, id: &PlaylistId, title: &str) -> Result<()>;
    fn playlist_add(&self, id: &PlaylistId, video: &VideoId) -> Result<()>;
    fn playlist_remove(&self, id: &PlaylistId, video: &VideoId, set_video_id: &str) -> Result<()>;
    fn set_subscribed(&self, channel: &BrowseId, subscribed: bool) -> Result<()>;
    fn is_subscribed(&self, channel: &BrowseId) -> Result<bool>;
    fn artist(&self, id: &BrowseId) -> Result<RowPage>;
    fn album(&self, id: &BrowseId) -> Result<RowPage>;
    fn playlist(&self, id: &BrowseId) -> Result<RowPage>;
    fn liked_songs(&self) -> Result<Vec<Track>>;
    /// Autoplay continuation seeded from a track.
    fn radio(&self, seed: &VideoId) -> Result<Vec<Track>>;
    fn rate(&self, id: &VideoId, rating: Rating) -> Result<()>;
    /// Add to or remove from the library - not the same as thumbs-up.
    fn set_library(&self, token: &str) -> Result<()>;
    /// True rating + library state for a track (FR-R7).
    fn track_state(&self, id: &VideoId) -> Result<(Rating, Option<String>, Option<String>, bool)>;
    fn is_authenticated(&self) -> bool;
}

impl MusicBackend for Innertube {
    fn search(&self, query: &str, filter: SearchFilter) -> Result<RowPage> {
        Innertube::search(self, query, filter)
    }
    fn search_songs(&self, query: &str) -> Result<Vec<Track>> {
        Innertube::search_songs(self, query)
    }
    fn home(&self) -> Result<RowPage> {
        Innertube::home(self)
    }
    fn library(&self, section: LibrarySection) -> Result<RowPage> {
        Innertube::library(self, section)
    }
    fn explore(&self, section: ExploreSection) -> Result<RowPage> {
        Innertube::explore(self, section)
    }
    fn continue_rows(&self, token: &str) -> Result<RowPage> {
        Innertube::continue_rows(self, token)
    }
    fn lyrics(&self, id: &VideoId) -> Result<Option<String>> {
        Innertube::lyrics(self, id)
    }
    fn search_suggestions(&self, input: &str) -> Result<Vec<String>> {
        Innertube::search_suggestions(self, input)
    }
    fn create_playlist(&self, title: &str, description: &str) -> Result<PlaylistId> {
        Innertube::create_playlist(self, title, description)
    }
    fn delete_playlist(&self, id: &PlaylistId) -> Result<()> {
        Innertube::delete_playlist(self, id)
    }
    fn rename_playlist(&self, id: &PlaylistId, title: &str) -> Result<()> {
        Innertube::rename_playlist(self, id, title)
    }
    fn playlist_add(&self, id: &PlaylistId, video: &VideoId) -> Result<()> {
        Innertube::playlist_add(self, id, video)
    }
    fn playlist_remove(&self, id: &PlaylistId, video: &VideoId, set_video_id: &str) -> Result<()> {
        Innertube::playlist_remove(self, id, video, set_video_id)
    }
    fn set_subscribed(&self, channel: &BrowseId, subscribed: bool) -> Result<()> {
        Innertube::set_subscribed(self, channel, subscribed)
    }
    fn is_subscribed(&self, channel: &BrowseId) -> Result<bool> {
        Innertube::is_subscribed(self, channel)
    }
    fn artist(&self, id: &BrowseId) -> Result<RowPage> {
        Innertube::artist(self, id)
    }
    fn album(&self, id: &BrowseId) -> Result<RowPage> {
        Innertube::album(self, id)
    }
    fn playlist(&self, id: &BrowseId) -> Result<RowPage> {
        Innertube::playlist(self, id)
    }
    fn liked_songs(&self) -> Result<Vec<Track>> {
        Innertube::liked_songs(self)
    }
    fn radio(&self, seed: &VideoId) -> Result<Vec<Track>> {
        Innertube::radio(self, seed)
    }
    fn rate(&self, id: &VideoId, rating: Rating) -> Result<()> {
        Innertube::rate(self, id, rating)
    }
    fn set_library(&self, token: &str) -> Result<()> {
        Innertube::set_library(self, token)
    }
    fn track_state(&self, id: &VideoId) -> Result<(Rating, Option<String>, Option<String>, bool)> {
        Innertube::track_state(self, id)
    }
    fn is_authenticated(&self) -> bool {
        Innertube::is_authenticated(self)
    }
}

/// Load credentials from the usual places. Absence is not an error: the app
/// runs anonymously with search and playback intact.
pub fn load_backend() -> Result<Innertube> {
    if let Ok(v) = std::env::var("YTM_COOKIE") {
        if !v.trim().is_empty() {
            return Innertube::from_cookies(&v);
        }
    }
    if let Some(dir) = dirs_config() {
        let p = dir.join("cookies.txt");
        if p.exists() {
            return Innertube::from_cookies(&read_private(&p)?);
        }
    }
    let p = std::path::Path::new("cookies.txt");
    if p.exists() {
        return Innertube::from_cookies(&read_private(p)?);
    }
    Innertube::anonymous()
}

/// Read a credential file, tightening its permissions first (FR-A2). A cookie
/// jar is a live credential; leaving it world-readable is a real exposure.
fn read_private(path: &std::path::Path) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
    }
    Ok(std::fs::read_to_string(path)?)
}

fn dirs_config() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|d| d.join("ytmtui"))
}
