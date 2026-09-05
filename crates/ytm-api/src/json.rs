//! Defensive helpers for walking InnerTube's deeply nested renderer trees.
//!
//! These responses change without notice, so every accessor returns an Option
//! and an unknown shape yields nothing rather than a panic (R1).

use serde_json::Value;

/// Depth-first search for the first value under `key`.
pub fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key) {
                return Some(x);
            }
            m.values().find_map(|x| find(x, key))
        }
        Value::Array(a) => a.iter().find_map(|x| find(x, key)),
        _ => None,
    }
}

/// Collect every value appearing under `key`, at any depth.
pub fn find_all<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == key {
                    out.push(x);
                }
                find_all(x, key, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                find_all(x, key, out);
            }
        }
        _ => {}
    }
}

/// InnerTube wraps display strings as `{"runs":[{"text":..}]}` or `{"simpleText":..}`.
pub fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    if let Some(runs) = v.get("runs").and_then(|x| x.as_array()) {
        let s: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect();
        if !s.is_empty() {
            return Some(s);
        }
    }
    v.as_str().map(|s| s.to_string())
}

/// The individual runs of a text node, which is how InnerTube separates
/// artist / album / duration within one column.
pub fn runs(v: &Value) -> Vec<String> {
    v.get("runs")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Find the first text node anywhere under `v` that parses as a duration.
/// Different surfaces put it in different places (search uses `flexColumns`,
/// library and playlist responses use `fixedColumns`), so this is the
/// last-resort fallback after the known shapes have been tried.
pub fn find_duration(v: &Value) -> Option<u64> {
    match v {
        Value::Object(m) => {
            if let Some(s) = m.get("simpleText").and_then(|x| x.as_str()) {
                if let Some(d) = parse_duration(s) {
                    return Some(d);
                }
            }
            if let Some(runs) = m.get("runs").and_then(|x| x.as_array()) {
                for r in runs {
                    if let Some(d) = r
                        .get("text")
                        .and_then(|t| t.as_str())
                        .and_then(parse_duration)
                    {
                        return Some(d);
                    }
                }
            }
            m.values().find_map(find_duration)
        }
        Value::Array(a) => a.iter().find_map(find_duration),
        _ => None,
    }
}

/// The largest thumbnail URL in an item, if it has any.
///
/// The size in the URL is only a hint; `ytm_art::at_size` rewrites it to the
/// size that will actually be drawn.
pub fn thumbnail(v: &Value) -> Option<String> {
    let list = find(v, "thumbnails")?.as_array()?;
    let mut best: Option<(u64, &str)> = None;
    for t in list {
        let Some(url) = t.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        let w = t.get("width").and_then(|w| w.as_u64()).unwrap_or(0);
        if best.map(|(bw, _)| w > bw).unwrap_or(true) {
            best = Some((w, url));
        }
    }
    best.map(|(_, u)| u.to_string())
}

/// Parse "3:33" or "1:02:03" into seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || !s.contains(':') {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut total = 0u64;
    for (i, p) in parts.iter().enumerate() {
        let p = p.trim();
        // Reject things that merely contain a colon, e.g. "1:1 mix" or a time
        // of day: every field is numeric, and only the first may be one digit.
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if i > 0 && p.len() != 2 {
            return None;
        }
        total = total * 60 + p.parse::<u64>().ok()?;
    }
    // A track longer than 12 hours is a parse error, not a track.
    if total == 0 || total > 12 * 3600 {
        return None;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::parse_duration;

    #[test]
    fn parses_real_durations() {
        assert_eq!(parse_duration("3:33"), Some(213));
        assert_eq!(parse_duration("0:58"), Some(58));
        assert_eq!(parse_duration("1:02:03"), Some(3723));
    }

    #[test]
    fn rejects_things_that_merely_contain_a_colon() {
        for s in [
            "",
            "Aphex Twin",
            "1:1 mix",
            "Vol: 2",
            "12:3",
            "a:bc",
            "0:00",
            "99:99:99:99",
        ] {
            assert_eq!(parse_duration(s), None, "should reject {s:?}");
        }
    }
}
