# gpui-ohos

OpenHarmony platform backend for GPUI.

This crate is intentionally OHOS-only, so its implementation does not contain
`target_env = "ohos"` branches. Applications inject the platform explicitly,
following the same external-platform pattern as `gpui-mobile`:

```rust
let application = gpui::Application::with_platform(
    gpui_ohos::current_platform(openharmony_app, false),
);
```

The GPUI dependency is pinned to the `ohos-rs/zed` commit that exposes the
external-platform entry point and excludes OHOS from the Linux backend. All
OHOS windowing, input, rendering, IME, and gesture behavior lives here.

OpenHarmony Ability and its Rust plugins use the
[`feat/pr82-nearsend-integration`](https://github.com/richerfu/openharmony-ability/tree/feat/pr82-nearsend-integration)
branch. `Cargo.lock` fixes the resolved commit, and the example's ArkTS
submodule follows the same branch and commit.

## Touch input and rendering

XComponent's native ArkUI Pan supplies scroll recognition, displacement and
release velocity. Raw contacts support control capture, taps, long presses and
stopping inertia. The first accepted Pan packet moves the view immediately.
Rendering and post-release momentum advance through a single demand-driven
native VSync source; visibility and surface changes are applied before drawing.

See [touch input and frame scheduling](./docs/touch-input.md) for ownership and
lifecycle details. The host regression harness compiles the actual platform
input adapter without linking OHOS native libraries:

```bash
cargo test --manifest-path tests/touch/Cargo.toml --locked
cargo clippy --manifest-path tests/touch/Cargo.toml --locked --all-targets --no-deps -- -D warnings
```

## Example

[`example`](./example) contains a minimal OpenHarmony native module based on
the NearSend integration: it keeps the embedded GPUI application alive, opens
one full-size window, and renders a centered greeting.

```bash
cd example
ohrs build --arch aarch
```

## License

[Apache-2.0](./LICENSE-APACHE) or [MIT](./LICENSE-MIT)
