# OHOS adaptation performance

The OHOS backend manages task execution, frame demand and rendering resources
without changing GPUI core. See [touch input and frame scheduling](touch-input.md)
for gesture ownership and lifecycle details.

## Runtime ownership

- `OhosDispatcher` reuses 2–8 base workers, with a hard limit of 64 live workers.
  If queued work stalls for 10 ms and OS scheduling state shows insufficient
  capacity, a sleeping monitor starts another worker below that limit.
  Observations without growth back off to 100 ms. Dequeues notify only once per
  backed-off wait; progress, an empty queue, or growth resets the interval to
  10 ms. Additional workers retire after 1 second without work. Below the
  limit, this preserves synchronous waits on child tasks and externally queued
  work. The queue uses GPUI's 60/30/10 priority weights and FIFO within each
  priority. Realtime work retains its dedicated thread. Neither the monitor nor
  timer thread polls an empty queue. Shutdown wakes them and releases queued
  tasks.
- One timer thread owns the deadline heap and executes expired timer runnables
  without holding the timer lock, in deadline order and FIFO for equal deadlines.
  Awakened futures resume through their own executors. Foreground tasks execute
  on the UI thread; each UI turn processes up to 64 tasks or 2 ms and posts a
  continuation for remaining work. Scheduling foreground work wakes the UI,
  including when the screen has no animation.
- `MainWake` coalesces foreground task wake events, including `UserEvent` while
  no frame callback is registered. Each window owns an independent `FrameDemand`
  state machine; main and child windows do not share frame demand.
- XComponent is the display frame source. Demand switches
  `set_frame_input_delivery_for` to `FrameInputDelivery::Continuous`, registering
  the window's callback. After 12 consecutive callbacks without demand, it
  switches to `OnDemand` to unregister. Input callbacks stay registered. Startup,
  surface creation/recreation and becoming visible request a first frame.
- Registration and unregistration run only on the UI thread. `FrameCallbackScope`
  covers native redraw delivery across all windows; changes requested inside
  that scope are coalesced into a later foreground task. Native redraw events
  never drain the foreground queue. Each callback draws with `Instant::now()`.
- Failed registration with active demand schedules one fallback draw at least
  16 ms later through the existing dispatcher timer and foreground executor.
  Each window has at most one pending fallback. The draw consumes its demand;
  only new demand retries registration and can schedule another delayed draw.
  Successful registration cancels the fallback. Hiding, closing or destroying
  the surface cancels it as well. Timer tasks and draw targets use weak
  references to avoid retaining a window. Failed unregistration leaves
  continuous callbacks enabled without repeated attempts until surface
  destruction. Each operation warns only once per window.
- After successful idle unregistration, frame callbacks and frame timers stop;
  foreground tasks and input can still wake the application.
- The window index routes borrowed input directly to its owner. Global events
  still reach every live window. `SmallVec` has four inline entries and grows
  on the heap; four is not a window limit. Resize callbacks are deduplicated
  only when full bounds, effective content size and scale are all unchanged.
- The renderer reports presented, retry and unavailable surface outcomes.
  Transient acquisition failures keep presentation demand and request the next
  frame, which presents the latest scene without another layout. Presented and
  unavailable outcomes clear demand; unavailable surfaces wait for input or
  lifecycle events instead of spinning. Initial and lifecycle demand presents
  the latest scene even when GPUI is not dirty.
- Quads use general, solid-fill or borderless-rectangle pipelines selected per
  contiguous run, preserving scene order, clipping, blending and antialiasing.
  Resolved multisample path attachments use `StoreOp::Discard`; sample count
  and resolution are unchanged.
- Quads, shadows and underlines with integral content masks shrink their vertex
  geometry to the mask while preserving shape bounds and fragment clipping.
  Fractional masks retain their original geometry to preserve pixel output.
- The six ordinary scene instance arrays share one `Queue::write_buffer_with`
  allocation and transfer, preserving alignment, strides and primitive order.
  Empty and paths-only scenes skip this transfer; path rasterization uploads
  retain their separate handling.
- Windows allocate path attachments on first path use, reuse them across frames,
  and invalidate them on resize. Windows that never draw paths avoid these
  full-size textures. Allocation occurs on first path use and again after resize;
  path-free frames retain existing attachments.
- `WgpuContext` owns common shader/layout handles and caches pipelines by surface
  format, alpha mode, path sample count and dual-source blending. Window buffers,
  uniforms and surface textures retain their existing ownership; atlas reuse
  uses compatible layouts from the same device. No rendering quality changes.
- Capture copies and converts one row at a time instead of zeroing and rescanning
  a complete frame. RGBA, RGBX, BGRA and BGRX retain their channel/alpha semantics.

## Verification

```sh
export CC_aarch64_unknown_linux_ohos=aarch64-unknown-linux-ohos-clang
export CXX_aarch64_unknown_linux_ohos=aarch64-unknown-linux-ohos-clang++
export AR_aarch64_unknown_linux_ohos=llvm-ar
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER=aarch64-unknown-linux-ohos-clang
cargo clippy --locked --target aarch64-unknown-linux-ohos --all-features --all-targets -- -D warnings
cargo clippy --locked --target aarch64-unknown-linux-ohos --all-targets -- -D warnings
cargo fmt --check
cargo test --manifest-path tests/touch/Cargo.toml --locked
cargo clippy --manifest-path tests/touch/Cargo.toml --locked --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path tests/touch/Cargo.toml --check
(cd example && cargo check --locked --target aarch64-unknown-linux-ohos)
```

Host regressions cover gestures, callback demand and registration failure
recovery, presentation retry and lifecycle demand, unavailable surfaces without
idle retries, viewport deduplication, wake coalescing, timers, blocking background
tasks, worker limits, monitor backoff, priority fairness, shutdown, cache
variants, quad pipeline eligibility and pixel conversion. `FrameDemand`
regressions include fallback coalescing, demand during fallback, successful
registration, unexpected callbacks, hiding and surface destruction.

Host tests and target compilation do not establish native callback safety or
real-device power use. Simulator checks should cover first frames, main/child
window independence, hide/restore, surface recreation, scrolling, registration
failure recovery and continuous-callback degradation after failed unregistration.

Rendering checks are documented in the [path target GPU regression](../tests/path-targets/README.md)
and [shader pixel comparison](../tests/shader-compare/README.md) harnesses.
