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
Phone targets use the default configuration. For desktop or 2in1 targets:

```sh
OHOS_DEVICE_TYPE=desktop sh scripts/build-native.sh --release
```

To enable the runtime switch between NativeVSync and XComponent callbacks:

```sh
sh scripts/build-native.sh --release -- --locked --features native-vsync
```

See [frame source configuration](../README.md#optional-nativevsync-delivery)
for the embedding API.

## Build the HAP

From the `example` directory, copy the local project templates if they do not
already exist. Both destination files are ignored by Git:

```sh
cd ohos
cp -n AppScope/app.template.json5 AppScope/app.json5
cp -n build-profile.template.json5 build-profile.json5
```

Configure signing for the bundle name in `AppScope/app.json5` through DevEco
Studio, or set `signingConfigs` and the product's `signingConfig` in
`build-profile.json5`. Then build with DevEco Studio or its command-line tools:

```sh
ohpm install --all
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
