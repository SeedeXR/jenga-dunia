//! The 3-D viewport, a port of the desktop's three.js scene (`apps/desktop/src/viewport/
//! TerrainView.tsx`): the layer's preview as a mesh with the height ramp and slope rock (or the
//! baked PBR maps), lit like three's `MeshStandardMaterial` by the sun and the hemisphere light,
//! with distance fog and the sky dome (the physical-sky preview, or the gradient), rendered by
//! wgpu into Studio's own colour and depth targets (4× MSAA when supported) and shown in an egui
//! panel as a native texture. Either button drags orbit, the wheel zooms.

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
    /// Map coordinates: u east, v north from the south edge (the baked maps' row 0).
    uv: [f32; 2],
}

/// A coloured point of a line or of the sea (linear RGB and alpha).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorVertex {
    pos: [f32; 3],
    color: [f32; 4],
}

/// One Gerstner component of the wind sea (`ocean.waves`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wave {
    /// Amplitude (m).
    pub amplitude_m: f32,
    /// Wavelength (m).
    pub wavelength_m: f32,
    /// Travel direction, degrees clockwise from north.
    pub direction_deg: f32,
    /// Phase speed (m/s).
    pub speed_mps: f32,
    /// Phase offset (rad).
    pub phase: f32,
}

/// The sea as the desktop draws it (the graph's sea-level node and its wind sea).
#[derive(Debug, Clone, PartialEq)]
pub struct Sea {
    /// Water level (m).
    pub level_m: f32,
    /// Depth at which the bottom fades out (m).
    pub visibility_m: f32,
    /// Colour of the suspended matter (linear RGB).
    pub tint: [f32; 3],
    /// Depth above which the wind sea breaks: the surf zone's foam (m).
    pub breaking_depth_m: f32,
    /// The wind sea's components (empty: calm).
    pub waves: Vec<Wave>,
    /// Significant wave height and peak wavelength (m), for the readout.
    pub waves_m: Option<[f32; 2]>,
}

/// The sea grid (vertices, indices) and its shoreline segments.
type SeaGeometry = (Vec<ColorVertex>, Vec<u32>, Vec<ColorVertex>);

/// Pure-water absorption per channel in m⁻¹ (Pope & Fry 1997, rounded): red dies first.
const PURE_WATER_ABSORPTION: [f32; 3] = [0.4, 0.07, 0.005];

/// The desktop's `waterColour`: the sandy bottom through Beer–Lambert transmittance (down and up),
/// plus the tint scattered back; murkier or deeper water is more opaque.
fn water_colour(depth_m: f32, sea: &Sea) -> ([f32; 3], f32) {
    let k = 15.0 / sea.visibility_m.max(0.5);
    let bottom = [0.62, 0.55, 0.38];
    let mut rgb = [0.0; 3];
    let mut seen = 0.0;
    for c in 0..3 {
        let t = (-PURE_WATER_ABSORPTION[c] * k * depth_m * 2.0).exp();
        rgb[c] = bottom[c] * t + sea.tint[c] * (1.0 - t);
        seen += t;
    }
    (rgb, 0.35 + 0.55 * (1.0 - seen / 3.0))
}

/// The desktop's `buildSeaGeometry`: a flat grid at the sea level over the world, coloured by the
/// depth of the terrain under it, a foam band near the shore, transparent over land.
fn sea_mesh(pv: &Preview, extent_m: [f32; 2], sea: &Sea) -> (Vec<ColorVertex>, Vec<u32>) {
    let (ground, indices) = grid_mesh(pv, extent_m);
    let foam_m = (0.5f32)
        .max((pv.max - pv.min) * 0.01)
        .max(sea.breaking_depth_m);
    let foam = hex(0xdff3f5);
    let vertices = ground
        .iter()
        .map(|g| {
            let depth = sea.level_m - g.pos[1];
            let (mut rgb, mut alpha) = water_colour(depth.max(0.0), sea);
            if depth <= 0.0 {
                alpha = 0.0;
            } else if depth < foam_m {
                let f = 1.0 - depth / foam_m;
                rgb = [0, 1, 2].map(|i| rgb[i] + (foam[i] - rgb[i]) * f);
                alpha = alpha.max(0.5 + 0.45 * f);
            }
            ColorVertex {
                pos: [g.pos[0], sea.level_m, g.pos[2]],
                color: [rgb[0], rgb[1], rgb[2], alpha],
            }
        })
        .collect();
    (vertices, indices)
}

/// The desktop's `buildShoreline`: where the terrain crosses the sea level, by marching squares,
/// white (0.85) just above the water.
fn shoreline(pv: &Preview, extent_m: [f32; 2], level_m: f32) -> Vec<ColorVertex> {
    let (ground, _) = grid_mesh(pv, extent_m);
    let (w, h) = (pv.width.max(2) as usize, pv.height.max(2) as usize);
    let y = level_m + (0.2f32).max((pv.max - pv.min) * 0.002);
    let white = [1.0, 1.0, 1.0, 0.85];
    let cross = |a: usize, b: usize| -> Option<[f32; 3]> {
        let (pa, pb) = (ground[a].pos, ground[b].pos);
        let (da, db) = (pa[1] - level_m, pb[1] - level_m);
        if (da < 0.0) == (db < 0.0) {
            return None;
        }
        let t = da / (da - db);
        Some([pa[0] + (pb[0] - pa[0]) * t, y, pa[2] + (pb[2] - pa[2]) * t])
    };
    let mut out = Vec::new();
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            let id = |i: usize, j: usize| j * w + i;
            let hits: Vec<[f32; 3]> = [
                cross(id(i, j), id(i + 1, j)),
                cross(id(i + 1, j), id(i + 1, j + 1)),
                cross(id(i, j + 1), id(i + 1, j + 1)),
                cross(id(i, j), id(i, j + 1)),
            ]
            .into_iter()
            .flatten()
            .collect();
            for pair in hits.chunks(2).filter(|p| p.len() == 2) {
                out.push(ColorVertex {
                    pos: pair[0],
                    color: white,
                });
                out.push(ColorVertex {
                    pos: pair[1],
                    color: white,
                });
            }
        }
    }
    out
}

/// Everything the shaders read, in WGSL's uniform layout (16-byte rows).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    /// Camera position (w unused).
    eye: [f32; 4],
    /// The view basis for the sky rays: right × tan(fov/2)·aspect, up × tan(fov/2), forward.
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    /// Towards the sun (xyz), w unused.
    sun_dir: [f32; 4],
    /// The sun's colour × intensity (linear).
    sun_light: [f32; 4],
    /// The hemisphere light's sky and ground colours × intensity (linear).
    hemi_sky: [f32; 4],
    hemi_ground: [f32; 4],
    /// Fog colour (rgb) and planes: x near, y far (view depth, metres).
    fog_color: [f32; 4],
    fog: [f32; 4],
    /// x: baked maps on, y: emissive map on, z: sky texture on, w: sky radius (metres).
    flags: [f32; 4],
    /// x: seconds, y: wave components, z: waves animate (0 / 1), w unused.
    time: [f32; 4],
    /// Up to three Gerstner components: [amplitude, wavelength, direction°, speed], [phase, …].
    waves: [[f32; 4]; 6],
}

