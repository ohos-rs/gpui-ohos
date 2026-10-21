//! Explicit alpha encoding for externally rasterized images. The image and its
//! registration share a lifetime; ordinary GPUI images retain straight alpha.
use gpui::{AtlasTextureId, ImageId, PolychromeSprite, RenderImage};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

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
    policy: ImagePolicy,
}

/// The explicit alpha encoding and sampling policy of a registered image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImagePolicy {
    /// Premultiplied pixels with ordinary linear UI sampling.
    PremultipliedLinear,
    /// Straight-alpha pixels with nearest-neighbor raster sampling.
    StraightNearest,
    /// Premultiplied pixels with nearest-neighbor raster sampling.
    PremultipliedNearest,
}

impl ImagePolicy {
    fn flags(self) -> u32 {
        match self {
            Self::PremultipliedLinear => 1,
            Self::StraightNearest => 2,
            Self::PremultipliedNearest => 3,
        }
    }
}

/// An image already has a live owner with a different alpha or sampling policy.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageRegistrationError {
    /// The image whose existing registration was preserved.
    pub image_id: ImageId,
    /// The policy retained by the existing owner.
    pub existing: ImagePolicy,
    /// The incompatible policy requested by the caller.
    pub requested: ImagePolicy,
}

impl fmt::Display for ImageRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "image {:?} is already registered as {:?}; cannot register as {:?}",
            self.image_id, self.existing, self.requested
        )
    }
}

impl std::error::Error for ImageRegistrationError {}

/// Native font rasters use the original terminal's nearest atlas sampling.
/// Register before the first upload; ordinary UI images keep linear filtering.
#[derive(Clone)]
pub struct RasterImage(Arc<Registration>);

impl Registration {
    fn new(
        image: Arc<RenderImage>,
        policy: ImagePolicy,
    ) -> Result<Arc<Self>, ImageRegistrationError> {
        let mut images = registrations().lock();
        if let Some(existing) = images.get(&image.id).and_then(Weak::upgrade) {
            drop(images);
            if existing.policy != policy {
                return Err(ImageRegistrationError {
                    image_id: image.id,
                    existing: existing.policy,
                    requested: policy,
                });
            }
            return Ok(existing);
        }
        let owner = Arc::new(Registration { image, policy });
        images.insert(owner.image.id, Arc::downgrade(&owner));
        Ok(owner)
    }
}

impl RasterImage {
    /// Registers straight-alpha pixels for nearest-neighbor sampling.
    ///
    /// # Errors
    /// Returns an error if this image has a live owner with a different policy.
    pub fn straight(image: Arc<RenderImage>) -> Result<Self, ImageRegistrationError> {
        Registration::new(image, ImagePolicy::StraightNearest).map(Self)
    }

    /// Registers premultiplied pixels for nearest-neighbor sampling.
    ///
    /// # Errors
    /// Returns an error if this image has a live owner with a different policy.
    pub fn premultiplied(image: Arc<RenderImage>) -> Result<Self, ImageRegistrationError> {
        Registration::new(image, ImagePolicy::PremultipliedNearest).map(Self)
    }

    pub fn image(&self) -> &Arc<RenderImage> {
        &self.0.image
    }
}

impl PremultipliedImage {
    /// Registers premultiplied pixels without changing linear UI sampling.
    ///
    /// # Errors
    /// Returns an error if this image has a live owner with a different policy.
    pub fn new(image: Arc<RenderImage>) -> Result<Self, ImageRegistrationError> {
        Registration::new(image, ImagePolicy::PremultipliedLinear).map(Self)
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
    owner.map_or(0, |owner| owner.policy.flags())
}

pub(super) fn encode_sprite_flags<'a>(
    image_flags: &FxHashMap<(AtlasTextureId, u32), u32>,
    sprites: &'a [PolychromeSprite],
    scratch: &'a mut Vec<PolychromeSprite>,
) -> &'a [PolychromeSprite] {
    if image_flags.is_empty() || sprites.is_empty() {
        return sprites;
    }
    scratch.clear();
    scratch.extend_from_slice(sprites);
    for sprite in scratch.iter_mut() {
        sprite.pad = image_flags
            .get(&(sprite.tile.texture_id, sprite.tile.tile_id.0))
            .copied()
            .unwrap_or(0);
    }
    scratch
}

