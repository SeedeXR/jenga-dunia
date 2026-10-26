//! The 3-D viewport: the selected layer's preview as a sun-lit height-ramp mesh, rendered by
//! wgpu into Studio's own colour and depth targets (4× MSAA when the adapter allows it) and shown
//! in an egui panel as a native texture (ADR-0017; `docs/research/option-b-stack.md` §1 B).
//! Drag to orbit, right-drag to pan, scroll to zoom, double-click to fit.

use eframe::egui::{self, Align2, Color32, FontId, PointerButton, Rect, Sense, TextureId};
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
    /// Height in 0..1 over the layer's range, for the colour ramp.
    h: f32,
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
    @location(1) h: f32,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) h: f32) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.normal = normal;
    o.h = h;
    return o;
}

// Hypsometric ramp in linear colour: lowland green, upland olive, ochre, rock grey, snow.
fn ramp(h: f32) -> vec3<f32> {
    let c0 = vec3<f32>(0.05, 0.16, 0.06);
    let c1 = vec3<f32>(0.26, 0.30, 0.10);
    let c2 = vec3<f32>(0.36, 0.25, 0.14);
    let c3 = vec3<f32>(0.40, 0.38, 0.36);
    let c4 = vec3<f32>(0.85, 0.87, 0.90);
    if (h < 0.3) { return mix(c0, c1, h / 0.3); }
    if (h < 0.6) { return mix(c1, c2, (h - 0.3) / 0.3); }
    if (h < 0.85) { return mix(c2, c3, (h - 0.6) / 0.25); }
    return mix(c3, c4, (h - 0.85) / 0.15);
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(i.normal);
    // Steep ground shows rock whatever its height.
    let base = mix(ramp(clamp(i.h, 0.0, 1.0)), vec3<f32>(0.30, 0.28, 0.26), smoothstep(0.25, 0.55, 1.0 - n.y));
    let sun = max(dot(n, normalize(u.sun.xyz)), 0.0);
    let sky = 0.25 + 0.15 * n.y;
    return vec4<f32>(base * (sky + 0.9 * sun), 1.0);
}
";

/// Orbit camera around a target point (metres, y up, north = −z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Rotation about the vertical axis (radians).
    pub yaw: f32,
    /// Elevation above the horizon (radians).
    pub pitch: f32,
    /// Distance from the target (metres).
    pub distance: f32,
    /// The point orbited.
    pub target: [f32; 3],
}

impl Camera {
    /// A three-quarter view that frames a terrain of this extent and relief.
    pub fn fit(extent_m: [f32; 2], relief_m: f32) -> Self {
        Self {
            yaw: 0.7,
            pitch: 0.55,
            distance: 1.6 * extent_m[0].max(extent_m[1]).max(relief_m).max(1.0),
            target: [0.0, relief_m * 0.2, 0.0],
        }
    }

    fn eye(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let t = self.target;
        [
            t[0] + self.distance * cp * sy,
            t[1] + self.distance * sp,
            t[2] + self.distance * cp * cy,
        ]
    }

    fn view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let near = self.distance * 0.01;
        let far = self.distance * 20.0;
        mul(
            perspective(45f32.to_radians(), aspect, near, far),
            look_at(self.eye(), self.target),
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
            camera: Camera::fit([1.0, 1.0], 0.0),
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
        let at = |x: u32, y: u32| {
            let i = (y.min(pv.height - 1) * pv.width + x.min(pv.width - 1)) as usize;
            pv.min + f32::from(pv.data[i]) / 65535.0 * (pv.max - pv.min)
        };
        let relief = (pv.max - pv.min).max(1e-3);
        let (dx, dz) = (extent_m[0] / (w - 1) as f32, extent_m[1] / (h - 1) as f32);
        let mut vertices = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let hh = at(x, y);
                // Central differences (one-sided at the border) for the normal.
                let (l, r) = (at(x.saturating_sub(1), y), at(x + 1, y));
                let (n, s) = (at(x, y.saturating_sub(1)), at(x, y + 1));
                let nx = (l - r) / (dx * (x.min(w - 2) + 1 - x.saturating_sub(1)) as f32);
                let nz = (n - s) / (dz * (y.min(h - 2) + 1 - y.saturating_sub(1)) as f32);
                vertices.push(Vertex {
                    pos: [
                        x as f32 * dx - extent_m[0] / 2.0,
                        hh - pv.min,
                        y as f32 * dz - extent_m[1] / 2.0,
                    ],
                    normal: normalize([nx, 1.0, nz]),
                    h: (hh - pv.min) / relief,
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
            self.camera = Camera::fit(extent_m, pv.max - pv.min);
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
        painter.rect_filled(rect, 0.0, color::BG0);
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
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(t.tex_id, rect, uv, Color32::WHITE);
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

    fn handle_input(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let d = resp.drag_delta();
        if resp.dragged_by(PointerButton::Primary) && d != egui::Vec2::ZERO {
            self.camera.yaw -= d.x * 0.008;
            self.camera.pitch = (self.camera.pitch + d.y * 0.008).clamp(0.05, 1.5);
            self.dirty = true;
        }
        if resp.dragged_by(PointerButton::Secondary) && d != egui::Vec2::ZERO {
            // Pan on the ground plane, scaled so the ground follows the pointer roughly.
            let k = self.camera.distance * 0.0015;
            let (sy, cy) = self.camera.yaw.sin_cos();
            self.camera.target[0] -= (cy * d.x - sy * d.y) * k;
            self.camera.target[2] -= (-sy * d.x - cy * d.y) * k;
            self.dirty = true;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.distance = (self.camera.distance * (-scroll * 0.002).exp()).max(1.0);
                self.dirty = true;
            }
        }
        if resp.double_clicked()
            && let Some(i) = &self.info
        {
            self.camera = Camera::fit(i.extent_m, i.range_m[1] - i.range_m[0]);
            self.dirty = true;
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
    let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32];
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
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
    fn the_camera_target_projects_to_the_screen_centre_inside_the_depth_range() {
        let cam = Camera::fit([4096.0, 4096.0], 400.0);
        let p = apply(cam.view_proj(1.5), cam.target);
        assert!(p[0].abs() < 1e-4 && p[1].abs() < 1e-4, "{p:?}");
        assert!(p[2] > 0.0 && p[2] < 1.0, "{p:?}");
        // Above the target appears above the centre (y up on screen).
        let up = apply(cam.view_proj(1.5), [0.0, cam.target[1] + 100.0, 0.0]);
        assert!(up[1] > 0.0, "{up:?}");
    }

    #[test]
    fn a_preview_becomes_a_grid_mesh_with_upward_normals_on_flat_ground() {
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
        assert_eq!(verts[0].pos, [-100.0, 0.0, -50.0]);
        assert_eq!(verts[5].pos, [100.0, 10.0, 50.0]);
        assert_eq!((verts[0].h, verts[5].h), (0.0, 1.0));
        assert_eq!(verts[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(v.info().expect("info").triangles, 4);
    }
}
