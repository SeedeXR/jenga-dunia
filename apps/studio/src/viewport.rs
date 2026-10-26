//! The 3-D viewport: the selected layer's preview as a sun-lit height-ramp mesh, rendered by
//! wgpu into Studio's own colour and depth targets (4× MSAA when the adapter allows it) and shown
//! in an egui panel as a native texture (ADR-0017; `docs/research/option-b-stack.md` §1 B).
//! Drag to orbit, right-drag to pan, scroll to zoom, double-click to fit.

use eframe::egui::{self, Align2, FontId, PointerButton, Rect, Sense, TextureId};
use eframe::egui_wgpu::{RenderState, wgpu};
use jd_terrain::preview::Preview;

use crate::theme::color;

/// What egui samples (it expects gamma-encoded `Rgba8Unorm`).
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// What we render through: an sRGB view of the same texture, so the hardware gamma-encodes.
const COLOR_SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    /// Linear RGB: the height ramp blended to rock by slope, as the desktop's vertex colours.
    color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    /// Direction towards the sun (xyz), w unused.
    sun: [f32; 4],
}

const SHADER: &str = r"
struct Uniforms { view_proj: mat4x4<f32>, sun: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.normal = normal;
    o.color = color;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(i.normal);
    let sun = max(dot(n, normalize(u.sun.xyz)), 0.0);
    let sky = 0.25 + 0.15 * n.y;
    return vec4<f32>(i.color * (sky + 0.9 * sun), 1.0);
}
";

/// The desktop's height ramp (`TerrainView.tsx` `STOPS`, linear RGB) and rock colour.
const STOPS: [(f32, [f32; 3]); 5] = [
    (0.0, [0.16, 0.3, 0.2]),
    (0.25, [0.36, 0.48, 0.24]),
    (0.5, [0.62, 0.52, 0.34]),
    (0.75, [0.52, 0.5, 0.48]),
    (1.0, [0.95, 0.95, 0.97]),
];
const ROCK: [f32; 3] = [0.42, 0.4, 0.38];

/// `RAMP[q >> 8]`: the stop interpolation at one of 256 steps, exactly as the desktop builds it.
fn ramp(q: u16) -> [f32; 3] {
    let t = f32::from(q >> 8) / 255.0;
    let mut k = 0;
    while k < STOPS.len() - 2 && t > STOPS[k + 1].0 {
        k += 1;
    }
    let ((t0, c0), (t1, c1)) = (STOPS[k], STOPS[k + 1]);
    let f = (t - t0) / (t1 - t0).max(1e-6);
    [0, 1, 2].map(|i| c0[i] + (c1[i] - c0[i]) * f)
}

/// The desktop's orbit camera (`TerrainView.tsx`): around the origin, `theta` about the vertical
/// axis, `phi` from the zenith, fov 50°.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Rotation about the vertical axis (radians).
    pub theta: f32,
    /// Angle from straight up (radians, 0.15..1.5).
    pub phi: f32,
    /// Distance from the origin (metres).
    pub radius: f32,
    /// The world's larger side (metres): clip planes and zoom limits follow it.
    pub size: f32,
}

impl Camera {
    /// The home view for a world of this extent: θ 0.6, φ 1.0, radius 1.3 × the larger side.
    pub fn fit(extent_m: [f32; 2]) -> Self {
        let size = extent_m[0].max(extent_m[1]).max(1.0);
        Self {
            theta: 0.6,
            phi: 1.0,
            radius: size * 1.3,
            size,
        }
    }

    fn eye(&self) -> [f32; 3] {
        let (st, ct) = self.theta.sin_cos();
        let (sp, cp) = self.phi.sin_cos();
        [
            self.radius * sp * st,
            self.radius * cp,
            self.radius * sp * ct,
        ]
    }

    fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let near = (self.size * 0.001).max(0.1);
        let far = self.size * 50.0;
        mul(
            perspective(50f32.to_radians(), aspect, near, far),
            look_at(self.eye(), [0.0, 0.0, 0.0]),
        )
    }
}

// Column-major 4×4 maths for one camera: right-handed, depth 0..1 (wgpu).
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}
fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fovy / 2.0).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}
fn look_at(eye: [f32; 3], target: [f32; 3]) -> [[f32; 4]; 4] {
    let f = normalize(sub(target, eye));
    let s = normalize(cross(f, [0.0, 1.0, 0.0]));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}
fn mul(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut c = [[0.0; 4]; 4];
    for (j, col) in c.iter_mut().enumerate() {
        for (i, v) in col.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[k][i] * b[j][k]).sum();
        }
    }
    c
}

fn srgb_to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// What the viewport shows, for readouts and tests.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainInfo {
    /// Preview samples (width, height).
    pub size: [u32; 2],
    /// Ground extent (metres).
    pub extent_m: [f32; 2],
    /// Lowest and highest height (metres).
    pub range_m: [f32; 2],
    /// Triangles drawn.
    pub triangles: u64,
}

