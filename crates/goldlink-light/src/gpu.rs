//! The lighting and the cast shadows on the GPU (dev/docs/unified-client/UC14_GPU_LIGHTING.md):
//! wgpu, a device of its own. The CPU finds the scene (each pixel's place, the light sources, the
//! casters and each map's triangles: lib.rs, shadow.rs); this draws the maps and lights the
//! picture (light.wgsl: the same steps as the CPU's), and reads it back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};

use crate::shadow::{DepthMap, Shadows, Tri, View};

/// One pixel (light.wgsl `Pixel`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub(crate) struct Pixel {
    pub pos: [f32; 4],
    pub info: [u32; 4],
}

/// A light (light.wgsl `Light`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub(crate) struct Light {
    pub pos: [f32; 4],
    pub colour: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Face {
    pos: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    upward: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Card {
    origin: [f32; 4],
    normal: [f32; 4],
    across: [f32; 4],
    up: [f32; 4],
    ab: [f32; 4],
    foot: [f32; 4],
}

/// The frame's constants (light.wgsl `Frame`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub(crate) struct Frame {
    pub dims: [u32; 4],
    pub counts: [u32; 4],
    pub params: [f32; 4],
    pub key_dir: [f32; 4],
    pub ambient: [f32; 4],
    pub key: [f32; 4],
    pub tint: [f32; 4],
    pub shadow: [f32; 4],
    pub high: [f32; 4],
    pub up: [f32; 4],
    pub proj: [[f32; 4]; 4],
    pub rows: [[f32; 4]; 4],
    pub fit: [f32; 4],
    pub sun_dir: [f32; 4],
    pub sun_u: [f32; 4],
    pub sun_v: [f32; 4],
    pub sun_view: [f32; 4],
    pub sun_side: [u32; 4],
    pub taps: [[i32; 4]; 8],
    pub bloom: [f32; 4],
}

/// A triangle's corner on a map (maps.wgsl `In`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MapVert {
    pos: [f32; 4],
    raw: f32,
    st: [f32; 2],
    tex: [u32; 4],
    card: u32,
}

/// The largest sun map, a point light's face's tile in the atlas, and the atlas.
const SUN_SIDE: u32 = 1536;
const TILE: u32 = 64;
const ATLAS: [u32; 2] = [2048, 1024];
/// The card map: the scene's picture.
const PICTURE: [u32; 2] = [384, 192];
/// The textures' alpha, a row.
const ALPHA_ROW: u32 = 4096;
/// Words per pixel out of the light pass: its colour, its light as the world and as the player,
/// the bloom and the light sources' glow there (light.wgsl `WORDS`).
const WORDS: usize = 7;

/// A map's textures: how near the light (raw), the card, the depth test's.
struct Target {
    raw: wgpu::TextureView,
    card: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl Target {
    fn new(device: &wgpu::Device, size: [u32; 2], name: &str) -> Self {
        let make = |format, usage, label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let both = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        Self {
            raw: make(wgpu::TextureFormat::R32Float, both, name),
            card: make(wgpu::TextureFormat::R32Uint, both, name),
            depth: make(
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                name,
            ),
        }
    }
}

/// A storage buffer that grows as needed.
struct Grow {
    buf: wgpu::Buffer,
    size: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl Grow {
    fn new(device: &wgpu::Device, usage: wgpu::BufferUsages, label: &'static str) -> Self {
        Self {
            buf: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 256,
                usage,
                mapped_at_creation: false,
            }),
            size: 256,
            usage,
            label,
        }
    }

    /// At least `n` bytes (a multiple of 4, at least 256).
    fn fit(&mut self, device: &wgpu::Device, n: u64) {
        let n = n.max(256).next_multiple_of(4);
        if n > self.size {
            let size = n.next_power_of_two();
            self.buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size,
                usage: self.usage,
                mapped_at_creation: false,
            });
            self.size = size;
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        self.fit(device, data.len() as u64);
        if !data.is_empty() {
            queue.write_buffer(&self.buf, 0, data);
        }
    }
}

