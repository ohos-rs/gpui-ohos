# Host renderer tests

Run `cargo test --manifest-path tests/renderer/Cargo.toml` on a host with a
Metal or Vulkan GPU. This separate, unpublished harness compiles production
OHOS rendering modules directly; it avoids linking OHOS N-API on the host.
The Fast core is the unmodified official 0.1.13 release.

The pixel fixtures are adapted from gpui-fast-wgpu 0.1.13 and use its Apache
license and notices in `src/ohos/fast/`. GPU tests compare tile output with
direct drawing and exercise actual GPUI rounded-overflow scenes, including
paths and clips within tiles. Pure cache tests cover dirty generations and
memory-budget eviction. OHOS cross-compilation and device tests remain
necessary to validate GLES and native surfaces.