struct Gpu {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    samples: u32,
}

struct Targets {
    size: [u32; 2],
    /// The colour texture (what egui shows; Still reads it back).
    color: wgpu::Texture,
    /// The sRGB view we render (or resolve) into.
    target: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    tex_id: TextureId,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

/// The viewport's state; [`Viewport::show`] draws it into the remaining space of a `Ui`.
pub struct Viewport {
    gpu: Option<Gpu>,
    targets: Option<Targets>,
    mesh: Option<Mesh>,
    pending: Option<(Vec<Vertex>, Vec<u32>)>,
    info: Option<TerrainInfo>,
    /// The camera; set it and the next frame re-renders.
    pub camera: Camera,
    sun: [f32; 3],
    dirty: bool,
    rect: Rect,
    renders: u64,
}

impl Default for Viewport {
    fn default() -> Self {
        let mut v = Self {
            gpu: None,
            targets: None,
            mesh: None,
            pending: None,
            info: None,
            camera: Camera::fit([1.0, 1.0]),
            sun: [0.0, 1.0, 0.0],
            dirty: true,
            rect: Rect::NOTHING,
            renders: 0,
        };
        v.set_sun(140.0, 35.0);
        v
    }
}

impl Viewport {
    /// Show this preview of a layer covering `extent_m`. The camera re-frames when the ground's
    /// extent changes (a new project), not on every refresh.
    pub fn set_terrain(&mut self, pv: &Preview, extent_m: [f32; 2]) {
        let (w, h) = (pv.width.max(2), pv.height.max(2));
        let q = |x: u32, y: u32| {
            pv.data[(y.min(pv.height - 1) * pv.width + x.min(pv.width - 1)) as usize]
        };
        let at = |x: u32, y: u32| pv.min + f32::from(q(x, y)) / 65535.0 * (pv.max - pv.min);
        let (dx, dz) = (extent_m[0] / (w - 1) as f32, extent_m[1] / (h - 1) as f32);
        let mut vertices = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let height = at(x, y);
                // Central differences (one-sided at the border). The preview's row 0 is the south
                // edge (+z), as in the desktop, so z falls as the row index grows.
                let (l, r) = (at(x.saturating_sub(1), y), at(x + 1, y));
                let (south, north) = (at(x, y.saturating_sub(1)), at(x, y + 1));
                let nx = (l - r) / (dx * (x.min(w - 2) + 1 - x.saturating_sub(1)) as f32);
                let nz = (north - south) / (dz * (y.min(h - 2) + 1 - y.saturating_sub(1)) as f32);
                let normal = normalize([nx, 1.0, nz]);
                // Steep faces read as rock regardless of altitude (the desktop's slope blend).
                let slope = ((1.0 - normal[1]) * 1.4).clamp(0.0, 1.0);
                let c = ramp(q(x, y));
                vertices.push(Vertex {
                    pos: [
                        x as f32 * dx - extent_m[0] / 2.0,
                        height,
                        extent_m[1] / 2.0 - y as f32 * dz,
                    ],
                    normal,
                    color: [0, 1, 2].map(|i| c[i] + (ROCK[i] - c[i]) * slope),
                });
            }
        }
        let mut indices = Vec::with_capacity(((w - 1) * (h - 1) * 6) as usize);
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let i = y * w + x;
                indices.extend_from_slice(&[i, i + w, i + 1, i + 1, i + w, i + w + 1]);
            }
        }
        let refit = self.info.as_ref().is_none_or(|i| i.extent_m != extent_m);
        self.info = Some(TerrainInfo {
            size: [pv.width, pv.height],
            extent_m,
            range_m: [pv.min, pv.max],
            triangles: indices.len() as u64 / 3,
        });
        if refit {
            self.camera = Camera::fit(extent_m);
        }
        self.pending = Some((vertices, indices));
        self.dirty = true;
    }

    /// Drop the terrain (no project, or no layer).
    pub fn clear(&mut self) {
        self.mesh = None;
        self.pending = None;
        self.info = None;
        self.dirty = true;
    }

    /// Sun direction from azimuth (clockwise from north) and elevation, in degrees.
    pub fn set_sun(&mut self, azimuth_deg: f32, elevation_deg: f32) {
        let (az, el) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
        self.sun = [az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()];
        self.dirty = true;
    }

    /// The terrain shown, if any.
    pub fn info(&self) -> Option<&TerrainInfo> {
        self.info.as_ref()
    }

    /// Where the viewport was drawn last frame (points).
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// The last rendered frame as RGBA8 rows (Still): `(width, height, pixels)`.
    pub fn capture(&self, rs: &RenderState) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.targets.as_ref()?;
        self.mesh.as_ref()?;
        let [w, h] = t.size;
        let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("still"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("still"),
            });
        encoder.copy_texture_to_buffer(
            t.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let index = rs.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        rs.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let start = y * row as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        Some((w, h, out))
    }

    /// How many times the 3-D scene has been rendered (tests: a camera move re-renders).
    pub fn renders(&self) -> u64 {
        self.renders
    }

    /// Lay out the viewport in the remaining space, handle camera input, render if anything
    /// changed, and paint the result. Without a wgpu render state it shows a notice instead.
    pub fn show(&mut self, ui: &mut egui::Ui, rs: Option<&RenderState>) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.rect = rect;
        self.handle_input(ui, &resp);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 12.0, color::BG0);
        crate::look::ring(ui, rect);
        let Some(rs) = rs else {
            notice(
                &painter,
                rect,
                "The 3-D viewport needs a GPU (wgpu); none is available.",
            );
            return;
        };
        if self.gpu.is_none() {
            self.gpu = Some(create_gpu(rs));
        }
        if let Some((v, i)) = self.pending.take() {
            self.mesh = Some(upload(rs, &v, &i));
        }
        let ppp = ui.ctx().pixels_per_point();
        let size = [
            ((rect.width() * ppp).round() as u32).max(1),
            ((rect.height() * ppp).round() as u32).max(1),
        ];
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            let samples = self.gpu.as_ref().map_or(1, |g| g.samples);
            let old = self.targets.take().map(|t| t.tex_id);
            self.targets = Some(create_targets(rs, size, samples, old));
            self.dirty = true;
        }
        if self.mesh.is_none() {
            notice(
                &painter,
                rect,
                "No terrain yet: open a project, then generate terrain or evaluate the graph.",
            );
            return;
        }
        if self.dirty {
            self.render(rs, size);
            self.dirty = false;
        }
        if let Some(t) = &self.targets {
            egui::Image::from_texture(egui::load::SizedTexture::new(t.tex_id, rect.size()))
                .corner_radius(12.0)
                .paint_at(ui, rect);
            crate::look::ring(ui, rect);
        }
        if let Some(i) = &self.info {
            let text = format!(
                "{} x {} samples  |  {:.0} x {:.0} m  |  {:.0} to {:.0} m  |  {:.2} M triangles",
                i.size[0],
                i.size[1],
                i.extent_m[0],
                i.extent_m[1],
                i.range_m[0],
                i.range_m[1],
                i.triangles as f64 / 1e6
            );
            painter.text(
                rect.left_bottom() + egui::vec2(10.0, -10.0),
                Align2::LEFT_BOTTOM,
                text,
                FontId::proportional(12.0),
                color::FG2,
            );
        }
    }

    /// The desktop's controls: either button drags orbit (0.005 rad per point), the wheel zooms
    /// (radius × (1 + Δ · 0.001) within 0.02–20 × the world's size).
    fn handle_input(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let d = resp.drag_delta();
        if (resp.dragged_by(PointerButton::Primary) || resp.dragged_by(PointerButton::Secondary))
            && d != egui::Vec2::ZERO
        {
            self.camera.theta -= d.x * 0.005;
            self.camera.phi = (self.camera.phi - d.y * 0.005).clamp(0.15, 1.5);
            self.dirty = true;
        }
        if resp.hovered() {
            // egui's scroll delta is the browser's `deltaY` with the sign flipped.
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let size = self.camera.size;
                self.camera.radius =
                    (self.camera.radius * (1.0 - scroll * 0.001)).clamp(size * 0.02, size * 20.0);
                self.dirty = true;
            }
        }
    }

    fn render(&mut self, rs: &RenderState, size: [u32; 2]) {
        let (Some(gpu), Some(t), Some(mesh)) = (&self.gpu, &self.targets, &self.mesh) else {
            return;
        };
        let u = Uniforms {
            view_proj: self.camera.view_proj(size[0] as f32 / size[1] as f32),
            sun: [self.sun[0], self.sun[1], self.sun[2], 0.0],
        };
        rs.queue
            .write_buffer(&gpu.uniforms, 0, bytemuck::bytes_of(&u));
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewport"),
            });
        {
            let bg = color::BG0;
            let clear = wgpu::Color {
                r: srgb_to_linear(bg.r()),
                g: srgb_to_linear(bg.g()),
                b: srgb_to_linear(bg.b()),
                a: 1.0,
            };
            let (view, resolve) = match &t.msaa {
                Some(msaa) => (msaa, Some(&t.target)),
                None => (&t.target, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, 0..1);
        }
        rs.queue.submit([encoder.finish()]);
        self.renders += 1;
    }
}

