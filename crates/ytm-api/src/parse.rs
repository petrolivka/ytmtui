//! Turning InnerTube renderer trees into `Row`s.
//!
//! Every surface - Home, search, library, artist/album/playlist pages - is
//! built from the same handful of renderers, so one parser serves all of them.
//! Everything here is defensive: an unrecognised renderer yields nothing rather
//! than panicking, because these shapes change without notice (R1).

use serde_json::Value;
use std::time::Duration;
use ytm_core::{AlbumRef, ArtistRef, BrowseId, PlaylistId, PlaylistRef, Rating, Row, Track, VideoId};

use crate::json;

/// What a `browseEndpoint` points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageType {
    Album,
    Artist,
    Playlist,
    Unknown,
}

fn page_type(endpoint: &Value) -> PageType {
    // `pageType` sits under navigationEndpoint.browseEndpoint.
    // browseEndpointContextSupportedConfigs.browseEndpointContextMusicConfig -
    // deep enough that reaching for it by a fixed path is how it gets missed.
    let t = json::find(endpoint, "pageType")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    match t {
        "MUSIC_PAGE_TYPE_ALBUM" => PageType::Album,
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" => PageType::Artist,
        "MUSIC_PAGE_TYPE_PLAYLIST" => PageType::Playlist,
        _ => PageType::Unknown,
    }
}

fn browse_id(endpoint: &Value) -> Option<BrowseId> {
    endpoint
        .get("browseEndpoint")
        .or(Some(endpoint))
        .and_then(|b| b.get("browseId"))
        .and_then(|b| b.as_str())
        .map(|s| BrowseId(s.to_string()))
}

/// The token that fetches the next page of a list, if there is one.
///
/// InnerTube has used several shapes for this over time; accept any of them
/// rather than tying paging to one.
pub fn continuation(v: &Value) -> Option<String> {
    if let Some(t) = json::find(v, "continuationCommand")
        .and_then(|c| c.get("token"))
        .and_then(|c| c.as_str())
    {
        return Some(t.to_string());
    }
    if let Some(t) = json::find(v, "nextContinuationData")
        .and_then(|c| c.get("continuation"))
        .and_then(|c| c.as_str())
    {
        return Some(t.to_string());
    }
    None
}

/// Walk a browse response and produce the rows for its main pane.
pub fn page_rows(v: &Value) -> Vec<Row> {
    let mut out = Vec::new();
    let mut shelves = Vec::new();

    for key in [
        "musicCarouselShelfRenderer",
        "musicShelfRenderer",
        "musicPlaylistShelfRenderer",
        "gridRenderer",
    ] {
        let mut found = Vec::new();
        json::find_all(v, key, &mut found);
        for f in found {
            shelves.push((key, f));
        }
    }

    for (kind, shelf) in shelves {
        let title = shelf_title(shelf);
        let items = shelf_items(kind, shelf);
        let rows: Vec<Row> = items.iter().filter_map(|i| item_row(i)).collect();
        if rows.is_empty() {
            continue;
        }
        if let Some(t) = title {
            if !t.trim().is_empty() {
                out.push(Row::Header(t));
            }
        }
        out.extend(rows);
    }

    dedupe(out)
}

/// Tracks from a watch-queue response.
pub fn flat_rows_from_queue(v: &Value) -> Vec<Track> {
    let mut items = Vec::new();
    json::find_all(v, "playlistPanelVideoRenderer", &mut items);
    let mut out: Vec<Track> = Vec::new();
    for it in items {
        if let Some(t) = track_from(it) {
            if !out.iter().any(|x| x.id == t.id) {
                out.push(t);
            }
        }
    }
    out
}

/// Rows from a response that is a single flat track list (playlist, album,
/// library section), without shelf headers.
pub fn flat_rows(v: &Value) -> Vec<Row> {
    let mut items = Vec::new();
    json::find_all(v, "musicResponsiveListItemRenderer", &mut items);
    json::find_all(v, "musicTwoRowItemRenderer", &mut items);
    dedupe(items.iter().filter_map(|i| item_row(i)).collect())
}

fn shelf_title(shelf: &Value) -> Option<String> {
    for key in [
        "musicCarouselShelfBasicHeaderRenderer",
        "gridHeaderRenderer",
        "musicShelfRenderer",
    ] {
        if let Some(h) = json::find(shelf, key) {
            if let Some(t) = h.get("title").and_then(json::text) {
                return Some(t);
            }
        }
    }
    shelf.get("title").and_then(json::text)
}