const SHADER: &str = r"
struct U {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>, right: vec4<f32>, up: vec4<f32>, forward: vec4<f32>,
    sun_dir: vec4<f32>, sun_light: vec4<f32>, hemi_sky: vec4<f32>, hemi_ground: vec4<f32>,
    fog_color: vec4<f32>, fog: vec4<f32>, flags: vec4<f32>,
    time: vec4<f32>, waves: array<vec4<f32>, 6>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var albedo_map: texture_2d<f32>;
@group(0) @binding(3) var normal_map: texture_2d<f32>;
@group(0) @binding(4) var rough_map: texture_2d<f32>;
@group(0) @binding(5) var emissive_map: texture_2d<f32>;
@group(0) @binding(6) var sky_map: texture_2d<f32>;

const PI: f32 = 3.141592653589793;
const RECIPROCAL_PI: f32 = 0.3183098861837907;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @location(3) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    o.normal = normal;
    o.color = color;
    o.uv = uv;
    return o;
}

// three.js's physical BRDF (MeshStandardMaterial, metalness 0): Schlick Fresnel, height-correlated
// Smith visibility, GGX distribution, Lambert diffuse.
fn brdf_ggx(l: vec3<f32>, v: vec3<f32>, n: vec3<f32>, roughness: f32, f0: vec3<f32>) -> vec3<f32> {
    let alpha = roughness * roughness;
    let h = normalize(l + v);
    let dnl = clamp(dot(n, l), 0.0, 1.0);
    let dnv = clamp(dot(n, v), 0.0, 1.0);
    let dnh = clamp(dot(n, h), 0.0, 1.0);
    let dvh = clamp(dot(v, h), 0.0, 1.0);
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - dvh, 5.0);
    let a2 = alpha * alpha;
    let gv = dnl * sqrt(a2 + (1.0 - a2) * dnv * dnv);
    let gl = dnv * sqrt(a2 + (1.0 - a2) * dnl * dnl);
    let vis = 0.5 / max(gv + gl, 1e-6);
    let denom = dnh * dnh * (a2 - 1.0) + 1.0;
    let d = RECIPROCAL_PI * a2 / (denom * denom);
    return fresnel * (vis * d);
}

// three's PhysicalLightingModel for one surface: the sun (direct diffuse + GGX) and the
// hemisphere light (indirect diffuse), metalness mixing F0 towards the albedo.
fn shade(albedo: vec3<f32>, n: vec3<f32>, world: vec3<f32>, roughness_in: f32, metalness: f32, geo_n: vec3<f32>) -> vec3<f32> {
    // three: roughness ≥ 0.0525, plus the geometry's own roughness from the normal's derivatives.
    let dxy = max(abs(dpdx(geo_n)), abs(dpdy(geo_n)));
    let roughness = min(max(roughness_in, 0.0525) + max(max(dxy.x, dxy.y), dxy.z), 1.0);
    let v = normalize(u.eye.xyz - world);
    let l = normalize(u.sun_dir.xyz);
    let irradiance = clamp(dot(n, l), 0.0, 1.0) * u.sun_light.rgb;
    let diffuse_color = albedo * (1.0 - metalness);
    let f0 = mix(vec3<f32>(0.04), albedo, metalness);
    var c = irradiance * RECIPROCAL_PI * diffuse_color + irradiance * brdf_ggx(l, v, n, roughness, f0);
    let hemi = mix(u.hemi_ground.rgb, u.hemi_sky.rgb, 0.5 * n.y + 0.5);
    c += hemi * RECIPROCAL_PI * diffuse_color;
    return c;
}

fn fogged(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let depth = dot(world - u.eye.xyz, u.forward.xyz);
    return mix(c, u.fog_color.rgb, smoothstep(u.fog.x, u.fog.y, depth));
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let flat_n = normalize(i.normal);
    var albedo = i.color;
    var roughness = 0.95;
    var n = flat_n;
    var emissive = vec3<f32>(0.0);
    // The framed block's walls carry uv −1: their own colour, never the maps.
    if (u.flags.x > 0.5 && i.uv.x >= 0.0) {
        albedo = textureSample(albedo_map, samp, i.uv).rgb;
        roughness = textureSample(rough_map, samp, i.uv).g;
        // Tangent space: +x east (u), +y north (v), +z out of the surface (glTF), on the mesh
        // normal (three's normalMap on a plane).
        let m = textureSample(normal_map, samp, i.uv).xyz * 2.0 - 1.0;
        let t = normalize(vec3<f32>(1.0, 0.0, 0.0) - flat_n * flat_n.x);
        let b = normalize(vec3<f32>(0.0, 0.0, -1.0) + flat_n * flat_n.z);
        n = normalize(t * m.x + b * m.y + flat_n * m.z);
        if (u.flags.y > 0.5) {
            emissive = textureSample(emissive_map, samp, i.uv).rgb;
        }
    }
    let c = shade(albedo, n, i.world, roughness, 0.0, flat_n) + emissive;
    return vec4<f32>(fogged(c, i.world), 1.0);
}

// The framed block's rim: teal lines (0x3fd0c9), unlit, fogged like three's LineBasicMaterial.
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn line_vs(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var o: LineOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.world = pos;
    o.color = color;
    return o;
}

@fragment
fn line_fs(i: LineOut) -> @location(0) vec4<f32> {
    return vec4<f32>(fogged(i.color.rgb, i.world), i.color.a);
}

// The sea: the desktop's MeshStandardMaterial (roughness 0.2, metalness 0.05, vertex colour and
// alpha from depth), moved along Gerstner orbits when its waves animate.
struct SeaOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

