//! OpenHarmony platform support for GPUI.
//!
//! The platform backend lives outside the Zed workspace and is injected with
//! [`gpui::Application::with_platform`], matching the architecture used by
//! `gpui-mobile` for Android and iOS.

mod ohos;

pub use gpui::*;
pub use ohos::{PlatformOptions, current_platform, try_current_platform};

pub use ohos::{
    FontCatalog, FontDesignCoordinate, FontFaceData, GridLayoutError, GridLayoutRequest,
    GridTextCell, PlatformServices, SyntheticFontStyle,
};

pub use ohos::{ImagePolicy, ImageRegistrationError, PremultipliedImage, RasterImage};
