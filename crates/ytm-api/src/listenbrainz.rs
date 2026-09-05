//! Scrobbling to ListenBrainz.
//!
//! ListenBrainz rather than Last.fm: a plain bearer token, no API-key
//! registration and no request signing, which keeps the setup to one line of
//! config.

use anyhow::{bail, Result};
use serde_json::json;
use std::time::Duration;

const ENDPOINT: &str = "https://api.listenbrainz.org/1/submit-listens";

pub struct Scrobbler {
    http: reqwest::blocking::Client,
    token: String,
}

/// When a listen counts, per the ListenBrainz guidance: half the track, or
/// four minutes, whichever comes first.
pub fn should_submit(played: Duration, total: Option<Duration>) -> bool {
    if played >= Duration::from_secs(240) {
        return true;
    }
    match total {
        // Very short tracks would otherwise never qualify.
        Some(t) if t.as_secs() >= 30 => played.as_secs_f64() >= t.as_secs_f64() / 2.0,
        _ => false,
    }
}

impl Scrobbler {
    pub fn new(token: &str) -> Result<Self> {
        if token.trim().is_empty() {
            bail!("no ListenBrainz token configured");
        }
        Ok(Self {
            http: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?,
            token: token.trim().to_string(),
        })
    }

    fn submit(&self, payload: serde_json::Value) -> Result<()> {
        let resp = self
            .http
            .post(ENDPOINT)
            .header("Authorization", format!("Token {}", self.token))
            .json(&payload)
            .send()?;
        if !resp.status().is_success() {
            let code = resp.status();
            let body = resp.text().unwrap_or_default();
            bail!("listenbrainz {code}: {}", &body[..body.len().min(160)]);
        }
        Ok(())
    }

    /// "Now playing" - not a listen, just what is on.
    pub fn playing_now(&self, artist: &str, title: &str, album: Option<&str>) -> Result<()> {
        self.submit(json!({
            "listen_type": "playing_now",
            "payload": [{ "track_metadata": metadata(artist, title, album) }],
        }))
    }

    /// A completed listen, timestamped at when it started.
    pub fn listen(
        &self,
        artist: &str,
        title: &str,
        album: Option<&str>,
        started_unix: u64,
    ) -> Result<()> {
        self.submit(json!({
            "listen_type": "single",
            "payload": [{
                "listened_at": started_unix,
                "track_metadata": metadata(artist, title, album),
            }],
        }))
    }
}

fn metadata(artist: &str, title: &str, album: Option<&str>) -> serde_json::Value {
    let mut m = json!({
        "artist_name": artist,
        "track_name": title,
        "additional_info": { "media_player": "ytmtui", "submission_client": "ytmtui" },
    });
    if let Some(a) = album.filter(|a| !a.trim().is_empty()) {
        m["release_name"] = json!(a);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_required() {
        assert!(Scrobbler::new("   ").is_err());
        assert!(Scrobbler::new("abc").is_ok());
    }

    #[test]
    fn submission_threshold_follows_the_guidance() {
        let four_min = Duration::from_secs(240);
        let three = Duration::from_secs(180);
        // Half of a 6-minute track.
        assert!(should_submit(three, Some(Duration::from_secs(360))));
        assert!(!should_submit(Duration::from_secs(100), Some(Duration::from_secs(360))));
        // Four minutes always counts, however long the track.
        assert!(should_submit(four_min, Some(Duration::from_secs(3600))));
        // A track under 30 seconds is not scrobbleable.
        assert!(!should_submit(Duration::from_secs(20), Some(Duration::from_secs(25))));
        // Unknown duration falls back to the four-minute rule alone.
        assert!(!should_submit(three, None));
        assert!(should_submit(four_min, None));
    }

    #[test]
    fn album_is_omitted_when_absent() {
        let m = metadata("A", "T", None);
        assert!(m.get("release_name").is_none());
        let m = metadata("A", "T", Some("Rec"));
        assert_eq!(m["release_name"], "Rec");
    }
}
