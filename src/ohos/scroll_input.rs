//! Native wheel ticks/device pixels retained during synchronous GPUI dispatch.
use gpui::{Pixels, Point, ScrollDelta, WindowId, point};
use std::{cell::Cell, marker::PhantomData, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeScrollDelta {
    WheelTicks(Point<f64>),
    DevicePixels(Point<f64>),
}

#[derive(Clone, Copy)]
struct ScrollContext {
    window: WindowId,
    logical: ScrollDelta,
    scale: f32,
    native: Option<NativeScrollDelta>,
}

thread_local! {
    #[allow(clippy::missing_const_for_thread_local, reason = "OHOS std TLS expansion warns on an already-const initializer")]
    static CURRENT: Cell<Option<ScrollContext>> = const { Cell::new(None) };
}

pub(super) struct NativeScrollScope {
    previous: Option<ScrollContext>,
    _ui_thread: PhantomData<Rc<()>>,
}

impl Drop for NativeScrollScope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.previous));
    }
}

impl NativeScrollScope {
    pub(super) fn bind(
        window: WindowId,
        logical: ScrollDelta,
        scale: f32,
        native: Option<NativeScrollDelta>,
    ) -> Self {
        let previous = CURRENT.with(|current| {
            current.replace(Some(ScrollContext {
                window,
                logical,
                scale,
                native,
            }))
        });
        Self {
            previous,
            _ui_thread: PhantomData,
        }
    }
}

impl NativeScrollDelta {
    /// Query the exact native delta only while its event is dispatched. A
    /// recognizer may lock one axis to zero; that projection is preserved.
    pub fn get(window: WindowId, delta: ScrollDelta, scale: f32) -> Option<Self> {
        CURRENT.with(|current| {
            let context = current.get()?;
            if context.window != window || context.scale != scale {
                return None;
            }
            let (expected, actual) = match (context.logical, delta) {
                (ScrollDelta::Lines(expected), ScrollDelta::Lines(actual)) => (expected, actual),
                (ScrollDelta::Pixels(expected), ScrollDelta::Pixels(actual)) => {
                    (expected.map(Pixels::as_f32), actual.map(Pixels::as_f32))
                }
                _ => return None,
            };
            if (actual.x != 0. && actual.x != expected.x)
                || (actual.y != 0. && actual.y != expected.y)
            {
                return None;
            }
            let project = |value: Point<f64>| {
                point(
                    if actual.x == 0. && expected.x != 0. {
                        0.
                    } else {
                        value.x
                    },
                    if actual.y == 0. && expected.y != 0. {
                        0.
                    } else {
                        value.y
                    },
                )
            };
            Some(match context.native? {
                Self::WheelTicks(value) => Self::WheelTicks(project(value)),
                Self::DevicePixels(value) => Self::DevicePixels(project(value)),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::scroll_packet::ScrollPacket;
    use super::*;
    use gpui::px;

    #[test]
    fn native_pixels_keep_double_precision_and_are_scoped_to_the_matching_dispatch() {
        let window = WindowId::from(1);
        let raw = point(977., -0.50000000000001);
        let packet = ScrollPacket::pixels(raw, 1.9).unwrap();
        assert!(NativeScrollDelta::get(window, packet.delta, 1.9).is_none());
        {
            let _scope = packet.bind(window, 1.9);
            assert_eq!(
                NativeScrollDelta::get(window, packet.delta, 1.9),
                Some(NativeScrollDelta::DevicePixels(raw))
            );
            assert!(NativeScrollDelta::get(WindowId::from(2), packet.delta, 1.9).is_none());
            assert!(NativeScrollDelta::get(window, packet.delta, 2.).is_none());
            let ScrollDelta::Pixels(delta) = packet.delta else {
                unreachable!()
            };
            assert_eq!(
                NativeScrollDelta::get(window, ScrollDelta::Pixels(point(px(0.), delta.y)), 1.9),
                Some(NativeScrollDelta::DevicePixels(point(0., raw.y)))
            );
            assert!(
                NativeScrollDelta::get(window, ScrollDelta::Lines(point(1., 1.)), 1.9).is_none()
            );
        }
        assert!(NativeScrollDelta::get(window, packet.delta, 1.9).is_none());
        let tiny_raw = point(1e-100, -1e-100);
        let tiny = ScrollPacket::pixels(tiny_raw, 1.9).unwrap();
        let _scope = tiny.bind(window, 1.9);
        assert_eq!(
            NativeScrollDelta::get(window, tiny.delta, 1.9),
            Some(NativeScrollDelta::DevicePixels(tiny_raw))
        );
    }

    #[test]
    fn wheel_degrees_keep_system_steps_for_widgets_and_detents_for_terminal_clients() {
        let window = WindowId::from(1);
        let packet = ScrollPacket::wheel(point(45., -90.), 3).unwrap();
        let _outer = packet.bind(window, 1.9);
        assert!(matches!(packet.delta, ScrollDelta::Lines(delta) if delta == point(3., -6.)));
        assert_eq!(
            NativeScrollDelta::get(window, packet.delta, 1.9),
            Some(NativeScrollDelta::WheelTicks(point(1., -2.)))
        );
        {
            let nested = ScrollPacket::pixels(point(0., 40.), 1.9).unwrap();
            let _inner = nested.bind(window, 1.9);
            assert!(NativeScrollDelta::get(window, packet.delta, 1.9).is_none());
        }
        assert_eq!(
            NativeScrollDelta::get(window, packet.delta, 1.9),
            Some(NativeScrollDelta::WheelTicks(point(1., -2.)))
        );
        assert!(ScrollPacket::pixels(point(1., 1.), 0.).is_none());
        assert!(ScrollPacket::wheel(point(f64::NAN, 0.), 1).is_none());
        assert!(ScrollPacket::pixels(point(f64::MAX, 0.), 1.).is_none());
    }

    #[test]
    fn unknown_or_nonpositive_step_has_no_unscaled_wheel_detents() {
        let window = WindowId::from(1);
        let outer = ScrollPacket::wheel(point(45., -90.), 3).unwrap();
        let _outer = outer.bind(window, 1.9);
        for step in [i32::MIN, -1, 0] {
            let packet = ScrollPacket::wheel(point(45., -90.), step).unwrap();
            {
                let _scope = packet.bind(window, 1.9);
                assert!(NativeScrollDelta::get(window, packet.delta, 1.9).is_none());
            }
            assert_eq!(
                NativeScrollDelta::get(window, outer.delta, 1.9),
                Some(NativeScrollDelta::WheelTicks(point(1., -2.)))
            );
        }
    }
}
