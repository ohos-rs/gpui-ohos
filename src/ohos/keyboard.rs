use rustc_hash::FxHashMap as HashMap;
use std::collections::VecDeque;

use openharmony_ability::KeyboardEventData;
use openharmony_ability::xcomponent::{Action, KeyCode, KeyEventData};

use crate::{
    Capslock, KeyDownEvent, KeyUpEvent, KeybindingKeystroke, Keystroke, Modifiers,
    ModifiersChangedEvent, PlatformInput, PlatformKeyboardLayout, PlatformKeyboardMapper,
};

#[derive(Default)]
pub(crate) struct OhosKeyState {
    pressed: Vec<KeyCode>,
    modifiers: Modifiers,
    capslock: Capslock,
    numlock: bool,
    pending_ime_deletion: Option<KeyboardEventData>,
    pending_ime_escape: Option<KeyboardEventData>,
    keys_after_ime_escape: Vec<KeyboardEventData>,
    recovered_keys: HashMap<i64, RecoveredKeys>,
    pre_ime_dispatches: VecDeque<(KeyboardEventData, bool)>,
}

struct RecoveredKeys {
    first_timestamp: i64,
    timestamp: i64,
    keys_at_timestamp: Vec<(KeyCode, Action)>,
}

impl OhosKeyState {
    pub(crate) fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub(crate) fn capslock(&self) -> Capslock {
        self.capslock
    }

    pub(crate) fn clear_pressed(&mut self) -> Option<PlatformInput> {
        self.pending_ime_deletion = None;
        self.cancel_pending_ime_escape();
        self.pressed.clear();
        if self.modifiers == Modifiers::default() {
            return None;
        }
        self.modifiers = Modifiers::default();
        Some(self.modifiers_changed())
    }

    pub(crate) fn handle(&mut self, event: &KeyEventData) -> Vec<PlatformInput> {
        self.handle_key(event.code, event.action, None)
    }

    pub(crate) fn handle_arkui(&mut self, event: &KeyboardEventData) -> Vec<PlatformInput> {
        if self
            .pending_ime_deletion
            .as_ref()
            .is_some_and(|pending| Self::same_native_key(pending, event))
        {
            self.pending_ime_deletion = None;
        }
        self.handle_key(event.code, event.action, Some(event))
    }

    pub(crate) fn is_pre_ime_shortcut(&self, event: &KeyboardEventData) -> bool {
        event.code == KeyCode::Space
            && event.action == Action::Down
            && self.modifiers.control
            && !self.modifiers.alt
            && !self.modifiers.platform
            && !self.modifiers.function
    }

    pub(crate) fn take_ime_deletion(&mut self, forward: bool) -> Option<KeyboardEventData> {
        self.pending_ime_deletion.take().filter(|event| {
            event.code
                == if forward {
                    KeyCode::ForwardDel
                } else {
                    KeyCode::Del
                }
        })
    }

    pub(crate) fn cancel_pending_ime_deletion(&mut self) {
        self.pending_ime_deletion = None;
    }

    pub(crate) fn record_pre_ime_dispatch(&mut self, event: &KeyboardEventData, consumed: bool) {
        // Consumed presses have no post-IME callback. Bound their receipts while
        // retaining recent unhandled presses so native fallthrough cannot dispatch twice.
        if self.pre_ime_dispatches.len() == 256 {
            self.pre_ime_dispatches.pop_front();
        }
        self.pre_ime_dispatches.push_back((event.clone(), consumed));
        self.keys_after_ime_escape
            .retain(|pending| !Self::same_native_key(pending, event));
    }

    pub(crate) fn take_pre_ime_dispatch(&mut self, event: &KeyboardEventData) -> Option<bool> {
        let index = self
            .pre_ime_dispatches
            .iter()
            .position(|(pending, _)| Self::same_native_key(pending, event))?;
        self.pre_ime_dispatches
            .remove(index)
            .map(|(_, consumed)| consumed)
    }

    pub(crate) fn record_pre_ime(&mut self, event: &KeyboardEventData) {
        match event.action {
            Action::Down => {
                self.pending_ime_deletion =
                    matches!(event.code, KeyCode::Del | KeyCode::ForwardDel).then(|| event.clone());
            }
            Action::Up
                if self.pending_ime_deletion.as_ref().is_some_and(|pending| {
                    pending.device_id == event.device_id && pending.code == event.code
                }) =>
            {
                self.pending_ime_deletion = None;
            }
            Action::Up | Action::Unknown => {}
        }
        if self.pending_ime_escape.is_none()
            && event.code == KeyCode::Escape
            && event.action == Action::Down
        {
            self.pending_ime_escape = Some(event.clone());
            self.keys_after_ime_escape.clear();
        } else if self.pending_ime_escape.is_some() {
            // A composing IME can consume Escape without returning it. Bound
            // this optional recovery burst and fall back to native delivery.
            if self.keys_after_ime_escape.len() == 256 {
                self.cancel_pending_ime_escape();
            } else {
                self.keys_after_ime_escape.push(event.clone());
            }
        }
    }

