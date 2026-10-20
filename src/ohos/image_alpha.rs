//! Explicit alpha encoding for externally rasterized images. The image and its
//! registration share a lifetime; ordinary GPUI images retain straight alpha.
use gpui::{ImageId, RenderImage};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::{Arc, OnceLock, Weak};

type Registrations = FxHashMap<ImageId, Weak<Registration>>;

fn registrations() -> &'static Mutex<Registrations> {
    static IMAGES: OnceLock<Mutex<Registrations>> = OnceLock::new();
    IMAGES.get_or_init(Default::default)
}

/// Keeps an image marked as premultiplied while it is cached or being painted.
/// Register before the first paint uploads the image into an atlas.
#[derive(Clone)]
pub struct PremultipliedImage(Arc<Registration>);

struct Registration {
    image: Arc<RenderImage>,
    flags: u32,
}

/// Native font rasters use the original terminal's nearest atlas sampling.
/// Register before the first upload; ordinary UI images keep linear filtering.
#[derive(Clone)]
pub struct RasterImage(Arc<Registration>);

impl Registration {
    fn new(image: Arc<RenderImage>, flags: u32) -> Arc<Self> {
        let mut images = registrations().lock();
        if let Some(existing) = images.get(&image.id).and_then(Weak::upgrade) {
            drop(images);
            assert_eq!(
                existing.flags, flags,
                "image encoding and sampling are immutable"
            );
            return existing;
        }
        let owner = Arc::new(Registration { image, flags });
        images.insert(owner.image.id, Arc::downgrade(&owner));
        owner
    }
}

impl RasterImage {
    pub fn straight(image: Arc<RenderImage>) -> Self {
        Self(Registration::new(image, 2))
    }

    pub fn premultiplied(image: Arc<RenderImage>) -> Self {
        Self(Registration::new(image, 3))
    }

    pub fn image(&self) -> &Arc<RenderImage> {
        &self.0.image
    }
}

impl PremultipliedImage {
    pub fn new(image: Arc<RenderImage>) -> Self {
        Self(Registration::new(image, 1))
    }

    pub fn image(&self) -> &Arc<RenderImage> {
        &self.0.image
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut images = registrations().lock();
        // A concurrent constructor may have replaced an expired weak entry.
        if images
            .get(&self.image.id)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
        {
            images.remove(&self.image.id);
        }
    }
}

pub(super) fn flags(id: ImageId) -> u32 {
    // Dropping the last upgraded owner calls Registration::drop, which locks
    // this registry too. Release the guard before inspecting/releasing it.
    let owner = {
        let images = registrations().lock();
        images.get(&id).and_then(Weak::upgrade)
    };
    owner.map_or(0, |owner| owner.flags)
}
