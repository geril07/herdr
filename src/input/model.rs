#[cfg(not(windows))]
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsKeyRecord {
    pub key_down: bool,
    pub repeat_count: u16,
    pub virtual_key_code: u16,
    pub virtual_scan_code: u16,
    pub unicode: u16,
    pub control_key_state: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCommit {
    text: String,
}

impl TextCommit {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) fn into_string(self) -> String {
        self.text
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalKeyId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyIdentity {
    #[cfg(any(windows, test))]
    Physical(PhysicalKeyId),
    Semantic(KeyCode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeySource {
    Synthesized,
    Vt {
        bytes: Vec<u8>,
    },
    #[cfg(any(windows, test))]
    WindowsConsole {
        record: WindowsKeyRecord,
        physical_key: Option<PhysicalKeyId>,
    },
}

#[cfg(any(windows, test))]
impl WindowsKeyRecord {
    fn physical_key_id(self) -> Option<PhysicalKeyId> {
        const ENHANCED_KEY: u32 = 0x0100;
        (self.virtual_scan_code != 0).then(|| {
            PhysicalKeyId(
                u32::from(self.virtual_scan_code)
                    | (u32::from(self.control_key_state & ENHANCED_KEY != 0) << 16),
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalKey {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub kind: crossterm::event::KeyEventKind,
    pub repeat_count: u16,
    pub shifted_codepoint: Option<u32>,
    /// Kitty keyboard protocol "base layout key": the codepoint the same
    /// physical key produces on a standard PC-101/QWERTY layout. Terminals
    /// send this when `REPORT_ALTERNATE_KEYS` is active, e.g. Cyrillic `ц`
    /// reports base `w`, so `ctrl+ц` can resolve to `ctrl+w`. Distinct from
    /// `shifted_codepoint`, which is shift state, not layout information.
    /// `None` when the terminal omits it (many terminals send only
    /// `CSI 1094;5u`); callers fall back to the static Cyrillic table below.
    pub base_layout_codepoint: Option<u32>,
    pub generated_text: Option<String>,
    physical_identity_hint: bool,
    windows_dead_key: bool,
    source: KeySource,
}

impl TerminalKey {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: crossterm::event::KeyEventKind::Press,
            repeat_count: 1,
            shifted_codepoint: None,
            base_layout_codepoint: None,
            generated_text: None,
            physical_identity_hint: false,
            windows_dead_key: false,
            source: KeySource::Synthesized,
        }
    }

    pub fn with_kind(mut self, kind: crossterm::event::KeyEventKind) -> Self {
        if kind == crossterm::event::KeyEventKind::Release {
            self.repeat_count = 1;
            self.generated_text = None;
        }
        self.kind = kind;
        self
    }

    pub fn with_repeat_count(mut self, repeat_count: u16) -> Self {
        self.repeat_count = if self.kind == crossterm::event::KeyEventKind::Release {
            1
        } else {
            repeat_count.max(1)
        };
        self
    }

    pub(crate) fn with_modifiers(mut self, modifiers: KeyModifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub fn with_shifted_codepoint(mut self, shifted_codepoint: u32) -> Self {
        self.shifted_codepoint = Some(shifted_codepoint);
        self
    }

    pub fn with_base_layout_codepoint(mut self, base_layout_codepoint: u32) -> Self {
        self.base_layout_codepoint = Some(base_layout_codepoint);
        self
    }

    /// Layout-independent base character for `Ctrl`/`Super` shortcuts.
    ///
    /// Prefers the terminal-reported Kitty base-layout key when it is ASCII,
    /// otherwise falls back to the static Russian `ЙЦУКЕН` table. Returns
    /// `None` for ASCII input (no resolution needed) and for non-Cyrillic
    /// scripts without a reported base key.
    pub fn shortcut_base_char(&self) -> Option<char> {
        let KeyCode::Char(ch) = self.code else {
            return None;
        };
        if ch.is_ascii() {
            return None;
        }
        if let Some(base) = self.base_layout_codepoint.and_then(char::from_u32) {
            if base.is_ascii() {
                return Some(base);
            }
        }
        cyrillic_to_latin_shortcut_base(ch)
    }

    /// Normalized copy for shortcut matching and pane forwarding.
    ///
    /// When `Ctrl` or `Super` is held with a non-ASCII character, replaces the
    /// code with the layout-independent Latin base (terminal-reported or
    /// Russian fallback). Plain typing, `Shift`-only capitals, and `Alt`-only
    /// chords keep the original character so text input is untouched.
    pub fn normalized_for_shortcut(mut self) -> Self {
        let wants_layout_resolution = self.modifiers.contains(KeyModifiers::CONTROL)
            || self.modifiers.contains(KeyModifiers::SUPER);
        if !wants_layout_resolution {
            return self;
        }
        let Some(latin) = self.shortcut_base_char() else {
            return self;
        };
        self.code = KeyCode::Char(latin);
        if self
            .shifted_codepoint
            .is_some_and(|cp| char::from_u32(cp).is_none_or(|shifted| !shifted.is_ascii()))
        {
            self.shifted_codepoint = None;
        }
        self
    }

    pub(crate) fn with_generated_text(mut self, text: Option<String>) -> Self {
        self.generated_text = if self.kind == crossterm::event::KeyEventKind::Release {
            None
        } else {
            text
        };
        self
    }

    pub(crate) fn with_vt_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.source = KeySource::Vt { bytes };
        self
    }

    #[cfg(any(windows, test))]
    pub fn with_windows_record(mut self, record: WindowsKeyRecord) -> Self {
        self = self.with_windows_composition_hint(Some(record));
        self.repeat_count = if self.kind == crossterm::event::KeyEventKind::Release {
            1
        } else {
            record.repeat_count.max(1)
        };
        let physical_key = record.physical_key_id();
        self.physical_identity_hint = physical_key.is_some();
        self.source = KeySource::WindowsConsole {
            physical_key,
            record,
        };
        self
    }

    pub(crate) fn with_windows_composition_hint(
        mut self,
        record: Option<WindowsKeyRecord>,
    ) -> Self {
        // AltGr is normalized to text-only modifiers by the Windows input mapper.
        // Command chords can also have zero Unicode, so retain their fallback keys.
        self.windows_dead_key = matches!(self.code, KeyCode::Char(_))
            && self.modifiers.difference(KeyModifiers::SHIFT).is_empty()
            && record.is_some_and(|record| record.unicode == 0);
        self
    }

    pub(crate) fn with_physical_identity_hint(mut self, physical: bool) -> Self {
        self.physical_identity_hint = physical;
        self
    }

    #[cfg(any(windows, test))]
    pub(crate) fn vt_bytes(&self) -> Option<&[u8]> {
        match &self.source {
            KeySource::Vt { bytes } => Some(bytes),
            KeySource::Synthesized | KeySource::WindowsConsole { .. } => None,
        }
    }

    pub(crate) fn windows_record(&self) -> Option<WindowsKeyRecord> {
        #[cfg(any(windows, test))]
        match self.source {
            KeySource::WindowsConsole { record, .. } => Some(record),
            KeySource::Synthesized | KeySource::Vt { .. } => None,
        }
        #[cfg(not(any(windows, test)))]
        None
    }

    pub(crate) fn is_windows_dead_key(&self) -> bool {
        self.windows_dead_key
    }

    pub(crate) fn identity(&self) -> KeyIdentity {
        match self.source {
            #[cfg(any(windows, test))]
            KeySource::WindowsConsole {
                physical_key: Some(physical_key),
                ..
            } => KeyIdentity::Physical(physical_key),
            #[cfg(any(windows, test))]
            KeySource::WindowsConsole {
                physical_key: None, ..
            } => KeyIdentity::Semantic(self.code),
            KeySource::Synthesized | KeySource::Vt { .. } => KeyIdentity::Semantic(self.code),
        }
    }

    pub(crate) fn has_physical_identity(&self) -> bool {
        self.physical_identity_hint || self.physical_key_id().is_some()
    }

    pub(crate) fn physical_key_id(&self) -> Option<u32> {
        match &self.source {
            #[cfg(any(windows, test))]
            KeySource::WindowsConsole {
                physical_key: Some(PhysicalKeyId(id)),
                ..
            } => Some(*id),
            #[cfg(any(windows, test))]
            KeySource::WindowsConsole {
                physical_key: None, ..
            } => None,
            KeySource::Synthesized | KeySource::Vt { .. } => None,
        }
    }

    pub fn with_text_commit(mut self) -> Self {
        let has_text_only_modifiers = match self.code {
            KeyCode::Char(ch) if ch.is_uppercase() => {
                self.modifiers == KeyModifiers::SHIFT || self.modifiers.is_empty()
            }
            KeyCode::Char(_) => self.modifiers.is_empty(),
            _ => false,
        };
        if has_text_only_modifiers && self.kind == crossterm::event::KeyEventKind::Press {
            self.generated_text = match self.code {
                KeyCode::Char(ch) => Some(ch.to_string()),
                _ => None,
            };
        }
        self
    }

    pub fn as_key_event(&self) -> KeyEvent {
        KeyEvent::new_with_kind(self.code, self.modifiers, self.kind)
    }
}

impl From<KeyEvent> for TerminalKey {
    fn from(value: KeyEvent) -> Self {
        Self::new(value.code, value.modifiers).with_kind(value.kind)
    }
}

/// Russian `ЙЦУКЕН` physical-key fallback for `Ctrl`/`Super` shortcuts.
///
/// Maps the Cyrillic character to the Latin character on the same PC-101 key,
/// e.g. `ц`/`Ц` to `w`, `о`/`О` to `j`. Only letters that share a physical key
/// with an ASCII letter are covered, plus `[`/`]` (`х`/`ъ`, where `Ctrl+[`
/// is `Esc`). Used when the terminal omits the Kitty base-layout key; many
/// terminals send only `CSI 1094;5u` even with alternate-key reporting.
/// Returns lowercase Latin; `Ctrl` handling is case-insensitive.
pub fn cyrillic_to_latin_shortcut_base(ch: char) -> Option<char> {
    Some(match ch {
        'й' | 'Й' => 'q',
        'ц' | 'Ц' => 'w',
        'у' | 'У' => 'e',
        'к' | 'К' => 'r',
        'е' | 'Е' => 't',
        'н' | 'Н' => 'y',
        'г' | 'Г' => 'u',
        'ш' | 'Ш' => 'i',
        'щ' | 'Щ' => 'o',
        'з' | 'З' => 'p',
        'х' | 'Х' => '[',
        'ъ' | 'Ъ' => ']',
        'ф' | 'Ф' => 'a',
        'ы' | 'Ы' => 's',
        'в' | 'В' => 'd',
        'а' | 'А' => 'f',
        'п' | 'П' => 'g',
        'р' | 'Р' => 'h',
        'о' | 'О' => 'j',
        'л' | 'Л' => 'k',
        'д' | 'Д' => 'l',
        'ж' | 'Ж' => ';',
        'э' | 'Э' => '\'',
        'я' | 'Я' => 'z',
        'ч' | 'Ч' => 'x',
        'с' | 'С' => 'c',
        'м' | 'М' => 'v',
        'и' | 'И' => 'b',
        'т' | 'Т' => 'n',
        'ь' | 'Ь' => 'm',
        'б' | 'Б' => ',',
        'ю' | 'Ю' => '.',
        _ => return None,
    })
}

pub(crate) const KITTY_FLAG_REPORT_ALL_KEYS: u16 = 0b0000_1000;

#[cfg(not(windows))]
pub fn ime_compatible_keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifyOtherKeysMode {
    Mode1,
    Mode2,
}

impl ModifyOtherKeysMode {
    pub fn set_sequence(self) -> &'static [u8] {
        match self {
            Self::Mode1 => b"\x1b[>4;1m",
            Self::Mode2 => b"\x1b[>4;2m",
        }
    }
}

pub fn host_modify_other_keys_mode() -> Option<ModifyOtherKeysMode> {
    #[cfg(windows)]
    let alacritty_window_id = std::env::var_os("ALACRITTY_WINDOW_ID").is_some();
    #[cfg(not(windows))]
    let alacritty_window_id = false;

    host_modify_other_keys_mode_for_env(
        std::env::var("TMUX").is_ok(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var_os("WEZTERM_PANE").is_some(),
        alacritty_window_id,
    )
}

fn host_modify_other_keys_mode_for_env(
    in_tmux: bool,
    term_program: Option<&str>,
    wezterm_pane: bool,
    alacritty_window_id: bool,
) -> Option<ModifyOtherKeysMode> {
    if in_tmux {
        return Some(ModifyOtherKeysMode::Mode2);
    }

    if wezterm_pane
        || alacritty_window_id
        || term_program.is_some_and(|program| program.eq_ignore_ascii_case("wezterm"))
    {
        return Some(ModifyOtherKeysMode::Mode1);
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardProtocol {
    Legacy,
    Kitty { flags: u16 },
}

impl KeyboardProtocol {
    pub fn from_kitty_flags(flags: u16) -> Self {
        if flags == 0 {
            Self::Legacy
        } else {
            Self::Kitty { flags }
        }
    }

    pub(crate) fn reports_event_types(self) -> bool {
        matches!(self, Self::Kitty { flags } if flags & 0b0000_0010 != 0)
    }

    pub(crate) fn reports_all_keys(self) -> bool {
        matches!(self, Self::Kitty { flags } if flags & KITTY_FLAG_REPORT_ALL_KEYS != 0)
    }
}

#[cfg(any(unix, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseProtocolMode {
    None,
    Press,
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

#[cfg(any(unix, test))]
impl MouseProtocolMode {
    #[cfg(test)]
    pub fn reporting_enabled(self) -> bool {
        self != Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseProtocolEncoding {
    Default,
    Utf8,
    Sgr,
    SgrPixels,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_source_exposes_typed_identity_without_changing_semantics() {
        let record = WindowsKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code: 27,
            virtual_scan_code: 1,
            unicode: 27,
            control_key_state: 0,
        };
        let key = TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()).with_windows_record(record);
        let enhanced = TerminalKey::new(KeyCode::Esc, KeyModifiers::empty()).with_windows_record(
            WindowsKeyRecord {
                control_key_state: 0x0100,
                ..record
            },
        );

        assert!(matches!(key.identity(), KeyIdentity::Physical(_)));
        assert_ne!(key.identity(), enhanced.identity());
        assert_eq!(key.code, KeyCode::Esc);
    }

    #[test]
    fn semantic_source_uses_semantic_identity() {
        let key = TerminalKey::new(KeyCode::Char('x'), KeyModifiers::CONTROL);

        assert_eq!(key.identity(), KeyIdentity::Semantic(KeyCode::Char('x')));
        assert!(!key.has_physical_identity());
        assert_eq!(key.windows_record(), None);
    }

    #[test]
    fn native_source_remains_immutable_when_canonical_phase_changes() {
        let key = TerminalKey::new(KeyCode::Esc, KeyModifiers::empty())
            .with_windows_record(WindowsKeyRecord {
                key_down: true,
                repeat_count: 1,
                virtual_key_code: 27,
                virtual_scan_code: 1,
                unicode: 27,
                control_key_state: 0,
            })
            .with_kind(crossterm::event::KeyEventKind::Release);

        assert_eq!(key.kind, crossterm::event::KeyEventKind::Release);
        assert_eq!(
            key.windows_record().map(|record| record.key_down),
            Some(true)
        );
        assert_eq!(key.repeat_count, 1);
    }

    #[test]
    fn windows_composition_hint_requires_uncommitted_text_not_a_command() {
        let record = WindowsKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code: 52,
            virtual_scan_code: 5,
            unicode: 0,
            control_key_state: 9,
        };
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::SUPER,
        ] {
            let key = TerminalKey::new(KeyCode::Char('4'), modifiers)
                .with_windows_composition_hint(Some(record));
            assert!(
                !key.is_windows_dead_key(),
                "command modifiers: {modifiers:?}"
            );
        }
        for (code, source) in [
            (KeyCode::Left, Some(record)),
            (KeyCode::Char('4'), None),
            (
                KeyCode::Char('~'),
                Some(WindowsKeyRecord {
                    unicode: 126,
                    ..record
                }),
            ),
        ] {
            let key =
                TerminalKey::new(code, KeyModifiers::empty()).with_windows_composition_hint(source);
            assert!(!key.is_windows_dead_key(), "{code:?}, {source:?}");
        }
    }

    #[test]
    fn release_clears_generated_text_and_grouped_repeat_count() {
        let release = TerminalKey::new(KeyCode::Char('a'), KeyModifiers::empty())
            .with_generated_text(Some("a".to_owned()))
            .with_repeat_count(4)
            .with_kind(crossterm::event::KeyEventKind::Release);
        let regrouped_release = release
            .clone()
            .with_repeat_count(4)
            .with_generated_text(Some("ignored".to_owned()));

        assert_eq!(release.generated_text, None);
        assert_eq!(release.repeat_count, 1);
        assert_eq!(regrouped_release.generated_text, None);
        assert_eq!(regrouped_release.repeat_count, 1);
    }

    #[test]
    fn non_ascii_uppercase_with_shift_is_committed_text() {
        let key = TerminalKey::new(KeyCode::Char('É'), KeyModifiers::SHIFT).with_text_commit();

        assert_eq!(key.generated_text.as_deref(), Some("É"));
    }

    #[test]
    fn protocol_from_zero_flags_is_legacy() {
        assert_eq!(
            KeyboardProtocol::from_kitty_flags(0),
            KeyboardProtocol::Legacy
        );
    }

    #[test]
    fn protocol_from_nonzero_flags_is_kitty() {
        assert_eq!(
            KeyboardProtocol::from_kitty_flags(7),
            KeyboardProtocol::Kitty { flags: 7 }
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn keyboard_enhancement_flags_stay_ime_compatible() {
        let flags = ime_compatible_keyboard_enhancement_flags();

        assert!(flags.contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS));
        assert!(!flags.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
    }

    #[test]
    fn modify_other_keys_mode_is_enabled_for_tmux() {
        assert_eq!(
            host_modify_other_keys_mode_for_env(true, Some("WezTerm"), true, true),
            Some(ModifyOtherKeysMode::Mode2)
        );
    }

    #[test]
    fn modify_other_keys_mode_is_enabled_for_wezterm_hosts() {
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("WezTerm"), false, false),
            Some(ModifyOtherKeysMode::Mode1)
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, None, true, false),
            Some(ModifyOtherKeysMode::Mode1)
        );
    }

    #[test]
    fn modify_other_keys_mode_is_enabled_for_alacritty_hosts() {
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, None, false, true),
            Some(ModifyOtherKeysMode::Mode1)
        );
    }

    #[test]
    fn modify_other_keys_mode_is_not_enabled_for_unknown_hosts() {
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, Some("ghostty"), false, false),
            None
        );
        assert_eq!(
            host_modify_other_keys_mode_for_env(false, None, false, false),
            None
        );
    }

    #[test]
    fn cyrillic_fallback_covers_ctrl_word_and_newline_keys() {
        assert_eq!(cyrillic_to_latin_shortcut_base('ц'), Some('w'));
        assert_eq!(cyrillic_to_latin_shortcut_base('Ц'), Some('w'));
        assert_eq!(cyrillic_to_latin_shortcut_base('о'), Some('j'));
        assert_eq!(cyrillic_to_latin_shortcut_base('О'), Some('j'));
        assert_eq!(cyrillic_to_latin_shortcut_base('ф'), Some('a'));
        assert_eq!(cyrillic_to_latin_shortcut_base('с'), Some('c'));
        assert_eq!(cyrillic_to_latin_shortcut_base('в'), Some('d'));
        assert_eq!(cyrillic_to_latin_shortcut_base('х'), Some('['));
        assert_eq!(cyrillic_to_latin_shortcut_base('w'), None);
        assert_eq!(cyrillic_to_latin_shortcut_base('文'), None);
    }

    #[test]
    fn shortcut_base_prefers_terminal_reported_base_key() {
        let key = TerminalKey::new(KeyCode::Char('ц'), KeyModifiers::CONTROL)
            .with_base_layout_codepoint('w' as u32);
        assert_eq!(key.shortcut_base_char(), Some('w'));
    }

    #[test]
    fn shortcut_normalization_applies_only_to_ctrl_and_super() {
        let ctrl =
            TerminalKey::new(KeyCode::Char('ц'), KeyModifiers::CONTROL).normalized_for_shortcut();
        assert_eq!(ctrl.code, KeyCode::Char('w'));

        let latin =
            TerminalKey::new(KeyCode::Char('w'), KeyModifiers::CONTROL).normalized_for_shortcut();
        assert_eq!(latin.code, KeyCode::Char('w'));

        let plain =
            TerminalKey::new(KeyCode::Char('ц'), KeyModifiers::empty()).normalized_for_shortcut();
        assert_eq!(plain.code, KeyCode::Char('ц'));

        let alt = TerminalKey::new(KeyCode::Char('ф'), KeyModifiers::ALT).normalized_for_shortcut();
        assert_eq!(alt.code, KeyCode::Char('ф'));

        let shift =
            TerminalKey::new(KeyCode::Char('Ц'), KeyModifiers::SHIFT).normalized_for_shortcut();
        assert_eq!(shift.code, KeyCode::Char('Ц'));
    }
}
