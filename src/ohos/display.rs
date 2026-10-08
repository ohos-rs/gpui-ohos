use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

use super::platform::system_state_plugin::DisplaySnapshot;
use openharmony_ability::OpenHarmonyApp;

use crate::{Bounds, DisplayId, Pixels, PlatformDisplay, Result, point, px, size};

#[derive(Clone)]
pub(crate) struct OhosDisplay {
    app: OpenHarmonyApp,
    snapshot: Arc<Mutex<Option<DisplaySnapshot>>>,
    available_area: Arc<Mutex<Option<Bounds<Pixels>>>>,
}

impl OhosDisplay {
    pub(crate) fn new(app: OpenHarmonyApp) -> Self {
        Self {
            app,
            snapshot: Arc::new(Mutex::new(None)),
            available_area: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn from_snapshot(app: OpenHarmonyApp, snapshot: DisplaySnapshot) -> Self {
        let display = Self::new(app);
        display.update_snapshot(snapshot);
        display
    }

    pub(crate) fn id_raw(&self) -> i64 {
        self.snapshot
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |snapshot| snapshot.id)
    }

    pub(crate) fn update_snapshot(&self, snapshot: DisplaySnapshot) {
        if snapshot.id < 0
            || snapshot.width <= 0
            || snapshot.height <= 0
            || !snapshot.density_pixels.is_finite()
            || snapshot.density_pixels <= 0.0
        {
            log::warn!("Ignoring invalid OHOS display snapshot: {snapshot:?}");
            return;
        }
        let area = snapshot.available_area.clone();
        *self.snapshot.lock().unwrap() = Some(snapshot);
        *self.available_area.lock().unwrap() = None;
        if let Some(area) = area {
            self.set_available_area(area.left, area.top, area.width, area.height);
        }
    }

    pub(crate) fn dimensions_and_scale(&self) -> (i32, i32, f32) {
        if let Some(snapshot) = self.snapshot.lock().unwrap().as_ref() {
            return (
                snapshot.width,
                snapshot.height,
                snapshot.density_pixels as f32,
            );
        }
        let (width, height) = self.app.display_size();
        (
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
            self.app.scale().max(f32::EPSILON),
        )
    }

    pub(crate) fn scale_factor(&self) -> f32 {
        self.dimensions_and_scale().2
    }

    pub(crate) fn set_available_area(&self, left: i32, top: i32, width: i32, height: i32) {
        let (display_width, display_height, scale) = self.dimensions_and_scale();
        if left < 0
            || top < 0
            || width <= 0
            || height <= 0
            || left as i64 + width as i64 > display_width as i64
            || top as i64 + height as i64 > display_height as i64
        {
            log::warn!("Ignoring invalid OHOS available area: {left},{top} {width}x{height}");
            return;
        }
        let bounds = Bounds::new(
            point(px(left as f32 / scale), px(top as f32 / scale)),
            size(px(width as f32 / scale), px(height as f32 / scale)),
        );
        *self.available_area.lock().unwrap() = Some(bounds);
        log::info!("OHOS available display area: {bounds:?}");
    }
}

impl Debug for OhosDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OhosDisplay")
            .field("id", &self.id_raw())
            .finish()
    }
}

impl PlatformDisplay for OhosDisplay {
    fn id(&self) -> DisplayId {
        DisplayId::new(self.id_raw() as u64)
    }

    fn uuid(&self) -> Result<Uuid> {
        Ok(Uuid::from_u128(self.id_raw() as u128 + 1))
    }

    fn bounds(&self) -> Bounds<Pixels> {
        let (width, height, scale) = self.dimensions_and_scale();
        if width > 0 && height > 0 {
            Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(width as f32 / scale), px(height as f32 / scale)),
            )
        } else {
            let content_rect = self.app.content_rect();
            Bounds::new(
                point(px(0.0), px(0.0)),
                size(
                    px(content_rect.width.max(1) as f32 / scale),
                    px(content_rect.height.max(1) as f32 / scale),
                ),
            )
        }
    }

    fn visible_bounds(&self) -> Bounds<Pixels> {
        self.available_area
            .lock()
            .unwrap()
            .unwrap_or_else(|| self.bounds())
    }
}
