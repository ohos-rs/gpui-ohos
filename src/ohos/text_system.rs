use anyhow::{Context as _, Result};
use cosmic_text::{
    Attrs, AttrsList, CacheKey, Fallback, Family, Font as CosmicTextFont,
    FontFeatures as CosmicFontFeatures, FontSystem, PlatformFallback, ShapeBuffer, ShapeLine,
    SwashCache, SwashContent,
};
use gpui::{
    Bounds, DevicePixels, Font, FontFeatures, FontId, FontMetrics, FontRun, FontStyle, GlyphId,
    LineLayout, Pixels, PlatformTextSystem, RenderGlyphParams, SUBPIXEL_VARIANTS_X,
    SUBPIXEL_VARIANTS_Y, ShapedGlyph, ShapedRun, Size, TextRenderingMode, font_name_with_fallbacks,
    point, size,
};
use itertools::Itertools;
use log::warn;
use parking_lot::RwLock;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use smallvec::SmallVec;
use std::{borrow::Cow, path::Path, sync::Arc};
use unicode_script::Script;

struct OhosFontFallback;

impl OhosFontFallback {
    fn han_fallback(locale: &str) -> &'static [&'static str] {
        let mut components = locale.split(['-', '_']);
        let language = components.next().unwrap_or_default();
        if language.eq_ignore_ascii_case("ja") {
            return &["Noto Sans CJK JP"];
        }
        if language.eq_ignore_ascii_case("ko") {
            return &["Noto Sans CJK KR"];
        }
        let mut traditional = false;
        for component in components {
            if component.eq_ignore_ascii_case("HK") || component.eq_ignore_ascii_case("MO") {
                return &["Noto Sans CJK HK", "HarmonyOS Sans TC", "Noto Sans CJK TC"];
            }
            traditional |=
                component.eq_ignore_ascii_case("Hant") || component.eq_ignore_ascii_case("TW");
        }
        if traditional {
            &["HarmonyOS Sans TC", "Noto Sans CJK TC"]
        } else {
            &["HarmonyOS Sans SC", "Noto Sans CJK SC"]
        }
    }
}

impl Fallback for OhosFontFallback {
    fn common_fallback(&self) -> &[&'static str] {
        // Match the system's installed families before scanning every face.
        // Its commonly used HarmonyOS fonts also avoid cold Noto CJK page reads.
        &[
            "HMOS Color Emoji",
            "Noto Color Emoji",
            "HarmonyOS Sans",
            "Noto Sans",
            "Noto Sans Mono",
            "Noto Sans Symbols",
            "Noto Sans Symbols 2",
            "Noto Sans Symbols2",
        ]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        PlatformFallback.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        match script {
            Script::Han => Self::han_fallback(locale),
            Script::Bopomofo => &["HarmonyOS Sans TC", "Noto Sans CJK TC"],
            Script::Hiragana | Script::Katakana => &["Noto Sans CJK JP"],
            Script::Hangul => &["Noto Sans CJK KR"],
            Script::Arabic => &[
                "HarmonyOS Sans Naskh Arabic UI",
                "Noto Naskh Arabic UI",
                "Noto Sans Arabic",
            ],
            _ => PlatformFallback.script_fallback(script, locale),
        }
    }
}

use unicode_segmentation::UnicodeSegmentation;

#[path = "text_system/grid.rs"]
mod grid;
#[path = "text_system/named_instance.rs"]
mod named_instance;
#[cfg(test)]
#[path = "text_system/named_instances_tests.rs"]
mod named_instances_tests;
pub use grid::{GridLayoutError, GridLayoutRequest, GridTextCell};

pub(crate) struct OhosTextSystem(RwLock<OhosTextSystemState>);

/// A stable handle to the same font database used by GPUI shaping and painting.
/// Resolved descriptors identify exact faces and can be used in ordinary TextRuns.
#[derive(Clone)]
pub struct FontCatalog(Arc<OhosTextSystem>);

/// An effective design coordinate read from a font engine, in signed 16.16.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FontDesignCoordinate {
    pub tag: [u8; 4],
    pub value: i32,
}

/// Keeps the exact registered font bytes alive while an embedding font engine
/// reads them. The design weight matches this face's shaping/raster coordinate.
pub struct FontFaceData {
    font: Arc<CosmicTextFont>,
    index: u32,
    weight: u16,
    synthetic: SyntheticFontStyle,
    design_coordinates: Arc<[FontDesignCoordinate]>,
}

impl AsRef<[u8]> for FontFaceData {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}

