//! The player's picture put together with the world's on the window's GPU (UC14,
//! `composite.wgsl`): melonDS's `Composite` per screen pixel round the player, each source lit
//! as itself. Where it can't run (the pipeline isn't made on this GPU) the window does it on the
//! CPU as before (`AttachedData::image`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use eframe::egui_wgpu;

/// Whether the GPU's composite is ready on the window's device (else the CPU's).
static READY: AtomicBool = AtomicBool::new(false);

pub fn ready() -> bool {
    READY.load(Ordering::Relaxed)
}

/// The planes `composite.wgsl` reads, in its order.
pub const PLANES: usize = 20;

/// A frame's crop of the two pictures (and the lights, the plain mask) round the player, as the
/// shader reads them: each plane `crop[2]` x `crop[3]` cells (the f32 `b` planes three words a
/// cell, the world's light four: its light, the bloom, the glow; the player's two), one after
/// the other.
pub struct Frame {
    pub seq: u64,
    pub words: Vec<u32>,
    pub offsets: [u32; PLANES],
    /// x0, y0, width, height (the arrays' columns and rows).
    pub crop: [i32; 4],
    pub disp_cnt: u32,
    pub edges: [u16; 8],
    pub left: i32,
    pub right: i32,
    pub render_width: u32,
    pub lit: bool,
    pub grade: [f32; 12],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    map: [f32; 4],
    off: [f32; 4],
    dims: [i32; 4],
    crop: [i32; 4],
    misc: [u32; 4],
    edges: [u32; 4],
    offsets: [[u32; 4]; 5],
    tint: [f32; 4],
    shadow: [f32; 4],
    high: [f32; 4],
}

struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    planes: wgpu::Buffer,
    planes_size: u64,
    group: wgpu::BindGroup,
    seq: Option<u64>,
    srgb: bool,
}

/// Makes the pipeline on the window's device, once; READY says whether it worked.
pub fn install(rs: &egui_wgpu::RenderState) {
    static TRIED: AtomicBool = AtomicBool::new(false);
    if TRIED.swap(true, Ordering::Relaxed) {
        return;
    }
    let device = &rs.device;
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let src = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("composite"),
        source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("composite"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
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
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("composite"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("composite"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            }),
        ),
        vertex: wgpu::VertexState {
            module: &src,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &src,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(rs.target_format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("composite"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let planes_size = 1 << 16;
    let planes = planes_buffer(device, planes_size);
    let group = bind_group(device, &layout, &uniforms, &planes);
    if let Some(e) = pollster::block_on(device.pop_error_scope()) {
        eprintln!("[goldlink] the player's picture is put together on the CPU: {e}");
        return;
    }
    rs.renderer.write().callback_resources.insert(Resources {
        pipeline,
        layout,
        uniforms,
        planes,
        planes_size,
        group,
        seq: None,
        srgb: rs.target_format.is_srgb(),
    });
    READY.store(true, Ordering::Relaxed);
}

fn planes_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("composite planes"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    planes: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: planes.as_entire_binding(),
            },
        ],
    })
}

/// One draw: the frame's crop, and where this window frame puts it (`map`: the world's point of a
/// screen pixel; `off`: the attached picture's point less the world's; `w`: the shown picture's
/// width; `o`: the plain mask's columns' offset; `from`: the arrays' columns' offset).
pub struct Draw {
    pub frame: Arc<Frame>,
    pub map: [f32; 4],
    pub off: [f32; 2],
    pub w: i32,
    pub o: i32,
    pub from: i32,
}

impl Draw {
    /// The paint callback over `rect` (points).
    pub fn callback(self, rect: egui::Rect) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(rect, self)
    }
}

impl egui_wgpu::CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        let f = &self.frame;
        if r.seq != Some(f.seq) {
            let bytes: &[u8] = bytemuck::cast_slice(&f.words);
            let need = (bytes.len() as u64).max(4).next_multiple_of(4);
            if need > r.planes_size {
                r.planes_size = need.next_power_of_two();
                r.planes = planes_buffer(device, r.planes_size);
                r.group = bind_group(device, &r.layout, &r.uniforms, &r.planes);
            }
            queue.write_buffer(&r.planes, 0, bytes);
            r.seq = Some(f.seq);
        }
        let mut offsets = [[0u32; 4]; 5];
        for (k, &o) in f.offsets.iter().enumerate() {
            offsets[k / 4][k % 4] = o;
        }
        let e = f.edges;
        let u = Uniforms {
            map: self.map,
            off: [
                self.off[0],
                self.off[1],
                f32::from(u8::from(f.lit)),
                f32::from(u8::from(r.srgb)),
            ],
            dims: [
                self.w,
                self.o,
                self.from,
                (f.left.max(0) << 16) | (f.right.clamp(0, 0xFFFF)),
            ],
            crop: f.crop,
            misc: [f.disp_cnt, f.render_width, 0, 0],
            edges: [
                u32::from(e[0]) | (u32::from(e[1]) << 16),
                u32::from(e[2]) | (u32::from(e[3]) << 16),
                u32::from(e[4]) | (u32::from(e[5]) << 16),
                u32::from(e[6]) | (u32::from(e[7]) << 16),
            ],
            offsets,
            tint: [f.grade[0], f.grade[1], f.grade[2], f.grade[3]],
            shadow: [f.grade[4], f.grade[5], f.grade[6], f.grade[7]],
            high: [f.grade[8], f.grade[9], f.grade[10], f.grade[11]],
        };
        queue.write_buffer(&r.uniforms, 0, bytemuck::bytes_of(&u));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = resources.get::<Resources>() else {
            return;
        };
        pass.set_pipeline(&r.pipeline);
        pass.set_bind_group(0, &r.group, &[]);
        pass.draw(0..3, 0..1);
    }
}