    pub(crate) fn take_keys_after_ime_escape(
        &mut self,
        event: &KeyboardEventData,
    ) -> Option<Vec<KeyboardEventData>> {
        if self
            .pending_ime_escape
            .as_ref()
            .is_some_and(|pending| Self::same_native_key(pending, event))
        {
            self.pending_ime_escape = None;
            return Some(std::mem::take(&mut self.keys_after_ime_escape));
        }
        if event.code == KeyCode::Escape
            && event.action == Action::Down
            && let Some(index) = self
                .keys_after_ime_escape
                .iter()
                .position(|pending| Self::same_native_key(pending, event))
        {
            // Earlier Escapes may have cancelled native composition. Only the
            // Escape actually declined by the IME can release subsequent commands.
            self.pending_ime_escape = None;
            let following = self.keys_after_ime_escape.split_off(index + 1);
            self.keys_after_ime_escape.clear();
            return Some(following);
        }
        self.keys_after_ime_escape
            .retain(|pending| !Self::same_native_key(pending, event));
        None
    }

    pub(crate) fn cancel_pending_ime_escape(&mut self) {
        self.pending_ime_escape = None;
        self.keys_after_ime_escape.clear();
    }

    pub(crate) fn record_recovered_key(&mut self, event: &KeyboardEventData) {
        // Detach can discard callbacks entirely. Retain the completed physical
        // timeline without retaining every snapshot or relying on its reply lifetime.
        let recovered = self
            .recovered_keys
            .entry(event.device_id)
            .or_insert_with(|| RecoveredKeys {
                first_timestamp: event.timestamp,
                timestamp: event.timestamp,
                keys_at_timestamp: Vec::new(),
            });
        if event.timestamp > recovered.timestamp {
            recovered.timestamp = event.timestamp;
            recovered.keys_at_timestamp.clear();
        }
        if event.timestamp == recovered.timestamp {
            let key = (event.code, event.action);
            if !recovered.keys_at_timestamp.contains(&key) {
                recovered.keys_at_timestamp.push(key);
            }
        }
    }

    pub(crate) fn key_was_recovered(&self, event: &KeyboardEventData) -> bool {
        self.recovered_keys
            .get(&event.device_id)
            .is_some_and(|recovered| {
                (event.timestamp >= recovered.first_timestamp
                    && event.timestamp < recovered.timestamp)
                    || (event.timestamp == recovered.timestamp
                        && recovered
                            .keys_at_timestamp
                            .contains(&(event.code, event.action)))
            })
    }

    fn same_native_key(left: &KeyboardEventData, right: &KeyboardEventData) -> bool {
        left.device_id == right.device_id
            && left.timestamp == right.timestamp
            && left.code == right.code
            && left.action == right.action
    }

    pub(crate) fn handle_arkui_state(
        &mut self,
        event: &KeyboardEventData,
    ) -> Option<PlatformInput> {
        let previous_modifiers = self.modifiers;
        let previous_capslock = self.capslock;
        if let Some(pressed) = &event.pressed_keys {
            // Clear releases even when the IME consumes their post-IME event.
            // Never add the current down here: its first post-IME press is not a repeat.
            self.pressed.retain(|code| pressed.contains(code));
            self.modifiers = modifiers_for_keys(pressed);
        }
        if event.action == Action::Up {
            self.pressed.retain(|code| *code != event.code);
        }
        if let Some(state) = event.caps_lock {
            self.capslock.on = state;
        }
        if let Some(state) = event.num_lock {
            self.numlock = state;
        }
        (self.modifiers != previous_modifiers || self.capslock != previous_capslock)
            .then(|| self.modifiers_changed())
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        action: Action,
        snapshot: Option<&KeyboardEventData>,
    ) -> Vec<PlatformInput> {
        let held = self.pressed.contains(&code);
        if let Some(pressed) = snapshot.and_then(|event| event.pressed_keys.as_ref()) {
            self.pressed.clone_from(pressed);
        }
        match action {
            Action::Down if !self.pressed.contains(&code) => self.pressed.push(code),
            Action::Up => self.pressed.retain(|pressed| *pressed != code),
            Action::Down | Action::Unknown => {}
        }
        if action == Action::Unknown {
            return Vec::new();
        }

        let previous_modifiers = self.modifiers;
        let previous_capslock = self.capslock;
        if code == KeyCode::CapsLock && action == Action::Down && !held {
            self.capslock.on = !self.capslock.on;
        }
        if code == KeyCode::NumLock && action == Action::Down && !held {
            self.numlock = !self.numlock;
        }
        if let Some(state) = snapshot.and_then(|event| event.caps_lock) {
            self.capslock.on = state;
        }
        if let Some(state) = snapshot.and_then(|event| event.num_lock) {
            self.numlock = state;
        }
        self.modifiers = modifiers_for_keys(&self.pressed);

        let mut output = Vec::with_capacity(2);
        if self.modifiers != previous_modifiers || self.capslock != previous_capslock {
            output.push(self.modifiers_changed());
        }
        if is_modifier(code) {
            return output;
        }
        let Some(key) =
            keypad_navigation(code, self.numlock, self.modifiers.shift).or_else(|| key_name(code))
        else {
            return output;
        };
        let key_char = if self.modifiers.control
            || self.modifiers.alt
            || self.modifiers.platform
            || self.modifiers.function
            || keypad_navigation(code, self.numlock, self.modifiers.shift).is_some()
        {
            None
        } else {
            snapshot
                .and_then(|event| char::from_u32(event.unicode))
                .filter(|character| !character.is_control())
                .map(|character| {
                    // ArkUI can report the shifted glyph regardless of modifier state.
                    // Normalize known key pairs, retaining other layout translations.
                    let character = if character.is_ascii_alphabetic() {
                        if self.modifiers.shift ^ self.capslock.on {
                            character.to_ascii_uppercase()
                        } else {
                            character.to_ascii_lowercase()
                        }
                    } else if let Some((plain, shifted)) = printable_key_pair(code)
                        && (character == plain || character == shifted)
                    {
                        if self.modifiers.shift { shifted } else { plain }
                    } else {
                        character
                    };
                    character.to_string()
                })
                .or_else(|| printable_char(code, self.modifiers, self.capslock))
        };
        let keystroke = Keystroke {
            modifiers: self.modifiers,
            key: key.to_owned(),
            key_char,
        };
        match action {
            Action::Down => output.push(PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held: held,
                prefer_character_input: false,
            })),
            Action::Up => output.push(PlatformInput::KeyUp(KeyUpEvent { keystroke })),
            Action::Unknown => {}
        }
        output
    }

    fn modifiers_changed(&self) -> PlatformInput {
        PlatformInput::ModifiersChanged(ModifiersChangedEvent {
            modifiers: self.modifiers,
            capslock: self.capslock,
        })
    }
}