@vertex
fn sea_vs(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> SeaOut {
    var p = pos;
    var dx_ = vec3<f32>(1.0, 0.0, 0.0);
    var dz_ = vec3<f32>(0.0, 0.0, 1.0);
    let n = max(u.time.y, 1.0);
    if (u.time.z > 0.5) {
        for (var k = 0; k < i32(u.time.y); k++) {
            let a = u.waves[2 * k];
            let amp = a.x;
            let wk = 2.0 * PI / a.y;
            let rad = radians(a.z);
            let dir = vec2<f32>(sin(rad), -cos(rad)); // north is −z
            let phi = wk * (dir.x * pos.x + dir.y * pos.z) - wk * a.w * u.time.x + u.waves[2 * k + 1].x;
            let q = min(1.0, 0.5 / (wk * amp * n)); // GPU Gems 1 ch. 1
            let c = cos(phi);
            let s = sin(phi);
            p += vec3<f32>(q * amp * dir.x * c, amp * s, q * amp * dir.y * c);
            // The surface's tangents along x and z, for the lit normal.
            dx_ += vec3<f32>(-q * amp * dir.x * dir.x * wk * s, amp * wk * dir.x * c, -q * amp * dir.x * dir.y * wk * s);
            dz_ += vec3<f32>(-q * amp * dir.x * dir.y * wk * s, amp * wk * dir.y * c, -q * amp * dir.y * dir.y * wk * s);
        }
    }
    var o: SeaOut;
    o.clip = u.view_proj * vec4<f32>(p, 1.0);
    o.world = p;
    o.normal = normalize(cross(dz_, dx_));
    o.color = color;
    return o;
}

@fragment
fn sea_fs(i: SeaOut) -> @location(0) vec4<f32> {
    let n = normalize(i.normal);
    let c = shade(i.color.rgb, n, i.world, 0.2, 0.05, n);
    return vec4<f32>(fogged(c, i.world), i.color.a);
}

// The sky: a full-screen triangle; each pixel's ray meets the dome (a sphere of radius flags.w
// around the origin, as the desktop's scaled sphere) and reads the equirectangular preview —
// row 0 the zenith, column 0 north, clockwise — or the gradient. No fog, behind everything.
struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn sky_vs(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0);
    var o: SkyOut;
    o.clip = vec4<f32>(p, 1.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn sky_fs(i: SkyOut) -> @location(0) vec4<f32> {
    let ray = normalize(u.forward.xyz + u.right.xyz * i.ndc.x + u.up.xyz * i.ndc.y);
    // Where the ray leaves the dome (the camera is inside it).
    let o = u.eye.xyz;
    let b = dot(o, ray);
    let c = dot(o, o) - u.flags.w * u.flags.w;
    let t = -b + sqrt(max(b * b - c, 0.0));
    let d = normalize(o + ray * t);
    if (u.flags.z > 0.5) {
        let azimuth = atan2(d.x, -d.z);
        let uv = vec2<f32>(fract(azimuth / (2.0 * PI) + 1.0), acos(clamp(d.y, -1.0, 1.0)) / PI);
        return vec4<f32>(textureSampleLevel(sky_map, samp, uv, 0.0).rgb, 1.0);
    }
    // The gradient dome: horizon → zenith above, quickly to the dark ground below.
    let zenith = vec3<f32>(0.0176, 0.0595, 0.1946);
    let horizon = vec3<f32>(0.4851, 0.5711, 0.6939);
    let ground = vec3<f32>(0.0116, 0.0176, 0.0319);
    var col: vec3<f32>;
    if (d.y >= 0.0) {
        col = mix(horizon, zenith, pow(d.y, 0.6));
    } else {
        col = mix(horizon, ground, min(1.0, -d.y * 10.0));
    }
    return vec4<f32>(col, 1.0);
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

/// An sRGB hex colour in linear RGB (three's `new Color(0xrrggbb)` in its linear working space).
fn hex(rgb: u32) -> [f32; 3] {
    [16, 8, 0].map(|s| srgb_to_linear(((rgb >> s) & 0xff) as u8) as f32)
}

/// The desktop's lights for a sun at this azimuth / elevation (`TerrainView.tsx`): the sun's
/// colour × intensity, then the hemisphere light's sky and ground colours × intensity.
pub fn lights(azimuth_deg: f32, elevation_deg: f32) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3]) {
    let (az, el) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    // Azimuth is where the light comes FROM, clockwise from north (−z).
    let dir = [az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()];
    let warm = ((20.0 - elevation_deg) / 20.0).clamp(0.0, 1.0);
    let (white, amber) = ([1.0, 1.0, 1.0], hex(0xffa054));
    let sun_i = 1.6 * (0.55 + 0.45 * el.sin());
    let sun = [0, 1, 2].map(|i| (white[i] + (amber[i] - white[i]) * warm) * sun_i);
    let hemi_i = 0.9 * (0.6 + 0.4 * el.sin());
    let (sky, ground) = (hex(0xdfe8ff), hex(0x3a3630));
    (
        dir,
        sun,
        sky.map(|c| c * hemi_i),
        ground.map(|c| c * hemi_i),
    )
}

/// The desktop's fog planes (`applyFog`): 0 → a faint aerial perspective from 2× to 12× the
/// world's size, 1 → the view closes within the world.
pub fn fog_planes(size: f32, amount: f32) -> [f32; 2] {
    let f = amount.clamp(0.0, 1.0);
    [size * (2.0 - 1.95 * f), size * (12.0 - 11.2 * f)]
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
    /// The world's larger side (metres): clip planes, zoom limits, fog and the dome follow it.
    pub size: f32,
}

const FOV_DEG: f32 = 50.0;

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
            perspective(FOV_DEG.to_radians(), aspect, near, far),
            look_at(self.eye(), [0.0, 0.0, 0.0]),
        )
    }

    /// right, up, forward scaled for sky rays (`forward + right·x + up·y` through NDC x, y).
    fn basis(&self, aspect: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let f = normalize(sub([0.0, 0.0, 0.0], self.eye()));
        let r = normalize(cross(f, [0.0, 1.0, 0.0]));
        let u = cross(r, f);
        let t = (FOV_DEG.to_radians() / 2.0).tan();
        (r.map(|c| c * t * aspect), u.map(|c| c * t), f)
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

/// An RGBA8 image (PNG-decoded) for a texture.
#[derive(Clone)]
pub struct Image {
    /// Width, height.
    pub size: [u32; 2],
    /// RGBA rows, top first.
    pub rgba: Vec<u8>,
}

impl Image {
    /// Decode a base64 PNG (the ops' `*_png_base64`).
    pub fn from_png_base64(b64: &str) -> Result<Self, String> {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("bad base64: {e}"))?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|e| format!("bad PNG: {e}"))?
            .into_rgba8();
        Ok(Self {
            size: [img.width(), img.height()],
            rgba: img.into_raw(),
        })
    }
}

/// The baked material maps of the shown layer (`terrain.textures`).
#[derive(Clone)]
pub struct Maps {
    /// Albedo (sRGB).
    pub albedo: Image,
    /// Tangent-space normal (+x east, +y north, +z up).
    pub normal: Image,
    /// Roughness in G.
    pub roughness: Image,
    /// Emitted light (sRGB), when any material glows.
    pub emissive: Option<Image>,
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
    terrain: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    sea: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    samples: u32,
    /// Textures bound in slots 2–6 (a 1 × 1 stand-in where nothing is loaded).
    textures: [wgpu::TextureView; 5],
    bind_group: wgpu::BindGroup,
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

/// The framed block's wall triangles and rim line points.
type Frame = (Vec<Vertex>, Vec<ColorVertex>);

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    /// The framed block: wall triangles (terrain pipeline) and rim lines.
    walls: Option<(wgpu::Buffer, u32)>,
    rim: Option<(wgpu::Buffer, u32)>,
}

