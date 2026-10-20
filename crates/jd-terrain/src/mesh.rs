//! Terrain mesh export as binary glTF (`.glb`), written by hand: a regular grid with
//! positions (metres, +Y up, centred on the world like the viewport), normals, UVs, u32
//! indices and one PBR material. No dependency beyond `serde_json`; validated in CI with the
//! Khronos glTF validator (`apps/desktop/scripts/validate-glb.mjs`).
//!
//! With `texture` the material carries a baked base-colour PNG (the viewport's hypsometric ramp
//! blended towards rock on steep slopes), one texel per exported vertex, so the file looks like
//! the app in Blender / three.js / engines without any material setup.
//!
//! OBJ, binary STL and binary PLY (with vertex colours) share one [`GridMesh`] builder; the GLB
//! path keeps its own streaming writer. [`export_mesh`] picks the writer from the extension.
//!
//! ponytail: one dense grid primitive, optional `step` decimation, one baked albedo; the GLB
//! writer duplicates the grid sampling (~30 lines) rather than buffering a `GridMesh` for huge
//! worlds — fold them together when LOD/chunked meshes land. Draco, normal/roughness maps and
//! MTL materials come with the material graph (08-io-formats.md).

use std::fs;
use std::path::{Path, PathBuf};

use crate::Heightfield;
use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

/// What `export_glb` wrote.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MeshExportReport {
    /// Written file.
    pub path: PathBuf,
    /// Always `glb`.
    pub format: String,
    /// Vertex count.
    pub vertices: u32,
    /// Triangle count.
    pub triangles: u32,
    /// Grid step used (1 = every sample).
    pub step: u32,
    /// Bytes written.
    pub bytes: u64,
    /// World extent covered, metres.
    pub extent_m: [f64; 2],
    /// Baked base-colour texture size in pixels when one was embedded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_px: Option<[u32; 2]>,
}

/// Hypsometric ramp (normalised height → RGB): lowland green → ochre → bare rock → snow.
/// Same stops as the viewport (`apps/desktop/src/viewport/TerrainView.tsx`).
const STOPS: [(f32, [f32; 3]); 5] = [
    (0.0, [0.16, 0.3, 0.2]),
    (0.25, [0.36, 0.48, 0.24]),
    (0.5, [0.62, 0.52, 0.34]),
    (0.75, [0.52, 0.5, 0.48]),
    (1.0, [0.95, 0.95, 0.97]),
];
const ROCK: [f32; 3] = [0.42, 0.4, 0.38];

