use crossterm::event::{KeyCode, KeyModifiers};

use super::TerminalKey;

impl TerminalKey {
    /// A command-only alias. The original event must still be used for text,
    /// pane forwarding, and press/release ownership.
    pub(crate) fn shortcut_key(&self) -> Option<Self> {
        let KeyCode::Char(character) = self.code else {
            return None;
        };
        // Ctrl+Alt can be AltGr. Never turn generated text into a command.
        if self
            .modifiers
            .contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && self.generated_text.is_some()
        {
            return None;
        }
        let russian = russian_base_key(character);
        let base = self
            .base_layout_codepoint
            .and_then(char::from_u32)
            .filter(char::is_ascii_graphic)
            .or_else(|| russian.map(|(base, _)| base))
            .or_else(|| {
                (character.is_ascii_graphic() && self.shifted_codepoint.is_some())
                    .then_some(character)
            })?;
        let mut modifiers = self.modifiers;
        if self.base_layout_codepoint.is_none() && russian.is_some_and(|(_, upper)| upper) {
            modifiers |= KeyModifiers::SHIFT;
        }
        let shifted_codepoint = modifiers
            .contains(KeyModifiers::SHIFT)
            .then(|| shifted_ascii_char(base))
            .flatten()
            .map(u32::from);
        if self.code == KeyCode::Char(base)
            && self.modifiers == modifiers
            && self.shifted_codepoint == shifted_codepoint
        {
            return None;
        }
        let mut alias = self.clone();
        alias.code = KeyCode::Char(base);
        alias.modifiers = modifiers;
        alias.shifted_codepoint = shifted_codepoint;
        alias.generated_text = None;
        alias.base_layout_codepoint = None;
        Some(alias)
    }
}

fn russian_base_key(character: char) -> Option<(char, bool)> {
    const RUSSIAN: &str = "ёйцукенгшщзхъфывапролджэячсмитьбю";
    const US: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,.";
    let lower = character.to_lowercase().next()?;
    RUSSIAN
        .chars()
        .zip(US.chars())
        .find_map(|(russian, base)| (lower == russian).then_some((base, character.is_uppercase())))
}

pub(crate) fn shifted_ascii_char(ch: char) -> Option<char> {
    match ch {
        'a'..='z' => Some(ch.to_ascii_uppercase()),
        '1' => Some('!'),
        '2' => Some('@'),
        '3' => Some('#'),
        '4' => Some('$'),
        '5' => Some('%'),
        '6' => Some('^'),
        '7' => Some('&'),
        '8' => Some('*'),
        '9' => Some('('),
        '0' => Some(')'),
        '-' => Some('_'),
        '=' => Some('+'),
        '[' => Some('{'),
        ']' => Some('}'),
        '\\' => Some('|'),
        ';' => Some(':'),
        '\'' => Some('"'),
        ',' => Some('<'),
        '.' => Some('>'),
        '/' => Some('?'),
        '`' => Some('~'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::parse_terminal_key_sequence;

    #[test]
    fn shortcut_alias_uses_reported_base_before_russian_fallback() {
        for (sequence, code, modifiers, shifted) in [
            ("\x1b[1092::97;3u", 'a', KeyModifiers::ALT, None),
            ("\x1b[1092::113;3u", 'q', KeyModifiers::ALT, None),
            ("\x1b[945::97;3u", 'a', KeyModifiers::ALT, None),
            ("\x1b[1099:1067:115;2u", 's', KeyModifiers::SHIFT, Some('S')),
            ("\x1b[92:47;2u", '\\', KeyModifiers::SHIFT, Some('|')),
            ("\x1bф", 'a', KeyModifiers::ALT, None),
            ("Ы", 's', KeyModifiers::SHIFT, Some('S')),
        ] {
            let key = parse_terminal_key_sequence(sequence).expect("key");
            let original = key.clone();
            let alias = key.shortcut_key().expect("shortcut alias");
            assert_eq!(alias.code, KeyCode::Char(code), "{sequence:?}");
            assert_eq!(alias.modifiers, modifiers, "{sequence:?}");
            assert_eq!(alias.shifted_codepoint, shifted.map(u32::from));
            assert!(alias.generated_text.is_none());
            assert_eq!(key, original);
        }
    }

    #[test]
    fn shortcut_alias_does_not_guess_unknown_layouts_or_legacy_ascii_punctuation() {
        for sequence in ["α", "/", "?", "\x1b[99;3u", "\x1b[57352u"] {
            assert!(parse_terminal_key_sequence(sequence)
                .expect("key")
                .shortcut_key()
                .is_none());
        }
        let altgr = parse_terminal_key_sequence("\x1b[281::101;7;281u").expect("AltGr text");
        assert!(altgr.shortcut_key().is_none());
    }

    #[test]
    fn russian_shortcut_fallback_covers_every_letter_and_case() {
        for (russian, base) in "ёйцукенгшщзхъфывапролджэячсмитьбю"
            .chars()
            .zip("`qwertyuiop[]asdfghjkl;'zxcvbnm,.".chars())
        {
            for uppercase in [false, true] {
                let ch = if uppercase {
                    russian.to_uppercase().next().expect("uppercase")
                } else {
                    russian
                };
                let alias = TerminalKey::new(KeyCode::Char(ch), KeyModifiers::NONE)
                    .shortcut_key()
                    .expect("Russian alias");
                assert_eq!(alias.code, KeyCode::Char(base));
                assert_eq!(alias.modifiers.contains(KeyModifiers::SHIFT), uppercase);
            }
        }
    }
}
