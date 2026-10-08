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
                .map(|character| character.to_string())
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
    } else {
        let key = key_name(code)?;
        if key.len() != 1 {
            return None;
        }
        let plain = key.chars().next()?;
        if modifiers.shift
            && !matches!(
                code,
                KeyCode::NumpadDivide
                    | KeyCode::NumpadMultiply
                    | KeyCode::NumpadSubtract
                    | KeyCode::NumpadAdd
                    | KeyCode::NumpadDot
                    | KeyCode::NumpadComma
                    | KeyCode::NumpadEquals
                    | KeyCode::NumpadLeftParen
                    | KeyCode::NumpadRightParen
            )
        {
            match plain {
                '1' => '!',
                '2' => '@',
                '3' => '#',
                '4' => '$',
                '5' => '%',
                '6' => '^',
                '7' => '&',
                '8' => '*',
                '9' => '(',
                '0' => ')',
                '-' => '_',
                '=' => '+',
                '[' => '{',
                ']' => '}',
                '\\' => '|',
                ';' => ':',
                '\'' => '"',
                ',' => '<',
                '.' => '>',
                '/' => '?',
                '`' => '~',
                _ => plain,
            }
        } else {
            plain
        }
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
