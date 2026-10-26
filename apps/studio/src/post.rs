//! The viewport's post effects, ported from the desktop's three.js pipeline (`TerrainView.tsx`
//! post effect over three r186's `BloomNode` and ACES Filmic tone mapping): the scene renders into
//! a half-float HDR target; a final pass adds the bloom, darkens the corners and tone-maps into
//! the sRGB target. With every effect at its default the final pass only clamps, as three does
//! without tone mapping.
//!
//! Bloom, as `BloomNode`: a luminosity high pass at half resolution (`smoothstep(t, t + 0.01)`),
//! five mips each blurred horizontally then vertically (kernels 6, 10, 14, 18, 22, σ = r / 3,
//! adjacent taps merged into bilinear fetches), composited with factors 1.0 … 0.2 mixed towards
//! `1.2 − f` by the radius, times the strength.

use eframe::egui_wgpu::{RenderState, wgpu};

/// The HDR scene format.
pub const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MIPS: usize = 5;
const KERNELS: [usize; MIPS] = [6, 10, 14, 18, 22];
/// The desktop's bloom radius and threshold (`bloom(base, strength, 0.4, 0.85)`).
const RADIUS: f32 = 0.4;
const THRESHOLD: f32 = 0.85;

/// The Sun card's post settings (viewport only; exports untouched).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Post {
    /// Tone-mapping exposure (ACES while any effect is on); 1 = the plain render.
    pub exposure: f32,
    /// Bloom strength; 0 = off.
    pub bloom: f32,
    /// Corner darkening; 0 = off.
    pub vignette: f32,
}

impl Default for Post {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            bloom: 0.0,
            vignette: 0.0,
        }
    }
}

impl Post {
    /// Any effect on (the desktop switches to its pipeline and ACES then).
    pub fn on(&self) -> bool {
        self.exposure != 1.0 || self.bloom > 0.0 || self.vignette > 0.0
    }