pub(crate) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Set on a GPU error: the lighting turns off.
    failed: Arc<AtomicBool>,
    maps: wgpu::RenderPipeline,
    maps_layout: wgpu::BindGroupLayout,
    light_layout: wgpu::BindGroupLayout,
    passes: [wgpu::ComputePipeline; 5],
    cards_lit: wgpu::ComputePipeline,
    bloom_layout: wgpu::BindGroupLayout,
    /// The bloom's passes: down to half size, across, down, the finish; the glow's: seed, across,
    /// down (light.wgsl).
    bloom_passes: [wgpu::ComputePipeline; 4],
    glow_passes: [wgpu::ComputePipeline; 3],
    sun: Target,
    atlas: Target,
    card_px: Target,
    alpha: (wgpu::Texture, u32),
    frame: wgpu::Buffer,
    pix: Grow,
    work0: Grow,
    work1: Grow,
    outp: Grow,
    readback: Grow,
    lights: Grow,
    faces: Grow,
    cards: Grow,
    card_light: Grow,
    verts: Grow,
    bloom_a: Grow,
    bloom_b: Grow,
    glow_a: Grow,
    glow_b: Grow,
}

/// What the lighting hands the GPU for a frame.
pub(crate) struct Input<'a> {
    pub frame: Frame,
    pub pixels: &'a [Pixel],
    pub lights: &'a [Light],
    pub shadows: &'a Shadows,
}

impl Gpu {
    /// The lighting on a device of its own (on the lighting's own thread; the window's device
    /// stalled it behind the window's wait for the screen, 5 ms a frame), or None: no usable GPU,
    /// or its shaders can't run there (the lighting is off).
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("goldlink-light"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;
        Self::with(device, queue)
    }

    /// The lighting on `device` and `queue`, or None when its shaders can't run there.
    pub fn with(device: wgpu::Device, queue: wgpu::Queue) -> Option<Self> {
        let failed = Arc::new(AtomicBool::new(false));
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let gpu = Self::build(device, queue, failed);
        let error = pollster::block_on(gpu.device.pop_error_scope());
        if let Some(e) = error {
            eprintln!("[goldlink-light] the GPU can't light the picture, lighting off: {e}");
            return None;
        }
        Some(gpu)
    }

