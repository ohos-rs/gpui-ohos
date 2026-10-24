use rustc_hash::FxHashMap as HashMap;

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
}

impl OhosKeyState {
    pub(crate) fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub(crate) fn capslock(&self) -> Capslock {
        self.capslock
    }

    pub(crate) fn clear_pressed(&mut self) -> Option<PlatformInput> {
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
        self.handle_key(event.code, event.action, Some(event))
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        action: Action,
        snapshot: Option<&KeyboardEventData>,
    ) -> Vec<PlatformInput> {
        if action == Action::Unknown {
            return Vec::new();
        }
        let held = self.pressed.contains(&code);
        if let Some(pressed) = snapshot.and_then(|event| event.pressed_keys.as_ref()) {
            self.pressed.clone_from(pressed);
        }
        match action {
            Action::Down if !self.pressed.contains(&code) => self.pressed.push(code),
            Action::Up => self.pressed.retain(|pressed| *pressed != code),
            Action::Down | Action::Unknown => {}
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
        // Ability delivers each physical transition once, including IME-consumed keys.
        // Their state still matters, but their text/edit/shortcut is owned by the IME.
        if snapshot.is_some_and(|event| event.ime_consumed) || is_modifier(code) {
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
                    // Normalize known shifted pairs without replacing other layouts.
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

fn key_name(code: KeyCode) -> Option<&'static str> {
    match code {
        KeyCode::A => Some("a"),
        KeyCode::B => Some("b"),
        KeyCode::C => Some("c"),
        KeyCode::D => Some("d"),
        KeyCode::E => Some("e"),
        KeyCode::F => Some("f"),
        KeyCode::G => Some("g"),
        KeyCode::H => Some("h"),
        KeyCode::I => Some("i"),
        KeyCode::J => Some("j"),
        KeyCode::K => Some("k"),
        KeyCode::L => Some("l"),
        KeyCode::M => Some("m"),
        KeyCode::N => Some("n"),
        KeyCode::O => Some("o"),
        KeyCode::P => Some("p"),
        KeyCode::Q => Some("q"),
        KeyCode::R => Some("r"),
        KeyCode::S => Some("s"),
        KeyCode::T => Some("t"),
        KeyCode::U => Some("u"),
        KeyCode::V => Some("v"),
        KeyCode::W => Some("w"),
        KeyCode::X => Some("x"),
        KeyCode::Y => Some("y"),
        KeyCode::Z => Some("z"),
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
    let key = key_name(code)?;
    let character = if code == KeyCode::Space {
        ' '
    } else if let Some((plain, shifted)) = printable_key_pair(code) {
        if modifiers.shift { shifted } else { plain }
    } else if key.len() == 1 {
        let character = key.chars().next()?;
        if character.is_ascii_alphabetic() && (modifiers.shift ^ capslock.on) {
            character.to_ascii_uppercase()
        } else {
            character
        }
    } else if code == KeyCode::NumpadPlusMinus {
        '±'
    } else {
        return None;
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
            caps_lock: Some(false),
            num_lock: Some(true),
            ime_consumed: false,
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

    fn key_count(events: &[PlatformInput]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, PlatformInput::KeyDown(_) | PlatformInput::KeyUp(_)))
            .count()
    }

    #[test]
    fn consumed_text_deletion_escape_and_confirm_never_dispatch_commands() {
        for code in [
            KeyCode::A,
            KeyCode::Space,
            KeyCode::Del,
            KeyCode::ForwardDel,
            KeyCode::Escape,
            KeyCode::Enter,
        ] {
            let mut state = OhosKeyState::default();
            let mut event = snapshot(code, 'a' as u32, vec![code]);
            event.ime_consumed = true;
            assert_eq!(key_count(&state.handle_arkui(&event)), 0);
            event.action = Action::Up;
            event.pressed_keys = Some(vec![]);
            assert_eq!(key_count(&state.handle_arkui(&event)), 0);
            event.action = Action::Down;
            event.ime_consumed = false;
            event.pressed_keys = Some(vec![code]);
            assert!(!key_down(&state.handle_arkui(&event)).is_held);
        }
    }

    #[test]
    fn consumed_modifier_changes_and_releases_still_update_state() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::CtrlLeft, 0, vec![KeyCode::CtrlLeft]);
        event.ime_consumed = true;
        assert!(matches!(
            state.handle_arkui(&event).as_slice(),
            [PlatformInput::ModifiersChanged(_)]
        ));
        assert!(state.modifiers().control);
        event.action = Action::Up;
        event.pressed_keys = Some(vec![]);
        assert_eq!(key_count(&state.handle_arkui(&event)), 0);
        assert!(!state.modifiers().control);
    }

    #[test]
    fn consumed_shortcut_is_not_dispatched_but_declined_shortcut_is() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::C, 0, vec![KeyCode::CtrlLeft, KeyCode::C]);
        event.ime_consumed = true;
        assert_eq!(key_count(&state.handle_arkui(&event)), 0);
        event.ime_consumed = false;
        let events = state.handle_arkui(&event);
        assert_eq!(key_count(&events), 1);
        assert!(key_down(&events).keystroke.modifiers.control);
        assert!(key_down(&events).is_held);
        assert_eq!(key_down(&events).keystroke.key_char, None);
    }

    #[test]
    fn identical_timestamps_are_independent_transitions_not_deduplicated() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::A, 'a' as u32, vec![KeyCode::A]);
        for _ in 0..3 {
            let events = state.handle_arkui(&event);
            assert_eq!(key_count(&events), 1);
            assert!(!key_down(&events).is_held);
            event.action = Action::Up;
            event.pressed_keys = Some(vec![]);
            assert_eq!(key_count(&state.handle_arkui(&event)), 1);
            event.action = Action::Down;
            event.pressed_keys = Some(vec![KeyCode::A]);
        }
    }

    #[test]
    fn repeats_and_consumed_release_without_snapshot_reset_held_state() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::A, 'a' as u32, vec![KeyCode::A]);
        assert!(!key_down(&state.handle_arkui(&event)).is_held);
        assert!(key_down(&state.handle_arkui(&event)).is_held);
        event.action = Action::Up;
        event.ime_consumed = true;
        event.pressed_keys = None;
        assert_eq!(key_count(&state.handle_arkui(&event)), 0);
        event.action = Action::Down;
        event.ime_consumed = false;
        assert!(!key_down(&state.handle_arkui(&event)).is_held);
    }

    #[test]
    fn focus_loss_clears_pressed_modifiers_but_preserves_lock_state() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::A, 0, vec![KeyCode::CtrlLeft, KeyCode::A]);
        event.caps_lock = Some(true);
        state.handle_arkui(&event);
        assert!(state.clear_pressed().is_some());
        assert!(!state.modifiers().control);
        assert!(state.capslock().on);
        assert!(!key_down(&state.handle_arkui(&event)).is_held);
    }

    #[test]
    fn consumed_escape_never_replays_following_keys() {
        let mut state = OhosKeyState::default();
        let mut escape = snapshot(KeyCode::Escape, 0, vec![KeyCode::Escape]);
        escape.ime_consumed = true;
        assert_eq!(key_count(&state.handle_arkui(&escape)), 0);
        let key = snapshot(KeyCode::U, 'u' as u32, vec![KeyCode::U]);
        assert_eq!(key_count(&state.handle_arkui(&key)), 1);
        escape.ime_consumed = false;
        assert_eq!(key_count(&state.handle_arkui(&escape)), 1);
    }

    #[test]
    fn every_declined_chord_is_offered_without_control_space_policy() {
        for code in [KeyCode::Space, KeyCode::C, KeyCode::A, KeyCode::F1] {
            let mut state = OhosKeyState::default();
            let event = snapshot(code, 0, vec![KeyCode::CtrlLeft, code]);
            let events = state.handle_arkui(&event);
            assert_eq!(key_count(&events), 1);
            assert!(key_down(&events).keystroke.modifiers.control);
        }
    }

    #[test]
    fn unicode_case_uses_modifier_and_lock_snapshots() {
        for (caps, shift, expected) in [
            (false, false, "q"),
            (false, true, "Q"),
            (true, false, "Q"),
            (true, true, "q"),
        ] {
            let mut state = OhosKeyState::default();
            let mut pressed = vec![KeyCode::A];
            if shift {
                pressed.push(KeyCode::ShiftLeft);
            }
            let mut event = snapshot(KeyCode::A, 'Q' as u32, pressed);
            event.caps_lock = Some(caps);
            assert_eq!(
                key_down(&state.handle_arkui(&event))
                    .keystroke
                    .key_char
                    .as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn unicode_symbols_normalize_known_pairs_and_preserve_other_layouts() {
        for code in [
            KeyCode::Key2,
            KeyCode::Minus,
            KeyCode::Grave,
            KeyCode::Slash,
        ] {
            let (plain, shifted) = printable_key_pair(code).unwrap();
            for unicode in [0, plain as u32, shifted as u32, 'é' as u32] {
                for shift in [false, true] {
                    let mut state = OhosKeyState::default();
                    let mut pressed = vec![code];
                    if shift {
                        pressed.push(KeyCode::ShiftLeft);
                    }
                    let event = snapshot(code, unicode, pressed);
                    let expected = if unicode == 'é' as u32 {
                        'é'
                    } else if shift {
                        shifted
                    } else {
                        plain
                    };
                    assert_eq!(
                        key_down(&state.handle_arkui(&event)).keystroke.key_char,
                        Some(expected.to_string())
                    );
                }
            }
        }
    }

    #[test]
    fn keypad_navigation_and_space_keep_text_semantics() {
        let mut state = OhosKeyState::default();
        let mut event = snapshot(KeyCode::Numpad1, 0, vec![KeyCode::Numpad1]);
        assert_eq!(
            key_down(&state.handle_arkui(&event))
                .keystroke
                .key_char
                .as_deref(),
            Some("1")
        );
        event.num_lock = Some(false);
        let events = state.handle_arkui(&event);
        assert_eq!(key_down(&events).keystroke.key, "end");
        assert_eq!(key_down(&events).keystroke.key_char, None);
        let event = snapshot(KeyCode::Space, 0, vec![KeyCode::Space]);
        assert_eq!(
            key_down(&state.handle_arkui(&event))
                .keystroke
                .key_char
                .as_deref(),
            Some(" ")
        );
    }
}
