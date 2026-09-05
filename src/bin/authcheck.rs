//! Credentialed self-test: proves the authenticated read AND write paths.
//!
//! Read-only by default. The write test is opt-in because it mutates the real
//! account - see the account-safety risk (R11) in
//! docs/TECH-STACK-RISK-ANALYSIS.md.
//!
//!   authcheck                    # read-only check
//!   authcheck --find <query>     # look up video ids to test with
//!   authcheck --like <videoId>   # like, verify, then restore the previous state

use anyhow::Result;
use ytm_core::{Rating, VideoId};

/// Accept a bare id or any YouTube / YouTube Music URL.
fn extract_video_id(s: &str) -> String {
    if let Some(rest) = s.split("v=").nth(1) {
        return rest.chars().take(11).collect();
    }
    if let Some(rest) = s.split("youtu.be/").nth(1) {
        return rest.chars().take(11).collect();
    }
    s.trim().to_string()
}

fn main() -> Result<()> {
    let yt = ytm_api::load_backend()?;
    if !yt.is_authenticated() {
        eprintln!(
            "not signed in.\n\
             Provide credentials as either:\n  \
               - ~/.config/ytmtui/cookies.txt (Netscape format, exported for\n    \
                 music.youtube.com while signed in), or\n  \
               - env YTM_COOKIE='<raw Cookie header value>'\n\
             The jar must contain __Secure-3PAPISID."
        );
        std::process::exit(1);
    }

    let args: Vec<String> = std::env::args().collect();

    // Look up ids without touching the account, so the write test below can be
    // pointed at a specific track without hunting for an id in a browser.
    if let Some(i) = args.iter().position(|a| a == "--find") {
        let q = args[i + 1..].join(" ");
        if q.trim().is_empty() {
            eprintln!("--find needs a search query");
            std::process::exit(2);
        }
        let liked: std::collections::HashSet<String> = yt
            .liked_songs()
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.id.0)
            .collect();
        for t in yt.search_songs(&q)?.iter().take(10) {
            let mark = if liked.contains(&t.id.0) {
                "already liked"
            } else {
                ""
            };
            println!(
                "  {:11}  {:>7}  {} - {}  {}",
                t.id,
                t.duration_str(),
                t.title,
                t.artist,
                mark
            );
        }
        println!("\n(the first page of Liked Songs was checked; \"already liked\" rows make an inconclusive test)");
        return Ok(());
    }

    // Inspect the autoplay continuation for a track. Read-only.
    if let Some(i) = args.iter().position(|a| a == "--radio") {
        let seed = ytm_core::VideoId(extract_video_id(
            args.get(i + 1).map(|s| s.as_str()).unwrap_or(""),
        ));
        if !seed.is_valid() {
            eprintln!("--radio needs a video id or URL");
            std::process::exit(2);
        }
        let r = yt.radio(&seed)?;
        println!("radio seeded from {seed}: {} tracks", r.len());
        for t in r.iter().take(12) {
            println!(
                "  {:11}  {:>7}  {} - {}",
                t.id,
                t.duration_str(),
                t.title,
                t.artist
            );
        }
        return Ok(());
    }

    // Set a rating explicitly. Exists because an automated UI test once typed
    // a bare 'd' into a signed-in instance and left a thumbs-down behind.
    if let Some(i) = args.iter().position(|a| a == "--rate") {
        let id = VideoId(extract_video_id(
            args.get(i + 1).map(|s| s.as_str()).unwrap_or(""),
        ));
        let want = args.get(i + 2).map(|s| s.as_str()).unwrap_or("none");
        let rating = match want {
            "like" => Rating::Like,
            "dislike" => Rating::Dislike,
            "none" | "clear" | "indifferent" => Rating::Indifferent,
            other => {
                eprintln!("unknown rating '{other}' (use like | dislike | none)");
                std::process::exit(2);
            }
        };
        if !id.is_valid() {
            eprintln!("--rate needs a video id or URL, then like|dislike|none");
            std::process::exit(2);
        }
        println!("  before: {:?}", yt.track_state(&id)?.0);
        yt.rate(&id, rating)?;
        std::thread::sleep(std::time::Duration::from_millis(1200));
        println!("  after : {:?}", yt.track_state(&id)?.0);
        return Ok(());
    }

    println!("== authenticated read ==");
    // A missing account name is cosmetic; do not fail the whole check for it.
    match yt.account_name() {
        Ok(Some(n)) => println!("  signed in as : {n}"),
        Ok(None) => println!("  signed in as : <name not present in response>"),
        Err(e) => println!("  signed in as : <lookup failed: {e}>"),
    }

    let liked = yt.liked_songs()?;
    println!(
        "  liked songs  : {} returned on the first page",
        liked.len()
    );
    for t in liked.iter().take(8) {
        println!(
            "     {:11}  {:>7}  {} - {}",
            t.id,
            t.duration_str(),
            t.title,
            t.artist
        );
    }
    let missing = liked.iter().filter(|t| t.duration.is_none()).count();
    if missing > 0 {
        println!(
            "  note: {missing}/{} rows had no parseable duration",
            liked.len()
        );
    }
    if liked.is_empty() {
        println!("  (empty first page - if you do have liked songs, the parser needs a look)");
    }

    let Some(i) = args.iter().position(|a| a == "--like") else {
        println!(
            "\nread path OK.\n\
             Next: `--find <query>` to get a video id, then `--like <id>` for the write path."
        );
        return Ok(());
    };
    let Some(raw) = args.get(i + 1) else {
        eprintln!("--like needs a videoId, e.g. --like dQw4w9WgXcQ");
        std::process::exit(2);
    };
    let id = VideoId(extract_video_id(raw));
    if !id.is_valid() {
        eprintln!("could not read a video id from '{raw}' - pass an 11-character id or a YouTube/YouTube Music URL");
        std::process::exit(2);
    }

    println!("\n== write path (this mutates your real account) ==");

    // Verify against the track's own rating, not membership of the Liked Music
    // list. That list is a derived auto-playlist and lags behind by minutes, so
    // checking it reports failures that did not happen - and, worse, would hide
    // real ones.
    let before = yt.track_state(&id)?.0;
    println!("  rating before        : {before:?}");

    yt.rate(&id, Rating::Like)?;
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let during = yt.track_state(&id)?.0;
    println!("  rating after like    : {during:?}");

    // Restore exactly what was there. Never clobber a rating the user had.
    yt.rate(&id, before)?;
    std::thread::sleep(std::time::Duration::from_millis(1000));
    let after = yt.track_state(&id)?.0;
    println!("  rating restored to   : {after:?}");

    println!();
    if during == Rating::Like && after == before {
        println!("  WRITE PATH CONFIRMED (liked, then restored)");
    } else if during != Rating::Like {
        println!("  the like did not take effect - check the account in a browser");
    } else {
        println!("  liked, but restoring to {before:?} did not stick - now {after:?}");
    }

    if let Some(i) = args.iter().position(|a| a == "--library") {
        let _ = i;
        println!("\n== library toggle (also mutates the account) ==");
        let (_, add, remove, in_lib) = yt.track_state(&id)?;
        println!("  in library before    : {in_lib}");
        let token = if in_lib { remove.clone() } else { add.clone() };
        match token {
            Some(t) => {
                yt.set_library(&t)?;
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let (_, add2, rem2, now_lib) = yt.track_state(&id)?;
                println!("  in library after     : {now_lib}");
                let back = if now_lib { rem2 } else { add2 };
                if let Some(t2) = back {
                    yt.set_library(&t2)?;
                    println!("  restored");
                }
                if now_lib != in_lib {
                    println!("\n  LIBRARY TOGGLE CONFIRMED");
                } else {
                    println!("\n  library state did not change");
                }
            }
            None => println!("  no feedback token available for this track"),
        }
    }
    Ok(())
}
