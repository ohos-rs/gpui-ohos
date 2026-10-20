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
`8d66b703f9044a966eaedd720a61ed797049982d` from `openharmony-ability` `main`
([Ability #109](https://github.com/harmony-contrib/openharmony-ability/pull/109) +
non-activating Ability window launch focus restoration);
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

```sh
cargo check --target aarch64-unknown-linux-ohos
```

Creation-time input is buffered
until `PlatformWindow::on_input` is registered, and vetoed close requests restore
window visibility and focus for task-center confirmation prompts. Enforced
provider permission behavior still requires device verification.


## Shared font services

Embedders that need exact face bytes, synthetic styles or fixed-cell text can use
`PlatformServices::try_new(app, PlatformOptions::default())`. Its `fonts` catalog
shares immutable font IDs and face data with the returned GPUI platform. Ordinary
UI text retains the platform's locale-aware fallback and bidirectional shaping.
`layout_grid` is an explicit fixed-cell path; it does not replace document layout.
Named instances preserve every original 16.16 axis coordinate, including width-only
fonts. Explicit variation overrides retain the instance's other axes.