/// A preview as the desktop's `buildTerrainGeometry`: a grid over `extent_m` centred on the
/// origin, row 0 to the south (+z), absolute heights, the height ramp blended to rock by slope.
fn grid_mesh(pv: &Preview, extent_m: [f32; 2]) -> (Vec<Vertex>, Vec<u32>) {
    let (w, h) = (pv.width.max(2), pv.height.max(2));
    let q =
        |x: u32, y: u32| pv.data[(y.min(pv.height - 1) * pv.width + x.min(pv.width - 1)) as usize];
    let at = |x: u32, y: u32| pv.min + f32::from(q(x, y)) / 65535.0 * (pv.max - pv.min);
    let (dx, dz) = (extent_m[0] / (w - 1) as f32, extent_m[1] / (h - 1) as f32);
    let mut vertices = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            // Central differences (one-sided at the border).
            let (l, r) = (at(x.saturating_sub(1), y), at(x + 1, y));
            let (south, north) = (at(x, y.saturating_sub(1)), at(x, y + 1));
            let nx = (l - r) / (dx * (x.min(w - 2) + 1 - x.saturating_sub(1)) as f32);
            let nz = (north - south) / (dz * (y.min(h - 2) + 1 - y.saturating_sub(1)) as f32);
            let normal = normalize([nx, 1.0, nz]);
            let slope = ((1.0 - normal[1]) * 1.4).clamp(0.0, 1.0);
            let c = ramp(q(x, y));
            vertices.push(Vertex {
                pos: [
                    x as f32 * dx - extent_m[0] / 2.0,
                    at(x, y),
                    extent_m[1] / 2.0 - y as f32 * dz,
                ],
                normal,
                color: [0, 1, 2].map(|i| c[i] + (ROCK[i] - c[i]) * slope),
                uv: [x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32],
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
    (vertices, indices)
}

/// Where a ray from `eye` along `dir` first meets the preview's surface over `extent_m` (the
/// mesh's bilinear heights): march in half-cell steps, then bisect the crossing.
fn pick_heightfield(
    pv: &Preview,
    extent_m: [f32; 2],
    eye: [f32; 3],
    dir: [f32; 3],
) -> Option<[f32; 2]> {
    let (w, h) = (pv.width.max(2), pv.height.max(2));
    let (dx, dz) = (extent_m[0] / (w - 1) as f32, extent_m[1] / (h - 1) as f32);
    let height_at = |x: f32, z: f32| -> Option<f32> {
        let (u, v) = ((x + extent_m[0] / 2.0) / dx, (extent_m[1] / 2.0 - z) / dz);
        if !(0.0..=(w - 1) as f32).contains(&u) || !(0.0..=(h - 1) as f32).contains(&v) {
            return None;
        }
        let (x0, y0) = ((u as u32).min(w - 2), (v as u32).min(h - 2));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let q = |x: u32, y: u32| {
            let i = (y.min(pv.height - 1) * pv.width + x.min(pv.width - 1)) as usize;
            pv.min + f32::from(pv.data[i]) / 65535.0 * (pv.max - pv.min)
        };
        let top = q(x0, y0) * (1.0 - fx) + q(x0 + 1, y0) * fx;
        let bottom = q(x0, y0 + 1) * (1.0 - fx) + q(x0 + 1, y0 + 1) * fx;
        Some(top * (1.0 - fy) + bottom * fy)
    };
    let at = |t: f32| {
        [
            eye[0] + dir[0] * t,
            eye[1] + dir[1] * t,
            eye[2] + dir[2] * t,
        ]
    };
    let above = |t: f32| {
        let p = at(t);
        height_at(p[0], p[2]).map(|g| p[1] - g)
    };
    let step = dx.min(dz) * 0.5;
    let far = (dot(eye, eye).sqrt() + extent_m[0].max(extent_m[1])) * 2.0;
    let (mut t, mut prev): (f32, Option<(f32, f32)>) = (0.0, None);
    while t < far {
        if let Some(d) = above(t) {
            if d <= 0.0 {
                // Crossed between the last sample above ground and this one: bisect.
                let (mut lo, mut hi) = prev.map_or((t, t), |(pt, _)| (pt, t));
                for _ in 0..24 {
                    let mid = (lo + hi) / 2.0;
                    if above(mid).is_some_and(|m| m > 0.0) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let p = at(hi);
                return Some([p[0] + extent_m[0] / 2.0, extent_m[1] / 2.0 - p[2]]);
            }
            prev = Some((t, d));
        }
        t += step;
    }
    None
}

/// The desktop's horizon skirt: the noise continued over `scale` × the world, the ramp colours a
/// touch dimmer (× 0.85), never the maps, and the triangles over the world itself dropped so the
/// real terrain shows through the hole.
fn skirt_mesh(pv: &Preview, world_m: [f32; 2], scale: f32) -> (Vec<Vertex>, Vec<u32>) {
    let (mut vertices, indices) = grid_mesh(pv, [world_m[0] * scale, world_m[1] * scale]);
    for v in &mut vertices {
        v.color = v.color.map(|c| c * 0.85);
        v.uv = [-1.0, -1.0];
    }
    let kept: Vec<u32> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|t| {
            let c = |k: usize| t.iter().map(|&i| vertices[i as usize].pos[k]).sum::<f32>() / 3.0;
            c(0).abs() > world_m[0] / 2.0 || c(2).abs() > world_m[1] / 2.0
        })
        .flatten()
        .copied()
        .collect();
    (vertices, kept)
}

/// The framed block of a preview (the desktop's `buildFrame`): walls from the edge profile down
/// to `base`, flat-shaded facing out, and the rim — the edge profile and the base rectangle.
fn frame_geometry(vertices: &[Vertex], w: u32, h: u32, base: f32) -> Frame {
    // The boundary once around: north row west → east, east column north → south, south row
    // east → west, west column south → north (rows run south → north in `vertices`).
    let mut ring: Vec<(u32, u32)> = (0..w).map(|x| (x, h - 1)).collect();
    ring.extend((0..h - 1).rev().map(|y| (w - 1, y)));
    ring.extend((0..w - 1).rev().map(|x| (x, 0)));
    ring.extend((1..h - 1).map(|y| (0, y)));
    let wall = hex(0x2a2f36);
    let (mut walls, mut rim) = (Vec::new(), Vec::new());
    for k in 0..ring.len() {
        let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
        let pa = vertices[(a.1 * w + a.0) as usize].pos;
        let pb = vertices[(b.1 * w + b.0) as usize].pos;
        let (qa, qb) = ([pa[0], base, pa[2]], [pb[0], base, pb[2]]);
        // Outward: the walk is clockwise seen from above, so the edge direction (e.x, e.z) turns
        // to (e.z, −e.x): east along the north row → north, south down the east column → east.
        let e = normalize(sub(pb, pa));
        let normal = normalize([e[2], 0.0, -e[0]]);
        for pos in [pa, pb, qb, pa, qb, qa] {
            walls.push(Vertex {
                pos,
                normal,
                color: wall,
                uv: [-1.0, -1.0],
            });
        }
        // The rim's teal (0x3fd0c9), as the desktop's LineBasicMaterial.
        let teal = [0.0497, 0.6308, 0.5841, 1.0];
        rim.extend([pa, pb, qa, qb].map(|pos| ColorVertex { pos, color: teal }));
    }
    (walls, rim)
}

/// The viewport's state; [`Viewport::show`] draws it into the remaining space of a `Ui`.
pub struct Viewport {
    gpu: Option<Gpu>,
    targets: Option<Targets>,
    mesh: Option<Mesh>,
    pending: Option<(Vec<Vertex>, Vec<u32>, Option<Frame>)>,
    pending_maps: Option<Option<Maps>>,
    pending_skirt: Option<Option<(Vec<Vertex>, Vec<u32>)>>,
    skirt: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    skirt_px: Option<u32>,
    /// The preview shown and its extent (the frame toggle rebuilds from it).
    shown: Option<(Preview, [f32; 2])>,
    pending_sky: Option<Option<Image>>,
    info: Option<TerrainInfo>,
    /// The camera; set it and the next frame re-renders.
    pub camera: Camera,
    /// Draw the world as a framed block (walls and rim), as the desktop does by default.
    pub frame: bool,
    sun_deg: [f32; 2],
    fog: f32,
    maps_px: Option<[u32; 2]>,
    emissive: bool,
    sky_px: Option<[u32; 2]>,
    dirty: bool,
    rect: Rect,
    renders: u64,
    sea: Option<Sea>,
    pending_sea: Option<Option<SeaGeometry>>,
    sea_mesh: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    shore: Option<(wgpu::Buffer, u32)>,
    shore_segments: usize,
    sea_vertices: usize,
    started: std::time::Instant,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            gpu: None,
            targets: None,
            mesh: None,
            pending: None,
            pending_maps: None,
            pending_skirt: None,
            skirt: None,
            skirt_px: None,
            shown: None,
            pending_sky: None,
            info: None,
            camera: Camera::fit([1.0, 1.0]),
            frame: true,
            sun_deg: [140.0, 35.0],
            fog: 0.0,
            maps_px: None,
            emissive: false,
            sky_px: None,
            dirty: true,
            rect: Rect::NOTHING,
            renders: 0,
            sea: None,
            pending_sea: None,
            sea_mesh: None,
            shore: None,
            shore_segments: 0,
            sea_vertices: 0,
            started: std::time::Instant::now(),
        }
    }
}

