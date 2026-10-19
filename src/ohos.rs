mod capture_pixels;
mod clipboard;
mod credentials;
mod cursor_visibility;
mod dispatcher;
mod display;
mod file_access;
mod frame_delivery;
mod frame_demand;
mod keyboard;
mod launch;
#[cfg(feature = "native-vsync")]
mod native_frames;
mod path_targets;
mod platform;
mod presentation;
mod quad_pipeline;
mod render_cache;
mod screen_capture;
mod task_queue;
mod text_system;
mod touch_scroll;
mod viewport;
mod wgpu_atlas;
mod wgpu_context;
mod wgpu_renderer;
mod window;
mod window_manipulation;
mod window_state;
mod worker_state;
mod workers;

use openharmony_ability::OpenHarmonyApp;

#[derive(Clone, Copy)]
pub struct PlatformOptions {
    /// Disable native menus when the application renders and dispatches its own menus.
    pub native_menus: bool,
}

impl Default for PlatformOptions {
    fn default() -> Self {
        Self { native_menus: true }
    }
}

pub fn try_current_platform(
    app: OpenHarmonyApp,
    options: PlatformOptions,
) -> anyhow::Result<std::rc::Rc<dyn gpui::Platform>> {
    Ok(std::rc::Rc::new(platform::OhosPlatform::new(app, options)?))
}

pub fn current_platform(app: OpenHarmonyApp, _headless: bool) -> std::rc::Rc<dyn gpui::Platform> {
    std::rc::Rc::new(
        platform::OhosPlatform::new(app, PlatformOptions::default())
            .inspect_err(|err| {
                log::error!("Failed to initialize OHOS platform: {}", err);
            })
            .unwrap_or_else(|_| panic!("Failed to initialize OHOS platform")),
    )
}