impl FontFaceData {
    pub fn bytes(&self) -> &[u8] {
        self.font.data()
    }
    pub fn index(&self) -> u32 {
        self.index
    }
    pub fn weight(&self) -> u16 {
        self.weight
    }
    pub fn synthetic(&self) -> SyntheticFontStyle {
        self.synthetic
    }
    pub fn design_coordinates(&self) -> &[FontDesignCoordinate] {
        &self.design_coordinates
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SyntheticFontStyle {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RegisteredFace {
    id: cosmic_text::fontdb::ID,
    weight: cosmic_text::Weight,
    features: FontFeatures,
    synthetic: SyntheticFontStyle,
    design_coordinates: Arc<[FontDesignCoordinate]>,
}

struct OhosTextSystemState {
    swash_cache: SwashCache,
    font_system: FontSystem,
    scratch: ShapeBuffer,
    loaded_fonts: Vec<LoadedFont>,
    font_cache: HashMap<Font, FontId>,
    system_fonts_loaded: bool,
    registered: HashMap<Font, FontId>,
    registered_faces: HashMap<RegisteredFace, Font>,
    synthetic_scaler: swash::scale::ScaleContext,
    synthetic_images:
        HashMap<(CacheKey, SyntheticFontStyle, FontId), Option<cosmic_text::SwashImage>>,
    grid_shaper: grid::GridShaper,
}

struct LoadedFont {
    font: Arc<CosmicTextFont>,
    font_weight: cosmic_text::Weight,
    features: CosmicFontFeatures,
    has_color_glyphs: bool,
    fallbacks: Arc<[FontId]>,
    coordinates: SmallVec<[i16; 4]>,
    synthetic: SyntheticFontStyle,
    design_coordinates: Arc<[FontDesignCoordinate]>,
    variable_shaper: Option<VariableShaper>,
}

struct VariableShaper {
    index: u32,
    data: harfrust::ShaperData,
    instance: harfrust::ShaperInstance,
}
impl VariableShaper {
    fn new(
        font: &CosmicTextFont,
        index: u32,
        coordinates: &[FontDesignCoordinate],
    ) -> Option<Self> {
        let face = harfrust::FontRef::from_index(font.data(), index).ok()?;
        Some(Self {
            index,
            data: harfrust::ShaperData::new(&face),
            instance: harfrust::ShaperInstance::from_variations(
                &face,
                coordinates
                    .iter()
                    .map(|v| (harfrust::Tag::new(&v.tag), v.value as f32 / 65536.)),
            ),
        })
    }
    fn shaper<'a>(&'a self, font: &'a CosmicTextFont) -> harfrust::Shaper<'a> {
        let face = harfrust::FontRef::from_index(font.data(), self.index)
            .expect("validated variable face");
        self.data
            .shaper(&face)
            .instance(Some(&self.instance))
            .build()
    }
}

impl LoadedFont {
    fn new(
        font: Arc<CosmicTextFont>,
        weight: cosmic_text::Weight,
        features: CosmicFontFeatures,
    ) -> Self {
        let coordinates = font
            .as_swash()
            .variations()
            .normalized_coords([("wght", f32::from(weight.0))])
            .collect();
        Self {
            has_color_glyphs: has_color_glyphs(&font),
            font,
            font_weight: weight,
            features,
            fallbacks: Arc::from([]),
            coordinates,
            synthetic: SyntheticFontStyle::default(),
            design_coordinates: Arc::from([]),
            variable_shaper: None,
        }
    }
}

impl OhosTextSystem {
    pub(crate) fn new(locale: String) -> Self {
        let locale = if locale.is_empty() {
            "en-US".to_owned()
        } else {
            locale
        };
        let mut database = cosmic_text::fontdb::Database::new();
        database.set_sans_serif_family("HarmonyOS Sans");
        database.set_serif_family("Noto Serif");
        database.set_monospace_family("Noto Sans Mono");
        let font_system =
            FontSystem::new_with_locale_and_db_and_fallback(locale, database, OhosFontFallback);

        Self(RwLock::new(OhosTextSystemState::new(font_system, false)))
    }
}

impl OhosTextSystemState {
    fn new(font_system: FontSystem, system_fonts_loaded: bool) -> Self {
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            scratch: ShapeBuffer::default(),
            loaded_fonts: Vec::new(),
            font_cache: HashMap::default(),
            system_fonts_loaded,
            registered: HashMap::default(),
            registered_faces: HashMap::default(),
            synthetic_scaler: swash::scale::ScaleContext::new(),
            synthetic_images: HashMap::default(),
            grid_shaper: grid::GridShaper::default(),
        }
    }
}

impl FontCatalog {
    pub(crate) fn new(system: Arc<OhosTextSystem>) -> Self {
        Self(system)
    }

    pub fn face_data(&self, font: &Font) -> Option<FontFaceData> {
        let mut state = self.0.0.write();
        let id = state.resolve_font(font).ok()?;
        self.face_data_from_state(&state, id)
    }

    /// A shaped run's exact face, including its synthetic style and TTC index.
    pub fn face_data_for_id(&self, id: FontId) -> Option<FontFaceData> {
        let state = self.0.0.read();
        self.face_data_from_state(&state, id)
    }

    fn face_data_from_state(
        &self,
        state: &OhosTextSystemState,
        id: FontId,
    ) -> Option<FontFaceData> {
        if id.0 >= state.loaded_fonts.len() {
            return None;
        }
        let loaded = state.loaded_font(id);
        let index = state.font_system.db().face(loaded.font.id())?.index;
        Some(FontFaceData {
            font: loaded.font.clone(),
            index,
            weight: loaded.font_weight.0,
            synthetic: loaded.synthetic,
            design_coordinates: loaded.design_coordinates.clone(),
        })
    }

    /// Resolve only the requested family and style, without system substitution.
    /// A named style overrides the weight/slant category, as advertised by the font.
    pub fn resolve(&self, requested: &Font, named_style: Option<&str>) -> Option<Font> {
        let mut state = self.0.0.write();
        state.ensure_system_fonts_loaded();
        let family = OhosTextSystemState::normalize_family_name(&requested.family);
        let mut candidates = state
            .font_system
            .db()
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|name| OhosTextSystemState::normalize_family_name(&name.0) == family)
                    || face.post_script_name == requested.family.as_ref()
            })
            .map(|face| {
                (
                    face.id,
                    face.post_script_name.clone(),
                    face.weight,
                    face.style,
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| a.1.cmp(&b.1));
        let mut matches = Vec::new();
        for (id, _, weight, style) in candidates {
            let Some(font) = state.font_system.get_font(id, weight) else {
                continue;
            };
            let face = font.as_swash();
            let mut named_instance =
                named_style.and_then(|name| named_instance::NamedInstance::find(face, name));
            if let Some(name) = named_style {
                if named_instance.is_none()
                    && !face.localized_strings().any(|value| {
                        matches!(
                            value.id(),
                            swash::StringId::SubFamily
                                | swash::StringId::TypographicSubFamily
                                | swash::StringId::WwsSubFamily
                        ) && value.to_string().eq_ignore_ascii_case(name)
                    })
                {
                    continue;
                }
                if named_instance.is_none() {
                    named_instance = named_instance::NamedInstance::defaults(face);
                }
            } else {
                let italic = matches!(requested.style, FontStyle::Italic | FontStyle::Oblique);
                if italic
                    != matches!(
                        style,
                        cosmic_text::Style::Italic | cosmic_text::Style::Oblique
                    )
                {
                    continue;
                }
                let variable_bold = face
                    .variations()
                    .find_by_tag(swash::Tag::from_be_bytes(*b"wght"))
                    .is_some_and(|axis| axis.max_value() >= requested.weight.0);
                if requested.weight.0 >= 600. && weight.0 < 600 && !variable_bold {
                    continue;
                }
            }
            let named_weight = named_instance.as_ref().and_then(|instance| {
                instance
                    .coordinates
                    .iter()
                    .find(|axis| axis.tag == *b"wght")
                    .map(|axis| {
                        cosmic_text::Weight(
                            (f64::from(axis.value) / 65536.).round().clamp(1., 1000.) as u16,
                        )
                    })
            });
            let effective_weight = if let Some(weight) = named_weight {
                weight
            } else if named_style.is_none()
                && face
                    .variations()
                    .find_by_tag(swash::Tag::from_be_bytes(*b"wght"))
                    .is_some()
            {
                cosmic_text::Weight(requested.weight.0.round().clamp(1., 1000.) as u16)
            } else {
                weight
            };
            let distance = (f32::from(weight.0) - requested.weight.0).abs();
            matches.push((
                distance,
                id,
                effective_weight,
                named_instance.map_or_else(|| Arc::from([]), |instance| instance.coordinates),
            ));
        }
        matches.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (_, id, weight, design_coordinates) = matches.into_iter().next()?;
        state.register_face(RegisteredFace {
            id,
            weight,
            features: requested.features.clone(),
            synthetic: SyntheticFontStyle::default(),
            design_coordinates,
        })
    }

    pub fn synthesize(&self, font: &Font, style: SyntheticFontStyle) -> Option<Font> {
        let mut state = self.0.0.write();
        let id = *state.registered.get(font)?;
        let loaded = state.loaded_font(id);
        let key = RegisteredFace {
            id: loaded.font.id(),
            weight: loaded.font_weight,
            features: font.features.clone(),
            synthetic: SyntheticFontStyle {
                bold: style.bold || loaded.synthetic.bold,
                italic: style.italic || loaded.synthetic.italic,
            },
            design_coordinates: loaded.design_coordinates.clone(),
        };
        state.register_face(key)
    }

    /// Register an immutable fixed-grid face with the engine's effective axes.
    /// The resulting FontId fixes axes, features and synthetic style together.
    pub fn with_design_coordinates(
        &self,
        font: &Font,
        coordinates: &[FontDesignCoordinate],
    ) -> Option<Font> {
        if coordinates.is_empty() {
            return Some(font.clone());
        }
        let mut state = self.0.0.write();
        let id = *state.registered.get(font)?;
        let loaded = state.loaded_font(id);
        let face = loaded.font.as_swash();
        if coordinates.len() > 32
            || coordinates.len() != face.variations().count()
            || face
                .variations()
                .zip(coordinates)
                .any(|(axis, value)| axis.tag() != u32::from_be_bytes(value.tag))
        {
            return None;
        }
        let key = RegisteredFace {
            id: loaded.font.id(),
            weight: loaded.font_weight,
            features: font.features.clone(),
            synthetic: loaded.synthetic,
            design_coordinates: Arc::from(coordinates),
        };
        state.register_face(key)
    }

    /// `emoji` requests color/text presentation; None accepts either.
    pub fn supports(&self, font: &Font, cluster: &str, emoji: Option<bool>) -> bool {
        let mut state = self.0.0.write();
        let Some(id) = state.registered.get(font).copied() else {
            return false;
        };
        let loaded = state.loaded_font(id);
        let (font, weight) = (loaded.font.clone(), loaded.font_weight);
        state.font_supports(&font, weight, cluster, emoji)
    }

    /// Native fallback discovery always requests a regular face. It never
    /// synthesizes an unrelated style just to satisfy one missing codepoint.
    pub fn discover(&self, cluster: &str, emoji: bool, features: &FontFeatures) -> Option<Font> {
        let mut state = self.0.0.write();
        state.ensure_system_fonts_loaded();
        let mut candidates = state
            .font_system
            .db()
            .faces()
            .map(|face| {
                (
                    face.id,
                    face.weight,
                    face.style,
                    face.post_script_name.clone(),
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| {
            let rank = |weight: cosmic_text::Weight, style| {
                (style != cosmic_text::Style::Normal, weight.0.abs_diff(400))
            };
            rank(a.1, a.2)
                .cmp(&rank(b.1, b.2))
                .then_with(|| a.3.cmp(&b.3))
        });
        for (id, weight, _, _) in candidates {
            let Some(font) = state.font_system.get_font(id, weight) else {
                continue;
            };
            if state.font_supports(&font, weight, cluster, Some(emoji)) {
                return state.register_face(RegisteredFace {
                    id,
                    weight,
                    features: features.clone(),
                    synthetic: SyntheticFontStyle::default(),
                    design_coordinates: Arc::from([]),
                });
            }
        }
        None
    }

    pub fn has_text(&self, font: &Font) -> bool {
        let state = self.0.0.read();
        state.registered.get(font).is_some_and(|id| {
            let loaded = state.loaded_font(*id);
            !loaded.has_color_glyphs || loaded.font.as_swash().charmap().map('A') != 0
        })
    }
}

impl OhosTextSystemState {
    fn font_supports(
        &mut self,
        font: &CosmicTextFont,
        weight: cosmic_text::Weight,
        cluster: &str,
        emoji: Option<bool>,
    ) -> bool {
        let map = font.as_swash().charmap();
        let visible = |ch: char| {
            !matches!(ch as u32, 0x200c..=0x200d | 0xfe00..=0xfe0f | 0xe0100..=0xe01ef | 0xe0020..=0xe007f)
                && !ch.is_control()
        };
        if !font_covers(font, cluster) {
            return false;
        }
        let Some(emoji) = emoji else {
            return true;
        };
        let Some(base) = cluster.chars().find(|ch| visible(*ch)) else {
            return true;
        };
        let color = if has_color_glyphs(font) {
            let key = CacheKey::new(
                font.id(),
                map.map(base),
                32.,
                (0., 0.),
                weight,
                cosmic_text::CacheKeyFlags::empty(),
            )
            .0;
            self.swash_cache
                .get_image(&mut self.font_system, key)
                .as_ref()
                .is_some_and(|image| image.content == SwashContent::Color)
        } else {
            false
        };
        color == emoji
    }

    fn register_face(&mut self, key: RegisteredFace) -> Option<Font> {
        if let Some(font) = self.registered_faces.get(&key) {
            return Some(font.clone());
        }
        let face = self.font_system.get_font(key.id, key.weight)?;
        let mut loaded = LoadedFont::new(
            face,
            key.weight,
            cosmic_font_features_from(&key.features).ok()?,
        );
        loaded.synthetic = key.synthetic;
        if !key.design_coordinates.is_empty() {
            let index = self.font_system.db().face(key.id)?.index;
            loaded.coordinates = loaded
                .font
                .as_swash()
                .variations()
                .normalized_coords(
                    key.design_coordinates
                        .iter()
                        .map(|v| (u32::from_be_bytes(v.tag), v.value as f32 / 65536.)),
                )
                .collect();
            loaded.variable_shaper = Some(VariableShaper::new(
                &loaded.font,
                index,
                &key.design_coordinates,
            )?);
            loaded.design_coordinates = key.design_coordinates.clone();
        }
        let id = FontId(self.loaded_fonts.len());
        let mut font = gpui::font(format!(".OhosResolvedFace{}", id.0));
        font.features = key.features.clone();
        self.loaded_fonts.push(loaded);
        self.registered.insert(font.clone(), id);
        self.registered_faces.insert(key, font.clone());
        Some(font)
    }
}

impl Default for OhosTextSystem {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl PlatformTextSystem for OhosTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.0.write().add_fonts(fonts)
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut result = self
            .0
            .read()
            .font_system
            .db()
            .faces()
            .filter_map(|face| face.families.first().map(|family| family.0.clone()))
            .collect_vec();
        result.sort();
        result.dedup();
        result
    }

    fn font_id(&self, font: &Font) -> Result<FontId> {
        self.0.write().resolve_font(font)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        let state = self.0.read();
        let loaded = state.loaded_font(font_id);
        let metrics = loaded.font.as_swash().metrics(&loaded.coordinates);

        FontMetrics {
            units_per_em: metrics.units_per_em as u32,
            ascent: metrics.ascent,
            descent: -metrics.descent,
            line_gap: metrics.leading,
            underline_position: metrics.underline_offset,
            underline_thickness: metrics.stroke_size,
            cap_height: metrics.cap_height,
            x_height: metrics.x_height,
            bounding_box: Bounds {
                origin: point(0.0, 0.0),
                size: size(metrics.max_width, metrics.ascent + metrics.descent),
            },
        }
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let lock = self.0.read();
        let loaded = lock.loaded_font(font_id);
        let glyph_metrics = loaded.font.as_swash().glyph_metrics(&loaded.coordinates);
        let glyph_id = glyph_id.0 as u16;
        Ok(Bounds {
            origin: point(0.0, 0.0),
            size: size(
                glyph_metrics.advance_width(glyph_id),
                glyph_metrics.advance_height(glyph_id),
            ),
        })
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.0.read().advance(font_id, glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.0.read().glyph_for_char(font_id, ch)
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.0.write().raster_bounds(params)
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.0.write().rasterize_glyph(params, raster_bounds)
    }

    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        self.0.write().layout_line(text, font_size, runs)
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
}

impl OhosTextSystemState {
    fn normalize_family_name(name: &str) -> String {
        name.trim()
            .to_lowercase()
            .replace(['_', '-'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn ensure_system_fonts_loaded(&mut self) {
        if self.system_fonts_loaded {
            return;
        }

        let db = self.font_system.db_mut();
        let initial_count = db.faces().count();

        const FONT_DIRS: [&str; 5] = [
            "/system/fonts",
            "/system/font",
            "/vendor/fonts",
            "/system_ext/fonts",
            "/data/fonts",
        ];
        Self::load_font_directories(db, &FONT_DIRS.map(Path::new));

        let final_count = db.faces().count();
        if final_count == initial_count {
            db.load_system_fonts();
        }

        self.system_fonts_loaded = true;
        if db.faces().next().is_none() {
            warn!(
                "OHOS text system: no system fonts found in common directories or fontdb defaults"
            );
        }
    }

    fn load_font_directories(db: &mut cosmic_text::fontdb::Database, roots: &[&Path]) {
        let mut directories = roots
            .iter()
            .map(|path| path.to_path_buf())
            .collect::<Vec<_>>();
        let mut seen = HashSet::default();
        let mut files = std::collections::BTreeSet::new();
        while let Some(directory) = directories.pop() {
            let Ok(directory) = directory.canonicalize() else {
                continue;
            };
            if !seen.insert(directory.clone()) {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let Ok(path) = entry.path().canonicalize() else {
                    continue;
                };
                let Ok(metadata) = path.metadata() else {
                    continue;
                };
                let kind = metadata.file_type();
                if kind.is_dir() {
                    directories.push(path);
                } else if kind.is_file()
                    && path
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|extension| {
                            ["ttf", "ttc", "otf", "otc"]
                                .iter()
                                .any(|supported| extension.eq_ignore_ascii_case(supported))
                        })
                {
                    files.insert(path);
                }
            }
        }
        for path in files {
            if let Err(error) = db.load_font_file(&path) {
                warn!(
                    "OHOS text system: could not load {}: {error}",
                    path.display()
                );
            }
        }
    }

    fn loaded_font(&self, font_id: FontId) -> &LoadedFont {
        &self.loaded_fonts[font_id.0]
    }

    fn add_fonts(&mut self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.font_cache.clear();
        let db = self.font_system.db_mut();
        for bytes in fonts {
            match bytes {
                Cow::Borrowed(embedded_font) => {
                    db.load_font_data(embedded_font.to_vec());
                }
                Cow::Owned(bytes) => {
                    db.load_font_data(bytes);
                }
            }
        }
        Ok(())
    }

    fn resolve_font(&mut self, requested: &Font) -> Result<FontId> {
        if let Some(id) = self.registered.get(requested) {
            return Ok(*id);
        }
        if let Some(id) = self.font_cache.get(requested) {
            return Ok(*id);
        }
        self.ensure_system_fonts_loaded();
        let mut ids = SmallVec::<[FontId; 4]>::new();
        let names = std::iter::once(requested.family.as_ref()).chain(
            requested
                .fallbacks
                .iter()
                .flat_map(|list| list.fallback_list().iter().map(String::as_str)),
        );
        for name in names {
            if let Some(id) = self.load_family(name, requested)? {
                // A repeated/aliased family must not change priority or load
                // the same face multiple times in this stack.
                if ids.iter().any(|previous| {
                    self.loaded_font(*previous).font.id() == self.loaded_font(id).font.id()
                }) {
                    self.loaded_fonts.pop();
                } else {
                    ids.push(id);
                }
            }
        }
        if ids.is_empty() {
            for name in [
                "HarmonyOS Sans",
                "HarmonyOS_Sans",
                "sans-serif",
                "Noto Sans",
            ] {
                if let Some(id) = self.load_family(name, requested)? {
                    ids.push(id);
                    break;
                }
            }
        }
        if ids.is_empty() {
            let first = self
                .font_system
                .db()
                .faces()
                .min_by(|a, b| a.post_script_name.cmp(&b.post_script_name))
                .context("no system fonts available")?
                .id;
            warn!(
                "OHOS text system: no family match for '{}', using a system face",
                requested.family
            );
            ids.push(self.load_face(first, requested)?);
        }
        let primary = ids[0];
        self.loaded_fonts[primary.0].fallbacks = Arc::from(&ids[1..]);
        self.font_cache.insert(requested.clone(), primary);
        Ok(primary)
    }

    fn load_family(&mut self, name: &str, requested: &Font) -> Result<Option<FontId>> {
        let name = font_name_with_fallbacks(name, "HarmonyOS Sans");
        let normalized = Self::normalize_family_name(name);
        let mut candidates = self
            .font_system
            .db()
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|family| normalized == Self::normalize_family_name(&family.0))
            })
            .collect::<SmallVec<[_; 4]>>();
        candidates.sort_by(|a, b| a.post_script_name.cmp(&b.post_script_name));
        if candidates.is_empty() {
            return Ok(None);
        }
        let properties = candidates
            .iter()
            .map(|face| face_info_into_properties(face))
            .collect::<SmallVec<[_; 4]>>();
        let index =
            font_kit::matching::find_best_match(&properties, &font_into_properties(requested))
                .unwrap_or_default();
        let id = candidates[index].id;
        drop(candidates);
        Ok(Some(self.load_face(id, requested)?))
    }

    fn load_face(&mut self, id: cosmic_text::fontdb::ID, requested: &Font) -> Result<FontId> {
        let weight = cosmic_text::Weight(requested.weight.0.round().clamp(1., 1000.) as u16);
        let font = self
            .font_system
            .get_font(id, weight)
            .context("could not load font")?;
        let id = FontId(self.loaded_fonts.len());
        self.loaded_fonts.push(LoadedFont::new(
            font,
            weight,
            cosmic_font_features_from(&requested.features)?,
        ));
        Ok(id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        let loaded = self.loaded_font(font_id);
        let glyph_metrics = loaded.font.as_swash().glyph_metrics(&loaded.coordinates);
        Ok(Size {
            width: glyph_metrics.advance_width(glyph_id.0 as u16),
            height: glyph_metrics.advance_height(glyph_id.0 as u16),
        })
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        let glyph_id = self.loaded_font(font_id).font.as_swash().charmap().map(ch);
        if glyph_id == 0 {
            None
        } else {
            Some(GlyphId(glyph_id.into()))
        }
    }

    fn raster_key(&self, params: &RenderGlyphParams) -> (CacheKey, SyntheticFontStyle) {
        let loaded = self.loaded_font(params.font_id);
        let shift = point(
            params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
            params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
        );
        (
            CacheKey::new(
                loaded.font.id(),
                params.glyph_id.0 as u16,
                (params.font_size * params.scale_factor).into(),
                (shift.x, shift.y.trunc()),
                loaded.font_weight,
                cosmic_text::CacheKeyFlags::empty(),
            )
            .0,
            loaded.synthetic,
        )
    }

    fn glyph_image(
        &mut self,
        key: CacheKey,
        style: SyntheticFontStyle,
        id: FontId,
    ) -> Option<cosmic_text::SwashImage> {
        let loaded = self.loaded_font(id);
        let varied = !loaded.design_coordinates.is_empty();
        if style == SyntheticFontStyle::default() && !varied {
            return self
                .swash_cache
                .get_image(&mut self.font_system, key)
                .clone();
        }
        let identity = if varied { id } else { FontId(0) };
        if let Some(image) = self.synthetic_images.get(&(key, style, identity)) {
            return image.clone();
        }
        let font = loaded.font.clone();
        let coordinates = loaded.coordinates.clone();
        let face = font.as_swash();
        let size = f32::from_bits(key.font_size_bits);
        let metrics = face.metrics(&coordinates);
        let mut scaler = self
            .synthetic_scaler
            .builder(face)
            .size(size)
            .normalized_coords(coordinates.iter().copied())
            .hint(true)
            .build();
        let mut render = swash::scale::Render::new(&[
            swash::scale::Source::ColorOutline(0),
            swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            swash::scale::Source::Outline,
        ]);
        render
            .format(swash::zeno::Format::Alpha)
            .offset(swash::zeno::Vector::new(
                key.x_bin.as_float(),
                key.y_bin.as_float(),
            ));
        if style.italic {
            render.transform(Some(swash::zeno::Transform::skew(
                swash::zeno::Angle::from_degrees(12.),
                swash::zeno::Angle::from_degrees(0.),
            )));
        }
        if style.bold {
            // Ghostty FreeType uses ceil(face_height_26_6 / 32) in 26.6 units.
            let height = (metrics.ascent + metrics.descent + metrics.leading) * size
                / f32::from(metrics.units_per_em);
            let strength = (height.round().max(1.) * 2.).ceil() / 64.;
            render.embolden(strength);
        }
        let image = render.render(&mut scaler, key.glyph_id);
        if self.synthetic_images.len() == 8192 {
            self.synthetic_images.clear();
        }
        self.synthetic_images
            .insert((key, style, identity), image.clone());
        image
    }

    fn raster_bounds(&mut self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let (key, style) = self.raster_key(params);
        let image = self
            .glyph_image(key, style, params.font_id)
            .with_context(|| format!("no image for {params:?}"))?;
        Ok(Bounds {
            origin: point(image.placement.left.into(), (-image.placement.top).into()),
            size: size(image.placement.width.into(), image.placement.height.into()),
        })
    }

    fn rasterize_glyph(
        &mut self,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }
        let (key, style) = self.raster_key(params);
        let mut image = self
            .glyph_image(key, style, params.font_id)
            .with_context(|| format!("no image for {params:?}"))?;
        if params.is_emoji {
            for pixel in image.data.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        Ok((glyph_bounds.size, image.data))
    }

    fn font_id_for_cosmic_id(
        &mut self,
        id: cosmic_text::fontdb::ID,
        weight: cosmic_text::Weight,
        source: FontId,
    ) -> FontId {
        let features = self.loaded_font(source).features.clone();
        if let Some(index) = self.loaded_fonts.iter().position(|loaded_font| {
            loaded_font.font.id() == id
                && loaded_font.font_weight == weight
                && loaded_font.features == features
                && loaded_font.synthetic == SyntheticFontStyle::default()
                && loaded_font.design_coordinates.is_empty()
        }) {
            FontId(index)
        } else {
            let font = self
                .font_system
                .get_font(id, weight)
                .expect("font id should be valid");
            let font_id = FontId(self.loaded_fonts.len());
            self.loaded_fonts
                .push(LoadedFont::new(font, weight, features));

            font_id
        }
    }

    fn font_for_cluster(&self, primary: FontId, cluster: &str) -> FontId {
        std::iter::once(primary)
            .chain(self.loaded_font(primary).fallbacks.iter().copied())
            .find(|id| font_covers(&self.loaded_font(*id).font, cluster))
            .unwrap_or(primary)
    }

    fn glyph_is_emoji(&mut self, id: FontId, glyph: u16, font_size: Pixels) -> bool {
        let loaded = self.loaded_font(id);
        if !loaded.has_color_glyphs {
            return false;
        }
        let key = CacheKey::new(
            loaded.font.id(),
            glyph,
            font_size.into(),
            (0., 0.),
            loaded.font_weight,
            cosmic_text::CacheKeyFlags::empty(),
        )
        .0;
        let style = loaded.synthetic;
        if style == SyntheticFontStyle::default() && loaded.design_coordinates.is_empty() {
            self.swash_cache
                .get_image(&mut self.font_system, key)
                .as_ref()
                .is_some_and(|image| image.content == SwashContent::Color)
        } else {
            self.glyph_image(key, style, id)
                .is_some_and(|image| image.content == SwashContent::Color)
        }
    }

    fn add_font_span(&self, attrs: &mut AttrsList, range: std::ops::Range<usize>, id: FontId) {
        let loaded = self.loaded_font(id);
        let face = self
            .font_system
            .db()
            .face(loaded.font.id())
            .expect("font face should exist");
        attrs.add_span(
            range,
            &Attrs::new()
                .metadata(id.0)
                .family(Family::Name(
                    &face.families.first().expect("font family should exist").0,
                ))
                .stretch(face.stretch)
                .style(face.style)
                .weight(loaded.font_weight)
                .font_features(loaded.features.clone()),
        );
    }

    fn layout_line(&mut self, text: &str, font_size: Pixels, font_runs: &[FontRun]) -> LineLayout {
        let mut attrs_list = AttrsList::new(&Attrs::new());
        let mut offset = 0;
        for run in font_runs {
            let end = offset + run.len;
            let mut segment_start = offset;
            let mut selected = run.font_id;
            for (index, cluster) in text[offset..end].grapheme_indices(true) {
                let next = self.font_for_cluster(run.font_id, cluster);
                if next != selected {
                    let boundary = offset + index;
                    if boundary > segment_start {
                        self.add_font_span(&mut attrs_list, segment_start..boundary, selected);
                    }
                    segment_start = boundary;
                    selected = next;
                }
            }
            if segment_start < end {
                self.add_font_span(&mut attrs_list, segment_start..end, selected);
            }
            offset = end;
        }

        let line = ShapeLine::new(
            &mut self.font_system,
            text,
            &attrs_list,
            cosmic_text::Shaping::Advanced,
            4,
        );
        let mut layout_lines = Vec::with_capacity(1);
        line.layout_to_buffer(
            &mut self.scratch,
            font_size.as_f32(),
            None,
            cosmic_text::Wrap::None,
            None,
            &mut layout_lines,
            None,
            cosmic_text::Hinting::Disabled,
        );
        let layout = layout_lines
            .first()
            .expect("layout should contain one line");

        let mut runs: Vec<ShapedRun> = Vec::new();
        for glyph in &layout.glyphs {
            let mut font_id = FontId(glyph.metadata);
            let loaded_font = self.loaded_font(font_id);
            if loaded_font.font.id() != glyph.font_id
                || loaded_font.font_weight != glyph.font_weight
            {
                font_id = self.font_id_for_cosmic_id(glyph.font_id, glyph.font_weight, font_id);
            }
            // GPUI's `is_emoji` selects the RGBA atlas. Font names are not a
            // reliable format test (HarmonyOS uses other color emoji fonts).
            // Mixed fonts can also have ordinary mask glyphs, so inspect the
            // cached raster content only for fonts containing color tables.
            let is_emoji = self.glyph_is_emoji(font_id, glyph.glyph_id, font_size);

            let shaped_glyph = ShapedGlyph {
                id: GlyphId(glyph.glyph_id as u32),
                position: point(
                    (glyph.x + glyph.font_size * glyph.x_offset).into(),
                    (glyph.y - glyph.font_size * glyph.y_offset).into(),
                ),
                index: glyph.start,
                is_emoji,
            };

            if let Some(last_run) = runs
                .last_mut()
                .filter(|last_run| last_run.font_id == font_id)
            {
                last_run.glyphs.push(shaped_glyph);
            } else {
                runs.push(ShapedRun {
                    font_id,
                    glyphs: vec![shaped_glyph],
                });
            }
        }

        LineLayout {
            font_size,
            width: layout.w.into(),
            ascent: layout.max_ascent.into(),
            descent: layout.max_descent.into(),
            runs,
            len: text.len(),
        }
    }
}

fn font_covers(font: &CosmicTextFont, cluster: &str) -> bool {
    let map = font.as_swash().charmap();
    cluster.chars().all(|ch| {
        matches!(ch as u32, 0x200c..=0x200d | 0xfe00..=0xfe0f | 0xe0100..=0xe01ef | 0xe0020..=0xe007f)
            || ch.is_control() || map.map(ch) != 0
    })
}

fn cosmic_font_features_from(features: &FontFeatures) -> Result<CosmicFontFeatures> {
    let mut result = CosmicFontFeatures::new();
    for feature in features.0.iter() {
        let name_bytes: [u8; 4] = feature
            .0
            .as_bytes()
            .try_into()
            .context("incorrect feature flag format")?;

        let tag = cosmic_text::FeatureTag::new(&name_bytes);
        result.set(tag, feature.1);
    }
    Ok(result)
}

fn font_into_properties(font: &Font) -> font_kit::properties::Properties {
    font_kit::properties::Properties {
        style: match font.style {
            FontStyle::Normal => font_kit::properties::Style::Normal,
            FontStyle::Italic => font_kit::properties::Style::Italic,
            FontStyle::Oblique => font_kit::properties::Style::Oblique,
        },
        weight: font_kit::properties::Weight(font.weight.0),
        stretch: Default::default(),
    }
}

fn face_info_into_properties(
    face_info: &cosmic_text::fontdb::FaceInfo,
) -> font_kit::properties::Properties {
    font_kit::properties::Properties {
        style: match face_info.style {
            cosmic_text::Style::Normal => font_kit::properties::Style::Normal,
            cosmic_text::Style::Italic => font_kit::properties::Style::Italic,
            cosmic_text::Style::Oblique => font_kit::properties::Style::Oblique,
        },
        weight: font_kit::properties::Weight(face_info.weight.0.into()),
        stretch: match face_info.stretch {
            cosmic_text::Stretch::Condensed => font_kit::properties::Stretch::CONDENSED,
            cosmic_text::Stretch::Expanded => font_kit::properties::Stretch::EXPANDED,
            cosmic_text::Stretch::ExtraCondensed => font_kit::properties::Stretch::EXTRA_CONDENSED,
            cosmic_text::Stretch::ExtraExpanded => font_kit::properties::Stretch::EXTRA_EXPANDED,
            cosmic_text::Stretch::Normal => font_kit::properties::Stretch::NORMAL,
            cosmic_text::Stretch::SemiCondensed => font_kit::properties::Stretch::SEMI_CONDENSED,
            cosmic_text::Stretch::SemiExpanded => font_kit::properties::Stretch::SEMI_EXPANDED,
            cosmic_text::Stretch::UltraCondensed => font_kit::properties::Stretch::ULTRA_CONDENSED,
            cosmic_text::Stretch::UltraExpanded => font_kit::properties::Stretch::ULTRA_EXPANDED,
        },
    }
}

fn has_color_glyphs(font: &CosmicTextFont) -> bool {
    let font = font.as_swash();
    [*b"COLR", *b"CBDT", *b"sbix"]
        .into_iter()
        .any(|tag| font.table(u32::from_be_bytes(tag)).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn isolated() -> OhosTextSystem {
        let system = OhosTextSystem(RwLock::new(OhosTextSystemState::new(
            FontSystem::new_with_locale_and_db(
                "en-US".into(),
                cosmic_text::fontdb::Database::new(),
            ),
            true,
        )));
        system
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/primary.ttf")),
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/primary-bold.ttf")),
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/first.ttf")),
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/second.ttf")),
            ])
            .unwrap();
        system
    }

    #[test]
    fn general_ui_keeps_bidi_order_and_honors_gpos_offsets() {
        let system = isolated();
        system
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/direction.ttf")),
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/positioned.ttf")),
            ])
            .unwrap();
        let rtl = shape(&system, &gpui::font("Parity Direction"), "אב");
        assert!(
            rtl.runs[0].glyphs[0].position.x > rtl.runs[0].glyphs[1].position.x,
            "general UI retains RTL visual positions"
        );
        let positioned = shape(&system, &gpui::font("Parity Positioned"), "AB");
        assert!((f32::from(positioned.runs[0].glyphs[0].position.x) - 2.4).abs() < 0.001);
        assert!((f32::from(positioned.runs[0].glyphs[0].position.y) + 1.6).abs() < 0.001);
    }

