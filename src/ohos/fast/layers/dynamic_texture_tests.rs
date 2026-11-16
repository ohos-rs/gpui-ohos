use super::*;
use crate::ohos::dynamic_textures::{DynamicTextureUploads, DynamicTextures};
use crate::ohos::dynamic_textures::{GpuTexture, GpuTextureError};
use gpui::{
    AppContext as _, DynamicTexture, DynamicTexturePlatform, ParentElement as _, Styled as _,
};

struct TextureFixture(DynamicTexture);

impl gpui::Render for TextureFixture {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let texture = self.0;
        gpui::div()
            .relative()
            .size(px(64.))
            .bg(gpui::white())
            .child(
                gpui::div()
                    .w(px(48.))
                    .h(px(64.))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .child(
                        gpui::canvas(
                            |_, _, _| (),
                            move |bounds, (), window, _| {
                                window.paint_dynamic_texture(bounds, texture).unwrap();
                            },
                        )
                        .size(px(64.)),
                    ),
            )
            .child(
                gpui::div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(8.))
                    .bg(gpui::black()),
            )
    }
}

fn assert_pixel(pixels: &[u8], side: usize, x: usize, y: usize, expected: [u8; 4]) {
    let actual = pixel(pixels, side, x, y);
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
        "{actual:?} != {expected:?}"
    );
}

fn revision(scene: &Scene, id: gpui::DynamicTextureId) -> Option<u32> {
    scene
        .polychrome_sprites
        .iter()
        .find(|sprite| sprite.tile.texture_id == id.atlas_texture_id())
        .map(|sprite| sprite.pad)
        .or_else(|| {
            scene
                .layers
                .frames
                .iter()
                .find_map(|frame| frame.content.scene().and_then(|scene| revision(scene, id)))
        })
}

#[gpui::test]
fn window_uploads_update_clipped_pixels_and_release_textures(cx: &mut gpui::TestAppContext) {
    let mut harness = Harness::new().expect("dynamic texture tests require a GPU");
    harness.uploads =
        DynamicTextureUploads::install(harness.device.limits().max_texture_dimension_2d);
    harness.dynamic_textures = Arc::new(DynamicTextures::new(
        harness.device.clone(),
        harness.queue.clone(),
        harness.uploads.clone(),
    ));
    let mut bytes = [0xaa; 96];
    for row in bytes.chunks_exact_mut(24) {
        for pixel in row[..16].chunks_exact_mut(4) {
            pixel.copy_from_slice(&[17, 34, 200, 128]);
        }
    }
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            let texture = window
                .create_dynamic_texture(device_size(4, 4), &bytes[..88], 24)
                .unwrap();
            cx.new(|_| TextureFixture(texture))
        })
        .unwrap()
    });

    let (texture, gpu_view, first_revision) = window
        .update(cx, |fixture, window, cx| {
            window.draw(cx).clear(cx);
            let scale = window.scale_factor() as usize;
            let side = 64 * scale;
            let extent = device_size(side as i32, side as i32);
            let scene = window.painted_scene();
            let version = revision(scene, fixture.0.id()).unwrap();
            let direct = harness.render(scene, extent, rgba(0xffffffff));
            assert_pixel(&direct, side, 16 * scale, 16 * scale, [227, 144, 136, 255]);
            assert_pixel(&direct, side, 56 * scale, 16 * scale, [255; 4]);
            assert_pixel(&direct, side, 4 * scale, 4 * scale, [0, 0, 0, 255]);
            let tiled = tiled_scene(scene, extent);
            let actual = harness.render(&tiled, extent, rgba(0xffffffff));
            assert_same_pixels(&actual, &direct, side);
            assert_eq!(revision(scene, fixture.0.id()), Some(version));
            (
                fixture.0,
                harness.dynamic_textures.view(fixture.0.id()).unwrap(),
                version,
            )
        })
        .unwrap();

    window
        .update(cx, |_, window, _| {
            let mut patch = [0xcc; 24];
            for row in patch.chunks_exact_mut(12) {
                row[..8].copy_from_slice(&[200, 0, 0, 128, 200, 0, 0, 128]);
            }
            window
                .update_dynamic_texture(
                    texture,
                    Bounds::new(point(DevicePixels(0), DevicePixels(0)), device_size(2, 2)),
                    &patch[..20],
                    12,
                )
                .unwrap();
            window
                .update_dynamic_texture(
                    texture,
                    Bounds::new(point(DevicePixels(2), DevicePixels(2)), device_size(2, 2)),
                    &[0, 200, 0, 128].repeat(4),
                    8,
                )
                .unwrap();
            assert!(
                window
                    .update_dynamic_texture(
                        texture,
                        Bounds::new(point(DevicePixels(3), DevicePixels(3)), device_size(2, 2)),
                        &[0; 16],
                        8
                    )
                    .is_err()
            );
            assert!(
                window
                    .create_dynamic_texture(device_size(i32::MAX, 1), &[0; 4], 4)
                    .is_err()
            );
        })
        .unwrap();

    window
        .update(cx, |_, window, cx| {
            // No entity notification: the update API itself invalidates retained views.
            window.draw(cx).clear(cx);
            let scale = window.scale_factor() as usize;
            let side = 64 * scale;
            let extent = device_size(side as i32, side as i32);
            let scene = window.painted_scene();
            let version = revision(scene, texture.id()).unwrap();
            assert!(version > first_revision);
            let actual = harness.render(scene, extent, rgba(0xffffffff));
            assert_pixel(&actual, side, 16 * scale, 16 * scale, [127, 127, 227, 255]);
            assert_pixel(&actual, side, 40 * scale, 16 * scale, [227, 144, 136, 255]);
            assert_pixel(&actual, side, 40 * scale, 40 * scale, [127, 227, 127, 255]);
            assert_eq!(
                harness.dynamic_textures.view(texture.id()).unwrap(),
                gpu_view
            );
            let mut tiled = tiled_scene(scene, extent);
            tiled.layers.frames[0].generation = u64::from(version);
            let through_tiles = harness.render(&tiled, extent, rgba(0xffffffff));
            assert_same_pixels(&through_tiles, &actual, side);

            window.remove_dynamic_texture(texture);
            assert!(harness.dynamic_textures.view(texture.id()).is_none());
            let removed = harness.render(window.painted_scene(), extent, rgba(0xffffffff));
            assert_pixel(&removed, side, 16 * scale, 16 * scale, [255; 4]);
            assert!(
                window
                    .update_dynamic_texture(
                        texture,
                        Bounds::new(point(DevicePixels(0), DevicePixels(0)), device_size(1, 1)),
                        &[0; 4],
                        4
                    )
                    .is_err()
            );
            window.remove_dynamic_texture(texture);
        })
        .unwrap();
}

