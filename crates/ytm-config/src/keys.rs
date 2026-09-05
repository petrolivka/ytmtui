//! Key names, as written in the config file.
//!
//! Deliberately independent of the terminal backend so the config format does
//! not change if the backend does.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Tab,
    BackTab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

impl Chord {
    pub fn plain(key: Key) -> Self {
        Self { key, mods: Mods::default() }
    }
    pub fn shift(key: Key) -> Self {
        Self { key, mods: Mods { shift: true, ..Default::default() } }
    }
    pub fn ctrl(key: Key) -> Self {
        Self { key, mods: Mods { ctrl: true, ..Default::default() } }
    }
}

/// `"ctrl+shift+left"`, `"space"`, `"f1"`, `"a"`.
impl FromStr for Chord {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = Mods::default();
        let lower = s.trim().to_ascii_lowercase();
        let mut parts: Vec<&str> = lower.split('+').map(|p| p.trim()).collect();
        // "+" is itself a bindable key, so splitting on '+' leaves a trailing
        // empty segment: "+" -> ["", ""], "ctrl++" -> ["ctrl", "", ""].
        let name: String = if parts.len() >= 2 && parts.last() == Some(&"") {
            parts.pop();
            parts.pop();
            "+".to_string()
        } else {
            match parts.pop() {
                Some(n) => n.to_string(),
                None => return Err(format!("empty key binding '{s}'")),
            }
        };
        let name = name.as_str();
        for m in parts {
            match m {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "meta" => mods.alt = true,
                "shift" => mods.shift = true,
                other => return Err(format!("unknown modifier '{other}' in '{s}'")),
            }
        }
        let key = match name {
            "enter" | "return" => Key::Enter,
            "esc" | "escape" => Key::Esc,
            "backspace" => Key::Backspace,
            "tab" => Key::Tab,
            "backtab" | "shift+tab" => Key::BackTab,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" | "pgup" => Key::PageUp,
            "pagedown" | "pgdn" => Key::PageDown,
            "delete" | "del" => Key::Delete,
            "insert" | "ins" => Key::Insert,
            "space" => Key::Char(' '),
            f if f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
                Key::F(f[1..].parse().unwrap())
            }
            other => {
                let mut it = other.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) => Key::Char(c),
                    _ => return Err(format!("unknown key '{other}' in '{s}'")),
                }
            }
        };
        Ok(Chord { key, mods })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("ctrl+")?;
        }
        if self.mods.alt {
            f.write_str("alt+")?;
        }
        if self.mods.shift {
            f.write_str("shift+")?;
        }
        match self.key {
            Key::Char(' ') => f.write_str("space"),
            Key::Char(c) => write!(f, "{c}"),
            Key::Enter => f.write_str("enter"),
            Key::Esc => f.write_str("esc"),
            Key::Backspace => f.write_str("backspace"),
            Key::Tab => f.write_str("tab"),
            Key::BackTab => f.write_str("shift+tab"),
            Key::Left => f.write_str("\u{2190}"),
            Key::Right => f.write_str("\u{2192}"),
            Key::Up => f.write_str("\u{2191}"),
            Key::Down => f.write_str("\u{2193}"),
            Key::Home => f.write_str("home"),
            Key::End => f.write_str("end"),
            Key::PageUp => f.write_str("pgup"),
            Key::PageDown => f.write_str("pgdn"),
            Key::Delete => f.write_str("del"),
            Key::Insert => f.write_str("ins"),
            Key::F(n) => write!(f, "f{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bindings() {
        assert_eq!("space".parse::<Chord>().unwrap(), Chord::plain(Key::Char(' ')));
        assert_eq!("ctrl+c".parse::<Chord>().unwrap(), Chord::ctrl(Key::Char('c')));
        assert_eq!("shift+left".parse::<Chord>().unwrap(), Chord::shift(Key::Left));
        assert_eq!("f5".parse::<Chord>().unwrap(), Chord::plain(Key::F(5)));
        // A literal plus is a plausible binding and must not parse as an empty key.
        assert_eq!("+".parse::<Chord>().unwrap(), Chord::plain(Key::Char('+')));
    }

    #[test]
    fn rejects_nonsense() {
        assert!("hyper+a".parse::<Chord>().is_err());
        assert!("nosuchkey".parse::<Chord>().is_err());
    }

    #[test]
    fn round_trips() {
        for s in ["space", "ctrl+c", "enter", "f7", "a"] {
            let c: Chord = s.parse().unwrap();
            assert_eq!(c.to_string().parse::<Chord>().unwrap(), c, "{s}");
        }
    }
}