    fn stack(fallbacks: &[&str]) -> Font {
        let mut font = gpui::font("Parity Primary");
        font.fallbacks = Some(gpui::FontFallbacks::from_fonts(
            fallbacks.iter().map(|name| (*name).to_owned()).collect(),
        ));
        font
    }

    pub(super) fn shape(system: &OhosTextSystem, font: &Font, text: &str) -> LineLayout {
        let font_id = system.font_id(font).unwrap();
        system.layout_line(
            text,
            gpui::px(20.),
            &[FontRun {
                len: text.len(),
                font_id,
            }],
        )
    }

    fn families(system: &OhosTextSystem, line: &LineLayout) -> Vec<String> {
        let state = system.0.read();
        line.runs
            .iter()
            .map(|run| {
                state
                    .font_system
                    .db()
                    .face(state.loaded_font(run.font_id).font.id())
                    .unwrap()
                    .families[0]
                    .0
                    .clone()
            })
            .collect()
    }

    #[test]
    fn configured_fallback_order_survives_shaping_and_distinguishes_cache_entries() {
        let system = isolated();
        let first = stack(&["Parity First", "Parity Second"]);
        let second = stack(&["Parity Second", "Parity First"]);
        let first_id = system.font_id(&first).unwrap();
        assert_eq!(system.font_id(&first).unwrap(), first_id);
        assert_ne!(system.font_id(&second).unwrap(), first_id);
        assert_eq!(
            families(&system, &shape(&system, &first, "AΩB")),
            ["Parity Primary", "Parity First", "Parity Primary"]
        );
        assert_eq!(
            families(&system, &shape(&system, &second, "AΩB")),
            ["Parity Primary", "Parity Second", "Parity Primary"]
        );
    }

