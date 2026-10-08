use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use gpui_ohos::{
    App, AppContext, Application, ApplicationHandle, Bounds, ClipboardItem, Context, IntoElement,
    Menu, MenuItem, PathPromptOptions, Render, SystemNotification, SystemNotificationAction,
    WeakEntity, Window, WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowId,
    WindowOptions, actions, div, point, prelude::*, px, rgb, rgba, size,
};
use log::LevelFilter;
use ohos_hilog_binding::log::Config;
use openharmony_ability::OpenHarmonyApp;
use openharmony_ability_plugin_permission::{PermissionBridgePlugin, PermissionExt as _};

mod capabilities_demo;
mod ime_demo;

thread_local! {
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

actions!(gpui_ohos_example, [CountMenuAction, UnavailableMenuAction]);

const CHILD_WINDOW_LABELS: [&str; 3] = ["A", "B", "C"];

struct ChildWindowState {
    id: Option<WindowId>,
    status: String,
    opens: u32,
}

impl ChildWindowState {
    fn new() -> Self {
        Self {
            id: None,
            status: "idle".into(),
            opens: 0,
        }
    }
}

struct CaptureDemo {
    capabilities: gpui_ohos::Entity<capabilities_demo::CapabilitiesDemo>,
    ime: gpui_ohos::Entity<ime_demo::ImeDemo>,
    app: OpenHarmonyApp,
    status: String,
    credentials: String,
    appearance_step: usize,
    appearance_status: String,
    background_step: usize,
    background_appearance: WindowBackgroundAppearance,
    app_path_status: String,
    url_scheme_status: String,
    display_status: String,
    reopens: Arc<AtomicUsize>,
    notification_responses: Arc<AtomicUsize>,
    next_frame_callbacks: Arc<AtomicUsize>,
    child_windows: [ChildWindowState; CHILD_WINDOW_LABELS.len()],
    menu_actions: Arc<AtomicUsize>,
    frames: Arc<AtomicUsize>,
    stream: Option<Box<dyn gpui_ohos::ScreenCaptureStream>>,
}

struct ChildWindowDemo {
    capabilities: gpui_ohos::Entity<capabilities_demo::CapabilitiesDemo>,
    ime: gpui_ohos::Entity<ime_demo::ImeDemo>,
    app: OpenHarmonyApp,
    label: &'static str,
    clicks: u32,
    clipboard: String,
    picker: String,
    window_state: String,
    background_step: usize,
    menu_actions: Arc<AtomicUsize>,
}

impl Render for ChildWindowDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .id(format!("child-scroll-{}", self.label))
            .overflow_y_scroll()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .text_color(rgb(0x18181b))
            .child(format!("GPUI child window {}", self.label))
            .child(self.capabilities.clone())
            .child(self.ime.clone())
            .child(format!(
                "Menu actions: {}",
                self.menu_actions.load(Ordering::Relaxed)
            ))
            .child(format!("GPUI window state: {}", self.window_state))
            .child(
                div()
                    .id("cycle-window-background")
                    .p_4()
                    .bg(rgb(0x67e8f9))
                    .child("Cycle blur / transparent / opaque background")
                    .on_click(cx.listener(|this, _event, window, cx| {
                        let appearance = match this.background_step % 3 {
                            0 => WindowBackgroundAppearance::Transparent,
                            1 => WindowBackgroundAppearance::Opaque,
                            _ => WindowBackgroundAppearance::Blurred,
                        };
                        this.background_step += 1;
                        window.set_background_appearance(appearance);
                        this.window_state = format!("background requested: {appearance:?}");
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("check-window-state")
                    .p_4()
                    .bg(rgb(0x64748b))
                    .child("Read GPUI window state")
                    .on_click(cx.listener(|this, _event, window, cx| {
                        this.window_state = format!(
                            "maximized={}, fullscreen={}, visibility={:?}",
                            window.is_maximized(),
                            window.is_fullscreen(),
                            window.visibility()
                        );
                        log::info!(
                            "GPUI child window {} state: {}",
                            this.label,
                            this.window_state
                        );
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("second-window-click")
                    .p_4()
                    .bg(rgb(0x2563eb))
                    .child(format!("Clicks: {}", self.clicks))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.clicks += 1;
                        log::info!(
                            "GPUI child window {} click count: {}",
                            this.label,
                            this.clicks
                        );
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("second-window-clipboard")
                    .p_4()
                    .bg(rgb(0x0d9488))
                    .child(format!("Clipboard: {}", self.clipboard))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        const VALUE: &str = "gpui-ohos second window";
                        let app = this.app.clone();
                        this.clipboard = "requesting permission".into();
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            match app
                                .request_permission("ohos.permission.READ_PASTEBOARD")
                                .await
                            {
                                Ok(codes) if codes.iter().all(|item| item.code == 0) => {}
                                other => {
                                    let _ = this.update(cx, |this, cx| {
                                        this.clipboard =
                                            format!("permission unavailable: {other:?}");
                                        cx.notify();
                                    });
                                    return;
                                }
                            }
                            let Ok(read) = this.update(cx, |this, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(VALUE.into()));
                                this.clipboard = "reading".into();
                                cx.notify();
                                cx.read_from_clipboard_async()
                            }) else {
                                return;
                            };
                            let result = read.await;
                            let _ = this.update(cx, |this, cx| {
                                this.clipboard = match result {
                                    Ok(Some(item)) if item.text().as_deref() == Some(VALUE) => {
                                        "round trip passed".into()
                                    }
                                    other => format!("unexpected: {other:?}"),
                                };
                                cx.notify();
                            });
                        })
                        .detach();
                    })),
            )
            .child(
                div()
                    .id("second-window-picker")
                    .p_4()
                    .bg(rgb(0x7c3aed))
                    .child(format!("File picker: {}", self.picker))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let dialog = cx.prompt_for_paths(PathPromptOptions {
                            files: true,
                            directories: false,
                            multiple: false,
                            prompt: None,
                        });
                        this.picker = "opening".into();
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            let result = dialog.await;
                            let _ = this.update(cx, |this, cx| {
                                this.picker = format!("{result:?}");
                                cx.notify();
                            });
                        })
                        .detach();
                    })),
            )
    }
}

