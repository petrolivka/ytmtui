//! Time-synced lyrics from LRCLIB.
//!
//! YouTube Music serves plain lyrics only, so synced lines come from a second
//! provider. Failure here is never an error worth surfacing: no match simply
//! means the plain lyrics stay.

use anyhow::Result;
use serde_json::Value;
use std::time::Duration;

const ENDPOINT: &str = "https://lrclib.net/api/get";

/// One line of synced lyrics.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub at: Duration,
    pub text: String,
}

/// Parse an LRC body into timed lines, sorted by time.
///
/// Tolerates the common variations: two- or three-digit fractions, several
/// timestamps on one line, and metadata tags, which are skipped.
pub fn parse_lrc(body: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let mut rest = raw;
        let mut stamps: Vec<Duration> = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some(d) = parse_stamp(tag) {
                stamps.push(d);
            } else if !tag.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                // A metadata tag such as [ar:Artist]; not a timestamp.
            } else {
                break;
            }
            rest = &rest[end + 1..];
        }
        let text = rest.trim();
        for at in stamps {
            out.push(Line {
                at,
                text: text.to_string(),
            });
        }
    }
    out.sort_by_key(|l| l.at);
    out
}

fn parse_stamp(tag: &str) -> Option<Duration> {
    let (m, rest) = tag.split_once(':')?;
    let minutes: u64 = m.trim().parse().ok()?;
    let (s, frac) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, "0"),
    };
    let seconds: u64 = s.trim().parse().ok()?;
    // Fractions appear as hundredths or thousandths.
    let frac_digits = frac.trim();
    let frac_val: u64 = frac_digits.parse().ok()?;
    let millis = match frac_digits.len() {
        1 => frac_val * 100,
        2 => frac_val * 10,
        _ => frac_val,
    };
    Some(Duration::from_millis(
        minutes * 60_000 + seconds * 1000 + millis,
    ))
}

/// Look up synced lyrics. Returns None when there is no match.
pub fn fetch(
    http: &reqwest::blocking::Client,
    artist: &str,
    title: &str,
    album: Option<&str>,
    duration: Option<Duration>,
) -> Result<Option<Vec<Line>>> {
    if artist.trim().is_empty() || title.trim().is_empty() {
        return Ok(None);
    }
    let mut req = http
        .get(ENDPOINT)
        .query(&[("artist_name", artist), ("track_name", title)]);
    if let Some(a) = album {
        req = req.query(&[("album_name", a)]);
    }
    if let Some(d) = duration {
        req = req.query(&[("duration", d.as_secs().to_string())]);
    }
    let resp = req.send()?;
    if !resp.status().is_success() {
        // 404 just means "not in the database".
        return Ok(None);
    }
    let v: Value = resp.json()?;
    let synced = v.get("syncedLyrics").and_then(|s| s.as_str()).unwrap_or("");
    if synced.trim().is_empty() {
        return Ok(None);
    }
    let lines = parse_lrc(synced);
    Ok(if lines.is_empty() { None } else { Some(lines) })
}

/// Index of the line that should be highlighted at `pos`.
pub fn active_line(lines: &[Line], pos: Duration) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    match lines.binary_search_by(|l| l.at.cmp(&pos)) {
        Ok(i) => Some(i),
        // Before the first timestamp, nothing is active yet.
        Err(0) => None,
        Err(i) => Some(i - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamps() {
        let lrc = "[ar:Someone]\n[00:12.34]first\n[01:05.6]second\n[02:00]third";
        let l = parse_lrc(lrc);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].at, Duration::from_millis(12_340));
        assert_eq!(l[0].text, "first");
        // A one-digit fraction is tenths, not milliseconds.
        assert_eq!(l[1].at, Duration::from_millis(65_600));
        assert_eq!(l[2].at, Duration::from_millis(120_000));
    }

    #[test]
    fn handles_repeated_timestamps_on_one_line() {
        let l = parse_lrc("[00:10.00][00:20.00]chorus");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "chorus");
        assert_eq!(l[1].at, Duration::from_millis(20_000));
    }

    #[test]
    fn picks_the_active_line() {
        let l = parse_lrc("[00:00.00]a\n[00:10.00]b\n[00:20.00]c");
        assert_eq!(active_line(&l, Duration::from_secs(5)), Some(0));
        assert_eq!(active_line(&l, Duration::from_secs(10)), Some(1));
        assert_eq!(active_line(&l, Duration::from_secs(19)), Some(1));
        assert_eq!(active_line(&l, Duration::from_secs(999)), Some(2));
        assert_eq!(active_line(&[], Duration::from_secs(1)), None);
    }

    #[test]
    fn ignores_metadata_only_files() {
        assert!(parse_lrc("[ar:X]\n[ti:Y]\n[length:03:21]").is_empty());
    }
}