fn shelf_items<'a>(kind: &str, shelf: &'a Value) -> Vec<&'a Value> {
    let key = if kind == "gridRenderer" { "items" } else { "contents" };
    let mut out = Vec::new();
    if let Some(list) = shelf.get(key).and_then(|c| c.as_array()) {
        for entry in list {
            if let Some(x) = entry.get("musicResponsiveListItemRenderer") {
                out.push(x);
            } else if let Some(x) = entry.get("musicTwoRowItemRenderer") {
                out.push(x);
            }
        }
    }
    out
}

/// A single card or list row.
pub fn item_row(item: &Value) -> Option<Row> {
    // Playable? Then it is a track, whatever renderer it came from.
    if let Some(t) = track_from(item) {
        return Some(Row::Track(t));
    }

    let nav = item
        .get("navigationEndpoint")
        .or_else(|| item.get("title").and_then(|t| t.get("navigationEndpoint")))
        .or_else(|| json::find(item, "navigationEndpoint"))?;

    let id = browse_id(nav)?;
    let title = item
        .get("title")
        .and_then(json::text)
        .or_else(|| flex_text(item, 0))?;
    if title.trim().is_empty() {
        return None;
    }
    let subtitle = item
        .get("subtitle")
        .and_then(json::text)
        .or_else(|| flex_text(item, 1))
        .unwrap_or_default();

    match page_type(nav) {
        PageType::Album => {
            let (artist, year) = split_album_subtitle(&subtitle);
            Some(Row::Album(AlbumRef { id, title, artist, year }))
        }
        PageType::Artist => Some(Row::Artist(ArtistRef { id, name: title, subtitle })),
        PageType::Playlist => Some(Row::Playlist(PlaylistRef {
            playlist_id: id.0.strip_prefix("VL").map(|p| PlaylistId(p.to_string())),
            id,
            title,
            subtitle,
        })),
        PageType::Unknown => None,
    }
}

/// Album subtitles vary by surface: "Album • Aphex Twin • 1992" in search, but
/// just "2001" on an artist page, where the artist is implied. Pulling the
/// first field blindly puts a year in the artist column.
fn split_album_subtitle(s: &str) -> (String, Option<String>) {
    let is_year = |x: &str| x.len() == 4 && x.chars().all(|c| c.is_ascii_digit());
    let is_kind = |x: &str| {
        matches!(x, "Album" | "Single" | "EP" | "Playlist" | "Artist" | "Song" | "Video")
    };
    let parts: Vec<&str> = s
        .split('\u{2022}')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    let year = parts.iter().rev().find(|p| is_year(p)).map(|p| p.to_string());
    let artist = parts
        .iter()
        .find(|p| !is_kind(p) && !is_year(p))
        .map(|p| p.to_string())
        .unwrap_or_default();
    (artist, year)
}

#[cfg(test)]
mod tests {
    use super::split_album_subtitle;

    #[test]
    fn album_subtitles() {
        assert_eq!(
            split_album_subtitle("Album \u{2022} Aphex Twin \u{2022} 1992"),
            ("Aphex Twin".into(), Some("1992".into()))
        );
        // Artist pages omit the artist; the year must not land in its place.
        assert_eq!(split_album_subtitle("2001"), (String::new(), Some("2001".into())));
        assert_eq!(split_album_subtitle("Single \u{2022} 2024"), (String::new(), Some("2024".into())));
    }
}

fn flex_col<'a>(item: &'a Value, i: usize) -> Option<&'a Value> {
    item.get("flexColumns")?
        .as_array()?
        .get(i)?
        .get("musicResponsiveListItemFlexColumnRenderer")?
        .get("text")
}

fn flex_text(item: &Value, i: usize) -> Option<String> {
    flex_col(item, i).and_then(json::text)
}

