//! Native mouse buttons held concurrently, independent of event arrival order.
use gpui::{MouseButton, NavigationDirection};

#[derive(Clone, Copy, Default)]
pub(super) struct MouseButtons(u8);

impl MouseButtons {
    const ORDER: [MouseButton; 5] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Navigate(NavigationDirection::Back),
        MouseButton::Navigate(NavigationDirection::Forward),
    ];

    fn mask(button: MouseButton) -> u8 {
        match button {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            MouseButton::Navigate(NavigationDirection::Back) => 8,
            MouseButton::Navigate(NavigationDirection::Forward) => 16,
        }
    }

    pub(super) fn press(&mut self, button: MouseButton) {
        self.0 |= Self::mask(button);
    }

    pub(super) fn release(&mut self, button: MouseButton) {
        self.0 &= !Self::mask(button);
    }

    pub(super) fn contains(self, button: MouseButton) -> bool {
        self.0 & Self::mask(button) != 0
    }

    pub(super) fn iter(self) -> impl Iterator<Item = MouseButton> {
        Self::ORDER
            .into_iter()
            .filter(move |button| self.contains(*button))
    }

    pub(super) fn first(self) -> Option<MouseButton> {
        Self::ORDER
            .into_iter()
            .find(|button| self.contains(*button))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releasing_one_button_retains_other_buttons_for_motion() {
        let mut held = MouseButtons::default();
        held.press(MouseButton::Right);
        held.press(MouseButton::Left);
        assert_eq!(held.first(), Some(MouseButton::Left));
        held.release(MouseButton::Right);
        assert_eq!(held.first(), Some(MouseButton::Left));
        held.release(MouseButton::Right);
        assert!(held.contains(MouseButton::Left));
        held.press(MouseButton::Middle);
        held.release(MouseButton::Left);
        assert_eq!(held.first(), Some(MouseButton::Middle));
        held.release(MouseButton::Middle);
        assert_eq!(held.first(), None);
        held.press(MouseButton::Navigate(NavigationDirection::Forward));
        held.press(MouseButton::Navigate(NavigationDirection::Back));
        assert_eq!(
            held.first(),
            Some(MouseButton::Navigate(NavigationDirection::Back))
        );
        held.release(MouseButton::Navigate(NavigationDirection::Back));
        assert_eq!(
            held.first(),
            Some(MouseButton::Navigate(NavigationDirection::Forward))
        );
    }
}
