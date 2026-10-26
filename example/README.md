# gpui-ohos example

An OpenHarmony app demonstrating GPUI windows, scrolling, text input, menus,
clipboard, file pickers, credentials, screen capture, notifications, and
application lifecycle.

## Build the native module

Run from the `example` directory with Rust, `ohrs`, and the OpenHarmony SDK
configured:

```sh
sh scripts/build-native.sh --release
```

The script copies the native library and declarations into the HAP project.
The example's default keyboard features and build-profile template require an
API-26 SDK and runtime. The library keeps those features opt-in; disabling them
does not establish that the rest of the example supports an older system.
Phone targets use the default configuration. For desktop or 2in1 targets:

```sh
OHOS_DEVICE_TYPE=desktop sh scripts/build-native.sh --release
```

To build with NativeVSync instead of XComponent callbacks:

```sh
sh scripts/build-native.sh --release -- --locked --features native-vsync
```

The app displays the frame source selected at build time. See
[NativeVSync delivery](../README.md#optional-nativevsync-delivery) for details.

## Contextual menu shortcut check

The existing Demo menu's Count action displays **Ctrl-Shift-M**. Clicking it
increments the visible menu-action count and logs `GPUI menu action count`.
Focus the IME input and type that chord: its `ImeDemo` key context should instead
increment `focused Ctrl-Shift-M` and log `GPUI focused menu chord count`, without
incrementing the menu count. Repeat a press/release to check exactly one increment
per press. This is a manual acceptance target, not a claim that hardware,
composition or terminal shortcut behavior has been verified.

## Build the HAP

From the `example` directory, copy the local project templates if they do not
already exist. Both destination files are ignored by Git:

```sh
cd ohos
cp -n AppScope/app.template.json5 AppScope/app.json5
cp -n build-profile.template.json5 build-profile.json5
```

For an existing local profile, set both `targetSdkVersion` and
`compatibleSdkVersion` to `"26.0.0"` for the default native build; copying with
`-n` does not update an older profile. Do not package the API-26 native module
with the interim `"6.0.0(20)"` compatibility declaration.

Configure signing for the bundle name in `AppScope/app.json5` through DevEco
Studio, or set `signingConfigs` and the product's `signingConfig` in
`build-profile.json5`. Then build with DevEco Studio or its command-line tools:

```sh
cargo install ohpm-cli --version 0.3.0 --locked
ohpm-rs install --all
hvigorw assembleHap --mode module -p product=default
```

The signed package is `entry/build/default/outputs/default/entry-default-signed.hap`
relative to the `ohos` directory.

## Permissions

Clipboard reads require `ohos.permission.READ_PASTEBOARD` in the module manifest,
a signing profile whose ACL grants it, and runtime permission. The example
declares and requests the permission; the local signing profile must provide
the matching ACL. Notifications and screen capture may also prompt for system
authorization.

The clipboard HAR factory takes no constructor options. The GPUI backend
configures its metadata MIME in Rust. Clipboard buttons explicitly request read
permission and report denial without continuing with an own-write fallback.
The existing mixed check compares exact text, nonempty metadata, PNG bytes and
file paths. Empty metadata is an unsupported empty binary representation;
embedded-NUL plain text is rejected rather than lossy-converted. This demo is
not a terminal or arbitrary binary-MIME acceptance fixture.

The source packages use `workspace:*` dependencies, which require `ohpm-rs` for
installation. The native and ArkTS dependencies use the same pinned Ability
submodule. Run
`git submodule update --init --recursive` before building either side. The native
render exports receive the actual native window ID from this matching runtime.
