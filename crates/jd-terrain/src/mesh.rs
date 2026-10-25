//! Terrain mesh export as binary glTF (`.glb`), written by hand: a regular grid with
//! positions (metres, +Y up, centred on the world like the viewport), normals, UVs, u32
//! indices and one PBR material. No dependency beyond `serde_json`; validated in CI with the
//! Khronos glTF validator (`apps/desktop/scripts/validate-glb.mjs`).
//!
//! With `texture` the material carries a baked base-colour PNG (the viewport's hypsometric ramp
//! blended towards rock on steep slopes), one texel per exported vertex, plus a tangent-space
//! normal map and a roughness map baked from the *full-resolution* field (capped at 2048²), so a
//! `--step`-decimated mesh keeps the fine relief and the file looks like the app in Blender /
//! three.js / engines without any material setup.
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
    /// Baked normal + roughness map size in pixels (full field resolution, capped at 2048²).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_px: Option<[u32; 2]>,
    /// Topology of the written triangles (None for FBX, which writes quads).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<MeshTopology>,
    /// Coarser levels written beside this file (`<stem>_lod1.<ext>`, …), each with about a
    /// quarter of the previous level's triangles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lods: Vec<MeshExportReport>,
}

/// Topology summary of an indexed triangle list — the checks a mesh importer trips over.
/// A closed-grid terrain has 0 degenerate, 0 non-manifold, 0 unreferenced and a perimeter of
/// boundary edges; decimation must keep it that way.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct MeshTopology {
    /// Triangles that repeat a vertex (zero area by construction).
    pub degenerate: u32,
    /// Edges used by exactly one triangle (the outline; holes add to it).
    pub boundary_edges: u32,
    /// Edges shared by more than two triangles.
    pub non_manifold_edges: u32,
    /// Vertices no triangle references.
    pub unreferenced_vertices: u32,
    /// Indices past the vertex count (a broken file; always 0 from our writers).
    pub out_of_range: u32,
}

/// Compute [`MeshTopology`] from a triangle iterator over `vertices` vertices.
pub fn mesh_topology(triangles: impl Iterator<Item = [u32; 3]>, vertices: usize) -> MeshTopology {
    let mut t = MeshTopology::default();
    let mut edges: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let mut used = vec![false; vertices];
    for [a, b, c] in triangles {
        if a == b || b == c || a == c {
            t.degenerate += 1;
        }
        for v in [a, b, c] {
            match used.get_mut(v as usize) {
                Some(u) => *u = true,
                None => t.out_of_range += 1,
            }
        }
        for (u, v) in [(a, b), (b, c), (c, a)] {
            if u != v {
                *edges.entry((u.min(v), u.max(v))).or_insert(0) += 1;
            }
        }
    }
    for n in edges.values() {
        if *n == 1 {
            t.boundary_edges += 1;
        } else if *n > 2 {
            t.non_manifold_edges += 1;
        }
    }
    t.unreferenced_vertices = used.iter().filter(|u| !**u).count() as u32;
    t
}

