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

OpenHarmony Ability, its derive macro and Rust plugins use the published
`1.0.0-beta.3` crates. The example's ArkTS packages use the
[`feat/ohos-adaptation-runtime-fixes`](https://github.com/richerfu/openharmony-ability/tree/feat/ohos-adaptation-runtime-fixes)
branch through the `openharmony-ability` submodule; its Git entry pins the exact
commit. Run `git submodule update --init --recursive` before building the HAP.

Ability uses published ArkUI `0.4.2`, XComponent `0.5.2`, and IME `0.3.1`
bindings. GPUI enables Ability's `keyboard`, `mouse`, and `drag`
features for hardware key delivery, native mouse cancellation, and file drag/drop.
Ability's `mouse` feature forwards XComponent's `api-18`, so GPUI uses Ability's
re-exported XComponent types without a direct XComponent binding dependency.
The crate and example need no crates.io patches or neighboring local binding checkout.

## Touch input and rendering

XComponent's native ArkUI Pan supplies scroll recognition, displacement and
release velocity. Raw contacts support control capture, taps, long presses and
stopping inertia. The first accepted Pan packet moves the view immediately.
By default, rendering and post-release momentum advance through XComponent frame
callbacks, registered on demand and removed after 12 idle callbacks. A failed
registration schedules one delayed draw; visibility and surface changes are
applied before drawing.

See [touch input and frame scheduling](./docs/touch-input.md) for ownership and
lifecycle details, and [OHOS adaptation performance](./docs/ohos-performance.md)
for frame pacing, quad rendering and runtime ownership.

## Optional NativeVSync delivery

Enable the `native-vsync` Cargo feature to allow each window to switch between
NativeVSync and XComponent callbacks at runtime. XComponent remains the default,
including when this feature is enabled. Builds without the feature do not link
NativeVSync. Receiver and callback ownership use `ohos-vsync-binding` `0.3.1`.

On the embedding UI thread, after opening the GPUI window:

```rust
// Ability window ID: 0 for the main window, or the child window's ID.
// Use the OS window ID and a rate from its display's supported refresh rates.
gpui_ohos::configure_native_refresh(window_id, os_window_id, Some(display_hz))?;

// Return this window to XComponent callbacks.
gpui_ohos::configure_native_refresh(window_id, os_window_id, None)?;
```

The switch is deferred until outside frame callbacks. Only one source delivers
frames to a window; callbacks queued for an old native receiver are discarded.
Native callbacks return to the UI thread, and the next VSync is requested before
drawing. Hidden, destroyed or idle windows release their receiver and refresh-rate
request; new demand recreates it. Missing native rate APIs or receiver failures
fall back to XComponent. There is no fixed refresh-rate target.

Invalid rates and windows absent from the calling UI thread return
`NativeRefreshError`. Passing `None` for an already closed window succeeds.

## Example

[`example`](./example) exercises GPUI windows, scrolling, menus, file pickers,
clipboard, credentials, screen capture, notifications, appearance, system state,
and application lifecycle on OpenHarmony. Its native module uses the published
Ability Rust crates; the HAP uses the initialized submodule's ArkTS plugins.

```bash
cd example
OHOS_DEVICE_TYPE=desktop sh scripts/build-native.sh
```

## Platform limits

The backend implements the GPUI capabilities that the OpenHarmony application
SDK exposes. Some cross-platform methods cannot have the same effect on a
third-party OpenHarmony app:

- App identity and URL schemes are declared in `app.json5` and `module.json5`;
  they cannot be registered or changed at runtime. `register_url_scheme()`
  checks whether the installed Ability already declares the requested scheme.
- Hiding other applications, app-specific dock menus, recent-document lists,
  and jump lists have no matching third-party SDK API.
- `hide()` minimizes this app's visible windows on 2in1 and remembers which
  windows it hid. `activate()` or a system relaunch restores those windows;
  windows minimized before `hide()` stay minimized. The window bridge waits
  for the OS show/restore operation before reporting success. `hide_other_apps()`
  and `unhide_other_apps()` cannot affect other applications on OpenHarmony.
- Custom titlebar movement and all eight resize edges use native pointer screen
  coordinates with the SDK's window move/resize calls. Movement, resize permission,
  and minimum sizes follow `WindowOptions`. The GPUI window menu uses an anchored
  native dialog to perform minimize, maximize/restore, fullscreen, and close.
  OpenHarmony's `Window.startMoving()` itself requires an ArkTS touch callback;
  there is no public SDK entry point for the system titlebar context menu.
- Native prompts preserve button order and cancel indices. Window snapshots retain
  position, size, maximized/fullscreen state, and the restore frame. Window stacking
  comes from the SDK's actual current-app window order.
- Retaining file picker access across app restarts requires
  `ohos.permission.FILE_ACCESS_PERSIST` and is best effort on supported devices.
- Clipboard reads preserve all supported native records, string metadata, image
  bytes, and file URIs. Native update notifications invalidate cached data; queued
  writes complete before async reads. Reading requires the manifest permission,
  an ACL-enabled signing profile, and runtime permission.
- Blurred window backgrounds use an ArkUI backdrop blur layer behind GPUI's
  transparent render surface. Main windows also switch their container color
  so the blur can sample content behind the window (API 20+ requires
  `ohos.permission.SET_WINDOW_TRANSPARENT`). Main and floating windows
  were checked on the 2in1 emulator by cycling blurred, transparent, and opaque.
- Display enumeration tracks connected displays and their usable areas.
  `WindowOptions.display_id` is passed to OpenHarmony for floating sub-windows;
  only the default-display path was tested on 2in1. External-display routing
  needs a multi-display device for validation. `app_path()` reads the bundle
  code directory through a plugin and can report “not ready” during early
  startup before that query finishes.
- Text entry uses the native IME. All eight `TextInputAction` hints are passed
  to the native editor and retained across reattachment. Confirm keys reach
  GPUI key handlers; cursor updates use absolute physical screen coordinates.
  Native insert/preview decoding preserves emoji. Backward and forward deletion
  preserve UTF-16 surrogate pairs. System selection requests reach the input
  handler, and cursor/edit commands reach application key bindings. Text and
  selection changes update a thread-safe snapshot for surrounding-text queries.
  The native selection notification accepts at most 8K UTF-16 units of whole
  text: longer documents provide bounded surrounding text and an absolute cursor
  index, while whole-document selection notifications are omitted. System edit
  commands first use application Ctrl-A/X/C/V bindings. Unhandled commands fall
  back to the input handler and system text clipboard. Cut deletes only after a
  successful copy; queued cut/paste is cancelled by input activity, focus changes,
  or a changed selection/input region. Selection setters and text length are
  required for the generic selection/all-selection path.
  The native IME API exposes no switches for `TextInputConfiguration`
  autocorrection, capitalization, or suggestions. ArkTS API 20 does expose
  capitalization; that separate controller path still needs integration. On API 14+, hardware input uses owned ArkUI
  key snapshots, including held modifiers and system Unicode. Optional API 19
  queries synchronize CapsLock/NumLock even for keys consumed by the IME.
  Unconsumed space/keypad characters have a text insertion path; NumLock-off
  keypad navigation reaches key handlers. The SDK's Unicode API covers basic
  Latin; missing values and physical key equivalents still use a US mapping.
  Handled keys are consumed in their native callback. Color glyphs are selected
  by font tables and raster content, including HarmonyOS emoji fonts. Tapping an
  already focused editor reopens the IME through either touch or desktop mouse
  input after the system dismisses it. The SDK provides no hardware layout
  identifier. The 2in1 emulator accepts native attributes but does not
  show a software keyboard, so visible confirm-key labels need device testing.

## License

[Apache-2.0](./LICENSE-APACHE) or [MIT](./LICENSE-MIT)
