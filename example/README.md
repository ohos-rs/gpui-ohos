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
new native library and declarations into the HAP project. The Ability's `isDesktop`
flag gates the floating window menu bar. Use the default mobile build for a
phone target.

The current local Cargo patches also use the sibling repository
`../../ohos-rs/ohos-native-bindings` (relative to `gpui-ohos`) for the native IME configuration
and cursor APIs. Keep that checkout alongside this repository when building.

## Local verification (2026-09-27)

The updated local signing profile grants `READ_PASTEBOARD`; HAP signing and an
in-place installation on the arm64 2in1 emulator succeeded. The native mixed
pasteboard check read all four records in order: two strings (including an emoji
and both nonempty and empty metadata), a PNG image, and a sandbox file URI.
Metadata uses UTF-8 bytes and a version byte to avoid the SDK's unreadable
zero-byte custom records. Copying `gpui-external-😀-20260927` in the system browser
and pasting it into the GPUI input verified cross-application text reads.


The arm64 2in1 emulator accepted all eight native IME action attributes. Main
and child windows accepted text; Search/Send confirm handling incremented the
demo counter without inserting a newline. Main window and two child windows
registered separate accessibility adapters without a registration collision.
Early decoration requests were retained until the child surface was ready.

The hide/relaunch check restored the main window and visible child A, kept the
previously minimized child B hidden, and triggered one reopen callback.
Native compilation, HAP assembly, IME binding clippy, formatting, and diff
whitespace checks passed. The 2in1 emulator has no full software keyboard;
visible confirm-key labels and candidate placement still need device testing.

The follow-up IME check verified text/selection notifications in the system's
`OnSelectionChange` logs for both main and child windows. In each window, typing
five characters, moving left, and deleting forward produced four characters;
backspace and typing at the current caret also worked. The system emoji panel
inserted an emoji into the child window (two UTF-16 units), and forward deletion
removed the whole emoji. Switching windows retained each input's independent
text. Four `ohos-ime-binding` tests ran on the arm64 emulator: non-BMP decoding,
invalid UTF-16 replacement, surrounding-text boundaries, and empty callback output.
The hardware Ctrl-A/X/C/V path now shares the generic system edit fallback.
On 2in1, copying `hello`, replacing it with `z`, and pasting produced `zhello`;
cutting the full selection emptied the field and pasting restored all six
characters. These operations do not require demo key bindings. System selection
and extended edit callbacks still need interactive coverage with an IME that
sends those callbacks; hardware Ctrl-A does not exercise the native selection callback.

ArkUI keyboard delivery was checked with Ctrl, Shift, CapsLock, NumLock-on
numeric insertion and NumLock-off navigation. IME typing `xyz` produced exactly
three insertions, and an unconsumed keypad digit inserted one character. Three
keyboard regression tests compile for arm64 OHOS, covering modifier recovery,
pre-IME/repeat state, and space/keypad translation. The standalone test binary
cannot run in the emulator's shell namespace because its N-API dependency cannot
load `libark_jsruntime.so`; the HAP UI checks run in the application process.

Main/child clipboard checks also preserved an emoji's two UTF-16 units. Emoji
rendering now uses color-font tables and actual raster content, rather than the
`NotoColorEmoji` font name; the system emoji panel's smile renders in color.
Desktop mouse clicks also reopen an already focused editor after the system
IME/emoji panel is dismissed. Handled native keys are consumed before bubbling
to enclosing ArkUI components.

The final package also verified reopening the same editor after closing the
system emoji panel: the next hardware character arrived through the native IME.
A 12-unit selection containing the emoji was cut to zero and pasted back to
12 units without a selection-update failure. The demo input canvas has explicit
top/left positioning so its reported input bounds match the visible input box.
The ability crate passes strict clippy. Full-platform strict clippy still reports
existing style/type-complexity warnings; native builds and UI checks pass.

The window-focus follow-up fixed native IME reattachment when a focused input
returns from another window. Closing a child and typing into the main window
again retained its original text and accepted the next character. Accessibility
teardown also avoids requesting a refresh for a GPUI window already removed.

To run the native binding tests, in the `../../ohos-rs/ohos-native-bindings` repository (relative to `gpui-ohos`):

```sh
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER="$OHOS_NDK_HOME/llvm/bin/aarch64-unknown-linux-ohos-clang" \
  cargo test -p ohos-ime-binding --lib --target aarch64-unknown-linux-ohos --no-run
```

Copy the executable printed by Cargo to `/data/local/tmp` with
`hdc -t 127.0.0.1:5555 file send`, mark it executable, and run it on the emulator.
`OHOS_NDK_HOME` refers to the SDK's `default/openharmony/native` directory.

## Build the HAP

Initialize the Ability dependency from the repository root:

```sh
git submodule update --init --recursive
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
ohpm install --all
hvigorw assembleHap --mode module -p product=default
```

The signed package is
`ohos/entry/build/default/outputs/default/entry-default-signed.hap`.
