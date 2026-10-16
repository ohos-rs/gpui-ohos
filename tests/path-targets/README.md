# Path target GPU regression

This standalone macOS/Metal harness compiles the production `PathTargets` owner.
It checks texture lifetimes after constructor handles are dropped, attachment
clear/resolve, bind-group sampling, reuse and resize with both 1× and 4× sampling.
Every output pixel is read back and compared with its expected color.

```sh
cargo test --manifest-path tests/path-targets/Cargo.toml --locked
cargo clippy --manifest-path tests/path-targets/Cargo.toml --locked --all-targets -- -D warnings
cargo fmt --manifest-path tests/path-targets/Cargo.toml --check
```

This requires a Metal adapter. Real-device GLES checks are still needed for
OHOS integration.
