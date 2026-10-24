# OHOS text backend host regressions

This workspace member compiles the actual OHOS text backend without linking
Ability or the OHOS NDK. It inherits dependencies and `Cargo.lock` from the root
workspace, rather than maintaining a second dependency graph.

```sh
python3 -m pip install -r tests/text/requirements.txt
cargo test -p ohos-text-tests --locked
cargo test -p ohos-text-tests --locked --features grid
```

The default configuration tests locale-aware CJK fallback, document bidi and
GPOS positioning, whole-cluster fallback, font registration/cache invalidation,
synthetic styles, color/mask glyph detection and exact face bytes. Named-instance
tests preserve every signed 16.16 coordinate, including fractional multi-axis
values, default subfamilies and variable fonts without a weight axis.

The nondefault `grid` configuration also tests explicit LTR cell layout, primary
baseline selection, device-pixel rounding, replacement clusters, variation
selectors, transparent runs, variable-face identity, and authoritative shaping
boundaries across paint fragments. A default-build compile-fail doctest checks
that grid types are not exported. CI runs both configurations.

`build.rs` generates the original rectangle-based fonts in `OUT_DIR/fonts` using
fonttools 4.60.1. No fonts are read from the host's installed inventory and no
generated binary fixtures are checked in. Set `PYTHON` to a Python 3 interpreter
with the pinned requirement installed, for example one from a virtual environment.
Generation failures stop the build and include the dependency installation hint.

The harness also checks typed image-policy conflicts, duplicate and cloned owner
lifetimes, concurrent lookup/cleanup, and tile policy retention. It executes the
production sprite encoder with a truly empty metadata map and verifies that it
returns the original slice without allocating or touching scratch storage.
Run only these regressions with:

```sh
cargo test -p ohos-text-tests --release --locked image_alpha
```

These are host shaping and raster regressions, not OHOS device acceptance. Native
font inventory, device locale changes, CJK/emoji fallback, document RTL text, and
terminal fixed-cell rendering still need coordinated device verification.
