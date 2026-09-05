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

/// Parse "3:33" or "1:02:03" into seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || !s.contains(':') {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut total = 0u64;
    for p in &parts {
        total = total * 60 + p.trim().parse::<u64>().ok()?;
    }
    Some(total)
}
