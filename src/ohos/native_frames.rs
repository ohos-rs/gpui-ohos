use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::atomic::{AtomicUsize, Ordering},
};

use ohos_vsync_binding::{OH_NativeVSync_ExpectedRateRange as ExpectedRateRange, Vsync};
use openharmony_ability::MainThreadScheduler;

use super::frame_delivery::WindowFrames;

thread_local! {
    #[allow(clippy::missing_const_for_thread_local, reason = "The initializer is const; OHOS std TLS expansion triggers this lint.")]
    static WINDOWS: RefCell<Vec<Weak<WindowFrames>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeRefresh {
    pub(crate) window_id: u64,
    pub(crate) hz: u32,
}

pub(crate) fn register(frames: &Rc<WindowFrames>) {
    WINDOWS.with_borrow_mut(|windows| {
        windows.retain(|window| window.strong_count() > 0);
        windows.push(Rc::downgrade(frames));
    });
}

/// Called on the embedding UI thread, independently for each GPUI window.
pub fn configure_native_refresh(
    window_id: i64,
    associated_window: u64,
    hz: Option<u32>,
) -> Result<(), String> {
    let frames = WINDOWS.with_borrow(|windows| {
        windows
            .iter()
            .filter_map(Weak::upgrade)
            .find(|frames| frames.window_id() == window_id)
    });
    match frames {
        Some(frames) => {
            frames.set_native_refresh(hz.map(|hz| NativeRefresh {
                window_id: associated_window,
                hz,
            }));
            Ok(())
        }
        None if hz.is_none() => Ok(()),
        None => Err(format!("No live GPUI frame owner for window {window_id}")),
    }
}

static NEXT_TOKEN: AtomicUsize = AtomicUsize::new(1);

/// The binding owns the receiver and its callback liveness gate. A generation
/// token also rejects UI-thread callbacks queued before this owner was dropped.
pub(crate) struct NativeFrames {
    vsync: Vsync,
    token: usize,
    main_thread: MainThreadScheduler,
    window_id: i64,
}

impl NativeFrames {
    pub(crate) fn new(
        target: NativeRefresh,
        window_id: i64,
        main_thread: MainThreadScheduler,
    ) -> Result<Self, String> {
        // Probe optional rate support before calling the binding on older runtimes.
        let setter = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"OH_NativeVSync_SetExpectedFrameRateRange".as_ptr(),
            )
        };
        if setter.is_null() {
            return Err("NativeVSync rate requests require API 20".into());
        }
        let associated = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                c"OH_NativeVSync_Create_ForAssociatedWindow".as_ptr(),
            )
        };
        let vsync = if target.window_id != 0 && !associated.is_null() {
            Vsync::try_new_for_associated_window(target.window_id, "gpui-native-render")
        } else {
            Vsync::try_new("gpui-native-render")
        }
        .ok_or("NativeVSync creation failed")?;
        let hz = i32::try_from(target.hz)
            .ok()
            .filter(|hz| *hz > 0)
            .ok_or("NativeVSync requires a positive supported refresh rate")?;
        let result = vsync.set_expected_frame_rate_range(ExpectedRateRange {
            min: hz,
            max: hz,
            expected: hz,
        });
        if result != 0 {
            return Err(format!("NativeVSync frame-rate request failed: {result}"));
        }
        let mut connection = Self {
            vsync,
            token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
            main_thread,
            window_id,
        };
        connection.request()?;
        Ok(connection)
    }

    pub(crate) fn token(&self) -> usize {
        self.token
    }

    pub(crate) fn request(&mut self) -> Result<(), String> {
        let main_thread = self.main_thread.clone();
        let window_id = self.window_id;
        let token = self.token;
        let result = self.vsync.request_frame_once(move |_| {
            // Binding callbacks may run on the VSync thread; GPUI stays on its UI thread.
            let _ = main_thread.dispatch(move || {
                let frames = WINDOWS.with_borrow(|windows| {
                    windows
                        .iter()
                        .filter_map(Weak::upgrade)
                        .find(|frames| frames.window_id() == window_id)
                });
                if let Some(frames) = frames {
                    frames.on_native_callback(token);
                }
            });
        });
        if result == 0 {
            Ok(())
        } else {
            Err(format!("NativeVSync frame request failed: {result}"))
        }
    }
}

impl Drop for NativeFrames {
    fn drop(&mut self) {
        // Withdraw the vote before the binding destroys the receiver. Its
        // liveness gate suppresses late native callbacks without raw userdata.
        self.vsync.set_expected_frame_rate_range(ExpectedRateRange {
            min: 0,
            max: 0,
            expected: 0,
        });
    }
}