fn notice(painter: &egui::Painter, rect: Rect, text: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        color::FG2,
    );
}

fn create_gpu(rs: &RenderState) -> Gpu {
    let device = &rs.device;
    let supports_4x = |f: wgpu::TextureFormat| {
        rs.adapter
            .get_texture_format_features(f)
            .flags
            .sample_count_supported(4)
    };
    let samples = if supports_4x(COLOR_SRGB) && supports_4x(DEPTH) {
        4
    } else {
        1
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport uniforms"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("terrain"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(COLOR_SRGB.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    Gpu {
        pipeline,
        uniforms,
        bind_group,
        samples,
    }
}

fn create_targets(
    rs: &RenderState,
    size: [u32; 2],
    samples: u32,
    reuse: Option<TextureId>,
) -> Targets {
    let device = &rs.device;
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let texture = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats,
        })
    };
    let color = texture(
        "viewport colour",
        COLOR,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        &[COLOR_SRGB],
    );
    let view_as = |format| {
        color.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        })
    };
    let (unorm, target) = (view_as(COLOR), view_as(COLOR_SRGB));
    let msaa = (samples > 1).then(|| {
        texture(
            "viewport msaa",
            COLOR_SRGB,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        )
        .create_view(&Default::default())
    });
    let depth = texture(
        "viewport depth",
        DEPTH,
        samples,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
        &[],
    )
    .create_view(&Default::default());
    let mut renderer = rs.renderer.write();
    let tex_id = match reuse {
        Some(id) => {
            renderer.update_egui_texture_from_wgpu_texture(
                device,
                &unorm,
                wgpu::FilterMode::Linear,
                id,
            );
            id
        }
        None => renderer.register_native_texture(device, &unorm, wgpu::FilterMode::Linear),
    };
    Targets {
        size,
        color,
        target,
        msaa,
        depth,
        tex_id,
    }
}

