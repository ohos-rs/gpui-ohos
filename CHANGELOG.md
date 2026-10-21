# Changelog

## Unreleased

### Breaking

- Audio playback is now behind the nondefault `audio` Cargo feature.
  `AudioBridgePlugin`, `NativeAudioService`, `NativeAudioPlayer`,
  `NativeAudioPlayerOptions` and `NativeAudioError` are unavailable without it.
  Existing audio consumers must enable `audio` and continue registering the Rust
  facade and matching ArkTS `AudioPlugin` explicitly. Owned-file acknowledgement
  and command ordering are unchanged.
