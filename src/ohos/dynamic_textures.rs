use std::sync::{
    Arc, OnceLock, Weak,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

use anyhow::{Context as _, Result};
use gpui::{Bounds, DevicePixels, DynamicTextureId, DynamicTexturePlatform, Size, point};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

/// CPU backing shared by the process-wide GPUI upload interface. Each device
/// keeps its own GPU copies, so replacing a surface or context preserves data.
pub(crate) struct DynamicTextureUploads {
    limit: AtomicU32,
    textures: Mutex<FxHashMap<DynamicTextureId, UploadTexture>>,
    devices: Mutex<Vec<Weak<DeviceTextures>>>,
}

struct UploadTexture {
    size: Size<DevicePixels>,
    rgba: Vec<u8>,
    revision: u64,
    dirty_since: u64,
    dirty: Option<Bounds<DevicePixels>>,
}

impl DynamicTextureUploads {
    pub(crate) fn new(limit: u32) -> Self {
        Self {
            limit: AtomicU32::new(limit),
            textures: Mutex::new(FxHashMap::default()),
            devices: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn install(limit: u32) -> Arc<Self> {
        static UPLOADS: OnceLock<Arc<DynamicTextureUploads>> = OnceLock::new();
        let uploads = UPLOADS.get_or_init(|| Arc::new(Self::new(limit))).clone();
        uploads.limit.fetch_min(limit, Ordering::Relaxed);
        gpui::set_dynamic_texture_platform(uploads.clone());
        uploads
    }
}

impl DynamicTexturePlatform for DynamicTextureUploads {
    fn create_texture(
        &self,
        id: DynamicTextureId,
        size: Size<DevicePixels>,
        bytes: &[u8],
    ) -> Result<()> {
        let limit = self.limit.load(Ordering::Relaxed);
        anyhow::ensure!(
            size.width.0 > 0
                && size.height.0 > 0
                && size.width.0 as u32 <= limit
                && size.height.0 as u32 <= limit,
            "dynamic texture size exceeds the device limit of {limit} pixels"
        );
        let bounds = Bounds::new(point(DevicePixels(0), DevicePixels(0)), size);
        let mut textures = self.textures.lock();
        anyhow::ensure!(!textures.contains_key(&id), "duplicate dynamic texture");
        UploadTexture::validate_region(size, bounds, bytes)?;
        let mut texture = UploadTexture {
            size,
            rgba: vec![0; bytes.len()],
            revision: 1,
            dirty_since: 0,
            dirty: Some(bounds),
        };
        texture.write_bgra(bounds, bytes);
        textures.insert(id, texture);
        Ok(())
    }

    fn update_texture(
        &self,
        id: DynamicTextureId,
        bounds: Bounds<DevicePixels>,
        bytes: &[u8],
    ) -> Result<()> {
        let mut textures = self.textures.lock();
        let texture = textures.get_mut(&id).context("unknown dynamic texture")?;
        UploadTexture::validate_region(texture.size, bounds, bytes)?;
        texture.write_bgra(bounds, bytes);
        texture.dirty = Some(texture.dirty.map_or(bounds, |dirty| dirty.union(&bounds)));
        texture.revision = texture.revision.wrapping_add(1).max(1);
        Ok(())
    }

    fn remove_texture(&self, id: DynamicTextureId) {
        self.textures.lock().remove(&id);
        self.devices.lock().retain(|device| {
            let Some(device) = device.upgrade() else {
                return false;
            };
            device.0.lock().remove(&id);
            true
        });
    }
}

impl UploadTexture {
    fn validate_region(
        size: Size<DevicePixels>,
        bounds: Bounds<DevicePixels>,
        bytes: &[u8],
    ) -> Result<()> {
        let (x, y) = (i64::from(bounds.origin.x.0), i64::from(bounds.origin.y.0));
        let (width, height) = (
            i64::from(bounds.size.width.0),
            i64::from(bounds.size.height.0),
        );
        anyhow::ensure!(
            x >= 0
                && y >= 0
                && width > 0
                && height > 0
                && x + width <= i64::from(size.width.0)
                && y + height <= i64::from(size.height.0),
            "dynamic texture update is outside the texture"
        );
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|len| len.checked_mul(4))
            .context("dynamic texture payload size overflow")?;
        anyhow::ensure!(bytes.len() == len, "invalid packed BGRA texture payload");
        Ok(())
    }

    fn write_bgra(&mut self, bounds: Bounds<DevicePixels>, bytes: &[u8]) {
        let stride = self.size.width.0 as usize * 4;
        let row_bytes = bounds.size.width.0 as usize * 4;
        for (row, source) in bytes.chunks_exact(row_bytes).enumerate() {
            let start =
                (bounds.origin.y.0 as usize + row) * stride + bounds.origin.x.0 as usize * 4;
            for (destination, bgra) in self.rgba[start..start + row_bytes]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(source.as_chunks::<4>().0)
            {
                destination.copy_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
            }
        }
    }
}

/// Device-local textures shared by every window using the same GPU context.
pub(crate) struct DynamicTextures {
    uploads: Arc<DynamicTextureUploads>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    textures: Arc<DeviceTextures>,
    gpu_textures: Mutex<FxHashMap<DynamicTextureId, Weak<wgpu::TextureView>>>,
    lost: Arc<AtomicBool>,
}

#[derive(Default)]
struct DeviceTextures(Mutex<FxHashMap<DynamicTextureId, UploadedTexture>>);

struct UploadedTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    revision: u64,
}

impl DynamicTextures {
    pub(crate) fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        uploads: Arc<DynamicTextureUploads>,
    ) -> Self {
        let textures = Arc::new(DeviceTextures::default());
        let lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let lost = lost.clone();
            move |_, _| lost.store(true, Ordering::Release)
        });
        {
            let mut devices = uploads.devices.lock();
            devices.retain(|device| device.strong_count() > 0);
            devices.push(Arc::downgrade(&textures));
        }
        Self {
            uploads,
            device,
            queue,
            textures,
            gpu_textures: Mutex::new(FxHashMap::default()),
            lost,
        }
    }

    pub(crate) fn view(&self, id: DynamicTextureId) -> Option<wgpu::TextureView> {
        if self.lost.load(Ordering::Acquire) {
            return None;
        }
        if let Some(view) = self.gpu_textures.lock().get(&id).and_then(Weak::upgrade) {
            return Some((*view).clone());
        }
        let mut uploads = self.uploads.textures.lock();
        let mut textures = self.textures.0.lock();
        let Some(source) = uploads.get_mut(&id) else {
            textures.remove(&id);
            return None;
        };
        let limit = self.device.limits().max_texture_dimension_2d;
        if source.size.width.0 as u32 > limit || source.size.height.0 as u32 > limit {
            return None;
        }
        let gpu = textures.entry(id).or_insert_with(|| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ohos_dynamic_texture"),
                size: wgpu::Extent3d {
                    width: source.size.width.0 as u32,
                    height: source.size.height.0 as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // RGBA8 is available on GLES as well as Vulkan and Metal.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            UploadedTexture {
                texture,
                view,
                revision: 0,
            }
        });
        if gpu.revision != source.revision {
            let bounds = if gpu.revision == source.dirty_since {
                source.dirty
            } else {
                None
            }
            .unwrap_or_else(|| Bounds::new(point(DevicePixels(0), DevicePixels(0)), source.size));
            let stride = source.size.width.0 as usize * 4;
            let start = bounds.origin.y.0 as usize * stride + bounds.origin.x.0 as usize * 4;
            let len =
                (bounds.size.height.0 as usize - 1) * stride + bounds.size.width.0 as usize * 4;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: bounds.origin.x.0 as u32,
                        y: bounds.origin.y.0 as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &source.rgba[start..start + len],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: bounds.size.width.0 as u32,
                    height: bounds.size.height.0 as u32,
                    depth_or_array_layers: 1,
                },
            );
            gpu.revision = source.revision;
            source.dirty_since = source.revision;
            source.dirty = None;
        }
        Some(gpu.view.clone())
    }
}

