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

### Optional persistent file access

File selections work with the provider's temporary grants by default. Persisting
picker, incoming open-URI and accepted drop grants is application policy and must
be explicitly enabled:

```rust
let platform = gpui_ohos::try_current_platform(
    openharmony_app,
    gpui_ohos::PlatformOptions {
        persist_file_access: true,
        ..Default::default()
    },
)?;
```

Declare `ohos.permission.FILE_ACCESS_PERSIST` only if using this option. The
backend does not read history or call permission persistence APIs when disabled.
Restoration and queued early selections run on a background executor only after
the first successfully presented GPUI frame, never during GPU initialization.
There is no startup barrier: applications must tolerate unavailable restored
files during their initial UI and let users reselect a path if needed.

History and the pre-frame queue each hold at most 256 distinct provider URIs;
policy IPC uses batches of at most 64. History reads are limited to 2 MiB and each
URI to 4096 bytes. Oversized or malformed history is reported and preserved,
not silently truncated. A full history keeps existing grants and reports that
additional selections cannot be retained; it never truncates the returned paths
or revokes provider grants. Revoked grants are removed on restoration.

Open pickers, incoming URIs and drops retain read access; save pickers explicitly
request read/write access. Provider rejection, read-only fallback, activation and
history-write failures are reported, while selected paths and successful grants
are kept. Saved grants remain available for a later activation retry. Disabling
persistence does not revoke grants already stored by the OS or delete history.

## Features

| Cargo feature | Purpose |
| --- | --- |
| `gles` | OpenGL ES rendering, enabled by default. |
| `vulkan` | Vulkan rendering. |
| `native-vsync` | Use NativeVSync frame callbacks instead of XComponent callbacks. |
| `grid` | Experimental fixed-cell terminal shaping; disabled by default. |

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

Rust dependencies use the original audited Ability Git revision `05ce9195ea66876dc64a9481a76f03abb2ea20db`;
the example's ArkTS submodule uses that same commit. The library no longer reads
Rust sources from `example/`.
It includes unpublished runtime interfaces; using registry beta.3 for only one
side of that interface is unsupported. The IME binding dependency is pinned by
Ability to the original exact Git commit `842f4d4105b6c34a7c108d29c88d17cb496ed912`.
These are reproducible source dependencies, **not merge-ready release pins**:
[Ability #93](https://github.com/harmony-contrib/openharmony-ability/pull/93),
[bindings #180](https://github.com/ohos-rs/ohos-native-bindings/pull/180), and
[GPUI core #3](https://github.com/ohos-rs/zed/pull/3) remain merge blockers.
No new draft revision is introduced. Replace these pins with compatible
published releases after the upstream interfaces land.

Applications can set `App::set_native_prompt_style` to provide semantic dialog
colors from their current theme. Native prompts convert GPUI RGBA colors to the
Ability's ARGB contract. Omitting the style uses system colors; Ability handles
alpha compositing and readable text/focus contrast without application-specific
theme knowledge.

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
cargo test --manifest-path tests/file-access/Cargo.toml
cargo check --target aarch64-unknown-linux-ohos
```

Host checks substitute native window/frame APIs. Native creation-time input
routing, task-center close confirmation and enforced provider permission behavior
still need further work; this does not establish complete desktop parity.


## Shared font services

Embedders that need exact face bytes or synthetic styles can use
`PlatformServices::try_new(app, PlatformOptions::default())`. Its `fonts` catalog
shares immutable font IDs and face data with the returned GPUI platform. Ordinary
UI text retains the platform's locale-aware fallback and bidirectional shaping.
The catalog, `PlatformServices`, face data, design coordinates and synthetic-style
APIs are **experimental** and may change between releases. Runtime face identities
and the exact registered bytes remain immutable.

Terminal consumers must explicitly enable the nondefault `grid` Cargo feature to
use `GridLayoutRequest`, `GridTextCell`, `GridLayoutError` and
`FontCatalog::layout_grid`. The grid module and its shaper state are absent without
that feature; ordinary document layout does not use it.
Named instances preserve every original 16.16 axis coordinate, including width-only
fonts. Explicit variation overrides retain the instance's other axes.

Host font regressions use the root workspace and lockfile:

```sh
python3 -m pip install -r tests/text/requirements.txt
cargo test -p ohos-text-tests --locked
cargo test -p ohos-text-tests --locked --features grid
```

The host harness generates its original font fixtures in Cargo's build output;
no binary fonts or separate text-test lockfile are committed. See
[the test harness](./tests/text/README.md) for coverage and Python selection.

## Explicit image alpha and sampling

Ordinary GPUI images keep straight alpha and linear filtering. Register externally
rasterized images before their first atlas upload:

```rust
let ui_image = gpui_ohos::PremultipliedImage::new(premultiplied_pixels)?;
let glyph_image = gpui_ohos::RasterImage::straight(straight_pixels)?;
let color_glyph = gpui_ohos::RasterImage::premultiplied(color_pixels)?;
```

Each constructor returns `Result<_, ImageRegistrationError>`. A conflicting live
registration returns the image ID and existing/requested `ImagePolicy` without
panicking or changing the existing owner. Matching registrations share ownership;
keep an owner alive while caching or painting its image. The last owner removes
the live registration, but uploaded tiles retain their original policy until
evicted. To change a policy, release every owner and evict the old atlas tiles
before registering and uploading the image again.

Empty atlas metadata borrows scene sprites directly, without copying them or
performing per-sprite hash lookups. The backend still temporarily carries nondefault
flags in GPU copies of `PolychromeSprite.pad`; this bounded workaround needs an
upstream `RenderImage` alpha/sampling API before it can be removed. It does not
change GPUI's scene ABI or the ordinary linear UI sampling path.
