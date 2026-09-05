//! Credentialed self-test: proves the authenticated read AND write paths.
//!
//! Read-only by default. The write test is opt-in because it mutates the real
//! account - see the account-safety risk (R11) in
//! docs/TECH-STACK-RISK-ANALYSIS.md.
//!
//!   authcheck                    # read-only check
//!   authcheck --like <videoId>   # like, verify, then restore the previous state

use anyhow::Result;
use ytm_core::{Rating, VideoId};

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

    println!("== authenticated read ==");
    // A missing account name is cosmetic; do not fail the whole check for it.
    match yt.account_name() {
        Ok(Some(n)) => println!("  signed in as : {n}"),
        Ok(None) => println!("  signed in as : <name not present in response>"),
        Err(e) => println!("  signed in as : <lookup failed: {e}>"),
    }

    let liked = yt.liked_songs()?;
    println!("  liked songs  : {} returned on the first page", liked.len());
    for t in liked.iter().take(5) {
        println!("     {} - {} [{}]", t.title, t.artist, t.duration_str());
    }
    if liked.is_empty() {
        println!("  (empty first page - if you do have liked songs, the parser needs a look)");
    }

    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--like") else {
        println!("\nread path OK. Pass `--like <videoId>` to exercise the write path.");
        return Ok(());
    };
    let Some(raw) = args.get(i + 1) else {
        eprintln!("--like needs a videoId, e.g. --like dQw4w9WgXcQ");
        std::process::exit(2);
    };
    let id = VideoId(raw.clone());
    if !id.is_valid() {
        eprintln!("'{raw}' is not an 11-character video id");
        std::process::exit(2);
    }

    println!("\n== write path (this mutates your real account) ==");

    // Verify by *presence of this id*, not by list length: the first page is
    // capped, so a count comparison stays flat once the page is full.
    let was_liked = liked.iter().any(|t| t.id == id);
    println!("  already liked before : {was_liked}");

    yt.rate(&id, Rating::Like)?;
    println!("  sent like for {id}");
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let after = yt.liked_songs()?;
    let now_liked = after.iter().any(|t| t.id == id);
    let at_top = after.first().map(|t| t.id == id).unwrap_or(false);
    println!("  present in liked now : {now_liked}{}", if at_top { " (at the top)" } else { "" });

    // Restore whatever the state was. Never clobber a like the user already had.
    if was_liked {
        println!("  leaving it liked, because it already was before this test");
    } else {
        yt.rate(&id, Rating::Indifferent)?;
        println!("  like removed again - previous state restored");
    }

    println!();
    if now_liked && !was_liked {
        println!("  WRITE PATH CONFIRMED");
    } else if was_liked {
        println!("  inconclusive: the track was already liked. Re-run with a track you have not liked.");
    } else {
        println!("  request succeeded but the like did not appear. Check the account in a browser.");
    }
    Ok(())
}
