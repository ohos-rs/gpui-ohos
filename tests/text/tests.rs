use super::*;

pub(super) fn isolated() -> OhosTextSystem {
    let system = OhosTextSystem(RwLock::new(OhosTextSystemState::new(
        FontSystem::new_with_locale_and_db("en-US".into(), cosmic_text::fontdb::Database::new()),
        true,
    )));
    system
        .add_fonts(vec![
            Cow::Borrowed(font_fixture!("primary.ttf")),
            Cow::Borrowed(font_fixture!("primary-bold.ttf")),
            Cow::Borrowed(font_fixture!("first.ttf")),
            Cow::Borrowed(font_fixture!("second.ttf")),
        ])
        .unwrap();
    system
}

#[test]
fn general_ui_keeps_bidi_order_and_honors_gpos_offsets() {
    let system = isolated();
    system
        .add_fonts(vec![
            Cow::Borrowed(font_fixture!("direction.ttf")),
            Cow::Borrowed(font_fixture!("positioned.ttf")),
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
        .add_fonts(vec![Cow::Borrowed(font_fixture!("new.ttf"))])
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
        .add_fonts(vec![Cow::Borrowed(font_fixture!("variable.ttf"))])
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
        .add_fonts(vec![Cow::Borrowed(font_fixture!("variable.ttf"))])
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
            Cow::Borrowed(font_fixture!("emoji-text.ttf")),
            Cow::Borrowed(font_fixture!("emoji-color.ttf")),
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
    std::fs::write(directory.0.join("z.ttf"), font_fixture!("primary.ttf")).unwrap();
    std::fs::write(
        directory.0.join("nested/a.ttf"),
        font_fixture!("primary-bold.ttf"),
    )
    .unwrap();
    std::os::unix::fs::symlink(directory.0.join("z.ttf"), directory.0.join("alias.ttf")).unwrap();
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
        .add_fonts(vec![Cow::Borrowed(font_fixture!("variable.ttf"))])
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
        #[cfg(feature = "grid")]
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

#[test]
fn face_data_keeps_exact_registered_bytes_alive_after_catalog_and_system_drop() {
    let bytes = font_fixture!("primary.ttf");
    let (system, catalog) = fonts(vec![Cow::Borrowed(bytes)]);
    let font = catalog
        .resolve(&gpui::font("Parity Primary"), None)
        .unwrap();
    let id = system.font_id(&font).unwrap();
    let data = catalog.face_data(&font).unwrap();
    let by_id = catalog.face_data_for_id(id).unwrap();
    assert_eq!(data.index(), 0);
    assert_eq!(data.weight(), 400);
    assert_eq!(data.synthetic(), SyntheticFontStyle::default());
    assert_eq!(data.bytes(), bytes);
    assert_eq!(data.bytes(), by_id.bytes());
    assert!(catalog.face_data_for_id(FontId(usize::MAX)).is_none());
    system
        .add_fonts(vec![Cow::Borrowed(font_fixture!("new.ttf"))])
        .unwrap();
    assert_eq!(catalog.face_data_for_id(id).unwrap().bytes(), bytes);
    drop(catalog);
    drop(system);
    assert_eq!(data.bytes(), bytes);
    assert_eq!(by_id.bytes(), bytes);
}