    #[test]
    fn an_unavailable_primary_uses_the_first_available_requested_family() {
        let system = isolated();
        let mut font = stack(&["Missing Too", "Parity Second", "Parity First"]);
        font.family = "Missing Primary".into();
        assert_eq!(
            families(&system, &shape(&system, &font, "AΩ")),
            ["Parity Second"]
        );
    }

    #[test]
    fn combining_clusters_choose_a_face_covering_the_entire_cluster() {
        let system = isolated();
        let font = stack(&["Parity First"]);
        assert_eq!(
            families(&system, &shape(&system, &font, "A\u{301}")),
            ["Parity First"]
        );
    }

    #[test]
    fn fallback_ligatures_respect_features_and_font_stack_cache_identity() {
        let system = isolated();
        let enabled = stack(&["Parity First"]);
        let mut disabled = enabled.clone();
        disabled.features = FontFeatures(Arc::new(vec![("liga".into(), 0)]));
        let with = shape(&system, &enabled, "ΩΩ");
        let without = shape(&system, &disabled, "ΩΩ");
        assert_eq!(families(&system, &with), ["Parity First"]);
        assert_eq!(
            with.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
            1
        );
        assert_eq!(
            without
                .runs
                .iter()
                .map(|run| run.glyphs.len())
                .sum::<usize>(),
            2
        );
        assert_eq!(shape(&system, &enabled, "ΩΩ").runs[0].glyphs.len(), 1);
    }

