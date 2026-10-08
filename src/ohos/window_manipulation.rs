use super::window_state::NativeWindowState;
use crate::ResizeEdge;

#[derive(Clone, Copy)]
pub(super) struct WindowManipulation {
    pub initial: NativeWindowState,
    pub pointer: (i64, i64),
    pub edge: Option<ResizeEdge>,
}
impl WindowManipulation {
    pub fn update(self, pointer: (i64, i64), minimum: (i64, i64)) -> NativeWindowState {
        let dx = pointer.0.saturating_sub(self.pointer.0);
        let dy = pointer.1.saturating_sub(self.pointer.1);
        let mut result = self.initial;
        match self.edge {
            None => {
                result.left = result.left.saturating_add(dx);
                result.top = result.top.saturating_add(dy);
            }
            Some(edge) => {
                if matches!(
                    edge,
                    ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
                ) {
                    result.width =
                        (self.initial.width - dx).clamp(minimum.0.max(1), i64::from(i32::MAX));
                    result.left = self.initial.left + self.initial.width - result.width;
                }
                if matches!(
                    edge,
                    ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
                ) {
                    result.width =
                        (self.initial.width + dx).clamp(minimum.0.max(1), i64::from(i32::MAX));
                }
                if matches!(
                    edge,
                    ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
                ) {
                    result.height =
                        (self.initial.height - dy).clamp(minimum.1.max(1), i64::from(i32::MAX));
                    result.top = self.initial.top + self.initial.height - result.height;
                }
                if matches!(
                    edge,
                    ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
                ) {
                    result.height =
                        (self.initial.height + dy).clamp(minimum.1.max(1), i64::from(i32::MAX));
                }
            }
        }
        result.left = result.left.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        result.top = result.top.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resize_keeps_opposite_edges_fixed_at_minimum() {
        let initial = NativeWindowState {
            left: 100,
            top: 200,
            width: 800,
            height: 600,
            maximized: false,
            fullscreen: false,
        };
        let session = WindowManipulation {
            initial,
            pointer: (110, 210),
            edge: Some(ResizeEdge::TopLeft),
        };
        let result = session.update((1000, 1000), (300, 200));
        assert_eq!(
            (result.left, result.top, result.width, result.height),
            (600, 600, 300, 200)
        );
        assert_eq!(result.left + result.width, initial.left + initial.width);
        assert_eq!(result.top + result.height, initial.top + initial.height);
        let moved = WindowManipulation {
            edge: None,
            ..session
        }
        .update((130, 240), (1, 1));
        assert_eq!(
            (moved.left, moved.top, moved.width, moved.height),
            (120, 230, 800, 600)
        );
    }
}
