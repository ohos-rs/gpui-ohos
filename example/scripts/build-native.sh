#!/bin/sh
set -eu

example_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$example_dir"

ohrs build --arch arm64 "$@"
mkdir -p ohos/entry/libs/arm64-v8a
cp dist/arm64-v8a/libgpui_demo.so dist/arm64-v8a/libc++_shared.so ohos/entry/libs/arm64-v8a/
sed -e '${/^$/d;}' dist/index.d.ts > ohos/entry/src/main/ets/types/libgpui_demo/Index.d.ts
