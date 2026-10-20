//! Rock generator: seeded, faceted boulders as meshes — scatter proxies, props, the stones a
//! desert pavement or a scree slope needs. VUE's "random rock generator" (`docs/research/
//! vue-features.md`). An icosphere is squashed anisotropically, its vertices pushed along their
//! normals by three octaves of seeded value noise, the bottom cut flat so the rock sits on the
//! ground, and every triangle gets its own vertices so the facets shade flat.
//!
//! ponytail: value noise on a 3-D lattice with a splitmix hash, no erosion of the rock itself;
//! a sediment-layer look (strata lines) and smooth river cobbles are the next presets.

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::mesh::GridMesh;

/// Rock settings.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RockParams {
    /// Seed: the same seed is the same rock.
    #[serde(default)]
    pub seed: u64,
    /// Nominal radius, metres (> 0).
    #[serde(default = "d_radius")]
    pub radius_m: f32,
    /// Icosphere subdivisions 0..=4: 20 · 4ⁿ triangles (2 → 320, 3 → 1 280).
    #[serde(default = "d_detail")]
    pub detail: u8,
    /// Surface displacement as a fraction of the radius, 0..=1 (0.35 is a weathered boulder).
    #[serde(default = "d_roughness")]
    pub roughness: f32,
    /// Share of the rock's height cut away at the bottom so it sits flat, 0..=0.5.
    #[serde(default = "d_flatten")]
    pub flatten: f32,
}

fn d_radius() -> f32 {
    1.0
}
fn d_detail() -> u8 {
    2
}
fn d_roughness() -> f32 {
    0.35
}
fn d_flatten() -> f32 {
    0.25
}

