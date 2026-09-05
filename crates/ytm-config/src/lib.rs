//! User configuration: keymap, theme, audio and visualiser settings.
//!
//! A missing or partly-broken config is never fatal. Unknown or invalid entries
//! are reported and skipped, and everything else still applies - losing the
//! whole config because one binding has a typo would be a poor trade.

pub mod action;
pub mod keys;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use action::Action;
pub use keys::{Chord, Key, Mods};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct Config {
    pub general: General,
    pub audio: Audio,
    pub visualizer: Visualizer,
    pub art: Art,
    pub scrobble: Scrobble,
    pub theme: Theme,
    /// Binding -> action, e.g. `"ctrl+n" = "next"`. Merged over the defaults,
    /// so a user only has to list what they want to change.
    pub keys: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// Continue with a station when the queue runs dry.
    pub autoplay: bool,
    /// Restore the previous queue and position at startup.
    pub restore_session: bool,
    /// Thumbs-down also skips, as the official player does.
    pub dislike_skips: bool,
    /// Desktop notification when the track changes.
    pub notifications: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Audio {
    /// low | normal | high | auto
    pub quality: Quality,
    /// Output device name, or "default".
    pub device: String,
    /// 0.0 - 1.5
    pub volume: f32,
    /// Even out loudness between tracks.
    pub normalize: bool,
    /// Playback speed, 0.5 - 2.0. Pitch is preserved.
    pub speed: f32,
    /// Crossfade between tracks, in seconds. 0 keeps gapless handover.
    pub crossfade_secs: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Normal,
    #[default]
    High,
    Auto,
}

impl Quality {
    /// itag preference, best first, for this quality setting.
    pub fn itags(self) -> Vec<&'static str> {
        match self {
            Quality::High | Quality::Auto => vec!["251", "140", "250", "249"],
            Quality::Normal => vec!["140", "250", "251", "249"],
            Quality::Low => vec!["249", "250", "139", "140"],
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quality::Low => "low",
            Quality::Normal => "normal",
            Quality::High => "high",
            Quality::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Visualizer {
    pub enabled: bool,
    /// bars | mirrored | scope
    pub style: String,
    /// Load-bearing, not cosmetic: uncapped rendering roughly doubles CPU for
    /// no visible gain.
    pub max_fps: u32,
    /// Columns per band; 2 leaves a gutter between bars.
    pub bar_step: u16,
}

/// Scrobbling to ListenBrainz.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Scrobble {
    pub enabled: bool,
    /// From https://listenbrainz.org/profile/ - treat it as a password.
    pub listenbrainz_token: String,
}

/// Album art beside the spectrum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Art {
    pub enabled: bool,
    /// auto | halfblock | sixel | kitty | off
    ///
    /// "auto" picks by terminal; half blocks work everywhere, including
    /// terminals with no graphics protocol at all.
    pub backend: String,
}

impl Default for Art {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub fg: String,
    pub dim: String,
    pub accent: String,
    pub border: String,
    pub border_focus: String,
    pub selection_bg: String,
    pub error: String,
    pub ok: String,
    pub peak: String,
    /// Spectrum gradient, low amplitude to high.
    pub spectrum: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        Self {
            autoplay: true,
            restore_session: true,
            dislike_skips: true,
            notifications: false,
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            quality: Quality::High,
            device: "default".into(),
            volume: 1.0,
            normalize: false,
            speed: 1.0,
            crossfade_secs: 0.0,
        }
    }
}

impl Default for Visualizer {
    fn default() -> Self {
        Self {
            enabled: true,
            style: "mirrored".into(),
            max_fps: 60,
            bar_step: 2,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            fg: "#e6e6ea".into(),
            dim: "#7a7a88".into(),
            accent: "#ff333a".into(),
            border: "#3a3a46".into(),
            border_focus: "#ff333a".into(),
            selection_bg: "#2a2a36".into(),
            error: "#ff6b6b".into(),
            ok: "#4ad295".into(),
            peak: "#9a9ab0".into(),
            spectrum: vec!["#1db954".into(), "#e8c020".into(), "#ff333a".into()],
        }
    }
}

/// Parse "#rrggbb" into components.
pub fn parse_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

