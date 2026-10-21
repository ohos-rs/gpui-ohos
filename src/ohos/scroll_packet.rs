use gpui::{Point, ScrollDelta, TouchPhase, px};

#[cfg(feature = "native-input")]
use super::scroll_input::{NativeScrollDelta, NativeScrollScope};

const WHEEL_DEGREES: f64 = 15.;

pub(super) struct ScrollPacket {
    pub(super) delta: ScrollDelta,
    #[cfg(feature = "native-input")]
    native: Option<NativeScrollDelta>,
}

impl ScrollPacket {
    pub(super) fn axis(raw: Point<f64>, scale: f32, wheel: bool, scroll_step: i32) -> Option<Self> {
        if wheel {
            Self::wheel(raw, scroll_step)
        } else {
            Self::pixels(raw, scale)
        }
    }

    pub(super) fn should_dispatch(&self, phase: TouchPhase) -> bool {
        let nonzero = match self.delta {
            ScrollDelta::Lines(delta) => delta.x != 0. || delta.y != 0.,
            ScrollDelta::Pixels(delta) => delta.x != px(0.) || delta.y != px(0.),
        };
        nonzero || matches!(phase, TouchPhase::Started | TouchPhase::Ended)
    }

    /// Keep the system scroll-step coefficient in ordinary widget line deltas.
    pub(super) fn wheel(degrees: Point<f64>, _scroll_step: i32) -> Option<Self> {
        let lines = degrees.map(|value| value / WHEEL_DEGREES);
        let logical = lines.map(|value| value as f32);
        if !logical.x.is_finite() || !logical.y.is_finite() {
            return None;
        }
        Some(Self {
            delta: ScrollDelta::Lines(logical),
            #[cfg(feature = "native-input")]
            native: (_scroll_step > 0).then(|| {
                NativeScrollDelta::WheelTicks(lines.map(|value| value / f64::from(_scroll_step)))
            }),
        })
    }

    pub(super) fn pixels(device: Point<f64>, scale: f32) -> Option<Self> {
        if !scale.is_finite() || scale <= 0. {
            return None;
        }
        let logical = device.map(|value| px(value as f32 / scale));
        if !logical.x.as_f32().is_finite() || !logical.y.as_f32().is_finite() {
            return None;
        }
        Some(Self {
            delta: ScrollDelta::Pixels(logical),
            #[cfg(feature = "native-input")]
            native: Some(NativeScrollDelta::DevicePixels(device)),
        })
    }

    #[cfg(feature = "native-input")]
    pub(super) fn bind(&self, window: gpui::WindowId, scale: f32) -> NativeScrollScope {
        NativeScrollScope::bind(window, self.delta, scale, self.native)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::point;

    #[test]
    fn axis_conversion_preserves_native_signs_for_wheels_and_touchpads() {
        for raw in [point(45., -90.), point(-45., 90.)] {
            let wheel = ScrollPacket::axis(raw, 1.9, true, 3).unwrap();
            assert!(matches!(wheel.delta, ScrollDelta::Lines(delta)
                    if delta == raw.map(|value| (value / WHEEL_DEGREES) as f32)));
            let touchpad = ScrollPacket::axis(raw, 1.9, false, 0).unwrap();
            assert!(matches!(touchpad.delta, ScrollDelta::Pixels(delta)
                    if delta == raw.map(|value| px(value as f32 / 1.9))));
        }
    }

    #[test]
    fn zero_axis_delta_only_dispatches_started_and_ended() {
        for wheel in [false, true] {
            let packet = ScrollPacket::axis(point(0., -0.), 1.9, wheel, 3).unwrap();
            for phase in [TouchPhase::Started, TouchPhase::Ended] {
                assert!(packet.should_dispatch(phase));
            }
            for phase in [TouchPhase::Moved, TouchPhase::Cancelled] {
                assert!(!packet.should_dispatch(phase));
            }
            for raw in [point(0., 0.5), point(-0.5, 0.)] {
                let packet = ScrollPacket::axis(raw, 1.9, wheel, 3).unwrap();
                assert!(packet.should_dispatch(TouchPhase::Moved));
            }
        }
    }

    #[test]
    fn unknown_scroll_step_keeps_generic_wheel_lines() {
        for step in [i32::MIN, -1, 0, 1, 3, i32::MAX] {
            let packet = ScrollPacket::wheel(point(45., -90.), step).unwrap();
            assert!(matches!(packet.delta, ScrollDelta::Lines(delta) if delta == point(3., -6.)));
        }
    }

    #[test]
    fn rejects_nonfinite_deltas_and_invalid_scales() {
        for scale in [0., -1., f32::NAN, f32::INFINITY] {
            assert!(ScrollPacket::pixels(point(1., 1.), scale).is_none());
        }
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            assert!(ScrollPacket::wheel(point(invalid, 0.), 1).is_none());
            assert!(ScrollPacket::pixels(point(0., invalid), 1.).is_none());
        }
    }
}
