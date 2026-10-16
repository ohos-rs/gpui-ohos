use gpui_ohos::{
    ClipboardEntry, ClipboardItem, ClipboardString, Context, Empty, ExternalDragPayload,
    ExternalPaths, FileDragPaths, Image, ImageFormat, MouseButton, PromptButton, PromptLevel,
    Render, ResizeEdge, Window, div, prelude::*, px, rgb,
};
use openharmony_ability::OpenHarmonyApp;
use openharmony_ability_plugin_permission::PermissionExt as _;
use std::path::PathBuf;

pub struct CapabilitiesDemo {
    app: OpenHarmonyApp,
    status: String,
    saved: Option<Vec<u8>>,
    file: PathBuf,
}
impl CapabilitiesDemo {
    pub fn new(app: OpenHarmonyApp) -> Self {
        let file = PathBuf::from("/data/storage/el2/base/files/gpui-capability-drag.txt");
        let status = match std::fs::write(&file, "GPUI native drag fixture\n") {
            Ok(()) => "ready".into(),
            Err(error) => format!("fixture error: {error}"),
        };
        Self {
            app,
            status,
            saved: None,
            file,
        }
    }
    fn mixed_item(&self) -> ClipboardItem {
        ClipboardItem {
            entries: vec![
                ClipboardEntry::String(ClipboardString {
                    text: "alpha😀".into(),
                    metadata: Some("{\"line\":7}".into()),
                }),
                ClipboardEntry::String(ClipboardString {
                    text: "beta\n".into(),
                    metadata: Some(String::new()),
                }),
                ClipboardEntry::Image(Image::from_bytes(
                    ImageFormat::Png,
                    include_bytes!("fixtures/clipboard.png").to_vec(),
                )),
                ClipboardEntry::ExternalPaths(ExternalPaths(
                    [self.file.clone()].into_iter().collect(),
                )),
            ],
        }
    }
}
impl Render for CapabilitiesDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().gap_2().text_sm()
            .child(format!("Native capabilities: {}", self.status))
            .child(div().flex().flex_wrap().gap_2()
                .child(div().id("native-prompt").p_2().bg(rgb(0x60a5fa)).child("Native prompt")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let result = window.prompt(PromptLevel::Warning, "GPUI native prompt",
                            Some("Choose a button. Cancel and Escape return index 2."),
                            &[PromptButton::ok("First"), PromptButton::new("Second"), PromptButton::cancel("Cancel")], cx);
                        this.status = "prompt pending".into(); cx.notify();
                        cx.spawn(async move |this, cx| {
                            let result = result.await;
                            let _ = this.update(cx, |this, cx| {
                                this.status = format!("prompt result: {result:?}");
                                log::info!("GPUI native capabilities: {}", this.status); cx.notify();
                            });
                        }).detach();
                    })))
                .child(div().id("mixed-clipboard").p_2().bg(rgb(0x2dd4bf)).child("Mixed clipboard")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let app = this.app.clone(); let expected = this.mixed_item();
                        this.status = "clipboard permission".into(); cx.notify();
                        cx.spawn(async move |this, cx| {
                            let permission = app.request_permission("ohos.permission.READ_PASTEBOARD").await;
                            if !matches!(permission, Ok(ref values) if values.iter().all(|value| value.code == 0)) {
                                log::info!("GPUI clipboard permission is unavailable; testing only this application's own write: {permission:?}");
                            }
                            let Ok(read) = this.update(cx, |_, cx| { cx.write_to_clipboard(expected); cx.read_from_clipboard_async() }) else { return; };
                            let result = read.await;
                            let _ = this.update(cx, |this, cx| {
                                this.status = match result {
                                    Ok(Some(item)) => {
                                        let expected = this.mixed_item();
                                        let valid = item.entries.len() == 4
                                            && item.entries[..2] == expected.entries[..2]
                                            && matches!(item.entries[2], ClipboardEntry::Image(ref image) if !image.bytes.is_empty())
                                            && item.entries[3] == expected.entries[3];
                                        format!("mixed clipboard {} ({} records)", if valid { "passed" } else { "FAILED" }, item.entries.len())
                                    }
                                    other => format!("clipboard: {other:?}"),
                                };
                                log::info!("GPUI native capabilities: {}", this.status); cx.notify();
                            });
                        }).detach();
                    })))
                .child(div().id("read-native-clipboard").p_2().bg(rgb(0x2dd4bf)).child("Read clipboard")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let cached = cx.read_from_clipboard().map_or(0, |item| item.entries.len());
                        let read = cx.read_from_clipboard_async();
                        this.status = "reading native clipboard".into(); cx.notify();
                        cx.spawn(async move |this, cx| {
                            let result = read.await;
                            let _ = this.update(cx, |this, cx| {
                                this.status = match result {
                                    Ok(Some(item)) => {
                                        let external_fixture = matches!(item.entries.as_slice(),
                                            [ClipboardEntry::String(text)] if text.text == "gpui-external-😀-20260927" && text.metadata.is_none());
                                        format!("clipboard read: cached={cached}, native={}, external fixture={external_fixture}", item.entries.len())
                                    }
                                    other => format!("clipboard read: {other:?}; cached={cached}"),
                                };
                                log::info!("GPUI native capabilities: {}", this.status); cx.notify();
                            });
                        }).detach();
                    })))
                .child(div().id("window-menu").p_2().bg(rgb(0xfbbf24)).child("Window menu")
                    .on_click(|event, window, _| window.show_window_menu(event.position()))))
            .child(div().flex().flex_wrap().gap_2()
                .child(div().id("save-native-window").p_2().bg(rgb(0xc4b5fd)).child("Save window")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.saved = window.native_window_state();
                        this.status = format!("saved {} bytes; title={}", this.saved.as_ref().map_or(0, Vec::len), window.window_title());
                        log::info!("GPUI native capabilities: {}", this.status); cx.notify();
                    })))
                .child(div().id("restore-native-window").p_2().bg(rgb(0xc4b5fd)).child("Restore window")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(state) = this.saved.as_ref() { window.restore_native_window_state(state); this.status = "restore requested".into(); cx.notify(); }
                    })))
                .child(div().id("system-window-stack").p_2().bg(rgb(0xc4b5fd)).child("Window stack")
                    .on_click(cx.listener(|_this, _, _, cx| {
                        let timer = cx.background_executor().clone();
                        let _ = cx.window_stack();
                        cx.spawn(async move |this, cx| {
                            timer.timer(std::time::Duration::from_millis(200)).await;
                            let _ = this.update(cx, |this, cx| {
                                this.status = format!("system window stack: {:?}", cx.window_stack().map(|stack| stack.iter().map(|handle| handle.window_id()).collect::<Vec<_>>()));
                                log::info!("GPUI native capabilities: {}", this.status); cx.notify();
                            });
                        }).detach();
                    }))))
            .child(div().flex().flex_wrap().gap_2()
                .child(div().id("custom-window-move").p_2().bg(rgb(0xfda4af)).child("Drag to move window")
                    .on_mouse_down(MouseButton::Left, |_, window, cx| { window.start_window_move(); cx.stop_propagation(); }))
                .child(div().id("custom-window-resize").p_2().bg(rgb(0xfda4af)).child("Drag to resize ↘")
                    .on_mouse_down(MouseButton::Left, |_, window, cx| { window.start_window_resize(ResizeEdge::BottomRight); cx.stop_propagation(); }))
                .child(div().id("native-file-source").p_2().bg(rgb(0x86efac)).child("Drag file outside")
                    .on_drag(self.file.clone(), |_, _, _, cx| cx.new(|_| Empty))
                    .external_drag_payload(|path: &PathBuf, _, _| Some(ExternalDragPayload::Files(FileDragPaths::new([(path.clone(), false)]))))))
            .child(div().id("native-file-drop").h(px(36.)).p_2().bg(rgb(0x86efac)).child("Drop files from another window / file manager here")
                .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                    this.status = format!("native drop: {:?}", paths.paths());
                    log::info!("GPUI native capabilities: {}", this.status); cx.stop_propagation(); cx.notify();
                })))
    }
}
