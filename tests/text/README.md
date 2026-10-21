# OHOS text backend host regressions

`cargo test --manifest-path tests/text/Cargo.toml --release --locked`
compiles the actual OHOS text backend on a host without linking Ability or the
OHOS NDK. The tests use an isolated font database and thirteen tiny original fonts,
so results do not depend on the developer's installed fonts.

`generate_fonts.py` regenerates the committed fixtures with fonttools 4.60.1.
It is not needed to run the tests. Fixtures contain simple rectangles, including
OpenType ligatures across spaces, GPOS offsets, RTL scripts, replacement/UVS
glyphs, named weight instances, a mixed COLR/mask face, and a proportional
face whose widest ASCII advance and large line gap differ from the zero glyph. They
identify real face selection, shaping, synthesis and presentation. The public
host harness also supplies an isolated catalog for terminal integration tests.

`FontCatalog::layout_grid` uses explicit LTR character clusters and VT cell
positions, while ordinary UI layout retains bidi behavior. Both paths use the
same font and raster caches. Device scale, primary baseline, whole-cell
replacement and selector removal are checked independently of installed fonts.
Optional explicit shaping boundaries allow equal font faces to join across
paint fragments; supplied boundaries must be sorted, unique UTF-8 VT cell
boundaries. Without them each caller-provided text run remains separate.

Combining and reordered Indic glyphs retain the official VT anchor byte for
painting, rather than their original codepoint offsets. Transparent runs keep
their text and cell extents without producing glyphs or raster queries.

`FontCatalog::face_data` shares the selected font bytes, collection index and
design weight with an embedder-owned metric engine. Optional device-pixel line
metrics on `GridLayoutRequest` keep every fallback face on that exact baseline.

`FontCatalog::face_data_for_id` exposes a shaped run's exact bytes, TTC index,
design weight and synthetic style. Embedders can own a separate raster engine
without retaining borrowed slices or inventing a different font identity.
The harness also checks typed image-policy conflicts, duplicate and cloned owner
lifetimes, concurrent lookup/cleanup, and tile policy retention. It executes the
production sprite encoder with a truly empty metadata map and verifies that it
returns the original slice without allocating or touching scratch storage.
Run only these regressions with:

```sh
cargo test --manifest-path tests/text/Cargo.toml --release --locked image_alpha
```

Window backdrop checks exercise the actual public GPUI configuration entry,
per-window radius ownership and disposal, and radius-aware request equality.
The native compositor effect is verified separately on OHOS.

Native menu accelerator checks preserve Control and Super independently, retain
the native matcher's modifier order, and exercise every modifier combination so
a more modified chord cannot alias an unrelated less modified binding.
