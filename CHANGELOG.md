# Changelog

## Unreleased

### Breaking

- Align with the native-only clipboard facade: remove HAR constructor options
  and implicit authorization reads; applications must arrange paste access or
  explicitly request `READ_PASTEBOARD` before native reads. Configure the existing
  GPUI metadata MIME in Rust. Clipboard failures are typed, native data IPC runs
  on the background executor, and there is no ArkTS data/observer transport.
- Plain text/HTML cannot contain embedded NUL; empty opaque values, including
  empty custom metadata, are unsupported. Do not replace them with absent values
  or private encodings. API-18 clipboard revisions are pull queries; zero/errors
  invalidate the cache and service restart can decrease the counter.
- Consume Ability's single `ArkUiInputEvent::Key` stream and `ime_consumed`
  disposition. Remove pre/post-IME receipts, timestamp deduplication, Escape
  replay, the Ctrl-Space interception policy and input-callback foreground-task
  pumping. IME attachment transfer now uses Ability's UI-thread owner APIs.
- Add opt-in API-26 `keyboard-text-input` and `keyboard-focus-recovery` features;
  the example enables them. Native key consumption is configured explicitly
  before attachment. Linked API-26 symbols may raise the application's minimum
  runtime requirement.
- File grant persistence is now disabled by default, including through
  `current_platform`. Applications that need cross-restart file access must use
  `try_current_platform` with `PlatformOptions::persist_file_access = true` and
  declare `ohos.permission.FILE_ACCESS_PERSIST`. Struct literals for
  `PlatformOptions` need the new field or `..Default::default()`.
- Restoration starts after the first successfully presented GPUI frame, without
  delaying GPU initialization or the launch callback. Restored workspace I/O must
  tolerate startup before grants are activated.

### Fixed

- Retain clipboard serialization until native completion even if a caller drops
  its write/read task. Invalidate cache after writes instead of attributing a
  potentially interleaved external write's counter to our old item. Preserve
  encoded image bytes and identify their format instead of labeling every image
  PNG; the existing demo now checks exact entries and stops on permission denial.
- Keep native menubar accelerator labels display-only so focused GPUI keymaps
  retain precedence. Existing focus-restoring window-command popups carry no
  accelerator. The IME demo counts an explicit Ctrl-Shift-M context override of
  the same shortcut displayed on the existing Count menu action.
- Bound file-access history and early selections to 256 grants, history reads to
  2 MiB, URI length to 4096 bytes and permission IPC to batches of 64. Report
  malformed, oversized and full histories without discarding selected paths.
- Request read-only grants for open pickers, incoming file URIs and accepted drops;
  only save pickers request read/write. Preserve successful persistence and report
  partial provider, activation and storage failures.
- Remove Rust dependencies into `example/` and align Rust/HAR source to the
  explicitly authorized combined Ability #98 revision
  `a4cfea0ea820b9149997a296744057ea080fbe52` (with `ohos-native-bindings` unified
  at `43aec6c125503a47f7e875c793aaf2462e91ad19`). Ability #93-#98 and GPUI core #3
  remain unmerged release blockers.
