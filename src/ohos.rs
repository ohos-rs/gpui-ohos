mod capture_pixels;
mod clipboard;
mod credentials;
mod dispatcher;
mod display;
mod frame_delivery;
mod frame_demand;
mod keyboard;
#[cfg(feature = "native-vsync")]
mod native_frames;
#[cfg(feature = "native-vsync")]
mod native_refresh;
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

#[cfg(feature = "native-vsync")]
pub use native_frames::configure_native_refresh;
#[cfg(feature = "native-vsync")]
pub use native_refresh::NativeRefreshError;
use openharmony_ability::OpenHarmonyApp;

pub fn current_platform(app: OpenHarmonyApp, _headless: bool) -> std::rc::Rc<dyn gpui::Platform> {
    std::rc::Rc::new(
        platform::OhosPlatform::new(app)
            .inspect_err(|err| {
                log::error!("Failed to initialize OHOS platform: {}", err);
            })
            .unwrap_or_else(|_| panic!("Failed to initialize OHOS platform")),
    )
}
