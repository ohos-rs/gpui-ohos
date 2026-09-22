use std::cell::RefCell;

use gpui_ohos::{
    App, AppContext, Application, ApplicationHandle, Bounds, Context, IntoElement, Render, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use openharmony_ability::OpenHarmonyApp;

thread_local! {
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

struct HelloWorld;

impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(rgb(0x18181b))
            .text_xl()
            .text_color(rgb(0xf4f4f5))
            .child("Hello, GPUI on OpenHarmony!")
    }
}

#[openharmony_ability_derive::ability]
fn openharmony_app(app: OpenHarmonyApp) {
    let inner_app = app.clone();
    let application = Application::with_platform(gpui_ohos::current_platform(app, false));

    let application_handle = application.run_embedded(move |cx: &mut App| {
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
            |_, cx| cx.new(|_| HelloWorld),
        )
        .expect("failed to open GPUI window");
        cx.activate(true);
    });

    APPLICATION.with(|application| {
        application.replace(Some(application_handle));
    });
}
