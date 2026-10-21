# Changelog

## Unreleased

### Breaking

- `PremultipliedImage::new`, `RasterImage::straight`, and
  `RasterImage::premultiplied` now return `Result<Self, ImageRegistrationError>`.
  Handle the result (for example, with `?`) instead of assuming registration
  succeeds. Conflicts identify the image and both `ImagePolicy` values; the
  existing registration remains unchanged.

### Fixed

- Empty image-policy atlas metadata skips GPU sprite copies and per-sprite hash
  lookups. Uploaded tiles retain their policy until removal even after their
  registration's final owner is dropped.
- Matching registrations and clones share lifetime-bound cleanup. Registry locks
  are released before upgraded owners can be dropped.

The private GPU padding-field encoding remains a bounded workaround pending an
upstream `RenderImage` alpha/sampling API; alpha math and default linear filtering
are unchanged.
