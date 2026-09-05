//! Fuzz key-binding parsing, which reads a user-editable config file.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::str::FromStr;
use ytm_config::Chord;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(c) = Chord::from_str(text) {
        // Anything that parses must render back to something that parses to
        // the same chord, or a written-out keymap would not round-trip.
        let rendered = c.to_string();
        assert_eq!(Chord::from_str(&rendered).unwrap(), c);
    }
});