fn modifiers_for_keys(pressed: &[KeyCode]) -> Modifiers {
    Modifiers {
        control: pressed.contains(&KeyCode::CtrlLeft) || pressed.contains(&KeyCode::CtrlRight),
        alt: pressed.contains(&KeyCode::AltLeft) || pressed.contains(&KeyCode::AltRight),
        shift: pressed.contains(&KeyCode::ShiftLeft) || pressed.contains(&KeyCode::ShiftRight),
        platform: pressed.contains(&KeyCode::MetaLeft) || pressed.contains(&KeyCode::MetaRight),
        function: pressed.contains(&KeyCode::Fn) || pressed.contains(&KeyCode::Function),
    }
}

fn is_modifier(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::CtrlLeft
            | KeyCode::CtrlRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::MetaLeft
            | KeyCode::MetaRight
            | KeyCode::Fn
            | KeyCode::Function
            | KeyCode::CapsLock
            | KeyCode::NumLock
    )
}

fn keypad_navigation(code: KeyCode, numlock: bool, shift: bool) -> Option<&'static str> {
    if numlock && !shift {
        return None;
    }
    match code {
        KeyCode::Numpad0 => Some("insert"),
        KeyCode::Numpad1 => Some("end"),
        KeyCode::Numpad2 => Some("down"),
        KeyCode::Numpad3 => Some("pagedown"),
        KeyCode::Numpad4 => Some("left"),
        KeyCode::Numpad5 => Some("clear"),
        KeyCode::Numpad6 => Some("right"),
        KeyCode::Numpad7 => Some("home"),
        KeyCode::Numpad8 => Some("up"),
        KeyCode::Numpad9 => Some("pageup"),
        KeyCode::NumpadDot => Some("delete"),
        _ => None,
    }
}

fn letter(code: KeyCode) -> Option<char> {
    match code {
        KeyCode::A => Some('a'),
        KeyCode::B => Some('b'),
        KeyCode::C => Some('c'),
        KeyCode::D => Some('d'),
        KeyCode::E => Some('e'),
        KeyCode::F => Some('f'),
        KeyCode::G => Some('g'),
        KeyCode::H => Some('h'),
        KeyCode::I => Some('i'),
        KeyCode::J => Some('j'),
        KeyCode::K => Some('k'),
        KeyCode::L => Some('l'),
        KeyCode::M => Some('m'),
        KeyCode::N => Some('n'),
        KeyCode::O => Some('o'),
        KeyCode::P => Some('p'),
        KeyCode::Q => Some('q'),
        KeyCode::R => Some('r'),
        KeyCode::S => Some('s'),
        KeyCode::T => Some('t'),
        KeyCode::U => Some('u'),
        KeyCode::V => Some('v'),
        KeyCode::W => Some('w'),
        KeyCode::X => Some('x'),
        KeyCode::Y => Some('y'),
        KeyCode::Z => Some('z'),
        _ => None,
    }
}

