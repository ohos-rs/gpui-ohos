use std::{
    collections::HashMap,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Result, bail, ensure};
use futures::channel::oneshot;
use gpui::{
    DevicePixels, ForegroundExecutor, ScreenCaptureFrame, ScreenCaptureSource, ScreenCaptureStream,
    SourceMetadata, size,
};
use image::RgbaImage;
use log::{error, info, warn};

static RECEIVED_VIDEO_BUFFERS: AtomicUsize = AtomicUsize::new(0);
static CAPTURE_CALLBACKS: OnceLock<Mutex<HashMap<usize, Arc<CaptureCallbacks>>>> = OnceLock::new();

fn capture_callbacks() -> &'static Mutex<HashMap<usize, Arc<CaptureCallbacks>>> {
    CAPTURE_CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

const RGBA_8888: i32 = 12;
const RGBX_8888: i32 = 11;
const BGRA_8888: i32 = 20;
const BGRX_8888: i32 = 19;

#[repr(C)]
struct NativeScreenCapture {
    _private: [u8; 0],
}

#[repr(C)]
struct NativeBuffer {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Default)]
struct NativeBufferConfig {
    width: i32,
    height: i32,
    format: i32,
    usage: i32,
    stride: i32,
}

#[repr(C)]
struct AudioCaptureInfo {
    sample_rate: i32,
    channels: i32,
    source: i32,
}

#[repr(C)]
struct AudioEncInfo {
    bitrate: i32,
    format: i32,
}

#[repr(C)]
struct AudioInfo {
    microphone: AudioCaptureInfo,
    internal: AudioCaptureInfo,
    encoder: AudioEncInfo,
}

#[repr(C)]
struct VideoCaptureInfo {
    display_id: u64,
    mission_ids: *mut i32,
    mission_ids_len: i32,
    width: i32,
    height: i32,
    source: i32,
}

#[repr(C)]
struct VideoEncInfo {
    format: i32,
    bitrate: i32,
    frame_rate: i32,
}

#[repr(C)]
struct VideoInfo {
    capture: VideoCaptureInfo,
    encoder: VideoEncInfo,
}

#[repr(C)]
struct RecorderInfo {
    url: *mut i8,
    url_len: u32,
    format: i32,
}

#[repr(C)]
struct ScreenCaptureConfig {
    capture_mode: i32,
    data_type: i32,
    audio: AudioInfo,
    video: VideoInfo,
    recorder: RecorderInfo,
}

#[repr(C)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[repr(C)]
struct LegacyCaptureCallbacks {
    on_error: Option<unsafe extern "C" fn(*mut NativeScreenCapture, i32)>,
    on_audio: Option<unsafe extern "C" fn(*mut NativeScreenCapture, bool, i32)>,
    on_video: Option<unsafe extern "C" fn(*mut NativeScreenCapture, bool)>,
}

#[link(name = "native_avscreen_capture")]
unsafe extern "C" {
    fn OH_AVScreenCapture_Create() -> *mut NativeScreenCapture;
    fn OH_AVScreenCapture_Init(
        capture: *mut NativeScreenCapture,
        config: ScreenCaptureConfig,
    ) -> i32;
    fn OH_AVScreenCapture_StartScreenCapture(capture: *mut NativeScreenCapture) -> i32;
    fn OH_AVScreenCapture_StopScreenCapture(capture: *mut NativeScreenCapture) -> i32;
    fn OH_AVScreenCapture_Release(capture: *mut NativeScreenCapture) -> i32;
    fn OH_AVScreenCapture_AcquireVideoBuffer(
        capture: *mut NativeScreenCapture,
        fence: *mut i32,
        timestamp: *mut i64,
        region: *mut Rect,
    ) -> *mut NativeBuffer;
    fn OH_AVScreenCapture_ReleaseVideoBuffer(capture: *mut NativeScreenCapture) -> i32;
    fn OH_AVScreenCapture_SetCallback(
        capture: *mut NativeScreenCapture,
        callbacks: LegacyCaptureCallbacks,
    ) -> i32;
    fn OH_AVScreenCapture_SetStateCallback(
        capture: *mut NativeScreenCapture,
        callback: unsafe extern "C" fn(*mut NativeScreenCapture, i32, *mut c_void),
        user_data: *mut c_void,
    ) -> i32;
    fn OH_AVScreenCapture_SetErrorCallback(
        capture: *mut NativeScreenCapture,
        callback: unsafe extern "C" fn(*mut NativeScreenCapture, i32, *mut c_void),
        user_data: *mut c_void,
    ) -> i32;
}

#[link(name = "native_buffer")]
unsafe extern "C" {
    fn OH_NativeBuffer_GetConfig(buffer: *mut NativeBuffer, config: *mut NativeBufferConfig);
    fn OH_NativeBuffer_Map(buffer: *mut NativeBuffer, address: *mut *mut c_void) -> i32;
    fn OH_NativeBuffer_Unmap(buffer: *mut NativeBuffer) -> i32;
}

