//! Minimal InnerTube client: unauthenticated reads (search) plus the
//! cookie-authenticated surface rustypipe does not provide (ratings, library).
//!
//! Kept deliberately small and defensive. Response shapes change without
//! notice, so parsing failures degrade to empty results rather than panics.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ytm_core::{Rating, Track, VideoId};

use crate::json;

const ORIGIN: &str = "https://music.youtube.com";
const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20250801.01.00";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
/// InnerTube filter selecting only songs in search results.
const FILTER_SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";

pub struct Innertube {
    http: reqwest::blocking::Client,
    /// None when running anonymously; search still works.
    auth: Option<Auth>,
}

struct Auth {
    cookie_header: String,
    sapisid: String,
}

impl Innertube {
    /// Anonymous client. Search and playback work; account features do not.
    pub fn anonymous() -> Result<Self> {
        Ok(Self { http: client()?, auth: None })
    }

    /// Authenticated from a raw `Cookie:` header value or Netscape cookies.txt.
    pub fn from_cookies(raw: &str) -> Result<Self> {
        let jar = parse_cookies(raw);
        let sapisid = jar
            .get("__Secure-3PAPISID")
            .or_else(|| jar.get("SAPISID"))
            .or_else(|| jar.get("__Secure-1PAPISID"))
            .context(
                "no __Secure-3PAPISID / SAPISID cookie found - \
                 export cookies for music.youtube.com while signed in",
            )?
            .clone();
        let cookie_header = jar
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        Ok(Self {
            http: client()?,
            auth: Some(Auth { cookie_header, sapisid }),
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.auth.is_some()
    }

    /// `SAPISIDHASH <ts>_<sha1(ts + " " + sapisid + " " + origin)>`
    fn auth_header(a: &Auth) -> String {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut h = Sha1::new();
        h.update(format!("{ts} {} {ORIGIN}", a.sapisid).as_bytes());
        format!("SAPISIDHASH {ts}_{}", hex::encode(h.finalize()))
    }

    fn post(&self, endpoint: &str, mut body: Value) -> Result<Value> {
        body["context"] = json!({
            "client": {
                "clientName": CLIENT_NAME,
                "clientVersion": CLIENT_VERSION,
                "hl": "en",
                "gl": "US",
            },
            "user": {},
        });

        let url = format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false");
        let mut req = self
            .http
            .post(&url)
            .header("x-origin", ORIGIN)
            .header("Origin", ORIGIN)
            .header("Referer", ORIGIN)
            .header("Content-Type", "application/json");

        if let Some(a) = &self.auth {
            req = req
                .header("Cookie", &a.cookie_header)
                .header("Authorization", Self::auth_header(a))
                .header("X-Goog-AuthUser", "0");
        }

        let resp = req.json(&body).send()?;
        let status = resp.status();
        let text = resp.text()?;
        if !status.is_success() {
            bail!("{endpoint} -> HTTP {status}: {}", &text[..text.len().min(240)]);
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    // ---- reads -------------------------------------------------------------

    /// Search for songs. Works anonymously.
    pub fn search_songs(&self, query: &str) -> Result<Vec<Track>> {
        let v = self.post(
            "search",
            json!({ "query": query, "params": FILTER_SONGS }),
        )?;
        let mut tracks = parse_tracks(&v);
        if tracks.is_empty() {
            // The songs filter is a magic constant that can go stale; retry
            // unfiltered rather than showing the user nothing.
            let v = self.post("search", json!({ "query": query }))?;
            tracks = parse_tracks(&v);
        }
        Ok(tracks)
    }

    /// Signed-in account name, if the response carries one.
    pub fn account_name(&self) -> Result<Option<String>> {
        let v = self.post("account/account_menu", json!({}))?;
        Ok(json::find(&v, "accountName").and_then(json::text))
    }

    /// The Liked Songs auto-playlist.
    pub fn liked_songs(&self) -> Result<Vec<Track>> {
        let v = self.post("browse", json!({ "browseId": "FEmusic_liked_videos" }))?;
        Ok(parse_tracks(&v))
    }

    // ---- writes ------------------------------------------------------------

    /// Thumbs up / down / clear. Requires authentication; mutates the account.
    pub fn rate(&self, id: &VideoId, rating: Rating) -> Result<()> {
        if self.auth.is_none() {
            bail!("not signed in");
        }
        let endpoint = match rating {
            Rating::Like => "like/like",
            Rating::Dislike => "like/dislike",
            Rating::Indifferent => "like/removelike",
        };
        self.post(endpoint, json!({ "target": { "videoId": id.0 } }))?;
        Ok(())
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()?)
}

/// Extract every track-shaped item from any InnerTube response.
fn parse_tracks(v: &Value) -> Vec<Track> {
    let mut items = Vec::new();
    json::find_all(v, "musicResponsiveListItemRenderer", &mut items);

    let mut out = Vec::new();
    for it in items {
        let Some(id) = json::find(it, "videoId").and_then(|x| x.as_str()) else {
            continue; // albums/artists have no videoId - not playable directly
        };
        let vid = VideoId(id.to_string());
        if !vid.is_valid() {
            continue;
        }

        let cols: Vec<&Value> = it
            .get("flexColumns")
            .and_then(|c| c.as_array())
            .map(|a| a.iter().collect())
            .unwrap_or_default();

        let col_text = |i: usize| -> Option<&Value> {
            cols.get(i)
                .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
                .and_then(|c| c.get("text"))
        };

        let title = col_text(0).and_then(json::text).unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        // Column 1 is typically "Artist • Album • 3:33".
        let sub = col_text(1).map(json::runs).unwrap_or_default();
        let parts: Vec<&String> = sub.iter().filter(|s| s.trim() != "\u{2022}" && !s.trim().is_empty()).collect();

        // Search puts the duration in the last run of column 1; library and
        // playlist responses put it in `fixedColumns` instead. Try both, then
        // fall back to scanning the item, so a shape change costs metadata
        // rather than the whole row.
        let duration = parts
            .last()
            .and_then(|s| json::parse_duration(s))
            .or_else(|| it.get("fixedColumns").and_then(json::find_duration))
            .or_else(|| json::find_duration(it))
            .map(Duration::from_secs);
        let artist = parts.first().map(|s| s.trim().to_string()).unwrap_or_default();
        let album = if parts.len() >= 3 {
            Some(parts[parts.len() - 2].trim().to_string())
        } else {
            None
        };

        // Dedupe: search shelves repeat items across sections.
        if out.iter().any(|t: &Track| t.id == vid) {
            continue;
        }
        out.push(Track {
            id: vid,
            title,
            artist,
            album,
            duration,
            feedback_token_add: None,
            feedback_token_remove: None,
            rating: Rating::Indifferent,
        });
    }
    out
}

fn parse_cookies(raw: &str) -> BTreeMap<String, String> {
    let mut jar = BTreeMap::new();
    // Netscape cookies.txt: 7 tab-separated fields, name [5], value [6].
    if raw.lines().any(|l| l.split('\t').count() >= 7) {
        for line in raw.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 7 {
                jar.insert(f[5].trim().to_string(), f[6].trim().to_string());
            }
        }
        if !jar.is_empty() {
            return jar;
        }
    }
    for part in raw.trim().trim_start_matches("Cookie:").split(';') {
        if let Some((k, v)) = part.split_once('=') {
            jar.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    jar
}
