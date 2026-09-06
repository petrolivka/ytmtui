//! Capture real InnerTube responses as test fixtures.
//!
//! Anonymous only, deliberately: fixtures are committed, and an authenticated
//! response is full of personal data. That also means the fixtures exercise
//! exactly the shapes an unauthenticated user sees, which is the majority of
//! the parser surface.
//!
//!   cargo run --release --bin dump-fixtures -- crates/ytm-api/tests/fixtures

use anyhow::{Context, Result};
use ytm_api::{Innertube, SearchFilter};

/// Remove tracking and session fields.
///
/// Two reasons: they are most of the bytes, and they carry per-session
/// identifiers that have no business in a committed file. Nothing here is read
/// by the parsers.
fn strip_noise(v: &mut serde_json::Value) {
    const DROP: &[&str] = &[
        "trackingParams",
        "clickTrackingParams",
        "loggingContext",
        "loggingDirectives",
        "visitorData",
        "sessionId",
        "serializedShareEntity",
        "responseContext",
        "accessibility",
        "accessibilityData",
        "trackingUrls",
        "playerParams",
        "adPlacements",
        "adSlots",
        "frameworkUpdates",
    ];
    match v {
        serde_json::Value::Object(m) => {
            m.retain(|k, _| !DROP.contains(&k.as_str()));
            for x in m.values_mut() {
                strip_noise(x);
            }
        }
        serde_json::Value::Array(a) => {
            for x in a {
                strip_noise(x);
            }
        }
        _ => {}
    }
}

fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/ytm-api/tests/fixtures".into());
    std::fs::create_dir_all(&dir)?;

    let yt = Innertube::anonymous().context("anonymous client")?;
    let mut wrote = 0usize;

    let mut save = |name: &str, v: &serde_json::Value| -> Result<()> {
        // Compact, not pretty: these are large and read by machine.
        let path = format!("{dir}/{name}.json");
        let mut v = v.clone();
        strip_noise(&mut v);
        let body = serde_json::to_string(&v)?;
        std::fs::write(&path, &body)?;
        println!("  {:<22} {:>8} KiB", name, body.len() / 1024);
        wrote += 1;
        Ok(())
    };

    for (name, filter) in [
        ("search_songs", SearchFilter::Songs),
        ("search_albums", SearchFilter::Albums),
        ("search_artists", SearchFilter::Artists),
        ("search_playlists", SearchFilter::Playlists),
    ] {
        save(name, &yt.debug_search("aphex twin", filter)?)?;
    }

    for (name, id) in [
        ("browse_charts", "FEmusic_charts"),
        ("browse_new_releases", "FEmusic_new_releases"),
        ("browse_explore", "FEmusic_explore"),
        ("browse_home", "FEmusic_home"),
        ("browse_moods", "FEmusic_moods_and_genres"),
    ] {
        save(name, &yt.debug_browse(id)?)?;
    }

    // An artist and an album reached from search, so the ids stay valid as
    // long as the search does.
    let artists = yt.search("aphex twin", SearchFilter::Artists)?;
    if let Some(id) = artists.rows.iter().find_map(|r| match r {
        ytm_core::Row::Artist(a) => Some(a.id.clone()),
        _ => None,
    }) {
        save("browse_artist", &yt.debug_browse(&id.0)?)?;
    }
    let albums = yt.search("selected ambient works", SearchFilter::Albums)?;
    if let Some(id) = albums.rows.iter().find_map(|r| match r {
        ytm_core::Row::Album(a) => Some(a.id.clone()),
        _ => None,
    }) {
        save("browse_album", &yt.debug_browse(&id.0)?)?;
    }

    save(
        "watch_next",
        &yt.debug_next(&ytm_core::VideoId("sWcLccMuCA8".into()))?,
    )?;

    println!("\n{wrote} fixtures written to {dir}");
    Ok(())
}
