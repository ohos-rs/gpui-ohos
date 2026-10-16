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

## Features

| Cargo feature | Purpose |
| --- | --- |
| `gles` | OpenGL ES rendering, enabled by default. |
| `vulkan` | Vulkan rendering. |
| `native-vsync` | Optional per-window switching between NativeVSync and XComponent frame callbacks. |

Touch scrolling uses native gesture recognition and supports momentum scrolling.
Frames are requested on demand through XComponent callbacks by default.

## Optional NativeVSync delivery

Enable `native-vsync`, then choose the frame source on the embedding UI thread
after opening the GPUI window:

```rust
// Ability window ID: 0 for the main window, or the child window's ID.
// Use the OS window ID, or 0 for an unassociated receiver.
// Choose a refresh rate supported by the display.
gpui_ohos::configure_native_refresh(window_id, os_window_id, Some(display_hz))?;

// Return this window to XComponent callbacks.
gpui_ohos::configure_native_refresh(window_id, os_window_id, None)?;
```

XComponent remains the default when the feature is enabled. Native API failures
fall back to XComponent. Invalid rates or windows unavailable on the calling UI
thread return `NativeRefreshError`.

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
