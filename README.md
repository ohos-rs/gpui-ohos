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

## Cursor grab

`gpui_ohos::set_cursor_grab(real_window_id, grab)` exposes the native OHOS
cursor lock through `openharmony-ability` and `ohos-window-manager-binding`.
Pass the **real OHOS window ID** from `getWindowProperties().id`, not the
framework's logical window ID (for example, `0` for the main window). The
`openharmony-ability-plugin-window` `get_real_window_id` action can resolve a
logical ID when that plugin is registered.

Cursor locking requires API 22 or newer and the
`ohos.permission.LOCK_WINDOW_CURSOR` permission. The function returns
`CursorGrabError::NotSupported` on older devices. Passing `false` releases the
lock; the system also releases it when the window loses focus.

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
