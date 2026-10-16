use std::{path::PathBuf, sync::mpsc};
use wgpu::util::DeviceExt;
const SIZE: u32 = 128;
struct Harness {
    device: wgpu::Device,
    queue: wgpu::Queue,
    globals: wgpu::BindGroup,
    layout: wgpu::PipelineLayout,
}
impl Harness {
    async fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        println!("GPU: {:?}", adapter.get_info());
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let globals_data = [SIZE as f32, SIZE as f32, 0.0, 0.0];
        let global_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&globals_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let gamma_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &[0; 32],
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: global_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: gamma_buffer.as_entire_binding(),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        Self {
            device,
            queue,
            globals,
            layout,
        }
    }
    fn pipeline(
        &self,
        shader: &wgpu::ShaderModule,
        kind: &str,
        stride: u64,
        attrs: &[wgpu::VertexAttribute],
        constants: &[(&str, f64)],
    ) -> wgpu::RenderPipeline {
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(kind),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(&format!("vs_{kind}")),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: attrs,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(&format!("fs_{kind}")),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
    }
    fn render(&self, pipeline: &wgpu::RenderPipeline, bytes: &[u8]) -> Vec<u8> {
        let extent = wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
        let result = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.globals, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..4, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &result,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            extent,
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        result
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        let pixels = result.slice(..).get_mapped_range().unwrap().to_vec();
        result.unmap();
        pixels
    }
}
fn write_floats(bytes: &mut [u8], offset: usize, values: &[f32]) {
    for (i, v) in values.iter().enumerate() {
        bytes[offset + i * 4..offset + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
}
fn write_uint(bytes: &mut [u8], offset: usize, v: u32) {
    bytes[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
}
fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    assert_eq!(paths.len(), 2);
    let h = pollster::block_on(Harness::new());
    let shaders: Vec<_> = paths
        .iter()
        .map(|p| {
            h.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: p.to_str(),
                source: wgpu::ShaderSource::Wgsl(std::fs::read_to_string(p).unwrap().into()),
            })
        })
        .collect();
    use wgpu::VertexFormat::*;
    let kinds = [
        (
            "quad",
            160,
            vec![
                (0, Uint32x2),
                (8, Float32x2),
                (16, Float32x2),
                (24, Float32x2),
                (32, Float32x2),
                (40, Uint32x2),
                (48, Float32x4),
                (64, Float32),
                (68, Float32x4),
                (84, Float32),
                (88, Float32x4),
                (104, Float32),
                (112, Float32x4),
                (128, Float32x4),
                (144, Float32x4),
            ],
        ),
        (
            "shadow",
            72,
            vec![
                (0, Uint32),
                (4, Float32),
                (8, Float32x2),
                (16, Float32x2),
                (24, Float32x4),
                (40, Float32x2),
                (48, Float32x2),
                (56, Float32x4),
            ],
        ),
        (
            "underline",
            64,
            vec![
                (0, Uint32),
                (4, Uint32),
                (8, Float32x2),
                (16, Float32x2),
                (24, Float32x2),
                (32, Float32x2),
                (40, Float32x4),
                (56, Float32),
                (60, Uint32),
            ],
        ),
    ];
    let masks = [
        [0., 0., 128., 128.],
        [32., 24., 48., 64.],
        [32.25, 24.75, 48.25, 64.5],
        [32.5, 24.5, 48., 64.],
        [127., 127., 1., 1.],
        [-40., -40., 20., 20.],
        [50., 50., 0., 0.],
        [60.001, 39.999, 20.001, 35.999],
    ];
    let mut failures = 0;
    let mut tests = 0;
    for (kind, stride, attrs) in kinds {
        let attributes: Vec<_> = attrs
            .into_iter()
            .enumerate()
            .map(|(i, (offset, format))| wgpu::VertexAttribute {
                offset,
                format,
                shader_location: i as u32,
            })
            .collect();
        let pipelines: Vec<_> = shaders
            .iter()
            .map(|s| h.pipeline(s, kind, stride, &attributes, &[]))
            .collect();
        for bounds in [
            [8.25, 10.75, 103.5, 100.25],
            [-24.25, 5.75, 103.5, 100.25],
            [45.25, -19.5, 103.5, 100.25],
            [-500., -200., 900., 700.],
        ] {
            for (m, mask) in masks.iter().enumerate() {
                for style in 0..6 {
                    let mut bytes = vec![0; stride as usize];
                    write_floats(&mut bytes, 8, &bounds);
                    match kind {
                        "quad" => {
                            write_floats(&mut bytes, 24, mask);
                            write_uint(&mut bytes, 40, if style >= 3 { 1 } else { 0 });
                            write_floats(&mut bytes, 48, &[0.55, 0.8, 0.6, 0.7]);
                            write_floats(&mut bytes, 64, &[35.]);
                            write_floats(&mut bytes, 68, &[0.12, 0.9, 0.4, 0.8]);
                            write_floats(&mut bytes, 84, &[0.]);
                            write_floats(&mut bytes, 88, &[0.6, 0.8, 0.4, 0.9]);
                            write_floats(&mut bytes, 104, &[1.]);
                            write_floats(&mut bytes, 112, &[0.02, 0.9, 0.5, 0.8]);
                            if style % 3 > 0 {
                                write_floats(&mut bytes, 128, &[12., 4., 23., 8.]);
                            }
                            if style % 3 > 1 {
                                write_floats(&mut bytes, 144, &[1.5, 3.25, 4., 0.75]);
                            }
                        }
                        "shadow" => {
                            write_floats(&mut bytes, 4, &[2.25 + 2.25 * style as f32]);
                            write_floats(&mut bytes, 24, &[8., 12., 4., 16.]);
                            write_floats(&mut bytes, 40, mask);
                            write_floats(&mut bytes, 56, &[0.55, 0.8, 0.6, 0.7]);
                        }
                        _ => {
                            write_floats(&mut bytes, 24, mask);
                            write_floats(&mut bytes, 40, &[0.55, 0.8, 0.6, 0.7]);
                            write_floats(&mut bytes, 56, &[1.25 + style as f32]);
                            write_uint(&mut bytes, 60, (style % 2) as u32);
                        }
                    }
                    let a = h.render(&pipelines[0], &bytes);
                    let b = h.render(&pipelines[1], &bytes);
                    let diff = a.iter().zip(&b).filter(|(a, b)| a != b).count();
                    let max = a.iter().zip(&b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                    tests += 1;
                    if diff > 0 {
                        println!("DIFF {kind} mask={m} style={style} bytes={diff} max={max}");
                        failures += 1;
                    }
                }
            }
        }
    }
    println!("Compared {tests} cases; {failures} with pixel differences");
    assert_eq!(failures, 0);
}
