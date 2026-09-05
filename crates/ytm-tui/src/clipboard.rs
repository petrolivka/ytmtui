//! Clipboard access, without taking a dependency for it.
//!
//! Shelling out to whatever the platform provides keeps this working over SSH
//! and on Wayland, X11 and macOS alike, and failing here is never fatal - the
//! caller shows the link instead.

use anyhow::{bail, Result};
use std::io::Write;
use std::process::{Command, Stdio};

const CANDIDATES: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
    ("pbcopy", &[]),
];

/// Copy `text`, returning which tool was used.
pub fn copy(text: &str) -> Result<&'static str> {
    for (bin, args) in CANDIDATES {
        let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            return Ok(bin);
        }
    }
    bail!("no clipboard tool found (tried wl-copy, xclip, xsel, pbcopy)")
}
