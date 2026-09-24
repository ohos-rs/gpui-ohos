# gpui-ohos example

This example is the test app for GPUI's OpenHarmony platform support. The
current screen exposes these checks:

- **Credential storage** writes a credential, updates it, reads it back, then deletes it
  using the native AssetStore binding.
- **Screen capture** starts and stops a display capture stream and shows the
  number of video frames received.
- **Child windows** open three independent GPUI windows (A, B, and C). Each
  has its own click counter, GPUI maximize/fullscreen/visibility state check,
  clipboard write/read check, and system file picker. Closing one updates only
  its status in the main window; it can then be opened again.
- **Menu** shows a GPUI app menu. Choosing its action increments the counter in
  every open window.

Clipboard read requires `ohos.permission.READ_PASTEBOARD` in the module manifest
and a signing profile whose ACL grants that permission. The example does not
request it by default, so it remains installable with debug profiles that lack
the ACL. The clipboard check requests runtime permission and reports denial
when the manifest or signing ACL does not grant access.

## Build the native module

From this directory:

```sh
ohrs build --arch arm64
mkdir -p ohos/entry/libs/arm64-v8a
cp dist/arm64-v8a/libgpui_demo.so dist/arm64-v8a/libc++_shared.so ohos/entry/libs/arm64-v8a/
cp dist/index.d.ts ohos/entry/src/main/ets/types/libgpui_demo/Index.d.ts
```

For a desktop or 2in1 target, build the native module with
`OHOS_DEVICE_TYPE=desktop ohrs build --arch arm64`. The Ability's `isDesktop`
flag gates the floating window menu bar. Use the default mobile build for a
phone target.

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
