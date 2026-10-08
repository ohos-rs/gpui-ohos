use log::{debug, info, warn};

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Instant,
};

use accesskit_ohos::Adapter as OhosA11yAdapter;
use anyhow::Result;
use futures::channel::oneshot;
use ohos_accessibility_binding::Provider;
use ohos_vsync_binding::Vsync;
use openharmony_ability::{
    ArkUiInputEvent, AvoidAreaType, Event, ImeEvent, InputEvent, OpenHarmonyApp, PointerInputData,
    XComponentInputEvent,
    arkui::arkui_input_binding::{UIInputAction, UIInputSourceType, UIInputToolType},
    ime::{
        Action as ImeAction, Direction as ImeDirection, EnterKey, InputType, Rect as ImeCursorRect,
        TextState,
    },
    xcomponent::{
        MouseAction, MouseButton as OhosMouseButton, TouchEvent as OhosTouchEvent, TouchEventData,
        TouchPointData,
    },
};
use openharmony_ability_plugin_window::WindowClient;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use super::display::OhosDisplay;
use super::keyboard::OhosKeyState;
use super::platform::appearance_for_color_mode;
use super::touch_scroll::{NativePanInput, TouchScroll};
use super::wgpu_atlas::WgpuAtlas;
use super::wgpu_context::WgpuContext;
use super::wgpu_renderer::{WgpuRenderer, WgpuSurfaceConfig};
use crate::{
    A11yCallbacks, AnyWindowHandle, Bounds, Capslock, DevicePixels, Edges, ForegroundExecutor,
    GestureTuning, GpuSpecs, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, NavigationDirection, Pixels, PlatformAtlas, PlatformDisplay,
    PlatformInput, PlatformInputHandler, PlatformWindow, Point, PromptButton, PromptLevel,
    RequestFrameOptions, ResizeEdge, Scene, ScrollDelta, ScrollWheelEvent, Size, Task,
    TextInputAction, TextInputConfiguration, TextInputStateChange, TouchEvent, TouchId, TouchPhase,
    WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowControls,
    WindowDecorations, WindowInsets, WindowParams, WindowVisibility, accesskit, point, px, size,
};
use openharmony_ability::FrameInputDelivery;

pub(crate) type BackHandler = Rc<RefCell<Option<Box<dyn FnMut()>>>>;
type ResizeCallback = Box<dyn FnMut(Size<Pixels>, f32)>;

pub(crate) struct OhosWindowContext {
    pub(crate) app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    pub(crate) handle: AnyWindowHandle,
    pub(crate) params: WindowParams,
    pub(crate) gpu_context: Arc<WgpuContext>,
    pub(crate) display: OhosDisplay,
    pub(crate) foreground_executor: ForegroundExecutor,
    pub(crate) frame_wake: Arc<super::dispatcher::MainWake>,
    pub(crate) clipboard: super::clipboard::OhosClipboard,
    pub(crate) cursor_hidden_until_move: Rc<Cell<bool>>,
    pub(crate) appearance_override: Rc<Cell<Option<WindowAppearance>>>,
    pub(crate) window_id: i64,
    pub(crate) fallback_atlas: Option<Arc<WgpuAtlas>>,
    pub(crate) quit: Rc<dyn Fn()>,
}

pub(crate) struct OhosWindow {
    app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    quit: Rc<dyn Fn()>,
    pub(crate) handle: AnyWindowHandle,
    bounds: RefCell<Bounds<Pixels>>,
    viewport: super::viewport::ViewportPublisher,
    scale: RefCell<f32>,
    appearance: Cell<WindowAppearance>,
    appearance_override: Rc<Cell<Option<WindowAppearance>>>,
    window_id: i64,
    title: Rc<RefCell<String>>,
    title_generation: Rc<Cell<u64>>,
    restore_state: Rc<RefCell<Option<super::window_state::NativeWindowState>>>,
    restore_pending: Rc<Cell<bool>>,
    floating_state: Cell<Option<super::window_state::NativeWindowState>>,
    is_movable: bool,
    is_resizable: bool,
    is_minimizable: bool,
    min_size: Option<Size<Pixels>>,
    frame_scheduler: Option<Arc<FrameScheduler>>,
    closed: Cell<bool>,
    maximized: Rc<Cell<bool>>,
    fullscreen: Rc<Cell<bool>>,
    background_appearance: Rc<Cell<WindowBackgroundAppearance>>,
    requested_background: Rc<Cell<Option<WindowBackgroundAppearance>>>,
    background_pending: Rc<Cell<bool>>,
    background_generation: Rc<Cell<u64>>,
    active: Cell<bool>,
    hovered: Cell<bool>,
    visibility: Cell<WindowVisibility>,
    decorations: Rc<Cell<WindowDecorations>>,
    requested_decorations: Cell<WindowDecorations>,
    insets: RefCell<WindowInsets>,
    keyboard_overlap_device_px: Cell<i32>,
    input_handler: Rc<RefCell<Option<PlatformInputHandler>>>,
    ime_text_update_pending: Rc<Cell<bool>>,
    edit_generation: Rc<Cell<u64>>,
    clipboard: super::clipboard::OhosClipboard,
    callbacks: Rc<RefCell<WindowCallbacks>>,
    renderer: RefCell<Option<WgpuRenderer>>,
    surface_available: Cell<bool>,
    gpu_context: Arc<WgpuContext>,
    display: OhosDisplay,
    fallback_atlas: RefCell<Option<Arc<WgpuAtlas>>>,
    foreground_executor: ForegroundExecutor,
    cursor_hidden_until_move: Rc<Cell<bool>>,
    keyboard_visible: Rc<Cell<bool>>,
    ime_action: Cell<TextInputAction>,
    pointer_position: Cell<Option<Point<Pixels>>>,
    pointer_screen_position: Cell<Option<(i64, i64)>>,
    pressed_mouse_button: Cell<Option<MouseButton>>,
    mouse_click: RefCell<Option<MouseClickState>>,
    native_pointer_id: Cell<Option<i32>>,
    native_drag: Rc<RefCell<Option<openharmony_ability::NativeFileDrag>>>,
    native_drag_pending: Rc<Cell<bool>>,
    incoming_drag: Cell<bool>,
    manipulation: Cell<Option<super::window_manipulation::WindowManipulation>>,
    geometry_request: Rc<RefCell<Option<super::window_state::NativeWindowState>>>,
    geometry_pending: Rc<Cell<bool>>,
    back_enabled: Cell<bool>,
    back_handler: BackHandler,
    a11y_callbacks: RefCell<Option<Arc<Mutex<A11yCallbacks>>>>,
    a11y_adapter: RefCell<Option<OhosA11yAdapter<'static>>>,
    key_state: RefCell<OhosKeyState>,
    active_touches: RefCell<HashMap<i32, TouchId>>,
    touch_tap_candidates: RefCell<HashMap<i32, TouchTapCandidate>>,
    next_touch_id: Cell<u64>,
    touch_scroll: Rc<RefCell<TouchScroll>>,
    long_press_timer: RefCell<Option<(TouchId, Task<()>)>>,
}

/// GPUI invalidation requests a system VSync. The native callback only records
/// the tick; drawing stays on the Ability's main thread.
struct FrameScheduler {
    vsync: Vsync,
    state: Arc<super::frame_request::FrameRequest>,
    waker: Arc<super::dispatcher::MainWake>,
}
impl FrameScheduler {
    fn new(window_id: i64, waker: Arc<super::dispatcher::MainWake>) -> Option<Arc<Self>> {
        Some(Arc::new(Self {
            vsync: Vsync::try_new(format!("gpui-ohos-{window_id}"))?,
            state: Arc::new(super::frame_request::FrameRequest::default()),
            waker,
        }))
    }
    fn request_frame(&self) {
        let Some(ticket) = self.state.request() else {
            return;
        };
        let state = self.state.clone();
        let waker = self.waker.clone();
        let result = self.vsync.request_frame_once(move |_| {
            if state.complete(ticket) {
                waker.notify();
            }
        });
        if result != 0 {
            self.state.fail(ticket);
            self.waker.notify();
            warn!("Failed to request OHOS VSync frame: {result}");
        }
    }
    fn take_pending(&self) -> bool {
        self.state.take_pending()
    }
    fn set_active(&self, active: bool) {
        self.state.set_active(active);
        if active {
            self.request_frame();
        }
    }
}

pub(crate) struct OhosWindowHandle {
    inner: Rc<RefCell<OhosWindow>>,
    input_handler: Rc<RefCell<Option<PlatformInputHandler>>>,
}

impl Drop for OhosWindow {
    fn drop(&mut self) {
        self.native_drag.borrow_mut().take();
        self.background_generation
            .set(self.background_generation.get().wrapping_add(1));
        self.invalidate_pending_edit();
        let was_closed = self.closed.replace(true);
        self.release_accessibility();
        if self.window_id == 0 || was_closed {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.destroy_window(window_id).await {
                    warn!("Failed to destroy OHOS sub-window {window_id}: {error}");
                }
            })
            .detach();
    }
}

impl OhosWindowHandle {
    pub(crate) fn new(inner: Rc<RefCell<OhosWindow>>) -> Self {
        let input_handler = inner.borrow().input_handler.clone();
        Self {
            inner,
            input_handler,
        }
    }

    fn with_window<R>(&self, f: impl FnOnce(&OhosWindow) -> R) -> R {
        let window = self.inner.borrow();
        f(&window)
    }

    fn with_window_mut<R>(&self, f: impl FnOnce(&mut OhosWindow) -> R) -> R {
        let mut window = self.inner.borrow_mut();
        f(&mut window)
    }
}

struct WindowCallbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<Box<dyn FnMut(PlatformInput) -> crate::DispatchEventResult>>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    visibility_change: Option<Box<dyn FnMut(WindowVisibility)>>,
    insets_changed: Option<Box<dyn FnMut(WindowInsets)>>,
    visual_viewport_changed: Option<Box<dyn FnMut()>>,
    hover_status_change: Option<Box<dyn FnMut(bool)>>,
    resize: Option<ResizeCallback>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
    hit_test_window_control: Option<Box<dyn FnMut() -> Option<WindowControlArea>>>,
}

#[derive(Clone, Copy)]
struct TouchTapCandidate {
    start_position: Point<Pixels>,
    started_in_text_input: bool,
}

struct MouseClickState {
    button: MouseButton,
    position: Point<Pixels>,
    time: Instant,
    count: usize,
}

struct OhosA11yActivation(Arc<Mutex<A11yCallbacks>>);

impl accesskit::ActivationHandler for OhosA11yActivation {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        self.0
            .lock()
            .ok()
            .and_then(|callbacks| (callbacks.activation)())
    }
}

struct OhosA11yAction(Arc<Mutex<A11yCallbacks>>);

impl accesskit::ActionHandler for OhosA11yAction {
    fn do_action(&mut self, request: accesskit::ActionRequest) {
        if let Ok(callbacks) = self.0.lock() {
            (callbacks.action)(request);
        }
    }
}

impl OhosWindow {
    fn initialize_accessibility(&self) {
        if self.a11y_adapter.borrow().is_some() {
            return;
        }
        let Some(callbacks) = self.a11y_callbacks.borrow().clone() else {
            return;
        };
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        let Some(provider_info) = app.with_xcomponent_for(self.window_id, |component| {
            let native = component.native_xcomponent();
            let provider = native
                .accessibility_provider()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            Ok::<_, anyhow::Error>(provider.as_raw() as usize)
        }) else {
            return;
        };
        let provider_raw = match provider_info {
            Ok(info) => info,
            Err(error) => {
                warn!("Failed to find OHOS accessibility provider: {error}");
                return;
            }
        };
        // The adapter is released on SurfaceDestroy, before the ability drops
        // the render XComponent that owns this provider.
        let provider: Provider<'static> =
            match unsafe { Provider::from_raw(provider_raw as *mut _) } {
                Ok(provider) => provider,
                Err(error) => {
                    warn!("Failed to retain OHOS accessibility provider: {error}");
                    return;
                }
            };
        // Single-instance callbacks have one process-wide Rust handler. The
        // API 15 callback table routes by the framework instance ID registered
        // here; native XComponent IDs are not unique across GPUI windows.
        // A new ID also keeps late callbacks from a destroyed surface isolated.
        let instance_id = format!("gpui-{}", uuid::Uuid::new_v4());
        match OhosA11yAdapter::new_with_instance(
            provider,
            &instance_id,
            OhosA11yActivation(callbacks.clone()),
            OhosA11yAction(callbacks),
        ) {
            Ok(adapter) => {
                if let Err(error) = adapter.set_host_focus_state(self.active.get()) {
                    warn!("Failed to set OHOS accessibility focus: {error}");
                }
                *self.a11y_adapter.borrow_mut() = Some(adapter);
                info!(
                    "OHOS accessibility adapter registered window={}",
                    self.window_id
                );
            }
            Err(error) => warn!("Failed to register OHOS accessibility adapter: {error}"),
        }
    }

    fn release_accessibility(&self) {
        let adapter = self.a11y_adapter.borrow_mut().take();
        if adapter.is_some() {
            drop(adapter);
            // Closed GPUI windows have no registry entry to refresh. Surface
            // recreation still deactivates the live window's accessibility.
            if !self.closed.get()
                && let Some(callbacks) = self.a11y_callbacks.borrow().as_ref()
                && let Ok(callbacks) = callbacks.lock()
            {
                (callbacks.deactivation)();
            }
        }
    }

