> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# gpui-ohos

OpenHarmony platform backend for GPUI, with native windows, GPU rendering,
touch and mouse input, keyboard and IME support, and system integrations.

## Usage

Create the GPUI application with the OpenHarmony platform:

```rust
let application = gpui::Application::with_platform(
    gpui_ohos::current_platform(openharmony_app, false),
);
```

Pass the Ability's `OpenHarmonyApp` as `openharmony_app`.

Configure the Ability and its ArkTS plugins as shown in the
[example entry Ability](./example/ohos/entry/src/main/ets/entryability/EntryAbility.ets),
including the clipboard metadata MIME type and notification response scheme.

## Features

| Cargo feature | Purpose |
| --- | --- |
| `gles` | OpenGL ES rendering, enabled by default. |
| `vulkan` | Vulkan rendering. |
| `native-vsync` | Use NativeVSync frame callbacks instead of XComponent callbacks. |

Touch scrolling uses native gesture recognition and supports momentum scrolling.
Frames are requested on demand through XComponent callbacks by default.

## Optional NativeVSync delivery

Enable `native-vsync` to select NativeVSync for all windows at build time:

```sh
cargo build --target aarch64-unknown-linux-ohos --features native-vsync
```

NativeVSync uses the display refresh rate reported by the system and requires
the platform's native rate APIs. Both modes register callbacks on demand and
release them when windows become hidden or idle.

## Example

The [example app](./example/README.md) demonstrates windows, text input,
scrolling, menus, file pickers, clipboard, credentials, screen capture,
notifications, and application lifecycle.

```bash
cd example
sh scripts/build-native.sh --release
```

For desktop or 2in1 targets, prefix the build command with
`OHOS_DEVICE_TYPE=desktop`. See the example's instructions for signing and
building the HAP.

## Platform notes

- Declare app identity and URL schemes in `app.json5` and `module.json5`.
  `register_url_scheme()` validates the installed declaration.
- Clipboard reads require `ohos.permission.READ_PASTEBOARD`, an ACL-enabled
  signing profile, and runtime permission. Persistent file access requires
  `ohos.permission.FILE_ACCESS_PERSIST`; transparent main windows may require
  `ohos.permission.SET_WINDOW_TRANSPARENT`.
- Hiding other applications, dock menus, and recent-document lists are
  unavailable through the application SDK.
- Native IME configuration does not expose autocorrection, capitalization,
  or suggestion controls. Missing hardware key mappings use a US layout.

For implementation details, see [touch input and frame scheduling](./docs/touch-input.md)
and [OHOS adaptation performance](./docs/ohos-performance.md).

## License

[Apache-2.0](./LICENSE-APACHE) or [MIT](./LICENSE-MIT)

## Desktop runtime adaptation

The backend preserves native window ownership across workspace handoff, restores
content focus around native menus, and keeps IME ownership separate from renderer
Surface readiness. Physical deletion keeps the application's editing commands;
handled completion shortcuts are consumed before the native IME. Clipboard reads
are asynchronous and serialized through native completion, including cancellation.
Picker results retain provider URIs and persisted access policies, and shared
windows track native display density and refresh rate.

Initialize the pinned Ability runtime before building:

```sh
git submodule update --init --recursive
```

Rust dependencies and the example's ArkTS packages use the same submodule commit.
It includes unpublished runtime interfaces; using registry beta.3 for only one
side of that interface is unsupported. The IME binding dependency is pinned by
Ability to an exact Git commit until a compatible release is available. When
sharing a local integration branch, publish its dependency commits first.

IME attachment cancellation only signals the owning background worker; the
worker completes native cleanup before another request for that editor runs.
Terminal echo reconciliation belongs to the terminal implementation and is not
part of the platform input interface.

Persist restart state before requesting a restart. A rejected SDK request keeps
GPUI windows alive and reports the error. An accepted restart can terminate the
process before shutdown observers finish, so they cannot be used for mandatory
restart persistence.

Portable checks:

```sh
cargo test --manifest-path tests/touch/Cargo.toml
cargo test --manifest-path tests/touch/Cargo.toml --features native-vsync
cargo check --target aarch64-unknown-linux-ohos
```

Host checks substitute native window/frame APIs. Native creation-time input
routing, task-center close confirmation and enforced provider permission behavior
still need further work; this does not establish complete desktop parity.


## Shared font services

Embedders that need exact face bytes, synthetic styles or fixed-cell text can use
`PlatformServices::try_new(app, PlatformOptions::default())`. Its `fonts` catalog
shares immutable font IDs and face data with the returned GPUI platform. Ordinary
UI text retains the platform's locale-aware fallback and bidirectional shaping.
`layout_grid` is an explicit fixed-cell path; it does not replace document layout.
Named instances preserve every original 16.16 axis coordinate, including width-only
fonts. Explicit variation overrides retain the instance's other axes.

Host font regressions: `cargo test --manifest-path tests/text/Cargo.toml`.
The small original font fixtures are reproducible with the generators in that directory.