impl Viewport {
    /// Show this preview of a layer covering `extent_m`. The camera re-frames when the ground's
    /// extent changes (a new project), not on every refresh.
    pub fn set_terrain(&mut self, pv: &Preview, extent_m: [f32; 2]) {
        self.shown = Some((pv.clone(), extent_m));
        let (w, h) = (pv.width.max(2), pv.height.max(2));
        let (vertices, indices) = grid_mesh(pv, extent_m);
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
        // The desktop's frame (on by default): the base sits 15 % of the relief (≥ 10 m) below.
        let frame = self.frame.then(|| {
            let base = pv.min - (pv.max - pv.min).mul_add(0.15, 0.0).max(10.0);
            frame_geometry(&vertices, w, h, base)
        });
        self.pending = Some((vertices, indices, frame));
        self.dirty = true;
        self.rebuild_sea();
    }

    /// Draw the world as a framed block or not (the viewport's Frame button).
    pub fn set_frame(&mut self, on: bool) {
        self.frame = on;
        if let Some((pv, extent)) = self.shown.take() {
            self.set_terrain(&pv, extent);
        }
    }

    /// The ground under a point of the viewport (points, as egui reports the pointer), as the
    /// desktop's raycast reports it: metres east of the west edge and north of the south edge.
    pub fn pick(&self, at: egui::Pos2) -> Option<[f32; 2]> {
        let (pv, extent) = self.shown.as_ref()?;
        let r = self.rect;
        if !r.contains(at) || r.width() < 1.0 || r.height() < 1.0 {
            return None;
        }
        let ndc = [
            (at.x - r.left()) / r.width() * 2.0 - 1.0,
            1.0 - (at.y - r.top()) / r.height() * 2.0,
        ];
        let (right, up, forward) = self.camera.basis(r.width() / r.height());
        let dir = normalize([0, 1, 2].map(|i| forward[i] + right[i] * ndc[0] + up[i] * ndc[1]));
        pick_heightfield(pv, *extent, self.camera.eye(), dir)
    }

    /// Back to the home view (the viewport's Fit button).
    pub fn fit(&mut self) {
        if let Some(i) = &self.info {
            self.camera = Camera::fit(i.extent_m);
            self.dirty = true;
        }
    }

    /// Drop the terrain (no project, or no layer).
    pub fn clear(&mut self) {
        self.shown = None;
        self.set_horizon(None);
        self.mesh = None;
        self.pending = None;
        self.info = None;
        self.set_maps(None);
        self.dirty = true;
    }

    /// Shade with these baked maps (`None`: the ramp vertex colours).
    pub fn set_maps(&mut self, maps: Option<Maps>) {
        self.maps_px = maps.as_ref().map(|m| m.albedo.size);
        self.emissive = maps.as_ref().is_some_and(|m| m.emissive.is_some());
        self.pending_maps = Some(maps);
        self.dirty = true;
    }

    /// The horizon skirt: this preview of the noise over `scale` × the world (`None`: off).
    pub fn set_horizon(&mut self, horizon: Option<(&Preview, f32)>) {
        let world = self.info.as_ref().map(|i| i.extent_m);
        self.skirt_px = horizon.map(|(pv, _)| pv.width);
        self.pending_skirt = Some(match (horizon, world) {
            (Some((pv, scale)), Some(world)) => Some(skirt_mesh(pv, world, scale)),
            _ => None,
        });
        self.dirty = true;
    }

    /// The skirt's samples per side while it is shown.
    pub fn horizon_px(&self) -> Option<u32> {
        self.skirt_px
    }

    /// The physical-sky preview on the dome (`None`: the gradient).
    pub fn set_sky(&mut self, sky: Option<Image>) {
        self.sky_px = sky.as_ref().map(|s| s.size);
        self.pending_sky = Some(sky);
        self.dirty = true;
    }

    /// Sun from azimuth (clockwise from north) and elevation, in degrees.
    pub fn set_sun(&mut self, azimuth_deg: f32, elevation_deg: f32) {
        self.sun_deg = [azimuth_deg, elevation_deg];
        self.dirty = true;
    }

    /// The sea (`None`: no sea-level node, or a non-height layer shown).
    pub fn set_sea(&mut self, sea: Option<Sea>) {
        if self.sea == sea {
            return;
        }
        self.sea = sea;
        self.rebuild_sea();
    }

    fn rebuild_sea(&mut self) {
        let built = match (&self.sea, &self.shown) {
            (Some(sea), Some((pv, extent))) => {
                let (v, i) = sea_mesh(pv, *extent, sea);
                // The desktop draws the shoreline only while the level is inside the terrain's range.
                let shore = if sea.level_m > pv.min && sea.level_m < pv.max {
                    shoreline(pv, *extent, sea.level_m)
                } else {
                    Vec::new()
                };
                Some((v, i, shore))
            }
            _ => None,
        };
        self.sea_vertices = built.as_ref().map_or(0, |b| b.0.len());
        self.shore_segments = built.as_ref().map_or(0, |b| b.2.len() / 2);
        self.pending_sea = Some(built);
        self.dirty = true;
    }