impl CaptureDemo {
    fn open_child_window(&mut self, index: usize, cx: &mut Context<Self>) {
        let state = &mut self.child_windows[index];
        if state.id.is_some() {
            return;
        }

        let app = self.app.clone();
        let label = CHILD_WINDOW_LABELS[index];
        let menu_actions = self.menu_actions.clone();
        let display_id = if index == 0 {
            cx.primary_display().map(|display| display.id())
        } else {
            None
        };
        let result = cx.open_window(
            WindowOptions {
                display_id,
                window_background: if index == 0 {
                    WindowBackgroundAppearance::Blurred
                } else {
                    WindowBackgroundAppearance::Opaque
                },
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(20. + index as f32 * 340.), px(60.)),
                    size(px(320.), px(360.)),
                ))),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|cx| ChildWindowDemo {
                    ime: cx.new(ime_demo::ImeDemo::new),
                    capabilities: cx.new(|_| capabilities_demo::CapabilitiesDemo::new(app.clone())),
                    app,
                    label,
                    clicks: 0,
                    clipboard: "idle".into(),
                    picker: "idle".into(),
                    window_state: "unread".into(),
                    background_step: 0,
                    menu_actions,
                })
            },
        );
        match result {
            Ok(handle) => {
                state.id = Some(handle.window_id());
                state.opens += 1;
                state.status = format!("open (launch #{})", state.opens);
            }
            Err(error) => state.status = format!("error: {error}"),
        }
        cx.notify();
    }
}

