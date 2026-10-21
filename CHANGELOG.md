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

### Changed

- The text host harness uses the root workspace and lockfile. Original font
  fixtures are generated during its build with the pinned fonttools requirement
  instead of being stored as sixteen binary files.
