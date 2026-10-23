use super::tests::{isolated, shape};
use super::*;
use gpui::{FontFeatures, PlatformTextSystem};
use std::{borrow::Cow, sync::Arc};

fn context() -> (Arc<OhosTextSystem>, FontCatalog) {
    let system = Arc::new(isolated());
    system
        .add_fonts(vec![
            Cow::Borrowed(font_fixture!("direction.ttf")),
            Cow::Borrowed(font_fixture!("positioned.ttf")),
            Cow::Borrowed(font_fixture!("spaced.ttf")),
            Cow::Borrowed(font_fixture!("replacement.ttf")),
        ])
        .unwrap();
    let catalog = FontCatalog::new(system.clone());
    (system, catalog)
}

fn run(font: &Font, len: usize) -> gpui::TextRun {
    gpui::TextRun {
        len,
        font: font.clone(),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn cells(text: &str) -> Vec<GridTextCell> {
    // Test strings below deliberately contain one scalar per cell.
    text.char_indices()
        .enumerate()
        .map(|(column, (byte, _))| GridTextCell {
            byte_index: byte as u32,
            column: column as u16,
            width: 1,
        })
        .collect()
}

fn layout(catalog: &FontCatalog, font: &Font, text: &str, scale_factor: f32) -> LineLayout {
    catalog
        .layout_grid(&GridLayoutRequest {
            text,
            cells: &cells(text),
            runs: &[run(font, text.len())],
            shape_breaks: None,
            line_metrics: None,
            base_font: font,
            font_size: gpui::px(20.),
            cell_width: gpui::px(10.),
            scale_factor,
        })
        .unwrap()
}

#[test]
fn terminal_is_ltr_while_ui_retains_bidi_and_grid_baseline_comes_from_primary() {
    let (system, catalog) = context();
    let direction = catalog
        .resolve(&gpui::font("Parity Direction"), None)
        .unwrap();
    let primary = catalog
        .resolve(&gpui::font("Parity Primary"), None)
        .unwrap();
    let ui = shape(&system, &direction, "אב");
    assert!(ui.runs[0].glyphs[0].position.x > ui.runs[0].glyphs[1].position.x);
    let text = "Aאב B";
    let line = catalog
        .layout_grid(&GridLayoutRequest {
            text,
            cells: &cells(text),
            runs: &[run(&direction, text.len())],
            shape_breaks: None,
            line_metrics: None,
            base_font: &primary,
            font_size: gpui::px(20.),
            cell_width: gpui::px(10.),
            scale_factor: 1.,
        })
        .unwrap();
    assert_eq!(
        line.runs[0]
            .glyphs
            .iter()
            .map(|glyph| (glyph.index, f32::from(glyph.position.x)))
            .collect::<Vec<_>>(),
        [(0, 0.), (1, 10.), (3, 20.), (5, 30.), (6, 40.)]
    );
    assert_eq!(line.ascent, gpui::px(16.));
    assert_eq!(line.descent, gpui::px(4.));
    assert_ne!(
        ui.ascent, line.ascent,
        "fallback fonts cannot move the terminal baseline"
    );
}

#[test]
fn combining_and_reordered_glyphs_report_the_official_vt_anchor_byte() {
    let (_, catalog) = context();
    let font = catalog
        .resolve(&gpui::font("Parity Direction"), None)
        .unwrap();
    let mut outcomes = Vec::new();
    for (text, cells, expected) in [
        (
            "A\u{301}B",
            vec![
                GridTextCell {
                    byte_index: 0,
                    column: 0,
                    width: 1,
                },
                GridTextCell {
                    byte_index: 3,
                    column: 1,
                    width: 1,
                },
            ],
            vec![0, 0, 3],
        ),
        (
            "ककिB",
            vec![
                GridTextCell {
                    byte_index: 0,
                    column: 0,
                    width: 1,
                },
                GridTextCell {
                    byte_index: 3,
                    column: 1,
                    width: 2,
                },
                GridTextCell {
                    byte_index: 9,
                    column: 3,
                    width: 1,
                },
            ],
            vec![0, 0, 0, 9],
        ),
    ] {
        let line = catalog
            .layout_grid(&GridLayoutRequest {
                text,
                cells: &cells,
                runs: &[run(&font, text.len())],
                shape_breaks: None,
                line_metrics: None,
                base_font: &font,
                font_size: gpui::px(20.),
                cell_width: gpui::px(10.),
                scale_factor: 1.,
            })
            .unwrap();
        outcomes.push((
            text,
            line.runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.index)
                .collect::<Vec<_>>(),
            expected,
        ));
    }
    assert!(
        outcomes
            .iter()
            .all(|(_, actual, expected)| actual == expected),
        "{outcomes:?}"
    );
}

#[test]
fn full_runs_shape_across_spaces_respect_features_and_keep_ligature_cell_extents() {
    let (system, catalog) = context();
    let font = catalog.resolve(&gpui::font("Parity Spaced"), None).unwrap();
    assert_eq!(shape(&system, &font, "A B").runs[0].glyphs.len(), 3);
    let line = layout(&catalog, &font, "A BA", 1.);
    assert_eq!(line.runs[0].glyphs.len(), 2);
    assert_eq!(line.runs[0].glyphs[1].position.x, gpui::px(30.));
    assert_eq!(line.width, gpui::px(40.));
    let mut disabled = gpui::font("Parity Spaced");
    disabled.features = FontFeatures(Arc::new(vec![("liga".into(), 0)]));
    let disabled = catalog.resolve(&disabled, None).unwrap();
    assert_eq!(
        layout(&catalog, &disabled, "A B", 1.).runs[0].glyphs.len(),
        3
    );
    assert_eq!(layout(&catalog, &font, "A B", 1.).runs[0].glyphs.len(), 1);
    let text = "A B";
    let split = catalog
        .layout_grid(&GridLayoutRequest {
            text,
            cells: &cells(text),
            runs: &[run(&font, 1), run(&font, 2)],
            shape_breaks: None,
            line_metrics: None,
            base_font: &font,
            font_size: gpui::px(20.),
            cell_width: gpui::px(10.),
            scale_factor: 1.,
        })
        .unwrap();
    assert_eq!(
        split.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
        3,
        "caller style breaks must survive"
    );
}

#[test]
fn explicit_shape_boundaries_allow_background_runs_to_join_and_can_split_inside_paint_runs() {
    let (_, catalog) = context();
    let font = catalog.resolve(&gpui::font("Parity Spaced"), None).unwrap();
    let text = "A B";
    let cells = cells(text);
    let mut second = run(&font, 2);
    second.background_color = Some(gpui::white());
    let runs = [run(&font, 1), second];
    let mut request = GridLayoutRequest {
        text,
        cells: &cells,
        runs: &runs,
        shape_breaks: Some(&[]),
        line_metrics: None,
        base_font: &font,
        font_size: gpui::px(20.),
        cell_width: gpui::px(10.),
        scale_factor: 1.,
    };
    assert_eq!(
        catalog.layout_grid(&request).unwrap().runs[0].glyphs.len(),
        1
    );
    request.shape_breaks = Some(&[1]);
    assert_eq!(
        catalog
            .layout_grid(&request)
            .unwrap()
            .runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>(),
        3
    );
    let one = [run(&font, text.len())];
    request.runs = &one;
    assert_eq!(
        catalog
            .layout_grid(&request)
            .unwrap()
            .runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>(),
        3
    );
    for bad in [&[0][..], &[3][..], &[2, 1][..], &[1, 1][..]] {
        request.shape_breaks = Some(bad);
        assert_eq!(
            catalog.layout_grid(&request).unwrap_err(),
            GridLayoutError::InvalidGeometry
        );
    }
}

#[test]
fn transparent_runs_preserve_cells_without_producing_glyphs() {
    let (_, catalog) = context();
    let font = catalog
        .resolve(&gpui::font("Parity Positioned"), None)
        .unwrap();
    let text = "ABA";
    let cells = cells(text);
    let hidden = gpui::TextRun {
        color: gpui::black().alpha(0.),
        ..run(&font, 1)
    };
    let runs = [run(&font, 1), hidden.clone(), run(&font, 1)];
    let mut request = GridLayoutRequest {
        text,
        cells: &cells,
        runs: &runs,
        shape_breaks: Some(&[]),
        line_metrics: None,
        base_font: &font,
        font_size: gpui::px(20.),
        cell_width: gpui::px(10.),
        scale_factor: 1.,
    };
    let line = catalog.layout_grid(&request).unwrap();
    assert_eq!(
        line.runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| (glyph.index, f32::from(glyph.position.x)))
            .collect::<Vec<_>>(),
        [(0, 2.), (2, 22.)]
    );
    assert_eq!(line.width, gpui::px(30.));
    let all_hidden = [gpui::TextRun {
        len: text.len(),
        ..hidden
    }];
    request.runs = &all_hidden;
    let line = catalog.layout_grid(&request).unwrap();
    assert!(line.runs.is_empty());
    assert_eq!(line.len, 3);
    assert_eq!(line.width, gpui::px(30.));
}

