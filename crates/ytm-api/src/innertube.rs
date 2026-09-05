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
use ytm_core::{BrowseId, Rating, Row, Track, VideoId};

use crate::{json, parse};

const ORIGIN: &str = "https://music.youtube.com";
const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20250801.01.00";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
/// InnerTube search filters. These are opaque protobuf blobs; if one goes
/// stale the search falls back to unfiltered results rather than showing
/// nothing.
const FILTER_SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";
const FILTER_ALBUMS: &str = "EgWKAQIYAWoKEAkQChAFEAMQBA%3D%3D";
const FILTER_ARTISTS: &str = "EgWKAQIgAWoKEAkQChAFEAMQBA%3D%3D";
const FILTER_PLAYLISTS: &str = "EgWKAQIoAWoKEAkQChAFEAMQBA%3D%3D";

/// Which tab of the search results to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFilter {
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl SearchFilter {
    pub fn params(self) -> &'static str {
        match self {
            SearchFilter::Songs => FILTER_SONGS,
            SearchFilter::Albums => FILTER_ALBUMS,
            SearchFilter::Artists => FILTER_ARTISTS,
            SearchFilter::Playlists => FILTER_PLAYLISTS,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SearchFilter::Songs => "Songs",
            SearchFilter::Albums => "Albums",
            SearchFilter::Artists => "Artists",
            SearchFilter::Playlists => "Playlists",
        }
    }
    pub const ALL: [SearchFilter; 4] = [
        SearchFilter::Songs,
        SearchFilter::Albums,
        SearchFilter::Artists,
        SearchFilter::Playlists,
    ];
}

/// A discovery surface, addressed by its InnerTube browse id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExploreSection {
    NewReleases,
    Charts,
    Explore,
}

impl ExploreSection {
    pub fn browse_id(self) -> &'static str {
        match self {
            ExploreSection::NewReleases => "FEmusic_new_releases",
            ExploreSection::Charts => "FEmusic_charts",
            ExploreSection::Explore => "FEmusic_explore",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ExploreSection::NewReleases => "New releases",
            ExploreSection::Charts => "Charts",
            ExploreSection::Explore => "Explore",
        }
    }
    pub const ALL: [ExploreSection; 3] =
        [ExploreSection::Explore, ExploreSection::NewReleases, ExploreSection::Charts];
}

/// A library section, addressed by its InnerTube browse id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibrarySection {
    Liked,
    Playlists,
    Albums,
    Artists,
    History,
}