#[cfg(test)]
fn is_premultiplied(id: ImageId) -> bool {
    flags(id) & 1 != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AtlasTextureKind, AtlasTile, TileId};

    fn sprite(texture_index: u32, tile_id: u32) -> PolychromeSprite {
        PolychromeSprite {
            order: Default::default(),
            pad: 0,
            grayscale: Default::default(),
            opacity: 1.0,
            bounds: Default::default(),
            content_mask: Default::default(),
            corner_radii: Default::default(),
            tile: AtlasTile {
                texture_id: AtlasTextureId {
                    index: texture_index,
                    kind: AtlasTextureKind::Polychrome,
                },
                tile_id: TileId(tile_id),
                padding: 0,
                bounds: Default::default(),
            },
        }
    }

    #[test]
    fn raster_sampling_and_encoding_are_independent_and_lifetime_bound() {
        let straight = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        let premultiplied = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        let ordinary = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        let owner = RasterImage::straight(straight.clone()).unwrap();
        let duplicate = RasterImage::straight(straight.clone()).unwrap();
        let color = RasterImage::premultiplied(premultiplied.clone()).unwrap();
        let ui = PremultipliedImage::new(ordinary.clone()).unwrap();
        assert!(Arc::ptr_eq(owner.image(), &straight));
        assert_eq!(flags(straight.id), 2);
        assert_eq!(flags(premultiplied.id), 3);
        assert_eq!(flags(ordinary.id), 1);
        drop(owner);
        assert_eq!(flags(straight.id), 2);
        drop(duplicate);
        assert_eq!(flags(straight.id), 0);
        drop(color);
        assert_eq!(flags(premultiplied.id), 0);
        assert_eq!(flags(ui.image().id), 1);
    }

    #[test]
    fn live_image_policy_is_immutable_and_can_be_replaced_after_release() {
        let image = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        let owner = RasterImage::straight(image.clone()).unwrap();
        let conflict = PremultipliedImage::new(image.clone()).err().unwrap();
        assert_eq!(conflict.image_id, image.id);
        assert_eq!(conflict.existing, ImagePolicy::StraightNearest);
        assert_eq!(conflict.requested, ImagePolicy::PremultipliedLinear);
        assert!(conflict.to_string().contains("StraightNearest"));
        assert!(conflict.to_string().contains("PremultipliedLinear"));
        assert_eq!(flags(image.id), 2);
        drop(owner);
        let replacement = PremultipliedImage::new(image.clone()).unwrap();
        assert_eq!(flags(replacement.image().id), 1);
    }

    #[test]
    fn alpha_registration_lives_until_the_last_owner_and_does_not_affect_other_images() {
        let image = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        let other = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        assert!(!is_premultiplied(image.id));
        let owner = PremultipliedImage::new(image.clone()).unwrap();
        assert!(Arc::ptr_eq(owner.image(), &image));
        let duplicate = PremultipliedImage::new(image.clone()).unwrap();
        let cloned = owner.clone();
        assert!(is_premultiplied(image.id));
        assert!(!is_premultiplied(other.id));
        drop(owner);
        drop(duplicate);
        assert!(is_premultiplied(image.id));
        drop(cloned);
        assert!(!is_premultiplied(image.id));
        assert!(!registrations().lock().contains_key(&image.id));
    }

    #[test]
    fn every_conflicting_policy_returns_an_error_without_replacing_the_owner() {
        let policies = [
            ImagePolicy::PremultipliedLinear,
            ImagePolicy::StraightNearest,
            ImagePolicy::PremultipliedNearest,
        ];
        for existing in policies {
            let image = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
            let owner = Registration::new(image.clone(), existing).unwrap();
            for requested in policies {
                let result = Registration::new(image.clone(), requested);
                if existing == requested {
                    assert!(Arc::ptr_eq(&owner, &result.unwrap()));
                } else {
                    let error = result.err().unwrap();
                    assert_eq!(error.image_id, image.id);
                    assert_eq!(error.existing, existing);
                    assert_eq!(error.requested, requested);
                }
                assert_eq!(flags(image.id), existing.flags());
            }
        }
    }

    #[test]
    fn concurrent_lookup_and_last_owner_cleanup_do_not_reenter_the_registry_lock() {
        let image = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..1000 {
                    let owner = PremultipliedImage::new(image.clone()).unwrap();
                    std::thread::yield_now();
                    drop(owner);
                }
            });
            for _ in 0..1000 {
                assert!(matches!(flags(image.id), 0 | 1));
                std::thread::yield_now();
            }
        });
        assert!(!registrations().lock().contains_key(&image.id));
    }

    #[test]
    fn truly_empty_metadata_borrows_original_sprites_without_copying_or_allocating() {
        let metadata = FxHashMap::default();
        assert!(metadata.is_empty());
        let sprites = [sprite(0, 1), sprite(0, 2)];
        let mut scratch = Vec::new();
        let encoded = encode_sprite_flags(&metadata, &sprites, &mut scratch);
        assert!(std::ptr::eq(encoded, sprites.as_slice()));
        assert_eq!(scratch.capacity(), 0);
        assert!(scratch.is_empty());
    }

    #[test]
    fn populated_metadata_encodes_only_gpu_copies_and_empty_metadata_skips_stale_scratch() {
        let sprites = [sprite(0, 1), sprite(0, 2), sprite(1, 1)];
        let mut metadata = FxHashMap::default();
        let key = (sprites[0].tile.texture_id, sprites[0].tile.tile_id.0);
        metadata.insert(key, 3);
        let mut scratch = Vec::new();
        let encoded = encode_sprite_flags(&metadata, &sprites, &mut scratch);
        assert!(!std::ptr::eq(encoded, sprites.as_slice()));
        assert_eq!(
            encoded.iter().map(|sprite| sprite.pad).collect::<Vec<_>>(),
            [3, 0, 0]
        );
        assert!(sprites.iter().all(|sprite| sprite.pad == 0));
        metadata.remove(&key);
        assert!(metadata.is_empty());
        let encoded = encode_sprite_flags(&metadata, &sprites, &mut scratch);
        assert!(std::ptr::eq(encoded, sprites.as_slice()));
        assert_eq!(scratch[0].pad, 3);
    }

    #[test]
    fn uploaded_tile_policy_survives_owner_cleanup_until_tile_removal() {
        let image = Arc::new(RenderImage::new(smallvec::SmallVec::new()));
        let owner = RasterImage::premultiplied(image.clone()).unwrap();
        let sprites = [sprite(0, 1)];
        let key = (sprites[0].tile.texture_id, sprites[0].tile.tile_id.0);
        let mut metadata = FxHashMap::default();
        metadata.insert(key, flags(image.id));
        drop(owner);
        assert_eq!(flags(image.id), 0);
        let mut scratch = Vec::new();
        assert_eq!(
            encode_sprite_flags(&metadata, &sprites, &mut scratch)[0].pad,
            3
        );
        metadata.remove(&key);
        assert!(std::ptr::eq(
            encode_sprite_flags(&metadata, &sprites, &mut scratch),
            sprites.as_slice()
        ));
    }
}