struct CaptureCallbacks {
    frame: Mutex<Box<dyn Fn(ScreenCaptureFrame) + Send>>,
}

struct OhosScreenCaptureSource {
    width: i32,
    height: i32,
}

impl OhosScreenCaptureSource {
    fn metadata(&self) -> SourceMetadata {
        SourceMetadata {
            id: 0,
            label: Some("OpenHarmony display".into()),
            is_main: Some(true),
            resolution: size(DevicePixels(self.width), DevicePixels(self.height)),
        }
    }
}

impl ScreenCaptureSource for OhosScreenCaptureSource {
    fn metadata(&self) -> Result<SourceMetadata> {
        Ok(self.metadata())
    }

    fn stream(
        &self,
        _foreground_executor: &ForegroundExecutor,
        frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    ) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>> {
        let (sender, receiver) = oneshot::channel();
        let stream = OhosScreenCaptureStream::start(self.metadata(), frame_callback)
            .map(|stream| Box::new(stream) as Box<dyn ScreenCaptureStream>);
        if sender.send(stream).is_err() {
            warn!("OHOS screen capture receiver was dropped before the stream started");
        }
        receiver
    }
}

pub(super) fn sources(
    width: i32,
    height: i32,
) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
    let (sender, receiver) = oneshot::channel();
    let result = if width > 0 && height > 0 {
        Ok(vec![
            Rc::new(OhosScreenCaptureSource { width, height }) as Rc<dyn ScreenCaptureSource>
        ])
    } else {
        Err(anyhow::anyhow!("OHOS display size is unavailable"))
    };
    if sender.send(result).is_err() {
        warn!("OHOS screen capture source receiver was dropped");
    }
    receiver
}

struct OhosScreenCaptureStream {
    capture: *mut NativeScreenCapture,
    callbacks: Arc<CaptureCallbacks>,
    metadata: SourceMetadata,
    started: bool,
}

impl OhosScreenCaptureStream {
    fn start(
        metadata: SourceMetadata,
        frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    ) -> Result<Self> {
        let capture = unsafe { OH_AVScreenCapture_Create() };
        ensure!(!capture.is_null(), "OHOS AVScreenCapture is unavailable");
        let callbacks = Arc::new(CaptureCallbacks {
            frame: Mutex::new(frame_callback),
        });
        let mut stream = Self {
            capture,
            callbacks,
            metadata,
            started: false,
        };
        let resolution = stream.metadata.resolution;
        let config = ScreenCaptureConfig {
            capture_mode: 0, // OH_CAPTURE_HOME_SCREEN
            data_type: 0,    // OH_ORIGINAL_STREAM
            audio: AudioInfo {
                microphone: AudioCaptureInfo {
                    sample_rate: 0,
                    channels: 0,
                    source: -1,
                },
                internal: AudioCaptureInfo {
                    sample_rate: 0,
                    channels: 0,
                    source: -1,
                },
                encoder: AudioEncInfo {
                    bitrate: 0,
                    format: 0,
                },
            },
            video: VideoInfo {
                capture: VideoCaptureInfo {
                    display_id: 0,
                    mission_ids: ptr::null_mut(),
                    mission_ids_len: 0,
                    width: resolution.width.0,
                    height: resolution.height.0,
                    source: 2, // OH_VIDEO_SOURCE_SURFACE_RGBA
                },
                encoder: VideoEncInfo {
                    format: 0,
                    bitrate: 0,
                    frame_rate: 0,
                },
            },
            recorder: RecorderInfo {
                url: ptr::null_mut(),
                url_len: 0,
                format: 0,
            },
        };
        check(
            unsafe { OH_AVScreenCapture_Init(capture, config) },
            "initialize capture",
        )?;
        let user_data = Arc::as_ptr(&stream.callbacks) as *mut c_void;
        check(
            unsafe {
                OH_AVScreenCapture_SetCallback(
                    capture,
                    LegacyCaptureCallbacks {
                        on_error: None,
                        on_audio: None,
                        on_video: Some(on_video_ready),
                    },
                )
            },
            "register video callback",
        )?;
        check(
            unsafe { OH_AVScreenCapture_SetStateCallback(capture, on_state, user_data) },
            "register state callback",
        )?;
        check(
            unsafe { OH_AVScreenCapture_SetErrorCallback(capture, on_error, user_data) },
            "register error callback",
        )?;
        capture_callbacks()
            .lock()
            .map_err(|_| anyhow::anyhow!("OHOS screen capture registry is poisoned"))?
            .insert(capture as usize, stream.callbacks.clone());
        check(
            unsafe { OH_AVScreenCapture_StartScreenCapture(capture) },
            "start capture",
        )?;
        stream.started = true;
        Ok(stream)
    }
}

impl ScreenCaptureStream for OhosScreenCaptureStream {
    fn metadata(&self) -> Result<SourceMetadata> {
        Ok(self.metadata.clone())
    }
}

