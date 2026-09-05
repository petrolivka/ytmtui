//! Read-only diagnostic: exercises every InnerTube surface the app uses and
//! reports what actually comes back. Run it after any parser change, and to
//! find out which browse ids have gone stale.

use anyhow::Result;
use ytm_api::{LibrarySection, SearchFilter};
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

fn more(p: &ytm_api::RowPage) -> String {
    if p.continuation.is_some() { "  (+more)".into() } else { String::new() }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "\u{2026}" }
}

fn main() -> Result<()> {
    let yt = ytm_api::load_backend()?;
    println!("authenticated: {}\n", yt.is_authenticated());

    println!("== home ==");
    match yt.home() {
        Ok(p) => { println!("   {}{}", summarise(&p.rows), more(&p)); show(&p.rows, 10); }
        Err(e) => println!("   FAILED: {e}"),
    }

    println!("\n== search tabs ==");
    for f in SearchFilter::ALL {
        match yt.search("aphex twin", f) {
            Ok(p) => { println!("   {:<10} {}{}", f.label(), summarise(&p.rows), more(&p)); show(&p.rows, 3); }
            Err(e) => println!("   {:<10} FAILED: {e}", f.label()),
        }
    }

    println!("\n== library ==");
    let mut first_album: Option<BrowseId> = None;
    let mut first_artist: Option<BrowseId> = None;
    let mut first_playlist: Option<BrowseId> = None;
    for sec in LibrarySection::ALL {
        match yt.library(sec) {
            Ok(p) => {
                println!("   {:<14} {}{}", sec.label(), summarise(&p.rows), more(&p));
                show(&p.rows, 3);
                for row in &p.rows {
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
        if let Ok(p) = yt.search("selected ambient works", SearchFilter::Albums) {
            first_album = p.rows.iter().find_map(|x| match x { Row::Album(a) => Some(a.id.clone()), _ => None });
        }
    }
    if first_artist.is_none() {
        if let Ok(p) = yt.search("aphex twin", SearchFilter::Artists) {
            first_artist = p.rows.iter().find_map(|x| match x { Row::Artist(a) => Some(a.id.clone()), _ => None });
        }
    }
    if first_playlist.is_none() {
        if let Ok(p) = yt.search("ambient", SearchFilter::Playlists) {
            first_playlist = p.rows.iter().find_map(|x| match x { Row::Playlist(q) => Some(q.id.clone()), _ => None });
        }
    }

    if let Some(id) = &first_artist {
        match yt.artist(id) {
            Ok(p) => { println!("   artist   {id} -> \"{}\": {}{}", p.title.clone().unwrap_or_default(), summarise(&p.rows), more(&p)); show(&p.rows, 6); }
            Err(e) => println!("   artist   {id} FAILED: {e}"),
        }
    }
    if let Some(id) = &first_album {
        match yt.album(id) {
            Ok(p) => { println!("   album    {id} -> \"{}\": {}{}", p.title.clone().unwrap_or_default(), summarise(&p.rows), more(&p)); show(&p.rows, 5); }
            Err(e) => println!("   album    {id} FAILED: {e}"),
        }
    }
    if let Some(id) = &first_playlist {
        match yt.playlist(id) {
            Ok(p) => { println!("   playlist {id} -> \"{}\": {}{}", p.title.clone().unwrap_or_default(), summarise(&p.rows), more(&p)); show(&p.rows, 5); }
            Err(e) => println!("   playlist {id} FAILED: {e}"),
        }
    }

    println!("\n== track state (rating + library tokens) ==");
    // Cross-check against the liked list: if a track that is demonstrably in
    // Liked Songs reports Indifferent, the likeStatus parsing is the problem,
    // not the account.
    let liked = yt.library(LibrarySection::Liked).map(|p| p.rows).unwrap_or_default();
    let ids: Vec<ytm_core::VideoId> = std::env::args()
        .skip(1)
        .filter(|a| a.len() == 11)
        .map(ytm_core::VideoId)
        .chain(liked.iter().filter_map(|r| r.as_track()).take(3).map(|t| t.id.clone()))
        .collect();
    for id in ids {
        let in_liked_list = liked.iter().filter_map(|r| r.as_track()).any(|t| t.id == id);
        match yt.track_state(&id) {
            Ok((r, add, rem, in_lib)) => println!(
                "   {id}: rating={r:?} (in Liked list: {in_liked_list})  in_library={in_lib} tokens={}/{}",
                add.is_some(), rem.is_some()
            ),
            Err(e) => println!("   {id}: FAILED: {e}"),
        }
    }
    if std::env::args().any(|a| a == "--where-is-likestatus") {
        let id = ytm_core::VideoId(
            std::env::args().find(|a| a.len() == 11).unwrap_or_else(|| "sWcLccMuCA8".into()),
        );
        let v = yt.debug_next(&id)?;
        let mut hits = Vec::new();
        walk(&v, String::new(), &mut hits);
        println!("occurrences of likeStatus in the watch-next response:");
        for (path, val) in hits.iter().take(20) {
            println!("   {val:<14} {path}");
        }
        if hits.is_empty() {
            println!("   none - likeStatus is not in this response at all");
        }
        return Ok(());
    }
    println!("\n== cover art ==");
    if let Ok(p) = yt.search("aphex twin", SearchFilter::Songs) {
        for t in p.rows.iter().filter_map(|r| r.as_track()).take(3) {
            match &t.thumbnail {
                Some(u) => println!("   {:<26} {}", trunc(&t.title, 26), trunc(u, 78)),
                None => println!("   {:<26} (no thumbnail)", trunc(&t.title, 26)),
            }
        }
    }

    println!("\n== synced lyrics (LRCLIB) ==");
    for (artist, title, secs) in [
        ("Daft Punk", "Around the World", 428u64),
        ("Boards of Canada", "Roygbiv", 150),
        ("Agnes Obel", "Familiar", 236),
    ] {
        let mut t = ytm_core::Track::new("00000000000", title, artist);
        t.duration = Some(std::time::Duration::from_secs(secs));
        match yt.synced_lyrics(&t) {
            Ok(Some(l)) => {
                println!("   {artist} - {title}: {} synced lines", l.len());
                for line in l.iter().take(3) {
                    println!("      [{:>6.2}s] {}", line.at.as_secs_f32(), trunc(&line.text, 50));
                }
            }
            Ok(None) => println!("   {artist} - {title}: no synced lyrics"),
            Err(e) => println!("   {artist} - {title}: FAILED: {e}"),
        }
    }

    println!("\n== lyrics ==");
    for id in ["sWcLccMuCA8", "EnjOz4wtS8Q"] {
        let vid = ytm_core::VideoId(id.into());
        match yt.lyrics(&vid) {
            Ok(Some(t)) => println!("   {id}: {} chars \u{2014} {:?}\u{2026}", t.len(), trunc(&t.replace('\n', " / "), 70)),
            Ok(None) => println!("   {id}: no lyrics available"),
            Err(e) => println!("   {id}: FAILED: {e}"),
        }
    }

    probe_candidates(&yt);
    Ok(())
}

/// Record the JSON path of every `likeStatus` so it is located from evidence
/// rather than guessed at.
fn walk(v: &serde_json::Value, path: String, out: &mut Vec<(String, String)>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                if k == "likeStatus" {
                    out.push((format!("{path}.{k}"), x.as_str().unwrap_or("?").to_string()));
                }
                walk(x, format!("{path}.{k}"), out);
            }
        }
        serde_json::Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                walk(x, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
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