/// Topology of the regular grid the GLB writer emits (two triangles per cell).
fn grid_topology(gw: u32, gh: u32) -> MeshTopology {
    // Closed form: a regular grid has no degenerate, non-manifold or unused element, only its
    // perimeter — counting 6 M edges in a HashMap per export told us nothing new.
    MeshTopology {
        boundary_edges: 2 * (gw - 1) + 2 * (gh - 1),
        ..MeshTopology::default()
    }
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn bake_albedo(
    sample: &dyn Fn(u32, u32) -> f32,
    normal_y: &dyn Fn(u32, u32) -> f32,
    gw: u32,
    gh: u32,
    min_h: f32,
    max_h: f32,
    layers: &[MaterialLayer],
    cell: &dyn Fn(u32, u32) -> (u32, u32),
    world_m: &dyn Fn(u32, u32) -> (f64, f64),
) -> JdResult<Vec<u8>> {
    let range = (max_h - min_h).max(1e-6);
    let mut px = Vec::with_capacity((gw * gh * 3) as usize);
    for gz in 0..gh {
        for gx in 0..gw {
            let t = (sample(gx, gz) - min_h) / range;
            let slope = ((1.0 - normal_y(gx, gz)) * 1.4).clamp(0.0, 1.0);
            let base = hypsometric_rgb(t, slope);
            if layers.is_empty() {
                px.extend_from_slice(&base);
                continue;
            }
            // The material stack: every layer lerps towards its colour by its mask at this texel's cell.
            let (cx, cz) = cell(gx, gz);
            let mut c = [
                base[0] as f32 / 255.0,
                base[1] as f32 / 255.0,
                base[2] as f32 / 255.0,
            ];
            for l in layers {
                let wgt = l
                    .mask
                    .as_ref()
                    .map_or(1.0, |m| m.get(cx, cz).clamp(0.0, 1.0));
                for (ck, lk) in c.iter_mut().zip(&l.albedo) {
                    *ck += (lk - *ck) * wgt;
                }
                // Texture set: the image's texel at this world position stands in for the flat colour.
                if let Some(t) = &l.texture {
                    let (xm, zm) = world_m(gx, gz);
                    for (ck, tk) in c.iter_mut().zip(&t.albedo_at(xm, zm)) {
                        *ck += (tk - *ck) * wgt;
                    }
                }
                // Detail: the tile's relief at this texel's world position darkens and
                // lightens the colour, as it does in the exported tile.
                if let Some(d) = &l.detail {
                    let (xm, zm) = world_m(gx, gz);
                    let (r, _, _) = crate::detail::relief_at_world(d, xm, zm);
                    let f = 1.0 + wgt * d.contrast * (r - 0.5) * 2.0;
                    for ck in c.iter_mut() {
                        *ck = (*ck * f).clamp(0.0, 1.0);
                    }
                }
            }
            px.extend(
                c.iter()
                    .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8),
            );
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

/// Largest texture side for the baked normal / roughness maps in mesh exports.
pub const NORMAL_MAP_CAP: u32 = 2048;

/// Full-field material maps as PNG bytes: albedo (hypsometric ramp + slope), tangent-space
/// normal, roughness; all `px` wide/high, row 0 = south.
pub struct BakedMaps {
    /// Base colour PNG.
    pub albedo: Vec<u8>,
    /// Tangent-space normal map PNG.
    pub normal: Vec<u8>,
    /// Roughness (G) / metallic (B = 0) PNG.
    pub roughness: Vec<u8>,
    /// Emissive PNG, only when a layer emits (black elsewhere).
    pub emissive: Option<Vec<u8>>,
    /// Texture size in pixels.
    pub px: [u32; 2],
    /// Detail tiles of the layers that have one, with the layer's index in the stack.
    pub details: Vec<(usize, crate::detail::DetailTile)>,
    /// The masks of the first four detail layers packed into an RGBA PNG at the map size
    /// (row 0 = south), for a viewport that blends the tiles itself; `None` without details.
    pub detail_weights: Option<Vec<u8>>,
}

/// A material layer blended over the base maps by a 0..1 mask (the material graph's layer
/// stack: biome class masks, snow, wetness, painted masks). Layers apply in order; a missing
/// mask means "everywhere".
#[derive(Debug, Clone)]
pub struct MaterialLayer {
    /// Weight field at the terrain's resolution (None = 1 everywhere).
    pub mask: Option<Heightfield>,
    /// Base colour, sRGB 0..1.
    pub albedo: [f32; 3],
    /// Roughness 0..1.
    pub roughness: f32,
    /// Emitted light, linear RGB 0..1 (black = none): molten lava, embers, city glow.
    pub emissive: [f32; 3],
    /// Tileable detail (grain repeated every `tile_m`): baked into the macro maps by world
    /// position and exported as tiles for the engines.
    pub detail: Option<crate::detail::Detail>,
    /// An image texture set (albedo + optional normal, repeated every `tile_m`): the albedo is
    /// sampled into the macro maps by world position and the set is exported as the layer's tile.
    pub texture: Option<std::sync::Arc<crate::detail::TextureSet>>,
}

/// Bake albedo, normal and roughness maps from the full field, sampled every `ts` cells so no
/// side exceeds `max_px` (see [`bake_normal_roughness_with`] for the conventions).
pub fn bake_maps(hf: &Heightfield, extent_m: [f64; 2], max_px: u32) -> JdResult<BakedMaps> {
    bake_maps_with(hf, extent_m, max_px, &[])
}

/// [`bake_maps`] with material layers: each layer lerps the albedo and roughness towards its own
/// by its mask's weight at every texel, on top of the hypsometric base.
pub fn bake_maps_with(
    hf: &Heightfield,
    extent_m: [f64; 2],
    max_px: u32,
    layers: &[MaterialLayer],
) -> JdResult<BakedMaps> {
    validate_layers(hf, layers)?;
    bake_maps_body(hf, extent_m, max_px, layers)
}

/// Every material mask matches the terrain and every colour / roughness sits in 0..=1 — checked
/// once here for the baked-maps path and the direct GLB path alike.
fn validate_layers(hf: &Heightfield, layers: &[MaterialLayer]) -> JdResult<()> {
    for (k, l) in layers.iter().enumerate() {
        if let Some(m) = &l.mask
            && (m.width(), m.height()) != (hf.width(), hf.height())
        {
            return Err(JdError::invalid(
                "materials",
                format!("layer {k}: mask size differs from the terrain"),
            ));
        }
        if l.albedo.iter().any(|c| !(0.0..=1.0).contains(c))
            || !(0.0..=1.0).contains(&l.roughness)
            || l.emissive.iter().any(|c| !(0.0..=1.0).contains(c))
        {
            return Err(JdError::invalid(
                "materials",
                format!("layer {k}: albedo, roughness and emissive must be within 0..=1"),
            ));
        }
        if let Some(d) = &l.detail {
            d.validate(k)?;
        }
        if let Some(t) = &l.texture {
            t.validate()?;
        }
    }
    Ok(())
}

fn bake_maps_body(
    hf: &Heightfield,
    extent_m: [f64; 2],
    max_px: u32,
    layers: &[MaterialLayer],
) -> JdResult<BakedMaps> {
    let (w, h) = (hf.width(), hf.height());
    let ts = w.max(h).div_ceil(max_px.max(16)).max(1);
    let (tw, th) = (w.div_ceil(ts), h.div_ceil(ts));
    let (sx, sz) = (extent_m[0] / w as f64, extent_m[1] / h as f64);
    let stats = hf.stats();
    let at = |x: u32, z: u32| hf.get((x * ts).min(w - 1), (z * ts).min(h - 1));
    let normal_y = |x: u32, z: u32| {
        let (xi, zi) = (((x * ts).min(w - 1)) as i64, ((z * ts).min(h - 1)) as i64);
        let d = ts as i64;
        let g = |xx: i64, zz: i64| {
            hf.get(
                xx.clamp(0, w as i64 - 1) as u32,
                zz.clamp(0, h as i64 - 1) as u32,
            ) as f64
        };
        let dx = (g(xi + d, zi) - g(xi - d, zi)) / (2.0 * d as f64 * sx);
        let dz = (g(xi, zi - d) - g(xi, zi + d)) / (2.0 * d as f64 * sz);
        (1.0 / (dx * dx + 1.0 + dz * dz).sqrt()) as f32
    };
    let albedo = bake_albedo(
        &at,
        &normal_y,
        tw,
        th,
        stats.min,
        stats.max,
        layers,
        &|gx, gz| ((gx * ts).min(w - 1), (gz * ts).min(h - 1)),
        &|gx, gz| ((gx * ts) as f64 * sx, (gz * ts) as f64 * sz),
    )?;
    let (normal, roughness, px) = bake_normal_roughness_with(hf, extent_m, max_px, layers)?;
    let emissive = bake_emissive(layers, tw, th, &|gx, gz| {
        ((gx * ts).min(w - 1), (gz * ts).min(h - 1))
    })?;
    let mut details = Vec::new();
    for (k, l) in layers.iter().enumerate() {
        if let Some(d) = &l.detail {
            details.push((k, crate::detail::bake_detail(d, l.albedo)?));
        } else if let Some(t) = &l.texture {
            details.push((k, t.tile()?));
        }
    }
    let detail_weights = if details.is_empty() {
        None
    } else {
        // RGBA = the first four detail layers' masks at the map's texels.
        let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
        for gz in 0..th {
            for gx in 0..tw {
                let (cx, cz) = ((gx * ts).min(w - 1), (gz * ts).min(h - 1));
                for ch in 0..4 {
                    let v = details.get(ch).map_or(0.0, |(k, _)| {
                        layers[*k]
                            .mask
                            .as_ref()
                            .map_or(1.0, |m| m.get(cx, cz).clamp(0.0, 1.0))
                    });
                    rgba.push((v * 255.0).round() as u8);
                }
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(
            &mut out,
            &rgba,
            tw,
            th,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| {
            JdError::new(
                jd_types::ErrorCode::E_INTERNAL,
                format!("PNG encode failed: {e}"),
            )
        })?;
        Some(out.into_inner())
    };
    Ok(BakedMaps {
        albedo,
        normal,
        roughness,
        emissive,
        px,
        details,
        detail_weights,
    })
}

/// Emission map: black, then each emitting layer lerps its colour in by its mask — the same
/// blend order as the albedo, so a glow sits exactly where its material does. `None` when no
/// layer emits. `cell` maps a texel to the field cell it samples.
pub(crate) fn bake_emissive(
    layers: &[MaterialLayer],
    tw: u32,
    th: u32,
    cell: &dyn Fn(u32, u32) -> (u32, u32),
) -> JdResult<Option<Vec<u8>>> {
    if !layers.iter().any(|l| l.emissive.iter().any(|c| *c > 0.0)) {
        return Ok(None);
    }
    let mut rgb = Vec::with_capacity((tw * th * 3) as usize);
    for gz in 0..th {
        for gx in 0..tw {
            let (cx, cz) = cell(gx, gz);
            let mut e = [0.0f32; 3];
            for l in layers {
                let wgt = l
                    .mask
                    .as_ref()
                    .map_or(1.0, |m| m.get(cx, cz).clamp(0.0, 1.0));
                for (k, ec) in e.iter_mut().enumerate() {
                    *ec += (l.emissive[k] - *ec) * wgt;
                }
            }
            rgb.extend(
                e.iter()
                    .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8),
            );
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut out,
        &rgb,
        tw,
        th,
        image::ExtendedColorType::Rgb8,
        image::ImageFormat::Png,
    )
    .map_err(|e| {
        JdError::new(
            jd_types::ErrorCode::E_INTERNAL,
            format!("PNG encode failed: {e}"),
        )
    })?;
    Ok(Some(out.into_inner()))
}

/// Bake a tangent-space normal map (glTF convention: +X = +U east, +Y = +V north, +Z out of
/// the surface) and a roughness map (G channel; B metallic = 0) from the full field, sampled
/// every `ts` cells so no side exceeds `max_px`. Row 0 = south, like the albedo.
/// Roughness: 0.92 on flat ground, down to 0.62 on steep rock, 0.6 above the snow line.
/// How far the baked ambient occlusion looks for cover, in cells of the source field.
pub(crate) const AO_REACH_CELLS: f32 = 16.0;

pub(crate) fn bake_normal_roughness_with(
    hf: &Heightfield,
    extent_m: [f64; 2],
    max_px: u32,
    layers: &[MaterialLayer],
) -> JdResult<(Vec<u8>, Vec<u8>, [u32; 2])> {
    let (w, h) = (hf.width(), hf.height());
    let ts = w.max(h).div_ceil(max_px.max(16)).max(1);
    let (tw, th) = (w.div_ceil(ts), h.div_ceil(ts));
    let (sx, sz) = (extent_m[0] / w as f64, extent_m[1] / h as f64);
    let stats = hf.stats();
    let range = (stats.max - stats.min).max(1e-6);
    let at = |x: i64, z: i64| {
        hf.get(
            x.clamp(0, w as i64 - 1) as u32,
            z.clamp(0, h as i64 - 1) as u32,
        ) as f64
    };
    let mut nrm = Vec::with_capacity((tw * th * 3) as usize);
    let mut rough = Vec::with_capacity((tw * th * 3) as usize);
    // Ambient occlusion = 1 − sky occlusion within 16 cells, in R: glTF's occlusion / roughness /
    // metallic packing, so the one texture serves as occlusionTexture and metallicRoughnessTexture.
    let cell = (sx.max(sz)) as f32;
    let occ = crate::occlusion_mask(hf, [sx as f32, sz as f32], AO_REACH_CELLS * cell)?;
    for ty in 0..th {
        for tx in 0..tw {
            let (x, z) = ((tx * ts).min(w - 1) as i64, (ty * ts).min(h - 1) as i64);
            let d = ts as i64;
            let mut dh_dx = (at(x + d, z) - at(x - d, z)) / (2.0 * d as f64 * sx);
            // Row index grows northwards (−Z): the derivative along +Z (south) is the reverse.
            let mut dh_dz = (at(x, z - d) - at(x, z + d)) / (2.0 * d as f64 * sz);
            // Detail: each layer's tile relief adds its slope here, weighted by the mask — the
            // same `bump × 0.25 × ∂relief` the exported tile encodes.
            for l in layers {
                if let Some(dt) = &l.detail {
                    let wgt = l
                        .mask
                        .as_ref()
                        .map_or(1.0, |m| m.get(x as u32, z as u32).clamp(0.0, 1.0));
                    let (_, du, dv) =
                        crate::detail::relief_at_world(dt, x as f64 * sx, z as f64 * sz);
                    dh_dx += f64::from(wgt * dt.bump * 0.25 * du);
                    dh_dz -= f64::from(wgt * dt.bump * 0.25 * dv);
                }
            }
            let len = (dh_dx * dh_dx + 1.0 + dh_dz * dh_dz).sqrt();
            let (nx, ny, nz) = (-dh_dx / len, 1.0 / len, -dh_dz / len);
            // Tangent frame: T = +X, B = +V = north = −Z, N = +Y.
            for c in [nx, -nz, ny] {
                nrm.push(((c * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            let slope = ((1.0 - ny) * 1.4).clamp(0.0, 1.0);
            let t = (at(x, z) - stats.min as f64) / range as f64;
            let mut r = if t > 0.85 { 0.6 } else { 0.92 - 0.3 * slope };
            for l in layers {
                let wgt = l
                    .mask
                    .as_ref()
                    .map_or(1.0, |m| m.get(x as u32, z as u32).clamp(0.0, 1.0));
                r += (f64::from(l.roughness) - r) * f64::from(wgt);
            }
            let ao = 1.0 - occ.get(x as u32, z as u32).clamp(0.0, 1.0);
            rough.extend_from_slice(&[(ao * 255.0).round() as u8, (r * 255.0).round() as u8, 0]);
        }
    }
    let png = |px: &[u8]| -> JdResult<Vec<u8>> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(
            &mut out,
            px,
            tw,
            th,
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
    };
    Ok((png(&nrm)?, png(&rough)?, [tw, th]))
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

pub(crate) fn pad4(v: &mut Vec<u8>, byte: u8) {
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
    export_glb_with(hf, extent_m, path, step, texture, &[])
}

/// [`export_glb`] with material layers baked into the albedo and roughness textures.
pub fn export_glb_with(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    materials: &[MaterialLayer],
) -> JdResult<MeshExportReport> {
    export_glb_with_holes(hf, extent_m, path, step, texture, materials, None)
}

/// [`export_glb_with`] leaving out the quads a 0..1 `holes` mask marks (≥ 0.5).
#[allow(clippy::too_many_arguments)]
pub fn export_glb_with_holes(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    materials: &[MaterialLayer],
    holes: Option<&Heightfield>,
) -> JdResult<MeshExportReport> {
    if let Some(m) = holes
        && (m.width(), m.height()) != (hf.width(), hf.height())
    {
        return Err(JdError::invalid(
            "holes",
            "the hole mask must match the terrain's size",
        ));
    }
    hf.validate_finite()?;
    validate_layers(hf, materials)?;
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
    // Tangents (textured only — the normal map needs a tangent space): the surface's ∂P/∂u, which
    // is what MikkTSpace computes. U runs east, so on a heightfield y = h(x, z) the tangent is
    // (1, ∂h/∂x, 0) with ∂h/∂x = −n.x / n.y: in the tangent plane and, unlike +X projected onto
    // the plane, without a z component (that projection was off by up to 51° on cross-slopes
    // against Blender's calc_tangents). w = +1: normal × tangent = −Z = +V (north).
    let tan_off = bin.len();
    if texture {
        for v in 0..n_vertices as usize {
            let o = pos_len + v * 12;
            let n: Vec<f32> = (0..3)
                .map(|k| {
                    f32::from_le_bytes(bin[o + 4 * k..o + 4 * k + 4].try_into().unwrap_or([0; 4]))
                })
                .collect();
            let t = tangent_of(&[n[0], n[1], n[2]]);
            for c in [t[0], t[1], t[2], 1.0] {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    let tan_len = bin.len() - tan_off;
    let idx_off = bin.len();
    let mut n_triangles = 0u32;
    for gz in 0..gh - 1 {
        for gx in 0..gw - 1 {
            if !quad_kept(holes, step, gx, gz) {
                continue;
            }
            let a = gz * gw + gx;
            let b = a + 1;
            let c = a + gw;
            let d = c + 1;
            // Row c is north of row a (−Z): (a, b, c) and (b, d, c) are counter-clockwise from +Y.
            for i in [a, b, c, b, d, c] {
                bin.extend_from_slice(&i.to_le_bytes());
            }
            n_triangles += 2;
        }
    }
    let idx_len = bin.len() - idx_off;
    pad4(&mut bin, 0);
    // Optional baked textures: albedo (one texel per vertex), normal + roughness maps (full field
    // resolution, capped), each in its own buffer view; UVs already map v = 0 to south.
    let mut image_views = Vec::new();
    let mut normal_px = None;
    if texture {
        let (fw, fh) = (hf.width(), hf.height());
        let albedo = bake_albedo(
            &sample,
            &|gx, gz| normal_y[(gz * gw + gx) as usize],
            gw,
            gh,
            min[1],
            max[1],
            materials,
            &|gx, gz| ((gx * step).min(fw - 1), (gz * step).min(fh - 1)),
            &|gx, gz| {
                (
                    (gx * step) as f64 * extent_m[0] / fw as f64,
                    (gz * step) as f64 * extent_m[1] / fh as f64,
                )
            },
        )?;
        let (normal_png, rough_png, px) =
            bake_normal_roughness_with(hf, extent_m, NORMAL_MAP_CAP, materials)?;
        normal_px = Some(px);
        // The albedo is one texel per exported vertex: the emissive map matches it.
        let emissive = bake_emissive(materials, gw, gh, &|gx, gz| {
            ((gx * step).min(fw - 1), (gz * step).min(fh - 1))
        })?;
        let mut pngs = vec![albedo, normal_png, rough_png];
        if let Some(e) = emissive {
            pngs.push(e);
        }
        for png in pngs {
            let off = bin.len();
            bin.extend_from_slice(&png);
            image_views.push((off, png.len()));
            pad4(&mut bin, 0);
        }
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
    if !image_views.is_empty() {
        let views = json["bufferViews"]
            .as_array_mut()
            .ok_or_else(|| JdError::new(jd_types::ErrorCode::E_INTERNAL, "bufferViews"))?;
        views.push(serde_json::json!({ "buffer": 0, "byteOffset": tan_off, "byteLength": tan_len, "target": 34962 }));
        let tangent_view = views.len() - 1;
        let first = views.len();
        let names = [
            "terrain-albedo",
            "terrain-normal",
            "terrain-roughness",
            "terrain-emissive",
        ];
        let mut images = Vec::new();
        for (k, (off, len)) in image_views.iter().enumerate() {
            views.push(serde_json::json!({ "buffer": 0, "byteOffset": off, "byteLength": len }));
            images.push(serde_json::json!({ "bufferView": first + k, "mimeType": "image/png", "name": names[k] }));
        }
        json["images"] = serde_json::Value::Array(images);
        json["samplers"] = serde_json::json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }]);
        json["textures"] = serde_json::Value::Array(
            (0..image_views.len())
                .map(|k| serde_json::json!({ "sampler": 0, "source": k }))
                .collect(),
        );
        let accessors = json["accessors"]
            .as_array_mut()
            .ok_or_else(|| JdError::new(jd_types::ErrorCode::E_INTERNAL, "accessors"))?;
        accessors.push(serde_json::json!({ "bufferView": tangent_view, "componentType": 5126, "count": n_vertices, "type": "VEC4" }));
        json["meshes"][0]["primitives"][0]["attributes"]["TANGENT"] =
            serde_json::json!(accessors.len() - 1);
        let m = &mut json["materials"][0];
        m["pbrMetallicRoughness"]["baseColorTexture"] =
            serde_json::json!({ "index": 0, "texCoord": 0 });
        m["pbrMetallicRoughness"]["baseColorFactor"] = serde_json::json!([1.0, 1.0, 1.0, 1.0]);
        m["pbrMetallicRoughness"]["metallicRoughnessTexture"] =
            serde_json::json!({ "index": 2, "texCoord": 0 });
        m["pbrMetallicRoughness"]["roughnessFactor"] = serde_json::json!(1.0);
        m["normalTexture"] = serde_json::json!({ "index": 1, "texCoord": 0 });
        m["occlusionTexture"] = serde_json::json!({ "index": 2, "texCoord": 0 }); // AO in R of the ORM map
        if image_views.len() > 3 {
            m["emissiveTexture"] = serde_json::json!({ "index": 3, "texCoord": 0 });
            m["emissiveFactor"] = serde_json::json!([1.0, 1.0, 1.0]);
        }
    }
    let bytes = write_glb(path, &json, &bin)?;
    Ok(MeshExportReport {
        lods: Vec::new(),
        path: path.to_path_buf(),
        format: "glb".into(),
        vertices: n_vertices as u32,
        triangles: n_triangles,
        step,
        bytes,
        extent_m,
        texture_px: texture.then_some([gw, gh]),
        normal_px,
        topology: Some(grid_topology(gw, gh)),
    })
}

/// Write a two-chunk GLB container (JSON + BIN, both 4-byte padded); returns the bytes written.
pub(crate) fn write_glb(path: &Path, json: &serde_json::Value, bin: &[u8]) -> JdResult<u64> {
    let mut json_bytes = serde_json::to_vec(json)?;
    pad4(&mut json_bytes, b' ');
    let mut bin = bin.to_vec();
    pad4(&mut bin, 0);
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
    Ok(out.len() as u64)
}

/// Whether the quad whose south-west vertex is grid cell (`gx`, `gz`) is kept: without a hole
/// mask always; with one, when the mask is below 0.5 at that cell (one cell = one quad, as Unity's
/// holes texture and Unreal's visibility layer read it).
pub(crate) fn quad_kept(holes: Option<&Heightfield>, step: u32, gx: u32, gz: u32) -> bool {
    holes.is_none_or(|m| {
        m.get(
            (gx * step).min(m.width() - 1),
            (gz * step).min(m.height() - 1),
        ) < 0.5
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
    /// Topology summary of this mesh's triangles.
    pub fn topology(&self) -> MeshTopology {
        mesh_topology(
            self.indices.as_chunks::<3>().0.iter().copied(),
            self.vertices() as usize,
        )
    }
    /// Sample the field every `step` pixels (always including the last row/column).
    pub fn build(hf: &Heightfield, extent_m: [f64; 2], step: u32) -> JdResult<Self> {
        Self::build_with(hf, extent_m, step, None)
    }

    /// [`GridMesh::build`] with an optional 0..1 hole mask (same size as `hf`): quads whose cell
    /// reads ≥ 0.5 are left out, so caves, tunnel mouths and building footprints open the mesh.
    pub fn build_with(
        hf: &Heightfield,
        extent_m: [f64; 2],
        step: u32,
        holes: Option<&Heightfield>,
    ) -> JdResult<Self> {
        hf.validate_finite()?;
        if let Some(m) = holes
            && (m.width(), m.height()) != (hf.width(), hf.height())
        {
            return Err(JdError::invalid(
                "holes",
                "the hole mask must match the terrain's size",
            ));
        }
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
                if !quad_kept(holes, step, gx, gz) {
                    continue;
                }
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
        lods: Vec::new(),
        path: path.to_path_buf(),
        format: format.into(),
        vertices: mesh.vertices(),
        triangles: mesh.triangles(),
        step,
        bytes: fs::metadata(path)?.len(),
        extent_m,
        texture_px: None,
        normal_px: None,
        topology: Some(mesh.topology()),
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
    write_obj(&mesh, extent_m, path, step)
}

/// Write an already built (or decimated) mesh as OBJ.
pub fn write_obj(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
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
    report(path, "obj", mesh, step, extent_m)
}

/// Binary STL: 80-byte header, triangle count, per-triangle face normal + 3 vertices.
pub fn export_stl(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    write_stl(&mesh, extent_m, path, step)
}

/// Write an already built (or decimated) mesh as STL.
pub fn write_stl(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
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
    report(path, "stl", mesh, step, extent_m)
}

/// Binary little-endian PLY with per-vertex normals and hypsometric RGB colours.
pub fn export_ply(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    write_ply(&mesh, extent_m, path, step)
}

/// Write an already built (or decimated) mesh as PLY.
pub fn write_ply(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
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
    report(path, "ply", mesh, step, extent_m)
}

/// USDA (ASCII OpenUSD): one `Xform` with a `Mesh` — points, triangles, vertex normals,
/// `primvars:st`, hypsometric `primvars:displayColor`, extent; `metersPerUnit = 1`, `upAxis = "Y"`.
/// Validate with `usdchecker`; for large worlds prefer `.glb`/`.ply` (USDA is text).
pub fn export_usda(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    write_usda(&mesh, extent_m, path, step)
}

/// Write an already built (or decimated) mesh as USDA.
pub fn write_usda(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let n = mesh.vertices() as usize;
    let mut out = String::with_capacity(n * 90);
    out.push_str(&format!(
        "#usda 1.0\n(\n    defaultPrim = \"Terrain\"\n    metersPerUnit = 1\n    upAxis = \"Y\"\n    doc = \"jenga-dunia {} — terrain grid, metres, +Y up, +Z south, centred on the world\"\n)\n\ndef Xform \"Terrain\" (\n    kind = \"component\"\n)\n{{\n    def Mesh \"Grid\"\n    {{\n",
        env!("CARGO_PKG_VERSION")
    ));
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in mesh.positions.as_chunks::<3>().0 {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    out.push_str(&format!(
        "        float3[] extent = [({}, {}, {}), ({}, {}, {})]\n",
        lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]
    ));
    out.push_str("        uniform token subdivisionScheme = \"none\"\n");
    out.push_str(&format!(
        "        int[] faceVertexCounts = [{}]\n",
        vec!["3"; mesh.triangles() as usize].join(", ")
    ));
    let idx: Vec<String> = mesh.indices.iter().map(|i| i.to_string()).collect();
    out.push_str(&format!(
        "        int[] faceVertexIndices = [{}]\n",
        idx.join(", ")
    ));
    let pts: Vec<String> = mesh
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| format!("({}, {}, {})", p[0], p[1], p[2]))
        .collect();
    out.push_str(&format!(
        "        point3f[] points = [{}]\n",
        pts.join(", ")
    ));
    let nrm: Vec<String> = mesh
        .normals
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| format!("({}, {}, {})", p[0], p[1], p[2]))
        .collect();
    out.push_str(&format!(
        "        normal3f[] normals = [{}] (\n            interpolation = \"vertex\"\n        )\n",
        nrm.join(", ")
    ));
    let st: Vec<String> = mesh
        .uvs
        .as_chunks::<2>()
        .0
        .iter()
        .map(|t| format!("({}, {})", t[0], t[1]))
        .collect();
    out.push_str(&format!("        texCoord2f[] primvars:st = [{}] (\n            interpolation = \"vertex\"\n        )\n", st.join(", ")));
    let range = (mesh.height_range[1] - mesh.height_range[0]).max(1e-6);
    let col: Vec<String> = mesh
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .zip(mesh.normals.as_chunks::<3>().0)
        .map(|(p, nr)| {
            let c = hypsometric_rgb(
                (p[1] - mesh.height_range[0]) / range,
                ((1.0 - nr[1]) * 1.4).clamp(0.0, 1.0),
            );
            format!(
                "({:.4}, {:.4}, {:.4})",
                c[0] as f32 / 255.0,
                c[1] as f32 / 255.0,
                c[2] as f32 / 255.0
            )
        })
        .collect();
    out.push_str(&format!("        color3f[] primvars:displayColor = [{}] (\n            interpolation = \"vertex\"\n        )\n", col.join(", ")));
    out.push_str("    }\n}\n");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, out)?;
    report(path, "usda", mesh, step, extent_m)
}

/// USDZ: the USDA layer packaged with the system `usdzip` (Apple USD Tools / OpenUSD), which
/// handles the 64-byte alignment rules. `E_DEPENDENCY_MISSING` when no `usdzip` is on PATH.
pub fn export_usdz(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    write_usdz(&mesh, extent_m, path, step)
}

/// USDC: the USDA layer converted to the binary crate format by the system `usdcat`
/// (Apple USD Tools / OpenUSD) — the same layer, a fraction of the bytes, no text parsing on
/// load. `E_DEPENDENCY_MISSING` when no `usdcat` is on PATH.
pub fn export_usdc(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    let mesh = GridMesh::build(hf, extent_m, step)?;
    write_usdc(&mesh, extent_m, path, step)
}

/// Write an already built (or decimated) mesh as USDC via `usdcat`.
pub fn write_usdc(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    // A private intermediate layer (`x.usdc.usda`) so a sibling `x.usda` export survives.
    let usda = path.with_extension("usdc.usda");
    let mut rep = write_usda(mesh, extent_m, &usda, step)?;
    let status = std::process::Command::new("usdcat")
        .arg(&usda)
        .arg("-o")
        .arg(path)
        .output();
    let _ = fs::remove_file(&usda);
    let status = status.map_err(|e| {
            JdError::new(jd_types::ErrorCode::E_DEPENDENCY_MISSING, format!("usdcat not available: {e}"))
                .next("Install OpenUSD tools (macOS 27 ships them in /usr/bin; `pip install usd-core` elsewhere) or export .usda.")
        })?;
    if !status.status.success() {
        return Err(JdError::new(
            jd_types::ErrorCode::E_SERIALIZE,
            format!(
                "usdcat failed: {}",
                String::from_utf8_lossy(&status.stderr).trim()
            ),
        )
        .detail("path", path.display().to_string()));
    }
    rep.path = path.to_path_buf();
    rep.format = "usdc".into();
    rep.bytes = fs::metadata(path)?.len();
    Ok(rep)
}

/// Write an already built (or decimated) mesh as USDZ via `usdzip`.
pub fn write_usdz(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
) -> JdResult<MeshExportReport> {
    // The layer is written as `<stem>.usda` inside a private directory next to the output (so
    // a sibling `.usda` export is never clobbered), usdzip runs in that directory with the bare
    // name — it stores the argument as the entry name, and `tmp/…/terrain.usda` is what a path
    // produced — and the directory goes whatever usdzip did.
    let stem = path
        .file_stem()
        .ok_or_else(|| JdError::invalid("path", "needs a file name"))?
        .to_owned();
    let out_abs = std::path::absolute(path)?;
    let dir = out_abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!(".jd-usdz-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let name = format!("{}.usda", stem.to_string_lossy());
    let usda = dir.join(&name);
    let run = || -> JdResult<MeshExportReport> {
        let mut rep = write_usda(mesh, extent_m, &usda, step)?;
        let status = std::process::Command::new("usdzip")
            .current_dir(&dir)
            .arg(&out_abs)
            .arg(&name)
            .output()
            .map_err(|e| {
                JdError::new(jd_types::ErrorCode::E_DEPENDENCY_MISSING, format!("usdzip not available: {e}"))
                    .next("Install OpenUSD tools (macOS 27 ships them in /usr/bin; `pip install usd-core` elsewhere) or export .usda and package it yourself.")
            })?;
        if !status.status.success() {
            return Err(JdError::new(
                jd_types::ErrorCode::E_SERIALIZE,
                format!(
                    "usdzip failed: {}",
                    String::from_utf8_lossy(&status.stderr).trim()
                ),
            ));
        }
        rep.path = path.to_path_buf();
        rep.format = "usdz".into();
        rep.bytes = fs::metadata(path)?.len();
        Ok(rep)
    };
    let result = run();
    let _ = fs::remove_dir_all(&dir);
    result
}

/// Write an irregular (decimated) mesh as `.glb`: positions, normals, planar UVs, tangents and
/// the full-field material maps when `maps` is given.
pub fn write_glb_mesh(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    maps: Option<&BakedMaps>,
) -> JdResult<MeshExportReport> {
    write_glb_mesh_named(mesh, extent_m, path, step, maps, "terrain")
}

/// [`write_glb_mesh`] with the mesh, node and material named `name` (a cave, a rock).
pub fn write_glb_mesh_named(
    mesh: &GridMesh,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    maps: Option<&BakedMaps>,
    name: &str,
) -> JdResult<MeshExportReport> {
    let n = mesh.vertices() as usize;
    let mut bin: Vec<u8> = Vec::with_capacity(n * 48 + mesh.indices.len() * 4);
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in mesh.positions.as_chunks::<3>().0 {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let push = |bin: &mut Vec<u8>, vals: &[f32]| {
        for v in vals {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    };
    push(&mut bin, &mesh.positions);
    let pos_len = bin.len();
    push(&mut bin, &mesh.normals);
    let nrm_len = bin.len() - pos_len;
    push(&mut bin, &mesh.uvs);
    let uv_len = bin.len() - pos_len - nrm_len;
    let tan_off = bin.len();
    if maps.is_some() {
        for t in mikk_tangents(&mesh.positions, &mesh.normals, &mesh.indices) {
            push(&mut bin, &t);
        }
    }
    let tan_len = bin.len() - tan_off;
    let idx_off = bin.len();
    for i in &mesh.indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let idx_len = bin.len() - idx_off;
    pad4(&mut bin, 0);
    let mut views = vec![
        serde_json::json!({ "buffer": 0, "byteOffset": 0, "byteLength": pos_len, "target": 34962 }),
        serde_json::json!({ "buffer": 0, "byteOffset": pos_len, "byteLength": nrm_len, "target": 34962 }),
        serde_json::json!({ "buffer": 0, "byteOffset": pos_len + nrm_len, "byteLength": uv_len, "target": 34962 }),
        serde_json::json!({ "buffer": 0, "byteOffset": idx_off, "byteLength": idx_len, "target": 34963 }),
    ];
    let mut accessors = vec![
        serde_json::json!({ "bufferView": 0, "componentType": 5126, "count": n, "type": "VEC3", "min": min, "max": max }),
        serde_json::json!({ "bufferView": 1, "componentType": 5126, "count": n, "type": "VEC3" }),
        serde_json::json!({ "bufferView": 2, "componentType": 5126, "count": n, "type": "VEC2" }),
        serde_json::json!({ "bufferView": 3, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }),
    ];
    let mut attributes = serde_json::json!({ "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 });
    let mut material = serde_json::json!({ "name": name, "pbrMetallicRoughness": { "baseColorFactor": [0.55, 0.5, 0.45, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.95 } });
    let mut images = Vec::new();
    let mut normal_px = None;
    if let Some(m) = maps {
        views.push(serde_json::json!({ "buffer": 0, "byteOffset": tan_off, "byteLength": tan_len, "target": 34962 }));
        accessors.push(serde_json::json!({ "bufferView": 4, "componentType": 5126, "count": n, "type": "VEC4" }));
        attributes["TANGENT"] = serde_json::json!(4);
        let mut pngs = vec![
            (&m.albedo, "terrain-albedo"),
            (&m.normal, "terrain-normal"),
            (&m.roughness, "terrain-roughness"),
        ];
        if let Some(e) = &m.emissive {
            pngs.push((e, "terrain-emissive"));
        }
        for (png, name) in pngs {
            let off = bin.len();
            bin.extend_from_slice(png);
            views.push(
                serde_json::json!({ "buffer": 0, "byteOffset": off, "byteLength": png.len() }),
            );
            images.push(serde_json::json!({ "bufferView": views.len() - 1, "mimeType": "image/png", "name": name }));
            pad4(&mut bin, 0);
        }
        material = serde_json::json!({ "name": name, "pbrMetallicRoughness": { "baseColorTexture": { "index": 0, "texCoord": 0 }, "baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicRoughnessTexture": { "index": 2, "texCoord": 0 }, "metallicFactor": 0.0, "roughnessFactor": 1.0 }, "normalTexture": { "index": 1, "texCoord": 0 } });
        material["occlusionTexture"] = serde_json::json!({ "index": 2, "texCoord": 0 }); // AO in R of the ORM map
        if m.emissive.is_some() {
            material["emissiveTexture"] = serde_json::json!({ "index": 3, "texCoord": 0 });
            material["emissiveFactor"] = serde_json::json!([1.0, 1.0, 1.0]);
        }
        normal_px = Some(m.px);
    }
    let mut json = serde_json::json!({
        "asset": { "version": "2.0", "generator": format!("jenga-dunia {}", env!("CARGO_PKG_VERSION")) },
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": accessors,
        "materials": [material],
        "meshes": [{ "name": name, "primitives": [{ "attributes": attributes, "indices": 3, "material": 0, "mode": 4 }] }],
        "nodes": [{ "mesh": 0, "name": name }],
        "scenes": [{ "nodes": [0] }],
        "scene": 0
    });
    if !images.is_empty() {
        json["images"] = serde_json::Value::Array(images);
        json["samplers"] = serde_json::json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }]);
        json["textures"] = serde_json::Value::Array(
            (0..json["images"].as_array().map_or(0, Vec::len))
                .map(|k| serde_json::json!({ "sampler": 0, "source": k }))
                .collect(),
        );
    }
    let bytes = write_glb(path, &json, &bin)?;
    Ok(MeshExportReport {
        lods: Vec::new(),
        path: path.to_path_buf(),
        format: "glb".into(),
        vertices: mesh.vertices(),
        triangles: mesh.triangles(),
        step,
        bytes,
        extent_m,
        texture_px: normal_px,
        normal_px,
        topology: Some(mesh.topology()),
    })
}

/// Export by extension: `.glb` (default, with optional texture), `.obj`, `.stl`, `.ply`, `.usda`, `.usdz`.
/// With `target_triangles` the sampled grid is decimated first (quadric edge collapse) and
/// written as an irregular mesh — every format but `.fbx`, which keeps its quad grid.
pub fn export_mesh(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    target_triangles: Option<u32>,
) -> JdResult<MeshExportReport> {
    export_mesh_with(hf, extent_m, path, step, texture, target_triangles, &[])
}

/// [`export_mesh`] with material layers baked into the GLB's textures.
#[allow(clippy::too_many_arguments)]
pub fn export_mesh_with(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    target_triangles: Option<u32>,
    materials: &[MaterialLayer],
) -> JdResult<MeshExportReport> {
    export_mesh_with_holes(
        hf,
        extent_m,
        path,
        step,
        texture,
        target_triangles,
        materials,
        None,
    )
}

/// [`export_mesh_with`] with an optional 0..1 hole mask: marked quads (≥ 0.5) are left out of
/// every format (GLB and FBX directly; the others through the grid-mesh writer).
#[allow(clippy::too_many_arguments)]
pub fn export_mesh_with_holes(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    target_triangles: Option<u32>,
    materials: &[MaterialLayer],
    holes: Option<&Heightfield>,
) -> JdResult<MeshExportReport> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if holes.is_some()
        && target_triangles.is_none()
        && !matches!(ext.as_deref(), None | Some("glb") | Some("fbx"))
    {
        let grid = GridMesh::build_with(hf, extent_m, step, holes)?;
        return write_decimated(&grid, ext.as_deref(), extent_m, path, step, None);
    }
    if let Some(target) = target_triangles {
        let grid = GridMesh::build_with(hf, extent_m, step, holes)?;
        let mesh = crate::decimate::decimate(&grid, extent_m, target)?;
        let maps = (texture && matches!(ext.as_deref(), None | Some("glb")))
            .then(|| bake_maps_with(hf, extent_m, NORMAL_MAP_CAP, materials))
            .transpose()?;
        return write_decimated(&mesh, ext.as_deref(), extent_m, path, step, maps.as_ref());
    }
    match ext.as_deref() {
        None | Some("glb") => {
            export_glb_with_holes(hf, extent_m, path, step, texture, materials, holes)
        }
        Some("obj") => export_obj(hf, extent_m, path, step),
        Some("stl") => export_stl(hf, extent_m, path, step),
        Some("ply") => export_ply(hf, extent_m, path, step),
        Some("usda") => export_usda(hf, extent_m, path, step),
        Some("usdc") => export_usdc(hf, extent_m, path, step),
        Some("usdz") => export_usdz(hf, extent_m, path, step),
        Some("fbx") => crate::fbx::export_fbx_with_holes(hf, extent_m, path, step, texture, holes),
        Some(other) => Err(JdError::new(
            jd_types::ErrorCode::E_FORMAT_UNSUPPORTED,
            format!("unsupported mesh extension `{other}`"),
        )
        .next("Use .glb, .fbx, .obj, .stl, .ply, .usda or .usdz.")),
    }
}

/// Unit tangent along +U (east) for a vertex normal `n` of a heightfield: ∂P/∂x = (1, ∂h/∂x, 0)
/// with ∂h/∂x = −n.x / n.y (MikkTSpace's answer for a grid whose U runs east). A vertical normal
/// (n.y ≈ 0 never happens on a heightfield) falls back to +X projected onto the plane.
pub fn tangent_of(n: &[f32; 3]) -> [f32; 3] {
    let t = if n[1] > 1e-6 {
        [1.0, -n[0] / n[1], 0.0]
    } else {
        [1.0 - n[0] * n[0], -n[0] * n[1], -n[0] * n[2]]
    };
    let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-6);
    [t[0] / len, t[1] / len, t[2] / len]
}

/// MikkTSpace tangents for a mesh whose U runs east and V north with no seams: every triangle's
/// ∂P/∂u = (1, ∂h/∂x, 0) from its plane, accumulated at each corner weighted by the corner angle,
/// then made orthogonal to the vertex normal. Irregular (decimated) triangles need this; on the
/// regular grid [`tangent_of`] is within a degree of it. w = +1 (normal × tangent = north).
pub fn mikk_tangents(positions: &[f32], normals: &[f32], indices: &[u32]) -> Vec<[f32; 4]> {
    let n = positions.len() / 3;
    let mut acc = vec![[0.0f32; 3]; n];
    let p = |i: u32| {
        let o = i as usize * 3;
        [positions[o], positions[o + 1], positions[o + 2]]
    };
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let angle = |u: [f32; 3], v: [f32; 3]| {
        let dot = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
        let lu = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
        let lv = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (dot / (lu * lv).max(1e-12)).clamp(-1.0, 1.0).acos()
    };
    for tri in indices.as_chunks::<3>().0 {
        let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
        let (e1, e2) = (sub(b, a), sub(c, a));
        let nf = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        if nf[1].abs() < 1e-9 {
            continue;
        }
        let hx = -nf[0] / nf[1];
        let len = (1.0 + hx * hx).sqrt();
        let tf = [1.0 / len, hx / len, 0.0];
        let weights = [
            angle(e1, e2),
            angle(sub(a, b), sub(c, b)),
            angle(sub(a, c), sub(b, c)),
        ];
        for (k, &vi) in tri.iter().enumerate() {
            for d in 0..3 {
                acc[vi as usize][d] += tf[d] * weights[k];
            }
        }
    }
    (0..n)
        .map(|v| {
            let nrm = [normals[v * 3], normals[v * 3 + 1], normals[v * 3 + 2]];
            let a = acc[v];
            let dot = a[0] * nrm[0] + a[1] * nrm[1] + a[2] * nrm[2];
            let t = [
                a[0] - dot * nrm[0],
                a[1] - dot * nrm[1],
                a[2] - dot * nrm[2],
            ];
            let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            let t = if len > 1e-6 {
                [t[0] / len, t[1] / len, t[2] / len]
            } else {
                tangent_of(&nrm)
            };
            [t[0], t[1], t[2], 1.0]
        })
        .collect()
}

/// Write an already decimated mesh in the format `ext` names (FBX keeps the full quad grid and
/// is refused here).
fn write_decimated(
    mesh: &GridMesh,
    ext: Option<&str>,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    maps: Option<&BakedMaps>,
) -> JdResult<MeshExportReport> {
    match ext {
        None | Some("glb") => write_glb_mesh(mesh, extent_m, path, step, maps),
        Some("obj") => write_obj(mesh, extent_m, path, step),
        Some("stl") => write_stl(mesh, extent_m, path, step),
        Some("ply") => write_ply(mesh, extent_m, path, step),
        Some("usda") => write_usda(mesh, extent_m, path, step),
        Some("usdc") => write_usdc(mesh, extent_m, path, step),
        Some("usdz") => write_usdz(mesh, extent_m, path, step),
        Some("fbx") => Err(JdError::invalid(
            "target_triangles",
            "the FBX export writes the full quad grid",
        )
        .next("Use `step` to thin the grid for FBX, or export a decimated .glb/.obj.")),
        Some(other) => Err(JdError::new(
            jd_types::ErrorCode::E_FORMAT_UNSUPPORTED,
            format!("unsupported mesh extension `{other}`"),
        )
        .next("Use .glb, .fbx, .obj, .stl, .ply, .usda or .usdz.")),
    }
}

/// [`export_mesh`] plus `lods` coarser levels beside it — `<stem>_lod1.<ext>`, `_lod2`, … — each
/// decimated to a quarter of the previous level's triangles (quadric edge collapse on the same
/// grid; the baked maps are shared). The report of the full mesh carries the levels in `lods`.
pub fn export_mesh_lods(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    target_triangles: Option<u32>,
    lods: u32,
) -> JdResult<MeshExportReport> {
    export_mesh_lods_with(
        hf,
        extent_m,
        path,
        step,
        texture,
        target_triangles,
        lods,
        &[],
    )
}

/// [`export_mesh_lods`] with material layers baked into every level's textures.
#[allow(clippy::too_many_arguments)]
pub fn export_mesh_lods_with(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    target_triangles: Option<u32>,
    lods: u32,
    materials: &[MaterialLayer],
) -> JdResult<MeshExportReport> {
    export_mesh_lods_with_holes(
        hf,
        extent_m,
        path,
        step,
        texture,
        target_triangles,
        lods,
        materials,
        None,
    )
}

/// [`export_mesh_lods_with`] with an optional hole mask applied to every level.
#[allow(clippy::too_many_arguments)]
pub fn export_mesh_lods_with_holes(
    hf: &Heightfield,
    extent_m: [f64; 2],
    path: &Path,
    step: u32,
    texture: bool,
    target_triangles: Option<u32>,
    lods: u32,
    materials: &[MaterialLayer],
    holes: Option<&Heightfield>,
) -> JdResult<MeshExportReport> {
    if !(1..=8).contains(&lods) {
        return Err(JdError::invalid("lods", "must be within 1..=8"));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if ext.as_deref() == Some("fbx") {
        return Err(
            JdError::invalid("lods", "the FBX export writes the full quad grid")
                .next("Export the LOD chain as .glb/.obj/.ply/.usda, or write FBX without `lods`."),
        );
    }
    let mut report = export_mesh_with_holes(
        hf,
        extent_m,
        path,
        step,
        texture,
        target_triangles,
        materials,
        holes,
    )?;
    let grid = GridMesh::build_with(hf, extent_m, step, holes)?;
    let maps = (texture && matches!(ext.as_deref(), None | Some("glb")))
        .then(|| bake_maps_with(hf, extent_m, NORMAL_MAP_CAP, materials))
        .transpose()?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mesh")
        .to_owned();
    let mut target = report.triangles;
    for i in 1..=lods {
        target = (target / 4).max(2);
        let mesh = crate::decimate::decimate(&grid, extent_m, target)?;
        let mut lod_path = path.with_file_name(format!("{stem}_lod{i}"));
        if let Some(e) = path.extension() {
            lod_path.set_extension(e);
        }
        report.lods.push(write_decimated(
            &mesh,
            ext.as_deref(),
            extent_m,
            &lod_path,
            step,
            maps.as_ref(),
        )?);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdz_holds_the_layer_under_a_bare_name_and_leaves_no_intermediate() {
        if std::process::Command::new("usdzip")
            .arg("--help")
            .output()
            .is_err()
        {
            eprintln!("no usdzip: skipping");
            return;
        }
        let dir = tempfile::tempdir().expect("dir");
        let mut hf = Heightfield::new(8, 8, 8, 0).expect("hf");
        hf.fill_with(|x, y| (x + y) as f32);
        let out = dir.path().join("terrain.usdz");
        export_usdz(&hf, [80.0, 80.0], &out, 1).expect("usdz");
        assert!(out.is_file());
        assert_eq!(
            fs::read_dir(dir.path()).expect("dir").count(),
            1,
            "only the archive is left behind"
        );
        // Stored zip: the first local header names the layer bare (no directory).
        let bytes = fs::read(&out).expect("read");
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let name = std::str::from_utf8(&bytes[30..30 + name_len]).expect("name");
        assert_eq!(name, "terrain.usda", "entry name {name}");
    }

    #[test]
    fn usdc_is_the_usda_layer_in_the_binary_crate() {
        if std::process::Command::new("usdcat")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("usdcat not on PATH: skipping");
            return;
        }
        let mut h = Heightfield::new(17, 13, 64, 0).expect("hf");
        h.fill_with(|x, y| (x * y) as f32);
        let dir = tempfile::tempdir().expect("tmp");
        let usdc = dir.path().join("t.usdc");
        let r = export_usdc(&h, [160.0, 120.0], &usdc, 1).expect("usdc");
        assert_eq!(
            (r.format.as_str(), r.vertices, r.triangles),
            ("usdc", 17 * 13, 16 * 12 * 2)
        );
        let bytes = std::fs::read(&usdc).expect("read");
        assert!(bytes.starts_with(b"PXR-USDC"), "binary crate magic");
        assert!(
            !dir.path().join("t.usdc.usda").exists(),
            "the intermediate layer is removed"
        );
        let usda = export_usda(&h, [160.0, 120.0], &dir.path().join("t.usda"), 1).expect("usda");
        assert!(r.bytes < usda.bytes, "smaller than the text layer");
    }

    #[test]
    fn topology_counts_boundary_degenerate_and_non_manifold() {
        let mut h = Heightfield::new(9, 7, 64, 0).expect("hf");
        h.fill_with(|x, y| (x + y) as f32);
        let grid = GridMesh::build(&h, [90.0, 70.0], 1).expect("grid");
        let t = grid.topology();
        assert_eq!(
            (
                t.degenerate,
                t.non_manifold_edges,
                t.unreferenced_vertices,
                t.out_of_range
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(t.boundary_edges, 2 * (9 - 1) + 2 * (7 - 1), "the perimeter");
        assert_eq!(
            grid_topology(9, 7),
            t,
            "the GLB writer's grid matches GridMesh"
        );
        let dec = crate::decimate::decimate(&grid, [90.0, 70.0], 40)
            .expect("decimate")
            .topology();
        assert_eq!(
            (dec.degenerate, dec.non_manifold_edges, dec.out_of_range),
            (0, 0, 0),
            "{dec:?}"
        );
        // A broken list: a degenerate face, a fan of three faces on one edge, an unused vertex, an index past the end.
        let bad = mesh_topology(
            [[0, 1, 1], [0, 1, 2], [0, 1, 3], [0, 1, 4], [0, 9, 2]].into_iter(),
            6,
        );
        assert_eq!(
            (
                bad.degenerate,
                bad.non_manifold_edges,
                bad.unreferenced_vertices,
                bad.out_of_range
            ),
            (1, 1, 1, 1),
            "{bad:?}"
        );
    }

    #[test]
    fn a_detail_layer_changes_the_macro_maps_and_ships_its_tile_and_weights() {
        let hf = Heightfield::new(64, 64, 64, 0).unwrap();
        let mut hf = hf;
        hf.fill_with(|x, _| x as f32);
        let plain = MaterialLayer {
            mask: None,
            albedo: [0.5, 0.5, 0.5],
            roughness: 0.8,
            emissive: [0.0; 3],
            detail: None,
            texture: None,
        };
        let grainy = MaterialLayer {
            detail: Some(crate::detail::Detail {
                tile_m: 8.0,
                bump: 1.0,
                contrast: 0.5,
                seed: 3,
                px: 64,
            }),
            texture: None,
            ..plain.clone()
        };
        let a = bake_maps_with(&hf, [640.0, 640.0], 64, std::slice::from_ref(&plain)).unwrap();
        let b = bake_maps_with(&hf, [640.0, 640.0], 64, std::slice::from_ref(&grainy)).unwrap();
        assert!(a.details.is_empty() && a.detail_weights.is_none());
        assert_eq!(b.details.len(), 1);
        assert_eq!(b.details[0].0, 0);
        let decode = |png: &[u8]| image::load_from_memory(png).unwrap().to_rgba8();
        let (pa, pb) = (decode(&a.albedo), decode(&b.albedo));
        let diff = pa
            .pixels()
            .zip(pb.pixels())
            .filter(|(x, y)| x.0[0] != y.0[0])
            .count();
        assert!(
            diff > pa.len() / 8,
            "the grain shows in the albedo: {diff} texels differ"
        );
        let (na, nb) = (decode(&a.normal), decode(&b.normal));
        let ndiff = na
            .pixels()
            .zip(nb.pixels())
            .filter(|(x, y)| x.0[0] != y.0[0] || x.0[1] != y.0[1])
            .count();
        assert!(
            ndiff > na.len() / 8,
            "the grain shows in the normal map: {ndiff}"
        );
        let w = decode(b.detail_weights.as_ref().unwrap());
        assert_eq!(w.dimensions(), (64, 64));
        assert!(
            w.pixels().all(|p| p.0[0] == 255 && p.0[1] == 0),
            "layer 0 everywhere in R, nothing in G"
        );
        let bad = MaterialLayer {
            detail: Some(crate::detail::Detail {
                px: 4,
                ..Default::default()
            }),
            texture: None,
            ..plain
        };
        assert!(bake_maps_with(&hf, [640.0, 640.0], 64, &[bad]).is_err());
    }

    #[test]
    fn a_hole_mask_leaves_its_quads_out_of_the_grid_mesh_and_the_glb() {
        let mut h = Heightfield::new(16, 16, 16, 0).expect("hf");
        h.fill_with(|x, y| (x + y) as f32);
        let mut holes = h.clone();
        holes.fill_with(|x, y| {
            if (4..8).contains(&x) && (4..8).contains(&y) {
                1.0
            } else {
                0.0
            }
        });
        let full = GridMesh::build(&h, [160.0, 160.0], 1).expect("full");
        let cut = GridMesh::build_with(&h, [160.0, 160.0], 1, Some(&holes)).expect("cut");
        assert_eq!(
            full.indices.len() - cut.indices.len(),
            16 * 6,
            "16 quads = 32 triangles gone"
        );
        assert_eq!(
            cut.positions.len(),
            full.positions.len(),
            "vertices stay; only faces go"
        );
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("holed.glb");
        let r =
            export_mesh_with_holes(&h, [160.0, 160.0], &path, 1, false, None, &[], Some(&holes))
                .expect("glb");
        assert_eq!(r.triangles, 15 * 15 * 2 - 32);
        let obj = dir.path().join("holed.obj");
        let r = export_mesh_with_holes(&h, [160.0, 160.0], &obj, 1, false, None, &[], Some(&holes))
            .expect("obj");
        assert_eq!(
            r.triangles,
            15 * 15 * 2 - 32,
            "other formats go through the grid mesh"
        );
        let mut wrong = Heightfield::new(8, 8, 8, 0).expect("hf");
        wrong.fill_with(|_, _| 0.0);
        assert!(GridMesh::build_with(&h, [160.0, 160.0], 1, Some(&wrong)).is_err());
    }

    #[test]
    fn the_orm_map_carries_ambient_occlusion_in_red() {
        let mut h = Heightfield::new(64, 64, 64, 0).expect("hf");
        // A 60 m deep bowl: the floor sees less sky than the rim.
        h.fill_with(|x, y| {
            let d = ((x as f32 - 31.5).powi(2) + (y as f32 - 31.5).powi(2)).sqrt();
            (d / 24.0).min(1.0) * 60.0
        });
        let (_, rough_png, px) =
            bake_normal_roughness_with(&h, [640.0, 640.0], 64, &[]).expect("maps");
        assert_eq!(px, [64, 64]);
        let img = image::load_from_memory(&rough_png)
            .expect("png")
            .into_rgb8();
        let floor = img.get_pixel(32, 32)[0];
        let rim = img.get_pixel(2, 32)[0];
        assert!(floor < rim, "AO floor {floor} vs rim {rim}");
        assert!(rim >= 250, "an open plain is unoccluded: {rim}");
        assert_eq!(img.get_pixel(32, 32)[2], 0, "metallic stays 0");
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("ao.glb");
        export_glb_with(&h, [640.0, 640.0], &path, 1, true, &[]).expect("glb");
        let bytes = std::fs::read(&path).expect("read");
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().expect("len")) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + json_len]).expect("json");
        assert_eq!(json["materials"][0]["occlusionTexture"]["index"], 2);
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"],
            2
        );
    }

    #[test]
    fn material_layers_blend_albedo_and_roughness_by_their_masks() {
        let mut h = Heightfield::new(32, 16, 64, 0).expect("hf");
        h.fill_with(|x, _| x as f32 * 10.0);
        let mut west = Heightfield::new(32, 16, 64, 0).expect("mask");
        west.fill_with(|x, _| f32::from(x < 16));
        let red = MaterialLayer {
            mask: Some(west),
            albedo: [1.0, 0.0, 0.0],
            roughness: 0.3,
            emissive: [0.0; 3],
            detail: None,
            texture: None,
        };
        let plain = bake_maps(&h, [320.0, 160.0], 64).expect("plain");
        let layered =
            bake_maps_with(&h, [320.0, 160.0], 64, std::slice::from_ref(&red)).expect("layered");
        let rgb = |png: &[u8]| image::load_from_memory(png).expect("png").to_rgb8();
        let (a, b) = (rgb(&plain.albedo), rgb(&layered.albedo));
        assert_eq!(b.get_pixel(4, 8).0, [255, 0, 0], "west is the layer's red");
        assert_eq!(
            b.get_pixel(28, 8).0,
            a.get_pixel(28, 8).0,
            "east keeps the base ramp"
        );
        let (ra, rb) = (rgb(&plain.roughness), rgb(&layered.roughness));
        assert_eq!(
            rb.get_pixel(4, 8)[1],
            (0.3f32 * 255.0).round() as u8,
            "west roughness is the layer's"
        );
        assert_eq!(rb.get_pixel(28, 8)[1], ra.get_pixel(28, 8)[1]);
        let bad = MaterialLayer {
            mask: None,
            albedo: [2.0, 0.0, 0.0],
            roughness: 0.5,
            emissive: [0.0; 3],
            detail: None,
            texture: None,
        };
        assert!(bake_maps_with(&h, [320.0, 160.0], 64, &[bad]).is_err());
    }

    #[test]
    fn an_emitting_layer_bakes_a_glow_map_where_its_mask_is_and_the_glb_carries_it() {
        let mut h = Heightfield::new(32, 16, 32, 0).expect("hf");
        h.fill_with(|x, _| x as f32);
        let mut west = Heightfield::new(32, 16, 32, 0).expect("mask");
        west.fill_with(|x, _| if x < 16 { 1.0 } else { 0.0 });
        let glow = MaterialLayer {
            mask: Some(west),
            albedo: [0.2, 0.2, 0.2],
            roughness: 0.7,
            emissive: [1.0, 0.35, 0.05],
            detail: None,
            texture: None,
        };
        let maps =
            bake_maps_with(&h, [320.0, 160.0], 64, std::slice::from_ref(&glow)).expect("bake");
        let e = image::load_from_memory(maps.emissive.as_deref().expect("emissive map"))
            .expect("png")
            .to_rgb8();
        assert_eq!(
            e.get_pixel(4, 8).0,
            [255, 89, 13],
            "west glows the layer's colour"
        );
        assert_eq!(e.get_pixel(28, 8).0, [0, 0, 0], "east emits nothing");
        assert!(
            bake_maps(&h, [320.0, 160.0], 64)
                .expect("plain")
                .emissive
                .is_none(),
            "no emitting layer, no map"
        );
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("glow.glb");
        export_glb_with(
            &h,
            [320.0, 160.0],
            &path,
            1,
            true,
            std::slice::from_ref(&glow),
        )
        .expect("glb");
        let bytes = std::fs::read(&path).expect("read");
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().expect("len")) as usize;
        let json: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + json_len]).expect("json");
        assert_eq!(json["materials"][0]["emissiveTexture"]["index"], 3);
        assert_eq!(json["images"][3]["name"], "terrain-emissive");
        assert_eq!(json["textures"].as_array().expect("textures").len(), 4);
    }

    #[test]
    fn tangent_follows_the_u_direction_and_stays_in_the_tangent_plane() {
        // A slope rising to the east (∂h/∂x = 0.5) with a cross-slope to the north (∂h/∂z = −0.8).
        let (hx, hz) = (0.5f32, -0.8f32);
        let len = (hx * hx + 1.0 + hz * hz).sqrt();
        let n = [-hx / len, 1.0 / len, -hz / len];
        let t = tangent_of(&n);
        assert!(
            (t[0] * n[0] + t[1] * n[1] + t[2] * n[2]).abs() < 1e-6,
            "orthogonal to the normal"
        );
        assert_eq!(t[2], 0.0, "∂P/∂u has no north–south component");
        assert!(
            (t[1] / t[0] - hx).abs() < 1e-6,
            "rises with the eastward slope"
        );
        assert_eq!(tangent_of(&[0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]);
        // Two triangles on that same plane: the face-averaged tangent agrees with the analytic one.
        let z = |x: f32, zz: f32| hx * x + hz * zz;
        let pos = [
            0.0,
            z(0.0, 0.0),
            0.0,
            10.0,
            z(10.0, 0.0),
            0.0,
            0.0,
            z(0.0, -10.0),
            -10.0,
            10.0,
            z(10.0, -10.0),
            -10.0,
        ];
        let nrm: Vec<f32> = n.iter().cycle().take(12).copied().collect();
        let m = mikk_tangents(&pos, &nrm, &[0, 1, 2, 2, 1, 3]);
        for mt in &m {
            for d in 0..3 {
                assert!((mt[d] - t[d]).abs() < 1e-5, "{mt:?} vs {t:?}");
            }
            assert_eq!(mt[3], 1.0);
        }
    }

    #[test]
    fn lod_chain_writes_quartering_levels_beside_the_full_mesh() {
        let mut hf = Heightfield::new(48, 48, 64, 0).expect("hf");
        hf.fill_with(|x, y| ((x as f32 * 0.4).sin() + (y as f32 * 0.3).cos()) * 20.0);
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("t.glb");
        let r = export_mesh_lods(&hf, [470.0, 470.0], &path, 1, false, None, 2).expect("lods");
        assert_eq!(r.triangles, 47 * 47 * 2);
        assert_eq!(r.lods.len(), 2);
        assert!(dir.path().join("t_lod1.glb").exists());
        assert!(dir.path().join("t_lod2.glb").exists());
        let (t1, t2) = (r.lods[0].triangles, r.lods[1].triangles);
        assert!(t1 <= r.triangles / 4 && t1 > r.triangles / 8, "lod1 {t1}");
        assert!(t2 <= t1 / 4 + 1 && t2 > t1 / 8, "lod2 {t2}");
        assert!(
            export_mesh_lods(
                &hf,
                [470.0, 470.0],
                &dir.path().join("t.fbx"),
                1,
                false,
                None,
                1
            )
            .is_err()
        );
        assert!(export_mesh_lods(&hf, [470.0, 470.0], &path, 1, false, None, 0).is_err());
    }

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
        assert_eq!(
            (r.texture_px, r.normal_px),
            (Some([33, 17]), Some([33, 17]))
        );
        let bytes = std::fs::read(&p).expect("read");
        let (json, bin) = chunks(&bytes);
        assert_eq!(json["images"][0]["mimeType"], "image/png");
        assert_eq!(json["images"].as_array().map(Vec::len), Some(3));
        assert_eq!(json["materials"][0]["normalTexture"]["index"], 1);
        let tangent = json["meshes"][0]["primitives"][0]["attributes"]["TANGENT"]
            .as_u64()
            .expect("tangent accessor") as usize;
        assert_eq!(json["accessors"][tangent]["count"], 33 * 17);
        let tv = &json["bufferViews"]
            [json["accessors"][tangent]["bufferView"].as_u64().unwrap() as usize];
        let to = tv["byteOffset"].as_u64().unwrap() as usize;
        let tangent0: Vec<f32> = (0..4)
            .map(|k| f32::from_le_bytes(bin[to + 4 * k..to + 4 * k + 4].try_into().unwrap()))
            .collect();
        // The 0.1 slope tilts the normal west; the tangent stays unit, leans up, w = +1.
        assert!(
            (tangent0[0] - 0.995).abs() < 0.01 && tangent0[1] > 0.09 && tangent0[3] == 1.0,
            "{tangent0:?}"
        );
        assert_eq!(
            json["materials"][0]["pbrMetallicRoughness"]["metallicRoughnessTexture"]["index"],
            2
        );
        // Normal map: a 0.1 east-facing slope → tangent-space (−0.0995, 0, 0.995) → ≈ (115, 128, 254).
        let decode = |i: usize| {
            let v =
                &json["bufferViews"][json["images"][i]["bufferView"].as_u64().unwrap() as usize];
            let (o, l) = (
                v["byteOffset"].as_u64().unwrap() as usize,
                v["byteLength"].as_u64().unwrap() as usize,
            );
            image::load_from_memory(&bin[o..o + l])
                .expect("png")
                .into_rgb8()
        };
        let n = decode(1).get_pixel(16, 8).0;
        assert!(
            (110..=120).contains(&n[0]) && (126..=130).contains(&n[1]) && n[2] >= 250,
            "normal texel {n:?}"
        );
        let rough = decode(2).get_pixel(16, 8).0;
        assert!(
            rough[0] >= 200 && rough[2] == 0 && (150..=240).contains(&rough[1]), // R = ambient occlusion (open here), B = metallic 0
            "roughness texel {rough:?}"
        );
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
        let obj =
            export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.obj"), 1, false, None).expect("obj");
        let stl =
            export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.stl"), 1, false, None).expect("stl");
        let ply =
            export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.ply"), 1, false, None).expect("ply");
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
        assert!(export_mesh(&hf, [50.0, 40.0], &dir.path().join("g.xyz"), 1, false, None).is_err());
    }

    #[test]
    fn usda_has_the_stage_metadata_and_counts() {
        let mut hf = Heightfield::new(4, 3, 64, 0).expect("hf");
        hf.fill_with(|x, y| x as f32 + y as f32);
        let dir = tempfile::tempdir().expect("tmp");
        let r = export_mesh(
            &hf,
            [40.0, 30.0],
            &dir.path().join("t.usda"),
            1,
            false,
            None,
        )
        .expect("usda");
        assert_eq!(
            (r.format.as_str(), r.vertices, r.triangles),
            ("usda", 12, 12)
        );
        let text = std::fs::read_to_string(dir.path().join("t.usda")).expect("read");
        assert!(text.starts_with("#usda 1.0\n"));
        for needle in [
            "metersPerUnit = 1",
            "upAxis = \"Y\"",
            "defaultPrim = \"Terrain\"",
            "def Mesh \"Grid\"",
            "primvars:st",
            "primvars:displayColor",
            "uniform token subdivisionScheme = \"none\"",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        assert!(
            text.contains(&format!(
                "int[] faceVertexCounts = [{}]",
                ["3"; 12].join(", ")
            )),
            "one faceVertexCount per triangle"
        );
        // 4×3 samples over 40×30 m: centres at x = ±15, z = ±10; heights 0..5.
        assert!(
            text.contains("float3[] extent = [(-15, 0, -10), (15, 5, 10)]"),
            "{}",
            text.lines().find(|l| l.contains("extent")).unwrap_or("")
        );
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
