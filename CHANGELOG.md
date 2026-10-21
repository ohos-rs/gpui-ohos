# Changelog

## Unreleased

### Breaking

- `NativePointerPosition` and `NativeScrollDelta` now require the nondefault
  `native-input` Cargo feature. Their modules and thread-local recording are
  excluded from default builds. Ordinary GPUI input is unchanged by this gate.
- Unknown or nonpositive wheel `scroll_step` no longer fabricates unscaled
  detents: `NativeScrollDelta::get` returns `None`. Generic GPUI line deltas
  remain available.

- Restore the pre-pointer-scroll native wheel and touchpad signs. Discrete
  wheels continue to use GPUI line units with the system scroll-step coefficient;
  touchpads retain device-pixel scaling. Zero-delta axis updates are ignored
  except for gesture start and end.
- Scroll direction has not been verified on hardware. Mouse-wheel and touchpad
  acceptance, including natural scrolling on and off, remains required.
