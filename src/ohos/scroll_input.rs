//! Native wheel ticks/device pixels retained during synchronous GPUI dispatch.
use gpui::{Pixels, Point, ScrollDelta, WindowId, point, px};
use std::{cell::Cell, marker::PhantomData, rc::Rc};

const WHEEL_DEGREES: f64 = 15.;

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
    native: NativeScrollDelta,
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

pub(super) struct ScrollPacket {
    pub(super) delta: ScrollDelta,
    native: NativeScrollDelta,
}

impl ScrollPacket {
    /// ArkUI mouse axis values include the system scroll-step coefficient.
    /// Generic GPUI widgets retain it; terminal clients can recover detents
    /// and apply their own explicit precision/discrete configuration.
    pub(super) fn wheel(degrees: Point<f64>, scroll_step: i32) -> Option<Self> {
        let lines = degrees.map(|value| -value / WHEEL_DEGREES);
        let logical = lines.map(|value| value as f32);
        if !logical.x.is_finite() || !logical.y.is_finite() {
            return None;
        }
        Some(Self {
            delta: ScrollDelta::Lines(logical),
            native: NativeScrollDelta::WheelTicks(
                lines.map(|value| value / f64::from(scroll_step.max(1))),
            ),
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
            native: NativeScrollDelta::DevicePixels(device),
        })
    }

    pub(super) fn bind(&self, window: WindowId, scale: f32) -> NativeScrollScope {
        let previous = CURRENT.with(|current| {
            current.replace(Some(ScrollContext {
                window,
                logical: self.delta,
                scale,
                native: self.native,
            }))
        });
        NativeScrollScope {
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
            Some(match context.native {
                Self::WheelTicks(value) => Self::WheelTicks(project(value)),
                Self::DevicePixels(value) => Self::DevicePixels(project(value)),
            })
        })
    }
}
