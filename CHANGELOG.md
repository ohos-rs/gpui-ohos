# Changelog

## Unreleased

### Breaking

- Fixed-cell terminal shaping now requires the nondefault `grid` Cargo feature.
  This includes `FontCatalog::layout_grid`, `GridLayoutRequest`,
  `GridLayoutError`, and `GridTextCell`. Enable `gpui-ohos/grid` in terminal
  consumers; ordinary GPUI text layout is unchanged.
- `FontCatalog`, `PlatformServices`, `FontFaceData`, `FontDesignCoordinate`, and
  `SyntheticFontStyle` are explicitly experimental APIs whose signatures may
  change between releases. Runtime face identities and registered bytes remain
  immutable.

- File grant persistence is now disabled by default, including through
  `current_platform`. Applications that need cross-restart file access must use
  `try_current_platform` with `PlatformOptions::persist_file_access = true` and
  declare `ohos.permission.FILE_ACCESS_PERSIST`. Struct literals for
  `PlatformOptions` need the new field or `..Default::default()`.
- Restoration starts after the first successfully presented GPUI frame, without
  delaying GPU initialization or the launch callback. Restored workspace I/O must
  tolerate startup before grants are activated.

### Changed

- The text host harness uses the root workspace and lockfile. Original font
  fixtures are generated during its build with the pinned fonttools requirement
  instead of being stored as sixteen binary files.

### Fixed

- Bound file-access history and early selections to 256 grants, history reads to
  2 MiB, URI length to 4096 bytes and permission IPC to batches of 64. Report
  malformed, oversized and full histories without discarding selected paths.
- Request read-only grants for open pickers, incoming file URIs and accepted drops;
  only save pickers request read/write. Preserve successful persistence and report
  partial provider, activation and storage failures.
- Replace Rust dependencies into `example/` with the same original audited
  Ability Git revision. Ability #93, bindings #180 and GPUI core #3 remain
  unmerged dependency blockers; these are not new release pins.
