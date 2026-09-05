//! Fuzz the InnerTube renderer parsers with arbitrary JSON.
//!
//! The seed corpus is the captured fixtures, so the fuzzer starts from real
//! shapes and mutates outwards rather than guessing JSON from nothing.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ytm_api::{json, parse};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return; // invalid JSON is the HTTP layer's problem, not the parser's
    };
    let _ = parse::page_rows(&v);
    let _ = parse::flat_rows(&v);
    let _ = parse::flat_rows_from_queue(&v);
    let _ = parse::continuation(&v);
    let _ = json::thumbnail(&v);
    let _ = json::find_duration(&v);
});
