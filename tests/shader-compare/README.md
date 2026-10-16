# Shader pixel comparison (macOS)

This standalone harness renders the reference and candidate OHOS WGSL through
wgpu's Metal backend and requires byte-for-byte identical RGBA8 images. It uses
the same pinned wgpu revision as gpui-ohos. It does not require an OHOS SDK.

From the repository root:

```sh
git show fda4f2e:src/ohos/shaders.wgsl > /tmp/gpui-ohos-reference.wgsl
cargo run --manifest-path tests/shader-compare/Cargo.toml --locked -- \
  /tmp/gpui-ohos-reference.wgsl src/ohos/shaders.wgsl
```

The 576 comparisons cover quads (plain, rounded, borders and gradients), blurred
shadows, straight/wavy underlines, integral and fractional masks, half-pixel mask
edges, empty/fully clipped masks, and shapes extending outside the viewport.
The reference is supplied explicitly so a candidate cannot silently update its
own expected shader. Input records match the production vertex attribute layout.

This is an offscreen correctness check, not an OHOS performance measurement or a
substitute for real-device GLES checks.
