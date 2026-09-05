//! Translating terminal key events into configured actions.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ytm_config::{Chord, Key, Mods};

/// Convert a terminal key event into a bindable chord.
pub fn chord_of(k: KeyEvent) -> Option<Chord> {
    let key = match k.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Delete => Key::Delete,
        KeyCode::Insert => Key::Insert,
        KeyCode::F(n) => Key::F(n),
        _ => return None,
    };
    let mut mods = Mods {
        ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
        alt: k.modifiers.contains(KeyModifiers::ALT),
        shift: k.modifiers.contains(KeyModifiers::SHIFT),
    };
    // Shift is already baked into the character: terminals report 'G' with the
    // shift flag set, so keeping the flag would stop a plain "G" binding from
    // ever matching.
    if matches!(key, Key::Char(_)) {
        mods.shift = false;
    }
    Some(Chord { key, mods })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn ev(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: m,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn uppercase_letters_ignore_the_shift_flag() {
        let c = chord_of(ev(KeyCode::Char('G'), KeyModifiers::SHIFT)).unwrap();
        assert_eq!(c, Chord::plain(Key::Char('G')));
    }

    #[test]
    fn shift_still_matters_for_non_character_keys() {
        let c = chord_of(ev(KeyCode::Right, KeyModifiers::SHIFT)).unwrap();
        assert_eq!(c, Chord::shift(Key::Right));
    }

    #[test]
    fn control_is_preserved() {
        let c = chord_of(ev(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(c, Chord::ctrl(Key::Char('c')));
    }
}
