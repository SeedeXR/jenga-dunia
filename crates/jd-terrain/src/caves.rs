//! Caves: subterranean voids beside the heightfield (ADR-0016, `docs/research/caves.md`).
//!
//! A cave system is a skeleton — passages (polylines with a cross-section profile and a size per
//! point), chambers (ellipsoids with a flat floor) and entrances — turned into a signed distance
//! (negative = air), roughened by seeded noise for natural caves, and meshed with surface nets on
//! a sparse voxel grid that only covers the skeleton. The mesh is clipped where it rises above the
//! terrain; the cells where the void breaks the surface become the terrain's hole mask, so the
//! terrain opens exactly over the entrances. Speleothems (stalactites, stalagmites, columns) are
//! separate meshes: they are thinner than any useful voxel.
//!
//! Coordinates: `u` metres east of the west edge, `v` metres north of the south edge, `z` metres
//! (the heightfield's unit). Meshes use the terrain export's frame: x = u − extent_x/2,
//! y = z, z = extent_y/2 − v.

use std::collections::{BTreeSet, HashMap};

use jd_types::{JdError, JdResult};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::heightfield::Heightfield;
use crate::mesh::GridMesh;
use crate::rocks::{mix, unit, value_noise};

/// Cross-section of a passage.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Elliptical tube: phreatic passages, lava tubes, shafts.
    #[default]
    Tube,
    /// A tube over a narrow slot: a phreatic tube later cut down by a vadose stream.
    Keyhole,
    /// Flat floor, straight walls and a round roof: canyons (tall and narrow), mine drifts, road
    /// tunnels, catacomb galleries.
    Arch,
    /// Rectangle: mine shafts and stopes.
    Rect,
}

/// One passage: a polyline in (u, v, z) with a half-width and half-height per point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Passage {
    /// Centreline points `[u, v, z]`, metres.
    pub points: Vec<[f32; 3]>,
    /// Half-width per point, metres.
    pub half_width_m: Vec<f32>,
    /// Half-height per point, metres.
    pub half_height_m: Vec<f32>,
    /// Cross-section.
    pub profile: Profile,
}

/// A chamber: an ellipsoid with its lowest 40 % cut flat (a breakdown floor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Chamber {
    /// Centre `[u, v, z]`, metres.
    pub center: [f32; 3],
    /// Semi-axes along u, v, z, metres.
    pub radii_m: [f32; 3],
}

/// How a cave meets the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntranceKind {
    /// A doline / sinkhole swallowing water into the cave (karst inlet).
    Sinkhole,
    /// A cave mouth at the foot of a slope or cliff; a mine adit; a tunnel portal.
    Portal,
    /// A roof collapse into a lava tube.
    Skylight,
    /// Where the cave's water comes out: a spring, often under a lake, river or the sea.
    Resurgence,
    /// A wave-cut notch at sea level.
    SeaCave,
    /// A vertical shaft from the surface.
    Shaft,
}

/// Where a cave opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Entrance {
    /// Position `[u, v, z]` on the surface, metres.
    pub at: [f32; 3],
    /// Kind.
    pub kind: EntranceKind,
    /// Below the system's water level (a diver's entrance).
    pub underwater: bool,
}

/// Speleothem kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpeleothemKind {
    /// Hangs from the ceiling.
    Stalactite,
    /// Grows up from the floor under a drip.
    Stalagmite,
    /// A stalactite and stalagmite grown together, floor to ceiling.
    Column,
}

/// A speleothem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Speleothem {
    /// Kind.
    pub kind: SpeleothemKind,
    /// Anchor `[u, v, z]`: the ceiling point of a stalactite or column, the floor point of a stalagmite.
    pub at: [f32; 3],
    /// Length, metres (a column: floor to ceiling).
    pub length_m: f32,
    /// Radius at the anchor, metres.
    pub radius_m: f32,
}

/// A generated cave system.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CaveSystem {
    /// Passages.
    pub passages: Vec<Passage>,
    /// Chambers.
    pub chambers: Vec<Chamber>,
    /// Entrances.
    pub entrances: Vec<Entrance>,
    /// Speleothems.
    pub speleothems: Vec<Speleothem>,
    /// Water level, metres: air above, flooded below (the water table, a lake or the sea).
    pub water_level_m: Option<f32>,
    /// Wall roughness amplitude, metres (0 = machined walls).
    pub roughness_m: f32,
    /// Noise seed for the walls.
    pub seed: u64,
}

impl CaveSystem {
    /// Total passage length, metres.
    pub fn length_m(&self) -> f32 {
        self.passages
            .iter()
            .flat_map(|p| p.points.windows(2))
            .map(|w| dist3(w[0], w[1]))
            .sum()
    }