/// Build a `Track` if this item is playable.
pub fn track_from(item: &Value) -> Option<Track> {
    // The watch endpoint is authoritative; `playlistItemData` covers rows where
    // the endpoint sits on a nested column.
    let id = item
        .get("playlistItemData")
        .and_then(|d| d.get("videoId"))
        .and_then(|d| d.as_str())
        .or_else(|| {
            json::find(item, "watchEndpoint")
                .and_then(|w| w.get("videoId"))
                .and_then(|w| w.as_str())
        })
        .or_else(|| item.get("videoId").and_then(|d| d.as_str()))?;

    let vid = VideoId(id.to_string());
    if !vid.is_valid() {
        return None;
    }

    let title = item
        .get("title")
        .and_then(json::text)
        .or_else(|| flex_text(item, 0))
        .unwrap_or_default();
    if title.trim().is_empty() {
        return None;
    }

    // Column 1 is "Artist • Album • 3:33"; watch-queue rows use a byline.
    let runs = flex_col(item, 1)
        .map(json::runs)
        .filter(|r| !r.is_empty())
        .or_else(|| {
            item.get("longBylineText")
                .or_else(|| item.get("shortBylineText"))
                .or_else(|| item.get("subtitle"))
                .map(json::runs)
                .filter(|r| !r.is_empty())
        })
        .unwrap_or_default();
    let parts: Vec<&String> = runs
        .iter()
        .filter(|s| s.trim() != "\u{2022}" && !s.trim().is_empty())
        .collect();

    let duration = parts
        .last()
        .and_then(|s| json::parse_duration(s))
        .or_else(|| item.get("lengthText").and_then(json::find_duration))
        .or_else(|| item.get("fixedColumns").and_then(json::find_duration))
        .or_else(|| json::find_duration(item))
        .map(Duration::from_secs);

    let artist = parts
        .iter()
        .find(|s| json::parse_duration(s).is_none())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let album = if parts.len() >= 3 {
        Some(parts[parts.len() - 2].trim().to_string()).filter(|a| json::parse_duration(a).is_none())
    } else {
        None
    };

    // Navigation targets, for "go to artist" / "go to album" (FR-B5).
    let (mut artist_id, mut album_id) = (None, None);
    let nav_runs = flex_col(item, 1)
        .or_else(|| item.get("subtitle"))
        .or_else(|| item.get("longBylineText"))
        .and_then(|c| c.get("runs"))
        .and_then(|r| r.as_array());
    if let Some(col) = nav_runs {
        for run in col {
            if let Some(nav) = run.get("navigationEndpoint") {
                match page_type(nav) {
                    PageType::Artist if artist_id.is_none() => artist_id = browse_id(nav),
                    PageType::Album if album_id.is_none() => album_id = browse_id(nav),
                    _ => {}
                }
            }
        }
    }

    let (feedback_token_add, feedback_token_remove, in_library) = library_tokens(item);
    let set_video_id = item
        .get("playlistItemData")
        .and_then(|d| d.get("playlistSetVideoId"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());

    Some(Track {
        id: vid,
        title,
        artist,
        album,
        duration,
        feedback_token_add,
        feedback_token_remove,
        rating: like_status(item),
        in_library,
        album_id,
        artist_id,
        set_video_id,
    })
}

/// Library add/remove is driven by opaque per-item feedback tokens, which only
/// appear in authenticated responses.
fn library_tokens(item: &Value) -> (Option<String>, Option<String>, bool) {
    let mut toggles = Vec::new();
    json::find_all(item, "toggleMenuServiceItemRenderer", &mut toggles);
    for t in toggles {
        let default_label = t.get("defaultText").and_then(json::text).unwrap_or_default();
        let add = t
            .get("defaultServiceEndpoint")
            .and_then(|e| json::find(e, "feedbackToken"))
            .and_then(|e| e.as_str())
            .map(|s| s.to_string());
        let remove = t
            .get("toggledServiceEndpoint")
            .and_then(|e| json::find(e, "feedbackToken"))
            .and_then(|e| e.as_str())
            .map(|s| s.to_string());
        if add.is_none() && remove.is_none() {
            continue;
        }
        // "Add to library" means it is not in the library yet; the toggled
        // variant ("Remove from library") means it is.
        let lower = default_label.to_ascii_lowercase();
        if lower.contains("librar") {
            let in_lib = lower.contains("remove");
            return (add, remove, in_lib);
        }
    }
    (None, None, false)
}

fn like_status(item: &Value) -> Rating {
    match json::find(item, "likeStatus").and_then(|s| s.as_str()) {
        Some("LIKE") => Rating::Like,
        Some("DISLIKE") => Rating::Dislike,
        _ => Rating::Indifferent,
    }
}

/// Drop repeated rows while keeping the first occurrence, and drop headers that
/// ended up with nothing under them.
fn dedupe(rows: Vec<Row>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    for r in rows {
        let dup = match &r {
            Row::Header(h) => out.iter().any(|x| matches!(x, Row::Header(y) if y == h)),
            Row::Track(t) => out.iter().any(|x| matches!(x, Row::Track(y) if y.id == t.id)),
            Row::Album(a) => out.iter().any(|x| matches!(x, Row::Album(y) if y.id == a.id)),
            Row::Artist(a) => out.iter().any(|x| matches!(x, Row::Artist(y) if y.id == a.id)),
            Row::Playlist(p) => out.iter().any(|x| matches!(x, Row::Playlist(y) if y.id == p.id)),
        };
        if !dup {
            out.push(r);
        }
    }
    while matches!(out.last(), Some(Row::Header(_))) {
        out.pop();
    }
    out
}
