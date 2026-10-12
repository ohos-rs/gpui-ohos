# Frame pacing and quad rendering

Scrolling on a 120 Hz display has about 8.33 ms per frame. NativeVSync can
supply that cadence while delayed main-queue delivery or GPU work still makes
the displayed animation uneven. These changes keep the existing NativeVSync
scheduler, synchronous GLES FIFO presentation, and unmodified wgpu dependency.

## Frame clock and presentation demand

`frame_clock.rs` calibrates NativeVSync CLOCK_MONOTONIC timestamps to the
Instant clock used for input dispatch. The native callback publishes the frame
time together with its request ticket. The main queue consumes the latest
pending tick instead of replaying missed ticks; late or duplicate callbacks
cannot complete another request. Visibility changes clear pending frame demand
and time. Momentum retains its release-dispatch origin and advances using the
delivered frame clock, so queue delay is not added to animation time.

The renderer reports submitted, retry, and unavailable surface outcomes.
Transient acquisition failures retain presentation demand and request the next
VSync after GPUI clears its own flag. A retry draws the latest GPUI scene without
forcing another render/layout. Unavailable surfaces wait for input or lifecycle
events instead of spinning. XComponent frame delivery remains the fallback
when NativeVSync fails; rendering still runs on the UI thread.

## Quad and path rendering

The renderer selects general, solid-fill, or borderless rectangular quad
pipelines. Pure fills omit gradient/pattern and dashed-border branches;
borderless rectangles also omit rounded-corner and border distance calculations.
`quad_pipeline.rs` checks every corner and border. Contiguous runs preserve
scene order, clipping, blending, and antialiasing.

Resolved multisample path attachments use `StoreOp::Discard` while retaining
the resolved single-sample image. The attachment is cleared on its next use.
Sampling count and rendering resolution are unchanged.

## Validation

The host harness includes clock calibration, coalesced ticks, stale callbacks,
momentum, and specialized pipeline eligibility alongside existing lifecycle,
input, and dispatcher regressions:

```sh
cargo test --manifest-path tests/touch/Cargo.toml --locked
cargo clippy --manifest-path tests/touch/Cargo.toml --locked --all-targets --no-deps -- -D warnings
```

The same adapter changes were built in NearSend 1.2.2 Release with a frozen
build environment based on gpui-ohos `58e6b6b`. On the connected 120 Hz phone,
994 instrumented GLES synchronous draw samples had a median producer interval
of 8.283 ms and p95 of 8.671 ms. The clean build's gpui-fps Observed mode showed
119–121 FPS while scrolling settings. These observations cover that app/device,
not every workload or zero dropped frames; producer timestamps do not measure
panel scanout. The FPS HUD is an optional NearSend feature, not part of this
adapter change.