fn upload(rs: &RenderState, vertices: &[Vertex], indices: &[u32]) -> Mesh {
    use wgpu::util::DeviceExt as _;
    let vertices = rs
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let count = indices.len() as u32;
    let indices = rs
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
    Mesh {
        vertices,
        indices,
        count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
        let v: Vec<f32> = (0..4)
            .map(|i| m[0][i] * p[0] + m[1][i] * p[1] + m[2][i] * p[2] + m[3][i])
            .collect();
        [v[0] / v[3], v[1] / v[3], v[2] / v[3]]
    }

    #[test]
    fn the_home_view_is_the_desktops_and_looks_at_the_origin() {
        let cam = Camera::fit([4096.0, 2048.0]);
        assert_eq!((cam.theta, cam.phi, cam.radius), (0.6, 1.0, 4096.0 * 1.3));
        let p = apply(cam.view_proj(1.5), [0.0, 0.0, 0.0]);
        assert!(p[0].abs() < 1e-4 && p[1].abs() < 1e-4, "{p:?}");
        assert!(p[2] > 0.0 && p[2] < 1.0, "{p:?}");
        let up = apply(cam.view_proj(1.5), [0.0, 100.0, 0.0]);
        assert!(up[1] > 0.0, "{up:?}");
    }

    #[test]
    fn the_ramp_matches_the_desktops_stops() {
        assert_eq!(ramp(0), [0.16, 0.3, 0.2]);
        assert_eq!(ramp(u16::MAX), [0.95, 0.95, 0.97]);
        // Index 127 of 255 (t = 0.498): just short of the 0.5 stop.
        let mid = ramp(127 << 8);
        assert!((mid[0] - 0.618).abs() < 0.01, "{mid:?}");
    }

    #[test]
    fn a_preview_becomes_the_desktops_mesh() {
        let pv = Preview {
            width: 3,
            height: 2,
            min: 10.0,
            max: 20.0,
            data: vec![0, 0, 0, 0, 0, 65535],
        };
        let mut v = Viewport::default();
        v.set_terrain(&pv, [200.0, 100.0]);
        let (verts, idx) = v.pending.as_ref().expect("mesh");
        assert_eq!((verts.len(), idx.len()), (6, 2 * 2 * 3));
        // Row 0 is the south edge (+z); heights are absolute.
        assert_eq!(verts[0].pos, [-100.0, 10.0, 50.0]);
        assert_eq!(verts[5].pos, [100.0, 20.0, -50.0]);
        assert_eq!(verts[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(verts[0].color, [0.16, 0.3, 0.2]);
        // Normals lean away from higher ground: west of the raised corner (vertex 4) towards −x,
        // south of it (vertex 2, the ground rising northwards) towards +z.
        assert!(verts[4].normal[0] < 0.0, "{:?}", verts[4].normal);
        assert!(verts[2].normal[2] > 0.0, "{:?}", verts[2].normal);
        assert_eq!(v.info().expect("info").triangles, 4);
    }
}