fn key_name(code: KeyCode) -> Option<&'static str> {
    if let Some(letter) = letter(code) {
        return Some(match letter {
            'a' => "a",
            'b' => "b",
            'c' => "c",
            'd' => "d",
            'e' => "e",
            'f' => "f",
            'g' => "g",
            'h' => "h",
            'i' => "i",
            'j' => "j",
            'k' => "k",
            'l' => "l",
            'm' => "m",
            'n' => "n",
            'o' => "o",
            'p' => "p",
            'q' => "q",
            'r' => "r",
            's' => "s",
            't' => "t",
            'u' => "u",
            'v' => "v",
            'w' => "w",
            'x' => "x",
            'y' => "y",
            'z' => "z",
            _ => unreachable!(),
        });
    }
    match code {
        KeyCode::Key0 | KeyCode::Numpad0 => Some("0"),
        KeyCode::Key1 | KeyCode::Numpad1 => Some("1"),
        KeyCode::Key2 | KeyCode::Numpad2 => Some("2"),
        KeyCode::Key3 | KeyCode::Numpad3 => Some("3"),
        KeyCode::Key4 | KeyCode::Numpad4 => Some("4"),
        KeyCode::Key5 | KeyCode::Numpad5 => Some("5"),
        KeyCode::Key6 | KeyCode::Numpad6 => Some("6"),
        KeyCode::Key7 | KeyCode::Numpad7 => Some("7"),
        KeyCode::Key8 | KeyCode::Numpad8 => Some("8"),
        KeyCode::Key9 | KeyCode::Numpad9 => Some("9"),
        KeyCode::DpadUp => Some("up"),
        KeyCode::DpadDown => Some("down"),
        KeyCode::DpadLeft => Some("left"),
        KeyCode::DpadRight => Some("right"),
        KeyCode::DpadCenter | KeyCode::Enter | KeyCode::NumpadEnter => Some("enter"),
        KeyCode::Del => Some("backspace"),
        KeyCode::ForwardDel => Some("delete"),
        KeyCode::Tab => Some("tab"),
        KeyCode::Space => Some("space"),
        KeyCode::Escape => Some("escape"),
        KeyCode::PageUp => Some("pageup"),
        KeyCode::PageDown => Some("pagedown"),
        KeyCode::MoveHome => Some("home"),
        KeyCode::MoveEnd => Some("end"),
        KeyCode::Insert => Some("insert"),
        KeyCode::Comma => Some(","),
        KeyCode::Period => Some("."),
        KeyCode::Grave => Some("`"),
        KeyCode::Minus => Some("-"),
        KeyCode::Equals => Some("="),
        KeyCode::LeftBracket => Some("["),
        KeyCode::RightBracket => Some("]"),
        KeyCode::Backslash => Some("\\"),
        KeyCode::Semicolon => Some(";"),
        KeyCode::Apostrophe => Some("'"),
        KeyCode::Slash => Some("/"),
        KeyCode::F1 => Some("f1"),
        KeyCode::F2 => Some("f2"),
        KeyCode::F3 => Some("f3"),
        KeyCode::F4 => Some("f4"),
        KeyCode::F5 => Some("f5"),
        KeyCode::F6 => Some("f6"),
        KeyCode::F7 => Some("f7"),
        KeyCode::F8 => Some("f8"),
        KeyCode::F9 => Some("f9"),
        KeyCode::F10 => Some("f10"),
        KeyCode::F11 => Some("f11"),
        KeyCode::F12 => Some("f12"),
        KeyCode::NumpadDivide => Some("/"),
        KeyCode::NumpadMultiply => Some("*"),
        KeyCode::NumpadSubtract => Some("-"),
        KeyCode::NumpadAdd => Some("+"),
        KeyCode::NumpadDot => Some("."),
        KeyCode::NumpadComma => Some(","),
        KeyCode::NumpadEquals => Some("="),
        KeyCode::NumpadLeftParen => Some("("),
        KeyCode::NumpadRightParen => Some(")"),
        KeyCode::NumpadPlusMinus => Some("±"),
        _ => None,
    }
}

fn printable_key_pair(code: KeyCode) -> Option<(char, char)> {
    Some(match code {
        KeyCode::Key0 => ('0', ')'),
        KeyCode::Key1 => ('1', '!'),
        KeyCode::Key2 => ('2', '@'),
        KeyCode::Key3 => ('3', '#'),
        KeyCode::Key4 => ('4', '$'),
        KeyCode::Key5 => ('5', '%'),
        KeyCode::Key6 => ('6', '^'),
        KeyCode::Key7 => ('7', '&'),
        KeyCode::Key8 => ('8', '*'),
        KeyCode::Key9 => ('9', '('),
        KeyCode::Minus => ('-', '_'),
        KeyCode::Equals => ('=', '+'),
        KeyCode::LeftBracket => ('[', '{'),
        KeyCode::RightBracket => (']', '}'),
        KeyCode::Backslash => ('\\', '|'),
        KeyCode::Semicolon => (';', ':'),
        KeyCode::Apostrophe => ('\'', '"'),
        KeyCode::Comma => (',', '<'),
        KeyCode::Period => ('.', '>'),
        KeyCode::Slash => ('/', '?'),
        KeyCode::Grave => ('`', '~'),
        _ => return None,
    })
}