impl Drop for OhosScreenCaptureStream {
    fn drop(&mut self) {
        if self.started {
            let result = unsafe { OH_AVScreenCapture_StopScreenCapture(self.capture) };
            if result != 0 {
                warn!("Failed to stop OHOS screen capture: {result}");
            }
        }
        let result = unsafe { OH_AVScreenCapture_Release(self.capture) };
        if result != 0 {
            warn!("Failed to release OHOS screen capture: {result}");
        }
        if let Ok(mut callbacks) = capture_callbacks().lock() {
            callbacks.remove(&(self.capture as usize));
        }
    }
}

fn check(code: i32, operation: &str) -> Result<()> {
    if code != 0 {
        bail!("OHOS AVScreenCapture failed to {operation}: {code}");
    }
    Ok(())
}

unsafe extern "C" fn on_state(
    _capture: *mut NativeScreenCapture,
    state: i32,
    _user_data: *mut c_void,
) {
    info!("OHOS screen capture changed state: {state}");
}

unsafe extern "C" fn on_error(
    _capture: *mut NativeScreenCapture,
    code: i32,
    _user_data: *mut c_void,
) {
    error!("OHOS screen capture error: {code}");
}

unsafe extern "C" fn on_video_ready(capture: *mut NativeScreenCapture, ready: bool) {
    if !ready || capture.is_null() {
        return;
    }
    let callbacks = capture_callbacks()
        .lock()
        .ok()
        .and_then(|callbacks| callbacks.get(&(capture as usize)).cloned());
    let Some(callbacks) = callbacks else { return };
    let count = RECEIVED_VIDEO_BUFFERS.fetch_add(1, Ordering::Relaxed) + 1;
    if count <= 3 {
        info!("OHOS screen capture video buffer ready {count}");
    }
    let mut fence = -1;
    let mut timestamp = 0;
    let mut region = Rect {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };
    let buffer = unsafe {
        OH_AVScreenCapture_AcquireVideoBuffer(capture, &mut fence, &mut timestamp, &mut region)
    };
    if buffer.is_null() {
        warn!("OHOS screen capture reported a frame but acquire returned null");
        return;
    }
    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        copy_frame(buffer, &callbacks)
    }));
    let release_result = unsafe { OH_AVScreenCapture_ReleaseVideoBuffer(capture) };
    if release_result != 0 {
        warn!("Failed to release OHOS capture frame: {release_result}");
    }
    if result.is_err() {
        error!("OHOS screen capture frame callback panicked");
    }
}

unsafe fn copy_frame(native: *mut NativeBuffer, callbacks: &CaptureCallbacks) {
    let mut config = NativeBufferConfig::default();
    unsafe { OH_NativeBuffer_GetConfig(native, &mut config) };
    if RECEIVED_VIDEO_BUFFERS.load(Ordering::Relaxed) <= 3 {
        info!(
            "OHOS capture buffer: {}x{}, stride {}, format {}",
            config.width, config.height, config.stride, config.format
        );
    }
    let Ok(width) = usize::try_from(config.width) else {
        return;
    };
    let Ok(height) = usize::try_from(config.height) else {
        return;
    };
    let Ok(stride) = usize::try_from(config.stride) else {
        return;
    };
    let Some(row_bytes) = width.checked_mul(4) else {
        return;
    };
    let Some(frame_bytes) = row_bytes.checked_mul(height) else {
        return;
    };
    if width == 0 || height == 0 || stride < row_bytes || frame_bytes > 256 * 1024 * 1024 {
        return;
    }
    if !matches!(config.format, RGBA_8888 | RGBX_8888 | BGRA_8888 | BGRX_8888) {
        warn!("Unsupported OHOS capture pixel format: {}", config.format);
        return;
    }
    let mut address: *mut c_void = ptr::null_mut();
    if unsafe { OH_NativeBuffer_Map(native, &mut address) } != 0 || address.is_null() {
        warn!("Failed to map OHOS screen capture frame");
        return;
    }
    let mut pixels = vec![0u8; frame_bytes];
    for row in 0..height {
        let Some(source_offset) = row.checked_mul(stride) else {
            break;
        };
        let target_offset = row * row_bytes;
        let source = unsafe {
            std::slice::from_raw_parts((address as *const u8).add(source_offset), row_bytes)
        };
        pixels[target_offset..target_offset + row_bytes].copy_from_slice(source);
    }
    let result = unsafe { OH_NativeBuffer_Unmap(native) };
    if result != 0 {
        warn!("Failed to unmap OHOS screen capture frame: {result}");
        return;
    }
    if matches!(config.format, BGRA_8888 | BGRX_8888) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    if matches!(config.format, RGBX_8888 | BGRX_8888) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    if let Some(frame) = RgbaImage::from_raw(config.width as u32, config.height as u32, pixels) {
        match callbacks.frame.lock() {
            Ok(callback) => callback(ScreenCaptureFrame(frame)),
            Err(_) => error!("OHOS screen capture callback lock is poisoned"),
        }
    }
}
