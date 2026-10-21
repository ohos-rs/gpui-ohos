# Changelog

## Unreleased

- Restore the pre-pointer-scroll native wheel and touchpad signs. Discrete
  wheels continue to use GPUI line units with the system scroll-step coefficient;
  touchpads retain device-pixel scaling. Zero-delta axis updates are ignored
  except for gesture start and end.
- Scroll direction has not been verified on hardware. Mouse-wheel and touchpad
  acceptance, including natural scrolling on and off, remains required.