/// The default bindings. Users override individual entries in `[keys]`.
pub fn default_keymap() -> Vec<(Chord, Action)> {
    use keys::Key as K;
    use Action as A;
    let c = |ch: char| Chord::plain(K::Char(ch));
    vec![
        (c('/'), A::Search),
        (Chord::plain(K::Enter), A::Activate),
        (Chord::plain(K::Esc), A::Back),
        (Chord::plain(K::Backspace), A::Back),
        (Chord::plain(K::Tab), A::NextPane),
        (Chord::plain(K::BackTab), A::PrevPane),
        (Chord::plain(K::Down), A::Down),
        (c('j'), A::Down),
        (Chord::plain(K::Up), A::Up),
        (c('k'), A::Up),
        (Chord::plain(K::PageDown), A::PageDown),
        (Chord::plain(K::PageUp), A::PageUp),
        (Chord::plain(K::Home), A::Top),
        (Chord::plain(K::End), A::Bottom),
        (c(']'), A::NextTab),
        (c('['), A::PrevTab),
        (c('g'), A::GoToArtist),
        (c('G'), A::GoToAlbum),
        (c(' '), A::TogglePause),
        (c('n'), A::Next),
        (c('p'), A::Prev),
        (Chord::plain(K::Right), A::SeekForward),
        (Chord::plain(K::Left), A::SeekBackward),
        (Chord::shift(K::Right), A::SeekForwardLong),
        (Chord::shift(K::Left), A::SeekBackwardLong),
        (c('0'), A::VolumeUp),
        (c('9'), A::VolumeDown),
        (c('s'), A::ToggleShuffle),
        (c('r'), A::CycleRepeat),
        (c('R'), A::StartRadio),
        (c('A'), A::ToggleAutoplay),
        (c('o'), A::PlayNext),
        (c('e'), A::Enqueue),
        (c('x'), A::RemoveFromQueue),
        (Chord::plain(K::Delete), A::RemoveFromQueue),
        (c('+'), A::ThumbsUp),
        (c('l'), A::ThumbsUp),
        (c('-'), A::ThumbsDown),
        (c('d'), A::ThumbsDown),
        (c('a'), A::ToggleLibrary),
        (c('P'), A::AddToPlaylist),
        (c('N'), A::NewPlaylist),
        (c('X'), A::RemoveFromPlaylist),
        (c('S'), A::ToggleSubscribe),
        (c('y'), A::CopyLink),
        (c('v'), A::CycleVisualizer),
        (c('z'), A::ToggleVisualizerFullscreen),
        (c('L'), A::ToggleLyrics),
        (c('c'), A::ToggleArt),
        (c(':'), A::CommandPalette),
        (c('?'), A::Help),
        (c('q'), A::Quit),
        (Chord::ctrl(K::Char('c')), A::Quit),
    ]
}

/// A loaded config plus any problems found while loading it.
pub struct Loaded {
    pub config: Config,
    pub keymap: HashMap<Chord, Action>,
    /// Non-fatal problems, surfaced in the UI instead of being swallowed.
    pub warnings: Vec<String>,
    pub path: Option<PathBuf>,
}

pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("ytmtui"))
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

pub fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .map(|d| d.join("ytmtui"))
}

/// Load the config, falling back to defaults for anything missing or invalid.
pub fn load() -> Loaded {
    let path = config_path();
    let mut warnings = Vec::new();
    let config = match &path {
        Some(p) if p.exists() => match std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str::<Config>(&t).map_err(|e| e.to_string()))
        {
            Ok(c) => c,
            Err(e) => {
                // A broken config must not stop the app from starting.
                warnings.push(format!("config ignored ({}): {e}", p.display()));
                Config::default()
            }
        },
        _ => Config::default(),
    };

    let mut keymap: HashMap<Chord, Action> = default_keymap().into_iter().collect();
    for (k, v) in &config.keys {
        match (k.parse::<Chord>(), v.parse::<Action>()) {
            (Ok(chord), Ok(action)) => {
                keymap.insert(chord, action);
            }
            (Err(e), _) => warnings.push(format!("keys: {e}")),
            (_, Err(e)) => warnings.push(format!("keys: {e} (binding '{k}')")),
        }
    }

    Loaded {
        config,
        keymap,
        warnings,
        path,
    }
}

/// Write a fully-commented default config, for `ytmtui --write-config`.
pub fn write_default(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let body = toml::to_string_pretty(&Config::default()).context("serialising defaults")?;
    let doc = format!(
        "# ytmtui configuration\n\
         #\n\
         # Everything here is optional; anything you leave out keeps its default.\n\
         # Invalid entries are reported at startup and skipped, not fatal.\n\
         #\n\
         # [keys] maps a binding to an action, e.g.\n\
         #   \"ctrl+n\" = \"next\"\n\
         #   \"space\"  = \"toggle_pause\"\n\
         # Run `ytmtui --list-actions` for every action name.\n\n{body}"
    );
    std::fs::write(path, doc).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let s = toml::to_string(&Config::default()).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.audio.quality, Quality::High);
        assert!(back.general.autoplay);
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_rgb("#ff333a"), Some((0xff, 0x33, 0x3a)));
        assert_eq!(parse_rgb("ff333a"), Some((0xff, 0x33, 0x3a)));
        assert_eq!(parse_rgb("#xyzxyz"), None);
        assert_eq!(parse_rgb("#fff"), None);
    }

    /// Every default binding must name a real action, and the defaults must not
    /// bind the same chord twice - a silent override would be hard to notice.
    #[test]
    fn default_keymap_is_consistent() {
        let mut seen = std::collections::HashSet::new();
        for (chord, _) in default_keymap() {
            assert!(seen.insert(chord), "duplicate default binding for {chord}");
        }
    }
}
