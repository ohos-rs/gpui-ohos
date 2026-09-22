# gpui-ohos minimal example

This native module follows NearSend's OpenHarmony integration pattern while
keeping the GPUI application intentionally small. It opens one window and
renders a centered greeting without component libraries or application state.

Build it from the repository root:

```bash
cd example
ohrs build --arch aarch
```

The generated native module is
`dist/arm64-v8a/libgpui_ohos_example.so`. Copy it (and the adjacent
`libc++_shared.so`) into the application's native library directory. Configure
the Ability package with `@ohos-rs/ability` and
`@ohos-rs/ability-plugin-app-control`, then use the same module name on both
the Ability and page sides:

```ts
// EntryAbility.ets
import { LazyPlugin, NativeAbility } from "@ohos-rs/ability";
import { AppControlPlugin } from "@ohos-rs/ability-plugin-app-control";

export default class EntryAbility extends NativeAbility {
  public moduleName: string = "gpui_ohos_example";
  public defaultPage: boolean = true;
  public bridgePlugins = [new LazyPlugin(() => new AppControlPlugin())];
}
```

```ts
// pages/Index.ets
import { DefaultXComponent } from "@ohos-rs/ability";

@Entry
@Component
struct Index {
  build() {
    DefaultXComponent({ moduleName: "gpui_ohos_example" })
      .height("100%")
      .width("100%");
  }
}
```
