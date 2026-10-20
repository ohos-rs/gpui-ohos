/// Isolated font database for terminal-to-platform integration tests.
pub fn fonts(inputs: Vec<Cow<'static, [u8]>>) -> (Arc<dyn PlatformTextSystem>, FontCatalog) {
    let system = Arc::new(OhosTextSystem(RwLock::new(OhosTextSystemState::new(
        FontSystem::new_with_locale_and_db("en-US".into(), cosmic_text::fontdb::Database::new()),
        true,
    ))));
    system.add_fonts(inputs).expect("valid font fixtures");
    let catalog = FontCatalog::new(system.clone());
    (system, catalog)
}

// Exercise the actual platform shaper on a host without linking OHOS APIs.
include!("../../src/ohos/text_system.rs");

#[cfg(test)]
mod locale_tests {
    use super::*;
    #[test]
    fn catalog_keeps_upstream_locale_preferences() {
        assert_eq!(
            OhosFontFallback::han_fallback("ja-JP"),
            &["Noto Sans CJK JP"]
        );
        assert_eq!(
            OhosFontFallback::han_fallback("ko-KR"),
            &["Noto Sans CJK KR"]
        );
        assert_eq!(
            OhosFontFallback::han_fallback("zh-Hant-TW"),
            &["HarmonyOS Sans TC", "Noto Sans CJK TC"]
        );
        assert_eq!(
            OhosFontFallback::han_fallback("zh-HK"),
            &["Noto Sans CJK HK", "HarmonyOS Sans TC", "Noto Sans CJK TC"]
        );
        assert_eq!(
            OhosFontFallback::han_fallback("zh-Hans-CN"),
            &["HarmonyOS Sans SC", "Noto Sans CJK SC"]
        );
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[path = "../../src/ohos/image_alpha.rs"]
mod image_alpha;
pub use image_alpha::{PremultipliedImage, RasterImage};