    pub(crate) fn new(context: OhosWindowContext) -> Result<Self> {
        let OhosWindowContext {
            app,
            handle,
            params,
            gpu_context,
            display,
            foreground_executor,
            frame_wake,
            clipboard,
            cursor_hidden_until_move,
            appearance_override,
            window_id,
            fallback_atlas,
            quit,
        } = context;
        let scale = display.scale_factor();
        let appearance = appearance_override.get().unwrap_or_else(|| {
            app.borrow()
                .as_ref()
                .map(|app| appearance_for_color_mode(app.config().color_mode))
                .unwrap_or_default()
        });
        let bounds = params.bounds;
        let frame_scheduler = FrameScheduler::new(window_id, frame_wake);
        if frame_scheduler.is_none() {
            warn!("OHOS VSync is unavailable for window {window_id}");
        }

        // Don't create renderer immediately - native_window may not be available yet.
        // Renderer will be initialized lazily in draw() or when SurfaceCreate event is received.
        // At that point, native_window from OpenHarmonyApp will be available.

        Ok(Self {
            app: app.clone(),
            quit,
            handle,
            bounds: RefCell::new(bounds),
            viewport: super::viewport::ViewportPublisher::default(),
            scale: RefCell::new(scale),
            appearance: Cell::new(appearance),
            appearance_override,
            window_id,
            title: Rc::new(RefCell::new(
                params
                    .titlebar
                    .and_then(|titlebar| titlebar.title)
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
            )),
            title_generation: Rc::new(Cell::new(0)),
            restore_state: Rc::new(RefCell::new(None)),
            restore_pending: Rc::new(Cell::new(false)),
            floating_state: Cell::new(None),
            is_movable: params.is_movable,
            is_resizable: params.is_resizable,
            is_minimizable: params.is_minimizable,
            min_size: params.window_min_size,
            frame_scheduler,
            closed: Cell::new(false),
            maximized: Rc::new(Cell::new(false)),
            fullscreen: Rc::new(Cell::new(false)),
            background_appearance: Rc::new(Cell::new(WindowBackgroundAppearance::Opaque)),
            requested_background: Rc::new(Cell::new(None)),
            background_pending: Rc::new(Cell::new(false)),
            background_generation: Rc::new(Cell::new(0)),
            active: Cell::new(window_id == 0),
            hovered: Cell::new(false),
            visibility: Cell::new(WindowVisibility::Visible),
            decorations: Rc::new(Cell::new(WindowDecorations::Server)),
            requested_decorations: Cell::new(WindowDecorations::Server),
            insets: RefCell::new(WindowInsets::default()),
            keyboard_overlap_device_px: Cell::new(0),
            input_handler: Rc::new(RefCell::new(None)),
            ime_text_update_pending: Rc::new(Cell::new(false)),
            edit_generation: Rc::new(Cell::new(0)),
            clipboard,
            callbacks: Rc::new(RefCell::new(WindowCallbacks {
                request_frame: None,
                input: None,
                active_status_change: None,
                visibility_change: None,
                insets_changed: None,
                visual_viewport_changed: None,
                hover_status_change: None,
                resize: None,
                moved: None,
                should_close: None,
                close: None,
                appearance_changed: None,
                hit_test_window_control: None,
            })),
            renderer: RefCell::new(None),
            surface_available: Cell::new(false),
            gpu_context,
            display,
            fallback_atlas: RefCell::new(fallback_atlas),
            foreground_executor,
            cursor_hidden_until_move,
            keyboard_visible: Rc::new(Cell::new(false)),
            ime_action: Cell::new(TextInputAction::Unspecified),
            pointer_position: Cell::new(None),
            pointer_screen_position: Cell::new(None),
            pressed_mouse_button: Cell::new(None),
            mouse_click: RefCell::new(None),
            native_pointer_id: Cell::new(None),
            native_drag: Rc::new(RefCell::new(None)),
            native_drag_pending: Rc::new(Cell::new(false)),
            incoming_drag: Cell::new(false),
            manipulation: Cell::new(None),
            geometry_request: Rc::new(RefCell::new(None)),
            geometry_pending: Rc::new(Cell::new(false)),
            back_enabled: Cell::new(false),
            back_handler: Rc::new(RefCell::new(None)),
            a11y_callbacks: RefCell::new(None),
            a11y_adapter: RefCell::new(None),
            key_state: RefCell::new(OhosKeyState::default()),
            active_touches: RefCell::new(HashMap::new()),
            touch_tap_candidates: RefCell::new(HashMap::new()),
            next_touch_id: Cell::new(0),
            touch_scroll: Rc::new(RefCell::new(TouchScroll::default())),
            long_press_timer: RefCell::new(None),
        })
    }

    pub(crate) fn atlas(&self) -> Option<Arc<WgpuAtlas>> {
        self.fallback_atlas.borrow().clone()
    }

    pub(crate) fn window_id(&self) -> i64 {
        self.window_id
    }

    pub(crate) fn set_appearance(&self, appearance: WindowAppearance) {
        if self.appearance.replace(appearance) != appearance {
            let mut callback = self.callbacks.borrow_mut().appearance_changed.take();
            if let Some(ref mut callback) = callback {
                callback();
            }
            self.callbacks.borrow_mut().appearance_changed = callback;
        }
    }

    pub(crate) fn take_pending_frame(&self) -> bool {
        self.sync_frame_delivery();
        !self.closed.get()
            && self.surface_available.get()
            && self.visibility.get() == WindowVisibility::Visible
            && self
                .frame_scheduler
                .as_ref()
                .is_some_and(|scheduler| scheduler.take_pending())
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        let scheduler = self.frame_scheduler.as_ref()?.clone();
        Some(Rc::new(move || scheduler.request_frame()))
    }

    fn sync_frame_delivery(&self) {
        let delivery = if self.surface_available.get()
            && self.visibility.get() == WindowVisibility::Visible
            && self
                .frame_scheduler
                .as_ref()
                .is_none_or(|scheduler| scheduler.state.failed())
        {
            FrameInputDelivery::Continuous
        } else {
            FrameInputDelivery::OnDemand
        };
        if let Some(app) = self.app.borrow().as_ref()
            && let Err(error) = app.set_frame_input_delivery_for(self.window_id, delivery)
        {
            warn!("Cannot configure OHOS frame delivery: {error}");
        }
    }

    pub(crate) fn draw_requested_frame(&self) {
        if self.closed.get()
            || !self.surface_available.get()
            || self.visibility.get() != WindowVisibility::Visible
        {
            return;
        }
        let momentum_input = self.touch_scroll.borrow_mut().tick(Instant::now());
        if let Some(input) = momentum_input {
            self.dispatch_input(input);
        }
        let mut callback = self.callbacks.borrow_mut().request_frame.take();
        if let Some(ref mut callback) = callback {
            callback(RequestFrameOptions {
                require_presentation: false,
                force_render: false,
            });
        }
        self.callbacks.borrow_mut().request_frame = callback;
        self.request_touch_momentum_frame();
    }

    pub(crate) fn apply_window_status(&self, status: i32) {
        // OHOS WindowStatusType: FULL_SCREEN=1, MAXIMIZE=2,
        // MINIMIZE=3, FLOATING=4, SPLIT_SCREEN=5.
        if !(1..=5).contains(&status) {
            warn!(
                "Unknown OHOS window status {status} for window {}",
                self.window_id
            );
            return;
        }
        self.maximized.set(status == 2);
        self.fullscreen.set(status == 1);
        self.update_visibility(if status == 3 {
            WindowVisibility::Hidden
        } else {
            WindowVisibility::Visible
        });
    }

    fn dispatch_input_with_callbacks(
        callbacks: &Rc<RefCell<WindowCallbacks>>,
        input: PlatformInput,
    ) -> crate::DispatchEventResult {
        let mut callback = callbacks.borrow_mut().input.take();
        let mut result = crate::DispatchEventResult::default();
        if let Some(ref mut cb) = callback {
            result = cb(input);
        }
        callbacks.borrow_mut().input = callback;
        result
    }

    fn point_from_device_pixels(&self, x: f32, y: f32) -> Point<Pixels> {
        let scale = (*self.scale.borrow()).max(f32::EPSILON);
        point(px(x / scale), px(y / scale))
    }

    fn pointer_position_from_arkui(&self, pointer: PointerInputData) -> Point<Pixels> {
        self.point_from_device_pixels(pointer.x, pointer.y)
    }

    fn allocate_touch_id(&self) -> TouchId {
        let raw_id = self.next_touch_id.get();
        self.next_touch_id
            .set(raw_id.checked_add(1).expect("touch ID exhausted"));
        TouchId(raw_id)
    }

