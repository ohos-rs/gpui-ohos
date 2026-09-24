import { AbilityConstant, Want } from "@kit.AbilityKit";
import { LazyPlugin, NativeAbility } from "@ohos-rs/ability";
import { AppControlPlugin } from "@ohos-rs/ability-plugin-app-control";
import { ClipboardPlugin } from "@ohos-rs/ability-plugin-clipboard";
import { FilesPlugin } from "@ohos-rs/ability-plugin-files";
import { MenuPlugin } from "@ohos-rs/ability-plugin-menu";
import { PermissionPlugin } from "@ohos-rs/ability-plugin-permission";
import { ProcessPlugin } from "@ohos-rs/ability-plugin-process";
import { UrlPlugin } from "@ohos-rs/ability-plugin-url";
import { WindowPlugin } from "@ohos-rs/ability-plugin-window";
import * as nativeModule from "libgpui_demo.so";

export default class EntryAbility extends NativeAbility {
  public moduleName: string = "gpui_demo";
  public defaultPage: boolean = true;
  public bridgePlugins = [
    new LazyPlugin(() => new AppControlPlugin()),
    new LazyPlugin(() => new ClipboardPlugin()),
    new LazyPlugin(() => new FilesPlugin()),
    new LazyPlugin(() => new MenuPlugin()),
    new LazyPlugin(() => new PermissionPlugin()),
    new LazyPlugin(() => new ProcessPlugin()),
    new LazyPlugin(() => new UrlPlugin()),
    new LazyPlugin(() => new WindowPlugin()),
  ];

  override onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    if (typeof nativeModule.init !== "function") {
      throw new Error("GPUI example native module is unavailable");
    }
    super.onCreate(want, launchParam);
  }
}