    #[test]
    fn requested_weight_selects_the_matching_face_and_new_fonts_invalidate_resolution() {
        let system = isolated();
        let regular = stack(&[]);
        let bold = regular.clone().bold();
        let regular_id = system.font_id(&regular).unwrap();
        let metrics = system.font_metrics(regular_id);
        assert_eq!(metrics.descent, -200.);
        assert_eq!(metrics.bounding_box.size.height, 1000.);
        let bold_id = system.font_id(&bold).unwrap();
        let glyph = system.glyph_for_char(regular_id, 'A').unwrap();
        assert_eq!(system.advance(regular_id, glyph).unwrap().width, 500.);
        assert_eq!(
            system
                .advance(bold_id, system.glyph_for_char(bold_id, 'A').unwrap())
                .unwrap()
                .width,
            600.
        );
        let mut missing = stack(&["Parity Primary"]);
        missing.family = "Parity New".into();
        let previous = system.font_id(&missing).unwrap();
        assert_eq!(
            system.0.read().loaded_font(previous).font.id(),
            system.0.read().loaded_font(regular_id).font.id()
        );
        system
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../../tests/text/fonts/new.ttf"
            ))])
            .unwrap();
        assert_eq!(
            families(&system, &shape(&system, &missing, "AΩ")),
            ["Parity New"]
        );
        assert_eq!(
            system.glyph_for_char(regular_id, 'A'),
            Some(glyph),
            "previously shaped font IDs remain valid"
        );
    }

    #[test]
    fn variable_weight_is_shared_by_shaping_advance_and_rasterization() {
        let system = isolated();
        system
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../../tests/text/fonts/variable.ttf"
            ))])
            .unwrap();
        let regular = gpui::font("Parity Variable");
        let bold = regular.clone().bold();
        let regular_line = shape(&system, &regular, "AB");
        let bold_line = shape(&system, &bold, "AB");
        assert!((f32::from(bold_line.width) - f32::from(regular_line.width) * 1.2).abs() < 0.01);
        let mut widths = Vec::new();
        for (font, advance) in [(&regular, 500.), (&bold, 600.)] {
            let id = system.font_id(font).unwrap();
            let glyph = system.glyph_for_char(id, 'A').unwrap();
            assert!((system.advance(id, glyph).unwrap().width - advance).abs() < 0.01);
            let params = RenderGlyphParams {
                font_id: id,
                glyph_id: glyph,
                font_size: gpui::px(20.),
                subpixel_variant: point(0, 0),
                scale_factor: 1.,
                is_emoji: false,
                subpixel_rendering: false,
                dilation: 0,
            };
            let bounds = system.glyph_raster_bounds(&params).unwrap();
            widths.push(bounds.size.width);
            let (size, pixels) = system.rasterize_glyph(&params, bounds).unwrap();
            assert_eq!(pixels.len(), size.width.0 as usize * size.height.0 as usize);
        }
        assert!(
            widths[1] > widths[0],
            "bold rasterization must use the same variable weight as shaping"
        );
    }

    #[test]
    fn catalog_preserves_exact_family_named_style_and_face_identity() {
        let system = Arc::new(isolated());
        let catalog = FontCatalog::new(system.clone());
        let requested = gpui::font("Parity Primary");
        assert!(
            catalog
                .resolve(&gpui::font("Absent Family"), None)
                .is_none()
        );
        assert!(catalog.resolve(&requested.clone().italic(), None).is_none());
        assert!(catalog.resolve(&requested, Some("Absent Style")).is_none());
        let regular = catalog.resolve(&requested, None).unwrap();
        let named = catalog.resolve(&requested, Some("bold")).unwrap();
        let bold = catalog.resolve(&requested.clone().bold(), None).unwrap();
        assert_eq!(named, bold);
        assert_ne!(regular, bold);
        assert_eq!(
            system.font_id(&named).unwrap(),
            shape(&system, &named, "A").runs[0].font_id
        );
        for (font, width) in [(&regular, 500.), (&bold, 600.)] {
            let id = system.font_id(font).unwrap();
            assert_eq!(
                system
                    .advance(id, system.glyph_for_char(id, 'A').unwrap())
                    .unwrap()
                    .width,
                width
            );
        }
        assert!(catalog.supports(&regular, "AB", None));
        assert!(!catalog.supports(&regular, "A\u{301}", None));
        assert!(catalog.has_text(&regular));
    }

    #[test]
    fn named_variable_weight_instances_use_their_coordinates_in_shaping_and_painting() {
        let system = Arc::new(isolated());
        system
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../../tests/text/fonts/variable.ttf"
            ))])
            .unwrap();
        let catalog = FontCatalog::new(system.clone());
        let requested = gpui::font("Parity Variable");
        let regular = catalog.resolve(&requested, Some("Regular")).unwrap();
        let named_bold = catalog.resolve(&requested, Some("Bold")).unwrap();
        assert_eq!(
            named_bold,
            catalog.resolve(&requested, Some("bOlD")).unwrap()
        );
        assert_ne!(regular, named_bold);
        assert_eq!(f32::from(shape(&system, &named_bold, "AB").width), 24.);
        let id = system.font_id(&named_bold).unwrap();
        assert_eq!(
            system
                .advance(id, system.glyph_for_char(id, 'A').unwrap())
                .unwrap()
                .width,
            600.
        );
    }

    #[test]
    fn native_presentation_checks_the_actual_glyph_and_discovery_preserves_features() {
        let system = Arc::new(isolated());
        system
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/emoji-text.ttf")),
                Cow::Borrowed(include_bytes!("../../tests/text/fonts/emoji-color.ttf")),
            ])
            .unwrap();
        let catalog = FontCatalog::new(system.clone());
        let features = FontFeatures(Arc::new(vec![("liga".into(), 0)]));
        let text = catalog.discover("😀", false, &features).unwrap();
        let color = catalog.discover("😀", true, &features).unwrap();
        assert_ne!(text, color);
        for (font, emoji) in [(&text, false), (&color, true)] {
            assert_eq!(font.features, features);
            assert!(catalog.supports(font, "😀\u{fe0f}", None));
            assert!(catalog.supports(font, "😀", Some(emoji)));
            assert!(!catalog.supports(font, "😀", Some(!emoji)));
            assert_eq!(shape(&system, font, "😀").runs[0].glyphs[0].is_emoji, emoji);
        }
        assert!(
            catalog.supports(&color, "B", Some(false)),
            "mixed color fonts still have mask glyphs"
        );
        assert!(!catalog.supports(&color, "B", Some(true)));
    }

    #[cfg(unix)]
    #[test]
    fn native_font_inventory_sorts_files_follows_links_and_breaks_directory_cycles() {
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory =
            Directory(std::env::temp_dir().join(format!("gpui-font-links-{}", std::process::id())));
        std::fs::create_dir_all(directory.0.join("nested")).unwrap();
        std::fs::write(
            directory.0.join("z.ttf"),
            include_bytes!("../../tests/text/fonts/primary.ttf"),
        )
        .unwrap();
        std::fs::write(
            directory.0.join("nested/a.ttf"),
            include_bytes!("../../tests/text/fonts/primary-bold.ttf"),
        )
        .unwrap();
        std::os::unix::fs::symlink(directory.0.join("z.ttf"), directory.0.join("alias.ttf"))
            .unwrap();
        std::os::unix::fs::symlink(&directory.0, directory.0.join("nested/cycle")).unwrap();
        let mut db = cosmic_text::fontdb::Database::new();
        OhosTextSystemState::load_font_directories(
            &mut db,
            &[&directory.0, &directory.0.join("nested")],
        );
        assert_eq!(
            db.faces().count(),
            2,
            "overlapping roots, aliases and cycles must not duplicate faces"
        );
        assert_eq!(
            db.faces().map(|face| face.weight.0).collect::<Vec<_>>(),
            [700, 400],
            "inventory order follows sorted canonical file paths"
        );
    }

    #[test]
    fn synthetic_faces_keep_advance_but_have_distinct_rasters_and_cache_entries() {
        let system = Arc::new(isolated());
        let catalog = FontCatalog::new(system.clone());
        let regular = catalog.resolve(&gpui::font("Parity First"), None).unwrap();
        let bold = catalog
            .synthesize(
                &regular,
                SyntheticFontStyle {
                    bold: true,
                    italic: false,
                },
            )
            .unwrap();
        let italic = catalog
            .synthesize(
                &regular,
                SyntheticFontStyle {
                    bold: false,
                    italic: true,
                },
            )
            .unwrap();
        let both = catalog
            .synthesize(
                &bold,
                SyntheticFontStyle {
                    bold: false,
                    italic: true,
                },
            )
            .unwrap();
        assert_eq!(
            catalog
                .synthesize(
                    &italic,
                    SyntheticFontStyle {
                        bold: true,
                        italic: false
                    }
                )
                .unwrap(),
            both
        );
        let mut rasters = Vec::new();
        for font in [&regular, &bold, &italic, &both] {
            let id = system.font_id(font).unwrap();
            let line = shape(&system, font, "AB");
            assert_eq!(line.runs[0].font_id, id);
            assert_eq!(f32::from(line.width), 28.);
            let glyph = system.glyph_for_char(id, 'A').unwrap();
            assert_eq!(system.advance(id, glyph).unwrap().width, 700.);
            let params = RenderGlyphParams {
                font_id: id,
                glyph_id: glyph,
                font_size: gpui::px(32.),
                subpixel_variant: point(0, 0),
                scale_factor: 1.,
                is_emoji: false,
                subpixel_rendering: false,
                dilation: 0,
            };
            let bounds = system.glyph_raster_bounds(&params).unwrap();
            let raster = system.rasterize_glyph(&params, bounds).unwrap();
            assert_eq!(system.rasterize_glyph(&params, bounds).unwrap(), raster);
            rasters.push(raster);
        }
        for left in 0..rasters.len() {
            for right in left + 1..rasters.len() {
                assert_ne!(rasters[left], rasters[right]);
            }
        }
        // System fallback must not accidentally borrow a synthetic ID for the
        // same underlying file when weight and OpenType features are equal.
        let regular_id = system.font_id(&regular).unwrap();
        let mut state = system.0.write();
        let loaded = state.loaded_font(regular_id);
        let (id, weight) = (loaded.font.id(), loaded.font_weight);
        let fallback = state.font_id_for_cosmic_id(id, weight, regular_id);
        assert_eq!(
            state.loaded_font(fallback).synthetic,
            SyntheticFontStyle::default()
        );
    }
    #[test]
    fn effective_variable_axes_share_shaping_metrics_raster_and_immutable_identity() {
        let system = Arc::new(isolated());
        system
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../../tests/text/fonts/variable.ttf"
            ))])
            .unwrap();
        let catalog = FontCatalog::new(system.clone());
        let base = catalog
            .resolve(&gpui::font("Parity Variable"), None)
            .unwrap();
        let mut rasters = Vec::new();
        let mut ids = Vec::new();
        for (weight, advance) in [(400, 500), (700, 600)] {
            let axes = [FontDesignCoordinate {
                tag: *b"wght",
                value: weight * 65536,
            }];
            let font = catalog.with_design_coordinates(&base, &axes).unwrap();
            assert_eq!(font, catalog.with_design_coordinates(&base, &axes).unwrap());
            let id = system.font_id(&font).unwrap();
            ids.push(id);
            assert_eq!(catalog.face_data(&font).unwrap().design_coordinates(), axes);
            let glyph = system.glyph_for_char(id, 'A').unwrap();
            assert!((system.advance(id, glyph).unwrap().width - advance as f32).abs() < 0.01);
            {
                let state = system.0.read();
                let loaded = state.loaded_font(id);
                let shaper = loaded
                    .variable_shaper
                    .as_ref()
                    .unwrap()
                    .shaper(&loaded.font);
                let mut buffer = harfrust::UnicodeBuffer::new();
                buffer.push_str("A");
                buffer.guess_segment_properties();
                let shaped = shaper.shape(buffer, &[]);
                assert_eq!(shaped.glyph_positions()[0].x_advance, advance);
            }
            let params = RenderGlyphParams {
                font_id: id,
                glyph_id: glyph,
                font_size: gpui::px(20.),
                subpixel_variant: point(0, 0),
                scale_factor: 1.,
                is_emoji: false,
                subpixel_rendering: false,
                dilation: 0,
            };
            let bounds = system.glyph_raster_bounds(&params).unwrap();
            rasters.push((bounds, system.rasterize_glyph(&params, bounds).unwrap()));
        }
        assert_ne!(ids[0], ids[1]);
        assert_ne!(rasters[0], rasters[1]);
        let params = RenderGlyphParams {
            font_id: ids[0],
            glyph_id: system.glyph_for_char(ids[0], 'A').unwrap(),
            font_size: gpui::px(20.),
            subpixel_variant: point(0, 0),
            scale_factor: 1.,
            is_emoji: false,
            subpixel_rendering: false,
            dilation: 0,
        };
        let bounds = system.glyph_raster_bounds(&params).unwrap();
        assert_eq!(
            (bounds, system.rasterize_glyph(&params, bounds).unwrap()),
            rasters[0]
        );
        assert!(
            catalog
                .face_data(&base)
                .unwrap()
                .design_coordinates()
                .is_empty()
        );
        assert!(
            catalog
                .with_design_coordinates(
                    &base,
                    &[FontDesignCoordinate {
                        tag: *b"abcd",
                        value: 0
                    }]
                )
                .is_none()
        );
        let synthetic = catalog
            .synthesize(
                &catalog
                    .with_design_coordinates(
                        &base,
                        &[FontDesignCoordinate {
                            tag: *b"wght",
                            value: 700 * 65536,
                        }],
                    )
                    .unwrap(),
                SyntheticFontStyle {
                    bold: true,
                    italic: true,
                },
            )
            .unwrap();
        assert_eq!(
            catalog.face_data(&synthetic).unwrap().design_coordinates()[0].value,
            700 * 65536
        );
    }
}