    fn dispatch_raw_touch_point(&self, raw_id: i32, x: f32, y: f32, force: f32, phase: TouchPhase) {
        if phase == TouchPhase::Started {
            self.invalidate_pending_edit();
        }
        let position = self.point_from_device_pixels(x, y);
        if phase == TouchPhase::Started {
            self.native_pointer_id.set(Some(raw_id));
        }
        let id = if phase == TouchPhase::Started {
            let id = self.allocate_touch_id();
            let mut active_touches = self.active_touches.borrow_mut();
            if active_touches.is_empty() {
                self.touch_tap_candidates.borrow_mut().insert(
                    raw_id,
                    TouchTapCandidate {
                        start_position: position,
                        // Capture this before GPUI translates the touch into its
                        // compatibility mouse gesture. A newly focused input will
                        // use the regular FocusGained path; only an input that was
                        // already focused needs an explicit IME reopen request.
                        started_in_text_input: self.pointer_targets_text_input(position),
                    },
                );
            } else {
                self.touch_tap_candidates.borrow_mut().clear();
            }
            active_touches.insert(raw_id, id);
            id
        } else {
            let Some(id) = self.active_touches.borrow().get(&raw_id).copied() else {
                return;
            };
            id
        };
        self.pointer_position.set(Some(position));
        let should_reopen_keyboard = match phase {
            TouchPhase::Moved => {
                if self.touch_moved_beyond_tap_slop(raw_id, position) {
                    self.touch_tap_candidates.borrow_mut().remove(&raw_id);
                }
                false
            }
            TouchPhase::Ended => self
                .touch_tap_candidates
                .borrow_mut()
                .remove(&raw_id)
                .is_some_and(|candidate| {
                    candidate.started_in_text_input
                        && (position - candidate.start_position).magnitude()
                            <= f64::from(GestureTuning::default().touch_slop)
                }),
            TouchPhase::Cancelled => {
                self.touch_tap_candidates.borrow_mut().remove(&raw_id);
                false
            }
            TouchPhase::Started => false,
        };
        // Native window geometry owns movement until release. Do not also pan
        // a scrollable custom-titlebar parent while moving the native window.
        let tap_allowed = if phase != TouchPhase::Moved || self.manipulation.get().is_none() {
            self.touch_scroll.borrow_mut().relay(
                TouchEvent {
                    id,
                    phase,
                    position,
                    predicted_position: None,
                    force: force.is_finite().then(|| force.clamp(0.0, 1.0)),
                },
                Instant::now(),
                |input| self.dispatch_input(input),
            )
        } else {
            false
        };
        self.update_touch_long_press_timer();
        self.request_touch_momentum_frame();
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.active_touches.borrow_mut().remove(&raw_id);
        }
        if should_reopen_keyboard && tap_allowed {
            // Focusing an already-focused GPUI input is otherwise a no-op.
            // OHOS can dismiss its IME without changing that focus, so an
            // editable tap must always be treated as a fresh show request.
            // Do not inspect DispatchEventResult::default_prevented here:
            // focusable GPUI elements set it specifically to stop ancestors
            // from stealing focus.
            self.request_keyboard();
        }
    }

    fn request_touch_momentum_frame(&self) {
        if self.touch_scroll.borrow().has_momentum()
            && let Some(scheduler) = &self.frame_scheduler
        {
            scheduler.request_frame();
        }
    }

    fn update_touch_long_press_timer(&self) {
        let pending = self
            .touch_scroll
            .borrow()
            .pending_long_press(Instant::now());
        let Some((id, delay)) = pending else {
            self.long_press_timer.borrow_mut().take();
            return;
        };
        if self
            .long_press_timer
            .borrow()
            .as_ref()
            .is_some_and(|(scheduled, _)| *scheduled == id)
        {
            return;
        }
        let touch_scroll = self.touch_scroll.clone();
        let callbacks = self.callbacks.clone();
        let task = self.foreground_executor.spawn(async move {
            smol::Timer::after(delay).await;
            touch_scroll.borrow_mut().offer_long_press(id, |input| {
                Self::dispatch_input_with_callbacks(&callbacks, input)
            });
        });
        *self.long_press_timer.borrow_mut() = Some((id, task));
    }

    fn cancel_touch_input(&self) {
        self.long_press_timer.borrow_mut().take();
        self.touch_scroll
            .borrow_mut()
            .cancel(|input| self.dispatch_input(input));
        self.active_touches.borrow_mut().clear();
        self.touch_tap_candidates.borrow_mut().clear();
    }

    fn touch_moved_beyond_tap_slop(&self, raw_id: i32, position: Point<Pixels>) -> bool {
        self.touch_tap_candidates
            .borrow()
            .get(&raw_id)
            .is_some_and(|candidate| {
                (position - candidate.start_position).magnitude()
                    > f64::from(GestureTuning::default().touch_slop)
            })
    }

    fn pointer_targets_text_input(&self, position: Point<Pixels>) -> bool {
        let Some(mut handler) = self.input_handler.borrow_mut().take() else {
            return false;
        };
        let targets_text_input = handler.query_accepts_text_input()
            && handler
                .element_bounds()
                .is_some_and(|bounds| bounds.contains(&position));
        *self.input_handler.borrow_mut() = Some(handler);
        targets_text_input
    }

    fn dispatch_raw_touch_data(&self, event: &TouchEventData, point: &TouchPointData) {
        let phase = match event.event_type {
            OhosTouchEvent::Down => TouchPhase::Started,
            OhosTouchEvent::Move => TouchPhase::Moved,
            OhosTouchEvent::Up => TouchPhase::Ended,
            OhosTouchEvent::Cancel => TouchPhase::Cancelled,
            OhosTouchEvent::Unknown => return,
        };
        self.dispatch_raw_touch_point(point.id, point.x, point.y, point.force, phase);
    }

    fn dispatch_raw_touch_event(&self, event: &TouchEventData) {
        match event.event_type {
            OhosTouchEvent::Move | OhosTouchEvent::Cancel if !event.touch_points.is_empty() => {
                for point in &event.touch_points {
                    self.dispatch_raw_touch_data(event, point);
                }
            }
            OhosTouchEvent::Down | OhosTouchEvent::Up => {
                if let Some(point) = event.touch_points.iter().find(|point| point.id == event.id) {
                    self.dispatch_raw_touch_data(event, point);
                } else {
                    let phase = if event.event_type == OhosTouchEvent::Down {
                        TouchPhase::Started
                    } else {
                        TouchPhase::Ended
                    };
                    self.dispatch_raw_touch_point(event.id, event.x, event.y, event.force, phase);
                }
            }
            OhosTouchEvent::Move | OhosTouchEvent::Cancel => {
                let phase = if event.event_type == OhosTouchEvent::Move {
                    TouchPhase::Moved
                } else {
                    TouchPhase::Cancelled
                };
                self.dispatch_raw_touch_point(event.id, event.x, event.y, event.force, phase);
            }
            OhosTouchEvent::Unknown => {}
        }
    }

    fn dispatch_scroll(
        &self,
        position: Point<Pixels>,
        delta: Point<Pixels>,
        touch_phase: TouchPhase,
    ) -> crate::DispatchEventResult {
        Self::dispatch_input_with_callbacks(
            &self.callbacks,
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(delta),
                modifiers: self.key_state.borrow().modifiers(),
                touch_phase,
            }),
        )
    }

    fn mouse_button(button: OhosMouseButton) -> Option<MouseButton> {
        match button {
            OhosMouseButton::NoneButton => None,
            OhosMouseButton::LeftButton => Some(MouseButton::Left),
            OhosMouseButton::RightButton => Some(MouseButton::Right),
            OhosMouseButton::MiddleButton => Some(MouseButton::Middle),
            OhosMouseButton::BackButton => Some(MouseButton::Navigate(NavigationDirection::Back)),
            OhosMouseButton::ForwardButton => {
                Some(MouseButton::Navigate(NavigationDirection::Forward))
            }
        }
    }

    fn show_keyboard_if_needed(&self) {
        if !self.keyboard_visible.get() {
            self.request_keyboard();
        }
    }

    fn request_keyboard(&self) {
        self.apply_ime_action();
        if let Some(app) = self.app.borrow().as_ref() {
            match app.try_show_keyboard_for(self.window_id) {
                Ok(()) => self.keyboard_visible.set(true),
                Err(error) => {
                    self.keyboard_visible.set(false);
                    warn!(
                        "Failed to show OHOS IME for window {}: {error}",
                        self.window_id
                    );
                }
            }
        }
    }

    fn configure_text_input(&self, configuration: TextInputConfiguration) {
        // Configuration changes during input dispatch must not mutably borrow
        // OhosWindow. The native editor also retains hints while detached.
        // The SDK has no autocorrect, capitalization, or suggestion flags.
        if self.ime_action.replace(configuration.input_action) != configuration.input_action {
            self.apply_ime_action();
        }
    }

    fn apply_ime_action(&self) {
        let action = self.ime_action.get();
        let enter_key = match action {
            TextInputAction::Unspecified => EnterKey::Unspecified,
            TextInputAction::Enter => EnterKey::Newline,
            TextInputAction::Done => EnterKey::Done,
            TextInputAction::Go => EnterKey::Go,
            TextInputAction::Next => EnterKey::Next,
            TextInputAction::Previous => EnterKey::Previous,
            TextInputAction::Search => EnterKey::Search,
            TextInputAction::Send => EnterKey::Send,
        };
        let input_type = if action == TextInputAction::Enter {
            InputType::Multiline
        } else {
            InputType::Text
        };
        if let Some(app) = self.app.borrow().as_ref() {
            match app.configure_ime_for(self.window_id, enter_key, input_type) {
                Ok(()) => info!(
                    "OHOS IME configured window={} action={action:?}",
                    self.window_id
                ),
                Err(error) => warn!(
                    "Failed to configure OHOS IME for window {}: {error}",
                    self.window_id
                ),
            }
        }
    }

    fn hide_keyboard_if_needed(&self) {
        if self.keyboard_visible.replace(false)
            && let Some(app) = self.app.borrow().as_ref()
        {
            app.hide_keyboard_for(self.window_id);
        }
    }

    fn notify_keyboard_hidden_by_user_if_needed(&self) {
        self.keyboard_visible.set(false);
    }

    fn keyboard_inset_for_overlap(&self, overlap_device_px: i32) -> Pixels {
        const MIN_CONTENT_HEIGHT: f32 = 64.0;

        let overlap = overlap_device_px.max(0) as f32;
        let scale = self.scale_factor().max(1.0);
        let mut inset = (overlap / scale).max(0.0);
        let bounds_height = self.bounds.borrow().size.height.as_f32().max(0.0);
        let max_inset = (bounds_height - MIN_CONTENT_HEIGHT).max(0.0);
        if inset > max_inset {
            inset = max_inset;
        }
        px(inset)
    }

    fn keyboard_overlap_from_avoid_area_device_px(&self) -> Option<i32> {
        let app_ref = self.app.borrow();
        let app = app_ref.as_ref()?;

        let content_rect = app.content_rect_for(self.window_id);
        if content_rect.height <= 0 {
            return Some(0);
        }

        // Use actual XComponent rect as layout basis for keyboard-avoid computation.
        // This keeps behavior correct for embedded/non-fullscreen XComponents.
        let layout_top = content_rect.top;
        let layout_height = content_rect.height.max(0);
        if layout_height <= 0 {
            return Some(0);
        }
        let window_rect = app.window_rect_for(self.window_id);
        let window_top = window_rect.top;
        let window_bottom = window_rect.top.saturating_add(window_rect.height.max(0));

        let keyboard_area = app.avoid_area_for(self.window_id, AvoidAreaType::Keyboard);
        let system_area = app.avoid_area_for(self.window_id, AvoidAreaType::System);
        let system_gesture_area = app.avoid_area_for(self.window_id, AvoidAreaType::SystemGesture);
        let navigation_indicator_area =
            app.avoid_area_for(self.window_id, AvoidAreaType::NavigationIndicator);

        // OHOS avoid-area bottomRect coordinates are in window/screen space.
        // XComponent's content_rect can be reported in safe-content coordinates on some devices.
        // For root full-width layouts, infer top-safe offset so intersection uses a consistent space.
        let root_layout_width_matches_window = content_rect.width > 0
            && window_rect.width > 0
            && (content_rect.width - window_rect.width).abs() <= 1;
        let can_infer_root_safe_top = layout_top == 0
            && layout_height > 0
            && window_rect.height >= layout_height
            && root_layout_width_matches_window;
        let inferred_outside_bottom_safe = if can_infer_root_safe_top {
            let bottom_safe_overlap = |area: Option<openharmony_ability::AvoidArea>| -> i32 {
                let Some(area) = area else {
                    return 0;
                };
                if !area.visible || area.bottom_rect.height <= 0 {
                    return 0;
                }
                let start = area.bottom_rect.top;
                let end = area
                    .bottom_rect
                    .top
                    .saturating_add(area.bottom_rect.height.max(0));
                if end < window_bottom {
                    return 0;
                }
                (window_bottom - start)
                    .max(0)
                    .min(area.bottom_rect.height.max(0))
            };

            bottom_safe_overlap(system_area)
                .max(bottom_safe_overlap(system_gesture_area))
                .max(bottom_safe_overlap(navigation_indicator_area))
        } else {
            0
        };
        let inferred_top_safe = if can_infer_root_safe_top {
            (window_rect.height.max(0) - layout_height - inferred_outside_bottom_safe).max(0)
        } else {
            0
        };
        // Convert GPUI layout bounds to screen space before intersection.
        let layout_top_screen = window_top
            .saturating_add(inferred_top_safe)
            .saturating_add(layout_top);
        let layout_bottom_screen = layout_top_screen.saturating_add(layout_height);

        let keyboard_avoid_visible = keyboard_area.map(|a| a.visible).unwrap_or(false);
        // A show request does not prove that an on-screen keyboard occupies
        // the window. Without keyboard geometry, system/navigation bars are
        // only safe-area insets; counting them here shrinks the viewport twice.
        if !keyboard_area.is_some_and(|area| area.bottom_rect.height > 0) {
            return Some(0);
        }
        if !(self.keyboard_visible.get() || keyboard_avoid_visible) {
            return Some(0);
        }

        // Keyboard event only determines show/hide state.
        // Actual inset is derived from avoid-area geometry.
        // When keyboard is shown, include bottom occlusion union of:
        // - Keyboard area
        // - System bottom area (3-button navigation etc.)
        // - System gesture area
        // - Navigation indicator area
        // This prevents under-subtraction where keyboard area excludes nav area.
        let mut intervals: Vec<(i32, i32)> = Vec::with_capacity(4);
        let mut push_bottom_overlap_interval =
            |area: openharmony_ability::AvoidArea, require_visible: bool| {
                if area.bottom_rect.height <= 0 {
                    return;
                }
                if require_visible && !area.visible {
                    return;
                }
                let start = area.bottom_rect.top.max(layout_top_screen);
                let end = area
                    .bottom_rect
                    .top
                    .saturating_add(area.bottom_rect.height.max(0))
                    .min(layout_bottom_screen);
                if end > start {
                    intervals.push((start, end));
                }
            };

        if let Some(area) = keyboard_area {
            push_bottom_overlap_interval(area, true);
        }
        if let Some(area) = system_area {
            push_bottom_overlap_interval(area, false);
        }
        if let Some(area) = system_gesture_area {
            push_bottom_overlap_interval(area, false);
        }
        if let Some(area) = navigation_indicator_area {
            push_bottom_overlap_interval(area, false);
        }

        if intervals.is_empty() {
            return Some(0);
        }

        intervals.sort_unstable_by_key(|(start, _)| *start);
        let mut union_overlap = 0i32;
        let mut current = intervals[0];
        for &(start, end) in intervals.iter().skip(1) {
            if start <= current.1 {
                current.1 = current.1.max(end);
            } else {
                union_overlap = union_overlap.saturating_add(current.1 - current.0);
                current = (start, end);
            }
        }
        union_overlap = union_overlap.saturating_add(current.1 - current.0);

        let geometric_overlap = union_overlap.min(layout_height.max(0));
        let clamped_overlap = geometric_overlap;

        Some(clamped_overlap)
    }

    fn refresh_keyboard_overlap_device_px(&self) -> bool {
        let previous_overlap = self.keyboard_overlap_device_px.get();
        let next_overlap = self
            .keyboard_overlap_from_avoid_area_device_px()
            .unwrap_or(0)
            .max(0);
        if previous_overlap != next_overlap {
            self.keyboard_overlap_device_px.set(next_overlap);
            true
        } else {
            false
        }
    }

    fn current_safe_area_insets(&self) -> WindowInsets {
        let Some(app) = self.app.borrow().clone() else {
            return WindowInsets::default();
        };
        let content = app.content_rect_for(self.window_id);
        let window = app.window_rect_for(self.window_id);
        let mut top = 0_i32;
        let mut right = 0_i32;
        let mut bottom = 0_i32;
        let mut left = 0_i32;
        for kind in [
            AvoidAreaType::System,
            AvoidAreaType::Cutout,
            AvoidAreaType::NavigationIndicator,
        ] {
            if let Some(area) = app.avoid_area_for(self.window_id, kind)
                && area.visible
            {
                top = top.max(area.top_rect.height.max(0));
                right = right.max(area.right_rect.width.max(0));
                bottom = bottom.max(area.bottom_rect.height.max(0));
                left = left.max(area.left_rect.width.max(0));
            }
        }

        let scale = self.scale_factor().max(1.0);
        let applied_top = content.top.max(0);
        let applied_left = content.left.max(0);
        let applied_right = (window.width - content.width - content.left).max(0);
        let applied_bottom = (window.height - content.height - content.top).max(0);
        WindowInsets {
            safe_area: Edges {
                top: px((top - applied_top).max(0) as f32 / scale),
                right: px((right - applied_right).max(0) as f32 / scale),
                bottom: px((bottom - applied_bottom).max(0) as f32 / scale),
                left: px((left - applied_left).max(0) as f32 / scale),
            },
            // Keyboard overlap is represented by visual_viewport_bounds.
            ime: Edges::default(),
        }
    }

    fn refresh_insets(&self) {
        let next = self.current_safe_area_insets();
        if *self.insets.borrow() == next {
            return;
        }
        *self.insets.borrow_mut() = next.clone();
        let mut callback = self.callbacks.borrow_mut().insets_changed.take();
        if let Some(ref mut callback) = callback {
            callback(next);
        }
        self.callbacks.borrow_mut().insets_changed = callback;
    }

    fn update_visibility(&self, next: WindowVisibility) {
        if next != WindowVisibility::Visible {
            self.cancel_touch_input();
        }
        if let Some(scheduler) = &self.frame_scheduler {
            scheduler.set_active(next == WindowVisibility::Visible && self.surface_available.get());
        }
        let changed = self.visibility.replace(next) != next;
        self.sync_frame_delivery();
        if !changed {
            return;
        }
        let mut callback = self.callbacks.borrow_mut().visibility_change.take();
        if let Some(ref mut callback) = callback {
            callback(next);
        }
        self.callbacks.borrow_mut().visibility_change = callback;
    }

    pub(crate) fn back_handler_state(&self) -> (bool, BackHandler) {
        (self.back_enabled.get(), self.back_handler.clone())
    }

    fn set_hovered(&self, hovered: bool) {
        if self.hovered.replace(hovered) == hovered {
            return;
        }
        let mut callback = self.callbacks.borrow_mut().hover_status_change.take();
        if let Some(ref mut callback) = callback {
            callback(hovered);
        }
        self.callbacks.borrow_mut().hover_status_change = callback;
    }

    fn window_client(&self) -> Option<WindowClient> {
        let app = self.app.borrow().clone()?;
        match WindowClient::new(&app) {
            Ok(client) => Some(client),
            Err(error) => {
                warn!("Cannot access OHOS window bridge: {error}");
                None
            }
        }
    }

    fn record_screen_pointer(&self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            let pointer = (x.round() as i64, y.round() as i64);
            self.pointer_screen_position.set(Some(pointer));
            self.update_manipulation(pointer);
        }
    }

    fn begin_manipulation(&self, edge: Option<ResizeEdge>) {
        if self.closed.get()
            || self.maximized.get()
            || self.fullscreen.get()
            || (edge.is_some() && !self.is_resizable)
            || (edge.is_none() && !self.is_movable)
            || (self.pressed_mouse_button.get() != Some(MouseButton::Left)
                && self.active_touches.borrow().is_empty())
        {
            return;
        }
        let Some(pointer) = self.pointer_screen_position.get() else {
            return;
        };
        let Some(initial) = self.current_native_state() else {
            return;
        };
        debug!("Geometry begin: initial={initial:?} pointer={pointer:?}");
        self.manipulation
            .set(Some(super::window_manipulation::WindowManipulation {
                initial,
                pointer,
                edge,
            }));
    }

    fn update_manipulation(&self, pointer: (i64, i64)) {
        let Some(session) = self.manipulation.get() else {
            return;
        };
        let minimum = self
            .min_size
            .map(|size| {
                (
                    (size.width.as_f32() * self.scale_factor()).ceil().max(1.0) as i64,
                    (size.height.as_f32() * self.scale_factor()).ceil().max(1.0) as i64,
                )
            })
            .unwrap_or((1, 1));
        *self.geometry_request.borrow_mut() = Some(session.update(pointer, minimum));
        if self.geometry_pending.get() {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        self.geometry_pending.set(true);
        let requests = self.geometry_request.clone();
        let pending = self.geometry_pending.clone();
        let window_id = self.window_id;
        let resizing = session.edge.is_some();
        self.foreground_executor
            .spawn(async move {
                loop {
                    let Some(rect) = requests.borrow_mut().take() else {
                        break;
                    };
                    let result = async {
                        if resizing {
                            client
                                .resize_window(window_id, rect.width, rect.height)
                                .await?;
                        }
                        client
                            .move_window_to(window_id, rect.left, rect.top)
                            .await?;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    if let Err(error) = result {
                        warn!("OHOS window manipulation failed: {error}");
                        break;
                    }
                }
                pending.set(false);
            })
            .detach();
    }

    fn hit_window_control(&self) -> Option<WindowControlArea> {
        let mut callback = self.callbacks.borrow_mut().hit_test_window_control.take();
        let area = callback.as_mut().and_then(|callback| callback());
        self.callbacks.borrow_mut().hit_test_window_control = callback;
        area
    }

    fn current_native_state(&self) -> Option<super::window_state::NativeWindowState> {
        let app = self.app.borrow().clone()?;
        let rect = app.window_rect_for(self.window_id);
        (rect.width > 0 && rect.height > 0).then_some(super::window_state::NativeWindowState {
            left: i64::from(rect.left),
            top: i64::from(rect.top),
            width: i64::from(rect.width),
            height: i64::from(rect.height),
            maximized: self.maximized.get(),
            fullscreen: self.fullscreen.get(),
        })
    }

    fn remember_floating_state(&self) {
        if !self.maximized.get() && !self.fullscreen.get() {
            self.floating_state.set(self.current_native_state());
        }
    }

    fn apply_requested_background(&self) {
        if self.requested_background.get().is_none()
            || self.background_pending.get()
            || !self
                .app
                .borrow()
                .as_ref()
                .is_some_and(|app| app.native_window_for(self.window_id).is_some())
        {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        self.background_pending.set(true);
        let requested = self.requested_background.clone();
        let current = self.background_appearance.clone();
        let pending = self.background_pending.clone();
        let generation = self.background_generation.clone();
        let expected_generation = generation.get();
        let window_id = self.window_id;
        let window_appearance = self.appearance.get();
        self.foreground_executor
            .spawn(async move {
                while generation.get() == expected_generation {
                    let Some(appearance) = requested.get() else {
                        break;
                    };
                    let color = if appearance == WindowBackgroundAppearance::Opaque {
                        if matches!(
                            window_appearance,
                            WindowAppearance::Dark | WindowAppearance::VibrantDark
                        ) {
                            0xff000000
                        } else {
                            0xffffffff
                        }
                    } else {
                        0x00000000
                    };
                    let result = client.set_window_background_color(window_id, color).await;
                    if generation.get() != expected_generation {
                        return;
                    }
                    match result {
                        Ok(()) => {
                            let radius = if appearance == WindowBackgroundAppearance::Blurred {
                                24.0
                            } else {
                                0.0
                            };
                            match client.set_window_blur(window_id, radius).await {
                                Ok(()) => current.set(appearance),
                                Err(error) => {
                                    warn!("Failed to set OHOS window backdrop blur: {error}");
                                    if appearance == WindowBackgroundAppearance::Blurred {
                                        current.set(WindowBackgroundAppearance::Transparent);
                                    }
                                }
                            }
                        }
                        Err(error) => warn!("Failed to set OHOS window background: {error}"),
                    }
                    if requested.get() == Some(appearance) {
                        break;
                    }
                }
                if generation.get() == expected_generation {
                    pending.set(false);
                }
            })
            .detach();
    }

    pub(crate) fn apply_window_options(&self) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        if app.native_window_for(self.window_id).is_none() {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        let title = self.title.borrow().clone();
        let min_size = self.min_size.map(|size| {
            (
                (size.width.as_f32() * self.scale_factor()).ceil().max(1.0) as i64,
                (size.height.as_f32() * self.scale_factor()).ceil().max(1.0) as i64,
            )
        });
        let flags =
            1 | if self.is_resizable { 2 | 8 } else { 0 } | if self.is_minimizable { 4 } else { 0 };
        let movable = self.is_movable;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.set_window_title(window_id, title).await {
                    warn!("Failed to apply OHOS initial title: {error}");
                }
                if let Err(error) = client.set_window_decoration_flags(window_id, flags).await {
                    warn!("Failed to apply OHOS window controls: {error}");
                }
                if let Some((width, height)) = min_size
                    && let Err(error) = client
                        .set_window_limits(window_id, width, height, 0, 0)
                        .await
                {
                    warn!("Failed to apply OHOS window minimum size: {error}");
                }
                // Older SDKs have no draggable switch; custom GPUI dragging still checks this flag.
                if !movable && let Err(error) = client.set_window_draggable(window_id, false).await
                {
                    warn!("Failed to disable OHOS native window dragging: {error}");
                }
            })
            .detach();
    }

    fn apply_restored_state(&self) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        if app.native_window_for(self.window_id).is_none() || self.restore_pending.get() {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        if self.restore_state.borrow().is_none() {
            return;
        }
        self.restore_pending.set(true);
        let window_id = self.window_id;
        let fullscreen = self.fullscreen.clone();
        let maximized = self.maximized.clone();
        let pending = self.restore_pending.clone();
        let requests = self.restore_state.clone();
        self.foreground_executor
            .spawn(async move {
                loop {
                    let Some(state) = requests.borrow_mut().take() else {
                        break;
                    };
                    let result = async {
                        if fullscreen.get() {
                            client.set_fullscreen(window_id, false).await?;
                        }
                        if maximized.get() {
                            client.recover_window(window_id).await?;
                        }
                        client
                            .move_window_to(window_id, state.left, state.top)
                            .await?;
                        client
                            .resize_window(window_id, state.width, state.height)
                            .await?;
                        if state.fullscreen {
                            client.set_fullscreen(window_id, true).await?;
                        } else if state.maximized {
                            client.maximize_window(window_id).await?;
                        }
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    match result {
                        Ok(()) => {
                            fullscreen.set(state.fullscreen);
                            maximized.set(state.maximized && !state.fullscreen);
                        }
                        Err(error) => warn!("Failed to restore OHOS native window state: {error}"),
                    }
                }
                pending.set(false);
            })
            .detach();
    }

    fn request_resize(&self, size: Size<Pixels>) {
        let Some(client) = self.window_client() else {
            return;
        };
        let scale = self.scale_factor();
        let width = (size.width.as_f32() * scale).round() as i64;
        let height = (size.height.as_f32() * scale).round() as i64;
        if width <= 0 || height <= 0 {
            return;
        }
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.resize_window(window_id, width, height).await {
                    warn!("Failed to resize OHOS window {window_id}: {error}");
                }
            })
            .detach();
    }

    fn restore_cursor_after_move(&self) {
        if !self.cursor_hidden_until_move.replace(false) {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.set_cursor_visible(true).await {
                    warn!("Failed to restore OHOS cursor: {error}");
                }
            })
            .detach();
    }

    fn effective_content_size(&self) -> Size<Pixels> {
        let bounds_size = self.bounds.borrow().size;
        let bounds_height = bounds_size.height.as_f32().max(0.0);
        let keyboard_inset = self
            .keyboard_inset_for_overlap(self.keyboard_overlap_device_px.get())
            .as_f32();
        size(
            bounds_size.width,
            px((bounds_height - keyboard_inset).max(0.0)),
        )
    }

    fn emit_resize_callback(&self) {
        let scale = *self.scale.borrow();
        let content_size = self.effective_content_size();
        let mut callback = self.callbacks.borrow_mut().resize.take();
        self.viewport.publish(content_size, scale, &mut callback);
        self.callbacks.borrow_mut().resize = callback;
    }

    fn emit_visual_viewport_changed(&self) {
        self.emit_resize_callback();
        let mut callback = self.callbacks.borrow_mut().visual_viewport_changed.take();
        if let Some(ref mut callback) = callback {
            callback();
        }
        self.callbacks.borrow_mut().visual_viewport_changed = callback;
    }

    /// Initialize the renderer when native_window becomes available (after SurfaceCreate event).
    /// This method gets the raw_window_handle from OpenHarmonyApp's native_window.
    pub(crate) fn initialize_renderer(&self) -> Result<()> {
        let mut renderer_guard = self.renderer.borrow_mut();
        if renderer_guard.is_some() {
            // Already initialized
            return Ok(());
        }

        // Get native_window from OpenHarmonyApp - it should be available after SurfaceCreate
        let app = self.app.borrow();
        let app_ref = app.as_ref().ok_or_else(|| {
            anyhow::anyhow!("OpenHarmonyApp not available when initializing renderer")
        })?;

        // Check that native_window is available - this is required for the renderer to work.
        // The actual window handle is obtained via HasWindowHandle trait implementation.
        let _native_window = app_ref.native_window_for(self.window_id).ok_or_else(|| {
            anyhow::anyhow!(
                "native_window not available yet - SurfaceCreate event may not have been received"
            )
        })?;

        // Get the actual window size from content_rect.
        // Using the correct size is important because mismatched sizes between
        // the surface configuration and the actual native_window can cause
        // rendering issues (stretched/cropped content, black borders, etc.)
        // even though create_platform_window_surface itself won't fail.
        let content_rect = app_ref.content_rect_for(self.window_id);
        let scale = self.scale_factor();
        let device_width = if content_rect.width > 0 {
            content_rect.width as u32
        } else {
            // Fallback to bounds if content_rect is not available yet
            (self.bounds.borrow().size.width.as_f32() * scale) as u32
        };
        let device_height = if content_rect.height > 0 {
            content_rect.height as u32
        } else {
            (self.bounds.borrow().size.height.as_f32() * scale) as u32
        };

        debug!(
            "OhosWindow: Initializing renderer with size {}x{}",
            device_width, device_height
        );

        // Update window bounds to match actual content_rect (convert device px -> logical px)
        if content_rect.width > 0 && content_rect.height > 0 {
            let logical_size = size(
                px(device_width as f32 / scale),
                px(device_height as f32 / scale),
            );
            let logical_origin = point(
                px(content_rect.left as f32 / scale),
                px(content_rect.top as f32 / scale),
            );
            *self.bounds.borrow_mut() = Bounds::new(logical_origin, logical_size);
        }

        let config = WgpuSurfaceConfig {
            size: Size {
                width: DevicePixels(device_width as i32),
                height: DevicePixels(device_height as i32),
            },
            transparent: true,
        };

        debug!(
            "OhosWindow: Surface config - width: {}, height: {}, transparent: true",
            device_width, device_height
        );

        // Debug: Check window handle before creating renderer
        match self.window_handle() {
            Ok(handle) => {
                debug!(
                    "OhosWindow: Window handle obtained successfully: {:?}",
                    handle.as_raw()
                );
            }
            Err(e) => {
                warn!("OhosWindow: Failed to get window handle: {:?}", e);
                return Err(anyhow::anyhow!("Window handle not available: {:?}", e));
            }
        }

        debug!("OhosWindow: Creating WgpuRenderer...");

        // Create renderer using the window's HasWindowHandle and HasDisplayHandle implementation
        // which will get the raw_window_handle from native_window
        let renderer = WgpuRenderer::new(&self.gpu_context, self, config, self.fallback_atlas.borrow().clone())
            .map_err(|e| {
                warn!("OhosWindow: WgpuRenderer::new failed: {}", e);
                anyhow::anyhow!("Failed to create Wgpu renderer: {}. Make sure native_window is available from OpenHarmonyApp.", e)
            })?;

        *self.fallback_atlas.borrow_mut() = Some(renderer.sprite_atlas().clone());
        *renderer_guard = Some(renderer);
        debug!("OhosWindow: Renderer initialized successfully");
        Ok(())
    }

    pub(crate) fn handle_event(&self, event: &Event) {
        let ended = self
            .native_drag
            .borrow()
            .as_ref()
            .is_some_and(|drag| drag.ended());
        if ended {
            info!(
                "OHOS native file drag ended; releasing action for window {}",
                self.window_id
            );
            self.native_drag.borrow_mut().take();
            self.dispatch_input(PlatformInput::FileDrop(crate::FileDropEvent::Ended));
        }
        match event {
            Event::SurfaceCreate => {
                self.surface_available.set(true);
                if let Some(scheduler) = &self.frame_scheduler {
                    scheduler.set_active(self.visibility.get() == WindowVisibility::Visible);
                }
                self.sync_frame_delivery();
                debug!("OhosWindow: SurfaceCreate event received - initializing renderer");
                // A recreated render surface owns a new native editor.
                self.apply_ime_action();
                self.request_decorations(self.requested_decorations.get());
                self.apply_window_options();
                self.background_generation
                    .set(self.background_generation.get().wrapping_add(1));
                self.background_pending.set(false);
                self.apply_requested_background();
                self.apply_restored_state();
                self.initialize_accessibility();
                // Initialize renderer when SurfaceCreate event is received
                // Note: on_finish_launching is handled at the platform level (OhosPlatform::handle_ohos_event)
                // before windows are created.
                match self.initialize_renderer() {
                    Ok(()) => {
                        debug!("OhosWindow: Renderer initialized successfully");
                    }
                    Err(e) => {
                        warn!(
                            "OhosWindow: Failed to initialize renderer: {}. Make sure native_window is available from OpenHarmonyApp.",
                            e
                        );
                    }
                }
                if self.refresh_keyboard_overlap_device_px() {
                    self.emit_visual_viewport_changed();
                }
                self.emit_resize_callback();
                self.refresh_insets();
            }
            Event::SurfaceDestroy => {
                self.cancel_touch_input();
                self.surface_available.set(false);
                if let Some(scheduler) = &self.frame_scheduler {
                    scheduler.set_active(false);
                }
                self.background_generation
                    .set(self.background_generation.get().wrapping_add(1));
                self.background_pending.set(false);
                self.native_drag.borrow_mut().take();
                self.manipulation.set(None);
                self.geometry_request.borrow_mut().take();
                if self.incoming_drag.replace(false) {
                    self.dispatch_input(PlatformInput::FileDrop(crate::FileDropEvent::Exited));
                }
                self.keyboard_visible.set(false);
                self.release_accessibility();
                self.renderer.borrow_mut().take();
                self.set_hovered(false);
            }
            Event::WindowResize {
                window_id,
                size: ohos_size,
            } if *window_id == self.window_id => {
                // openharmony-ability currently maps both the ArkTS windowSizeChange callback and
                // the XComponent surface callback to WindowResize. In a floating 2-in-1 window the
                // former includes the server-side title bar, while the native render surface does
                // not. Prefer the active XComponent rect whenever it is available so that the
                // renderer and GPUI viewport always use the drawable content size.
                let content_rect = self
                    .app
                    .borrow()
                    .as_ref()
                    .map(|app| app.content_rect_for(self.window_id))
                    .unwrap_or_default();
                let device_width = if content_rect.width > 0 {
                    content_rect.width
                } else {
                    ohos_size.width
                };
                let device_height = if content_rect.height > 0 {
                    content_rect.height
                } else {
                    ohos_size.height
                };
                if device_width != ohos_size.width || device_height != ohos_size.height {
                    debug!(
                        "OhosWindow: Normalizing window resize {}x{} to XComponent surface {}x{}",
                        ohos_size.width, ohos_size.height, device_width, device_height,
                    );
                }
                let scale = *self.scale.borrow();
                let width = device_width as f32;
                let height = device_height as f32;
                let new_size = size(px(width / scale), px(height / scale));
                let origin = self.bounds.borrow().origin;
                *self.bounds.borrow_mut() = Bounds::new(origin, new_size);
                self.refresh_keyboard_overlap_device_px();

                // Update renderer's drawable size
                if let Some(ref mut renderer) = *self.renderer.borrow_mut() {
                    let device_size = Size {
                        width: DevicePixels(width as i32),
                        height: DevicePixels(height as i32),
                    };
                    renderer.update_drawable_size(device_size);
                }
                self.emit_resize_callback();
                self.refresh_insets();
            }
            Event::ContentRectChange(info) if info.window_id == self.window_id => {
                let scale = self.scale_factor();
                let content_rect = self
                    .app
                    .borrow()
                    .as_ref()
                    .map(|app| app.content_rect_for(self.window_id))
                    .unwrap_or_default();
                let next_origin = point(
                    px((info.rect.left + content_rect.left) as f32 / scale),
                    px((info.rect.top + content_rect.top) as f32 / scale),
                );
                let mut bounds = self.bounds.borrow_mut();
                let moved = bounds.origin != next_origin;
                bounds.origin = next_origin;
                drop(bounds);
                if moved {
                    let mut callback = self.callbacks.borrow_mut().moved.take();
                    if let Some(ref mut callback) = callback {
                        callback();
                    }
                    self.callbacks.borrow_mut().moved = callback;
                }
                if self.refresh_keyboard_overlap_device_px() {
                    self.emit_visual_viewport_changed();
                }
                self.refresh_insets();
            }
            Event::AvoidAreaChange(info) => {
                if matches!(
                    info.area_type,
                    AvoidAreaType::Keyboard
                        | AvoidAreaType::System
                        | AvoidAreaType::SystemGesture
                        | AvoidAreaType::NavigationIndicator
                ) && self.refresh_keyboard_overlap_device_px()
                {
                    self.emit_visual_viewport_changed();
                }
                self.refresh_insets();
            }
            Event::Start => self.update_visibility(WindowVisibility::Visible),
            Event::Stop => self.update_visibility(WindowVisibility::Hidden),
            Event::WindowRedraw(_) => {
                // The platform event loop already drains pending GPUI VSync
                // frames. Drawing again for XComponent's continuous callback
                // can advance momentum and submit a second frame in one VSync.
                if self
                    .frame_scheduler
                    .as_ref()
                    .is_none_or(|scheduler| scheduler.state.failed())
                {
                    self.draw_requested_frame();
                }
            }
            Event::Input(input_event) => {
                self.handle_input_event(input_event);
            }
            Event::WindowFocusChanged { window_id, focused } => {
                let next = *focused && *window_id == self.window_id;
                if let Some(adapter) = self.a11y_adapter.borrow().as_ref()
                    && let Err(error) = adapter.set_host_focus_state(next)
                {
                    warn!("Failed to update OHOS accessibility focus: {error}");
                }
                if self.active.replace(next) != next {
                    self.invalidate_pending_edit();
                    let mut callback = self.callbacks.borrow_mut().active_status_change.take();
                    if let Some(ref mut cb) = callback {
                        cb(next);
                    }
                    self.callbacks.borrow_mut().active_status_change = callback;
                }
                if !next {
                    self.set_hovered(false);
                    self.pressed_mouse_button.set(None);
                    self.mouse_click.borrow_mut().take();
                    let modifiers_changed = self.key_state.borrow_mut().clear_pressed();
                    if let Some(event) = modifiers_changed {
                        self.dispatch_input(event);
                    }
                    self.hide_keyboard_if_needed();
                    if self.refresh_keyboard_overlap_device_px() {
                        self.emit_visual_viewport_changed();
                    }
                } else {
                    // GPUI can retain the editor's focus while another native
                    // window owns the process-wide IME. Returning to this window
                    // must reattach its editor even without a GPUI FocusGained.
                    let handler = self.input_handler.borrow_mut().take();
                    if let Some(mut handler) = handler {
                        let accepts_text = handler.query_accepts_text_input();
                        *self.input_handler.borrow_mut() = Some(handler);
                        if accepts_text {
                            self.request_keyboard();
                            self.schedule_ime_text_update();
                        }
                    }
                }
            }
            Event::ConfigChanged(configuration) => {
                let appearance = self
                    .appearance_override
                    .get()
                    .unwrap_or_else(|| appearance_for_color_mode(configuration.color_mode));
                self.set_appearance(appearance);
                let new_scale = self.display.scale_factor();
                let old_scale = self.scale.replace(new_scale);
                if old_scale != new_scale {
                    let mut bounds = self.bounds.borrow_mut();
                    let ratio = old_scale / new_scale;
                    bounds.size = size(bounds.size.width * ratio, bounds.size.height * ratio);
                    bounds.origin *= ratio;
                }
                self.refresh_keyboard_overlap_device_px();
                self.emit_resize_callback();
                self.refresh_insets();
            }
            Event::WindowDestroy => {
                if self.closed.replace(true) {
                    return;
                }
                if let Some(scheduler) = &self.frame_scheduler {
                    scheduler.set_active(false);
                }
                self.update_visibility(WindowVisibility::Hidden);
                self.active.set(false);
                self.set_hovered(false);
                if self.refresh_keyboard_overlap_device_px() {
                    self.emit_visual_viewport_changed();
                }
                // The native window has already been destroyed. A close veto
                // cannot restore it, so always release GPUI's window state.
                if let Some(callback) = self.callbacks.borrow_mut().close.take() {
                    callback();
                }
            }
            Event::KeyboardEvent(height) => {
                if *height <= 0 {
                    self.notify_keyboard_hidden_by_user_if_needed();
                } else {
                    self.keyboard_visible.set(true);
                }
                if self.refresh_keyboard_overlap_device_px() {
                    self.emit_visual_viewport_changed();
                }
            }
            _ => {}
        }
    }

    fn schedule_ime_text_update(&self) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        if self.ime_text_update_pending.replace(true) {
            return;
        }
        let pending = self.ime_text_update_pending.clone();
        let input_handler = self.input_handler.clone();
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                pending.set(false);
                // A queued snapshot can outlive a child window close. Avoid
                // querying GPUI's handler after its native surface is gone.
                if app.native_window_for(window_id).is_none() {
                    return;
                }
                let state = {
                    let Ok(mut guard) = input_handler.try_borrow_mut() else {
                        return;
                    };
                    let Some(handler) = guard.as_mut() else {
                        return;
                    };
                    let Some(selection) = handler.selected_text_range(true) else {
                        return;
                    };
                    let caret = if selection.reversed {
                        selection.range.start
                    } else {
                        selection.range.end
                    };
                    // Fetch bounded context, including one extra unit to detect
                    // documents exceeding the native selection-notification limit.
                    let start = if caret <= 8192 {
                        0
                    } else {
                        caret.saturating_sub(4096)
                    };
                    let mut adjusted = None;
                    let Some(text) =
                        handler.text_for_range(start..start.saturating_add(8193), &mut adjusted)
                    else {
                        return;
                    };
                    TextState {
                        text,
                        offset: adjusted.map_or(start, |range| range.start),
                        selection: selection.range,
                        reversed: selection.reversed,
                    }
                };
                if app.native_window_for(window_id).is_some()
                    && let Err(error) = app.update_ime_text_state_for(window_id, state)
                {
                    warn!("Failed to update OHOS IME text for window {window_id}: {error}");
                }
            })
            .detach();
    }

    pub(crate) fn handle_input_event(&self, event: &InputEvent) {
        match event {
            InputEvent::Ime(ime_event) => {
                if let ImeEvent::ExtendActionEvent(action) = ime_event {
                    let editing = self.text_editing();
                    let action = *action;
                    let expected_generation = editing.edit_generation.get();
                    // Keep command ordering with queued selection/text callbacks.
                    self.foreground_executor
                        .spawn(async move {
                            if editing.edit_generation.get() == expected_generation
                                && editing.app.borrow().as_ref().is_some_and(|app| {
                                    app.native_window_for(editing.window_id).is_some()
                                })
                            {
                                editing.dispatch_edit_action(action);
                            }
                        })
                        .detach();
                    self.schedule_ime_text_update();
                    return;
                }
                if !matches!(ime_event, ImeEvent::ImeStatusEvent(_)) {
                    self.invalidate_pending_edit();
                }
                if matches!(
                    ime_event,
                    ImeEvent::ImeStatusEvent(openharmony_ability::ime::KeyboardStatus::Hide)
                ) {
                    self.notify_keyboard_hidden_by_user_if_needed();
                    if self.refresh_keyboard_overlap_device_px() {
                        self.emit_visual_viewport_changed();
                    }
                }

                let handler_ref = self.input_handler.clone();
                let callbacks = self.callbacks.clone();
                let ime_event = ime_event.clone();
                let executor = self.foreground_executor.clone();

                executor
                    .spawn(async move {
                        if let ImeEvent::MoveCursorEvent(direction) = ime_event {
                            let key = match direction {
                                ImeDirection::Up => "up",
                                ImeDirection::Down => "down",
                                ImeDirection::Left => "left",
                                ImeDirection::Right => "right",
                                ImeDirection::None => return,
                            };
                            Self::dispatch_input_with_callbacks(
                                &callbacks,
                                PlatformInput::KeyDown(KeyDownEvent {
                                    keystroke: Keystroke {
                                        key: key.into(),
                                        key_char: None,
                                        modifiers: Modifiers::default(),
                                    },
                                    is_held: false,
                                    prefer_character_input: false,
                                }),
                            );
                            return;
                        }
                        if let ImeEvent::EnterEvent(action) = ime_event {
                            if let Some(handler) = handler_ref.borrow_mut().as_mut() {
                                handler.unmark_text();
                            }
                            // Confirm keys must reach application key bindings;
                            // Search/Send/Next must not silently insert a newline.
                            let result = Self::dispatch_input_with_callbacks(
                                &callbacks,
                                PlatformInput::KeyDown(KeyDownEvent {
                                    keystroke: Keystroke {
                                        key: "enter".into(),
                                        key_char: None,
                                        modifiers: Modifiers::default(),
                                    },
                                    is_held: false,
                                    prefer_character_input: false,
                                }),
                            );
                            if result.propagate
                                && matches!(
                                    EnterKey::from(action as u32),
                                    EnterKey::Newline | EnterKey::Unspecified
                                )
                                && let Some(handler) = handler_ref.borrow_mut().as_mut()
                            {
                                handler.replace_text_in_range(None, "\n");
                            }
                            return;
                        }
                        let mut handler_guard = handler_ref.borrow_mut();
                        let Some(handler) = handler_guard.as_mut() else {
                            return;
                        };

                        match ime_event {
                            ImeEvent::TextInputEvent(data) => {
                                handler.replace_text_in_range(None, &data.text);
                                handler.unmark_text();
                            }
                            ImeEvent::PreviewTextEvent { text, start, end } => {
                                let range = (start >= 0 && end >= start)
                                    .then_some(start as usize..end as usize);
                                handler.replace_and_mark_text_in_range(range, &text, None);
                            }
                            ImeEvent::FinishPreviewEvent => handler.unmark_text(),
                            ImeEvent::EnterEvent(_) => unreachable!("confirm key handled above"),
                            ImeEvent::MoveCursorEvent(_) => {
                                unreachable!("cursor key handled above")
                            }
                            ImeEvent::ExtendActionEvent(_) => {
                                unreachable!("edit command handled above")
                            }
                            ImeEvent::SelectionEvent { start, end } => {
                                if start >= 0 && end >= 0 {
                                    handler.unmark_text();
                                    handler.set_selected_text_range(
                                        start.min(end) as usize..start.max(end) as usize,
                                    );
                                }
                            }
                            ImeEvent::BackspaceEvent(len) | ImeEvent::DeleteForwardEvent(len) => {
                                let forward = matches!(ime_event, ImeEvent::DeleteForwardEvent(_));
                                let len = (len).max(0) as usize;
                                if len == 0 {
                                    return;
                                }

                                if let Some(selection) = handler.selected_text_range(true) {
                                    let range = if selection.range.start != selection.range.end {
                                        selection.range
                                    } else {
                                        let caret = if selection.reversed {
                                            selection.range.start
                                        } else {
                                            selection.range.end
                                        };
                                        // Read one extra UTF-16 unit so a one-unit
                                        // deletion cannot split a surrogate pair.
                                        let request = if forward {
                                            caret..caret.saturating_add(len).saturating_add(1)
                                        } else {
                                            caret.saturating_sub(len.saturating_add(1))..caret
                                        };
                                        let mut adjusted = None;
                                        let text = handler.text_for_range(request, &mut adjusted);
                                        let mut units = 0;
                                        if let Some(text) = text {
                                            if forward {
                                                for character in text.chars() {
                                                    units += character.len_utf16();
                                                    if units >= len {
                                                        break;
                                                    }
                                                }
                                            } else {
                                                for character in text.chars().rev() {
                                                    units += character.len_utf16();
                                                    if units >= len {
                                                        break;
                                                    }
                                                }
                                            }
                                        } else {
                                            units = len;
                                        }
                                        if forward {
                                            caret..caret.saturating_add(units)
                                        } else {
                                            caret.saturating_sub(units)..caret
                                        }
                                    };
                                    handler.replace_text_in_range(Some(range), "");
                                } else {
                                    handler.replace_text_in_range(None, "");
                                }
                            }
                            ImeEvent::ImeStatusEvent(status) => {
                                if matches!(status, openharmony_ability::ime::KeyboardStatus::Hide)
                                {
                                    handler.unmark_text();
                                }
                            }
                        }
                    })
                    .detach();
                self.schedule_ime_text_update();
            }
            InputEvent::XComponent(XComponentInputEvent::Mouse(mouse_event)) => {
                if mouse_event.screen_x.is_finite() && mouse_event.screen_y.is_finite() {
                    let absolute = (
                        mouse_event.screen_x.round() as i64,
                        mouse_event.screen_y.round() as i64,
                    );
                    self.pointer_screen_position.set(Some(absolute));
                    if mouse_event.action == MouseAction::Press {
                        if let Some(mut session) = self.manipulation.get() {
                            session.pointer = absolute;
                            self.manipulation.set(Some(session));
                        }
                    } else if mouse_event.action != MouseAction::Cancel {
                        self.update_manipulation(absolute);
                    }
                }
                let position = self.point_from_device_pixels(mouse_event.x, mouse_event.y);
                self.pointer_position.set(Some(position));
                let event_button = Self::mouse_button(mouse_event.button);
                match mouse_event.action {
                    MouseAction::Press => {
                        self.invalidate_pending_edit();
                        let Some(button) = event_button else {
                            return;
                        };
                        let now = Instant::now();
                        let tuning = GestureTuning::default();
                        let mut click = self.mouse_click.borrow_mut();
                        let click_count = click
                            .as_ref()
                            .filter(|last| {
                                last.button == button
                                    && now.duration_since(last.time) <= tuning.multi_tap_interval
                                    && (position - last.position).magnitude()
                                        <= f64::from(tuning.multi_tap_slop)
                            })
                            .map_or(1, |last| last.count.saturating_add(1));
                        *click = Some(MouseClickState {
                            button,
                            position,
                            time: now,
                            count: click_count,
                        });
                        drop(click);
                        self.pressed_mouse_button.set(Some(button));
                        self.dispatch_input(PlatformInput::MouseDown(MouseDownEvent {
                            button,
                            position,
                            modifiers: self.key_state.borrow().modifiers(),
                            click_count,
                            first_mouse: false,
                        }));
                        if button == MouseButton::Left
                            && matches!(self.hit_window_control(), Some(WindowControlArea::Drag))
                        {
                            self.begin_manipulation(None);
                        }
                    }
                    MouseAction::Release => {
                        self.manipulation.set(None);
                        let button = event_button.or(self.pressed_mouse_button.get());
                        self.pressed_mouse_button.set(None);
                        let Some(button) = button else {
                            return;
                        };
                        let reopen_keyboard = button == MouseButton::Left
                            && self.mouse_click.borrow().as_ref().is_some_and(|click| {
                                (position - click.position).magnitude()
                                    <= f64::from(GestureTuning::default().touch_slop)
                            })
                            && self.pointer_targets_text_input(position);
                        self.dispatch_input(PlatformInput::MouseUp(MouseUpEvent {
                            button,
                            position,
                            modifiers: self.key_state.borrow().modifiers(),
                            click_count: self
                                .mouse_click
                                .borrow()
                                .as_ref()
                                .filter(|click| click.button == button)
                                .map_or(1, |click| click.count),
                        }));
                        // Desktop/2in1 taps can arrive through the mouse callback.
                        // Reopen a retained text focus after a system IME dismissal.
                        if reopen_keyboard {
                            self.request_keyboard();
                        }
                    }
                    MouseAction::Move => {
                        self.restore_cursor_after_move();
                        if self.manipulation.get().is_some() {
                            return;
                        }
                        self.dispatch_input(PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            pressed_button: self.pressed_mouse_button.get().or(event_button),
                            modifiers: self.key_state.borrow().modifiers(),
                        }));
                    }
                    MouseAction::Cancel => {
                        self.manipulation.set(None);
                        self.pressed_mouse_button.set(None);
                        self.mouse_click.borrow_mut().take();
                    }
                    MouseAction::None => {}
                }
            }
            InputEvent::XComponent(XComponentInputEvent::Hover(hovered)) => {
                self.set_hovered(*hovered);
            }
            InputEvent::ArkUi(ArkUiInputEvent::Pointer(pointer)) => {
                if let Some(id) = pointer.pointer_id {
                    self.native_pointer_id.set(Some(id));
                }
                if pointer.action == UIInputAction::Down {
                    if pointer.display_x.is_finite() && pointer.display_y.is_finite() {
                        let absolute = (
                            pointer.display_x.round() as i64,
                            pointer.display_y.round() as i64,
                        );
                        self.pointer_screen_position.set(Some(absolute));
                        if let Some(mut session) = self.manipulation.get() {
                            session.pointer = absolute;
                            self.manipulation.set(Some(session));
                        }
                    }
                } else if pointer.source_type != UIInputSourceType::Mouse
                    && pointer.action != UIInputAction::Cancel
                {
                    self.record_screen_pointer(pointer.display_x, pointer.display_y);
                }
                if pointer.action == UIInputAction::Cancel
                    || (pointer.source_type != UIInputSourceType::Mouse
                        && pointer.action == UIInputAction::Up)
                {
                    self.manipulation.set(None);
                }
            }
            InputEvent::ArkUi(ArkUiInputEvent::Drag(drag)) => {
                use openharmony_ability::DragPhase;
                let offset = self
                    .app
                    .borrow()
                    .as_ref()
                    .map(|app| app.content_rect_for(self.window_id))
                    .unwrap_or_default();
                let position = self.point_from_device_pixels(
                    drag.window_x - offset.left as f32,
                    drag.window_y - offset.top as f32,
                );
                let paths = drag
                    .file_uris
                    .iter()
                    .filter_map(|uri| {
                        ohos_fileuri_binding::get_path_from_uri(uri)
                            .ok()
                            .map(std::path::PathBuf::from)
                    })
                    .collect::<smallvec::SmallVec<[_; 2]>>();
                if matches!(drag.phase, DragPhase::Enter | DragPhase::Drop) {
                    info!(
                        "OHOS native drag {:?} window={} uris={} paths={} at ({}, {})",
                        drag.phase,
                        self.window_id,
                        drag.file_uris.len(),
                        paths.len(),
                        drag.window_x,
                        drag.window_y
                    );
                }
                if matches!(drag.phase, DragPhase::Enter | DragPhase::Drop) && !paths.is_empty() {
                    self.incoming_drag.set(true);
                    self.dispatch_input(PlatformInput::FileDrop(crate::FileDropEvent::Entered {
                        position,
                        paths: crate::ExternalPaths(paths),
                    }));
                }
                match drag.phase {
                    DragPhase::Move if self.incoming_drag.get() => {
                        self.dispatch_input(PlatformInput::FileDrop(
                            crate::FileDropEvent::Pending { position },
                        ));
                    }
                    DragPhase::Drop if self.incoming_drag.replace(false) => {
                        let result = self.dispatch_input(PlatformInput::FileDrop(
                            crate::FileDropEvent::Submit { position },
                        ));
                        if !result.propagate || result.default_prevented {
                            drag.response.accept();
                        }
                        self.dispatch_input(PlatformInput::FileDrop(crate::FileDropEvent::Exited));
                    }
                    DragPhase::Leave if self.incoming_drag.replace(false) => {
                        self.dispatch_input(PlatformInput::FileDrop(crate::FileDropEvent::Exited));
                    }
                    _ => {}
                }
            }
            InputEvent::ArkUi(ArkUiInputEvent::Axis(axis_event)) => {
                let position = self.pointer_position_from_arkui(axis_event.pointer);
                self.pointer_position.set(Some(position));
                let raw_delta = point(axis_event.delta_x as f32, axis_event.delta_y as f32);
                let delta = if axis_event.pointer.tool_type == UIInputToolType::Touchpad {
                    self.point_from_device_pixels(raw_delta.x, raw_delta.y)
                } else {
                    raw_delta.map(px)
                };
                let touch_phase = match axis_event.pointer.action {
                    UIInputAction::Down => TouchPhase::Started,
                    UIInputAction::Up | UIInputAction::Cancel => TouchPhase::Ended,
                    UIInputAction::Move => TouchPhase::Moved,
                };
                if delta.x != px(0.0)
                    || delta.y != px(0.0)
                    || matches!(touch_phase, TouchPhase::Started | TouchPhase::Ended)
                {
                    self.dispatch_scroll(position, delta, touch_phase);
                }
            }
            InputEvent::ArkUi(ArkUiInputEvent::Gesture(
                openharmony_ability::GestureEvent::Pan(pan),
            )) => {
                let phase = match pan.phase {
                    openharmony_ability::GesturePhase::Start => TouchPhase::Started,
                    openharmony_ability::GesturePhase::Update => TouchPhase::Moved,
                    openharmony_ability::GesturePhase::End => TouchPhase::Ended,
                    openharmony_ability::GesturePhase::Cancel => TouchPhase::Cancelled,
                };
                let id = pan
                    .pointer
                    .pointer_id
                    .and_then(|id| self.active_touches.borrow().get(&id).copied());
                let delta = self.point_from_device_pixels(pan.delta_x, pan.delta_y);
                let velocity = self
                    .point_from_device_pixels(pan.velocity_x, pan.velocity_y)
                    .map(f32::from);
                self.touch_scroll.borrow_mut().native_pan(
                    NativePanInput {
                        id,
                        phase,
                        delta,
                        velocity,
                    },
                    Instant::now(),
                    |input| self.dispatch_input(input),
                );
                self.update_touch_long_press_timer();
                self.request_touch_momentum_frame();
            }
            // Raw contacts retain click/long-press and control capture support.
            // Pan End already contains ArkUI's velocity; Swipe must not launch
            // a second momentum curve for the same contact.
            InputEvent::ArkUi(ArkUiInputEvent::Gesture(_)) => {}
            InputEvent::XComponent(XComponentInputEvent::Touch(touch_event)) => {
                self.dispatch_raw_touch_event(touch_event);
            }
            InputEvent::XComponent(XComponentInputEvent::Key(key_event)) => {
                let inputs = self.key_state.borrow_mut().handle(key_event);
                for input in inputs {
                    self.dispatch_keyboard_input(input, false);
                }
            }
            InputEvent::ArkUi(ArkUiInputEvent::Key(key_event)) => {
                let inputs = self.key_state.borrow_mut().handle_arkui(key_event);
                for input in inputs {
                    if self.dispatch_keyboard_input(input, true) {
                        key_event.response.consume();
                    }
                }
            }
            InputEvent::ArkUi(ArkUiInputEvent::KeyPreIme(key_event)) => {
                let change = self.key_state.borrow_mut().handle_arkui_state(key_event);
                if let Some(change) = change {
                    self.dispatch_input(change);
                }
            }
        }
    }

    fn invalidate_pending_edit(&self) {
        self.edit_generation
            .set(self.edit_generation.get().wrapping_add(1));
    }

    fn dispatch_keyboard_input(&self, input: PlatformInput, after_ime: bool) -> bool {
        let character = if after_ime {
            match &input {
                PlatformInput::KeyDown(event) => event.keystroke.key_char.clone(),
                _ => None,
            }
        } else {
            None
        };
        let edit_action = if let PlatformInput::KeyDown(event) = &input {
            self.invalidate_pending_edit();
            let modifiers = event.keystroke.modifiers;
            if modifiers.control
                && !modifiers.alt
                && !modifiers.shift
                && !modifiers.platform
                && !modifiers.function
            {
                match event.keystroke.key.as_str() {
                    "a" => Some(ImeAction::SelectAll),
                    "x" => Some(ImeAction::Cut),
                    "c" => Some(ImeAction::Copy),
                    "v" => Some(ImeAction::Paste),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };
        let result = self.dispatch_input(input);
        let mut consumed = !result.propagate || result.default_prevented;
        if !consumed {
            if let Some(action) = edit_action {
                consumed = self.fallback_edit_action(action);
            } else if let Some(character) = character {
                // ArkUI sends this callback only after the IME declines the key.
                // Numeric-keypad/space input still needs a text insertion path.
                if let Some(handler) = self.input_handler.borrow_mut().as_mut()
                    && handler.query_accepts_text_input()
                {
                    handler.unmark_text();
                    handler.replace_text_in_range(None, &character);
                    consumed = true;
                }
                self.schedule_ime_text_update();
            }
        }
        consumed
    }

    fn text_editing(&self) -> OhosTextEditing {
        OhosTextEditing {
            app: self.app.clone(),
            callbacks: self.callbacks.clone(),
            input_handler: self.input_handler.clone(),
            edit_generation: self.edit_generation.clone(),
            foreground_executor: self.foreground_executor.clone(),
            clipboard: self.clipboard.clone(),
            window_id: self.window_id,
        }
    }

    fn fallback_edit_action(&self, action: ImeAction) -> bool {
        let accepted = self.text_editing().fallback_edit_action(action);
        self.schedule_ime_text_update();
        accepted
    }

    fn dispatch_input(&self, input: PlatformInput) -> crate::DispatchEventResult {
        Self::dispatch_input_with_callbacks(&self.callbacks, input)
    }
}

/// Owned main-thread edit context, also retained by queued native IME commands.
struct OhosTextEditing {
    app: Rc<RefCell<Option<OpenHarmonyApp>>>,
    callbacks: Rc<RefCell<WindowCallbacks>>,
    input_handler: Rc<RefCell<Option<PlatformInputHandler>>>,
    edit_generation: Rc<Cell<u64>>,
    foreground_executor: ForegroundExecutor,
    clipboard: super::clipboard::OhosClipboard,
    window_id: i64,
}

impl OhosTextEditing {
    fn invalidate_pending_edit(&self) {
        self.edit_generation
            .set(self.edit_generation.get().wrapping_add(1));
    }

    fn dispatch_input(&self, input: PlatformInput) -> crate::DispatchEventResult {
        OhosWindow::dispatch_input_with_callbacks(&self.callbacks, input)
    }

    fn dispatch_edit_action(&self, action: ImeAction) {
        let key = match action {
            ImeAction::SelectAll => "a",
            ImeAction::Cut => "x",
            ImeAction::Copy => "c",
            ImeAction::Paste => "v",
        };
        let result = self.dispatch_input(PlatformInput::KeyDown(KeyDownEvent {
            keystroke: Keystroke {
                key: key.into(),
                key_char: None,
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
            },
            is_held: false,
            prefer_character_input: false,
        }));
        if result.propagate && !result.default_prevented {
            self.fallback_edit_action(action);
        }
    }

    fn fallback_edit_action(&self, action: ImeAction) -> bool {
        self.invalidate_pending_edit();
        let (selection, selected_text, input_bounds) = {
            let mut guard = self.input_handler.borrow_mut();
            let Some(handler) = guard.as_mut() else {
                return false;
            };
            if !handler.query_accepts_text_input() {
                return false;
            }
            if action == ImeAction::SelectAll {
                if let Some(length) = handler.text_length_utf16() {
                    handler.set_selected_text_range(0..length);
                    return true;
                }
                return false;
            }
            let Some(selection) = handler.selected_text_range(false) else {
                return false;
            };
            if action != ImeAction::Paste && selection.range.is_empty() {
                return false;
            }
            let mut adjusted = None;
            let selected_text = handler.text_for_range(selection.range.clone(), &mut adjusted);
            if action != ImeAction::Paste
                && (selected_text
                    .as_ref()
                    .map(|text| text.encode_utf16().count())
                    != Some(selection.range.len())
                    || adjusted.is_some_and(|range| range != selection.range))
            {
                return false;
            }
            (selection, selected_text, handler.element_bounds())
        };
        let Some(app) = self.app.borrow().clone() else {
            return false;
        };
        let handler_ref = self.input_handler.clone();
        let generation = self.edit_generation.clone();
        let expected_generation = generation.get();
        let window_id = self.window_id;
        let clipboard = self.clipboard.clone();
        self.foreground_executor
            .spawn(async move {
                let replacement = if action == ImeAction::Paste {
                    match clipboard.read().await {
                        Ok(item) => item.and_then(|item| item.text()),
                        Err(error) => {
                            warn!("Failed to paste OHOS clipboard: {error}");
                            return;
                        }
                    }
                } else {
                    let Some(text) = &selected_text else {
                        return;
                    };
                    if let Err(error) = clipboard
                        .write(crate::ClipboardItem::new_string(text.clone()))
                        .await
                    {
                        warn!("Failed to copy OHOS selection: {error}");
                        return;
                    }
                    if action == ImeAction::Copy {
                        return;
                    }
                    Some(String::new())
                };
                let Some(replacement) = replacement else {
                    return;
                };
                // An async clipboard permission prompt/read can outlive the input.
                // Never replace another selection or delete text after a failed copy.
                if generation.get() != expected_generation
                    || app.native_window_for(window_id).is_none()
                {
                    return;
                }
                let mut guard = handler_ref.borrow_mut();
                let Some(handler) = guard.as_mut() else {
                    return;
                };
                if !handler.query_accepts_text_input()
                    || handler.element_bounds() != input_bounds
                    || handler.selected_text_range(false).is_none_or(|current| {
                        current.range != selection.range || current.reversed != selection.reversed
                    })
                {
                    return;
                }
                if let Some(original) = &selected_text {
                    let mut adjusted = None;
                    if handler
                        .text_for_range(selection.range.clone(), &mut adjusted)
                        .as_ref()
                        != Some(original)
                    {
                        return;
                    }
                }
                handler.unmark_text();
                handler.replace_text_in_range(Some(selection.range), &replacement);
            })
            .detach();
        true
    }
}

impl HasWindowHandle for OhosWindow {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        self.app
            .borrow()
            .as_ref()
            .and_then(|app| app.native_window_for(self.window_id))
            .and_then(|native_window| native_window.raw_window_handle())
            .map(|raw_handle| unsafe { raw_window_handle::WindowHandle::borrow_raw(raw_handle) })
            .ok_or(raw_window_handle::HandleError::Unavailable)
    }
}