/// Failure to create, write or paint an OHOS GPU render target.
#[derive(Debug)]
#[non_exhaustive]
pub enum GpuTextureError {
    NotReady,
    InitializationFailed(String),
    DeviceLost,
    InvalidSize { limit: u32 },
    Operation(anyhow::Error),
}

impl std::fmt::Display for GpuTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady => f.write_str("OHOS GPU initialization is not complete"),
            Self::InitializationFailed(error) => {
                write!(f, "OHOS GPU initialization failed: {error}")
            }
            Self::DeviceLost => f.write_str("dynamic texture device is lost"),
            Self::InvalidSize { limit } => write!(
                f,
                "texture dimensions must be positive and at most {limit} pixels"
            ),
            Self::Operation(error) => std::fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for GpuTextureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

type GpuInitialization = OnceLock<Result<Arc<super::wgpu_context::WgpuContext>, String>>;

/// Creates render targets on the same device as the platform's windows.
#[derive(Clone)]
pub struct GpuTextureService {
    context: Arc<GpuInitialization>,
}

impl GpuTextureService {
    pub(crate) fn new(context: Arc<GpuInitialization>) -> Self {
        Self { context }
    }

    /// Available after GPU initialization, including from a window's creation callback.
    pub fn create(&self, size: Size<DevicePixels>) -> Result<GpuTexture, GpuTextureError> {
        let context = self
            .context
            .get()
            .ok_or(GpuTextureError::NotReady)?
            .as_ref()
            .map_err(|error| GpuTextureError::InitializationFailed(error.clone()))?;
        context.dynamic_textures.create_gpu(size)
    }
}