fn printable_char(code: KeyCode, modifiers: Modifiers, capslock: Capslock) -> Option<String> {
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    let character = if code == KeyCode::Space {
        ' '
    } else if code == KeyCode::NumpadPlusMinus {
        '±'
    } else if let Some(letter) = letter(code) {
        if modifiers.shift ^ capslock.on {
            letter.to_ascii_uppercase()
        } else {
            letter
        }
    } else if let Some((plain, shifted)) = printable_key_pair(code) {
        if modifiers.shift { shifted } else { plain }
    } else {
        let key = key_name(code)?;
        if key.len() != 1 {
            return None;
        }
        key.chars().next()?
    };
    Some(character.to_string())
}

pub(crate) struct OhosKeyboardLayout;

impl PlatformKeyboardLayout for OhosKeyboardLayout {
    fn id(&self) -> &str {
        "ohos-default"
    }

    fn name(&self) -> &str {
        "OHOS Default"
    }
}

pub(crate) struct OhosKeyboardMapper;

impl PlatformKeyboardMapper for OhosKeyboardMapper {
    fn map_key_equivalent(
        &self,
        keystroke: Keystroke,
        _use_key_equivalents: bool,
    ) -> KeybindingKeystroke {
        KeybindingKeystroke::from_keystroke(keystroke)
    }

    fn get_key_equivalents(&self) -> Option<&HashMap<char, char>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openharmony_ability::xcomponent::EventSource;

    fn snapshot(code: KeyCode, unicode: u32, pressed: Vec<KeyCode>) -> KeyboardEventData {
        KeyboardEventData {
            code,
            action: Action::Down,
            device_id: 1,
            source: EventSource::Keyboard,
            timestamp: 0,
            unicode,
            pressed_keys: Some(pressed),
            caps_lock: Some(true),
            num_lock: Some(true),
            response: Default::default(),
        }
    }

    fn key_down(events: &[PlatformInput]) -> &KeyDownEvent {
        events
            .iter()
            .find_map(|event| match event {
                PlatformInput::KeyDown(event) => Some(event),
                _ => None,
            })
            .expect("missing key down")
    }

    #[test]
    fn only_control_space_down_is_offered_before_the_ime() {
        for (code, action, pressed, expected) in [
            (
                KeyCode::Space,
                Action::Down,
                vec![KeyCode::CtrlLeft, KeyCode::Space],
                true,
            ),
            (
                KeyCode::Space,
                Action::Down,
                vec![KeyCode::CtrlRight, KeyCode::ShiftLeft, KeyCode::Space],
                true,
            ),
            (KeyCode::Space, Action::Up, vec![KeyCode::CtrlLeft], false),
            (KeyCode::Space, Action::Down, vec![KeyCode::Space], false),
            (
                KeyCode::A,
                Action::Down,
                vec![KeyCode::CtrlLeft, KeyCode::A],
                false,
            ),
            (
                KeyCode::Space,
                Action::Down,
                vec![KeyCode::CtrlLeft, KeyCode::AltLeft, KeyCode::Space],
                false,
            ),
            (
                KeyCode::Space,
                Action::Down,
                vec![KeyCode::CtrlLeft, KeyCode::MetaLeft, KeyCode::Space],
                false,
            ),
            (
                KeyCode::Space,
                Action::Down,
                vec![KeyCode::CtrlLeft, KeyCode::Fn, KeyCode::Space],
                false,
            ),
        ] {
            let mut state = OhosKeyState::default();
            let mut event = snapshot(code, 0, pressed);
            event.action = action;
            state.handle_arkui_state(&event);
            assert_eq!(state.is_pre_ime_shortcut(&event), expected, "{event:?}");
        }
    }

    #[test]
    fn hardware_deletion_waits_for_the_ime_and_preserves_modifiers() {
        for code in [KeyCode::Del, KeyCode::ForwardDel] {
            for modifiers in [vec![], vec![KeyCode::CtrlLeft], vec![KeyCode::ShiftLeft]] {
                let mut pressed = modifiers;
                pressed.push(code);
                let event = snapshot(code, 0, pressed);
                let mut state = OhosKeyState::default();
                state.record_pre_ime(&event);
                state.handle_arkui_state(&event);
                assert!(!state.is_pre_ime_shortcut(&event));
                let forward = code == KeyCode::ForwardDel;
                let deletion = state.take_ime_deletion(forward).expect("physical deletion");
                let inputs = state.handle_arkui(&deletion);
                let key = key_down(&inputs);
                assert_eq!(
                    key.keystroke.key,
                    if forward { "delete" } else { "backspace" }
                );
                assert_eq!(key.keystroke.modifiers, state.modifiers());
                assert!(state.take_ime_deletion(forward).is_none());
            }
        }
    }