/// Colour for a normalised height `t` (0..1) and a slope factor (0 flat … 1 steep).
pub fn hypsometric_rgb(t: f32, slope: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut k = 0;
    while k < STOPS.len() - 2 && t > STOPS[k + 1].0 {
        k += 1;
    }
    let (t0, c0) = STOPS[k];
    let (t1, c1) = STOPS[k + 1];
    let f = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for i in 0..3 {
        let c = c0[i] + (c1[i] - c0[i]) * f;
        let c = c + (ROCK[i] - c) * slope.clamp(0.0, 1.0);
        out[i] = (c * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Bake the base-colour texture for the exported grid (`gw`×`gh`, row 0 = south, matching the
/// mesh UVs where v = 0 on the south edge) as PNG bytes.
fn bake_albedo(
    sample: &dyn Fn(u32, u32) -> f32,
    normal_y: &dyn Fn(u32, u32) -> f32,
    gw: u32,
    gh: u32,
    min_h: f32,
    max_h: f32,
) -> JdResult<Vec<u8>> {
    let range = (max_h - min_h).max(1e-6);
    let mut px = Vec::with_capacity((gw * gh * 3) as usize);
    for gz in 0..gh {
        for gx in 0..gw {
            let t = (sample(gx, gz) - min_h) / range;
            let slope = ((1.0 - normal_y(gx, gz)) * 1.4).clamp(0.0, 1.0);
            px.extend_from_slice(&hypsometric_rgb(t, slope));
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut out,
        &px,
        gw,
        gh,
        image::ExtendedColorType::Rgb8,
        image::ImageFormat::Png,
    )
    .map_err(|e| {
        JdError::new(
            jd_types::ErrorCode::E_INTERNAL,
            format!("PNG encode failed: {e}"),
        )
    })?;
    Ok(out.into_inner())
}

/// Smallest step so the grid has at most `max_vertices` vertices.
pub fn auto_step(width: u32, height: u32, max_vertices: u32) -> u32 {
    let mut step = 1u32;
    let grid = |n: u32, s: u32| ((n.saturating_sub(1)).div_ceil(s) + 1) as u64;
    while grid(width, step) * grid(height, step) > max_vertices as u64 {
        step += 1;
    }
    step
}

fn pad4(v: &mut Vec<u8>, byte: u8) {
    while !v.len().is_multiple_of(4) {
        v.push(byte);
    }
}

/// Write the field as a `.glb` grid mesh covering `extent_m`: +X east, +Z south (field row 0 is
/// south and lands at +Z/2, matching the viewport and the image exports).
pub fn export_glb(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
) -> JdResult<MeshExportReport> {
    hf.validate_finite()?;
    if step == 0 {
        return Err(JdError::invalid("step", "must be ≥ 1"));
    }
    if !(extent_m[0] > 0.0 && extent_m[1] > 0.0) {
        return Err(JdError::invalid("extent_m", "must be positive"));
    }
    let (w, h) = (hf.width(), hf.height());
    // Sample 0, step, 2·step, … and always the last row/column so the mesh reaches the edge.
    let (gw, gh) = ((w - 1).div_ceil(step) + 1, (h - 1).div_ceil(step) + 1);
    if gw < 2 || gh < 2 {
        return Err(JdError::invalid(
            "step",
            "grid must keep at least 2×2 samples",
        ));
    }
    let n_vertices = (gw as u64) * (gh as u64);
    if n_vertices > u32::MAX as u64 {
        return Err(JdError::invalid(
            "step",
            "too many vertices for 32-bit indices",
        ));
    }
    let (sx, sz) = (extent_m[0] / w as f64, extent_m[1] / h as f64);
    let sample = |gx: u32, gz: u32| hf.get((gx * step).min(w - 1), (gz * step).min(h - 1));
    let xm = |gx: u32| ((gx * step).min(w - 1) as f64 + 0.5) * sx - extent_m[0] / 2.0;
    let zm = |gz: u32| extent_m[1] / 2.0 - ((gz * step).min(h - 1) as f64 + 0.5) * sz;

    let mut bin: Vec<u8> =
        Vec::with_capacity((n_vertices as usize) * 32 + ((gw - 1) * (gh - 1)) as usize * 24);
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for gz in 0..gh {
        for gx in 0..gw {
            let p = [xm(gx) as f32, sample(gx, gz), zm(gz) as f32];
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let pos_len = bin.len();
    let mut normal_y: Vec<f32> = Vec::with_capacity(n_vertices as usize);
    for gz in 0..gh {
        for gx in 0..gw {
            // Central differences over the exported grid (clamped at the border).
            let (x0, x1) = (gx.saturating_sub(1), (gx + 1).min(gw - 1));
            let (z0, z1) = (gz.saturating_sub(1), (gz + 1).min(gh - 1));
            // zm decreases with the row index (south is +Z), so guard the magnitude, not the sign.
            let guard = |d: f64| if d.abs() < 1e-9 { 1e-9 } else { d };
            let dx = (sample(x1, gz) - sample(x0, gz)) as f64 / guard(xm(x1) - xm(x0));
            let dz = (sample(gx, z1) - sample(gx, z0)) as f64 / guard(zm(z1) - zm(z0));
            let len = (dx * dx + 1.0 + dz * dz).sqrt();
            normal_y.push((1.0 / len) as f32);
            for v in [(-dx / len) as f32, (1.0 / len) as f32, (-dz / len) as f32] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let nrm_len = bin.len() - pos_len;
    for gz in 0..gh {
        for gx in 0..gw {
            for v in [gx as f32 / (gw - 1) as f32, gz as f32 / (gh - 1) as f32] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let uv_len = bin.len() - pos_len - nrm_len;
    let idx_off = bin.len();
    for gz in 0..gh - 1 {
        for gx in 0..gw - 1 {
            let a = gz * gw + gx;
            let b = a + 1;
            let c = a + gw;
            let d = c + 1;
            // Row c is north of row a (−Z): (a, b, c) and (b, d, c) are counter-clockwise from +Y.
            for i in [a, b, c, b, d, c] {
                bin.extend_from_slice(&i.to_le_bytes());
            }
        }
    }
    let idx_len = bin.len() - idx_off;
    let n_triangles = (gw - 1) * (gh - 1) * 2;
    pad4(&mut bin, 0);
    // Optional baked albedo: one more buffer view holding a PNG; UVs already map v = 0 to south.
    let mut image_view = None;
    if texture {
        let png = bake_albedo(
            &sample,
            &|gx, gz| normal_y[(gz * gw + gx) as usize],
            gw,
            gh,
            min[1],
            max[1],
        )?;
        let off = bin.len();
        bin.extend_from_slice(&png);
        image_view = Some((off, png.len()));
        pad4(&mut bin, 0);
    }

    let mut json = serde_json::json!({
        "asset": { "version": "2.0", "generator": format!("jenga-dunia {}", env!("CARGO_PKG_VERSION")) },
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": pos_len, "target": 34962 },
            { "buffer": 0, "byteOffset": pos_len, "byteLength": nrm_len, "target": 34962 },
            { "buffer": 0, "byteOffset": pos_len + nrm_len, "byteLength": uv_len, "target": 34962 },
            { "buffer": 0, "byteOffset": idx_off, "byteLength": idx_len, "target": 34963 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": n_vertices, "type": "VEC3", "min": min, "max": max },
            { "bufferView": 1, "componentType": 5126, "count": n_vertices, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": n_vertices, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5125, "count": n_triangles * 3, "type": "SCALAR" }
        ],
        "materials": [{ "name": "terrain", "pbrMetallicRoughness": { "baseColorFactor": [0.55, 0.5, 0.45, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.95 } }],
        "meshes": [{ "name": "terrain", "primitives": [{ "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 }, "indices": 3, "material": 0, "mode": 4 }] }],
        "nodes": [{ "mesh": 0, "name": "terrain" }],
        "scenes": [{ "nodes": [0] }],
        "scene": 0
    });
    if let Some((off, len)) = image_view {
        json["bufferViews"]
            .as_array_mut()
            .ok_or_else(|| JdError::new(jd_types::ErrorCode::E_INTERNAL, "bufferViews"))?
            .push(serde_json::json!({ "buffer": 0, "byteOffset": off, "byteLength": len }));
        json["images"] = serde_json::json!([{ "bufferView": 4, "mimeType": "image/png", "name": "terrain-albedo" }]);
        json["samplers"] = serde_json::json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }]);
        json["textures"] = serde_json::json!([{ "sampler": 0, "source": 0 }]);
        json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] =
            serde_json::json!({ "index": 0, "texCoord": 0 });
        json["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] =
            serde_json::json!([1.0, 1.0, 1.0, 1.0]);
    }
    let mut json_bytes = serde_json::to_vec(&json)?;
    pad4(&mut json_bytes, b' ');

    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &out)?;
    Ok(MeshExportReport {
        path: path.to_path_buf(),
        format: "glb".into(),
        vertices: n_vertices as u32,
        triangles: n_triangles,
        step,
        bytes: out.len() as u64,
        extent_m,
        texture_px: texture.then_some([gw, gh]),
    })
}

/// A regular grid mesh in metres: +X east, +Y up, +Z south, centred on the world (same frame as
/// the GLB writer and the viewport).
pub struct GridMesh {
    /// Vertex positions, 3 per vertex.
    pub positions: Vec<f32>,
    /// Unit normals, 3 per vertex.
    pub normals: Vec<f32>,
    /// UVs, 2 per vertex (v = 0 on the south edge).
    pub uvs: Vec<f32>,
    /// Triangle indices, counter-clockwise from +Y.
    pub indices: Vec<u32>,
    /// Grid size in vertices.
    pub grid: [u32; 2],
    /// Height range of the sampled vertices.
    pub height_range: [f32; 2],
}

impl GridMesh {
    /// Sample the field every `step` pixels (always including the last row/column).
    pub fn build(hf: &Heightfield, extent_m: [f64; 2], step: u32) -> JdResult<Self> {
        hf.validate_finite()?;
        if step == 0 {
            return Err(JdError::invalid("step", "must be ≥ 1"));
        }
        if !(extent_m[0] > 0.0 && extent_m[1] > 0.0) {
            return Err(JdError::invalid("extent_m", "must be positive"));
        }
        let (w, h) = (hf.width(), hf.height());
        let (gw, gh) = ((w - 1).div_ceil(step) + 1, (h - 1).div_ceil(step) + 1);
        if gw < 2 || gh < 2 {
            return Err(JdError::invalid(
                "step",
                "grid must keep at least 2×2 samples",
            ));
        }
        if (gw as u64) * (gh as u64) > u32::MAX as u64 {
            return Err(JdError::invalid(
                "step",
                "too many vertices for 32-bit indices",
            ));
        }
        let (sx, sz) = (extent_m[0] / w as f64, extent_m[1] / h as f64);
        let sample = |gx: u32, gz: u32| hf.get((gx * step).min(w - 1), (gz * step).min(h - 1));
        let xm = |gx: u32| ((gx * step).min(w - 1) as f64 + 0.5) * sx - extent_m[0] / 2.0;
        let zm = |gz: u32| extent_m[1] / 2.0 - ((gz * step).min(h - 1) as f64 + 0.5) * sz;
        let n = (gw * gh) as usize;
        let (mut positions, mut normals, mut uvs) = (
            Vec::with_capacity(n * 3),
            Vec::with_capacity(n * 3),
            Vec::with_capacity(n * 2),
        );
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for gz in 0..gh {
            for gx in 0..gw {
                let y = sample(gx, gz);
                lo = lo.min(y);
                hi = hi.max(y);
                positions.extend_from_slice(&[xm(gx) as f32, y, zm(gz) as f32]);
                let (x0, x1) = (gx.saturating_sub(1), (gx + 1).min(gw - 1));
                let (z0, z1) = (gz.saturating_sub(1), (gz + 1).min(gh - 1));
                let guard = |d: f64| if d.abs() < 1e-9 { 1e-9 } else { d };
                let dx = (sample(x1, gz) - sample(x0, gz)) as f64 / guard(xm(x1) - xm(x0));
                let dz = (sample(gx, z1) - sample(gx, z0)) as f64 / guard(zm(z1) - zm(z0));
                let len = (dx * dx + 1.0 + dz * dz).sqrt();
                normals.extend_from_slice(&[
                    (-dx / len) as f32,
                    (1.0 / len) as f32,
                    (-dz / len) as f32,
                ]);
                uvs.extend_from_slice(&[gx as f32 / (gw - 1) as f32, gz as f32 / (gh - 1) as f32]);
            }
        }
        let mut indices = Vec::with_capacity(((gw - 1) * (gh - 1) * 6) as usize);
        for gz in 0..gh - 1 {
            for gx in 0..gw - 1 {
                let a = gz * gw + gx;
                let (b, c, d) = (a + 1, a + gw, a + gw + 1);
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        Ok(Self {
            positions,
            normals,
            uvs,
            indices,
            grid: [gw, gh],
            height_range: [lo, hi],
        })
    }

    fn triangles(&self) -> u32 {
        (self.indices.len() / 3) as u32
    }
    fn vertices(&self) -> u32 {
        (self.positions.len() / 3) as u32
    }
}

fn report(
    path: &Path,
    format: &str,
    mesh: &GridMesh,
    step: u32,
    extent_m: [f64; 2],
) -> JdResult<MeshExportReport> {
    Ok(MeshExportReport {
        path: path.to_path_buf(),
        format: format.into(),
        vertices: mesh.vertices(),
        triangles: mesh.triangles(),
        step,
        bytes: fs::metadata(path)?.len(),
        extent_m,
        texture_px: None,
    })
}

/// Wavefront OBJ: positions, UVs, normals and triangles (1-based `v/vt/vn` faces), no MTL.
pub fn export_obj(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    let mut out = String::with_capacity(mesh.positions.len() * 12);
    out.push_str(&format!(
        "# jenga-dunia {} — metres, +Y up, +Z south, centred\no terrain\n",
        env!("CARGO_PKG_VERSION")
    ));
    for p in mesh.positions.as_chunks::<3>().0 {
        out.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
    }
    for t in mesh.uvs.as_chunks::<2>().0 {
        out.push_str(&format!("vt {} {}\n", t[0], t[1]));
    }
    for nr in mesh.normals.as_chunks::<3>().0 {
        out.push_str(&format!("vn {} {} {}\n", nr[0], nr[1], nr[2]));
    }
    for f in mesh.indices.as_chunks::<3>().0 {
        let (a, b, c) = (f[0] + 1, f[1] + 1, f[2] + 1);
        out.push_str(&format!("f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}\n"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, out)?;
    report(path, "obj", &mesh, step, extent_m)
}

/// Binary STL: 80-byte header, triangle count, per-triangle face normal + 3 vertices.
pub fn export_stl(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    let mut out = Vec::with_capacity(84 + mesh.indices.len() / 3 * 50);
    let mut header = format!(
        "jenga-dunia {} binary STL, metres, +Y up",
        env!("CARGO_PKG_VERSION")
    )
    .into_bytes();
    header.resize(80, b' ');
    out.extend_from_slice(&header);
    out.extend_from_slice(&mesh.triangles().to_le_bytes());
    let p = |i: u32| -> [f32; 3] {
        let i = i as usize * 3;
        [
            mesh.positions[i],
            mesh.positions[i + 1],
            mesh.positions[i + 2],
        ]
    };
    for f in mesh.indices.as_chunks::<3>().0 {
        let (a, b, c) = (p(f[0]), p(f[1]), p(f[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        for x in n {
            out.extend_from_slice(&(x / len).to_le_bytes());
        }
        for vtx in [a, b, c] {
            for x in vtx {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &out)?;
    report(path, "stl", &mesh, step, extent_m)
}

/// Binary little-endian PLY with per-vertex normals and hypsometric RGB colours.
pub fn export_ply(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    let header = format!(
        "ply\nformat binary_little_endian 1.0\ncomment jenga-dunia {} — metres, +Y up, +Z south, centred\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face {}\nproperty list uchar uint vertex_indices\nend_header\n",
        env!("CARGO_PKG_VERSION"),
        mesh.vertices(),
        mesh.triangles()
    );
    let mut out = header.into_bytes();
    let range = (mesh.height_range[1] - mesh.height_range[0]).max(1e-6);
    for (p, nr) in mesh
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .zip(mesh.normals.as_chunks::<3>().0)
    {
        for x in p.iter().chain(nr.iter()) {
            out.extend_from_slice(&x.to_le_bytes());
        }
        let t = (p[1] - mesh.height_range[0]) / range;
        let slope = ((1.0 - nr[1]) * 1.4).clamp(0.0, 1.0);
        out.extend_from_slice(&hypsometric_rgb(t, slope));
    }
    for f in mesh.indices.as_chunks::<3>().0 {
        out.push(3);
        for i in f {
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &out)?;
    report(path, "ply", &mesh, step, extent_m)
}

/// Export by extension: `.glb` (default, with optional texture), `.obj`, `.stl`, `.ply`.
pub fn export_mesh(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
) -> JdResult<MeshExportReport> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        None | Some("glb") => export_glb(hf, extent_m, path, step, texture),
        Some("obj") => export_obj(hf, extent_m, path, step),
        Some("stl") => export_stl(hf, extent_m, path, step),
        Some("ply") => export_ply(hf, extent_m, path, step),
        Some(other) => Err(JdError::new(
            jd_types::ErrorCode::E_FORMAT_UNSUPPORTED,
            format!("unsupported mesh extension `{other}`"),
        )
        .next("Use .glb, .obj, .stl or .ply.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(bytes: &[u8]) -> (serde_json::Value, &[u8]) {
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        let jlen = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + jlen]).expect("json chunk");
        let boff = 20 + jlen;
        let blen = u32::from_le_bytes(bytes[boff..boff + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[boff + 4..boff + 8], b"BIN\0");
        (json, &bytes[boff + 8..boff + 8 + blen])
    }

    #[test]
    fn glb_is_well_formed_and_faces_point_up() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(9, 7, 8, 0).expect("hf");
        hf.fill_with(|x, y| (x as f32) * 2.0 + (y as f32) * 0.5);
        let p = dir.path().join("t.glb");
        let r = export_glb(&hf, [90.0, 70.0], &p, 1, false).expect("export");
        assert_eq!((r.vertices, r.triangles, r.step), (63, 96, 1));
        let bytes = fs::read(&p).expect("read");
        assert_eq!(bytes.len() as u64, r.bytes);
        let (json, bin) = chunks(&bytes);
        assert_eq!(json["asset"]["version"], "2.0");
        assert_eq!(json["buffers"][0]["byteLength"], bin.len());
        assert_eq!(json["accessors"][0]["count"], 63);
        assert_eq!(json["accessors"][3]["count"], 96 * 3);
        // Positions are centred on the world: min/max symmetric in x and z.
        let (mn, mx) = (&json["accessors"][0]["min"], &json["accessors"][0]["max"]);
        assert!((mn[0].as_f64().unwrap() + mx[0].as_f64().unwrap()).abs() < 1e-3);
        assert!((mn[2].as_f64().unwrap() + mx[2].as_f64().unwrap()).abs() < 1e-3);
        // Every normal has a positive Y component (a gentle slope never flips a face).
        let nrm_off = json["bufferViews"][1]["byteOffset"].as_u64().unwrap() as usize;
        for v in 0..63 {
            let o = nrm_off + v * 12 + 4;
            let ny = f32::from_le_bytes(bin[o..o + 4].try_into().unwrap());
            assert!(ny > 0.9, "normal y {ny}");
        }
        // Triangle winding: the first triangle's geometric normal points up.
        let pos = |i: usize| -> [f32; 3] {
            let o = i * 12;
            [0, 4, 8].map(|k| f32::from_le_bytes(bin[o + k..o + k + 4].try_into().unwrap()))
        };
        let idx_off = json["bufferViews"][3]["byteOffset"].as_u64().unwrap() as usize;
        let ix = |k: usize| {
            u32::from_le_bytes(
                bin[idx_off + k * 4..idx_off + k * 4 + 4]
                    .try_into()
                    .unwrap(),
            ) as usize
        };
        let (a, b, c) = (pos(ix(0)), pos(ix(1)), pos(ix(2)));
        let (u, v) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let ny = u[2] * v[0] - u[0] * v[2];
        assert!(
            ny > 0.0,
            "first triangle must be counter-clockwise from above: {ny}"
        );
    }

    #[test]
    fn textured_export_embeds_a_png_that_follows_height_and_slope() {
        // Height rises gently west → east (1 m per 10 m cell, so slope barely tints the colour):
        // the west edge is lowland green, the east edge snow.
        let mut hf = Heightfield::new(33, 17, 64, 0).expect("hf");
        hf.fill_with(|x, _| x as f32);
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path().join("t.glb");
        let r = export_glb(&hf, [330.0, 170.0], &p, 1, true).expect("export");
        assert_eq!(r.texture_px, Some([33, 17]));
        let bytes = std::fs::read(&p).expect("read");
        let (json, bin) = chunks(&bytes);
        assert_eq!(json["images"][0]["mimeType"], "image/png");
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );
        let view = &json["bufferViews"][json["images"][0]["bufferView"].as_u64().unwrap() as usize];
        let (off, len) = (
            view["byteOffset"].as_u64().unwrap() as usize,
            view["byteLength"].as_u64().unwrap() as usize,
        );
        let img = image::load_from_memory(&bin[off..off + len])
            .expect("png")
            .into_rgb8();
        assert_eq!(img.dimensions(), (33, 17));
        let west = img.get_pixel(0, 8).0;
        let east = img.get_pixel(32, 8).0;
        assert!(
            west[1] > west[0] && west[1] > west[2],
            "lowland is green: {west:?}"
        );
        assert!(east.iter().all(|c| *c > 200), "summit is snow: {east:?}");
        // Validator-relevant: the image view is 4-byte aligned and inside the buffer.
        assert_eq!(off % 4, 0);
        assert!(off + len <= bin.len());
        // Steep ground reads as rock: a cliff colour sits between the ramp colour and ROCK.
        let flat = hypsometric_rgb(0.5, 0.0);
        let steep = hypsometric_rgb(0.5, 1.0);
        assert_ne!(flat, steep);
        assert_eq!(steep, [107, 102, 97]);
    }

    #[test]
    fn obj_stl_ply_share_the_glb_grid_and_have_the_right_sizes() {
        let mut hf = Heightfield::new(5, 4, 64, 0).expect("hf");
        hf.fill_with(|x, y| x as f32 * 2.0 + y as f32);
        let dir = tempfile::tempdir().expect("tmp");
        let glb = export_glb(&hf, [50.0, 40.0], &dir.path().join("g.glb"), 1, false).expect("glb");
        let obj = export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.obj"), 1, false).expect("obj");
        let stl = export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.stl"), 1, false).expect("stl");
        let ply = export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.ply"), 1, false).expect("ply");
        for r in [&obj, &stl, &ply] {
            assert_eq!((r.vertices, r.triangles), (glb.vertices, glb.triangles));
        }
        assert_eq!(
            (
                obj.format.as_str(),
                stl.format.as_str(),
                ply.format.as_str()
            ),
            ("obj", "stl", "ply")
        );
        let text = std::fs::read_to_string(dir.path().join("g.obj")).expect("obj");
        assert_eq!(text.lines().filter(|l| l.starts_with("v ")).count(), 20);
        assert_eq!(text.lines().filter(|l| l.starts_with("f ")).count(), 24);
        // First OBJ vertex is the GLB's first position (south-west corner, +Z/2 side).
        let glb_bytes = std::fs::read(dir.path().join("g.glb")).expect("glb");
        let (_, bin) = chunks(&glb_bytes);
        let gx = f32::from_le_bytes([bin[0], bin[1], bin[2], bin[3]]);
        let gz = f32::from_le_bytes([bin[8], bin[9], bin[10], bin[11]]);
        let first = text.lines().find(|l| l.starts_with("v ")).expect("v");
        let parts: Vec<f32> = first[2..]
            .split(' ')
            .map(|p| p.parse().expect("f32"))
            .collect();
        assert_eq!((parts[0], parts[2]), (gx, gz));
        assert!(gz > 0.0, "south row is at +Z");
        assert_eq!(stl.bytes, 84 + 24 * 50);
        let ply_bytes = std::fs::read(dir.path().join("g.ply")).expect("ply");
        let header_end = ply_bytes
            .windows(11)
            .position(|w| w == b"end_header\n")
            .expect("header")
            + 11;
        assert!(
            std::str::from_utf8(&ply_bytes[..header_end])
                .expect("utf8")
                .contains("element vertex 20\n")
        );
        assert_eq!(
            ply_bytes.len() - header_end,
            20 * (6 * 4 + 3) + 24 * (1 + 12)
        );
        assert!(export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.fbx"), 1, false).is_err());
    }

    #[test]
    fn south_row_lands_at_positive_z() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut hf = Heightfield::new(8, 8, 8, 0).expect("hf");
        hf.fill_with(|_, y| if y == 0 { 100.0 } else { 0.0 }); // row 0 = south is high
        let p = dir.path().join("o.glb");
        export_glb(&hf, [80.0, 80.0], &p, 1, false).expect("export");
        let bytes = fs::read(&p).expect("read");
        let (_, bin) = chunks(&bytes);
        let v = |i: usize| -> [f32; 3] {
            let o = i * 12;
            [0, 4, 8].map(|k| f32::from_le_bytes(bin[o + k..o + k + 4].try_into().unwrap()))
        };
        let [_, y0, z0] = v(0); // first vertex: row 0, column 0
        assert_eq!(y0, 100.0);
        assert!(z0 > 0.0, "south must be +Z like the viewport: {z0}");
        let [_, y7, z7] = v(7 * 8);
        assert!(y7 == 0.0 && z7 < 0.0);
    }

    #[test]
    fn step_decimates_and_auto_step_respects_the_budget() {
        // Budget = a 1025² grid (the Unity/Unreal 2ⁿ+1 size): 4096 and 4097 px both fit at step 4.
        assert_eq!(auto_step(4096, 4096, 1025 * 1025), 4);
        assert_eq!(auto_step(4097, 4097, 1025 * 1025), 4);
        assert_eq!(auto_step(4098, 4098, 1025 * 1025), 5);
        assert_eq!(auto_step(100, 100, 1025 * 1025), 1);
        let dir = tempfile::tempdir().expect("tmp");
        let hf = Heightfield::new(64, 64, 32, 4).expect("hf");
        let r =
            export_glb(&hf, [640.0, 640.0], &dir.path().join("s.glb"), 4, false).expect("export");
        assert_eq!(
            (r.vertices, r.triangles),
            (289, 512),
            "64 px at step 4 → 17 samples incl. the edge"
        );
        let (json, _) = chunks(&fs::read(dir.path().join("s.glb")).expect("read"));
        let mx = &json["accessors"][0]["max"];
        assert!(
            (mx[0].as_f64().unwrap() - 320.0 + 5.0).abs() < 1e-3,
            "last column sits on the east edge: {mx}"
        );
        assert!(export_glb(&hf, [640.0, 640.0], &dir.path().join("x.glb"), 0, false).is_err());
        // A huge step still keeps the first and last sample: a 2×2 grid, two triangles.
        let r =
            export_glb(&hf, [640.0, 640.0], &dir.path().join("y.glb"), 64, false).expect("export");
        assert_eq!((r.vertices, r.triangles), (4, 2));
    }
}
