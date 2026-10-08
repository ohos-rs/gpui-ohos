# OHOS touch regressions

This harness compiles production touch, frame, dispatcher, priority queue,
worker, viewport, cache, and pixel-conversion modules against the unmodified,
pinned GPUI dependency. Native UI wake is a counter shim. On macOS, a test
adapter reads Mach thread wait state; on OHOS/Linux the production worker-state
reader is used. Host results do not replace simulator integration checks.

The regressions include CPU-only long polls (no worker growth), synchronous
parent/child waits, external work while workers block, viewport publication
across changed geometry and scale, and frame/gesture lifetime boundaries.
See [the native validation report](../../docs/fix-validation-2026-10-07.md) for
phone/2in1 runs and the evaluated FFRT alternative.

```sh
cargo test --manifest-path tests/touch/Cargo.toml --locked
cargo clippy --manifest-path tests/touch/Cargo.toml --locked --all-targets --no-deps -- -D warnings
```