    fn build(device: wgpu::Device, queue: wgpu::Queue, failed: Arc<AtomicBool>) -> Self {
        let maps_src = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("maps"),
            source: wgpu::ShaderSource::Wgsl(include_str!("maps.wgsl").into()),
        });
        let light_src = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("light"),
            source: wgpu::ShaderSource::Wgsl(include_str!("light.wgsl").into()),
        });

        let maps_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("maps"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let maps = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("maps"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("maps"),
                    bind_group_layouts: &[&maps_layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &maps_src,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MapVert>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32, 2 => Float32x2, 3 => Uint32x4, 4 => Uint32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &maps_src,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::TextureFormat::R32Float.into()),
                    Some(wgpu::TextureFormat::R32Uint.into()),
                ],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let uint = wgpu::TextureSampleType::Uint;
        let light_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, true),
                storage(6, true),
                storage(7, true),
                storage(8, false),
                texture(9, float),
                texture(10, uint),
                texture(11, float),
                texture(12, uint),
                texture(13, uint),
            ],
        });
        let light_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("light"),
                bind_group_layouts: &[&light_layout],
                push_constant_ranges: &[],
            });
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&light_pipeline_layout),
                module: &light_src,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let passes = [
            compute("normals_raw"),
            compute("normals_smooth"),
            compute("occlusion_raw"),
            compute("occlusion_smooth"),
            compute("shade"),
        ];
        let cards_lit = compute("cards_lit");
        // The bloom's passes have a bind group of their own: the light pass's has the most
        // storage buffers a stage may have (eight, wgpu's default limits).
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(4, false),
                storage(14, false),
                storage(15, false),
                storage(16, false),
                storage(17, false),
            ],
        });
        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bloom"),
                bind_group_layouts: &[&bloom_layout],
                push_constant_ranges: &[],
            });
        let bloom_pass = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&bloom_pipeline_layout),
                module: &light_src,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let bloom_passes = [
            bloom_pass("bloom_down"),
            bloom_pass("bloom_across"),
            bloom_pass("bloom_down_blur"),
            bloom_pass("finish"),
        ];
        let glow_passes = [
            bloom_pass("glow_seed"),
            bloom_pass("glow_across"),
            bloom_pass("glow_down"),
        ];

        let alpha = Self::alpha_texture(&device, 1);
        let storage_rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        Self {
            sun: Target::new(&device, [SUN_SIDE, SUN_SIDE], "sun"),
            atlas: Target::new(&device, ATLAS, "atlas"),
            card_px: Target::new(&device, PICTURE, "card_px"),
            alpha: (alpha, 1),
            frame: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame"),
                size: std::mem::size_of::<Frame>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pix: Grow::new(&device, storage_rw, "pix"),
            work0: Grow::new(&device, storage_rw, "work0"),
            work1: Grow::new(&device, storage_rw, "work1"),
            outp: Grow::new(
                &device,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                "outp",
            ),
            readback: Grow::new(
                &device,
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                "readback",
            ),
            lights: Grow::new(&device, storage_rw, "lights"),
            faces: Grow::new(&device, storage_rw, "faces"),
            cards: Grow::new(&device, storage_rw, "cards"),
            card_light: Grow::new(&device, storage_rw, "card_light"),
            verts: Grow::new(
                &device,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "verts",
            ),
            bloom_a: Grow::new(&device, wgpu::BufferUsages::STORAGE, "bloom_a"),
            bloom_b: Grow::new(&device, wgpu::BufferUsages::STORAGE, "bloom_b"),
            glow_a: Grow::new(&device, wgpu::BufferUsages::STORAGE, "glow_a"),
            glow_b: Grow::new(&device, wgpu::BufferUsages::STORAGE, "glow_b"),
            device,
            queue,
            failed,
            maps,
            maps_layout,
            light_layout,
            passes,
            cards_lit,
            bloom_layout,
            bloom_passes,
            glow_passes,
        }
    }

    fn alpha_texture(device: &wgpu::Device, rows: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("alpha"),
            size: wgpu::Extent3d {
                width: ALPHA_ROW,
                height: rows,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Whether a GPU error turned the lighting off.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Lights the frame: `out` gets each pixel (0x00RRGGBB), `lights` each pixel's light as the
    /// world and as the player, the bloom and the glow there (light.wgsl `shade`, `finish`);
    /// false when the GPU failed.
    pub fn run(&mut self, input: &Input, out: &mut [u32], lights: &mut Vec<[u32; 6]>) -> bool {
        if self.failed() {
            return false;
        }
        let mut frame = input.frame;
        let (w, h) = (frame.dims[0], frame.dims[1]);
        let n = (w * h) as usize;
        if input.pixels.len() < n || out.len() < n || n == 0 {
            return false;
        }
        let sh = input.shadows;

        // The maps' triangles, each map's range of corners.
        let mut verts: Vec<MapVert> = Vec::new();
        let mut sun_range = 0..0;
        // (tile x, tile y, size, range)
        let mut face_draws: Vec<(u32, u32, u32, std::ops::Range<u32>)> = Vec::new();
        let mut faces: Vec<Face> = Vec::new();
        let mut card_range = 0..0;
        if sh.ready {
            if let Some((map, dir)) = &sh.sun {
                let View::Sun {
                    u,
                    v,
                    origin,
                    texels,
                    ..
                } = map.view
                else {
                    unreachable!("the sun's map looks along the sun");
                };
                let side = [map.side[0] as u32, map.side[1] as u32];
                let start = verts.len() as u32;
                push_tris(&mut verts, &map.tris, [side[0] as f32, side[1] as f32]);
                sun_range = start..verts.len() as u32;
                frame.counts[3] |= 2;
                frame.sun_dir = [dir[0], dir[1], dir[2], 0.0];
                frame.sun_u = [u[0], u[1], u[2], 0.0];
                frame.sun_v = [v[0], v[1], v[2], 0.0];
                frame.sun_view = [origin[0], origin[1], texels, 0.0];
                frame.sun_side = [side[0].min(SUN_SIDE), side[1].min(SUN_SIDE), 0, 0];
            }
            let per_row = ATLAS[0] / TILE;
            for (s, pm) in sh.points.iter().enumerate() {
                for f in 0..6 {
                    let t = (s * 6 + f) as u32;
                    let (tx, ty) = ((t % per_row) * TILE, (t / per_row) * TILE);
                    let map: Option<&DepthMap> = pm.faces.get(f);
                    let mut face = Face {
                        pos: [pm.pos[0], pm.pos[1], pm.pos[2], 0.0],
                        ..Default::default()
                    };
                    if let Some(map) = map
                        && let View::Face {
                            forward,
                            right,
                            upward,
                            size,
                            ..
                        } = map.view
                        && map.side[0] > 0
                        && ty + TILE <= ATLAS[1]
                    {
                        face.pos[3] = size;
                        face.forward = [forward[0], forward[1], forward[2], tx as f32];
                        face.right = [right[0], right[1], right[2], ty as f32];
                        face.upward = [upward[0], upward[1], upward[2], 0.0];
                        let start = verts.len() as u32;
                        push_tris(&mut verts, &map.tris, [size, size]);
                        face_draws.push((tx, ty, size as u32, start..verts.len() as u32));
                    }
                    faces.push(face);
                }
            }
            let start = verts.len() as u32;
            push_tris(
                &mut verts,
                &sh.card_tris,
                [PICTURE[0] as f32, PICTURE[1] as f32],
            );
            card_range = start..verts.len() as u32;
            frame.counts[3] |= 1;
            frame.fit = sh.fit;
            frame.proj = sh.proj;
            frame.rows = sh.rows;
            frame.up = [sh.up[0], sh.up[1], sh.up[2], 0.0];
        }
        let cards: Vec<Card> = if sh.ready {
            sh.cards
                .iter()
                .map(|c| Card {
                    origin: [
                        c.origin[0],
                        c.origin[1],
                        c.origin[2],
                        f32::from(u8::from(c.attached)),
                    ],
                    normal: [c.normal[0], c.normal[1], c.normal[2], 0.0],
                    across: [c.across[0], c.across[1], c.across[2], 0.0],
                    up: [c.up[0], c.up[1], c.up[2], 0.0],
                    ab: [c.a[0], c.a[1], c.b[0], c.b[1]],
                    foot: [c.foot[0], c.foot[1], c.foot[2], 0.0],
                })
                .collect()
        } else {
            Vec::new()
        };
        let slots = if sh.ready { sh.points.len() as u32 } else { 0 };
        frame.counts[1] = cards.len() as u32;
        frame.counts[2] = slots;

        // Uploads.
        let (device, queue) = (&self.device, &self.queue);
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&frame));
        self.pix
            .write(device, queue, bytemuck::cast_slice(&input.pixels[..n]));
        self.work0.fit(device, (n * 16) as u64);
        self.work1.fit(device, (n * 16) as u64);
        self.outp.fit(device, (n * WORDS * 4) as u64);
        self.readback.fit(device, (n * WORDS * 4) as u64);
        let half = (w.div_ceil(2) * h.div_ceil(2)) as u64 * 16;
        self.bloom_a.fit(device, half);
        self.bloom_b.fit(device, half);
        self.glow_a.fit(device, (n * 16) as u64);
        self.glow_b.fit(device, (n * 16) as u64);
        self.lights
            .write(device, queue, bytemuck::cast_slice(input.lights));
        self.faces
            .write(device, queue, bytemuck::cast_slice(&faces));
        self.cards
            .write(device, queue, bytemuck::cast_slice(&cards));
        self.card_light
            .fit(device, (cards.len() as u64 * (1 + slots as u64)) * 4);
        self.verts
            .write(device, queue, bytemuck::cast_slice(&verts));
        if sh.ready {
            let rows = (sh.alpha.len() as u32).div_ceil(ALPHA_ROW).max(1);
            if rows > self.alpha.1 {
                self.alpha = (Self::alpha_texture(device, rows.next_power_of_two()), rows);
                self.alpha.1 = self.alpha.0.height();
            }
            let mut data = sh.alpha.clone();
            data.resize((rows * ALPHA_ROW) as usize, 31);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.alpha.0,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ALPHA_ROW),
                    rows_per_image: Some(rows),
                },
                wgpu::Extent3d {
                    width: ALPHA_ROW,
                    height: rows,
                    depth_or_array_layers: 1,
                },
            );
        }

        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut enc = device.create_command_encoder(&Default::default());
        let alpha_view = self.alpha.0.create_view(&Default::default());
        let maps_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("maps"),
            layout: &self.maps_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&alpha_view),
            }],
        });
        // The maps: cleared (nothing over anything), then the triangles.
        let map_pass =
            |enc: &mut wgpu::CommandEncoder,
             t: &Target,
             draws: &[(u32, u32, u32, u32, std::ops::Range<u32>)]| {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("map"),
                    color_attachments: &[
                        Some(wgpu::RenderPassColorAttachment {
                            view: &t.raw,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: -3.0e38,
                                    g: 0.0,
                                    b: 0.0,
                                    a: 0.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        }),
                        Some(wgpu::RenderPassColorAttachment {
                            view: &t.card,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        }),
                    ],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                if verts.is_empty() {
                    return;
                }
                pass.set_pipeline(&self.maps);
                pass.set_bind_group(0, &maps_group, &[]);
                pass.set_vertex_buffer(0, self.verts.buf.slice(..));
                for (x, y, sw, shh, range) in draws {
                    if range.is_empty() {
                        continue;
                    }
                    pass.set_viewport(*x as f32, *y as f32, *sw as f32, *shh as f32, 0.0, 1.0);
                    pass.set_scissor_rect(*x, *y, *sw, *shh);
                    pass.draw(range.clone(), 0..1);
                }
            };
        let side = frame.sun_side;
        map_pass(
            &mut enc,
            &self.sun,
            &[(0, 0, side[0].max(1), side[1].max(1), sun_range)],
        );
        let face_draws: Vec<_> = face_draws
            .into_iter()
            .map(|(x, y, s, r)| (x, y, s, s, r))
            .collect();
        map_pass(&mut enc, &self.atlas, &face_draws);
        map_pass(
            &mut enc,
            &self.card_px,
            &[(0, 0, PICTURE[0], PICTURE[1], card_range)],
        );

        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light"),
            layout: &self.light_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.pix.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.work0.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.work1.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.outp.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.lights.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.faces.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.cards.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.card_light.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&self.sun.raw),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&self.sun.card),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(&self.atlas.raw),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(&self.atlas.card),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::TextureView(&self.card_px.card),
                },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &group, &[]);
            let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
            for p in &self.passes[..4] {
                pass.set_pipeline(p);
                pass.dispatch_workgroups(gx, gy, 1);
            }
            let jobs = frame.counts[1] * (1 + frame.counts[2]);
            if jobs > 0 {
                pass.set_pipeline(&self.cards_lit);
                pass.dispatch_workgroups(jobs.div_ceil(64), 1, 1);
            }
            pass.set_pipeline(&self.passes[4]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        let bloom_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom"),
            layout: &self.bloom_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.pix.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.outp.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: self.bloom_a.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: self.bloom_b.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: self.glow_a.buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 17,
                    resource: self.glow_b.buf.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &bloom_group, &[]);
            let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
            if frame.bloom[2] > 0.0 {
                let [seed, across, down] = &self.glow_passes;
                pass.set_pipeline(seed);
                pass.dispatch_workgroups(gx, gy, 1);
                for _ in 0..2 {
                    pass.set_pipeline(across);
                    pass.dispatch_workgroups(gx, gy, 1);
                    pass.set_pipeline(down);
                    pass.dispatch_workgroups(gx, gy, 1);
                }
            }
            if frame.bloom[0] > 0.0 {
                let (hx, hy) = (w.div_ceil(2).div_ceil(8), h.div_ceil(2).div_ceil(8));
                let [down, across, down_blur, _] = &self.bloom_passes;
                pass.set_pipeline(down);
                pass.dispatch_workgroups(hx, hy, 1);
                for _ in 0..2 {
                    pass.set_pipeline(across);
                    pass.dispatch_workgroups(hx, hy, 1);
                    pass.set_pipeline(down_blur);
                    pass.dispatch_workgroups(hx, hy, 1);
                }
            }
            pass.set_pipeline(&self.bloom_passes[3]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        enc.copy_buffer_to_buffer(
            &self.outp.buf,
            0,
            &self.readback.buf,
            0,
            (n * WORDS * 4) as u64,
        );
        queue.submit([enc.finish()]);
        if let Some(e) = pollster::block_on(device.pop_error_scope()) {
            eprintln!("[goldlink-light] GPU error, lighting off: {e}");
            self.failed.store(true, Ordering::Relaxed);
            return false;
        }

        let slice = self.readback.buf.slice(..(n * WORDS * 4) as u64);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::Maintain::Wait);
        if !matches!(rx.recv(), Ok(Ok(()))) || self.failed() {
            return false;
        }
        {
            let data = slice.get_mapped_range();
            let words: &[u32] = bytemuck::cast_slice(&data);
            lights.clear();
            for (k, p) in words.chunks_exact(WORDS).take(n).enumerate() {
                out[k] = p[0];
                lights.push([p[1], p[2], p[3], p[4], p[5], p[6]]);
            }
        }
        self.readback.buf.unmap();
        true
    }
}

/// `tris` as corners on a map `side` texels across (columns and rows from its top left, depth
/// normalised over them for the test; the CPU's value kept for the lookups).
fn push_tris(out: &mut Vec<MapVert>, tris: &[Tri], side: [f32; 2]) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for t in tris {
        for m in &t.m {
            lo = lo.min(m[2]);
            hi = hi.max(m[2]);
        }
    }
    let span = if hi > lo { hi - lo } else { 1.0 };
    for t in tris {
        let tex = t
            .tex
            .map_or([0; 4], |x| [x.offset, x.width, x.height, t.param]);
        for k in 0..3 {
            let m = t.m[k];
            out.push(MapVert {
                pos: [
                    m[0] / side[0] * 2.0 - 1.0,
                    1.0 - m[1] / side[1] * 2.0,
                    ((m[2] - lo) / span).clamp(0.0, 1.0),
                    1.0,
                ],
                raw: m[2],
                st: t.st[k],
                tex,
                card: t.card,
            });
        }
    }
}