impl HasWindowHandle for OhosWindowHandle {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let window = self.inner.borrow();
        window
            .app
            .borrow()
            .as_ref()
            .and_then(|app| app.native_window_for(window.window_id))
            .and_then(|native_window| native_window.raw_window_handle())
            .map(|raw_handle| unsafe { raw_window_handle::WindowHandle::borrow_raw(raw_handle) })
            .ok_or(raw_window_handle::HandleError::Unavailable)
    }
}

impl HasDisplayHandle for OhosWindow {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::ohos())
    }
}

impl HasDisplayHandle for OhosWindowHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::ohos())
    }
}

impl PlatformWindow for OhosWindowHandle {
    fn insets(&self) -> WindowInsets {
        self.with_window(|window| window.insets())
    }

    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        self.with_window(|window| window.on_insets_changed(callback))
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.with_window(|window| window.bounds())
    }

    fn is_maximized(&self) -> bool {
        self.with_window(|window| window.is_maximized())
    }

    fn window_bounds(&self) -> WindowBounds {
        self.with_window(|window| window.window_bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.with_window(|window| window.content_size())
    }

    fn visual_viewport_bounds(&self) -> Bounds<Pixels> {
        self.with_window(|window| window.visual_viewport_bounds())
    }

    fn on_visual_viewport_changed(&self, callback: Box<dyn FnMut()>) {
        self.with_window(|window| window.on_visual_viewport_changed(callback))
    }

    fn resize(&mut self, size: Size<Pixels>) {
        self.with_window(|window| window.request_resize(size))
    }

    fn scale_factor(&self) -> f32 {
        self.with_window(|window| window.scale_factor())
    }

    fn appearance(&self) -> WindowAppearance {
        self.with_window(|window| window.appearance())
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.with_window(|window| window.display())
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.with_window(|window| window.mouse_position())
    }

    fn modifiers(&self) -> Modifiers {
        self.with_window(|window| window.modifiers())
    }

    fn capslock(&self) -> Capslock {
        self.with_window(|window| window.capslock())
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        *self.input_handler.borrow_mut() = Some(input_handler);
        self.with_window(|window| window.schedule_ime_text_update());
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.input_handler.borrow_mut().take()
    }

    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        self.with_window(|window| window.configure_text_input(configuration));
    }

    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        self.with_window(|window| window.prompt(level, msg, detail, answers))
    }

    fn activate(&self) {
        self.with_window(|window| window.activate())
    }

    fn request_attention(&self) {
        self.with_window(|window| window.request_attention())
    }

    fn is_active(&self) -> bool {
        self.with_window(|window| window.is_active())
    }

    fn is_hovered(&self) -> bool {
        self.with_window(|window| window.is_hovered())
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.with_window(|window| window.background_appearance())
    }

    fn get_title(&self) -> String {
        self.with_window(|window| window.get_title())
    }
    fn native_window_state(&self) -> Option<Vec<u8>> {
        self.with_window(|window| window.native_window_state())
    }
    fn restore_native_window_state(&self, state: &[u8]) {
        self.with_window(|window| window.restore_native_window_state(state));
    }

    fn set_title(&mut self, title: &str) {
        self.with_window_mut(|window| window.set_title(title))
    }

    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        self.with_window(|window| window.set_background_appearance(background_appearance))
    }

    fn minimize(&self) {
        self.with_window(|window| window.minimize())
    }

    fn zoom(&self) {
        self.with_window(|window| window.zoom())
    }

    fn toggle_fullscreen(&self) {
        self.with_window(|window| window.toggle_fullscreen())
    }

    fn is_fullscreen(&self) -> bool {
        self.with_window(|window| window.is_fullscreen())
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        self.with_window(OhosWindow::frame_waker)
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.with_window(|window| window.on_request_frame(callback))
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> crate::DispatchEventResult>) {
        self.with_window(|window| window.on_input(callback))
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.with_window(|window| window.on_active_status_change(callback))
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.with_window(|window| window.on_hover_status_change(callback))
    }

    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        self.with_window(|window| window.set_back_handler(callback))
    }

    fn set_back_enabled(&self, enabled: bool) {
        self.with_window(|window| window.set_back_enabled(enabled))
    }

    fn a11y_init(&self, callbacks: A11yCallbacks) {
        self.with_window(|window| window.a11y_init(callbacks))
    }

    fn a11y_tree_update(&self, update: accesskit::TreeUpdate) {
        self.with_window(|window| window.a11y_tree_update(update))
    }

    fn visibility(&self) -> WindowVisibility {
        self.with_window(|window| window.visibility())
    }

    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.with_window(|window| window.on_visibility_change(callback))
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.with_window(|window| window.on_resize(callback))
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.with_window(|window| window.on_moved(callback))
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.with_window(|window| window.on_should_close(callback))
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.with_window(|window| window.on_hit_test_window_control(callback))
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.with_window(|window| window.on_close(callback))
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.with_window(|window| window.on_appearance_changed(callback))
    }

    fn draw(&self, scene: &Scene) {
        self.with_window(|window| window.draw(scene))
    }

    fn schedule_frame(&self) {
        self.with_window(|window| window.schedule_frame())
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.with_window(|window| window.sprite_atlas())
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.with_window(|window| window.gpu_specs())
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        self.with_window(|window| window.is_subpixel_rendering_supported())
    }

    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        self.with_window(|window| window.update_ime_position(bounds))
    }

    fn show_soft_keyboard(&self) {
        self.with_window(|window| window.show_soft_keyboard())
    }

    fn hide_soft_keyboard(&self) {
        self.with_window(|window| window.hide_soft_keyboard())
    }

    fn text_input_state_changed(&self, change: TextInputStateChange) {
        self.with_window(|window| window.text_input_state_changed(change))
    }

    fn request_decorations(&self, decorations: WindowDecorations) {
        self.with_window(|window| window.request_decorations(decorations))
    }

    fn show_window_menu(&self, position: Point<Pixels>) {
        self.with_window(|window| window.show_window_menu(position))
    }

    fn start_window_move(&self) {
        self.with_window(|window| window.start_window_move())
    }

    fn can_start_external_drag(&self) -> bool {
        self.with_window(|window| window.can_start_external_drag())
    }
    fn start_external_drag(&self, payload: &crate::ExternalDragPayload) -> bool {
        self.with_window(|window| window.start_external_drag(payload))
    }

    fn start_window_resize(&self, edge: ResizeEdge) {
        self.with_window(|window| window.start_window_resize(edge))
    }

    fn window_decorations(&self) -> crate::Decorations {
        self.with_window(|window| window.window_decorations())
    }

    fn set_app_id(&mut self, app_id: &str) {
        self.with_window_mut(|window| window.set_app_id(app_id))
    }

    fn map_window(&mut self) -> Result<()> {
        self.with_window_mut(|window| window.map_window())
    }

    fn window_controls(&self) -> WindowControls {
        self.with_window(|window| window.window_controls())
    }

    fn set_client_inset(&self, inset: Pixels) {
        self.with_window(|window| window.set_client_inset(inset))
    }
}

