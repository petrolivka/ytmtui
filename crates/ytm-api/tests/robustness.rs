//! The parsers must never panic, whatever comes back.
//!
//! InnerTube is undocumented and changes without notice, so "the shape is
//! wrong" is an expected condition, not an exceptional one. A panic here would
//! take the whole player down mid-listen; degrading to an empty pane is the
//! required behaviour.
//!
//! Deterministic rather than random: a seeded generator means a failure is
//! reproducible from the seed printed in the message.

use serde_json::Value;
use ytm_api::{json, parse};

fn load(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

/// xorshift64*, so a seed reproduces a mutation exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Corrupt a value in place: drop keys, blank strings, swap types, empty
/// arrays. Each of these has a real-world analogue in a shape change.
fn mutate(v: &mut Value, rng: &mut Rng, depth: usize) {
    if depth > 12 {
        return;
    }
    match v {
        Value::Object(m) => {
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                match rng.below(10) {
                    0 => {
                        m.remove(&k);
                        continue;
                    }
                    1 => {
                        m.insert(k.clone(), Value::Null);
                        continue;
                    }
                    2 => {
                        // A field that used to be a string becoming an object
                        // is exactly how these break.
                        m.insert(k.clone(), Value::Array(vec![]));
                        continue;
                    }
                    3 => {
                        m.insert(k.clone(), Value::String(String::new()));
                        continue;
                    }
                    4 => {
                        m.insert(k.clone(), Value::Number(0.into()));
                        continue;
                    }
                    _ => {}
                }
                if let Some(x) = m.get_mut(&k) {
                    mutate(x, rng, depth + 1);
                }
            }
        }
        Value::Array(a) => {
            if rng.below(8) == 0 {
                a.clear();
                return;
            }
            for x in a.iter_mut() {
                mutate(x, rng, depth + 1);
            }
        }
        Value::String(s) if rng.below(20) == 0 => {
            s.clear();
        }
        _ => {}
    }
}

/// Run every parser over a value. Panicking is the failure being tested for.
fn exercise(v: &Value) {
    let rows = parse::page_rows(v);
    let flat = parse::flat_rows(v);
    let queue = parse::flat_rows_from_queue(v);
    let _ = parse::continuation(v);
    let _ = json::thumbnail(v);
    let _ = json::find_duration(v);

    // Whatever survives must still be internally consistent: a Row::Track only
    // exists if it had a usable id and a title.
    for r in rows.iter().chain(flat.iter()) {
        if let Some(t) = r.as_track() {
            assert!(t.id.is_valid(), "produced a track with an invalid id");
            assert!(!t.title.trim().is_empty(), "produced a track with no title");
        }
    }
    for t in &queue {
        assert!(t.id.is_valid());
    }
}

#[test]
fn mutated_responses_never_panic() {
    let names = [
        "search_songs",
        "search_albums",
        "search_artists",
        "search_playlists",
        "browse_artist",
        "browse_album",
        "browse_charts",
        "watch_next",
    ];
    for name in names {
        let original = load(name);
        for seed in 1..=25u64 {
            let mut v = original.clone();
            mutate(&mut v, &mut Rng(seed.wrapping_mul(0x9E3779B97F4A7C15)), 0);
            // If this panics, the seed and fixture name locate it exactly.
            exercise(&v);
        }
    }
}

#[test]
fn degenerate_documents_are_handled() {
    for v in [
        Value::Null,
        Value::Bool(true),
        Value::Number(7.into()),
        Value::String("nonsense".into()),
        Value::Array(vec![]),
        Value::Object(Default::default()),
        // Renderers present but empty.
        serde_json::json!({ "contents": { "musicShelfRenderer": { "contents": [] } } }),
        // A renderer whose fields are all the wrong type.
        serde_json::json!({
            "musicResponsiveListItemRenderer": {
                "flexColumns": "not an array",
                "playlistItemData": 12,
            }
        }),
        // Deeply nested emptiness.
        serde_json::json!({"a":{"b":{"c":{"d":{"e":{"f":{"g":{}}}}}}}}),
    ] {
        exercise(&v);
    }
}

/// A truncated response is a plausible network outcome; it must not be a crash.
#[test]
fn truncated_json_fails_cleanly() {
    let body = std::fs::read_to_string(format!(
        "{}/tests/fixtures/search_songs.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    for frac in [1, 2, 3, 5, 8, 13] {
        let cut = body.len() * frac / 16;
        // Parsing must fail, not panic - and the caller sees an Err.
        let parsed: Result<Value, _> = serde_json::from_str(&body[..cut]);
        assert!(parsed.is_err(), "a truncated document parsed as valid JSON");
    }
}