impl Render for CaptureDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(
            cx.window_appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let child_controls = self
            .child_windows
            .iter()
            .enumerate()
            .map(|(index, state)| {
                let action = if state.id.is_some() {
                    "already open"
                } else {
                    "tap to open"
                };
                div()
                    .id(format!("open-child-window-{index}"))
                    .p_4()
                    .bg(rgb(0x7c3aed))
                    .child(format!(
                        "Window {}: {} — {action}",
                        CHILD_WINDOW_LABELS[index], state.status,
                    ))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.open_child_window(index, cx);
                    }))
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .size_full()
            .id("main-scroll")
            .overflow_y_scroll()
            .min_h_0()
            .flex_col()
            .gap_4()
            .p_8()
            .text_xl()
            .bg(
                if self.background_appearance == WindowBackgroundAppearance::Opaque {
                    rgba(if dark { 0x18181bff } else { 0xffffffff })
                } else {
                    rgba(0x00000000)
                },
            )
            .text_color(rgb(if dark { 0xf4f4f5 } else { 0x18181b }))
            .child("Hello, GPUI on OpenHarmony!")
            .child(self.capabilities.clone())
            .child(self.ime.clone())
            .child(format!("Appearance: {}", self.appearance_status))
            .child(format!(
                "Window background: {:?}",
                self.background_appearance
            ))
            .child(format!("Reopens: {}", self.reopens.load(Ordering::Relaxed)))
            .child(format!(
                "Notification responses: {}",
                self.notification_responses.load(Ordering::Relaxed)
            ))
            .child(format!("Thermal state: {:?}", cx.thermal_state()))
            .child(format!("Window controls: {:?}", window.window_controls()))
            .child(format!(
                "Layout: {:?}; visual viewport: {:?}",
                window.viewport_size(),
                window.visual_viewport_bounds()
            ))
            .child(format!("Bundle path: {}", self.app_path_status))
            .child(format!("URL scheme: {}", self.url_scheme_status))
            .child(format!("Display area: {}", self.display_status))
            .child(format!(
                "Next-frame callbacks: {}",
                self.next_frame_callbacks.load(Ordering::Relaxed)
            ))
            .child(
                div()
                    .id("check-app-path")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Read GPUI application path")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.app_path_status = match cx.app_path() {
                            Ok(path) => path.display().to_string(),
                            Err(error) => format!("unavailable: {error}"),
                        };
                        log::info!("GPUI application path: {}", this.app_path_status);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("check-url-scheme")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Check installed URL scheme")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let declared = cx.register_url_scheme("gpui-demo");
                        let absent = cx.register_url_scheme("gpui-absent");
                        this.url_scheme_status = "checking".into();
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            let declared = declared.await;
                            let absent = absent.await;
                            let _ = this.update(cx, |this, cx| {
                                this.url_scheme_status = format!(
                                    "declared={}, absent rejected={}",
                                    declared.is_ok(),
                                    absent.is_err()
                                );
                                log::info!("GPUI URL scheme check: {}", this.url_scheme_status);
                                cx.notify();
                            });
                        })
                        .detach();
                    })),
            )
            .child(
                div()
                    .id("zoom-main-window")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Toggle main window maximize")
                    .on_click(cx.listener(|_this, _event, window, _cx| window.zoom_window())),
            )
            .child(
                div()
                    .id("check-display-area")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Read usable display area")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let displays = cx.displays();
                        this.display_status = cx
                            .primary_display()
                            .map(|display| {
                                format!(
                                    "count={}, full={:?}, usable={:?}",
                                    displays.len(),
                                    display.bounds(),
                                    display.visible_bounds()
                                )
                            })
                            .unwrap_or_else(|| "unavailable".into());
                        log::info!("GPUI display area: {}", this.display_status);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("request-window-attention")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Request window attention")
                    .on_click(cx.listener(|_this, _event, window, _cx| window.request_attention())),
            )
            .child(
                div()
                    .id("schedule-next-frame")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Schedule next-frame callback")
                    .on_click(cx.listener(|this, _event, window, _cx| {
                        let callbacks = this.next_frame_callbacks.clone();
                        window.on_next_frame(move |_window, cx| {
                            let count = callbacks.fetch_add(1, Ordering::Relaxed) + 1;
                            log::info!("GPUI next-frame callback: {count}");
                            cx.refresh_windows();
                        });
                    })),
            )
            .child(
                div()
                    .id("toggle-appearance")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Cycle dark / light / system appearance")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let appearance = match this.appearance_step % 3 {
                            0 => Some(WindowAppearance::Dark),
                            1 => Some(WindowAppearance::Light),
                            _ => None,
                        };
                        this.appearance_step += 1;
                        cx.set_window_appearance(appearance);
                        this.appearance_status = format!(
                            "requested {appearance:?}, reported {:?}",
                            cx.window_appearance()
                        );
                        log::info!("GPUI appearance check: {}", this.appearance_status);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("cycle-main-background")
                    .p_4()
                    .bg(rgb(0x06b6d4))
                    .child("Cycle main blur / transparent / opaque background")
                    .on_click(cx.listener(|this, _event, window, cx| {
                        let appearance = match this.background_step % 3 {
                            0 => WindowBackgroundAppearance::Blurred,
                            1 => WindowBackgroundAppearance::Transparent,
                            _ => WindowBackgroundAppearance::Opaque,
                        };
                        this.background_step += 1;
                        this.background_appearance = appearance;
                        window.set_background_appearance(appearance);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("show-notification")
                    .p_4()
                    .bg(rgb(0x2563eb))
                    .child("Show system notification")
                    .on_click(cx.listener(|_this, _event, _window, cx| {
                        cx.show_system_notification(SystemNotification {
                            tag: "gpui-demo-notification".into(),
                            title: "GPUI on OpenHarmony".into(),
                            body: "Tap this notification or its action".into(),
                            actions: vec![SystemNotificationAction {
                                id: "acknowledge".into(),
                                label: "Acknowledge".into(),
                            }],
                        });
                    })),
            )
            .child(
                div()
                    .id("dismiss-notification")
                    .p_4()
                    .bg(rgb(0x475569))
                    .child("Dismiss system notification")
                    .on_click(cx.listener(|_this, _event, _window, cx| {
                        cx.dismiss_system_notification("gpui-demo-notification");
                    })),
            )
            .child(
                div()
                    .id("hide-ability")
                    .p_4()
                    .bg(rgb(0x64748b))
                    .child("Hide Ability (relaunch to check reopen)")
                    .on_click(cx.listener(|_this, _event, _window, cx| {
                        log::info!("GPUI visibility check: requesting hide");
                        cx.hide();
                    })),
            )
            .child(
                div()
                    .id("quit-ability")
                    .p_4()
                    .bg(rgb(0x991b1b))
                    .child("Quit Ability")
                    .on_click(cx.listener(|_this, _event, _window, cx| {
                        log::info!("GPUI quit check: requesting quit");
                        cx.quit();
                    })),
            )
            .child("Screen capture demo")
            .child(format!("Status: {}", self.status))
            .child(format!(
                "Video frames: {}",
                self.frames.load(Ordering::Relaxed)
            ))
            .child(format!("Credential check: {}", self.credentials))
            .child(format!(
                "Menu actions: {}",
                self.menu_actions.load(Ordering::Relaxed)
            ))
            .children(child_controls)
            .child(
                div()
                    .id("check-credentials")
                    .p_4()
                    .bg(rgb(0x0d9488))
                    .child("Check credential storage")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        const KEY: &str = "gpui-ohos-example://credential-smoke";
                        const USER: &str = "gpui-demo-user";
                        const PASSWORD: &[u8] = b"gpui-demo-password";
                        const UPDATED_USER: &str = "gpui-demo-updated";
                        const UPDATED_PASSWORD: &[u8] = b"gpui-demo-updated-password";
                        let write = cx.write_credentials(KEY, USER, PASSWORD);
                        this.credentials = "writing".into();
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            let result: anyhow::Result<()> = async {
                                write.await?;
                                let update = this.update(cx, |_, cx| {
                                    cx.write_credentials(KEY, UPDATED_USER, UPDATED_PASSWORD)
                                })?;
                                update.await?;
                                let read = this.update(cx, |_, cx| cx.read_credentials(KEY))?;
                                let stored = read.await?;
                                anyhow::ensure!(
                                    stored
                                        == Some((UPDATED_USER.into(), UPDATED_PASSWORD.to_vec())),
                                    "stored credential differs"
                                );
                                let delete = this.update(cx, |_, cx| cx.delete_credentials(KEY))?;
                                delete.await?;
                                let read = this.update(cx, |_, cx| cx.read_credentials(KEY))?;
                                anyhow::ensure!(
                                    read.await?.is_none(),
                                    "credential was not deleted"
                                );
                                Ok(())
                            }
                            .await;
                            let _ = this.update(cx, |this, cx| {
                                this.credentials = match result {
                                    Ok(()) => "write/update/read/delete passed".into(),
                                    Err(error) => format!("error: {error}"),
                                };
                                cx.notify();
                            });
                        })
                        .detach();
                    })),
            )
            .child(
                div()
                    .id("toggle-capture")
                    .p_4()
                    .bg(rgb(0x2563eb))
                    .child(if self.stream.is_some() {
                        "Stop capture"
                    } else {
                        "Start capture"
                    })
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        if this.stream.take().is_some() {
                            this.status = "stopped".into();
                            cx.notify();
                            return;
                        }

                        let sources = cx.screen_capture_sources();
                        let executor = cx.foreground_executor().clone();
                        let timer = cx.background_executor().clone();
                        this.frames = Arc::new(AtomicUsize::new(0));
                        let frames = this.frames.clone();
                        this.status = "requesting consent".into();
                        cx.notify();

                        cx.spawn(async move |this, cx| {
                            let result: anyhow::Result<Box<dyn gpui_ohos::ScreenCaptureStream>> =
                                async {
                                    let source = sources
                                        .await??
                                        .into_iter()
                                        .next()
                                        .ok_or_else(|| anyhow::anyhow!("No capture display"))?;
                                    let stream = source
                                        .stream(
                                            &executor,
                                            Box::new(move |_frame| {
                                                frames.fetch_add(1, Ordering::Relaxed);
                                            }),
                                        )
                                        .await??;
                                    Ok(stream)
                                }
                                .await;

                            let started = this
                                .update(cx, |this, cx| {
                                    match result {
                                        Ok(stream) => {
                                            this.stream = Some(stream);
                                            this.status = "capturing".into();
                                        }
                                        Err(error) => this.status = format!("error: {error}"),
                                    }
                                    cx.notify();
                                    this.stream.is_some()
                                })
                                .unwrap_or(false);
                            if !started {
                                return;
                            }

                            loop {
                                timer.timer(Duration::from_millis(500)).await;
                                if !this
                                    .update(cx, |this, cx| {
                                        if this.stream.is_some() {
                                            cx.notify();
                                            true
                                        } else {
                                            false
                                        }
                                    })
                                    .unwrap_or(false)
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    })),
            )
    }
}

