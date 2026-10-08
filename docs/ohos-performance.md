# OHOS adaptation performance

The optimization preserves the existing platform APIs, raw touch and ArkUI Pan
streams, system velocity, animation frame rate, presentation mode, antialiasing,
window count, pixel formats and the existing capture size guard. GPUI core is
unchanged at `f45c7c22d0`.

## Runtime ownership

- `OhosDispatcher` reuses 2–8 base workers. This is not a concurrency ceiling:
  if queued work makes no progress for 10 ms, a sleeping monitor starts another
  worker. Additional workers retire after 1 second without work. This preserves
  synchronous waits on child tasks and externally queued work. The queue uses
  GPUI's 60/30/10 priority weights and FIFO within each priority. Realtime work
  retains its dedicated thread. Neither the monitor nor timer thread polls an
  empty queue. Shutdown wakes them and releases queued tasks.
- One timer thread owns the deadline heap. Foreground tasks and expired timers
  execute on the UI thread; each UI turn processes up to 64 tasks or 2 ms and
  posts a continuation for remaining work. Background completions explicitly
  wake the UI, including when the screen has no animation.
- Windows share `MainWake` to coalesce native wake events. Each window retains
  an independent atomic frame state and generation ticket. A callback from an
  earlier surface/visibility generation cannot clear a new request.
- Healthy native VSync uses Ability's `FrameInputDelivery::OnDemand` to remove
  the unused continuous XComponent display callback. All input callbacks remain
  registered. Missing/failed native VSync restores `Continuous`; this was tested
  with forced native VSync creation failure on phone and 2in1. Resume requests
  a fresh frame and retries native VSync after a failed render session.
- The window index routes borrowed input directly to its owner. Global events
  still reach every live window. `SmallVec` has four inline entries and grows
  on the heap; four is not a window limit. Equal effective geometry skips a
  redundant resize callback.
- `WgpuContext` owns common shader/layout handles and caches pipelines by surface
  format, alpha mode, path sample count and dual-source blending. Window buffers,
  uniforms and surface textures retain their existing ownership; atlas reuse
  uses compatible layouts from the same device. No rendering quality changes.
- Capture copies and converts one row at a time instead of zeroing and rescanning
  a complete frame. RGBA, RGBX, BGRA and BGRX retain their channel/alpha semantics.

Ability branch `feat/pr82-nearsend-integration` at `ab1d6c20` supplies frame
delivery selection and releases native consumers before dropping their ArkUI
root. The earlier reverse order caused a reproduced multiwindow close crash.
The SDK contains only OHOS implementations; target-platform fallback branches
were removed without changing the native frame callback body or device classes.

## Verification

```sh
cargo test --manifest-path tests/touch/Cargo.toml --locked
cargo clippy --manifest-path tests/touch/Cargo.toml --locked -- -D warnings
cargo fmt --manifest-path tests/touch/Cargo.toml --check
```

33 regressions cover gestures, frame generations, wake coalescing, timers,
blocking background tasks, priority fairness, shutdown, cache variants and
pixel conversion. ARM64 Release integration built successfully.

The same standalone GPUI example was measured before and after optimization:

| Case | Before | Optimized |
| --- | ---: | ---: |
| Phone idle native redraw events/s | 64.0 | 0 |
| 2in1 idle native redraw events/s | 63.291 | 0 |
| Phone idle CPU, one core | 1.098% | 0 observed ticks |
| 2in1 idle CPU, one core | 1.649% | 0 observed ticks |
| Compatible pipeline sets for main + 3 child windows | 4 | 1 |
| Worker thread IDs for 2,064 cooperative continuations (host) | 2,064 | 8 |
| Median dispatcher duration for that workload (host) | 31.893 ms | 2.616 ms |

Phone and 2in1 both passed foreground/background cycles. 2in1 passed concurrent
child-window creation, independent input, close and reopen, including fallback
mode. CPU samples lasted 9 seconds at procfs 100-tick/second resolution: these
measurements do not establish zero power or a real-device thermal result. Host
timings must not be interpreted as an OHOS speedup. Temporary counters and fault
injection are absent from production source.

[Structured evidence](../tests/touch/evidence/performance.json) records the
measurements and production source hashes.