    /// The desktop's readout part: `bloom 0.3 · vignette 0.2 · exposure 1.5`.
    pub fn info(&self) -> Option<String> {
        let mut parts = Vec::new();
        let n = |v: f32| {
            let r = (f64::from(v) * 100.0).round() / 100.0;
            if r.fract() == 0.0 {
                format!("{r:.0}")
            } else {
                r.to_string()
            }
        };
        if self.bloom > 0.0 {
            parts.push(format!("bloom {}", n(self.bloom)));
        }
        if self.vignette > 0.0 {
            parts.push(format!("vignette {}", n(self.vignette)));
        }
        if self.exposure != 1.0 {
            parts.push(format!("exposure {}", n(self.exposure)));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// `BloomNode._getSeparableBlurMaterial`'s kernel: the centre weight, then merged offsets and
/// weights.
pub fn kernel(radius: usize) -> (f32, Vec<f32>, Vec<f32>) {
    let sigma = radius as f32 / 3.0;
    let c: Vec<f32> = (0..radius)
        .map(|i| 0.39894 * (-0.5 * (i * i) as f32 / (sigma * sigma)).exp() / sigma)
        .collect();
    let (mut offsets, mut weights) = (Vec::new(), Vec::new());
    let mut i = 1;
    while i < radius {
        let wa = c[i];
        let wb = if i + 1 < radius { c[i + 1] } else { 0.0 };
        let w = wa + wb;
        offsets.push((i as f32 * wa + (i + 1) as f32 * wb) / w);
        weights.push(w);
        i += 2;
    }
    (c[0], offsets, weights)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PassUniforms {
    /// Blur: direction (xy), 1 / size (zw).
    dir_inv: [f32; 4],
    /// x: blur centre weight, y: tap count, z: strength, w: radius.
    params: [f32; 4],
    /// x: exposure, y: bloom on, z: vignette, w: tone mapping on.
    final_: [f32; 4],
    /// x: threshold.
    bright: [f32; 4],
    offsets: [[f32; 4]; 3],
    weights: [[f32; 4]; 3],
}

const SHADER: &str = r"
struct P {
    dir_inv: vec4<f32>, params: vec4<f32>, final_: vec4<f32>, bright: vec4<f32>,
    offsets: array<vec4<f32>, 3>, weights: array<vec4<f32>, 3>,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var t0: texture_2d<f32>;
@group(0) @binding(3) var t1: texture_2d<f32>;
@group(0) @binding(4) var t2: texture_2d<f32>;
@group(0) @binding(5) var t3: texture_2d<f32>;
@group(0) @binding(6) var t4: texture_2d<f32>;

struct V { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> V {
    let q = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: V;
    o.pos = vec4<f32>(q * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(q.x, 1.0 - q.y);
    return o;
}

fn tap(i: u32) -> vec2<f32> {
    return vec2<f32>(p.offsets[i / 4u][i % 4u], p.weights[i / 4u][i % 4u]);
}

@fragment
fn bright(v: V) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(t0, samp, v.uv, 0.0);
    let l = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec4<f32>(0.0), c, smoothstep(p.bright.x, p.bright.x + 0.01, l));
}

@fragment
fn blur(v: V) -> @location(0) vec4<f32> {
    var sum = textureSampleLevel(t0, samp, v.uv, 0.0).rgb * p.params.x;
    for (var i = 0u; i < u32(p.params.y); i++) {
        let t = tap(i);
        let off = p.dir_inv.xy * p.dir_inv.zw * t.x;
        sum += (textureSampleLevel(t0, samp, v.uv + off, 0.0).rgb + textureSampleLevel(t0, samp, v.uv - off, 0.0).rgb) * t.y;
    }
    return vec4<f32>(sum, 1.0);
}

fn factor(f: f32) -> f32 {
    return mix(f, 1.2 - f, p.params.w);
}

@fragment
fn composite(v: V) -> @location(0) vec4<f32> {
    let s = factor(1.0) * textureSampleLevel(t0, samp, v.uv, 0.0)
        + factor(0.8) * textureSampleLevel(t1, samp, v.uv, 0.0)
        + factor(0.6) * textureSampleLevel(t2, samp, v.uv, 0.0)
        + factor(0.4) * textureSampleLevel(t3, samp, v.uv, 0.0)
        + factor(0.2) * textureSampleLevel(t4, samp, v.uv, 0.0);
    return s * p.params.z;
}

// three's ACESFilmicToneMapping (Stephen Hill's fit, exposure / 0.6).
fn aces(c_in: vec3<f32>, exposure: f32) -> vec3<f32> {
    let input = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    let output = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    var c = input * (c_in * exposure / 0.6);
    let a = c * (c + 0.0245786) - 0.000090537;
    let b = c * (0.983729 * c + 0.4329510) + 0.238081;
    return clamp(output * (a / b), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn final_pass(v: V) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(t0, samp, v.uv, 0.0).rgb;
    if (p.final_.y > 0.5) {
        c += textureSampleLevel(t1, samp, v.uv, 0.0).rgb;
    }
    if (p.final_.z > 0.0) {
        c *= 1.0 - smoothstep(0.35, 0.9, length(v.uv - vec2<f32>(0.5))) * p.final_.z;
    }
    if (p.final_.w > 0.5) {
        return vec4<f32>(aces(c, p.final_.x), 1.0);
    }
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
";

/// The post pipelines (made once per device).
pub struct PostGpu {
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    final_: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// A 1 × 1 texture for unused slots.
    blank: wgpu::TextureView,
}

/// One pass: its uniforms and bind group.
struct Pass {
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
}

/// The size-dependent targets and the passes reading them.
pub struct PostTargets {
    bright_view: wgpu::TextureView,
    h: Vec<wgpu::TextureView>,
    v: Vec<wgpu::TextureView>,
    bright: Pass,
    blurs: Vec<(Pass, Pass)>,
    composite: Pass,
    final_: Pass,
}

impl PostGpu {
    /// Pipelines for writing the final image into `out` (the sRGB view's format).
    pub fn new(rs: &RenderState, out: wgpu::TextureFormat) -> Self {
        let device = &rs.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(2),
                tex(3),
                tex(4),
                tex(5),
                tex(6),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("post blank"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            bright: pipeline("bright", HDR),
            blur: pipeline("blur", HDR),
            composite: pipeline("composite", HDR),
            final_: pipeline("final_pass", out),
            layout,
            sampler,
            blank,
        }
    }

    fn pass(&self, rs: &RenderState, inputs: &[&wgpu::TextureView]) -> Pass {
        let uniforms = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post pass"),
            size: std::mem::size_of::<PassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        for k in 0..5 {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + k as u32,
                resource: wgpu::BindingResource::TextureView(
                    inputs.get(k).copied().unwrap_or(&self.blank),
                ),
            });
        }
        let group = rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post pass"),
            layout: &self.layout,
            entries: &entries,
        });
        Pass { uniforms, group }
    }

    /// Targets for an image of `size` whose HDR scene is `hdr`.
    pub fn targets(
        &self,
        rs: &RenderState,
        size: [u32; 2],
        hdr: &wgpu::TextureView,
    ) -> PostTargets {
        let target = |w: u32, h: u32| {
            rs.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("bloom"),
                    size: wgpu::Extent3d {
                        width: w.max(1),
                        height: h.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        // BloomNode: half resolution, each mip half the last.
        let (mut w, mut h) = (size[0] / 2, size[1] / 2);
        let bright_view = target(w, h);
        let (mut hs, mut vs, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..MIPS {
            hs.push(target(w, h));
            vs.push(target(w, h));
            sizes.push([w.max(1), h.max(1)]);
            w /= 2;
            h /= 2;
        }
        let bright = self.pass(rs, &[hdr]);
        let blurs = (0..MIPS)
            .map(|i| {
                let input = if i == 0 { &bright_view } else { &vs[i - 1] };
                (self.pass(rs, &[input]), self.pass(rs, &[&hs[i]]))
            })
            .collect();
        let composite = self.pass(rs, &[&vs[0], &vs[1], &vs[2], &vs[3], &vs[4]]);
        let final_ = self.pass(rs, &[hdr, &hs[0]]);
        let t = PostTargets {
            bright_view,
            h: hs,
            v: vs,
            bright,
            blurs,
            composite,
            final_,
        };
        // The blur passes' fixed uniforms: direction, texel size, kernel.
        for (i, (hp, vp)) in t.blurs.iter().enumerate() {
            let (centre, offsets, weights) = kernel(KERNELS[i]);
            let [w, h] = sizes[i];
            let mut u = PassUniforms {
                dir_inv: [1.0, 0.0, 1.0 / w as f32, 1.0 / h as f32],
                params: [centre, offsets.len() as f32, 0.0, 0.0],
                final_: [0.0; 4],
                bright: [0.0; 4],
                offsets: [[0.0; 4]; 3],
                weights: [[0.0; 4]; 3],
            };
            for (k, (o, wt)) in offsets.iter().zip(&weights).enumerate() {
                u.offsets[k / 4][k % 4] = *o;
                u.weights[k / 4][k % 4] = *wt;
            }
            rs.queue
                .write_buffer(&hp.uniforms, 0, bytemuck::bytes_of(&u));
            u.dir_inv[0] = 0.0;
            u.dir_inv[1] = 1.0;
            rs.queue
                .write_buffer(&vp.uniforms, 0, bytemuck::bytes_of(&u));
        }
        t
    }

    /// Encode the post passes: the HDR scene (already resolved) to `out`.
    pub fn encode(
        &self,
        rs: &RenderState,
        encoder: &mut wgpu::CommandEncoder,
        t: &PostTargets,
        post: Post,
        out: &wgpu::TextureView,
    ) {
        let zero = PassUniforms {
            dir_inv: [0.0; 4],
            params: [0.0; 4],
            final_: [0.0; 4],
            bright: [THRESHOLD, 0.0, 0.0, 0.0],
            offsets: [[0.0; 4]; 3],
            weights: [[0.0; 4]; 3],
        };
        let run = |encoder: &mut wgpu::CommandEncoder,
                   pipeline: &wgpu::RenderPipeline,
                   pass: &Pass,
                   view: &wgpu::TextureView| {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, &pass.group, &[]);
            rp.draw(0..3, 0..1);
        };
        let bloom = post.bloom > 0.0;
        if bloom {
            rs.queue
                .write_buffer(&t.bright.uniforms, 0, bytemuck::bytes_of(&zero));
            run(encoder, &self.bright, &t.bright, &t.bright_view);
            for (i, (hp, vp)) in t.blurs.iter().enumerate() {
                run(encoder, &self.blur, hp, &t.h[i]);
                run(encoder, &self.blur, vp, &t.v[i]);
            }
            let c = PassUniforms {
                params: [0.0, 0.0, post.bloom, RADIUS],
                ..zero
            };
            rs.queue
                .write_buffer(&t.composite.uniforms, 0, bytemuck::bytes_of(&c));
            run(encoder, &self.composite, &t.composite, &t.h[0]);
        }
        let f = PassUniforms {
            final_: [
                post.exposure,
                f32::from(u8::from(bloom)),
                post.vignette,
                f32::from(u8::from(post.on())),
            ],
            ..zero
        };
        rs.queue
            .write_buffer(&t.final_.uniforms, 0, bytemuck::bytes_of(&f));
        run(encoder, &self.final_, &t.final_, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blur_kernel_is_three_s() {
        // Kernel 6: σ = 2, merged taps at (1, 2), (3, 4), (5).
        let (c, o, w) = kernel(6);
        assert!((c - 0.19947).abs() < 1e-4, "{c}");
        assert_eq!(o.len(), 3);
        assert!(
            (o[0] - 1.4073).abs() < 1e-3 && (o[2] - 5.0).abs() < 1e-6,
            "{o:?}"
        );
        // The weights sum to one over both sides (a normalised Gaussian).
        let total = c + 2.0 * w.iter().sum::<f32>();
        assert!((total - 1.0).abs() < 0.03, "{total}");
        assert_eq!(kernel(22).1.len(), 11);
    }

    #[test]
    fn the_readout_names_the_effects_like_the_desktop() {
        assert_eq!(Post::default().info(), None);
        assert!(!Post::default().on());
        let p = Post {
            exposure: 1.5,
            bloom: 0.3,
            vignette: 0.25,
        };
        assert_eq!(
            p.info().as_deref(),
            Some("bloom 0.3 · vignette 0.25 · exposure 1.5")
        );
        assert!(p.on());
    }
}
