//! MPRIS2 D-Bus interface, so playerctl, media keys and desktop widgets can
//! drive the player.
//!
//! Best-effort: if there is no session bus (a bare TTY, a container, SSH
//! without a bus) this quietly does nothing rather than failing startup.

use crate::engine::{Command, PlayerHandle};
use std::collections::HashMap;
use std::time::Duration;
use ytm_core::{PlayState, RepeatMode};
use zbus::blocking::connection;
use zbus::interface;
use zbus::zvariant::{ObjectPath, Value};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.ytmtui";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";

struct Root;

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        "ytmtui".into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }
}

struct Player {
    handle: PlayerHandle,
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.handle.send(Command::Next);
    }
    fn previous(&self) {
        self.handle.send(Command::Prev);
    }
    fn pause(&self) {
        if self.handle.status().state == PlayState::Playing {
            self.handle.send(Command::TogglePause);
        }
    }
    fn play(&self) {
        if self.handle.status().state == PlayState::Paused {
            self.handle.send(Command::TogglePause);
        }
    }
    fn play_pause(&self) {
        self.handle.send(Command::TogglePause);
    }
    fn stop(&self) {
        self.handle.send(Command::Stop);
    }
    /// Offset is in microseconds, per the MPRIS spec.
    fn seek(&self, offset: i64) {
        self.handle.send(Command::SeekRelative(offset as f64 / 1e6));
    }
    fn set_position(&self, _track: ObjectPath<'_>, position: i64) {
        let now = self.handle.status().position.as_secs_f64();
        self.handle
            .send(Command::SeekRelative(position as f64 / 1e6 - now));
    }
    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        match self.handle.status().state {
            PlayState::Playing => "Playing",
            PlayState::Paused => "Paused",
            _ => "Stopped",
        }
        .into()
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        match self.handle.status().repeat {
            RepeatMode::Off => "None",
            RepeatMode::All => "Playlist",
            RepeatMode::One => "Track",
        }
        .into()
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.handle.status().shuffle
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.handle.status().volume as f64
    }

    #[zbus(property)]
    fn set_volume(&self, v: f64) {
        self.handle
            .send(Command::SetVolume(v.clamp(0.0, 1.5) as f32));
    }

    /// Position in microseconds.
    #[zbus(property)]
    fn position(&self) -> i64 {
        self.handle.status().position.as_micros() as i64
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, Value<'_>> {
        let st = self.handle.status();
        let mut m: HashMap<String, Value<'_>> = HashMap::new();
        let Some(t) = st.current.clone() else {
            return m;
        };
        // The track id must be a valid object path, so the video id is escaped
        // into one rather than used directly.
        let safe: String =
            t.id.0
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
        if let Ok(p) = ObjectPath::try_from(format!("/org/mpris/MediaPlayer2/Track/{safe}")) {
            m.insert("mpris:trackid".into(), Value::from(p));
        }
        m.insert(
            "mpris:length".into(),
            Value::from(t.duration.unwrap_or(Duration::ZERO).as_micros() as i64),
        );
        m.insert("xesam:title".into(), Value::from(t.title.clone()));
        m.insert("xesam:artist".into(), Value::from(vec![t.artist.clone()]));
        if let Some(a) = &t.album {
            m.insert("xesam:album".into(), Value::from(a.clone()));
        }
        m.insert(
            "xesam:url".into(),
            Value::from(format!("https://music.youtube.com/watch?v={}", t.id)),
        );
        m
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

/// Publish the MPRIS interface. Returns the connection, which must be kept
/// alive for the name to stay claimed.
pub fn serve(handle: PlayerHandle) -> zbus::Result<connection::Connection> {
    connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, Root)?
        .serve_at(OBJECT_PATH, Player { handle })?
        .build()
}
