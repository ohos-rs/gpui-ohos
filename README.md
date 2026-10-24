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
including the clipboard permission flow and notification response scheme.

### Clipboard integration

The matching clipboard HAR uses `new ClipboardPlugin()` without options. Data
operations use the native Rust facade, not ArkTS data actions. The backend
explicitly configures `application/x-gpui-ohos-text-metadata-v1` in Rust for GPUI
text metadata; readers and writers must use the same custom MIME.

Arrange temporary native paste access or explicitly obtain
`ohos.permission.READ_PASTEBOARD` through the application's permission flow and
signing-profile ACL before requesting a native read. The backend does not prompt
implicitly, does not use the removed authorization action and reports denial
rather than returning an empty success. The example's read controls request
permission and stop if it is declined.

Native data IPC runs on the existing background executor. Reads and writes wait
for native completion; canceling a waiter does not release serialization while
its native operation is still running. Async reads always obtain a native
snapshot rather than using a cached success to bypass authorization.
Synchronous cache lookups validate with the native API-18 change counter.
Zero or query errors invalidate the cache and are reported; counters may decrease
after a clipboard-service restart. There is no clipboard observer/event bridge.

Plain text and HTML must be valid UTF-8 without embedded NUL. Empty plain text
is a present value, but empty opaque/binary representations (including empty
custom metadata) are unsupported and return an explicit error. Encoded image
bytes are not decoded, reencoded or converted through PixelMap; unrecognized
image formats are reported as unsupported. The existing mixed demo uses
nonempty metadata and compares the exact PNG/text/metadata/path entries.

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
| `audio` | Opt-in owned-file audio playback and its Ability plugin dependency. Disabled by default. |
| `grid` | Experimental fixed-cell terminal shaping; disabled by default. |
| `native-input` | Opt in to native pointer coordinates and event-scoped wheel metadata; disabled by default. |
| `api-19` | Enable API-19 CapsLock / NumLock / ScrollLock state snapshots on keyboard events. |
| `api-26` | Enable `api-19` and configure native hardware-key consumption before IME attachment (API 26+). |

Touch scrolling uses native gesture recognition and supports momentum scrolling.
Frames are requested on demand through XComponent callbacks by default.
The `api-19` and `api-26` features are opt-in for the library, and `api-26` is
enabled in the example. Applications enabling them must ship with a compatible
SDK and runtime; runtime configuration is guarded, but linking newer native APIs
can raise the application's minimum supported system version.

Ability delivers each physical key once with its IME disposition. Consumed
transitions update held keys and modifiers without dispatching GPUI commands or
hardware text. Native IME edits retain their text, UTF-16 selection, composition
and confirmation semantics. Timestamp deduplication, Escape replay, a special
Ctrl-Space policy and synchronous executor pumping inside input callbacks are
not part of this contract. Foreground tasks run only on ordered `UserEvent`
wake turns.

Native menubar shortcuts are display-only hints; GPUI's focused key context
owns accelerator dispatch. Window-command popups retain their focus-restoring
transport and carry no accelerator, so they cannot register a native shortcut.

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
surface readiness. IME-consumed deletion and composition are applied through
native edit callbacks; keys declined by the IME reach GPUI once. Clipboard reads
are asynchronous and serialized through native completion, including cancellation.
Picker results retain provider URIs and persisted access policies, and shared
windows track native display density and refresh rate.

Initialize the pinned Ability runtime before building:

```sh
git submodule update --init --recursive
```

Rust dependencies use the authorized coherent Ability integration revision
`b22ac744017d86cb6e7acddfc0f230126c536bff` from existing
[Ability #109](https://github.com/harmony-contrib/openharmony-ability/pull/109);
the example's ArkTS submodule uses that same commit. The library no longer reads
Rust sources from `example/`.
It includes unpublished runtime interfaces; using registry beta.3 for only one
side of that interface is unsupported. The combined source contains the D3
single-delivery input contract and D4 native-only clipboard contract, with
`ohos-native-bindings` unified on `43aec6c125503a47f7e875c793aaf2462e91ad19`.
These are reproducible source dependencies, **not merge-ready release pins**:
[Ability #109](https://github.com/harmony-contrib/openharmony-ability/pull/109)
and [GPUI core #3](https://github.com/ohos-rs/zed/pull/3) remain merge blockers.
Replace these temporary, explicitly authorized integration pins with compatible
published releases after the upstream interfaces land.

Native prompts implement the existing `PlatformWindow::prompt` interface.
Ability resolves their palette from the target window's system theme; no
application color protocol or additional GPUI prompt interface is required.

Ability owns UI-thread IME attachment transfer. Focus/readiness generations
cancel stale retries; no consumer-owned background attachment worker remains.
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


## Pointer and scroll input

Wheel and touchpad deltas preserve the pre-pointer-scroll native sign.
Discrete wheels use GPUI `Lines` with the system scroll-step coefficient;
touchpads use scaled device pixels. Zero-delta axis updates are ignored except
for gesture start and end. Fractional pointer coordinates and simultaneous
mouse-button tracking are preserved.

`NativePointerPosition` and `NativeScrollDelta` require the nondefault
`native-input` Cargo feature. Without it, their modules, thread-local storage and
recording calls are not compiled; ordinary GPUI input remains available.
Unscaled wheel detents are available only when the system reports a positive
`scroll_step`. For an unknown or nonpositive step, `NativeScrollDelta::get`
returns `None`, while ordinary widgets still receive the original line delta.

Hardware wheel/touchpad direction, including natural scrolling on and off, has
not been verified. Host regressions are not hardware acceptance.

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

## Optional file audio

Enable the nondefault `audio` Cargo feature when building this backend, or add
`features = ["audio"]` to the application's existing `gpui-ohos` dependency:

```sh
cargo build --target aarch64-unknown-linux-ohos --features audio
```

Without this feature, the audio module, public audio types and Ability audio
dependency are excluded. Enabling it does not register a plugin automatically.

Register `gpui_ohos::AudioBridgePlugin` on the application's `OpenHarmonyApp` and
add the matching `AudioPlugin` factory from `@ohos-rs/ability-plugin-audio` to its
ArkTS entry. Then construct `NativeAudioService::new(app, foreground_executor)`.
It rejects an unregistered Rust facade instead of silently adding a required HAR
to every existing application. Players accept owned `File` values, serialize
commands and retain each file until native descriptor duplication completes.
Dropping a player queues release behind its pending playback updates.

This service is a consumer convenience, not part of GPUI's platform interface.
Applications such as ohostty should eventually own this integration directly.

Host checks (the audio bridge is a shim; the command queue and service are real):

```sh
cargo test --manifest-path tests/touch/Cargo.toml --no-default-features
cargo test --manifest-path tests/touch/Cargo.toml --no-default-features --features audio
```

These checks do not establish native playback or descriptor-duplication acceptance.

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
