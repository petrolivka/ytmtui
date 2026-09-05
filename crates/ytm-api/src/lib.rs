//! YouTube Music access. Everything the app needs from the network lives behind
//! `MusicBackend`, so an InnerTube change or a library swap is a contained fix.

pub mod innertube;
pub mod json;

use anyhow::Result;
use ytm_core::{Rating, Track, VideoId};

pub use innertube::Innertube;

pub trait MusicBackend: Send + Sync {
    fn search_songs(&self, query: &str) -> Result<Vec<Track>>;
    fn liked_songs(&self) -> Result<Vec<Track>>;
    fn rate(&self, id: &VideoId, rating: Rating) -> Result<()>;
    fn is_authenticated(&self) -> bool;
}

impl MusicBackend for Innertube {
    fn search_songs(&self, query: &str) -> Result<Vec<Track>> {
        Innertube::search_songs(self, query)
    }
    fn liked_songs(&self) -> Result<Vec<Track>> {
        Innertube::liked_songs(self)
    }
    fn rate(&self, id: &VideoId, rating: Rating) -> Result<()> {
        Innertube::rate(self, id, rating)
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
            return Innertube::from_cookies(&std::fs::read_to_string(p)?);
        }
    }
    for p in ["cookies.txt"] {
        if std::path::Path::new(p).exists() {
            return Innertube::from_cookies(&std::fs::read_to_string(p)?);
        }
    }
    Innertube::anonymous()
}

fn dirs_config() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|d| d.join("ytmtui"))
}