#[test]
fn texture_updates_reach_each_device_and_survive_context_recreation() {
    let mut first = Harness::new().expect("GPU required");
    let mut second = Harness::new().expect("second GPU required");
    second.uploads = first.uploads.clone();
    second.dynamic_textures = Arc::new(DynamicTextures::new(
        second.device.clone(),
        second.queue.clone(),
        second.uploads.clone(),
    ));
    let texture = DynamicTexture::new_gpu(device_size(4, 4)).unwrap();
    first
        .uploads
        .create_texture(
            texture.id(),
            device_size(4, 4),
            &[0, 0, 255, 255].repeat(16),
        )
        .unwrap();
    let mut scene = Scene::default();
    scene.insert_primitive(PolychromeSprite {
        order: 0,
        pad: 1,
        grayscale: false.into(),
        opacity: 1.,
        bounds: sp(0., 0., 32., 32.),
        content_mask: no_mask(),
        corner_radii: Default::default(),
        tile: AtlasTile {
            texture_id: texture.id().atlas_texture_id(),
            tile_id: gpui::TileId(0),
            padding: 0,
            bounds: Bounds::new(point(DevicePixels(0), DevicePixels(0)), device_size(4, 4)),
        },
    });
    scene.finish();
    let extent = device_size(32, 32);
    let before = first.render(&scene, extent, rgba(0xffffffff));
    assert_pixel(&before, 32, 8, 8, [255, 0, 0, 255]);
    assert_same_pixels(
        &second.render(&scene, extent, rgba(0xffffffff)),
        &before,
        32,
    );
    let old_view = first.dynamic_textures.view(texture.id()).unwrap();

    // This small update retains both the GPU allocation and untouched pixels.
    first
        .uploads
        .update_texture(
            texture.id(),
            Bounds::new(point(DevicePixels(1), DevicePixels(1)), device_size(1, 1)),
            &[255, 0, 0, 255],
        )
        .unwrap();
    assert!(
        first
            .uploads
            .update_texture(
                texture.id(),
                Bounds::new(point(DevicePixels(4), DevicePixels(0)), device_size(1, 1)),
                &[0; 4]
            )
            .is_err()
    );
    assert!(
        first
            .uploads
            .update_texture(
                texture.id(),
                Bounds::new(point(DevicePixels(0), DevicePixels(0)), device_size(1, 1)),
                &[0; 3]
            )
            .is_err()
    );
    assert!(
        first
            .uploads
            .create_texture(texture.id(), device_size(4, 4), &[0; 64])
            .is_err()
    );
    let after = first.render(&scene, extent, rgba(0xffffffff));
    assert_eq!(old_view, first.dynamic_textures.view(texture.id()).unwrap());
    assert_ne!(before, after);
    assert_pixel(&after, 32, 28, 28, [255, 0, 0, 255]);
    assert_same_pixels(&second.render(&scene, extent, rgba(0xffffffff)), &after, 32);
    second.dynamic_textures = Arc::new(DynamicTextures::new(
        second.device.clone(),
        second.queue.clone(),
        second.uploads.clone(),
    ));
    assert_same_pixels(&second.render(&scene, extent, rgba(0xffffffff)), &after, 32);

    first.uploads.remove_texture(texture.id());
    assert!(first.dynamic_textures.view(texture.id()).is_none());
    assert!(second.dynamic_textures.view(texture.id()).is_none());
    texture.remove();
}

