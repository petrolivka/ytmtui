//! Fuzz the LRC lyrics parser, which reads text from a third-party service.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ytm_api::lrclib;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let lines = lrclib::parse_lrc(text);
    // Timestamps must come out ordered, or the active-line search misbehaves.
    for pair in lines.windows(2) {
        assert!(pair[0].at <= pair[1].at);
    }
    let _ = lrclib::active_line(&lines, std::time::Duration::from_secs(30));
});