    #[test]
    fn ime_deletion_receipt_expires_before_unrelated_edits() {
        let deletion = snapshot(KeyCode::Del, 0, vec![KeyCode::Del]);
        let mut state = OhosKeyState::default();
        state.record_pre_ime(&deletion);
        assert!(state.take_ime_deletion(true).is_none());
        assert!(state.take_ime_deletion(false).is_none());
        state.record_pre_ime(&deletion);
        state.record_pre_ime(&snapshot(KeyCode::A, 'a' as u32, vec![KeyCode::A]));
        assert!(state.take_ime_deletion(false).is_none());
        state.record_pre_ime(&deletion);
        let mut release = deletion.clone();
        release.action = Action::Up;
        release.timestamp += 1;
        state.record_pre_ime(&release);
        assert!(state.take_ime_deletion(false).is_none());
        state.record_pre_ime(&deletion);
        state.handle_arkui(&deletion);
        assert!(state.take_ime_deletion(false).is_none());
        state.record_pre_ime(&deletion);
        state.cancel_pending_ime_deletion();
        assert!(state.take_ime_deletion(false).is_none());
        state.record_pre_ime(&deletion);
        state.clear_pressed();
        assert!(state.take_ime_deletion(false).is_none());
    }

    #[test]
    fn pre_ime_dispatch_receipts_match_the_exact_native_press_once() {
        for consumed in [false, true] {
            let mut state = OhosKeyState::default();
            let event = snapshot(KeyCode::Space, 0, vec![KeyCode::CtrlLeft, KeyCode::Space]);
            state.record_pre_ime_dispatch(&event, consumed);
            let mut other = event.clone();
            other.device_id += 1;
            assert!(state.take_pre_ime_dispatch(&other).is_none());
            other = event.clone();
            other.timestamp += 1;
            assert!(state.take_pre_ime_dispatch(&other).is_none());
            other = event.clone();
            other.action = Action::Up;
            assert!(state.take_pre_ime_dispatch(&other).is_none());
            other = event.clone();
            other.code = KeyCode::A;
            assert!(state.take_pre_ime_dispatch(&other).is_none());
            state.clear_pressed();
            assert_eq!(state.take_pre_ime_dispatch(&event), Some(consumed));
            assert!(state.take_pre_ime_dispatch(&event).is_none());
        }
    }

    #[test]
    fn pre_ime_shortcut_is_not_replayed_after_a_delayed_escape() {
        let mut state = OhosKeyState::default();
        let escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        let shortcut = snapshot(KeyCode::Space, 0, vec![KeyCode::CtrlLeft, KeyCode::Space]);
        state.record_pre_ime(&escape);
        state.record_pre_ime(&shortcut);
        state.record_pre_ime_dispatch(&shortcut, true);
        let pending = state
            .take_keys_after_ime_escape(&escape)
            .expect("matching Escape");
        assert!(pending.is_empty());
    }

