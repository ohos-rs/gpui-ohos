# Changelog

## Unreleased

### Breaking

- `NativePointerPosition` and `NativeScrollDelta` now require the nondefault
  `native-input` Cargo feature. Their modules and thread-local recording are
  excluded from default builds. Ordinary GPUI input is unchanged by this gate.
- Unknown or nonpositive wheel `scroll_step` no longer fabricates unscaled
  detents: `NativeScrollDelta::get` returns `None`. Generic GPUI line deltas
  remain available.

- `PremultipliedImage::new`, `RasterImage::straight`, and
  `RasterImage::premultiplied` now return `Result<Self, ImageRegistrationError>`.
  Handle the result (for example, with `?`) instead of assuming registration
  succeeds. Conflicts identify the image and both `ImagePolicy` values; the
  existing registration remains unchanged.

### Fixed

- Restore the pre-pointer-scroll native wheel and touchpad signs. Discrete
  wheels continue to use GPUI line units with the system scroll-step coefficient;
  touchpads retain device-pixel scaling. Zero-delta axis updates are ignored
  except for gesture start and end.
- Scroll direction has not been verified on hardware. Mouse-wheel and touchpad
  acceptance, including natural scrolling on and off, remains required.
- Empty image-policy atlas metadata skips GPU sprite copies and per-sprite hash
  lookups. Uploaded tiles retain their policy until removal even after their
  registration's final owner is dropped.
- Matching registrations and clones share lifetime-bound cleanup. Registry locks
  are released before upgraded owners can be dropped.

The private GPU padding-field encoding remains a bounded workaround pending an
upstream `RenderImage` alpha/sampling API; alpha math and default linear filtering
are unchanged.
