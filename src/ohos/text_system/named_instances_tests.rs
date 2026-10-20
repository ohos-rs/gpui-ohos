use super::*;

fn fixture(bytes: &'static [u8]) -> (Arc<OhosTextSystem>, FontCatalog) {
    let system = Arc::new(OhosTextSystem(RwLock::new(OhosTextSystemState::new(
        FontSystem::new_with_locale_and_db("en-US".into(), cosmic_text::fontdb::Database::new()),
        true,
    ))));
    system.add_fonts(vec![Cow::Borrowed(bytes)]).unwrap();
    let catalog = FontCatalog::new(system.clone());
    (system, catalog)
}

#[test]
fn named_multi_axis_instances_preserve_exact_values_and_separate_cached_shapers() {
    let (system, catalog) = fixture(include_bytes!("../../../tests/text/fonts/multi-axis.ttf"));
    let requested = gpui::font("Parity Multi Axis");
    let mut faces = Vec::new();
    for (name, weight, width, advance) in [
        ("Regular", 400 * 65536, 100 * 65536, 500),
        ("Narrow", 400 * 65536, 75 * 65536, 400),
        ("Wide", 700 * 65536, 125 * 65536, 700),
        ("Fractional", 550 * 65536 + 8193, 87 * 65536 + 32769, 500),
    ] {
        let font = catalog.resolve(&requested, Some(name)).unwrap();
        assert_eq!(
            font,
            catalog
                .resolve(&requested, Some(&name.to_ascii_lowercase()))
                .unwrap()
        );
        let data = catalog.face_data(&font).unwrap();
        assert_eq!(
            data.design_coordinates(),
            [
                FontDesignCoordinate {
                    tag: *b"wght",
                    value: weight
                },
                FontDesignCoordinate {
                    tag: *b"wdth",
                    value: width
                },
            ]
        );
        let id = system.font_id(&font).unwrap();
        assert!(!faces.contains(&id));
        faces.push(id);
        let state = system.0.read();
        let loaded = state.loaded_font(id);
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str("A");
        buffer.guess_segment_properties();
        let shaped = loaded
            .variable_shaper
            .as_ref()
            .unwrap()
            .shaper(&loaded.font)
            .shape(buffer, &[]);
        assert_eq!(shaped.glyph_positions()[0].x_advance, advance);
    }
    assert!(catalog.resolve(&requested, Some("Missing")).is_none());
    let font = catalog.resolve(&requested, Some("Fractional")).unwrap();
    let synthetic = catalog
        .synthesize(
            &font,
            SyntheticFontStyle {
                bold: true,
                italic: true,
            },
        )
        .unwrap();
    assert_eq!(
        catalog.face_data(&synthetic).unwrap().design_coordinates(),
        catalog.face_data(&font).unwrap().design_coordinates()
    );
    let mut featured = requested;
    featured.features = FontFeatures(Arc::new(vec![("liga".into(), 0)]));
    let featured = catalog.resolve(&featured, Some("Fractional")).unwrap();
    assert_ne!(featured, font);
    assert_eq!(
        catalog.face_data(&featured).unwrap().design_coordinates(),
        catalog.face_data(&font).unwrap().design_coordinates()
    );
}

#[test]
fn named_instances_without_a_weight_axis_are_resolved_and_rasterized() {
    let (system, catalog) = fixture(include_bytes!("../../../tests/text/fonts/width-axis.ttf"));
    let requested = gpui::font("Parity Width Axis");
    let mut rasters = Vec::new();
    for (name, width) in [("Narrow", 75), ("Regular", 100), ("Wide", 125)] {
        let font = catalog.resolve(&requested, Some(name)).unwrap();
        assert_eq!(
            catalog.face_data(&font).unwrap().design_coordinates(),
            [FontDesignCoordinate {
                tag: *b"wdth",
                value: width * 65536
            }]
        );
        let id = system.font_id(&font).unwrap();
        let params = RenderGlyphParams {
            font_id: id,
            glyph_id: system.glyph_for_char(id, 'A').unwrap(),
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
    assert_ne!(rasters[0], rasters[1]);
    assert_ne!(rasters[1], rasters[2]);
}