impl PlatformWindow for OhosWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        *self.bounds.borrow()
    }

    fn is_maximized(&self) -> bool {
        self.maximized.get()
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Windowed(*self.bounds.borrow())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.bounds.borrow().size
    }

    fn visual_viewport_bounds(&self) -> Bounds<Pixels> {
        Bounds::new(Point::default(), self.effective_content_size())
    }

    fn on_visual_viewport_changed(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().visual_viewport_changed = Some(callback);
    }

    fn resize(&mut self, size: Size<Pixels>) {
        self.request_resize(size);
    }

    fn scale_factor(&self) -> f32 {
        *self.scale.borrow()
    }

    fn appearance(&self) -> WindowAppearance {
        self.appearance.get()
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(self.display.clone()))
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.pointer_position
            .get()
            .unwrap_or_else(|| point(px(0.0), px(0.0)))
    }

    fn modifiers(&self) -> Modifiers {
        self.key_state.borrow().modifiers()
    }

    fn capslock(&self) -> Capslock {
        self.key_state.borrow().capslock()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        *self.input_handler.borrow_mut() = Some(input_handler);
        self.schedule_ime_text_update();
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.input_handler.borrow_mut().take()
    }

    fn set_text_input_configuration(&mut self, configuration: TextInputConfiguration) {
        self.configure_text_input(configuration);
    }

    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        let client = self.window_client()?;
        if answers.is_empty() || answers.len() > 128 {
            return None;
        }
        let request = openharmony_ability_plugin_window::WindowPromptRequest {
            window_id: self.window_id,
            message: msg.to_owned(),
            detail: detail.map(str::to_owned),
            level: match level {
                PromptLevel::Info => 0,
                PromptLevel::Warning => 1,
                PromptLevel::Critical => 2,
            },
            buttons: answers
                .iter()
                .map(|answer| answer.label().to_string())
                .collect(),
            cancel_index: answers
                .iter()
                .position(PromptButton::is_cancel)
                .map(|index| index as u32),
            anchor: None,
        };
        let (sender, receiver) = oneshot::channel();
        self.foreground_executor
            .spawn(async move {
                match client.show_prompt(request).await {
                    Ok(index) => {
                        let _ = sender.send(index as usize);
                    }
                    Err(error) => warn!("OHOS native prompt failed: {error}"),
                }
            })
            .detach();
        Some(receiver)
    }

    fn activate(&self) {
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.focus_window(window_id).await {
                    warn!("Failed to focus OHOS window: {error}");
                }
            })
            .detach();
    }

    fn request_attention(&self) {
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.request_user_attention(window_id).await {
                    warn!("Failed to request OHOS window attention: {error}");
                }
            })
            .detach();
    }

    fn is_active(&self) -> bool {
        self.active.get()
    }

    fn visibility(&self) -> WindowVisibility {
        self.visibility.get()
    }

    fn is_hovered(&self) -> bool {
        self.hovered.get()
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        self.background_appearance.get()
    }

    fn get_title(&self) -> String {
        self.title.borrow().clone()
    }

    fn native_window_state(&self) -> Option<Vec<u8>> {
        let mut state = if self.maximized.get() || self.fullscreen.get() {
            self.floating_state
                .get()
                .or_else(|| self.current_native_state())?
        } else {
            self.current_native_state()?
        };
        state.maximized = self.maximized.get();
        state.fullscreen = self.fullscreen.get();
        Some(state.encode())
    }

    fn restore_native_window_state(&self, state: &[u8]) {
        let Some(state) = super::window_state::NativeWindowState::decode(state) else {
            warn!("Ignoring invalid OHOS native window state");
            return;
        };
        *self.restore_state.borrow_mut() = Some(state);
        self.apply_restored_state();
    }

    fn set_title(&mut self, title: &str) {
        *self.title.borrow_mut() = title.to_owned();
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        if app.native_window_for(self.window_id).is_none() {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        let requested = title.to_owned();
        let generation = self.title_generation.clone();
        let token = generation.get().wrapping_add(1);
        generation.set(token);
        self.foreground_executor
            .spawn(async move {
                if generation.get() != token {
                    return;
                }
                if let Err(error) = client.set_window_title(window_id, requested).await {
                    warn!("Failed to set OHOS window title: {error}");
                }
            })
            .detach();
    }

    fn set_background_appearance(&self, appearance: WindowBackgroundAppearance) {
        // Preserve the system's default background until an explicit transition is requested.
        if self
            .requested_background
            .get()
            .unwrap_or(self.background_appearance.get())
            == appearance
        {
            return;
        }
        self.requested_background.set(Some(appearance));
        self.apply_requested_background();
    }

    fn minimize(&self) {
        if !self.is_minimizable {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.minimize_window(window_id).await {
                    warn!("Failed to minimize OHOS window: {error}");
                }
            })
            .detach();
    }

    fn zoom(&self) {
        if !self.is_resizable {
            return;
        }
        self.remember_floating_state();
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        let state = self.maximized.clone();
        self.foreground_executor
            .spawn(async move {
                let next = !state.get();
                let result = if next {
                    client.maximize_window(window_id).await
                } else {
                    client.restore_window(window_id).await
                };
                match result {
                    Ok(()) => state.set(next),
                    Err(error) => warn!("Failed to zoom OHOS window: {error}"),
                }
            })
            .detach();
    }

    fn toggle_fullscreen(&self) {
        self.remember_floating_state();
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        let state = self.fullscreen.clone();
        self.foreground_executor
            .spawn(async move {
                let next = !state.get();
                match client.set_fullscreen(window_id, next).await {
                    Ok(()) => state.set(next),
                    Err(error) => warn!("Failed to set OHOS fullscreen: {error}"),
                }
            })
            .detach();
    }

    fn is_fullscreen(&self) -> bool {
        self.fullscreen.get()
    }

    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        OhosWindow::frame_waker(self)
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.callbacks.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> crate::DispatchEventResult>) {
        self.callbacks.borrow_mut().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.callbacks.borrow_mut().active_status_change = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.callbacks.borrow_mut().hover_status_change = Some(callback);
    }

    fn set_back_handler(&self, callback: Box<dyn FnMut()>) {
        *self.back_handler.borrow_mut() = Some(callback);
    }

    fn set_back_enabled(&self, enabled: bool) {
        self.back_enabled.set(enabled);
    }

    fn a11y_init(&self, callbacks: A11yCallbacks) {
        self.release_accessibility();
        *self.a11y_callbacks.borrow_mut() = Some(Arc::new(Mutex::new(callbacks)));
        self.initialize_accessibility();
    }

    fn a11y_tree_update(&self, update: accesskit::TreeUpdate) {
        self.initialize_accessibility();
        if let Some(adapter) = self.a11y_adapter.borrow().as_ref()
            && let Err(error) = adapter.update_if_active(|| update)
        {
            warn!("Failed to update OHOS accessibility tree: {error}");
        }
    }

    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        self.callbacks.borrow_mut().visibility_change = Some(callback);
    }

    fn insets(&self) -> WindowInsets {
        self.current_safe_area_insets()
    }

    fn on_insets_changed(&self, callback: Box<dyn FnMut(WindowInsets)>) {
        self.callbacks.borrow_mut().insets_changed = Some(callback);
    }

    fn show_soft_keyboard(&self) {
        // This is an explicit user-gesture request. Do not suppress it based
        // on cached visibility: the system can dismiss the IME while GPUI
        // focus remains on the same input.
        self.request_keyboard();
    }

    fn hide_soft_keyboard(&self) {
        self.hide_keyboard_if_needed();
    }

    fn text_input_state_changed(&self, change: TextInputStateChange) {
        match change {
            TextInputStateChange::FocusGained => {
                self.invalidate_pending_edit();
                self.show_keyboard_if_needed();
            }
            TextInputStateChange::FocusLost => {
                self.invalidate_pending_edit();
                self.hide_keyboard_if_needed();
            }
            TextInputStateChange::SelectionChanged | TextInputStateChange::ContentChanged => {
                self.invalidate_pending_edit();
                self.schedule_ime_text_update();
            }
        }
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.callbacks.borrow_mut().resize = Some(callback);
        self.viewport.reset();
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.callbacks.borrow_mut().should_close = Some(callback);
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.callbacks.borrow_mut().hit_test_window_control = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.callbacks.borrow_mut().close = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        if self.closed.get()
            || !self.surface_available.get()
            || self.visibility.get() != WindowVisibility::Visible
        {
            return;
        }
        // Initialize renderer lazily if not already initialized
        // This ensures native_window is available (after SurfaceCreate event)
        if self.renderer.borrow().is_none()
            && let Err(e) = self.initialize_renderer()
        {
            warn!("OhosWindow: Failed to initialize renderer in draw(): {}", e);
            return;
        }

        // Use WGPU renderer to render the scene.
        if let Some(ref mut renderer) = *self.renderer.borrow_mut() {
            renderer.draw(scene);
        } else {
            warn!("OhosWindow: draw called but renderer is not available");
        }
    }

    fn schedule_frame(&self) {
        if let Some(scheduler) = &self.frame_scheduler {
            scheduler.request_frame();
        }
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        if let Some(ref renderer) = *self.renderer.borrow() {
            renderer.sprite_atlas().clone()
        } else if let Some(atlas) = self.fallback_atlas.borrow().as_ref() {
            atlas.clone()
        } else {
            if let Err(error) = self.initialize_renderer() {
                panic!("OhosWindow: renderer must be initialized before sprite_atlas: {error}");
            }
            self.renderer
                .borrow()
                .as_ref()
                .expect("renderer should be initialized after initialize_renderer")
                .sprite_atlas()
                .clone()
        }
    }

    fn request_decorations(&self, decorations: WindowDecorations) {
        if self.window_id == 0 {
            return;
        }
        self.requested_decorations.set(decorations);
        // GPUI can request decorations before asynchronous OS window creation
        // finishes. SurfaceCreate reapplies the retained request once ready.
        if !self
            .app
            .borrow()
            .as_ref()
            .is_some_and(|app| app.native_window_for(self.window_id).is_some())
        {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        let window_id = self.window_id;
        let state = self.decorations.clone();
        self.foreground_executor
            .spawn(async move {
                match client
                    .set_window_decorations(window_id, decorations == WindowDecorations::Server)
                    .await
                {
                    Ok(()) => state.set(decorations),
                    Err(error) => warn!("Failed to set OHOS window decorations: {error}"),
                }
            })
            .detach();
    }

    fn show_window_menu(&self, position: Point<Pixels>) {
        if !self.window_controls().window_menu {
            return;
        }
        let Some(client) = self.window_client() else {
            return;
        };
        let mut commands = Vec::new();
        let mut labels = Vec::new();
        if self.is_minimizable {
            commands.push(0);
            labels.push("Minimize".into());
        }
        if self.is_resizable {
            commands.push(1);
            labels.push(
                if self.maximized.get() {
                    "Restore"
                } else {
                    "Maximize"
                }
                .into(),
            );
        }
        commands.push(2);
        labels.push(
            if self.fullscreen.get() {
                "Exit fullscreen"
            } else {
                "Fullscreen"
            }
            .into(),
        );
        commands.push(3);
        labels.push("Close".into());
        let cancel = labels.len() as u32;
        labels.push("Cancel".into());
        let request = openharmony_ability_plugin_window::WindowPromptRequest {
            window_id: self.window_id,
            message: "Window".into(),
            detail: None,
            level: 0,
            buttons: labels,
            cancel_index: Some(cancel),
            anchor: Some(openharmony_ability_plugin_window::WindowPromptAnchor {
                x: f64::from(position.x.as_f32().max(0.0)),
                y: f64::from(position.y.as_f32().max(0.0)),
            }),
        };
        let window_id = self.window_id;
        let maximized = self.maximized.clone();
        let fullscreen = self.fullscreen.clone();
        let callbacks = self.callbacks.clone();
        let quit = self.quit.clone();
        self.remember_floating_state();
        self.foreground_executor
            .spawn(async move {
                let index = match client.show_prompt(request).await {
                    Ok(index) => index as usize,
                    Err(error) => {
                        warn!("OHOS window menu closed: {error}");
                        return;
                    }
                };
                let Some(command) = commands.get(index) else {
                    return;
                };
                let result = match command {
                    0 => client.minimize_window(window_id).await,
                    1 => {
                        let next = !maximized.get();
                        let result = if next {
                            client.maximize_window(window_id).await
                        } else {
                            client.recover_window(window_id).await
                        };
                        if result.is_ok() {
                            maximized.set(next);
                        }
                        result
                    }
                    2 => {
                        let next = !fullscreen.get();
                        let result = client.set_fullscreen(window_id, next).await;
                        if result.is_ok() {
                            fullscreen.set(next);
                        }
                        result
                    }
                    3 => {
                        let mut should_close = callbacks.borrow_mut().should_close.take();
                        let allowed = should_close.as_mut().is_none_or(|callback| callback());
                        callbacks.borrow_mut().should_close = should_close;
                        if !allowed {
                            return;
                        }
                        if window_id == 0 {
                            quit();
                            Ok(())
                        } else {
                            client.destroy_window(window_id).await
                        }
                    }
                    _ => return,
                };
                if let Err(error) = result {
                    warn!("OHOS window menu command failed: {error}");
                }
            })
            .detach();
    }

    fn start_window_move(&self) {
        self.begin_manipulation(None);
    }

    fn can_start_external_drag(&self) -> bool {
        !self.closed.get()
            && !self.native_drag_pending.get()
            && self.native_pointer_id.get().is_some()
            && (self.pressed_mouse_button.get().is_some()
                || !self.active_touches.borrow().is_empty())
            && self
                .native_drag
                .borrow()
                .as_ref()
                .is_none_or(|drag| drag.ended())
    }
    fn start_external_drag(&self, payload: &crate::ExternalDragPayload) -> bool {
        if !self.can_start_external_drag() {
            return false;
        }
        let Some(app) = self.app.borrow().clone() else {
            return false;
        };
        let crate::ExternalDragPayload::Files(paths) = payload;
        let files = paths
            .entries()
            .iter()
            .map(|(path, directory)| {
                let path = path
                    .to_str()
                    .ok_or_else(|| "Drag path is not UTF-8".to_owned())?;
                ohos_fileuri_binding::get_uri_from_path(path)
                    .map(|uri| (uri, *directory))
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, String>>();
        let files = match files {
            Ok(files) => files,
            Err(error) => {
                warn!("Cannot start OHOS file drag: {error}");
                return false;
            }
        };
        // GPUI promotes an active mouse drag after it leaves the viewport.
        info!(
            "OHOS native file drag input pointer {:?}",
            self.native_pointer_id.get()
        );
        let window_id = self.window_id;
        let native_drag = self.native_drag.clone();
        let pending = self.native_drag_pending.clone();
        let callbacks = self.callbacks.clone();
        let pointer = self
            .native_pointer_id
            .get()
            .filter(|id| (0..=9).contains(id))
            .unwrap_or(0);
        pending.set(true);
        app.clone().queue_after_input(move || {
            let Some(node) = app
                .with_xcomponent_for(window_id, openharmony_ability::NativeFileDrag::node_handle)
            else {
                pending.set(false);
                Self::dispatch_input_with_callbacks(
                    &callbacks,
                    PlatformInput::FileDrop(crate::FileDropEvent::Ended),
                );
                return;
            };
            // ArkUI's mouse ID is outside the drag-action range. The SDK
            // documents 0 as the default ID for a single drag.
            match openharmony_ability::NativeFileDrag::start(
                node,
                pointer,
                &files,
                app.create_waker(),
            ) {
                Ok(session) => {
                    *native_drag.borrow_mut() = Some(session);
                    info!("OHOS native file drag started for window {window_id}");
                }
                Err(error) => {
                    warn!("Failed to start OHOS native file drag: {error}");
                    Self::dispatch_input_with_callbacks(
                        &callbacks,
                        PlatformInput::FileDrop(crate::FileDropEvent::Ended),
                    );
                }
            }
            pending.set(false);
        });
        true
    }

    fn start_window_resize(&self, edge: ResizeEdge) {
        self.begin_manipulation(Some(edge));
    }

    fn window_decorations(&self) -> crate::Decorations {
        if self.decorations.get() == WindowDecorations::Client {
            crate::Decorations::Client {
                tiling: Default::default(),
            }
        } else {
            crate::Decorations::Server
        }
    }

    fn set_app_id(&mut self, _app_id: &str) {
        // Not supported on OHOS
    }

    fn map_window(&mut self) -> Result<()> {
        anyhow::ensure!(!self.closed.get(), "Cannot map a closed OHOS window");
        let client = self
            .window_client()
            .ok_or_else(|| anyhow::anyhow!("OHOS window bridge is unavailable"))?;
        let window_id = self.window_id;
        self.foreground_executor
            .spawn(async move {
                if let Err(error) = client.show_window(window_id).await {
                    warn!("Failed to map OHOS window: {error}");
                }
            })
            .detach();
        Ok(())
    }

    fn window_controls(&self) -> WindowControls {
        let available = self.window_id != 0 || openharmony_ability::is_desktop_device();
        WindowControls {
            fullscreen: available,
            maximize: available && self.is_resizable,
            minimize: available && self.is_minimizable,
            window_menu: available,
        }
    }

    fn set_client_inset(&self, _inset: Pixels) {
        // Keyboard avoidance is driven by content_size updates from avoid-area overlap.
        // client_inset is intentionally ignored on OHOS.
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        // Return GPU specs from the WGPU renderer.
        self.renderer
            .borrow()
            .as_ref()
            .map(|renderer| renderer.gpu_specs())
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        false
    }

    fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        let scale = self.scale_factor() as f64;
        let window = app.window_rect_for(self.window_id);
        let content = app.content_rect_for(self.window_id);
        // GPUI supplies content-local logical pixels; the NDK requires absolute
        // physical screen coordinates, including window and XComponent offsets.
        let rect = ImeCursorRect {
            left: window.left as f64
                + content.left as f64
                + bounds.origin.x.as_f32() as f64 * scale,
            top: window.top as f64 + content.top as f64 + bounds.origin.y.as_f32() as f64 * scale,
            width: (bounds.size.width.as_f32() as f64 * scale).max(1.0),
            height: (bounds.size.height.as_f32() as f64 * scale).max(1.0),
        };
        if let Err(error) = app.update_ime_cursor_for(self.window_id, rect) {
            warn!(
                "Failed to update OHOS IME cursor for window {}: {error}",
                self.window_id
            );
        }
    }
}
