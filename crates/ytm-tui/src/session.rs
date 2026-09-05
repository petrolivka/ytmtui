//! Session persistence: the queue, position and volume survive a restart.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ytm_core::Track;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    pub queue: Vec<Track>,
    pub index: usize,
    /// Seconds into the current track.
    pub position: f64,
    pub volume: f32,
    /// Kept so a very stale session can be recognised.
    pub saved_at_unix: u64,
}

fn path() -> Option<PathBuf> {
    ytm_config::state_dir().map(|d| d.join("session.json"))
}

pub fn load() -> Option<Session> {
    let p = path()?;
    let text = std::fs::read_to_string(p).ok()?;
    // A corrupt session file must never stop the app from starting.
    serde_json::from_str(&text).ok()
}

pub fn save(s: &Session) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    // A queue can be hundreds of entries after a radio run; cap what is stored
    // so the file stays small and loading stays instant.
    let mut s = s.clone();
    const MAX: usize = 200;
    if s.queue.len() > MAX {
        let start = s.index.saturating_sub(MAX / 4);
        let end = (start + MAX).min(s.queue.len());
        s.index -= start;
        s.queue = s.queue[start..end].to_vec();
    }
    if let Ok(text) = serde_json::to_string(&s) {
        if std::fs::write(&p, text).is_ok() {
            // Not a credential, but it is a record of what was listened to.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

fn history_path() -> Option<PathBuf> {
    ytm_config::state_dir().map(|d| d.join("search_history.txt"))
}

/// Recent searches, most recent first (FR-S3).
pub fn load_search_history() -> Vec<String> {
    let Some(p) = history_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(p)
        .map(|t| {
            t.lines()
                .map(|l| l.to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_search_history(h: &[String]) {
    let Some(p) = history_path() else { return };
    if let Some(dir) = p.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let _ = std::fs::write(p, h.join("\n"));
}