#[openharmony_ability_derive::ability]
fn openharmony_app(app: OpenHarmonyApp) {
    ohos_hilog_binding::log::init_once(Config::default().with_max_level(LevelFilter::Info));
    app.register_plugin(PermissionBridgePlugin)
        .expect("failed to register permission plugin");
    let inner_app = app.clone();
    let application = Application::with_platform(gpui_ohos::current_platform(app, false));
    let reopens = Arc::new(AtomicUsize::new(0));
    let notification_responses = Arc::new(AtomicUsize::new(0));
    let reopened = reopens.clone();
    application.on_reopen(move |cx| {
        let count = reopened.fetch_add(1, Ordering::Relaxed) + 1;
        log::info!("GPUI reopen check: count={count}");
        cx.refresh_windows();
    });
    application.on_open_urls(|urls| {
        log::info!("GPUI received external URLs: {urls:?}");
    });

    let application_handle = application.run_embedded(move |cx: &mut App| {
        let response_count = notification_responses.clone();
        cx.on_system_notification_response(move |response, cx| {
            let count = response_count.fetch_add(1, Ordering::Relaxed) + 1;
            log::info!("GPUI notification response {count}: {response:?}");
            cx.refresh_windows();
        });
        cx.on_thermal_state_change(|cx| {
            log::info!("GPUI thermal state changed: {:?}", cx.thermal_state());
            cx.refresh_windows();
        })
        .detach();
        cx.on_system_sleep(|_| {
            log::info!("GPUI system sleep callback");
        })
        .detach();
        cx.on_system_wake(|cx| {
            log::info!("GPUI system wake callback");
            cx.refresh_windows();
        })
        .detach();
        let main_view: Rc<RefCell<Option<WeakEntity<CaptureDemo>>>> = Rc::new(RefCell::new(None));
        let closed_view = main_view.clone();
        cx.on_window_closed(move |cx, window_id| {
            if let Some(view) = closed_view.borrow().as_ref() {
                let _ = view.update(cx, |this, cx| {
                    if let Some(state) = this
                        .child_windows
                        .iter_mut()
                        .find(|state| state.id == Some(window_id))
                    {
                        state.id = None;
                        state.status = "closed".into();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
        let menu_actions = Arc::new(AtomicUsize::new(0));
        cx.on_app_quit(|_| async {
            log::info!("GPUI quit check: shutdown callback completed");
        })
        .detach();
        let action_count = menu_actions.clone();
        cx.on_action(move |_: &CountMenuAction, cx| {
            let count = action_count.fetch_add(1, Ordering::Relaxed) + 1;
            log::info!("GPUI menu action count: {count}");
            cx.refresh_windows();
        });
        cx.set_menus(vec![Menu::new("Demo").items(vec![
            MenuItem::action("Count menu action", CountMenuAction),
            MenuItem::action("Unavailable action", UnavailableMenuAction),
        ])]);
        let content_rect = inner_app.content_rect();
        let bounds = Bounds::centered(
            None,
            size(
                px(content_rect.width as f32),
                px(content_rect.height as f32),
            ),
            cx,
        );

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                let ime = cx.new(ime_demo::ImeDemo::new);
                let view = cx.new(|cx| CaptureDemo {
                    capabilities: cx
                        .new(|_| capabilities_demo::CapabilitiesDemo::new(inner_app.clone())),
                    ime,
                    app: inner_app.clone(),
                    status: "idle".into(),
                    credentials: "idle".into(),
                    appearance_step: 0,
                    appearance_status: "following system".into(),
                    background_step: 0,
                    background_appearance: WindowBackgroundAppearance::Opaque,
                    app_path_status: "not queried".into(),
                    url_scheme_status: "not checked".into(),
                    display_status: "not queried".into(),
                    reopens: reopens.clone(),
                    notification_responses: notification_responses.clone(),
                    next_frame_callbacks: Arc::new(AtomicUsize::new(0)),
                    child_windows: std::array::from_fn(|_| ChildWindowState::new()),
                    menu_actions: menu_actions.clone(),
                    frames: Arc::new(AtomicUsize::new(0)),
                    stream: None,
                });
                *main_view.borrow_mut() = Some(view.downgrade());
                view
            },
        )
        .expect("failed to open GPUI window");
        cx.activate(true);
    });

    APPLICATION.with(|application| {
        application.replace(Some(application_handle));
    });
}
