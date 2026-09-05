//! Read-only diagnostic: exercises every InnerTube surface the app uses and
//! reports what actually comes back. Run it after any parser change, and to
//! find out which browse ids have gone stale.

use anyhow::Result;
use ytm_api::{LibrarySection, MusicBackend, SearchFilter};
use ytm_core::{BrowseId, Row};

fn summarise(rows: &[Row]) -> String {
    let (mut h, mut t, mut al, mut ar, mut pl) = (0, 0, 0, 0, 0);
    for r in rows {
        match r {
            Row::Header(_) => h += 1,
            Row::Track(_) => t += 1,
            Row::Album(_) => al += 1,
            Row::Artist(_) => ar += 1,
            Row::Playlist(_) => pl += 1,
        }
    }
    format!("{t} tracks, {al} albums, {ar} artists, {pl} playlists, {h} headers")
}

fn show(rows: &[Row], n: usize) {
    for r in rows.iter().take(n) {
        match r {
            Row::Header(t) => println!("      -- {t}"),
            other => println!("      {:<9} {:<40} {}", other.tag(), trunc(other.title(), 40), trunc(&other.subtitle(), 30)),
        }
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "\u{2026}" }
}

fn main() -> Result<()> {
    let yt = ytm_api::load_backend()?;
    println!("authenticated: {}\n", yt.is_authenticated());

    println!("== home ==");
    match yt.home() {
        Ok(r) => { println!("   {}", summarise(&r)); show(&r, 10); }
        Err(e) => println!("   FAILED: {e}"),
    }

    println!("\n== search tabs ==");
    for f in SearchFilter::ALL {
        match yt.search("aphex twin", f) {
            Ok(r) => { println!("   {:<10} {}", f.label(), summarise(&r)); show(&r, 3); }
            Err(e) => println!("   {:<10} FAILED: {e}", f.label()),
        }
    }

    println!("\n== library ==");
    let mut first_album: Option<BrowseId> = None;
    let mut first_artist: Option<BrowseId> = None;
    let mut first_playlist: Option<BrowseId> = None;
    for sec in LibrarySection::ALL {
        match yt.library(sec) {
            Ok(r) => {
                println!("   {:<14} {}", sec.label(), summarise(&r));
                show(&r, 3);
                for row in &r {
                    match row {
                        Row::Album(a) if first_album.is_none() => first_album = Some(a.id.clone()),
                        Row::Artist(a) if first_artist.is_none() => first_artist = Some(a.id.clone()),
                        Row::Playlist(p) if first_playlist.is_none() => first_playlist = Some(p.id.clone()),
                        _ => {}
                    }
                }
            }
            Err(e) => println!("   {:<14} FAILED: {e}", sec.label()),
        }
    }

    println!("\n== entity pages ==");
    // Fall back to search results when the library has nothing of a kind.
    if first_album.is_none() {
        if let Ok(r) = yt.search("selected ambient works", SearchFilter::Albums) {
            first_album = r.iter().find_map(|x| match x { Row::Album(a) => Some(a.id.clone()), _ => None });
        }
    }
    if first_artist.is_none() {
        if let Ok(r) = yt.search("aphex twin", SearchFilter::Artists) {
            first_artist = r.iter().find_map(|x| match x { Row::Artist(a) => Some(a.id.clone()), _ => None });
        }
    }
    if first_playlist.is_none() {
        if let Ok(r) = yt.search("ambient", SearchFilter::Playlists) {
            first_playlist = r.iter().find_map(|x| match x { Row::Playlist(p) => Some(p.id.clone()), _ => None });
        }
    }

    if let Some(id) = &first_artist {
        match yt.artist(id) {
            Ok((t, r)) => { println!("   artist   {id} -> \"{t}\": {}", summarise(&r)); show(&r, 6); }
            Err(e) => println!("   artist   {id} FAILED: {e}"),
        }
    }
    if let Some(id) = &first_album {
        match yt.album(id) {
            Ok((t, r)) => { println!("   album    {id} -> \"{t}\": {}", summarise(&r)); show(&r, 5); }
            Err(e) => println!("   album    {id} FAILED: {e}"),
        }
    }
    if let Some(id) = &first_playlist {
        match yt.playlist(id) {
            Ok((t, r)) => { println!("   playlist {id} -> \"{t}\": {}", summarise(&r)); show(&r, 5); }
            Err(e) => println!("   playlist {id} FAILED: {e}"),
        }
    }

    println!("\n== track state (rating + library tokens) ==");
    let id = ytm_core::VideoId("sWcLccMuCA8".into());
    match yt.track_state(&id) {
        Ok((r, add, rem, in_lib)) => println!(
            "   {id}: rating={r:?} in_library={in_lib} add_token={} remove_token={}",
            add.is_some(), rem.is_some()
        ),
        Err(e) => println!("   FAILED: {e}"),
    }
    probe_candidates(&yt);
    Ok(())
}

/// Candidate browse ids for surfaces whose id may have gone stale.
/// Kept in the diagnostic rather than the client so a dead id is discovered
/// deliberately rather than silently returning an empty page.
fn probe_candidates(yt: &ytm_api::Innertube) {
    let candidates = [
        "FEmusic_library_corpus_track_artists",
        "FEmusic_library_corpus_artists",
        "FEmusic_liked_artists",
        "FEmusic_library_landing",
        "FEmusic_explore",
        "FEmusic_charts",
        "FEmusic_new_releases",
        "FEmusic_moods_and_genres",
    ];
    println!("\n== browse id candidates ==");
    for id in candidates {
        match yt.browse_raw(id) {
            Ok(r) => println!("   {:<40} {}", id, summarise(&r)),
            Err(e) => println!("   {:<40} FAILED: {e}", id),
        }
    }
}
