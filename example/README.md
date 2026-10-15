# gpui-ohos example

This example is the test app for GPUI's OpenHarmony platform support. The
current screen exposes these checks. The main screen and child windows scroll
vertically when their controls exceed the window height:

- **Credential storage** writes a credential, updates it, reads it back, then deletes it
  using the native AssetStore binding.
- **IME input** accepts real text and cycles all eight GPUI confirm-key hints.
  Native logs report whether the editor accepted each hint;
  pressing the confirm key increments the counter.
- **Screen capture** starts and stops a display capture stream and shows the
  number of video frames received.
- **Child windows** open three independent GPUI windows (A, B, and C). Each
  has its own click counter, GPUI maximize/fullscreen/visibility state check,
  clipboard write/read check, and system file picker. Closing one updates only
  its status in the main window; it can then be opened again. Window A requests
  the default display explicitly to exercise `WindowOptions.display_id`.
- **Menu** shows a GPUI app menu. Choosing its action increments the counter in
  every open window. Opening the menu refreshes action availability; the
  unhandled action is shown disabled.
- **Appearance** cycles dark, light, and system colors through
  `set_window_appearance` and displays the value reported by GPUI.
- **Window background** cycles blurred, transparent, and opaque on the main
  window and child windows. Window A starts blurred; its transparent areas
  show the desktop with a backdrop blur on the 2in1 emulator.
- **System state** displays the thermal state and logs sleep, wake, and thermal
  change callbacks. On a 2in1 emulator, `power-shell suspend` and
  `power-shell wakeup` exercise the sleep and wake callbacks.
- **Display and bundle paths** count connected displays, read GPUI's full and
  usable display bounds, and show the installed bundle code directory. On 2in1,
  the usable area excludes the system dock; display and usable-area changes are
  delivered through the system-state plugin.
- **URL schemes** verifies that `gpui-demo` is declared for this Ability in the
  installed `module.json5` and rejects an absent scheme. External
  `gpui-demo://...` links are logged by GPUI's `on_open_urls` callback. OpenHarmony
  requires schemes to be declared before installation; `register_url_scheme`
  validates the installed declaration rather than changing it at runtime.
- **Window controls** report the main window's available controls, toggle its
  maximized state, and request system attention through a notification.
- **Frame and viewport** shows the layout size and visible viewport, and counts
  callbacks scheduled for the next system VSync. Keyboard occlusion changes
  the visible viewport without changing the layout size.
- **Notifications** publishes a system notification with an action button,
  reports body and action activations, and can dismiss it. The first publish
  may ask for notification permission. A notification tap also restores a
  stopped app and delivers its response after startup.
- **App visibility** minimizes the app windows. Launch the Ability again to
  restore them and verify the reopen callback counter.
- **Quit** exercises GPUI's shutdown callback before terminating the process.

Clipboard read requires `ohos.permission.READ_PASTEBOARD` in the module manifest
and a signing profile whose ACL grants that permission. The example now declares
it and requests runtime permission before the mixed-record check. Use the full
local signing configuration in `build-profile.json5`; profiles without the ACL
cannot validate native clipboard reads.

## Build the native module

From this directory:

```sh
sh scripts/build-native.sh
```

For a desktop or 2in1 target, build the native module with
`OHOS_DEVICE_TYPE=desktop sh scripts/build-native.sh`. This also copies the
new native library and declarations into the HAP project. Ability `1.0.0-beta.3`
provides floating native windows with a menu host on both device classes. Use
the default mobile build for a phone target.

## Build the HAP

Rust Ability dependencies use the published `=1.0.0-beta.3` crates. Until the
HAR packages are approved, the ArkTS dependencies use `1.0.0-beta.3` sources
from Ability's `docs/complete-beta3-changelogs` branch, pinned at `93b3e22`.
Initialize the submodule from the repository root:

```sh
git submodule sync --recursive
git submodule update --init --recursive
```

`EntryAbility.ets` explicitly configures the clipboard metadata MIME and the
`gpui-notification` response scheme required by the plugins.
The source packages use `workspace:*` dependencies, so install them with
`ohpm-rs` from `ohpm-cli` 0.3.0 or later (the upstream Ability workspace uses
the same tool):

```sh
cargo install ohpm-cli --version 0.3.0 --locked
```

Copy the local project files and configure a debug signing certificate for the
bundle name in `AppScope/app.json5`. These two local files are ignored by Git:

```sh
cd example/ohos
cp AppScope/app.template.json5 AppScope/app.json5
cp build-profile.template.json5 build-profile.json5
```

You can configure signing in DevEco Studio, or add a `signingConfigs` entry and
its product `signingConfig` in `build-profile.json5`. Then build with DevEco
Studio or its command-line tools:

```sh
ohpm-rs install --all
hvigorw assembleHap --mode module -p product=default
```

The signed package is
`ohos/entry/build/default/outputs/default/entry-default-signed.hap`.