    #[test]
    fn repeated_pre_ime_space_preserves_held_key_state() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::Space, 0, vec![KeyCode::CtrlLeft, KeyCode::Space]);
        state.handle_arkui_state(&event);
        assert!(!key_down(&state.handle_arkui(&event)).is_held);
        state.record_pre_ime_dispatch(&event, true);
        event.timestamp += 1;
        state.handle_arkui_state(&event);
        assert!(key_down(&state.handle_arkui(&event)).is_held);
        event.action = Action::Up;
        event.pressed_keys = Some(vec![KeyCode::CtrlLeft]);
        state.handle_arkui_state(&event);
        state.handle_arkui(&event);
        event.action = Action::Down;
        event.pressed_keys = Some(vec![KeyCode::CtrlLeft, KeyCode::Space]);
        state.handle_arkui_state(&event);
        assert!(!key_down(&state.handle_arkui(&event)).is_held);
    }

    #[test]
    fn ime_escape_retains_following_keys_until_the_matching_return() {
        let mut state = OhosKeyState::default();
        let escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        let undo = snapshot(KeyCode::U, 'u' as u32, vec![KeyCode::U]);
        state.record_pre_ime(&escape);
        state.record_pre_ime(&undo);
        let following = state
            .take_keys_after_ime_escape(&escape)
            .expect("matching Escape must release retained keys");
        assert_eq!(following.len(), 1);
        assert_eq!(following.first().map(|event| event.code), Some(KeyCode::U));
        assert!(state.take_keys_after_ime_escape(&escape).is_none());
    }

    #[test]
    fn native_edits_and_focus_loss_cancel_escape_recovery() {
        let mut state = OhosKeyState::default();
        let escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        let undo = snapshot(KeyCode::U, 'u' as u32, vec![KeyCode::U]);
        for loses_focus in [false, true] {
            state.record_pre_ime(&escape);
            state.record_pre_ime(&undo);
            if loses_focus {
                state.clear_pressed();
            } else {
                state.cancel_pending_ime_escape();
            }
            assert!(state.take_keys_after_ime_escape(&escape).is_none());
        }
    }

    #[test]
    fn a_later_declined_escape_recovers_only_the_keys_after_it() {
        let mut state = OhosKeyState::default();
        let mut composition_escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        composition_escape.timestamp = 1;
        let mut composition_key = snapshot(KeyCode::N, 0, vec![KeyCode::N]);
        composition_key.timestamp = 2;
        let mut later_escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        later_escape.timestamp = 3;
        let mut undo = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        undo.timestamp = 4;
        for event in [&composition_escape, &composition_key, &later_escape, &undo] {
            state.record_pre_ime(event);
        }
        let following = state
            .take_keys_after_ime_escape(&later_escape)
            .expect("later unconsumed Escape");
        assert_eq!(following.len(), 1);
        assert_eq!(following.first().map(|event| event.code), Some(KeyCode::U));
        assert!(
            state
                .take_keys_after_ime_escape(&composition_escape)
                .is_none()
        );
    }

    #[test]
    fn consecutive_escapes_do_not_replace_the_first_pending_escape() {
        let mut state = OhosKeyState::default();
        let mut first_escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        first_escape.timestamp = 1;
        let mut later_escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        later_escape.timestamp = 2;
        let mut undo = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        undo.timestamp = 3;
        for event in [&first_escape, &later_escape, &undo] {
            state.record_pre_ime(event);
        }
        let following = state
            .take_keys_after_ime_escape(&first_escape)
            .expect("first unconsumed Escape");
        assert_eq!(
            following.iter().map(|event| event.code).collect::<Vec<_>>(),
            vec![KeyCode::Escape, KeyCode::U]
        );
    }

    #[test]
    fn late_callbacks_stay_suppressed_after_owned_snapshots_are_released() {
        let mut state = OhosKeyState::default();
        let mut recovered = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        recovered.timestamp = 10;
        state.record_recovered_key(&recovered);
        drop(recovered);
        let mut late = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        late.timestamp = 10;
        assert!(state.key_was_recovered(&late));
        state.clear_pressed();
        assert!(state.key_was_recovered(&late));
        late.timestamp = 11;
        assert!(!state.key_was_recovered(&late));
        late.timestamp = 10;
        late.device_id = 2;
        assert!(!state.key_was_recovered(&late));
    }

    #[test]
    fn recovery_does_not_consume_callbacks_before_the_recovered_timeline() {
        let mut state = OhosKeyState::default();
        let mut recovered = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        recovered.timestamp = 10;
        state.record_recovered_key(&recovered);
        let mut earlier = snapshot(KeyCode::A, 0, vec![KeyCode::A]);
        earlier.timestamp = 9;
        assert!(!state.key_was_recovered(&earlier));
    }

    #[test]
    fn a_consumed_escape_cannot_retain_an_unbounded_key_stream() {
        let mut state = OhosKeyState::default();
        let escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        state.record_pre_ime(&escape);
        for timestamp in 1..=257 {
            let mut event = snapshot(KeyCode::A, 0, vec![KeyCode::A]);
            event.timestamp = timestamp;
            state.record_pre_ime(&event);
        }
        assert!(state.take_keys_after_ime_escape(&escape).is_none());
    }

    #[test]
    fn equal_timestamps_keep_other_keys_and_releases_distinct() {
        let mut state = OhosKeyState::default();
        let recovered = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        state.record_recovered_key(&recovered);
        let other_key = snapshot(KeyCode::V, 0, vec![KeyCode::V]);
        assert!(!state.key_was_recovered(&other_key));
        let mut release = snapshot(KeyCode::U, 0, vec![]);
        release.action = Action::Up;
        assert!(!state.key_was_recovered(&release));
        state.record_recovered_key(&release);
        assert!(state.key_was_recovered(&release));
    }

    #[test]
    fn a_post_ime_key_already_delivered_is_removed_from_recovery() {
        let mut state = OhosKeyState::default();
        let escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        let undo = snapshot(KeyCode::U, 0, vec![KeyCode::U]);
        state.record_pre_ime(&escape);
        state.record_pre_ime(&undo);
        assert!(state.take_keys_after_ime_escape(&undo).is_none());
        assert!(
            state
                .take_keys_after_ime_escape(&escape)
                .is_some_and(|pending| pending.is_empty())
        );
    }

    #[test]
    fn snapshots_recover_modifiers_and_preexisting_caps_lock() {
        let mut state = OhosKeyState::default();
        let event = snapshot(KeyCode::A, 'A' as u32, vec![KeyCode::CtrlLeft, KeyCode::A]);
        assert!(state.handle_arkui_state(&event).is_some());
        assert!(state.capslock().on);
        let inputs = state.handle_arkui(&event);
        assert!(key_down(&inputs).keystroke.modifiers.control);
        assert_eq!(key_down(&inputs).keystroke.key_char, None);
        state.clear_pressed();
        let event = snapshot(
            KeyCode::Key2,
            '"' as u32,
            vec![KeyCode::ShiftLeft, KeyCode::Key2],
        );
        let inputs = state.handle_arkui(&event);
        assert_eq!(key_down(&inputs).keystroke.key_char.as_deref(), Some("\""));
        assert!(!key_down(&inputs).keystroke.modifiers.control);
    }

    #[test]
    fn pre_ime_state_does_not_turn_first_press_into_repeat() {
        let mut state = OhosKeyState::default();
        let event = snapshot(KeyCode::A, 'A' as u32, vec![KeyCode::A]);
        state.handle_arkui_state(&event);
        let inputs = state.handle_arkui(&event);
        assert!(!key_down(&inputs).is_held);
        state.handle_arkui_state(&event);
        let inputs = state.handle_arkui(&event);
        assert!(key_down(&inputs).is_held);
        let mut release = event.clone();
        release.action = Action::Up;
        release.pressed_keys = Some(vec![]);
        // The IME may consume the release, leaving only the pre-IME snapshot.
        state.handle_arkui_state(&release);
        state.handle_arkui_state(&event);
        let inputs = state.handle_arkui(&event);
        assert!(!key_down(&inputs).is_held);
    }

    #[test]
    fn unicode_letters_use_modifier_snapshot_for_case() {
        for (caps_lock, shift, expected) in [
            (None, false, "q"),
            (None, true, "Q"),
            (Some(false), false, "q"),
            (Some(false), true, "Q"),
            (Some(true), false, "Q"),
            (Some(true), true, "q"),
        ] {
            let mut state = OhosKeyState::default();
            let mut pressed = vec![KeyCode::A];
            if shift {
                pressed.push(KeyCode::ShiftLeft);
            }
            let mut event = snapshot(KeyCode::A, 'Q' as u32, pressed);
            event.caps_lock = caps_lock;
            state.handle_arkui_state(&event);
            let inputs = state.handle_arkui(&event);
            assert_eq!(
                key_down(&inputs).keystroke.key_char.as_deref(),
                Some(expected),
                "caps_lock={caps_lock:?}, shift={shift}"
            );
        }
    }

    #[test]
    fn unicode_symbols_use_shift_snapshot() {
        for (code, plain, shifted) in [
            (KeyCode::Key0, '0', ')'),
            (KeyCode::Key1, '1', '!'),
            (KeyCode::Key2, '2', '@'),
            (KeyCode::Key3, '3', '#'),
            (KeyCode::Key4, '4', '$'),
            (KeyCode::Key5, '5', '%'),
            (KeyCode::Key6, '6', '^'),
            (KeyCode::Key7, '7', '&'),
            (KeyCode::Key8, '8', '*'),
            (KeyCode::Key9, '9', '('),
            (KeyCode::Minus, '-', '_'),
            (KeyCode::Equals, '=', '+'),
            (KeyCode::LeftBracket, '[', '{'),
            (KeyCode::RightBracket, ']', '}'),
            (KeyCode::Backslash, '\\', '|'),
            (KeyCode::Semicolon, ';', ':'),
            (KeyCode::Apostrophe, '\'', '"'),
            (KeyCode::Comma, ',', '<'),
            (KeyCode::Period, '.', '>'),
            (KeyCode::Slash, '/', '?'),
            (KeyCode::Grave, '`', '~'),
        ] {
            for unicode in [0, plain as u32, shifted as u32] {
                for caps_lock in [false, true] {
                    let mut state = OhosKeyState::default();
                    for shift in [true, false, true] {
                        let mut pressed = vec![code];
                        if shift {
                            pressed.push(KeyCode::ShiftLeft);
                        }
                        let mut event = snapshot(code, unicode, pressed);
                        event.caps_lock = Some(caps_lock);
                        state.handle_arkui_state(&event);
                        let inputs = state.handle_arkui(&event);
                        assert_eq!(
                            key_down(&inputs).keystroke.key_char,
                            Some(if shift { shifted } else { plain }.to_string()),
                            "code={code:?}, unicode={unicode}, shift={shift}, caps={caps_lock}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn other_layout_symbols_are_preserved() {
        for (code, character) in [
            (KeyCode::Key2, 'é'),
            (KeyCode::Key2, '"'),
            (KeyCode::Key7, 'è'),
            (KeyCode::Semicolon, 'ö'),
        ] {
            for shift in [false, true] {
                let mut state = OhosKeyState::default();
                let mut pressed = vec![code];
                if shift {
                    pressed.push(KeyCode::ShiftLeft);
                }
                let event = snapshot(code, character as u32, pressed);
                let inputs = state.handle_arkui(&event);
                assert_eq!(
                    key_down(&inputs).keystroke.key_char,
                    Some(character.to_string()),
                );
            }
        }
    }

    #[test]
    fn keypad_navigation_and_space_have_text_semantics() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::Numpad1, 0, vec![KeyCode::Numpad1]);
        let inputs = state.handle_arkui(&event);
        assert_eq!(key_down(&inputs).keystroke.key_char.as_deref(), Some("1"));
        event.num_lock = Some(false);
        let inputs = state.handle_arkui(&event);
        assert_eq!(key_down(&inputs).keystroke.key, "end");
        assert_eq!(key_down(&inputs).keystroke.key_char, None);
        let event = snapshot(KeyCode::Space, 0, vec![KeyCode::Space]);
        let inputs = state.handle_arkui(&event);
        assert_eq!(key_down(&inputs).keystroke.key_char.as_deref(), Some(" "));
        let event = snapshot(
            KeyCode::NumpadSubtract,
            0,
            vec![KeyCode::ShiftLeft, KeyCode::NumpadSubtract],
        );
        let inputs = state.handle_arkui(&event);
        assert_eq!(key_down(&inputs).keystroke.key_char.as_deref(), Some("-"));
    }
}