    /// The sea shown: its level, and the wind sea's height and wavelength when it has one.
    pub fn sea_info(&self) -> Option<(f32, Option<[f32; 2]>)> {
        self.sea
            .as_ref()
            .filter(|_| self.shown.is_some())
            .map(|s| (s.level_m, s.waves_m))
    }

    /// Shoreline segments drawn.
    pub fn shore_segments(&self) -> usize {
        self.shore_segments
    }

    /// The desktop animates the waves on the CPU and skips seas over 70 000 vertices; Studio moves
    /// them in the vertex shader under the same rule, so the two look the same.
    fn waves_animate(&self) -> bool {
        self.sea.as_ref().is_some_and(|s| !s.waves.is_empty()) && self.sea_vertices <= 70_000
    }

    /// Fog amount 0..1 (the Sun panel's Fog).
    pub fn set_fog(&mut self, amount: f32) {
        self.fog = amount;
        self.dirty = true;
    }

    /// The terrain shown, if any.
    pub fn info(&self) -> Option<&TerrainInfo> {
        self.info.as_ref()
    }

    /// The baked maps' size while they shade the terrain.
    pub fn maps_px(&self) -> Option<[u32; 2]> {
        self.maps_px
    }

    /// The sky preview's size while it is on the dome.
    pub fn sky_px(&self) -> Option<[u32; 2]> {
        self.sky_px
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
        let gpu = self.gpu.get_or_insert_with(|| create_gpu(rs));
        let mut rebind = false;
        if let Some(maps) = self.pending_maps.take() {
            let views = maps_views(rs, maps.as_ref());
            gpu.textures[..4].clone_from_slice(&views);
            rebind = true;
        }
        if let Some(sky) = self.pending_sky.take() {
            gpu.textures[4] = match &sky {
                Some(img) => upload_image(rs, img, true, false),
                None => stand_in(rs, true),
            };
            rebind = true;
        }
        if rebind {
            gpu.bind_group = bind_group(rs, gpu);
        }
        if let Some(skirt) = self.pending_skirt.take() {
            use wgpu::util::DeviceExt as _;
            self.skirt = skirt.map(|(v, i)| {
                let buf = |label, contents: &[u8], usage| {
                    rs.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents,
                            usage,
                        })
                };
                (
                    buf(
                        "skirt vertices",
                        bytemuck::cast_slice(&v),
                        wgpu::BufferUsages::VERTEX,
                    ),
                    buf(
                        "skirt indices",
                        bytemuck::cast_slice(&i),
                        wgpu::BufferUsages::INDEX,
                    ),
                    i.len() as u32,
                )
            });
        }
        if let Some(sea) = self.pending_sea.take() {
            use wgpu::util::DeviceExt as _;
            let buf = |label, contents: &[u8], usage| {
                rs.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents,
                        usage,
                    })
            };
            self.sea_mesh = sea.as_ref().map(|(v, i, _)| {
                (
                    buf(
                        "sea vertices",
                        bytemuck::cast_slice(v),
                        wgpu::BufferUsages::VERTEX,
                    ),
                    buf(
                        "sea indices",
                        bytemuck::cast_slice(i),
                        wgpu::BufferUsages::INDEX,
                    ),
                    i.len() as u32,
                )
            });
            self.shore =
                sea.as_ref()
                    .filter(|(_, _, shore)| !shore.is_empty())
                    .map(|(_, _, shore)| {
                        (
                            buf(
                                "shoreline",
                                bytemuck::cast_slice(shore),
                                wgpu::BufferUsages::VERTEX,
                            ),
                            shore.len() as u32,
                        )
                    });
        }
        if self.waves_animate() {
            self.dirty = true;
            ui.ctx().request_repaint();
        }
        if let Some((v, i, frame)) = self.pending.take() {
            self.mesh = Some(upload(rs, &v, &i, frame.as_ref()));
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

    fn uniforms(&self, aspect: f32) -> Uniforms {
        let cam = &self.camera;
        let (dir, sun, sky, ground) = lights(self.sun_deg[0], self.sun_deg[1]);
        let (right, up, forward) = cam.basis(aspect);
        let fog = fog_planes(cam.size, self.fog);
        let w = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        Uniforms {
            view_proj: cam.view_proj(aspect),
            eye: w(cam.eye()),
            right: w(right),
            up: w(up),
            forward: w(forward),
            sun_dir: w(dir),
            sun_light: w(sun),
            hemi_sky: w(sky),
            hemi_ground: w(ground),
            fog_color: w(hex(0xb9c7d9)),
            fog: [fog[0], fog[1], 0.0, 0.0],
            flags: [
                f32::from(u8::from(self.maps_px.is_some())),
                f32::from(u8::from(self.emissive)),
                f32::from(u8::from(self.sky_px.is_some())),
                cam.size * 40.0,
            ],
            time: [
                self.started.elapsed().as_secs_f32(),
                self.sea.as_ref().map_or(0, |s| s.waves.len().min(3)) as f32,
                f32::from(u8::from(self.waves_animate())),
                0.0,
            ],
            waves: {
                let mut w = [[0.0; 4]; 6];
                for (k, c) in self
                    .sea
                    .iter()
                    .flat_map(|s| s.waves.iter())
                    .take(3)
                    .enumerate()
                {
                    w[2 * k] = [c.amplitude_m, c.wavelength_m, c.direction_deg, c.speed_mps];
                    w[2 * k + 1] = [c.phase, 0.0, 0.0, 0.0];
                }
                w
            },
        }
    }

    fn render(&mut self, rs: &RenderState, size: [u32; 2]) {
        let u = self.uniforms(size[0] as f32 / size[1] as f32);
        let (Some(gpu), Some(t), Some(mesh)) = (&self.gpu, &self.targets, &self.mesh) else {
            return;
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
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.set_pipeline(&gpu.sky);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&gpu.terrain);
            // The skirt first (the desktop's renderOrder −0.5), then the world.
            if let Some((v, i, n)) = &self.skirt {
                pass.set_vertex_buffer(0, v.slice(..));
                pass.set_index_buffer(i.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..*n, 0, 0..1);
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, 0..1);
            if let Some((walls, n)) = &mesh.walls {
                pass.set_vertex_buffer(0, walls.slice(..));
                pass.draw(0..*n, 0..1);
            }
            pass.set_pipeline(&gpu.lines);
            if let Some((rim, n)) = &mesh.rim {
                pass.set_vertex_buffer(0, rim.slice(..));
                pass.draw(0..*n, 0..1);
            }
            if let Some((shore, n)) = &self.shore {
                pass.set_vertex_buffer(0, shore.slice(..));
                pass.draw(0..*n, 0..1);
            }
            // Transparent last, as three sorts it: the sea.
            if let Some((v, i, n)) = &self.sea_mesh {
                pass.set_pipeline(&gpu.sea);
                pass.set_vertex_buffer(0, v.slice(..));
                pass.set_index_buffer(i.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..*n, 0, 0..1);
            }
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

/// A texture with its full mip chain (box-filtered on the CPU, as three generates mips).
fn upload_image(rs: &RenderState, img: &Image, srgb: bool, mips: bool) -> wgpu::TextureView {
    use wgpu::util::DeviceExt as _;
    let mut levels = vec![(img.size, img.rgba.clone())];
    if mips {
        while let Some(([w, h], data)) = levels.last()
            && (*w > 1 || *h > 1)
        {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut next = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..4 {
                        let px = |xx: u32, yy: u32| {
                            u32::from(data[((yy.min(h - 1) * w + xx.min(w - 1)) * 4 + c) as usize])
                        };
                        let sum = px(2 * x, 2 * y)
                            + px(2 * x + 1, 2 * y)
                            + px(2 * x, 2 * y + 1)
                            + px(2 * x + 1, 2 * y + 1);
                        next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                    }
                }
            }
            levels.push(([nw, nh], next));
        }
    }
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let data: Vec<u8> = levels.iter().flat_map(|(_, d)| d.iter().copied()).collect();
    rs.device
        .create_texture_with_data(
            &rs.queue,
            &wgpu::TextureDescriptor {
                label: Some("viewport map"),
                size: wgpu::Extent3d {
                    width: img.size[0],
                    height: img.size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::MipMajor,
            &data,
        )
        .create_view(&Default::default())
}

fn stand_in(rs: &RenderState, srgb: bool) -> wgpu::TextureView {
    let img = Image {
        size: [1, 1],
        rgba: vec![128, 128, 255, 255],
    };
    upload_image(rs, &img, srgb, false)
}

fn maps_views(rs: &RenderState, maps: Option<&Maps>) -> [wgpu::TextureView; 4] {
    match maps {
        Some(m) => [
            upload_image(rs, &m.albedo, true, true),
            upload_image(rs, &m.normal, false, true),
            upload_image(rs, &m.roughness, false, true),
            m.emissive
                .as_ref()
                .map_or_else(|| stand_in(rs, true), |e| upload_image(rs, e, true, true)),
        ],
        None => [
            stand_in(rs, true),
            stand_in(rs, false),
            stand_in(rs, false),
            stand_in(rs, true),
        ],
    }
}

fn bind_group(rs: &RenderState, gpu: &Gpu) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: gpu.uniforms.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(&gpu.sampler),
        },
    ];
    for (k, view) in gpu.textures.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry {
            binding: 2 + k as u32,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport"),
        layout: &gpu.layout,
        entries: &entries,
    })
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
        label: Some("viewport"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport uniforms"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
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
        label: Some("viewport"),
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
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_entry(2),
            texture_entry(3),
            texture_entry(4),
            texture_entry(5),
            texture_entry(6),
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("viewport"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let attributes =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x2];
    let multisample = wgpu::MultisampleState {
        count: samples,
        ..Default::default()
    };
    let target = [Some(COLOR_SRGB.into())];
    let terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        multisample,
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &target,
        }),
        multiview_mask: None,
        cache: None,
    });
    let line_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
    let blended = [Some(wgpu::ColorTargetState {
        format: COLOR_SRGB,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    })];
    let color_vertex = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ColorVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &line_attributes,
    };
    let sea = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sea"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("sea_vs"),
            compilation_options: Default::default(),
            buffers: &[Some(color_vertex.clone())],
        },
        primitive: wgpu::PrimitiveState::default(),
        // The desktop's sea: transparent, depth-tested, no depth write.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample,
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("sea_fs"),
            compilation_options: Default::default(),
            targets: &blended,
        }),
        multiview_mask: None,
        cache: None,
    });
    let lines = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rim"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("line_vs"),
            compilation_options: Default::default(),
            buffers: &[Some(color_vertex)],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample,
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("line_fs"),
            compilation_options: Default::default(),
            targets: &blended,
        }),
        multiview_mask: None,
        cache: None,
    });
    let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("sky_vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        // Behind everything, writes no depth (the desktop's dome: renderOrder −1, depthWrite off).
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample,
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("sky_fs"),
            compilation_options: Default::default(),
            targets: &target,
        }),
        multiview_mask: None,
        cache: None,
    });
    let mut gpu = Gpu {
        terrain,
        lines,
        sea,
        sky,
        layout,
        uniforms,
        sampler,
        samples,
        textures: [
            stand_in(rs, true),
            stand_in(rs, false),
            stand_in(rs, false),
            stand_in(rs, true),
            stand_in(rs, true),
        ],
        // Replaced right below; wgpu needs a real group to start from.
        bind_group: rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &rs
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &[],
                }),
            entries: &[],
        }),
    };
    gpu.bind_group = bind_group(rs, &gpu);
    gpu
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