    /// Check sizes and finiteness.
    pub fn validate(&self) -> JdResult<()> {
        for (i, p) in self.passages.iter().enumerate() {
            let n = p.points.len();
            if n < 2 || p.half_width_m.len() != n || p.half_height_m.len() != n {
                return Err(JdError::invalid(
                    "passages",
                    format!("passage {i}: needs ≥ 2 points and one width and height per point"),
                ));
            }
            if p.points.iter().flatten().any(|c| !c.is_finite())
                || p.half_width_m
                    .iter()
                    .chain(&p.half_height_m)
                    .any(|r| !(r.is_finite() && *r > 0.0))
            {
                return Err(JdError::invalid(
                    "passages",
                    format!("passage {i}: non-finite point or non-positive size"),
                ));
            }
        }
        for (i, c) in self.chambers.iter().enumerate() {
            if c.center.iter().any(|v| !v.is_finite())
                || c.radii_m.iter().any(|r| !(r.is_finite() && *r > 0.0))
            {
                return Err(JdError::invalid(
                    "chambers",
                    format!("chamber {i}: non-finite centre or non-positive radius"),
                ));
            }
        }
        if !(self.roughness_m.is_finite() && self.roughness_m >= 0.0) {
            return Err(JdError::invalid("roughness_m", "must be finite and ≥ 0"));
        }
        Ok(())
    }
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
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
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

/// 2-D distance to a cross-section centred on the passage axis (x across, y up).
fn profile_sdf(profile: Profile, x: f32, y: f32, w: f32, h: f32) -> f32 {
    let bx = |cx: f32, cy: f32, hx: f32, hy: f32| {
        let (dx, dy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
        (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0)
    };
    let circle = |cy: f32, r: f32| (x * x + (y - cy).powi(2)).sqrt() - r;
    match profile {
        Profile::Tube => ((x / w).powi(2) + (y / h).powi(2))
            .sqrt()
            .mul_add(w.min(h), -w.min(h)),
        Profile::Rect => bx(0.0, 0.0, w, h),
        Profile::Arch => {
            // Walls from the floor (−h) up to the springing line, a half-circle roof above it.
            let yc = h - w.min(h);
            let walls = bx(0.0, (yc - h) / 2.0, w, (yc + h) / 2.0);
            walls.min(circle(yc, w).max(yc - y))
        }
        Profile::Keyhole => {
            let r = w.min(h);
            let yc = h - r;
            let slot_w = KEYHOLE_SLOT * w;
            circle(yc, r).min(bx(0.0, (yc - h) / 2.0, slot_w, (yc + h) / 2.0))
        }
    }
}

#[derive(Clone)]
enum Prim {
    Seg {
        a: [f32; 3],
        b: [f32; 3],
        w: [f32; 2],
        h: [f32; 2],
        profile: Profile,
        /// How far the segment reaches past each end (interior joints overlap their neighbour).
        ext: [f32; 2],
    },
    Chamber {
        c: [f32; 3],
        r: [f32; 3],
    },
}

impl Prim {
    fn sdf(&self, p: [f32; 3]) -> f32 {
        match *self {
            Prim::Chamber { c, r } => {
                let q = sub(p, c);
                let k =
                    ((q[0] / r[0]).powi(2) + (q[1] / r[1]).powi(2) + (q[2] / r[2]).powi(2)).sqrt();
                let m = r[0].min(r[1]).min(r[2]);
                // Flat floor 40 % of the way down.
                ((k - 1.0) * m).max((c[2] - 0.6 * r[2]) - p[2])
            }
            Prim::Seg {
                a,
                b,
                w,
                h,
                profile,
                ext,
            } => {
                let d = sub(b, a);
                let len = dot(d, d).sqrt();
                let t_dir = if len > 1e-6 {
                    [d[0] / len, d[1] / len, d[2] / len]
                } else {
                    [1.0, 0.0, 0.0]
                };
                let along = dot(sub(p, a), t_dir);
                let t = if len > 1e-6 {
                    (along / len).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (ww, hh) = (w[0] + (w[1] - w[0]) * t, h[0] + (h[1] - h[0]) * t);
                let c = [
                    a[0] + t_dir[0] * t * len,
                    a[1] + t_dir[1] * t * len,
                    a[2] + t_dir[2] * t * len,
                ];
                let q = sub(p, c);
                let side = cross(t_dir, [0.0, 0.0, 1.0]);
                let side = if dot(side, side) < 1e-6 {
                    [1.0, 0.0, 0.0]
                } else {
                    norm(side)
                };
                let up = cross(side, t_dir);
                let (x, y) = (dot(q, side), dot(q, up));
                // Axial distance past the (extended) ends: positive outside, negative inside.
                let ax = (-along - ext[0]).max(along - len - ext[1]);
                if profile == Profile::Tube {
                    let a_out = ax.max(0.0);
                    let m = ww.min(hh);
                    ((x / ww).powi(2) + (y / hh).powi(2) + (a_out / ww).powi(2))
                        .sqrt()
                        .mul_add(m, -m)
                } else {
                    let d2 = profile_sdf(profile, x, y, ww, hh);
                    (d2.max(0.0).powi(2) + ax.max(0.0).powi(2)).sqrt() + d2.max(ax).min(0.0)
                }
            }
        }
    }

    /// Bounding box `[min, max]` grown by `pad`.
    fn aabb(&self, pad: f32) -> [[f32; 3]; 2] {
        match *self {
            Prim::Chamber { c, r } => [
                [c[0] - r[0] - pad, c[1] - r[1] - pad, c[2] - r[2] - pad],
                [c[0] + r[0] + pad, c[1] + r[1] + pad, c[2] + r[2] + pad],
            ],
            Prim::Seg {
                a, b, w, h, ext, ..
            } => {
                let m = w[0].max(w[1]).max(h[0]).max(h[1]) + ext[0].max(ext[1]) + pad;
                [
                    [a[0].min(b[0]) - m, a[1].min(b[1]) - m, a[2].min(b[2]) - m],
                    [a[0].max(b[0]) + m, a[1].max(b[1]) + m, a[2].max(b[2]) + m],
                ]
            }
        }
    }
}

/// The cave's signed distance (negative = air), with spatial binning so a query only looks at
/// the primitives near it.
pub struct CaveField {
    prims: Vec<Prim>,
    boxes: Vec<[[f32; 3]; 2]>,
    bins: HashMap<(i32, i32), Vec<u32>>,
    bin_m: f32,
    roughness_m: f32,
    seed: u64,
    /// Noise wavelength, metres.
    feature_m: f32,
}

impl CaveField {
    /// Build the field of a cave system.
    pub fn new(sys: &CaveSystem) -> JdResult<Self> {
        sys.validate()?;
        let mut prims = Vec::new();
        for p in &sys.passages {
            let n = p.points.len();
            for i in 0..n - 1 {
                let interior = |k: usize| {
                    if k == 0 || k == n - 1 {
                        0.0
                    } else {
                        p.half_width_m[k].max(p.half_height_m[k])
                    }
                };
                prims.push(Prim::Seg {
                    a: p.points[i],
                    b: p.points[i + 1],
                    w: [p.half_width_m[i], p.half_width_m[i + 1]],
                    h: [p.half_height_m[i], p.half_height_m[i + 1]],
                    profile: p.profile,
                    ext: [interior(i), interior(i + 1)],
                });
            }
        }
        for c in &sys.chambers {
            prims.push(Prim::Chamber {
                c: c.center,
                r: c.radii_m,
            });
        }
        let pad = 2.0 * sys.roughness_m;
        let boxes: Vec<_> = prims.iter().map(|p| p.aabb(pad)).collect();
        let bin_m = 16.0f32;
        let mut bins: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, b) in boxes.iter().enumerate() {
            for bx in (b[0][0] / bin_m).floor() as i32..=(b[1][0] / bin_m).floor() as i32 {
                for by in (b[0][1] / bin_m).floor() as i32..=(b[1][1] / bin_m).floor() as i32 {
                    bins.entry((bx, by)).or_default().push(i as u32);
                }
            }
        }
        Ok(Self {
            prims,
            boxes,
            bins,
            bin_m,
            roughness_m: sys.roughness_m,
            seed: sys.seed,
            feature_m: 6.0,
        })
    }

    fn noise(&self, p: [f32; 3]) -> f32 {
        if self.roughness_m <= 0.0 {
            return 0.0;
        }
        let f = 1.0 / self.feature_m;
        let q = [p[0] * f, p[1] * f, p[2] * f];
        let n = value_noise(self.seed, q)
            + 0.5 * value_noise(self.seed ^ 0xA5, [q[0] * 2.03, q[1] * 2.03, q[2] * 2.03])
            + 0.25 * value_noise(self.seed ^ 0x5A, [q[0] * 4.1, q[1] * 4.1, q[2] * 4.1]);
        self.roughness_m * n / 1.75
    }

    fn eval_with(&self, p: [f32; 3], cand: &[u32]) -> f32 {
        let mut d = f32::MAX;
        for &i in cand {
            d = d.min(self.prims[i as usize].sdf(p));
        }
        if d == f32::MAX { d } else { d + self.noise(p) }
    }

    /// Signed distance at `p` (metres; `f32::MAX` far from every passage).
    pub fn sdf(&self, p: [f32; 3]) -> f32 {
        let key = (
            (p[0] / self.bin_m).floor() as i32,
            (p[1] / self.bin_m).floor() as i32,
        );
        match self.bins.get(&key) {
            Some(c) => self.eval_with(p, c),
            None => f32::MAX,
        }
    }

    /// Union of the primitives' boxes, or `None` for an empty system.
    pub fn bounds(&self) -> Option<[[f32; 3]; 2]> {
        self.boxes.iter().copied().reduce(|a, b| {
            [
                [
                    a[0][0].min(b[0][0]),
                    a[0][1].min(b[0][1]),
                    a[0][2].min(b[0][2]),
                ],
                [
                    a[1][0].max(b[1][0]),
                    a[1][1].max(b[1][1]),
                    a[1][2].max(b[1][2]),
                ],
            ]
        })
    }
}

/// Terrain height at (u, v) metres, bilinear between cell centres.
pub fn ground(hf: &Heightfield, extent_m: [f64; 2], u: f32, v: f32) -> f32 {
    let (w, h) = (hf.width(), hf.height());
    let sx = (extent_m[0] / f64::from(w)) as f32;
    let sy = (extent_m[1] / f64::from(h)) as f32;
    let fx = (u / sx - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (v / sy - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let a = hf.get(x0, y0) + (hf.get(x1, y0) - hf.get(x0, y0)) * tx;
    let b = hf.get(x0, y1) + (hf.get(x1, y1) - hf.get(x0, y1)) * tx;
    a + (b - a) * ty
}

const CHUNK: i64 = 16;

/// A keyhole's slot half-width as a share of the tube's.
const KEYHOLE_SLOT: f32 = 0.45;

/// A surface-nets vertex: its cell, position (u, v, z) and normal.
type CellVertex = ([i64; 3], [f32; 3], [f32; 3]);

/// Mesh the cave walls with surface nets on a `voxel_m` grid over the skeleton, in the terrain
/// export frame; triangles wholly above the terrain (plus one voxel) are dropped so the walls stop
/// where the cave opens. Normals face into the cave.
pub fn cave_mesh(
    sys: &CaveSystem,
    hf: &Heightfield,
    extent_m: [f64; 2],
    voxel_m: f32,
) -> JdResult<GridMesh> {
    if !(voxel_m.is_finite() && voxel_m >= 0.05) {
        return Err(JdError::invalid("voxel_m", "must be ≥ 0.05 m"));
    }
    let field = CaveField::new(sys)?;
    let s = voxel_m;
    // Chunks touched by any primitive (sorted, so the vertex order is deterministic).
    let mut chunks: BTreeSet<[i64; 3]> = BTreeSet::new();
    let cell = |x: f32| (x / s).floor() as i64;
    let mut budget: u64 = 0;
    for b in &field.boxes {
        let lo = [cell(b[0][0]) - 1, cell(b[0][1]) - 1, cell(b[0][2]) - 1];
        let hi = [cell(b[1][0]) + 1, cell(b[1][1]) + 1, cell(b[1][2]) + 1];
        let span = (0..3).map(|k| (hi[k] - lo[k]) as u64).product::<u64>();
        budget = budget.saturating_add(span);
        if budget > 400_000_000 {
            return Err(JdError::invalid(
                "voxel_m",
                "the cave is too large for this voxel size (more than 400 M voxels)",
            )
            .next("Raise voxel_m (1 m suits passages a few metres wide)."));
        }
        for cx in lo[0].div_euclid(CHUNK)..=hi[0].div_euclid(CHUNK) {
            for cy in lo[1].div_euclid(CHUNK)..=hi[1].div_euclid(CHUNK) {
                for cz in lo[2].div_euclid(CHUNK)..=hi[2].div_euclid(CHUNK) {
                    chunks.insert([cx, cy, cz]);
                }
            }
        }
    }
    let chunks: Vec<[i64; 3]> = chunks.into_iter().collect();
    let n = (CHUNK + 1) as usize;
    let at = |g: [i64; 3]| [g[0] as f32 * s, g[1] as f32 * s, g[2] as f32 * s];
    // Corner samples per chunk; chunks with no primitive near them stay empty.
    let samples: Vec<Option<Vec<f32>>> = chunks
        .par_iter()
        .map(|c| {
            let o = [c[0] * CHUNK, c[1] * CHUNK, c[2] * CHUNK];
            let (lo, hi) = (at(o), at([o[0] + CHUNK, o[1] + CHUNK, o[2] + CHUNK]));
            let cand: Vec<u32> = field
                .boxes
                .iter()
                .enumerate()
                .filter(|(_, b)| (0..3).all(|k| b[0][k] <= hi[k] + s && b[1][k] >= lo[k] - s))
                .map(|(i, _)| i as u32)
                .collect();
            if cand.is_empty() {
                return None;
            }
            let mut v = Vec::with_capacity(n * n * n);
            for k in 0..n as i64 {
                for j in 0..n as i64 {
                    for i in 0..n as i64 {
                        v.push(field.eval_with(at([o[0] + i, o[1] + j, o[2] + k]), &cand));
                    }
                }
            }
            Some(v)
        })
        .collect();
    let idx = |i: i64, j: i64, k: i64| (k as usize * n + j as usize) * n + i as usize;
    // One vertex per cell with a sign change: the mean of its edge crossings.
    let per_chunk: Vec<Vec<CellVertex>> = chunks
        .par_iter()
        .zip(samples.par_iter())
        .map(|(c, smp)| {
            let Some(v) = smp else { return Vec::new() };
            let o = [c[0] * CHUNK, c[1] * CHUNK, c[2] * CHUNK];
            let mut out = Vec::new();
            for k in 0..CHUNK {
                for j in 0..CHUNK {
                    for i in 0..CHUNK {
                        let corner = |a: i64, b: i64, d: i64| v[idx(i + a, j + b, k + d)];
                        let cs = [
                            corner(0, 0, 0),
                            corner(1, 0, 0),
                            corner(0, 1, 0),
                            corner(1, 1, 0),
                            corner(0, 0, 1),
                            corner(1, 0, 1),
                            corner(0, 1, 1),
                            corner(1, 1, 1),
                        ];
                        let inside = cs.iter().filter(|x| **x < 0.0).count();
                        if inside == 0 || inside == 8 {
                            continue;
                        }
                        let mut sum = [0.0f32; 3];
                        let mut m = 0.0f32;
                        for (e0, e1) in [
                            (0, 1),
                            (2, 3),
                            (4, 5),
                            (6, 7),
                            (0, 2),
                            (1, 3),
                            (4, 6),
                            (5, 7),
                            (0, 4),
                            (1, 5),
                            (2, 6),
                            (3, 7),
                        ] {
                            let (a, b) = (cs[e0], cs[e1]);
                            if (a < 0.0) == (b < 0.0) {
                                continue;
                            }
                            let t = a / (a - b);
                            let pa = [
                                (e0 & 1) as f32,
                                ((e0 >> 1) & 1) as f32,
                                ((e0 >> 2) & 1) as f32,
                            ];
                            let pb = [
                                (e1 & 1) as f32,
                                ((e1 >> 1) & 1) as f32,
                                ((e1 >> 2) & 1) as f32,
                            ];
                            for q in 0..3 {
                                sum[q] += pa[q] + (pb[q] - pa[q]) * t;
                            }
                            m += 1.0;
                        }
                        let g = [o[0] + i, o[1] + j, o[2] + k];
                        let p = [
                            (g[0] as f32 + sum[0] / m) * s,
                            (g[1] as f32 + sum[1] / m) * s,
                            (g[2] as f32 + sum[2] / m) * s,
                        ];
                        // Gradient from the corners; the cave side is −∇f.
                        let gx =
                            (cs[1] - cs[0]) + (cs[3] - cs[2]) + (cs[5] - cs[4]) + (cs[7] - cs[6]);
                        let gy =
                            (cs[2] - cs[0]) + (cs[3] - cs[1]) + (cs[6] - cs[4]) + (cs[7] - cs[5]);
                        let gz =
                            (cs[4] - cs[0]) + (cs[5] - cs[1]) + (cs[6] - cs[2]) + (cs[7] - cs[3]);
                        out.push((g, p, norm([-gx, -gy, -gz])));
                    }
                }
            }
            out
        })
        .collect();
    let mut vid: HashMap<[i64; 3], u32> = HashMap::new();
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut nrm: Vec<[f32; 3]> = Vec::new();
    for list in &per_chunk {
        for (g, p, nv) in list {
            vid.insert(*g, pos.len() as u32);
            pos.push(*p);
            nrm.push(*nv);
        }
    }
    // Sample lookup across chunks for the edge signs.
    let chunk_of: HashMap<[i64; 3], usize> =
        chunks.iter().enumerate().map(|(i, c)| (*c, i)).collect();
    let sample = |g: [i64; 3]| -> Option<f32> {
        let c = [
            g[0].div_euclid(CHUNK),
            g[1].div_euclid(CHUNK),
            g[2].div_euclid(CHUNK),
        ];
        let v = samples[*chunk_of.get(&c)?].as_ref()?;
        let l = [
            g[0] - c[0] * CHUNK,
            g[1] - c[1] * CHUNK,
            g[2] - c[2] * CHUNK,
        ];
        Some(v[idx(l[0], l[1], l[2])])
    };
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for list in &per_chunk {
        for (g, _, _) in list {
            let f0 = match sample(*g) {
                Some(f) => f,
                None => continue,
            };
            // The three edges leaving this cell's minimum corner, each with the four cells around it.
            for (axis, ring) in [
                (0usize, [[0, 0, 0], [0, -1, 0], [0, -1, -1], [0, 0, -1]]),
                (1, [[0, 0, 0], [0, 0, -1], [-1, 0, -1], [-1, 0, 0]]),
                (2, [[0, 0, 0], [-1, 0, 0], [-1, -1, 0], [0, -1, 0]]),
            ] {
                let mut e = *g;
                e[axis] += 1;
                let Some(f1) = sample(e) else { continue };
                if (f0 < 0.0) == (f1 < 0.0) {
                    continue;
                }
                let q: Option<Vec<u32>> = ring
                    .iter()
                    .map(|d: &[i64; 3]| vid.get(&[g[0] + d[0], g[1] + d[1], g[2] + d[2]]).copied())
                    .collect();
                let Some(q) = q else { continue };
                // Wind so the triangles face the air side.
                let (a, b, c, d) = if f0 < 0.0 {
                    (q[0], q[3], q[2], q[1])
                } else {
                    (q[0], q[1], q[2], q[3])
                };
                if dist3(pos[a as usize], pos[c as usize])
                    <= dist3(pos[b as usize], pos[d as usize])
                {
                    tris.push([a, b, c]);
                    tris.push([a, c, d]);
                } else {
                    tris.push([a, b, d]);
                    tris.push([b, c, d]);
                }
            }
        }
    }
    // Taubin smoothing (λ | μ, two rounds): surface nets leave a staircase along sharp edges that
    // run diagonal to the grid (an arch's springing line); this irons it out without shrinking.
    let mut nb: Vec<Vec<u32>> = vec![Vec::new(); pos.len()];
    for t in &tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if !nb[a as usize].contains(&b) {
                nb[a as usize].push(b);
                nb[b as usize].push(a);
            }
        }
    }
    for factor in [0.5f32, -0.53, 0.5, -0.53] {
        let prev = pos.clone();
        for (i, list) in nb.iter().enumerate() {
            if list.is_empty() {
                continue;
            }
            let mut m = [0.0f32; 3];
            for &j in list {
                for q in 0..3 {
                    m[q] += prev[j as usize][q];
                }
            }
            for q in 0..3 {
                pos[i][q] = prev[i][q] + factor * (m[q] / list.len() as f32 - prev[i][q]);
            }
        }
    }
    // Drop what lies above the ground; keep the walls up to one voxel over it (the terrain hides them).
    let above = |p: [f32; 3]| p[2] > ground(hf, extent_m, p[0], p[1]) + s;
    tris.retain(|t| !t.iter().all(|&i| above(pos[i as usize])));
    Ok(to_mesh(&pos, &nrm, &tris, extent_m))
}

/// Pack (u, v, z) vertices and triangles into a mesh in the terrain export frame, dropping
/// unreferenced vertices; box-projected UVs at 4 m per tile.
fn to_mesh(pos: &[[f32; 3]], nrm: &[[f32; 3]], tris: &[[u32; 3]], extent_m: [f64; 2]) -> GridMesh {
    let mut remap = vec![u32::MAX; pos.len()];
    let (mut positions, mut normals, mut uvs, mut indices) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    let (hx, hy) = ((extent_m[0] / 2.0) as f32, (extent_m[1] / 2.0) as f32);
    for t in tris {
        for &i in t {
            if remap[i as usize] == u32::MAX {
                remap[i as usize] = (positions.len() / 3) as u32;
                let p = pos[i as usize];
                let nv = nrm[i as usize];
                let (x, y, z) = (p[0] - hx, p[2], hy - p[1]);
                positions.extend_from_slice(&[x, y, z]);
                normals.extend_from_slice(&[nv[0], nv[2], -nv[1]]);
                let (ax, ay, az) = (nv[0].abs(), nv[2].abs(), nv[1].abs());
                let uv = if ay >= ax && ay >= az {
                    [x, z]
                } else if ax >= az {
                    [z, y]
                } else {
                    [x, y]
                };
                uvs.extend_from_slice(&[uv[0] / 4.0, uv[1] / 4.0]);
                lo = lo.min(y);
                hi = hi.max(y);
            }
            indices.push(remap[i as usize]);
        }
    }
    if positions.is_empty() {
        (lo, hi) = (0.0, 0.0);
    }
    GridMesh {
        positions,
        normals,
        uvs,
        indices,
        grid: [0, 0],
        height_range: [lo, hi],
    }
}

/// The terrain's hole mask over the entrances: 1 where the void reaches the ground surface by
/// more than a quarter cell (so the terrain keeps a lip over the cave mouth), else 0.
pub fn entrance_holes(
    sys: &CaveSystem,
    hf: &Heightfield,
    extent_m: [f64; 2],
) -> JdResult<Heightfield> {
    let field = CaveField::new(sys)?;
    let (w, h) = (hf.width(), hf.height());
    let sx = (extent_m[0] / f64::from(w)) as f32;
    let sy = (extent_m[1] / f64::from(h)) as f32;
    let lip = 0.25 * sx.min(sy);
    let mut out = Heightfield::new(w, h, hf.tile_size(), hf.halo())?;
    out.fill_with(|x, y| {
        let (u, v) = ((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
        if field.sdf([u, v, hf.get(x, y)]) < -lip {
            1.0
        } else {
            0.0
        }
    });
    Ok(out)
}

/// The cave's footprint seen from above: 1 over any passage or chamber, else 0 (a map overlay of
/// what lies underground).
pub fn cave_plan(sys: &CaveSystem, hf: &Heightfield, extent_m: [f64; 2]) -> JdResult<Heightfield> {
    sys.validate()?;
    let (w, h) = (hf.width(), hf.height());
    let sx = (extent_m[0] / f64::from(w)) as f32;
    let sy = (extent_m[1] / f64::from(h)) as f32;
    let mut cov = vec![0u8; (w as usize) * (h as usize)];
    let mut paint = |lo: [f32; 2], hi: [f32; 2], inside: &dyn Fn(f32, f32) -> bool| {
        let x0 = ((lo[0] / sx - 0.5).floor().max(0.0) as u32).min(w - 1);
        let x1 = ((hi[0] / sx - 0.5).ceil().max(0.0) as u32).min(w - 1);
        let y0 = ((lo[1] / sy - 0.5).floor().max(0.0) as u32).min(h - 1);
        let y1 = ((hi[1] / sy - 0.5).ceil().max(0.0) as u32).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = ((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
                if inside(u, v) {
                    cov[(y * w + x) as usize] = 1;
                }
            }
        }
    };
    for ps in &sys.passages {
        for i in 0..ps.points.len() - 1 {
            let (a, b) = (ps.points[i], ps.points[i + 1]);
            let r = ps.half_width_m[i].max(ps.half_width_m[i + 1]);
            let lo = [a[0].min(b[0]) - r, a[1].min(b[1]) - r];
            let hi = [a[0].max(b[0]) + r, a[1].max(b[1]) + r];
            paint(lo, hi, &|u, v| {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l2 = dx * dx + dy * dy;
                let t = if l2 < 1e-9 {
                    0.0
                } else {
                    (((u - a[0]) * dx + (v - a[1]) * dy) / l2).clamp(0.0, 1.0)
                };
                (u - a[0] - t * dx).hypot(v - a[1] - t * dy) <= r
            });
        }
    }
    for c in &sys.chambers {
        let (cx, cy, rx, ry) = (c.center[0], c.center[1], c.radii_m[0], c.radii_m[1]);
        paint([cx - rx, cy - ry], [cx + rx, cy + ry], &|u, v| {
            ((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2) <= 1.0
        });
    }
    let mut out = Heightfield::new(w, h, hf.tile_size(), hf.halo())?;
    out.fill_with(|x, y| f32::from(cov[(y * w + x) as usize]));
    Ok(out)
}

/// Estimated wall area, m²: tubes as cylinders, chambers as ellipsoids.
pub fn wall_area_m2(sys: &CaveSystem) -> f32 {
    let tau = std::f32::consts::TAU;
    let tubes: f32 = sys
        .passages
        .iter()
        .flat_map(|p| (0..p.points.len() - 1).map(move |i| (p, i)))
        .map(|(p, i)| {
            dist3(p.points[i], p.points[i + 1])
                * tau
                * 0.5
                * (p.half_width_m[i] + p.half_height_m[i])
        })
        .sum();
    let rooms: f32 = sys
        .chambers
        .iter()
        .map(|c| {
            2.0 * tau
                * (c.radii_m[0] * c.radii_m[1]
                    + c.radii_m[1] * c.radii_m[2]
                    + c.radii_m[0] * c.radii_m[2])
                / 3.0
        })
        .sum();
    tubes + rooms
}

/// A voxel size that puts about five voxels across the narrowest passage (0.25..=2 m), coarsened
/// so the walls stay near `max_triangles` (surface nets on rough walls: about four triangles per
/// voxel² of the smooth-tube estimate, measured on the karst demo).
pub fn auto_voxel_for(sys: &CaveSystem, max_triangles: u32) -> f32 {
    let fine = auto_voxel_m(sys);
    let budget = (4.0 * wall_area_m2(sys) / max_triangles.max(1000) as f32).sqrt();
    fine.max(budget)
}

/// A voxel size that puts about five voxels across the narrowest passage, 0.25..=2 m.
pub fn auto_voxel_m(sys: &CaveSystem) -> f32 {
    let narrow = sys
        .passages
        .iter()
        .flat_map(|p| {
            let slot = if p.profile == Profile::Keyhole {
                KEYHOLE_SLOT
            } else {
                1.0
            };
            p.half_width_m
                .iter()
                .zip(&p.half_height_m)
                .map(move |(w, h)| (w * slot).min(*h))
        })
        .fold(f32::MAX, f32::min);
    if narrow == f32::MAX {
        1.0
    } else {
        (narrow * 2.0 / 5.0).clamp(0.25, 2.0)
    }
}

/// Append mesh `b` to mesh `a`.
pub fn append_mesh(a: &mut GridMesh, b: &GridMesh) {
    let base = (a.positions.len() / 3) as u32;
    a.positions.extend_from_slice(&b.positions);
    a.normals.extend_from_slice(&b.normals);
    a.uvs.extend_from_slice(&b.uvs);
    a.indices.extend(b.indices.iter().map(|i| i + base));
    if !b.positions.is_empty() {
        a.height_range = if a.positions.len() == b.positions.len() {
            b.height_range
        } else {
            [
                a.height_range[0].min(b.height_range[0]),
                a.height_range[1].max(b.height_range[1]),
            ]
        };
    }
}

fn rand01(seed: u64, i: u64) -> f32 {
    unit(mix(seed ^ mix(i)))
}

/// Ceiling and floor heights above and below `p` inside the void (bisection on the field).
fn ceiling_floor(field: &CaveField, p: [f32; 3], reach: f32) -> Option<(f32, f32)> {
    if field.sdf(p) >= 0.0 {
        return None;
    }
    let find = |dir: f32| {
        let (mut a, mut b) = (0.0f32, None);
        let step = 0.25f32;
        let mut t = step;
        while t <= reach {
            if field.sdf([p[0], p[1], p[2] + dir * t]) >= 0.0 {
                b = Some(t);
                break;
            }
            a = t;
            t += step;
        }
        let mut b = b?;
        for _ in 0..12 {
            let m = 0.5 * (a + b);
            if field.sdf([p[0], p[1], p[2] + dir * m]) >= 0.0 {
                b = m
            } else {
                a = m
            }
        }
        Some(p[2] + dir * 0.5 * (a + b))
    };
    Some((find(1.0)?, find(-1.0)?))
}

/// Speleothem settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpeleothemParams {
    /// Drip sites per 100 m² of air-filled floor, ≥ 0 (0 = none).
    pub per_100m2: f32,
    /// Longest stalactite, metres.
    pub max_length_m: f32,
    /// Share of drip sites with a stalagmite under the stalactite, 0..=1.
    pub stalagmite_share: f32,
}

impl Default for SpeleothemParams {
    fn default() -> Self {
        Self {
            per_100m2: 4.0,
            max_length_m: 2.5,
            stalagmite_share: 0.7,
        }
    }
}

/// Place speleothems in the air-filled parts (above the water level) of a system's passages and
/// chambers: a stalactite at each drip site on the ceiling, a stalagmite on the floor below it for
/// `stalagmite_share` of them, a column where the two would meet.
pub fn place_speleothems(
    sys: &CaveSystem,
    p: &SpeleothemParams,
    seed: u64,
) -> JdResult<Vec<Speleothem>> {
    if !(p.per_100m2.is_finite()
        && p.per_100m2 >= 0.0
        && p.max_length_m.is_finite()
        && p.max_length_m > 0.0
        && (0.0..=1.0).contains(&p.stalagmite_share))
    {
        return Err(JdError::invalid(
            "speleothems",
            "per_100m2 ≥ 0, max_length_m > 0, stalagmite_share within 0..=1",
        ));
    }
    let field = CaveField::new(sys)?;
    let water = sys.water_level_m.unwrap_or(f32::MIN);
    // Candidate drip sites: points scattered over each segment's and chamber's floor area.
    let mut sites: Vec<[f32; 3]> = Vec::new();
    let mut centres: Vec<[f32; 3]> = Vec::new();
    let mut k = 0u64;
    let mut scatter = |c: [f32; 3],
                       area: f32,
                       dx: [f32; 2],
                       sites: &mut Vec<[f32; 3]>,
                       centres: &mut Vec<[f32; 3]>| {
        let expect = area * p.per_100m2 / 100.0;
        let n = expect.floor() as u64 + u64::from(rand01(seed, k) < expect.fract());
        k += 1;
        for _ in 0..n {
            let (a, b) = (rand01(seed, k) * 2.0 - 1.0, rand01(seed, k + 1) * 2.0 - 1.0);
            k += 2;
            sites.push([c[0] + a * dx[0], c[1] + b * dx[1], c[2]]);
            centres.push(c);
        }
    };
    for ps in &sys.passages {
        for i in 0..ps.points.len() - 1 {
            let (a, b) = (ps.points[i], ps.points[i + 1]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            let w = 0.5 * (ps.half_width_m[i] + ps.half_width_m[i + 1]);
            if len < 0.5 {
                continue;
            }
            // Segment area as a run of short pieces.
            let pieces = (len / (2.0 * w)).ceil().max(1.0) as usize;
            for q in 0..pieces {
                let t = (q as f32 + 0.5) / pieces as f32;
                let c = [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ];
                scatter(
                    c,
                    len / pieces as f32 * 2.0 * w,
                    [0.6 * w, 0.6 * w],
                    &mut sites,
                    &mut centres,
                );
            }
        }
    }
    for c in &sys.chambers {
        scatter(
            c.center,
            std::f32::consts::PI * c.radii_m[0] * c.radii_m[1],
            [0.7 * c.radii_m[0], 0.7 * c.radii_m[1]],
            &mut sites,
            &mut centres,
        );
    }
    let mut out = Vec::new();
    for (i, s) in sites.iter().enumerate() {
        let i = i as u64;
        // A site off to the side of a narrow slot falls outside the cave: drip from the axis instead.
        let Some((ceil, floor)) = ceiling_floor(&field, *s, 80.0)
            .or_else(|| ceiling_floor(&field, centres[i as usize], 80.0))
        else {
            continue;
        };
        let s = if field.sdf(*s) < 0.0 {
            s
        } else {
            &centres[i as usize]
        };
        if floor < water || ceil - floor < 0.6 {
            continue; // flooded, or too low to stand anything in
        }
        let gap = ceil - floor;
        // Most stalactites are short: length ∝ r², capped by the gap.
        let r1 = rand01(seed ^ 0x51, i);
        let lt = (p.max_length_m * (0.15 + 0.85 * r1 * r1)).min(0.6 * gap);
        let x = [s[0], s[1], ceil];
        if rand01(seed ^ 0x52, i) < p.stalagmite_share {
            let lm = (lt * (0.5 + rand01(seed ^ 0x53, i))).min(gap);
            if lt + lm >= 0.9 * gap {
                out.push(Speleothem {
                    kind: SpeleothemKind::Column,
                    at: x,
                    length_m: gap,
                    radius_m: (0.12 * gap).clamp(0.1, 1.5),
                });
                continue;
            }
            out.push(Speleothem {
                kind: SpeleothemKind::Stalagmite,
                at: [s[0], s[1], floor],
                length_m: lm,
                radius_m: (0.25 * lm).clamp(0.04, 1.2),
            });
        }
        out.push(Speleothem {
            kind: SpeleothemKind::Stalactite,
            at: x,
            length_m: lt,
            radius_m: (0.12 * lt).clamp(0.02, 0.6),
        });
    }
    Ok(out)
}

/// Speleothems as one mesh in the terrain export frame: tapered, slightly irregular 8-sided cones
/// (stalactites point down, stalagmites up, columns pinch at the waist), rooted 0.2 m into the rock.
pub fn speleothem_mesh(list: &[Speleothem], extent_m: [f64; 2], seed: u64) -> GridMesh {
    const SIDES: usize = 8;
    const RINGS: usize = 5;
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut nrm: Vec<[f32; 3]> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for (n, s) in list.iter().enumerate() {
        // Rooted 0.2 m into the rock; a column is rooted at both ends.
        let (dir, extra) = match s.kind {
            SpeleothemKind::Stalagmite => (1.0f32, 0.2f32),
            SpeleothemKind::Stalactite => (-1.0, 0.2),
            SpeleothemKind::Column => (-1.0, 0.4),
        };
        let base = pos.len() as u32;
        let (root_z, total) = (s.at[2] - dir * 0.2, s.length_m + extra);
        for r in 0..=RINGS {
            let t = r as f32 / RINGS as f32;
            let z = root_z + dir * t * total;
            let rad = match s.kind {
                SpeleothemKind::Column => s.radius_m * (0.55 + 0.45 * (2.0 * t - 1.0).abs()),
                _ => s.radius_m * (1.0 - t).powf(0.8).max(0.02),
            };
            let wob = 1.0 + 0.15 * (rand01(seed ^ n as u64, r as u64) - 0.5);
            for k in 0..SIDES {
                let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
                let (c, sn) = (a.cos(), a.sin());
                pos.push([s.at[0] + c * rad * wob, s.at[1] + sn * rad * wob, z]);
                nrm.push([c, sn, 0.0]);
            }
        }
        for r in 0..RINGS as u32 {
            for k in 0..SIDES as u32 {
                let (a, b) = (
                    base + r * SIDES as u32 + k,
                    base + r * SIDES as u32 + (k + 1) % SIDES as u32,
                );
                let (c, d) = (a + SIDES as u32, b + SIDES as u32);
                if dir > 0.0 {
                    tris.push([a, b, d]);
                    tris.push([a, d, c]);
                } else {
                    tris.push([a, d, b]);
                    tris.push([a, c, d]);
                }
            }
        }
    }
    to_mesh(&pos, &nrm, &tris, extent_m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(z: f32, px: u32) -> Heightfield {
        let mut hf = Heightfield::new(px, px, 64, 2).expect("hf");
        hf.fill_with(|_, _| z);
        hf
    }

    fn tube(points: Vec<[f32; 3]>, w: f32, h: f32, profile: Profile) -> Passage {
        let n = points.len();
        Passage {
            points,
            half_width_m: vec![w; n],
            half_height_m: vec![h; n],
            profile,
        }
    }

    #[test]
    fn a_buried_tube_meshes_closed_at_its_size_facing_inward() {
        let hf = flat(100.0, 64);
        let sys = CaveSystem {
            passages: vec![tube(
                vec![[20.0, 32.0, 50.0], [44.0, 32.0, 50.0]],
                3.0,
                2.0,
                Profile::Tube,
            )],
            ..Default::default()
        };
        let m = cave_mesh(&sys, &hf, [64.0, 64.0], 0.5).expect("mesh");
        let t = m.topology();
        assert!(m.indices.len() > 300, "{} indices", m.indices.len());
        assert_eq!((t.boundary_edges, t.non_manifold_edges), (0, 0), "{t:?}");
        // Width 2 × 3 m across (mesh z = 32 − v), height 2 × 2 m, within a voxel.
        let p = m.positions.as_chunks::<3>().0;
        let span = |k: usize| {
            p.iter().map(|v| v[k]).fold(f32::MIN, f32::max)
                - p.iter().map(|v| v[k]).fold(f32::MAX, f32::min)
        };
        assert!((span(2) - 6.0).abs() < 0.6, "width {}", span(2));
        assert!((span(1) - 4.0).abs() < 0.6, "height {}", span(1));
        // Normals point at the axis (the air), i.e. against the outward offset.
        let n = m.normals.as_chunks::<3>().0;
        let inward = p
            .iter()
            .zip(n)
            .filter(|(v, nv)| (v[1] - 50.0) * nv[1] + v[2] * nv[2] < 0.0)
            .count();
        assert!(
            inward as f32 > 0.95 * p.len() as f32,
            "{inward}/{}",
            p.len()
        );
        // Triangle winding agrees with the normals.
        let agree = m
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| {
                let [a, b, c] = t.map(|i| p[i as usize]);
                let fnrm = cross(sub(b, a), sub(c, a));
                let vn = n[t[0] as usize];
                dot(fnrm, vn) > 0.0
            })
            .count();
        assert!(
            agree as f32 > 0.95 * (m.indices.len() / 3) as f32,
            "{agree}"
        );
    }

    #[test]
    fn an_arch_drift_has_a_flat_floor_and_a_round_roof() {
        let sys = CaveSystem {
            passages: vec![tube(
                vec![[10.0, 32.0, 50.0], [54.0, 32.0, 50.0]],
                1.5,
                1.5,
                Profile::Arch,
            )],
            ..Default::default()
        };
        let field = CaveField::new(&sys).expect("field");
        // Floor at 48.5 flat across the width; roof at 51.5 only on the axis, lower at the walls.
        for x in [-1.2f32, 0.0, 1.2] {
            assert!(
                field.sdf([30.0, 32.0 + x, 48.6]) < 0.0 && field.sdf([30.0, 32.0 + x, 48.4]) > 0.0,
                "floor at {x}"
            );
        }
        assert!(field.sdf([30.0, 32.0, 51.4]) < 0.0);
        assert!(
            field.sdf([30.0, 33.3, 51.0]) > 0.0,
            "the roof curves down at the wall"
        );
        // Keyhole: the slot under the tube is narrower than the tube.
        let k = CaveField::new(&CaveSystem {
            passages: vec![tube(
                vec![[10.0, 32.0, 50.0], [54.0, 32.0, 50.0]],
                2.0,
                4.0,
                Profile::Keyhole,
            )],
            ..Default::default()
        })
        .expect("k");
        assert!(k.sdf([30.0, 33.5, 51.5]) < 0.0, "tube part is wide");
        assert!(
            k.sdf([30.0, 33.5, 47.0]) > 0.0 && k.sdf([30.0, 32.3, 47.0]) < 0.0,
            "slot part is narrow"
        );
    }

    #[test]
    fn the_terrain_opens_only_where_the_cave_breaks_the_surface() {
        // A passage climbing from 30 m below a 100 m plain to above it at its east end.
        let hf = flat(100.0, 64);
        let sys = CaveSystem {
            passages: vec![tube(
                vec![[10.0, 32.0, 70.0], [40.0, 32.0, 75.0], [56.0, 32.0, 104.0]],
                3.0,
                3.0,
                Profile::Tube,
            )],
            entrances: vec![Entrance {
                at: [56.0, 32.0, 100.0],
                kind: EntranceKind::Portal,
                underwater: false,
            }],
            roughness_m: 0.4,
            seed: 3,
            ..Default::default()
        };
        let holes = entrance_holes(&sys, &hf, [64.0, 64.0]).expect("holes");
        let cells: Vec<(u32, u32)> = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| holes.get(x, y) > 0.5)
            .collect();
        assert!(!cells.is_empty(), "the entrance opens the terrain");
        assert!(
            cells
                .iter()
                .all(|&(x, y)| x >= 44 && (26..=38).contains(&y)),
            "{cells:?}"
        );
        // The mesh is open there (boundary edges) and stops at the ground.
        let m = cave_mesh(&sys, &hf, [64.0, 64.0], 0.5).expect("mesh");
        assert!(m.topology().boundary_edges > 0);
        assert!(m.height_range[1] <= 100.0 + 1.5, "{:?}", m.height_range);
    }

    #[test]
    fn speleothems_hang_from_ceilings_stand_under_drips_and_stay_dry() {
        let sys = CaveSystem {
            chambers: vec![Chamber {
                center: [32.0, 32.0, 50.0],
                radii_m: [14.0, 10.0, 6.0],
            }],
            passages: vec![tube(
                vec![[0.0, 32.0, 44.0], [20.0, 32.0, 44.0]],
                2.0,
                1.5,
                Profile::Tube,
            )],
            water_level_m: Some(45.0),
            ..Default::default()
        };
        let list = place_speleothems(
            &sys,
            &SpeleothemParams {
                per_100m2: 8.0,
                ..Default::default()
            },
            7,
        )
        .expect("place");
        let field = CaveField::new(&sys).expect("field");
        assert!(list.len() > 10, "{}", list.len());
        for s in &list {
            match s.kind {
                SpeleothemKind::Stalactite | SpeleothemKind::Column => assert!(
                    field.sdf([s.at[0], s.at[1], s.at[2] + 0.1]) > 0.0
                        && field.sdf([s.at[0], s.at[1], s.at[2] - 0.1]) < 0.0,
                    "anchored on a ceiling {s:?}"
                ),
                SpeleothemKind::Stalagmite => {
                    assert!(s.at[2] >= 45.0, "dry floor {s:?}");
                    assert!(
                        list.iter().any(|t| t.kind == SpeleothemKind::Stalactite
                            && (t.at[0] - s.at[0]).abs() < 1e-4
                            && (t.at[1] - s.at[1]).abs() < 1e-4),
                        "under a stalactite"
                    );
                }
            }
        }
        // The flooded passage (floor ≈ 42.5 < 45) has none.
        assert!(
            list.iter()
                .all(|s| s.at[0] > 17.0 || s.kind != SpeleothemKind::Stalagmite)
        );
        let m = speleothem_mesh(&list, [64.0, 64.0], 1);
        assert_eq!(m.indices.len(), list.len() * 5 * 8 * 6);
    }

    #[test]
    fn meshing_is_deterministic_and_refuses_absurd_voxels() {
        let hf = flat(100.0, 32);
        let sys = CaveSystem {
            passages: vec![tube(
                vec![[4.0, 16.0, 60.0], [28.0, 18.0, 58.0]],
                2.0,
                2.0,
                Profile::Tube,
            )],
            roughness_m: 0.6,
            seed: 9,
            ..Default::default()
        };
        let a = cave_mesh(&sys, &hf, [32.0, 32.0], 0.5).expect("a");
        let b = cave_mesh(&sys, &hf, [32.0, 32.0], 0.5).expect("b");
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.indices, b.indices);
        assert!(cave_mesh(&sys, &hf, [32.0, 32.0], 0.01).is_err());
        let huge = CaveSystem {
            passages: vec![tube(
                vec![[0.0, 0.0, 0.0], [100_000.0, 0.0, 0.0]],
                50.0,
                50.0,
                Profile::Tube,
            )],
            ..Default::default()
        };
        assert!(cave_mesh(&huge, &hf, [32.0, 32.0], 0.1).is_err());
    }
}