impl Default for RockParams {
    fn default() -> Self {
        Self {
            seed: 0,
            radius_m: d_radius(),
            detail: d_detail(),
            roughness: d_roughness(),
            flatten: d_flatten(),
        }
    }
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Seeded value noise on the integer lattice, trilinear, smoothstep-eased, in −1..1.
fn value_noise(seed: u64, p: [f32; 3]) -> f32 {
    let f = |x: f32| x.floor();
    let (x0, y0, z0) = (f(p[0]), f(p[1]), f(p[2]));
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty, tz) = (ease(p[0] - x0), ease(p[1] - y0), ease(p[2] - z0));
    let lat = |i: f32, j: f32, k: f32| {
        let h = mix(seed
            ^ mix((i as i64 as u64).wrapping_mul(73_856_093)
                ^ (j as i64 as u64).wrapping_mul(19_349_663)
                ^ (k as i64 as u64).wrapping_mul(83_492_791)));
        unit(h) * 2.0 - 1.0
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c00 = lerp(lat(x0, y0, z0), lat(x0 + 1.0, y0, z0), tx);
    let c10 = lerp(lat(x0, y0 + 1.0, z0), lat(x0 + 1.0, y0 + 1.0, z0), tx);
    let c01 = lerp(lat(x0, y0, z0 + 1.0), lat(x0 + 1.0, y0, z0 + 1.0), tx);
    let c11 = lerp(
        lat(x0, y0 + 1.0, z0 + 1.0),
        lat(x0 + 1.0, y0 + 1.0, z0 + 1.0),
        tx,
    );
    lerp(lerp(c00, c10, ty), lerp(c01, c11, ty), tz)
}

/// Unit icosphere: vertices and triangles after `detail` midpoint subdivisions.
fn icosphere(detail: u8) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0f32.sqrt()) / 2.0;
    let mut verts: Vec<[f32; 3]> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .iter()
    .map(|v| normalize(*v))
    .collect();
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..detail {
        let mut cache: std::collections::HashMap<(u32, u32), u32> =
            std::collections::HashMap::new();
        let mut next = Vec::with_capacity(tris.len() * 4);
        for tri in &tris {
            let mut mid = |a: u32, b: u32| -> u32 {
                let key = (a.min(b), a.max(b));
                *cache.entry(key).or_insert_with(|| {
                    let (pa, pb) = (verts[a as usize], verts[b as usize]);
                    verts.push(normalize([
                        (pa[0] + pb[0]) * 0.5,
                        (pa[1] + pb[1]) * 0.5,
                        (pa[2] + pb[2]) * 0.5,
                    ]));
                    verts.len() as u32 - 1
                })
            };
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    (verts, tris)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Build the rock as a flat-shaded mesh (three vertices per triangle), base at y = 0, centred
/// on x = z = 0, +Y up, metres.
pub fn rock_mesh(p: &RockParams) -> JdResult<GridMesh> {
    if !(p.radius_m > 0.0 && p.radius_m.is_finite()) {
        return Err(JdError::invalid("radius_m", "must be positive"));
    }
    if p.detail > 4 {
        return Err(JdError::invalid("detail", "must be within 0..=4"));
    }
    if !(0.0..=1.0).contains(&p.roughness) {
        return Err(JdError::invalid("roughness", "must be within 0..=1"));
    }
    if !(0.0..=0.5).contains(&p.flatten) {
        return Err(JdError::invalid("flatten", "must be within 0..=0.5"));
    }
    let (sphere, tris) = icosphere(p.detail);
    // Seeded shape: anisotropic squash in 0.7..1.3 per axis and a noise offset.
    let s = mix(p.seed);
    let squash = [
        0.7 + 0.6 * unit(mix(s ^ 1)),
        0.7 + 0.6 * unit(mix(s ^ 2)),
        0.7 + 0.6 * unit(mix(s ^ 3)),
    ];
    let offset = [
        unit(mix(s ^ 4)) * 100.0,
        unit(mix(s ^ 5)) * 100.0,
        unit(mix(s ^ 6)) * 100.0,
    ];
    let displaced: Vec<[f32; 3]> = sphere
        .iter()
        .map(|n| {
            // Three octaves of value noise along the normal: big lumps, then facets, then grit.
            let mut d = 0.0f32;
            let (mut amp, mut freq) = (1.0f32, 1.5f32);
            for _ in 0..3 {
                d += amp
                    * value_noise(
                        p.seed,
                        [
                            n[0] * freq + offset[0],
                            n[1] * freq + offset[1],
                            n[2] * freq + offset[2],
                        ],
                    );
                amp *= 0.5;
                freq *= 2.1;
            }
            let r = p.radius_m * (1.0 + p.roughness * d / 1.75);
            [
                n[0] * r * squash[0],
                n[1] * r * squash[1],
                n[2] * r * squash[2],
            ]
        })
        .collect();
    // Cut the bottom flat and rest it on y = 0.
    let (ymin, ymax) = displaced.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| {
        (lo.min(v[1]), hi.max(v[1]))
    });
    let floor = ymin + (ymax - ymin) * p.flatten;
    let mut positions = Vec::with_capacity(tris.len() * 9);
    let mut normals = Vec::with_capacity(tris.len() * 9);
    let mut uvs = Vec::with_capacity(tris.len() * 6);
    let mut indices = Vec::with_capacity(tris.len() * 3);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for tri in &tris {
        let v: Vec<[f32; 3]> = tri
            .iter()
            .map(|&i| {
                let q = displaced[i as usize];
                [q[0], q[1].max(floor) - floor, q[2]]
            })
            .collect();
        let (e1, e2) = (
            [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]],
            [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]],
        );
        let n = normalize([
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ]);
        for (k, q) in v.iter().enumerate() {
            indices.push(positions.len() as u32 / 3);
            positions.extend_from_slice(q);
            normals.extend_from_slice(&n);
            let dir = sphere[tri[k] as usize];
            uvs.extend_from_slice(&[
                dir[0].atan2(-dir[2]) / std::f32::consts::TAU + 0.5,
                dir[1] * 0.5 + 0.5,
            ]);
            lo = lo.min(q[1]);
            hi = hi.max(q[1]);
        }
    }
    Ok(GridMesh {
        positions,
        normals,
        uvs,
        indices,
        grid: [0, 0],
        height_range: [lo, hi],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rocks_are_seeded_faceted_sit_on_the_ground_and_stay_near_their_radius() {
        let p = RockParams::default();
        let a = rock_mesh(&p).expect("rock");
        assert_eq!(a.indices.len(), 20 * 16 * 3, "20·4² facets");
        assert_eq!(
            a.positions.len(),
            a.indices.len() * 3,
            "three vertices per facet"
        );
        assert_eq!(a.height_range[0], 0.0, "base on the ground");
        let (mut rmax, mut ymax) = (0.0f32, 0.0f32);
        for q in a.positions.as_chunks::<3>().0 {
            rmax = rmax.max((q[0] * q[0] + q[2] * q[2]).sqrt());
            ymax = ymax.max(q[1]);
        }
        assert!(
            (0.6..=1.8).contains(&rmax) && (0.6..=2.6).contains(&ymax),
            "size {rmax} × {ymax}"
        );
        // Same seed, same rock; another seed, another rock; detail scales the facet count.
        assert_eq!(rock_mesh(&p).expect("again").positions, a.positions);
        let b = rock_mesh(&RockParams {
            seed: 7,
            ..p.clone()
        })
        .expect("rock");
        assert_ne!(b.positions, a.positions);
        assert_eq!(
            rock_mesh(&RockParams {
                detail: 3,
                ..p.clone()
            })
            .expect("fine")
            .indices
            .len(),
            20 * 64 * 3
        );
        // Every facet normal is a unit vector pointing away from the centre of its triangle (closed outward shell).
        for (tri, n) in a
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(a.normals.as_chunks::<3>().0.iter().step_by(3))
        {
            let c: [f32; 3] = (0..3)
                .map(|k| {
                    tri.iter()
                        .map(|&i| a.positions[i as usize * 3 + k])
                        .sum::<f32>()
                        / 3.0
                })
                .collect::<Vec<_>>()
                .try_into()
                .unwrap();
            let centre_y = a.height_range[1] * 0.5;
            let dot = n[0] * c[0] + n[1] * (c[1] - centre_y) + n[2] * c[2];
            assert!(dot > -0.25, "facet faces outward: {dot}");
        }
        assert!(
            rock_mesh(&RockParams {
                detail: 5,
                ..p.clone()
            })
            .is_err()
        );
        assert!(rock_mesh(&RockParams { radius_m: 0.0, ..p }).is_err());
    }
}