/// An RGBA8Unorm render target with straight alpha, sampled directly by GPUI.
/// Dropping it releases its GPU resource and retires its GPUI handle.
pub struct GpuTexture {
    context: Arc<DynamicTextures>,
    handle: gpui::DynamicTexture,
    view: Arc<wgpu::TextureView>,
}

impl DynamicTextures {
    pub(crate) fn create_gpu(
        self: &Arc<Self>,
        size: Size<DevicePixels>,
    ) -> Result<GpuTexture, GpuTextureError> {
        if self.lost.load(Ordering::Acquire) {
            return Err(GpuTextureError::DeviceLost);
        }
        let limit = self.device.limits().max_texture_dimension_2d;
        if size.width.0 <= 0
            || size.height.0 <= 0
            || size.width.0 as u32 > limit
            || size.height.0 as u32 > limit
        {
            return Err(GpuTextureError::InvalidSize { limit });
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ohos_gpu_dynamic_texture"),
            size: wgpu::Extent3d {
                width: size.width.0 as u32,
                height: size.height.0 as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let handle = gpui::DynamicTexture::new_gpu(size).map_err(GpuTextureError::Operation)?;
        let view = Arc::new(texture.create_view(&Default::default()));
        self.gpu_textures
            .lock()
            .insert(handle.id(), Arc::downgrade(&view));
        Ok(GpuTexture {
            context: self.clone(),
            handle,
            view,
        })
    }
}

impl GpuTexture {
    /// Submit producer commands on the supplied queue before returning success.
    /// Success refreshes this window and invalidates cached tiles; notify any
    /// other windows displaying the texture separately. Do not submit partial
    /// writes and then return an error. The callback does not hold registry locks.
    pub fn render<T>(
        &self,
        window: &mut gpui::Window,
        render: impl FnOnce(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView) -> Result<T>,
    ) -> Result<T, GpuTextureError> {
        if self.context.lost.load(Ordering::Acquire) {
            return Err(GpuTextureError::DeviceLost);
        }
        let result = render(&self.context.device, &self.context.queue, &self.view)
            .map_err(GpuTextureError::Operation)?;
        window
            .refresh_dynamic_texture(self.handle)
            .map_err(GpuTextureError::Operation)?;
        Ok(result)
    }

    /// Paint during the paint phase, in a window created by this service's platform.
    pub fn paint(
        &self,
        window: &mut gpui::Window,
        bounds: Bounds<gpui::Pixels>,
    ) -> Result<(), GpuTextureError> {
        if self.context.lost.load(Ordering::Acquire) {
            return Err(GpuTextureError::DeviceLost);
        }
        window
            .paint_dynamic_texture(bounds, self.handle)
            .map_err(GpuTextureError::Operation)
    }
}

impl Drop for GpuTexture {
    fn drop(&mut self) {
        self.context.gpu_textures.lock().remove(&self.handle.id());
        self.handle.remove();
    }
}