impl LibrarySection {
    pub fn browse_id(self) -> &'static str {
        match self {
            LibrarySection::Liked => "FEmusic_liked_videos",
            LibrarySection::Playlists => "FEmusic_liked_playlists",
            LibrarySection::Albums => "FEmusic_liked_albums",
            LibrarySection::Artists => "FEmusic_library_corpus_artists",
            LibrarySection::History => "FEmusic_history",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LibrarySection::Liked => "Liked songs",
            LibrarySection::Playlists => "Playlists",
            LibrarySection::Albums => "Albums",
            LibrarySection::Artists => "Artists",
            LibrarySection::History => "History",
        }
    }
    pub const ALL: [LibrarySection; 5] = [
        LibrarySection::Liked,
        LibrarySection::Playlists,
        LibrarySection::Albums,
        LibrarySection::Artists,
        LibrarySection::History,
    ];
}

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

    /// Search one results tab. Works anonymously.
    pub fn search(&self, query: &str, filter: SearchFilter) -> Result<Vec<Row>> {
        let v = self.post("search", json!({ "query": query, "params": filter.params() }))?;
        let rows = parse::page_rows(&v);
        if !rows.is_empty() {
            return Ok(rows);
        }
        // The filter blob may have gone stale; unfiltered beats empty.
        let v = self.post("search", json!({ "query": query }))?;
        Ok(parse::page_rows(&v))
    }

    /// Songs only, as a track list - used for "play these results".
    pub fn search_songs(&self, query: &str) -> Result<Vec<Track>> {
        Ok(self
            .search(query, SearchFilter::Songs)?
            .into_iter()
            .filter_map(|r| match r {
                Row::Track(t) => Some(t),
                _ => None,
            })
            .collect())
    }

    /// The account's Home feed: Quick picks, Listen again, mixes.
    pub fn home(&self) -> Result<Vec<Row>> {
        let v = self.post("browse", json!({ "browseId": "FEmusic_home" }))?;
        Ok(parse::page_rows(&v))
    }

    /// Fetch an arbitrary browse id. Used by the `probe` diagnostic to find
    /// which ids are still live.
    pub fn browse_raw(&self, browse_id: &str) -> Result<Vec<Row>> {
        let v = self.post("browse", json!({ "browseId": browse_id }))?;
        let rows = parse::page_rows(&v);
        if rows.is_empty() {
            return Ok(parse::flat_rows(&v));
        }
        Ok(rows)
    }

    /// A library section, or the play history.
    pub fn library(&self, section: LibrarySection) -> Result<Vec<Row>> {
        self.browse_raw(section.browse_id())
    }

    /// A discovery surface: Explore, New releases, Charts.
    pub fn explore(&self, section: ExploreSection) -> Result<Vec<Row>> {
        self.browse_raw(section.browse_id())
    }

    /// Radio / autoplay queue seeded from a track, i.e. what the official
    /// player continues with when a queue runs dry.
    pub fn radio(&self, seed: &VideoId) -> Result<Vec<Track>> {
        let v = self.watch_next(seed)?;
        let mut tracks: Vec<Track> = parse::flat_rows_from_queue(&v);
        // The mix includes the seed itself, not always first; the caller
        // already has it queued.
        tracks.retain(|t| t.id != *seed);
        Ok(tracks)
    }

    /// The Liked Songs auto-playlist.
    pub fn liked_songs(&self) -> Result<Vec<Track>> {
        Ok(self
            .library(LibrarySection::Liked)?
            .into_iter()
            .filter_map(|r| match r {
                Row::Track(t) => Some(t),
                _ => None,
            })
            .collect())
    }

    /// An artist page: top songs, albums, singles.
    pub fn artist(&self, id: &BrowseId) -> Result<(String, Vec<Row>)> {
        let v = self.post("browse", json!({ "browseId": id.0 }))?;
        Ok((page_title(&v).unwrap_or_else(|| "Artist".into()), parse::page_rows(&v)))
    }

    /// An album page: its tracklist.
    pub fn album(&self, id: &BrowseId) -> Result<(String, Vec<Row>)> {
        let v = self.post("browse", json!({ "browseId": id.0 }))?;
        let mut rows = parse::flat_rows(&v);
        if rows.is_empty() {
            rows = parse::page_rows(&v);
        }
        // An album tracklist omits the artist on every row, since it is the
        // same throughout; take it from the header so rows are not blank.
        if let Some(artist) = page_artist(&v) {
            for r in rows.iter_mut() {
                if let Row::Track(t) = r {
                    if t.artist.trim().is_empty() {
                        t.artist = artist.clone();
                    }
                    if t.album.is_none() {
                        t.album = page_title(&v);
                    }
                }
            }
        }
        Ok((page_title(&v).unwrap_or_else(|| "Album".into()), rows))
    }

    /// A playlist page. Accepts either a browse id ("VL...") or a raw
    /// playlist id, which is not interchangeable with it.
    pub fn playlist(&self, id: &BrowseId) -> Result<(String, Vec<Row>)> {
        let browse = if id.0.starts_with("VL") || id.0.starts_with("FE") {
            id.0.clone()
        } else {
            format!("VL{}", id.0)
        };
        let v = self.post("browse", json!({ "browseId": browse }))?;
        let mut rows = parse::flat_rows(&v);
        if rows.is_empty() {
            rows = parse::page_rows(&v);
        }
        Ok((page_title(&v).unwrap_or_else(|| "Playlist".into()), rows))
    }

    /// Signed-in account name, if the response carries one.
    pub fn account_name(&self) -> Result<Option<String>> {
        let v = self.post("account/account_menu", json!({}))?;
        Ok(json::find(&v, "accountName").and_then(json::text))
    }

    /// The true rating and library state for a track (FR-R7), so the UI shows
    /// what the account actually holds rather than a guess.
    pub fn track_state(&self, id: &VideoId) -> Result<(Rating, Option<String>, Option<String>, bool)> {
        let v = self.watch_next(id)?;
        let mut items = Vec::new();
        json::find_all(&v, "playlistPanelVideoRenderer", &mut items);
        let me = items
            .iter()
            .find(|i| i.get("videoId").and_then(|x| x.as_str()) == Some(id.0.as_str()))
            .copied()
            .or_else(|| items.first().copied());
        match me.and_then(parse::track_from) {
            Some(t) => Ok((t.rating, t.feedback_token_add, t.feedback_token_remove, t.in_library)),
            None => Ok((Rating::Indifferent, None, None, false)),
        }
    }

    fn watch_next(&self, seed: &VideoId) -> Result<Value> {
        self.post(
            "next",
            json!({
                "videoId": seed.0,
                // "RDAMVM<id>" is the song-radio mix for a given track.
                "playlistId": format!("RDAMVM{}", seed.0),
                "isAudioOnly": true,
            }),
        )
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

    /// Add to or remove from the library. This is a different operation from
    /// thumbs-up, with its own opaque per-item token: liking a song puts it in
    /// Liked Songs, it does not put it in the library.
    pub fn set_library(&self, token: &str) -> Result<()> {
        if self.auth.is_none() {
            bail!("not signed in");
        }
        self.post("feedback", json!({ "feedbackTokens": [token] }))?;
        Ok(())
    }
}

/// The artist credited in a page header, used to fill in album tracklists.
fn page_artist(v: &Value) -> Option<String> {
    for key in ["musicDetailHeaderRenderer", "musicResponsiveHeaderRenderer"] {
        if let Some(h) = json::find(v, key) {
            let runs = h.get("subtitle").map(json::runs).unwrap_or_default();
            if let Some(a) = runs.iter().find(|s| {
                let s = s.trim();
                !s.is_empty()
                    && s != "\u{2022}"
                    && !matches!(s, "Album" | "Single" | "EP" | "Playlist")
                    && !(s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()))
            }) {
                return Some(a.trim().to_string());
            }
        }
    }
    None
}

/// Best-effort page heading for an artist/album/playlist response.
fn page_title(v: &Value) -> Option<String> {
    for key in ["musicDetailHeaderRenderer", "musicImmersiveHeaderRenderer", "musicResponsiveHeaderRenderer"] {
        if let Some(h) = json::find(v, key) {
            if let Some(t) = h.get("title").and_then(json::text) {
                return Some(t);
            }
        }
    }
    json::find(v, "header").and_then(|h| json::find(h, "title")).and_then(json::text)
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()?)
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
