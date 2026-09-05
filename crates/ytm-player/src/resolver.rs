//! Stream resolution, with caching and resolve-ahead.
//!
//! The trait is the hedge: if one implementation is broken by a Google-side
//! change, another can be selected at runtime without touching the player.
//! M0 measured a cold `yt-dlp` resolve at ~3.4 s, which is fine for a fallback
//! and far too slow to sit in the interactive path - hence FR-P12, the cache
//! and the resolve-ahead worker below.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use ytm_core::VideoId;

#[derive(Debug, Clone)]
pub struct Format {
    pub itag: String,
    pub codec: String,
    pub abr_kbps: f64,
    pub url: String,
    /// Unix time at which the URL stops working, if the URL declares one.
    pub expires_at: Option<u64>,
}

impl Format {
    fn still_valid(&self) -> bool {
        match self.expires_at {
            // Refuse anything within 60s of expiry so a track never dies mid-play.
            Some(e) => now_unix() + 60 < e,
            None => true,
        }
    }
}

pub trait StreamResolver: Send + Sync {
    fn name(&self) -> &'static str;
    fn resolve(&self, id: &VideoId) -> Result<Format>;
}

/// Resolver backed by the `yt-dlp` binary - the fallback that makes the project
/// survivable, since yt-dlp absorbs YouTube-side churn faster than we could.
pub struct YtDlpResolver {
    pub itag_preference: Vec<&'static str>,
}

impl YtDlpResolver {
    /// Preference order comes from the audio quality setting.
    pub fn new(itag_preference: Vec<&'static str>) -> Self {
        Self { itag_preference }
    }
}

impl Default for YtDlpResolver {
    fn default() -> Self {
        // 251 = opus ~160k (best), 140 = aac 128k, then the low tiers.
        Self::new(vec!["251", "140", "250", "249"])
    }
}

impl StreamResolver for YtDlpResolver {
    fn name(&self) -> &'static str {
        "yt-dlp"
    }

    fn resolve(&self, id: &VideoId) -> Result<Format> {
        let url = format!("https://music.youtube.com/watch?v={id}");
        let sel = self.itag_preference.join("/");
        let out = Command::new("yt-dlp")
            .args([
                "--no-warnings", "--quiet",
                "-f", &sel,
                "--print", "%(format_id)s\t%(acodec)s\t%(abr)s\t%(urls)s",
                &url,
            ])
            .output()
            .context("failed to spawn yt-dlp - is it on PATH?")?;

        if !out.status.success() {
            bail!(
                "yt-dlp failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }

        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next().unwrap_or_default();
        let mut p = line.split('\t');
        let itag = p.next().unwrap_or_default().to_string();
        let codec = p.next().unwrap_or_default().to_string();
        let abr = p.next().unwrap_or_default();
        let stream_url = p.next().unwrap_or_default().to_string();

        if stream_url.is_empty() {
            bail!("yt-dlp returned no stream URL for {id}");
        }
        Ok(Format {
            expires_at: parse_expire(&stream_url),
            itag,
            codec,
            abr_kbps: abr.parse().unwrap_or(0.0),
            url: stream_url,
        })
    }
}

/// Caching wrapper implementing FR-P12. Resolved URLs are reused until they are
/// close to expiry, and the next queue item can be resolved in the background
/// while the current one plays.
pub struct ResolverCache {
    inner: Arc<dyn StreamResolver>,
    cache: Arc<Mutex<HashMap<String, Format>>>,
    /// Ids currently being resolved, so a prefetch and a play don't duplicate work.
    inflight: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ResolverCache {
    pub fn new(inner: Arc<dyn StreamResolver>) -> Self {
        Self {
            inner,
            cache: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn backend_name(&self) -> &'static str {
        self.inner.name()
    }

    /// Cached format, if we have a still-valid one.
    pub fn cached(&self, id: &VideoId) -> Option<Format> {
        let c = self.cache.lock().unwrap();
        c.get(&id.0).filter(|f| f.still_valid()).cloned()
    }

    /// Resolve, using the cache when possible. Blocks.
    pub fn resolve(&self, id: &VideoId) -> Result<Format> {
        if let Some(f) = self.cached(id) {
            return Ok(f);
        }
        let f = self.inner.resolve(id)?;
        self.cache.lock().unwrap().insert(id.0.clone(), f.clone());
        Ok(f)
    }

    /// Kick off a background resolve so the *next* track starts instantly.
    pub fn prefetch(self: &Arc<Self>, id: &VideoId) {
        if self.cached(id).is_some() {
            return;
        }
        {
            let mut f = self.inflight.lock().unwrap();
            // Retry a stuck prefetch after 30s, but never pile them up.
            if let Some(t) = f.get(&id.0) {
                if t.elapsed() < Duration::from_secs(30) {
                    return;
                }
            }
            f.insert(id.0.clone(), Instant::now());
        }
        let this = self.clone();
        let id = id.clone();
        std::thread::spawn(move || {
            let r = this.inner.resolve(&id);
            if let Ok(f) = r {
                this.cache.lock().unwrap().insert(id.0.clone(), f);
            }
            this.inflight.lock().unwrap().remove(&id.0);
        });
    }
}

fn parse_expire(url: &str) -> Option<u64> {
    url.split(['?', '&'])
        .find_map(|kv| kv.strip_prefix("expire="))
        .and_then(|v| v.parse().ok())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
