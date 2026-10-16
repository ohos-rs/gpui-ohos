use crate::{Pixels, Size};
use std::cell::Cell;

type ResizeCallback = Box<dyn FnMut(Size<Pixels>, f32)>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    bounds_size: Size<Pixels>,
    effective_size: Size<Pixels>,
    scale: f32,
}

/// Tracks the viewport delivered to GPUI, independently of renderer geometry.
#[derive(Default)]
pub(crate) struct ViewportPublisher {
    published: Cell<Option<Viewport>>,
}

impl ViewportPublisher {
    pub(crate) fn reset(&self) {
        self.published.set(None);
    }

    pub(crate) fn publish(
        &self,
        bounds_size: Size<Pixels>,
        size: Size<Pixels>,
        scale: f32,
        callback: &mut Option<ResizeCallback>,
    ) {
        let viewport = Viewport {
            bounds_size,
            effective_size: size,
            scale,
        };
        if self.published.get() == Some(viewport) {
            return;
        }
        if let Some(callback) = callback {
            self.published.set(Some(viewport));
            callback(size, scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size};
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn publishes_recreated_surface_keyboard_and_scale_once_each() {
        let publisher = ViewportPublisher::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let result = events.clone();
        let mut callback: Option<ResizeCallback> = Some(Box::new(move |size, scale| {
            result.borrow_mut().push((size, scale))
        }));
        let initial = size(px(400.0), px(800.0));
        let recreated = size(px(600.0), px(800.0));
        let keyboard = size(px(600.0), px(500.0));
        publisher.publish(initial, initial, 2.0, &mut callback);
        // Renderer has already adopted the recreated surface's geometry.
        publisher.publish(recreated, recreated, 2.0, &mut callback);
        publisher.publish(recreated, recreated, 2.0, &mut callback);
        publisher.publish(recreated, keyboard, 2.0, &mut callback);
        publisher.publish(recreated, keyboard, 3.0, &mut callback);
        assert_eq!(
            *events.borrow(),
            [
                (initial, 2.0),
                (recreated, 2.0),
                (keyboard, 2.0),
                (keyboard, 3.0)
            ]
        );
    }

    #[test]
    fn publishes_full_bounds_changes_while_keyboard_keeps_effective_size_unchanged() {
        let publisher = ViewportPublisher::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let result = events.clone();
        let mut callback: Option<ResizeCallback> = Some(Box::new(move |size, scale| {
            result.borrow_mut().push((size, scale))
        }));
        let initial = size(px(600.0), px(800.0));
        let resized = size(px(600.0), px(700.0));
        let effective = size(px(600.0), px(500.0));
        publisher.publish(initial, effective, 2.0, &mut callback);
        publisher.publish(resized, effective, 2.0, &mut callback);
        publisher.publish(resized, effective, 2.0, &mut callback);
        assert_eq!(*events.borrow(), [(effective, 2.0), (effective, 2.0)]);
    }

    #[test]
    fn missing_callback_does_not_mark_viewport_as_published() {
        let publisher = ViewportPublisher::default();
        let size = size(px(400.0), px(800.0));
        publisher.publish(size, size, 2.0, &mut None);
        assert_eq!(publisher.published.get(), None);
        let mut callback: Option<ResizeCallback> = Some(Box::new(|_, _| {}));
        publisher.publish(size, size, 2.0, &mut callback);
        assert_eq!(
            publisher.published.get(),
            Some(Viewport {
                bounds_size: size,
                effective_size: size,
                scale: 2.0,
            })
        );
        publisher.reset();
        assert_eq!(publisher.published.get(), None);
    }
}