#[test]
fn positioning_offsets_round_to_device_pixels_without_changing_the_next_cell() {
    let (_, catalog) = context();
    let font = catalog
        .resolve(&gpui::font("Parity Positioned"), None)
        .unwrap();
    for (scale, expected) in [(1., (2., -2.)), (2., (2.5, -1.5))] {
        let line = layout(&catalog, &font, "AB", scale);
        assert_eq!(
            line.runs[0].glyphs[0].position,
            point(gpui::px(expected.0), gpui::px(expected.1))
        );
        assert_eq!(line.runs[0].glyphs[1].position.x, gpui::px(10.));
        assert_eq!(line.width, gpui::px(20.));
    }
}

#[test]
fn unsupported_cells_replace_the_entire_cluster_and_bad_geometry_is_rejected() {
    let (_, catalog) = context();
    let font = catalog
        .resolve(&gpui::font("Parity Replacement"), None)
        .unwrap();
    let mut request = GridLayoutRequest {
        text: "A\u{301}",
        cells: &[GridTextCell {
            byte_index: 0,
            column: 0,
            width: 1,
        }],
        runs: &[run(&font, 3)],
        shape_breaks: None,
        line_metrics: None,
        base_font: &font,
        font_size: gpui::px(20.),
        cell_width: gpui::px(10.),
        scale_factor: 1.,
    };
    let line = catalog.layout_grid(&request).unwrap();
    assert_eq!(line.runs[0].glyphs.len(), 1);
    assert_eq!(
        line.runs[0].glyphs[0].id.0, 3,
        "B is the fixture's replacement glyph"
    );
    request.scale_factor = f32::NAN;
    assert_eq!(
        catalog.layout_grid(&request).unwrap_err(),
        GridLayoutError::InvalidGeometry
    );
    request.scale_factor = 1.;
    request.font_size = gpui::px(f32::MAX);
    request.scale_factor = 2.;
    assert_eq!(
        catalog.layout_grid(&request).unwrap_err(),
        GridLayoutError::InvalidGeometry
    );
    request.font_size = gpui::px(20.);
    request.scale_factor = 1.;
    request.cells = &[GridTextCell {
        byte_index: 1,
        column: 0,
        width: 1,
    }];
    assert_eq!(
        catalog.layout_grid(&request).unwrap_err(),
        GridLayoutError::InvalidGeometry
    );
}