struct GpuFixture(std::sync::Weak<GpuTexture>);

impl gpui::Render for GpuFixture {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let texture = self.0.clone();
        gpui::div().size(px(32.)).bg(gpui::white()).child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    if let Some(texture) = texture.upgrade() {
                        texture.paint(window, bounds).unwrap();
                    }
                },
            )
            .size(px(32.)),
        )
    }
}

fn clear_target(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    color: wgpu::Color,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    queue.submit([encoder.finish()]);
}

#[gpui::test]
fn gpu_producers_refresh_and_release_context_local_targets(cx: &mut gpui::TestAppContext) {
    let mut harness = Harness::new().expect("GPU required");
    let mut other = Harness::new().expect("second GPU required");
    assert!(matches!(
        harness.dynamic_textures.create_gpu(device_size(0, 4)),
        Err(GpuTextureError::InvalidSize { .. })
    ));
    let mut owner = Some(Arc::new(
        harness
            .dynamic_textures
            .create_gpu(device_size(4, 4))
            .unwrap(),
    ));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| GpuFixture(Arc::downgrade(owner.as_ref().unwrap())))
        })
        .unwrap()
    });
    window
        .update(cx, |_, window, _| {
            owner
                .as_ref()
                .unwrap()
                .render(window, |device, queue, view| {
                    assert_eq!(view.texture().format(), wgpu::TextureFormat::Rgba8Unorm);
                    clear_target(
                        device,
                        queue,
                        view,
                        wgpu::Color {
                            r: 200. / 255.,
                            g: 0.,
                            b: 0.,
                            a: 128. / 255.,
                        },
                    );
                    Ok(())
                })
                .unwrap();
        })
        .unwrap();
    let (id, version) = window
        .update(cx, |_, window, cx| {
            window.draw(cx).clear(cx);
            let side = (32. * window.scale_factor()) as usize;
            let extent = device_size(side as i32, side as i32);
            let scene = window.painted_scene();
            let sprite = scene.polychrome_sprites.first().unwrap();
            let id = gpui::DynamicTextureId::from_atlas_texture_id(sprite.tile.texture_id).unwrap();
            let pixels = harness.render(scene, extent, rgba(0xffffffff));
            assert_pixel(&pixels, side, side / 2, side / 2, [227, 127, 127, 255]);
            let tiles = tiled_scene(scene, extent);
            assert_same_pixels(
                &harness.render(&tiles, extent, rgba(0xffffffff)),
                &pixels,
                side,
            );
            let foreign = other.render(scene, extent, rgba(0xffffffff));
            assert_pixel(&foreign, side, side / 2, side / 2, [255; 4]);
            (id, sprite.pad)
        })
        .unwrap();

    window
        .update(cx, |_, window, _| {
            let result: Result<(), GpuTextureError> = owner
                .as_ref()
                .unwrap()
                .render(window, |_, _, _| anyhow::bail!("producer failed"));
            assert!(matches!(result, Err(GpuTextureError::Operation(_))));
            owner
                .as_ref()
                .unwrap()
                .render(window, |device, queue, view| {
                    // Registry access inside the callback must not deadlock.
                    assert!(harness.dynamic_textures.view(id).is_some());
                    clear_target(device, queue, view, wgpu::Color::BLUE);
                    Ok(())
                })
                .unwrap();
        })
        .unwrap();
    window
        .update(cx, |_, window, cx| {
            window.draw(cx).clear(cx);
            let side = (32. * window.scale_factor()) as usize;
            let extent = device_size(side as i32, side as i32);
            let scene = window.painted_scene();
            assert_eq!(revision(scene, id), Some(version + 1));
            let pixels = harness.render(scene, extent, rgba(0xffffffff));
            assert_pixel(&pixels, side, side / 2, side / 2, [0, 0, 255, 255]);
            drop(owner.take());
            assert!(harness.dynamic_textures.view(id).is_none());
            let pixels = harness.render(scene, extent, rgba(0xffffffff));
            assert_pixel(&pixels, side, side / 2, side / 2, [255; 4]);
        })
        .unwrap();
}