fn upload(rs: &RenderState, vertices: &[Vertex], indices: &[u32], frame: Option<&Frame>) -> Mesh {
    use wgpu::util::DeviceExt as _;
    let buffer = |label, contents: &[u8], usage| {
        rs.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
    };
    let walls = frame.map(|(w, _)| {
        (
            buffer(
                "frame walls",
                bytemuck::cast_slice(w),
                wgpu::BufferUsages::VERTEX,
            ),
            w.len() as u32,
        )
    });
    let rim = frame.map(|(_, r)| {
        (
            buffer(
                "frame rim",
                bytemuck::cast_slice(r),
                wgpu::BufferUsages::VERTEX,
            ),
            r.len() as u32,
        )
    });
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
        walls,
        rim,
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
        // The sky rays agree with the projection: the screen's top edge is fov / 2 above forward.
        let (_, u, f) = cam.basis(1.5);
        let top = normalize([f[0] + u[0], f[1] + u[1], f[2] + u[2]]);
        assert!((dot(top, f).acos() - 25f32.to_radians()).abs() < 1e-4);
    }

    #[test]
    fn a_pick_lands_where_the_ray_meets_the_ground() {
        // A flat 10 m world over 200 × 100 m with a 30 m block in its north-east quarter.
        let mut data = vec![0u16; 5 * 3];
        data[2 * 5 + 4] = 65535; // row 2 (north), column 4 (east)
        let pv = Preview {
            width: 5,
            height: 3,
            min: 10.0,
            max: 40.0,
            data,
        };
        // Straight down onto the south-west quarter: lands right under the eye.
        let hit = pick_heightfield(&pv, [200.0, 100.0], [-50.0, 500.0, 25.0], [0.0, -1.0, 0.0])
            .expect("hit");
        assert!(
            (hit[0] - 50.0).abs() < 0.01 && (hit[1] - 25.0).abs() < 0.01,
            "{hit:?}"
        );
        // A slanted ray from the south-west, aimed at the flat ground's (100, 50): the ground is
        // 10 m up, so it lands a little short of the aim, on the line.
        let eye = [-100.0, 110.0, 150.0];
        let aim = [0.0, 0.0, 0.0];
        let d = normalize(sub(aim, eye));
        let hit = pick_heightfield(&pv, [200.0, 100.0], eye, d).expect("hit");
        let t = (110.0 - 10.0) / 110.0; // where y falls to 10 m along eye → aim
        let expect = [
            eye[0] + (aim[0] - eye[0]) * t + 100.0,
            50.0 - (eye[2] + (aim[2] - eye[2]) * t),
        ];
        assert!(
            (hit[0] - expect[0]).abs() < 0.05 && (hit[1] - expect[1]).abs() < 0.05,
            "{hit:?} vs {expect:?}"
        );
        // A ray into the sky misses.
        assert!(pick_heightfield(&pv, [200.0, 100.0], [0.0, 50.0, 0.0], [0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn the_sea_shades_by_depth_and_draws_its_shoreline() {
        let sea = Sea {
            level_m: 15.0,
            visibility_m: 15.0,
            tint: [0.1, 0.35, 0.4],
            breaking_depth_m: 0.0,
            waves: vec![],
            waves_m: None,
        };
        // Shallow water shows the sand; deep water the tint, and is more opaque.
        let (shallow, a0) = water_colour(0.5, &sea);
        let (deep, a1) = water_colour(60.0, &sea);
        assert!(shallow[0] > deep[0] && a1 > a0, "{shallow:?} {deep:?}");
        assert!((deep[1] - 0.35).abs() < 0.05, "{deep:?}");
        // A ramp from 10 m (west) to 20 m (east): water over the west half, land over the east.
        let data: Vec<u16> = (0..5 * 3)
            .map(|i| ((i % 5) as f32 / 4.0 * 65535.0) as u16)
            .collect();
        let pv = Preview {
            width: 5,
            height: 3,
            min: 10.0,
            max: 20.0,
            data,
        };
        let (v, i) = sea_mesh(&pv, [100.0, 50.0], &sea);
        assert_eq!((v.len(), i.len()), (15, 4 * 2 * 6));
        assert!(v.iter().all(|p| p.pos[1] == 15.0));
        assert_eq!(v[4].color[3], 0.0, "transparent over land");
        assert!(v[0].color[3] > 0.0, "visible over water");
        // The 15 m contour runs north–south through the middle column: one segment per cell row.
        let shore = shoreline(&pv, [100.0, 50.0], 15.0);
        assert_eq!(shore.len(), 2 * 2);
        assert!(
            shore
                .iter()
                .all(|p| p.pos[0].abs() < 1e-3 && p.color[3] == 0.85),
            "{:?}",
            shore.iter().map(|p| p.pos).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_skirt_leaves_a_hole_where_the_world_is() {
        let pv = Preview {
            width: 9,
            height: 9,
            min: 0.0,
            max: 10.0,
            data: vec![30000; 81],
        };
        let (v, i) = skirt_mesh(&pv, [100.0, 100.0], 3.0);
        assert_eq!(v.len(), 81);
        assert!(v.iter().all(|x| x.uv == [-1.0, -1.0]));
        // 8 × 8 cells of 37.5 m over 300 m: the centre cells (|centroid| < 50 m) are gone.
        assert!(i.len() < 8 * 8 * 6 && !i.is_empty());
        for t in i.as_chunks::<3>().0 {
            let c = |k: usize| t.iter().map(|&j| v[j as usize].pos[k]).sum::<f32>() / 3.0;
            assert!(c(0).abs() > 50.0 || c(2).abs() > 50.0);
        }
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
    fn lights_and_fog_follow_the_desktop() {
        // Sun at 90° elevation: full intensity white from straight above; hemi at 0.9.
        let (dir, sun, sky, _) = lights(140.0, 90.0);
        assert!((dir[1] - 1.0).abs() < 1e-6);
        assert!(sun.iter().all(|c| (c - 1.6).abs() < 1e-5), "{sun:?}");
        assert!((sky[2] - 0.9).abs() < 1e-3, "{sky:?}");
        // From the east at the horizon: warm, dimmer.
        let (dir, sun, ..) = lights(90.0, 0.0);
        assert!(
            (dir[0] - 1.0).abs() < 1e-6 && dir[2].abs() < 1e-6,
            "{dir:?}"
        );
        assert!(
            (sun[0] - 1.6 * 0.55).abs() < 1e-5 && sun[2] < 0.2,
            "{sun:?}"
        );
        assert_eq!(fog_planes(1000.0, 0.0), [2000.0, 12000.0]);
        let closed = fog_planes(1000.0, 1.0);
        assert!(
            (closed[0] - 50.0).abs() < 1e-3 && (closed[1] - 800.0).abs() < 1e-2,
            "{closed:?}"
        );
        // three's hex colours are sRGB, converted to linear.
        assert!((hex(0xb9c7d9)[0] - 0.4851).abs() < 1e-3);
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
        let (verts, idx, _) = v.pending.as_ref().expect("mesh");
        assert_eq!((verts.len(), idx.len()), (6, 2 * 2 * 3));
        // Row 0 is the south edge (+z); heights are absolute; uv v runs north from it.
        assert_eq!(verts[0].pos, [-100.0, 10.0, 50.0]);
        assert_eq!(verts[5].pos, [100.0, 20.0, -50.0]);
        assert_eq!((verts[0].uv, verts[5].uv), ([0.0, 0.0], [1.0, 1.0]));
        assert_eq!(verts[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(verts[0].color, [0.16, 0.3, 0.2]);
        // Normals lean away from higher ground: west of the raised corner (vertex 4) towards −x,
        // south of it (vertex 2, the ground rising northwards) towards +z.
        assert!(verts[4].normal[0] < 0.0, "{:?}", verts[4].normal);
        assert!(verts[2].normal[2] > 0.0, "{:?}", verts[2].normal);
        assert_eq!(v.info().expect("info").triangles, 4);
        // The framed block: the 3 × 2 boundary has 6 steps → 12 wall triangles, 24 rim points;
        // the base sits max(10, 15 % of 10 m) = 10 m under the lowest point.
        let (_, _, frame) = v.pending.as_ref().expect("mesh");
        let (walls, rim) = frame.as_ref().expect("frame");
        assert_eq!((walls.len(), rim.len()), (36, 24));
        assert!(
            walls
                .iter()
                .all(|w| w.uv == [-1.0, -1.0] && w.normal[1] == 0.0)
        );
        assert_eq!(rim.iter().map(|p| p.pos[1]).fold(f32::MAX, f32::min), 0.0);
        assert!(rim.iter().all(|p| p.color == [0.0497, 0.6308, 0.5841, 1.0]));
        // Walls face out: the north wall's normal points north (−z).
        let north = walls
            .iter()
            .find(|w| w.pos[2] == -50.0 && w.pos[0] == 0.0)
            .expect("north wall");
        assert!(north.normal[2] < -0.99, "{:?}", north.normal);
    }
}