#[test]
fn presentation_selectors_choose_faces_without_changing_the_shaped_glyph() {
    let (system, catalog) = context();
    let font = catalog
        .resolve(&gpui::font("Parity Replacement"), None)
        .unwrap();
    let text = "A\u{fe0e}B";
    let line = catalog
        .layout_grid(&GridLayoutRequest {
            text,
            cells: &[
                GridTextCell {
                    byte_index: 0,
                    column: 0,
                    width: 1,
                },
                GridTextCell {
                    byte_index: 4,
                    column: 1,
                    width: 1,
                },
            ],
            runs: &[run(&font, text.len())],
            shape_breaks: None,
            line_metrics: None,
            base_font: &font,
            font_size: gpui::px(20.),
            cell_width: gpui::px(10.),
            scale_factor: 1.,
        })
        .unwrap();
    let id = system.font_id(&font).unwrap();
    assert_eq!(line.runs[0].glyphs.len(), 2);
    assert_eq!(
        line.runs[0].glyphs[0].id,
        system.glyph_for_char(id, 'A').unwrap()
    );
    assert_eq!(line.runs[0].glyphs[1].position.x, gpui::px(10.));
}
#[test]
fn variable_grid_plans_keep_each_coordinate_identity() {
    let system = Arc::new(isolated());
    system
        .add_fonts(vec![Cow::Borrowed(font_fixture!("variable.ttf"))])
        .unwrap();
    let catalog = FontCatalog::new(system.clone());
    let base = catalog
        .resolve(&gpui::font("Parity Variable"), None)
        .unwrap();
    let mut ids = Vec::new();
    for weight in [400, 700, 400] {
        let font = catalog
            .with_design_coordinates(
                &base,
                &[FontDesignCoordinate {
                    tag: *b"wght",
                    value: weight * 65536,
                }],
            )
            .unwrap();
        let id = system.font_id(&font).unwrap();
        ids.push(id);
        let line = layout(&catalog, &font, "AB", 1.);
        assert_eq!(line.runs[0].font_id, id);
        assert_eq!(line.width, gpui::px(20.));
        assert_eq!(line.runs[0].glyphs[1].position.x, gpui::px(10.));
    }
    assert_eq!(ids[0], ids[2]);
    assert_ne!(ids[0], ids[1]);
}
