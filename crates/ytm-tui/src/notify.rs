//! Desktop notification on track change.
//!
//! Shelling out to the platform's tool keeps this free of a D-Bus dependency
//! here and works with whatever notification daemon is running. Failure is
//! silent: a missing notifier is not worth telling the user about on every
//! track.

use std::process::{Command, Stdio};

/// Send a notification, replacing the previous one where the notifier supports
/// it so a long listening session does not stack up a wall of popups.
pub fn track_changed(title: &str, body: &str) {
    let candidates: &[(&str, Vec<String>)] = &[
        (
            "notify-send",
            vec![
                "--app-name=ytmtui".into(),
                "--expire-time=4000".into(),
                // Replace rather than stack; ignored by notifiers that lack it.
                "--hint=string:x-canonical-private-synchronous:ytmtui".into(),
                title.into(),
                body.into(),
            ],
        ),
        (
            "terminal-notifier",
            vec![
                "-title".into(),
                title.into(),
                "-message".into(),
                body.into(),
            ],
        ),
    ];
    for (bin, args) in candidates {
        if Command::new(bin)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return;
        }
    }
}
