//! Cave generators: one skeleton builder per kind of cave, each driven by the terrain it sits in
//! (`docs/research/caves.md` for the geology and the numbers). They return a [`CaveSystem`] that
//! [`crate::caves`] turns into a field, a mesh and the terrain's entrance holes.
//!
//! - **karst**: solution caves. Water sinks at sinkholes on high ground and comes out at a spring
//!   at base level; the passages are the cheapest paths between them through a lattice laid on
//!   inception horizons, cheaper along two fracture sets — a branchwork, as most limestone caves
//!   are. Below the water table the passages are round phreatic tubes and flooded; above it a
//!   vadose stream cuts a keyhole; chambers open at junctions; speleothems grow in the dry parts.
//! - **lava_tube**: under a lava flow's path, a few metres below the surface, with skylights
//!   where the roof fell in.
//! - **sea_cave**: wave-cut at sea level into cliffs, tall and narrow along a joint, flooded floor;
//!   an arch when it breaks through a headland.
//! - **mountain**: caves under hills and in mountains — wandering tunnels from a slope inward,
//!   keeping some rock overhead, with side branches and chambers.
//! - **mine**: an adit driven into the hill rising 1 in 200 so it drains, crosscuts off it, deeper
//!   levels joined by a shaft that also reaches the surface.
//! - **tunnel**: a road tunnel between two portals, straight, with a grade check.
//! - **catacombs**: a shallow maze of narrow galleries with rooms, reached by a stair shaft.
//!
//! Water: an optional water-depth layer (a lake, river or sea depth) floods what lies under its
//! surface and puts springs and cave mouths under water.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use jd_types::{JdError, JdResult};
use serde::{Deserialize, Serialize};

use crate::caves::{
    CaveSystem, Chamber, Entrance, EntranceKind, Passage, Profile, SpeleothemParams, ground,
    place_speleothems,
};
use crate::heightfield::Heightfield;
use crate::rocks::{mix, unit, value_noise};

/// Kind of cave.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CaveKind {
    /// Limestone solution cave: sinkholes → spring, tubes, canyons, chambers, speleothems.
    #[default]
    Karst,
    /// Lava tube under a flow path, with skylights.
    LavaTube,
    /// Sea cave cut at sea level into a cliff (an arch through a headland).
    SeaCave,
    /// Wandering caves under hills and in mountains.
    Mountain,
    /// A mine: adit, crosscuts, levels, shaft.
    Mine,
    /// A road tunnel between two portals.
    Tunnel,
    /// Catacombs: a shallow maze of galleries and rooms.
    Catacombs,
}

/// Cave settings (the editable cave: a `jd.cave.system@1` node's params).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaveParams {
    /// Kind of cave.
    #[serde(default)]
    pub kind: CaveKind,
    /// Seed.
    #[serde(default)]
    pub seed: u64,
    /// Entrances you place, `[u, v]` metres (east, north): karst sinkholes (inlets), the vent of a
    /// lava tube, sea-cave or mountain-cave mouths, a mine's portal, a tunnel's two portals, the
    /// catacombs' stair. Empty: chosen from the terrain.
    #[serde(default)]
    pub entrances: Vec<[f32; 2]>,
    /// Passage size multiplier, 0.25..=4.
    #[serde(default = "one")]
    pub size: f32,
    /// How far the system reaches, metres (karst spread, tube / cave / adit length, catacomb side).
    #[serde(default = "d_reach")]
    pub reach_m: f32,
    /// Rock over the passages, metres (0 = the kind's default).
    #[serde(default)]
    pub cover_m: f32,
    /// Branching: karst inlets when none are placed, mountain branches, mine crosscuts, sea caves.
    #[serde(default = "d_complexity")]
    pub complexity: u32,
    /// Levels: karst inception horizons, mine levels.
    #[serde(default = "d_levels")]
    pub levels: u32,
    /// Sea level, metres (sea caves and flooding when no water layer is wired).
    #[serde(default)]
    pub sea_level_m: f32,
    /// Speleothem drip sites per 100 m² of dry floor (negative = the kind's default).
    #[serde(default = "minus_one")]
    pub speleothems: f32,
    /// Wall roughness 0..=2 (1 = the kind's natural roughness).
    #[serde(default = "one")]
    pub roughness: f32,
}

fn one() -> f32 {
    1.0
}
fn minus_one() -> f32 {
    -1.0
}
fn d_reach() -> f32 {
    400.0
}
fn d_complexity() -> u32 {
    3
}
fn d_levels() -> u32 {
    2
}

impl Default for CaveParams {
    fn default() -> Self {
        Self {
            kind: CaveKind::Karst,
            seed: 0,
            entrances: Vec::new(),
            size: 1.0,
            reach_m: d_reach(),
            cover_m: 0.0,
            complexity: d_complexity(),
            levels: d_levels(),
            sea_level_m: 0.0,
            speleothems: -1.0,
            roughness: 1.0,
        }
    }
}

impl CaveParams {
    /// Range checks.
    pub fn validate(&self) -> JdResult<()> {
        let bad = |k: &str, m: &str| Err(JdError::invalid(k, m));
        if !(0.25..=4.0).contains(&self.size) {
            return bad("size", "must be within 0.25..=4");
        }
        if !(20.0..=20_000.0).contains(&self.reach_m) {
            return bad("reach_m", "must be within 20..=20000");
        }
        if !(self.cover_m.is_finite() && (0.0..=2000.0).contains(&self.cover_m)) {
            return bad("cover_m", "must be within 0..=2000");
        }
        if self.complexity > 64 || !(1..=6).contains(&self.levels) {
            return bad("complexity", "complexity ≤ 64 and levels within 1..=6");
        }
        if !(self.speleothems.is_finite() && self.speleothems <= 50.0)
            || !(0.0..=2.0).contains(&self.roughness)
            || !self.sea_level_m.is_finite()
        {
            return bad(
                "speleothems",
                "speleothems ≤ 50 per 100 m², roughness within 0..=2, sea_level_m finite",
            );
        }
        if self.entrances.iter().flatten().any(|c| !c.is_finite()) {
            return bad("entrances", "must be finite");
        }
        Ok(())
    }
}

/// The terrain a cave is generated in.
pub struct Ground<'a> {
    /// Heights, metres.
    pub hf: &'a Heightfield,
    /// World size, metres.
    pub extent_m: [f64; 2],
    /// Water depth over the ground, metres (lakes, rivers, the sea), when known.
    pub water: Option<&'a Heightfield>,
}

impl Ground<'_> {
    fn z(&self, u: f32, v: f32) -> f32 {
        ground(self.hf, self.extent_m, u, v)
    }
    fn cell(&self) -> f32 {
        (self.extent_m[0] / f64::from(self.hf.width())) as f32
    }
    fn size(&self) -> [f32; 2] {
        [self.extent_m[0] as f32, self.extent_m[1] as f32]
    }
    fn inside(&self, u: f32, v: f32) -> bool {
        let s = self.size();
        u >= 0.0 && v >= 0.0 && u <= s[0] && v <= s[1]
    }
    /// Water depth at (u, v), metres.
    fn water(&self, u: f32, v: f32) -> f32 {
        self.water
            .map_or(0.0, |w| ground(w, self.extent_m, u, v).max(0.0))
    }
    /// Uphill gradient (dz/du, dz/dv).
    fn grad(&self, u: f32, v: f32) -> [f32; 2] {
        let d = self.cell().max(0.5);
        [
            (self.z(u + d, v) - self.z(u - d, v)) / (2.0 * d),
            (self.z(u, v + d) - self.z(u, v - d)) / (2.0 * d),
        ]
    }
    fn slope_deg(&self, u: f32, v: f32) -> f32 {
        let g = self.grad(u, v);
        (g[0].hypot(g[1])).atan().to_degrees()
    }
    /// Cells as (u, v) centres every `stride` cells.
    fn samples(&self, stride: u32) -> Vec<[f32; 2]> {
        let c = self.cell();
        let (w, h) = (self.hf.width(), self.hf.height());
        let mut out = Vec::new();
        for y in (0..h).step_by(stride.max(1) as usize) {
            for x in (0..w).step_by(stride.max(1) as usize) {
                out.push([(x as f32 + 0.5) * c, (y as f32 + 0.5) * c]);
            }
        }
        out
    }
}

struct Rng {
    seed: u64,
    i: u64,
}
impl Rng {
    fn new(seed: u64, salt: u64) -> Self {
        Self {
            seed: mix(seed ^ mix(salt)),
            i: 0,
        }
    }
    fn f(&mut self) -> f32 {
        self.i += 1;
        unit(mix(self.seed ^ mix(self.i)))
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

/// Pick up to `n` points from `cands` (sorted best first), at least `spacing` apart.
fn spread(cands: &[[f32; 2]], n: usize, spacing: f32) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = Vec::new();
    for c in cands {
        if out.len() >= n {
            break;
        }
        if out
            .iter()
            .all(|o| (o[0] - c[0]).hypot(o[1] - c[1]) >= spacing)
        {
            out.push(*c);
        }
    }
    out
}

/// A generated cave with what the generator wants you to know.
pub struct Generated {
    /// The cave.
    pub system: CaveSystem,
    /// Notes (a steep tunnel grade, a mouth that is not on a slope, …).
    pub notes: Vec<String>,
}

/// Generate a cave of `p.kind` in `g`.
pub fn generate_cave(p: &CaveParams, g: &Ground) -> JdResult<Generated> {
    p.validate()?;
    let mut notes = Vec::new();
    let mut sys = match p.kind {
        CaveKind::Karst => karst(p, g, &mut notes)?,
        CaveKind::LavaTube => lava_tube(p, g, &mut notes)?,
        CaveKind::SeaCave => sea_cave(p, g, &mut notes)?,
        CaveKind::Mountain => mountain(p, g, &mut notes)?,
        CaveKind::Mine => mine(p, g, &mut notes)?,
        CaveKind::Tunnel => tunnel(p, g, &mut notes)?,
        CaveKind::Catacombs => catacombs(p, g, &mut notes)?,
    };
    sys.seed = p.seed;
    // A mouth narrower than a terrain cell cannot open the terrain: widen it to about one cell.
    let cell = g.cell();
    for e in sys.entrances.clone() {
        let r = 0.8 * cell;
        let widest = sys
            .passages
            .iter()
            .filter(|q| {
                q.points
                    .iter()
                    .any(|x| (x[0] - e.at[0]).hypot(x[1] - e.at[1]) < cell)
            })
            .flat_map(|q| q.half_width_m.iter().copied())
            .fold(0.0f32, f32::max);
        if widest < r {
            sys.chambers.push(Chamber {
                center: [e.at[0], e.at[1], e.at[2]],
                radii_m: [r, r, r.max(2.0)],
            });
        }
    }
    let (natural_rough, spel) = match p.kind {
        CaveKind::Karst => (
            0.35,
            SpeleothemParams {
                per_100m2: 3.0,
                max_length_m: 2.5,
                stalagmite_share: 0.7,
            },
        ),
        CaveKind::Mountain => (
            0.35,
            SpeleothemParams {
                per_100m2: 1.5,
                max_length_m: 1.8,
                stalagmite_share: 0.6,
            },
        ),
        CaveKind::LavaTube => (
            0.2,
            SpeleothemParams {
                per_100m2: 2.0,
                max_length_m: 0.25,
                stalagmite_share: 0.15,
            },
        ),
        CaveKind::SeaCave => (
            0.3,
            SpeleothemParams {
                per_100m2: 0.0,
                ..Default::default()
            },
        ),
        _ => (
            0.03,
            SpeleothemParams {
                per_100m2: 0.0,
                ..Default::default()
            },
        ),
    };
    let typical_w = sys
        .passages
        .iter()
        .flat_map(|q| q.half_width_m.iter().copied())
        .fold(0.0f32, f32::max)
        .max(1.0);
    sys.roughness_m = natural_rough * typical_w.min(4.0) * p.roughness;
    let spel = SpeleothemParams {
        per_100m2: if p.speleothems >= 0.0 {
            p.speleothems
        } else {
            spel.per_100m2
        },
        ..spel
    };
    if spel.per_100m2 > 0.0 {
        sys.speleothems = place_speleothems(&sys, &spel, p.seed ^ 0x5bee)?;
    }
    Ok(Generated { system: sys, notes })
}

/// Points of a 0..1 mask (cells ≥ 0.5), strongest first, at least `spacing_m` apart, at most `max`
/// — the karst node's `sinkholes` mask becomes the cave's inlets this way.
pub fn mask_points(
    mask: &Heightfield,
    extent_m: [f64; 2],
    max: usize,
    spacing_m: f32,
) -> Vec<[f32; 2]> {
    let (w, h) = (mask.width(), mask.height());
    let (sx, sy) = (
        (extent_m[0] / f64::from(w)) as f32,
        (extent_m[1] / f64::from(h)) as f32,
    );
    let mut c: Vec<(f32, [f32; 2])> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let m = mask.get(x, y);
            if m >= 0.5 {
                c.push((m, [(x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy]));
            }
        }
    }
    c.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then(a.1[1].total_cmp(&b.1[1]))
            .then(a.1[0].total_cmp(&b.1[0]))
    });
    let pts: Vec<[f32; 2]> = c.into_iter().map(|x| x.1).collect();
    spread(&pts, max, spacing_m)
}

/// Generate a cave from a node's inputs: the terrain, an optional water depth and, for karst
/// without placed sinkholes, an optional inlet mask.
pub fn cave_in_terrain(
    p: &CaveParams,
    hf: &Heightfield,
    extent_m: [f64; 2],
    water: Option<&Heightfield>,
    inlets: Option<&Heightfield>,
) -> JdResult<Generated> {
    let mut q = p.clone();
    if q.entrances.is_empty()
        && q.kind == CaveKind::Karst
        && let Some(m) = inlets
    {
        q.entrances = mask_points(
            m,
            extent_m,
            q.complexity.max(1) as usize,
            (0.15 * q.reach_m).max(20.0),
        );
    }
    generate_cave(
        &q,
        &Ground {
            hf,
            extent_m,
            water,
        },
    )
}

fn passage(points: Vec<[f32; 3]>, w: f32, h: f32, profile: Profile) -> Passage {
    let n = points.len();
    Passage {
        points,
        half_width_m: vec![w; n],
        half_height_m: vec![h; n],
        profile,
    }
}

/// Subdivide a polyline into pieces of about `step` metres and push interior points sideways by
/// smooth noise of amplitude `amp` (ends stay put, so junctions still meet).
fn meander(pts: &[[f32; 3]], step: f32, amp: f32, seed: u64) -> Vec<[f32; 3]> {
    let mut out = vec![pts[0]];
    let mut s_acc = 0.0f32;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        let n = (len / step).ceil().max(1.0) as usize;
        // Sideways only as much as the run is horizontal: shafts stay straight.
        let side = {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy);
            let horiz = if len > 1e-3 { l / len } else { 0.0 };
            if l < 1e-3 {
                [0.0, 0.0]
            } else {
                [-dy / l * horiz * horiz, dx / l * horiz * horiz]
            }
        };
        for k in 1..=n {
            let t = k as f32 / n as f32;
            s_acc += len / n as f32;
            let mut p = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            let last = k == n;
            if !last {
                // Fade to zero at both ends of each edge so joints stay where the graph put them.
                let fade = (t * (1.0 - t) * 4.0).min(1.0);
                let off = amp * fade * value_noise(seed, [s_acc / (4.0 * step), 0.5, 0.25]);
                p[0] += side[0] * off;
                p[1] += side[1] * off;
            }
            out.push(p);
        }
    }
    out
}

#[derive(PartialEq)]
struct Open(f32, usize);
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| o.1.cmp(&self.1))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Dijkstra from `sources` over `adj`; returns predecessors.
fn shortest_tree(n: usize, adj: &[Vec<(usize, f32)>], sources: &[usize]) -> Vec<Option<usize>> {
    let mut dist = vec![f32::INFINITY; n];
    let mut prev = vec![None; n];
    let mut heap = BinaryHeap::new();
    for &s in sources {
        dist[s] = 0.0;
        heap.push(Open(0.0, s));
    }
    while let Some(Open(d, u)) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        for &(v, c) in &adj[u] {
            let nd = d + c;
            if nd < dist[v] {
                dist[v] = nd;
                prev[v] = Some(u);
                heap.push(Open(nd, v));
            }
        }
    }
    prev
}

fn karst(p: &CaveParams, g: &Ground, notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let mut rng = Rng::new(p.seed, 1);
    let st = g.hf.stats();
    let relief = (st.max - st.min).max(1.0);
    let cover = if p.cover_m > 0.0 { p.cover_m } else { 12.0 };
    // Inlets: placed sinkholes, else gentle high ground (where dolines form).
    let inlets: Vec<[f32; 2]> = if p.entrances.is_empty() {
        let mut c: Vec<([f32; 2], f32)> = g
            .samples(2)
            .into_iter()
            .filter(|s| {
                g.z(s[0], s[1]) > st.min + 0.45 * relief
                    && g.slope_deg(s[0], s[1]) < 15.0
                    && g.water(s[0], s[1]) < 0.1
            })
            .map(|s| (s, rng.f()))
            .collect();
        c.sort_by(|a, b| a.1.total_cmp(&b.1));
        let pts: Vec<[f32; 2]> = c.into_iter().map(|x| x.0).collect();
        // Cluster them within the reach around the first.
        let first = pts.first().copied();
        let near: Vec<[f32; 2]> = pts
            .into_iter()
            .filter(|q| first.is_some_and(|f| (q[0] - f[0]).hypot(q[1] - f[1]) < 0.5 * p.reach_m))
            .collect();
        spread(
            &near,
            p.complexity.max(1) as usize,
            (0.15 * p.reach_m).max(20.0),
        )
    } else {
        p.entrances.clone()
    };
    if inlets.is_empty() {
        return Err(JdError::invalid(
            "entrances",
            "no gentle high ground for sinkholes; place them yourself",
        ));
    }
    let centre = [
        inlets.iter().map(|q| q[0]).sum::<f32>() / inlets.len() as f32,
        inlets.iter().map(|q| q[1]).sum::<f32>() / inlets.len() as f32,
    ];
    // Outlet: springs feed lakes, rivers and the sea — the water cell nearest the sinkholes;
    // without water, the lowest ground at the foot of relief (a valley side), not mid-plain.
    let within: Vec<[f32; 2]> = g
        .samples(1)
        .into_iter()
        .filter(|s| (s[0] - centre[0]).hypot(s[1] - centre[1]) <= p.reach_m)
        .collect();
    let dist = |s: &[f32; 2]| (s[0] - centre[0]).hypot(s[1] - centre[1]);
    let foot = |s: &[f32; 2]| {
        let (z, c) = (g.z(s[0], s[1]), 3.0 * g.cell());
        [[c, 0.0], [-c, 0.0], [0.0, c], [0.0, -c]]
            .iter()
            .any(|d| g.z(s[0] + d[0], s[1] + d[1]) > z + 10.0)
    };
    let lowest = |it: &mut dyn Iterator<Item = &[f32; 2]>| {
        it.min_by(|a, b| g.z(a[0], a[1]).total_cmp(&g.z(b[0], b[1])))
            .copied()
    };
    let outlet = within
        .iter()
        .filter(|s| g.water(s[0], s[1]) > 0.5)
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .copied()
        .or_else(|| lowest(&mut within.iter().filter(|s| foot(s))))
        .or_else(|| lowest(&mut within.iter()))
        .ok_or_else(|| JdError::invalid("reach_m", "no ground within reach"))?;
    let z_out = g.z(outlet[0], outlet[1]);
    let w_out = g.water(outlet[0], outlet[1]);
    // The water table sits at the spring (or the water surface over it).
    let water_level = if w_out > 0.0 {
        z_out + w_out
    } else {
        z_out + 0.5
    };
    let top = inlets
        .iter()
        .map(|q| g.z(q[0], q[1]))
        .fold(f32::MAX, f32::min)
        - cover;
    if top <= water_level + 2.0 {
        notes.push(
            "little rock between the sinkholes and the spring: the cave stays near the water table"
                .to_owned(),
        );
    }
    // Inception horizons from just under the water table upward.
    let levels = p.levels.max(1);
    let span = (top - water_level).max(4.0);
    let horizons: Vec<f32> = (0..levels)
        .map(|k| water_level - 1.5 + span * k as f32 / levels as f32)
        .collect();
    // Lattice over the area between inlets and outlet.
    let pts_all: Vec<[f32; 2]> = inlets.iter().copied().chain([outlet]).collect();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for q in &pts_all {
        for k in 0..2 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let gstep = (p.reach_m / 12.0).clamp(12.0, 60.0);
    let pad = 2.0 * gstep;
    let sz = g.size();
    let (lo, hi) = (
        [(lo[0] - pad).max(0.0), (lo[1] - pad).max(0.0)],
        [(hi[0] + pad).min(sz[0]), (hi[1] + pad).min(sz[1])],
    );
    let nx = ((hi[0] - lo[0]) / gstep).ceil() as usize + 1;
    let ny = ((hi[1] - lo[1]) / gstep).ceil() as usize + 1;
    let mut nodes: Vec<[f32; 3]> = Vec::new();
    let mut at: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut level: Vec<usize> = Vec::new();
    for (k, &hz) in horizons.iter().enumerate() {
        for j in 0..ny {
            for i in 0..nx {
                let u = lo[0] + i as f32 * gstep + rng.range(-0.3, 0.3) * gstep;
                let v = lo[1] + j as f32 * gstep + rng.range(-0.3, 0.3) * gstep;
                let (u, v) = (u.clamp(0.0, sz[0]), v.clamp(0.0, sz[1]));
                if g.z(u, v) - hz < cover {
                    continue; // not enough rock overhead
                }
                at.insert((i, j, k), nodes.len());
                level.push(k);
                nodes.push([u, v, hz + rng.range(-0.15, 0.15) * gstep.min(6.0)]);
            }
        }
    }
    let lattice = nodes.len();
    // Fracture sets: two strikes about 90° apart.
    let phi = rng.range(0.0, std::f32::consts::PI);
    let strikes = [
        phi,
        phi + std::f32::consts::FRAC_PI_2 + rng.range(-0.3, 0.3),
    ];
    let mut adj: Vec<Vec<(usize, f32)>> = vec![Vec::new(); lattice];
    let edge = |a: usize, b: usize, cost: f32, adj: &mut Vec<Vec<(usize, f32)>>| {
        adj[a].push((b, cost));
        adj[b].push((a, cost));
    };
    let len3 = |a: [f32; 3], b: [f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let mut keys: Vec<(usize, usize, usize)> = at.keys().copied().collect();
    keys.sort_unstable();
    for (i, j, k) in keys {
        let a = at[&(i, j, k)];
        for (di, dj) in [(1i64, 0i64), (0, 1), (1, 1), (1, -1)] {
            let (ii, jj) = (i as i64 + di, j as i64 + dj);
            if ii < 0 || jj < 0 {
                continue;
            }
            if let Some(&b) = at.get(&(ii as usize, jj as usize, k)) {
                let (pa, pb) = (nodes[a], nodes[b]);
                let ang = (pb[1] - pa[1]).atan2(pb[0] - pa[0]);
                let along = strikes.iter().any(|s| {
                    let d = (ang - s).rem_euclid(std::f32::consts::PI);
                    d.min(std::f32::consts::PI - d) < 0.26
                });
                let h = unit(mix(p.seed ^ mix((a as u64) << 32 | b as u64)));
                edge(
                    a,
                    b,
                    len3(pa, pb) * if along { 0.6 } else { 1.0 } * (0.8 + 0.4 * h),
                    &mut adj,
                );
            }
        }
        if let Some(&b) = at.get(&(i, j, k + 1)) {
            edge(a, b, len3(nodes[a], nodes[b]) * 1.6, &mut adj);
        }
    }
    // A sinkhole's water meets the highest inception horizon under it first (vadose flow along
    // it, then shafts down); the spring drains the lowest horizon, at the water table.
    let near = |q: [f32; 2],
                nodes: &Vec<[f32; 3]>,
                max_k: usize,
                want: &dyn Fn(usize) -> bool|
     -> Vec<usize> {
        let mut c: Vec<(f32, usize)> = (0..lattice)
            .filter(|&i| want(i))
            .map(|i| ((nodes[i][0] - q[0]).hypot(nodes[i][1] - q[1]), i))
            .collect();
        c.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        c.into_iter().take(max_k).map(|x| x.1).collect()
    };
    let mut ends = Vec::new();
    let n_in = inlets.len();
    for (qi, q) in inlets.iter().chain([&outlet]).enumerate() {
        let z = g.z(q[0], q[1]);
        let id = nodes.len();
        nodes.push([q[0], q[1], z - 1.0]);
        adj.push(Vec::new());
        let targets = if qi < n_in {
            let top = near(*q, &nodes, 12, &|_| true)
                .into_iter()
                .filter(|&i| (nodes[i][0] - q[0]).hypot(nodes[i][1] - q[1]) < 2.5 * gstep)
                .map(|i| level[i])
                .max();
            match top {
                Some(t) => near(*q, &nodes, 4, &|i| level[i] == t),
                None => near(*q, &nodes, 4, &|_| true),
            }
        } else {
            let low = near(*q, &nodes, 6, &|i| level[i] == 0);
            if low.is_empty() {
                near(*q, &nodes, 6, &|_| true)
            } else {
                low
            }
        };
        for b in targets {
            let c = len3(nodes[id], nodes[b]) * 0.9;
            adj[id].push((b, c));
            adj[b].push((id, c));
        }
        ends.push(id);
    }
    let (outlet_id, inlet_ids) = (ends[ends.len() - 1], &ends[..ends.len() - 1]);
    // Route the sinkholes one by one; a conduit already dissolved is far cheaper to follow, so
    // later water joins it — tributaries merge into a branchwork (Paris et al. 2021 iterate the
    // same way). Farthest sinkhole first: it lays the trunk.
    let mut order: Vec<usize> = inlet_ids.to_vec();
    order.sort_by(|&a, &b| {
        let d =
            |i: usize| (nodes[i][0] - nodes[outlet_id][0]).hypot(nodes[i][1] - nodes[outlet_id][1]);
        d(b).total_cmp(&d(a)).then(a.cmp(&b))
    });
    let mut flow: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for s in order {
        let discounted: Vec<Vec<(usize, f32)>> = adj
            .iter()
            .enumerate()
            .map(|(u, list)| {
                list.iter()
                    .map(|&(v, c)| {
                        (
                            v,
                            if flow.contains_key(&(u.min(v), u.max(v))) {
                                0.25 * c
                            } else {
                                c
                            },
                        )
                    })
                    .collect()
            })
            .collect();
        let prev = shortest_tree(nodes.len(), &discounted, &[outlet_id]);
        let mut u = s;
        let mut guard = 0;
        while let Some(v) = prev[u] {
            *flow.entry((u.min(v), u.max(v))).or_default() += 1;
            u = v;
            guard += 1;
            if guard > nodes.len() {
                break;
            }
        }
        if u != outlet_id {
            notes.push(format!(
                "sinkhole at ({:.0}, {:.0}) found no path to the spring",
                nodes[s][0], nodes[s][1]
            ));
        }
    }
    if flow.is_empty() {
        return Err(JdError::invalid(
            "entrances",
            "no passage could join the sinkholes to a spring",
        )
        .next("Place the sinkholes on higher ground, or raise reach_m."));
    }
    // Chains between junctions.
    let mut deg: HashMap<usize, u32> = HashMap::new();
    let mut nb: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(a, b) in flow.keys() {
        *deg.entry(a).or_default() += 1;
        *deg.entry(b).or_default() += 1;
        nb.entry(a).or_default().push(b);
        nb.entry(b).or_default().push(a);
    }
    let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut chains: Vec<(Vec<usize>, u32)> = Vec::new();
    let mut starts: Vec<usize> = deg
        .iter()
        .filter(|(_, d)| **d != 2)
        .map(|(n, _)| *n)
        .collect();
    starts.sort_unstable();
    for &s in &starts {
        for &first in &nb[&s] {
            let key = (s.min(first), s.max(first));
            if used.contains(&key) {
                continue;
            }
            let mut chain = vec![s];
            let (mut prev_n, mut cur) = (s, first);
            let mut q = flow[&key];
            used.insert(key);
            loop {
                chain.push(cur);
                if deg[&cur] != 2 {
                    break;
                }
                let next = nb[&cur]
                    .iter()
                    .copied()
                    .find(|&x| x != prev_n)
                    .unwrap_or(cur);
                let k2 = (cur.min(next), cur.max(next));
                if used.contains(&k2) {
                    break;
                }
                used.insert(k2);
                q = q.max(flow[&k2]);
                prev_n = cur;
                cur = next;
            }
            chains.push((chain, q));
        }
    }
    let base_w = 1.4 * p.size;
    let mut sys = CaveSystem {
        water_level_m: Some(water_level),
        ..Default::default()
    };
    for (ci, (chain, q)) in chains.iter().enumerate() {
        let pts: Vec<[f32; 3]> = chain.iter().map(|&i| nodes[i]).collect();
        let pts = meander(&pts, 6.0, 0.25 * gstep, p.seed ^ ci as u64);
        let w = base_w * (*q as f32).powf(0.4);
        let mean_z = pts.iter().map(|x| x[2]).sum::<f32>() / pts.len() as f32;
        let vertical = {
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            (a[2] - b[2]).abs() > 1.5 * (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let ps = if vertical {
            passage(pts, w * 0.9, w * 0.9, Profile::Tube) // a shaft
        } else if mean_z > water_level + 3.0 {
            passage(pts, w * 0.9, w * 2.2, Profile::Keyhole) // vadose: a stream cut a slot under the tube
        } else {
            passage(pts, w * 1.3, w * 0.8, Profile::Tube) // phreatic: wider than tall
        };
        sys.passages.push(ps);
    }
    // Chambers at some junctions (breakdown rooms).
    let mut junctions: Vec<(usize, u32)> = deg.iter().map(|(n, d)| (*n, *d)).collect();
    junctions.sort_unstable();
    for (n, d) in junctions {
        if d >= 3 && n < lattice && unit(mix(p.seed ^ 0xC4A ^ n as u64)) < 0.5 {
            let r = base_w * rng.range(3.0, 5.5);
            sys.chambers.push(Chamber {
                center: nodes[n],
                radii_m: [r, r * rng.range(0.6, 1.0), r * rng.range(0.4, 0.6)],
            });
        }
    }
    sys.chambers.sort_by(|a, b| {
        a.center[0]
            .total_cmp(&b.center[0])
            .then(a.center[1].total_cmp(&b.center[1]))
    });
    // Shafts up to the sinkholes, and the spring mouth.
    for &q in &inlets {
        let z = g.z(q[0], q[1]);
        sys.passages.push(passage(
            vec![[q[0], q[1], z + 2.0], [q[0], q[1], z - 1.0]],
            base_w * 1.2,
            base_w * 1.2,
            Profile::Tube,
        ));
        sys.entrances.push(Entrance {
            at: [q[0], q[1], z],
            kind: EntranceKind::Sinkhole,
            underwater: false,
        });
    }
    sys.passages.push(passage(
        vec![
            [outlet[0], outlet[1], z_out + 1.0],
            [outlet[0], outlet[1], z_out - 1.0],
        ],
        base_w * 1.3,
        base_w,
        Profile::Tube,
    ));
    sys.entrances.push(Entrance {
        at: [outlet[0], outlet[1], z_out],
        kind: EntranceKind::Resurgence,
        underwater: w_out > 0.5,
    });
    Ok(sys)
}

fn lava_tube(p: &CaveParams, g: &Ground, notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let mut rng = Rng::new(p.seed, 2);
    let vent = match p.entrances.first() {
        Some(v) => *v,
        None => g
            .samples(2)
            .into_iter()
            .max_by(|a, b| g.z(a[0], a[1]).total_cmp(&g.z(b[0], b[1])))
            .ok_or_else(|| JdError::invalid("entrances", "empty terrain"))?,
    };
    let (w, h) = (4.0 * p.size, 2.6 * p.size);
    let depth = if p.cover_m > 0.0 {
        p.cover_m + h
    } else {
        3.0 + h
    };
    // Follow the flow downhill with some inertia and meander.
    let step = 4.0f32;
    let mut pos = vent;
    let mut dir = {
        let gr = g.grad(pos[0], pos[1]);
        let l = gr[0].hypot(gr[1]).max(1e-6);
        [-gr[0] / l, -gr[1] / l]
    };
    let mut path = vec![pos];
    let mut flat = 0;
    let mut length = 0.0f32;
    while length < p.reach_m {
        let gr = g.grad(pos[0], pos[1]);
        let l = gr[0].hypot(gr[1]);
        let down = if l > 1e-6 {
            [-gr[0] / l, -gr[1] / l]
        } else {
            dir
        };
        let wig = 0.35 * value_noise(p.seed, [length / 60.0, 0.3, 0.7]);
        let (c, s) = (wig.cos(), wig.sin());
        let nd = [0.6 * dir[0] + 0.4 * down[0], 0.6 * dir[1] + 0.4 * down[1]];
        let nd = [nd[0] * c - nd[1] * s, nd[0] * s + nd[1] * c];
        let nl = nd[0].hypot(nd[1]).max(1e-6);
        dir = [nd[0] / nl, nd[1] / nl];
        let next = [pos[0] + dir[0] * step, pos[1] + dir[1] * step];
        if !g.inside(next[0], next[1])
            || g.z(next[0], next[1]) + g.water(next[0], next[1]) <= p.sea_level_m
        {
            break;
        }
        flat = if g.slope_deg(next[0], next[1]) < 1.0 {
            flat + 1
        } else {
            0
        };
        if flat > 12 {
            break; // the flow spread out on flat ground
        }
        pos = next;
        path.push(pos);
        length += step;
    }
    if path.len() < 5 {
        return Err(JdError::invalid(
            "entrances",
            "the vent sits on flat ground: a lava tube needs a slope to run down",
        )
        .next("Place the vent higher on a volcano's flank."));
    }
    // Centreline a few metres under the surface, smoothed along the path.
    let raw: Vec<f32> = path.iter().map(|q| g.z(q[0], q[1]) - depth).collect();
    let k = 4usize;
    let pts: Vec<[f32; 3]> = path
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let (a, b) = (i.saturating_sub(k), (i + k).min(raw.len() - 1));
            let z = raw[a..=b].iter().copied().fold(f32::MAX, f32::min);
            [q[0], q[1], z]
        })
        .collect();
    let mut sys = CaveSystem::default();
    let n = pts.len();
    sys.passages.push(Passage {
        half_width_m: (0..n)
            .map(|i| w * (0.8 + 0.4 * value_noise(p.seed ^ 3, [i as f32 / 12.0, 0.0, 0.0]).abs()))
            .collect(),
        half_height_m: vec![h; n],
        points: pts.clone(),
        profile: Profile::Tube,
    });
    // Skylights every 120–250 m.
    let mut next_sky = rng.range(40.0, 120.0);
    let mut s = 0.0f32;
    for &q in &pts[1..] {
        s += step;
        if s >= next_sky {
            let zg = g.z(q[0], q[1]);
            let r = w * rng.range(0.5, 0.9);
            sys.chambers.push(Chamber {
                center: [q[0], q[1], (q[2] + zg) / 2.0 + 1.0],
                radii_m: [r, r, (zg - q[2]) / 2.0 + 3.0],
            });
            sys.entrances.push(Entrance {
                at: [q[0], q[1], zg],
                kind: EntranceKind::Skylight,
                underwater: false,
            });
            next_sky = s + rng.range(120.0, 250.0);
        }
    }
    if sys.entrances.is_empty() {
        notes.push("the tube is shorter than the first skylight: it has no way in".to_owned());
    }
    Ok(sys)
}

fn sea_cave(p: &CaveParams, g: &Ground, notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let sea = |u: f32, v: f32| {
        if g.water.is_some() && g.water(u, v) > 0.0 {
            g.z(u, v) + g.water(u, v)
        } else {
            p.sea_level_m
        }
    };
    let c = g.cell();
    // Cliff cells: land a few metres above the sea next to water, steep.
    let is_cliff = |q: [f32; 2]| {
        let z = g.z(q[0], q[1]);
        let s = sea(q[0], q[1]);
        z > s + 3.0
            && g.slope_deg(q[0], q[1]) > 25.0
            && [[2.0, 0.0], [-2.0, 0.0], [0.0, 2.0], [0.0, -2.0]]
                .iter()
                .any(|d| {
                    let (u, v) = (q[0] + d[0] * c * 1.5, q[1] + d[1] * c * 1.5);
                    g.inside(u, v) && g.z(u, v) < sea(u, v)
                })
    };
    let mouths: Vec<[f32; 2]> = if p.entrances.is_empty() {
        let mut rng = Rng::new(p.seed, 3);
        let mut cands: Vec<([f32; 2], f32)> = g
            .samples(1)
            .into_iter()
            .filter(|q| is_cliff(*q))
            .map(|q| (q, rng.f()))
            .collect();
        cands.sort_by(|a, b| a.1.total_cmp(&b.1));
        let pts: Vec<[f32; 2]> = cands.into_iter().map(|x| x.0).collect();
        spread(&pts, p.complexity.max(1) as usize, 60.0)
    } else {
        p.entrances.clone()
    };
    if mouths.is_empty() {
        return Err(JdError::invalid("entrances", "no sea cliffs: sea caves need land rising steeply from the water").next("Raise sea_level_m to meet your cliffs, wire a water layer, or place the mouths yourself."));
    }
    let (w, h) = (2.0 * p.size, 5.0 * p.size);
    let mut sys = CaveSystem::default();
    let mut level = f32::MIN;
    for (mi, m) in mouths.iter().enumerate() {
        let s = sea(m[0], m[1]);
        level = level.max(s);
        let gr = g.grad(m[0], m[1]);
        let l = gr[0].hypot(gr[1]);
        if l < 0.05 {
            notes.push(format!(
                "mouth at ({:.0}, {:.0}) is not on a cliff",
                m[0], m[1]
            ));
        }
        let inland = if l > 1e-6 {
            [gr[0] / l, gr[1] / l]
        } else {
            [1.0, 0.0]
        };
        // From a few metres out at sea, straight into the cliff along a joint, tapering.
        let len =
            (p.reach_m * 0.15).clamp(15.0, 150.0) * (0.6 + 0.8 * unit(mix(p.seed ^ mi as u64)));
        let z = s - 1.0 + h; // the floor a metre under the water
        let steps = (len / 4.0).ceil() as usize + 3;
        let mut pts = Vec::new();
        let mut half_w = Vec::new();
        let mut half_h = Vec::new();
        let mut through = false;
        let mut was_land = false;
        for k in 0..=steps {
            let d = -8.0 + k as f32 * 4.0;
            let wig = 2.0 * value_noise(p.seed ^ 77 ^ mi as u64, [d / 20.0, 0.0, 0.0]);
            let q = [
                m[0] + inland[0] * d - inland[1] * wig,
                m[1] + inland[1] * d + inland[0] * wig,
            ];
            if !g.inside(q[0], q[1]) {
                break;
            }
            let land = g.z(q[0], q[1]) > sea(q[0], q[1]);
            if was_land && !land && d > 6.0 {
                through = true; // out the other side: an arch
            }
            was_land |= land;
            let t = (d.max(0.0) / len).min(1.0);
            pts.push([q[0], q[1], z - t * h * 0.3]);
            half_w.push(w * (1.0 - 0.4 * t));
            half_h.push(h * (1.0 - 0.45 * t));
            if through {
                pts.push([q[0] + inland[0] * 8.0, q[1] + inland[1] * 8.0, z]);
                half_w.push(w);
                half_h.push(h);
                break;
            }
        }
        if pts.len() < 3 {
            continue;
        }
        sys.entrances.push(Entrance {
            at: [m[0], m[1], s],
            kind: EntranceKind::SeaCave,
            underwater: false,
        });
        if through {
            let e = pts[pts.len() - 1];
            sys.entrances.push(Entrance {
                at: [e[0], e[1], s],
                kind: EntranceKind::SeaCave,
                underwater: false,
            });
            notes.push(format!(
                "the cave at ({:.0}, {:.0}) breaks through: a sea arch",
                m[0], m[1]
            ));
        }
        sys.passages.push(Passage {
            points: pts,
            half_width_m: half_w,
            half_height_m: half_h,
            profile: Profile::Arch,
        });
    }
    if sys.passages.is_empty() {
        return Err(JdError::invalid(
            "entrances",
            "the mouths are at the map edge",
        ));
    }
    sys.water_level_m = Some(level);
    Ok(sys)
}

fn mountain(p: &CaveParams, g: &Ground, notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let st = g.hf.stats();
    let relief = (st.max - st.min).max(1.0);
    let mut rng = Rng::new(p.seed, 4);
    let mouths: Vec<[f32; 2]> = if p.entrances.is_empty() {
        let mut c: Vec<([f32; 2], f32)> = g
            .samples(2)
            .into_iter()
            .filter(|q| {
                let s = g.slope_deg(q[0], q[1]);
                (18.0..50.0).contains(&s)
                    && g.z(q[0], q[1]) > st.min + 0.25 * relief
                    && g.water(q[0], q[1]) < 0.1
            })
            .map(|q| (q, rng.f()))
            .collect();
        c.sort_by(|a, b| a.1.total_cmp(&b.1));
        let pts: Vec<[f32; 2]> = c.into_iter().map(|x| x.0).collect();
        spread(&pts, (p.complexity / 2).max(1) as usize, 120.0)
    } else {
        p.entrances.clone()
    };
    if mouths.is_empty() {
        return Err(JdError::invalid(
            "entrances",
            "no hillside steep enough for a cave mouth; place one yourself",
        ));
    }
    let cover = if p.cover_m > 0.0 { p.cover_m } else { 15.0 };
    let mut sys = CaveSystem::default();
    let mut worms: Vec<([f32; 3], [f32; 2], f32, u64)> = Vec::new();
    for (i, m) in mouths.iter().enumerate() {
        let gr = g.grad(m[0], m[1]);
        let l = gr[0].hypot(gr[1]);
        if l < 0.15 {
            notes.push(format!(
                "mouth at ({:.0}, {:.0}) is on gentle ground: the passage dives in steeply",
                m[0], m[1]
            ));
        }
        let into = if l > 1e-6 {
            [gr[0] / l, gr[1] / l]
        } else {
            [1.0, 0.0]
        };
        let z = g.z(m[0], m[1]);
        sys.entrances.push(Entrance {
            at: [m[0], m[1], z],
            kind: EntranceKind::Portal,
            underwater: false,
        });
        // Start a little outside the slope so the mouth breaks the surface.
        worms.push((
            [m[0] - into[0] * 4.0, m[1] - into[1] * 4.0, z + 0.5 * p.size],
            into,
            p.reach_m,
            p.seed ^ i as u64,
        ));
    }
    let mut budget = 64;
    while let Some((start, dir0, length, seed)) = worms.pop() {
        budget -= 1;
        if budget < 0 {
            break;
        }
        let mut r = Rng::new(seed, 5);
        let mut pos = start;
        let mut yaw = dir0[1].atan2(dir0[0]);
        let mut pitch = -0.12f32;
        let mut pts = vec![pos];
        let mut ws = vec![2.4 * p.size];
        let step = 3.0f32;
        let mut s = 0.0;
        while s < length {
            s += step;
            yaw += 0.5 * value_noise(seed, [s / 30.0, 1.0, 0.0]) * step / 10.0;
            let depth = g.z(pos[0], pos[1]) - pos[2];
            let want = if s < 25.0 {
                -0.15
            } else if depth < cover {
                -0.3
            } else if depth > 4.0 * cover {
                0.15
            } else {
                0.25 * value_noise(seed ^ 9, [s / 40.0, 0.0, 0.0])
            };
            pitch += (want - pitch) * 0.2;
            let next = [
                pos[0] + yaw.cos() * pitch.cos() * step,
                pos[1] + yaw.sin() * pitch.cos() * step,
                pos[2] + pitch.sin() * step,
            ];
            if !g.inside(next[0], next[1]) {
                break;
            }
            pos = next;
            pts.push(pos);
            ws.push(p.size * (2.4 + 1.2 * value_noise(seed ^ 11, [s / 25.0, 0.0, 0.0])).max(0.8));
            if s > 30.0 && r.f() < 0.012 * p.complexity as f32 {
                let side = if r.f() < 0.5 { 1.0 } else { -1.0 };
                let a = yaw + side * r.range(0.6, 1.4);
                worms.push((
                    pos,
                    [a.cos(), a.sin()],
                    length * r.range(0.25, 0.5),
                    seed.wrapping_mul(31).wrapping_add(pts.len() as u64),
                ));
            }
            if s > 40.0 && r.f() < 0.006 {
                let rr = p.size * r.range(6.0, 12.0);
                sys.chambers.push(Chamber {
                    center: [pos[0], pos[1], pos[2] + rr * 0.2],
                    radii_m: [rr, rr * r.range(0.6, 1.0), rr * 0.5],
                });
            }
        }
        if pts.len() >= 3 {
            let n = pts.len();
            let hs = ws.iter().map(|w| w * 0.8).collect();
            sys.passages.push(Passage {
                points: pts,
                half_width_m: ws[..n].to_vec(),
                half_height_m: hs,
                profile: Profile::Tube,
            });
        }
    }
    Ok(sys)
}

fn mine(p: &CaveParams, g: &Ground, notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let st = g.hf.stats();
    let relief = (st.max - st.min).max(1.0);
    let mut rng = Rng::new(p.seed, 6);
    let portal = match p.entrances.first() {
        Some(q) => *q,
        None => g
            .samples(2)
            .into_iter()
            .filter(|q| {
                (20.0..40.0).contains(&g.slope_deg(q[0], q[1]))
                    && g.z(q[0], q[1]) > st.min + 0.2 * relief
                    && g.z(q[0], q[1]) < st.min + 0.7 * relief
            })
            .min_by(|a, b| rng_key(p.seed, a).total_cmp(&rng_key(p.seed, b)))
            .ok_or_else(|| {
                JdError::invalid("entrances", "no hillside for a portal; place one yourself")
            })?,
    };
    if g.slope_deg(portal[0], portal[1]) < 10.0 {
        notes.push("the portal is on gentle ground: an adit is driven into a hillside".to_owned());
    }
    let gr = g.grad(portal[0], portal[1]);
    let l = gr[0].hypot(gr[1]);
    let into = if l > 1e-6 {
        [gr[0] / l, gr[1] / l]
    } else {
        [1.0, 0.0]
    };
    let (w, h) = (1.2 * p.size, 1.3 * p.size);
    let z0 = g.z(portal[0], portal[1]) + h; // floor at the portal's ground
    let grade = 1.0 / 200.0; // rises inward so water drains out
    let len = p.reach_m.min(2000.0);
    let mut sys = CaveSystem::default();
    let adit = |d: f32| {
        [
            portal[0] + into[0] * d,
            portal[1] + into[1] * d,
            z0 + grade * d.max(0.0),
        ]
    };
    let main: Vec<[f32; 3]> = (0..=((len + 6.0) / 10.0).ceil() as usize)
        .map(|k| adit(-6.0 + k as f32 * 10.0))
        .take_while(|q| g.inside(q[0], q[1]))
        .collect();
    if main.len() < 3 {
        return Err(JdError::invalid("entrances", "the adit runs off the map"));
    }
    let end = main[main.len() - 1];
    sys.passages
        .push(passage(main.clone(), w, h, Profile::Arch));
    sys.entrances.push(Entrance {
        at: [portal[0], portal[1], z0 - h],
        kind: EntranceKind::Portal,
        underwater: false,
    });
    // Crosscuts every ~40 m, alternating sides, on the same grade.
    let n_cross = p.complexity.min(24);
    let levels = p.levels.max(1);
    let level_gap = 30.0 * p.size;
    for lvl in 0..levels {
        let dz = -(lvl as f32) * level_gap;
        if lvl > 0 {
            // A level below: a drift back toward the portal from under the shaft, as long as the
            // adit so every crosscut on it starts on it.
            let start = [end[0], end[1], end[2] + dz];
            let back: Vec<[f32; 3]> = (0..=((len - 20.0).max(10.0) / 10.0) as usize)
                .map(|k| {
                    let d = k as f32 * 10.0;
                    [
                        start[0] - into[0] * d,
                        start[1] - into[1] * d,
                        start[2] - grade * d,
                    ]
                })
                .collect();
            sys.passages.push(passage(back, w, h, Profile::Arch));
        }
        for c in 0..n_cross {
            let along = 30.0 + (c as f32 + 0.5) * (len - 40.0).max(20.0) / n_cross as f32;
            if along > len {
                break;
            }
            let base = if lvl == 0 {
                adit(along)
            } else {
                [
                    end[0] - into[0] * (len - along),
                    end[1] - into[1] * (len - along),
                    end[2] + dz,
                ]
            };
            let side = if (c + lvl) % 2 == 0 { 1.0 } else { -1.0 };
            let perp = [-into[1] * side, into[0] * side];
            let clen = rng.range(20.0, 60.0) * p.size;
            let pts: Vec<[f32; 3]> = (0..=(clen / 10.0).ceil() as usize)
                .map(|k| {
                    let d = (k as f32 * 10.0).min(clen);
                    [
                        base[0] + perp[0] * d,
                        base[1] + perp[1] * d,
                        base[2] + grade * d,
                    ]
                })
                .collect();
            let tip = pts[pts.len() - 1];
            sys.passages
                .push(passage(pts, w * 0.9, h * 0.95, Profile::Arch));
            // A stope at the end: a wide, low working.
            sys.chambers.push(Chamber {
                center: [tip[0], tip[1], tip[2] + 0.5],
                radii_m: [4.0 * p.size, 4.0 * p.size, 2.5 * p.size],
            });
        }
    }
    // The shaft: from the surface over the adit's end down to the deepest level.
    let zs = g.z(end[0], end[1]);
    let bottom = end[2] - (levels - 1) as f32 * level_gap - h;
    sys.passages.push(passage(
        vec![[end[0], end[1], zs + 3.0], [end[0], end[1], bottom]],
        1.1 * p.size,
        1.1 * p.size,
        Profile::Rect,
    ));
    sys.entrances.push(Entrance {
        at: [end[0], end[1], zs],
        kind: EntranceKind::Shaft,
        underwater: false,
    });
    Ok(sys)
}

fn rng_key(seed: u64, q: &[f32; 2]) -> f32 {
    unit(mix(seed
        ^ mix(q[0].to_bits() as u64)
        ^ mix((q[1].to_bits() as u64) << 7)))
}

fn tunnel(p: &CaveParams, g: &Ground, notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let [a, b] = match p.entrances.as_slice() {
        [a, b, ..] => [*a, *b],
        _ => {
            return Err(JdError::invalid("entrances", "a tunnel needs two portals")
                .next("Place one portal on each side of the hill."));
        }
    };
    let (w, h) = (5.0 * p.size, 3.8 * p.size);
    let (za, zb) = (g.z(a[0], a[1]), g.z(b[0], b[1]));
    let horiz = (b[0] - a[0]).hypot(b[1] - a[1]);
    if horiz < 10.0 {
        return Err(JdError::invalid(
            "entrances",
            "the portals are less than 10 m apart",
        ));
    }
    let grade = (zb - za).abs() / horiz;
    if grade > 0.04 {
        notes.push(format!(
            "grade {:.1} % is steeper than a road tunnel's usual 4 %",
            grade * 100.0
        ));
    }
    let dir = [(b[0] - a[0]) / horiz, (b[1] - a[1]) / horiz];
    let n = (horiz / 10.0).ceil() as usize;
    let ext = 8.0;
    let mut pts = Vec::new();
    let mut min_cover = f32::MAX;
    for k in 0..=n + 2 {
        let d = -ext + k as f32 * (horiz + 2.0 * ext) / (n + 2) as f32;
        let t = (d / horiz).clamp(0.0, 1.0);
        let q = [a[0] + dir[0] * d, a[1] + dir[1] * d];
        let floor = za + (zb - za) * t;
        pts.push([q[0], q[1], floor + h]);
        if (0.15..=0.85).contains(&t) {
            min_cover = min_cover.min(g.z(q[0], q[1]) - (floor + 2.0 * h));
        }
    }
    if min_cover < 2.0 * w {
        notes.push(format!(
            "only {:.0} m of rock over the tunnel in places: a cutting or cut-and-cover would do",
            min_cover.max(0.0)
        ));
    }
    let mut sys = CaveSystem::default();
    sys.passages.push(passage(pts, w, h, Profile::Arch));
    sys.entrances.push(Entrance {
        at: [a[0], a[1], za],
        kind: EntranceKind::Portal,
        underwater: false,
    });
    sys.entrances.push(Entrance {
        at: [b[0], b[1], zb],
        kind: EntranceKind::Portal,
        underwater: false,
    });
    Ok(sys)
}

fn catacombs(p: &CaveParams, g: &Ground, _notes: &mut Vec<String>) -> JdResult<CaveSystem> {
    let mut rng = Rng::new(p.seed, 7);
    let stair = match p.entrances.first() {
        Some(q) => *q,
        None => {
            let s = g.size();
            [s[0] / 2.0, s[1] / 2.0]
        }
    };
    let side = p.reach_m.min(600.0);
    let cell = 8.0 * p.size;
    let n = ((side / cell) as usize).clamp(3, 80);
    let depth = if p.cover_m > 0.0 { p.cover_m } else { 15.0 };
    let o = [
        stair[0] - n as f32 * cell / 2.0,
        stair[1] - n as f32 * cell / 2.0,
    ];
    let at = |i: usize, j: usize| {
        [
            o[0] + (i as f32 + 0.5) * cell,
            o[1] + (j as f32 + 0.5) * cell,
        ]
    };
    let ground_min = (0..n)
        .flat_map(|i| (0..n).map(move |j| (i, j)))
        .map(|(i, j)| {
            let q = at(i, j);
            g.z(q[0], q[1])
        })
        .fold(f32::MAX, f32::min);
    let (w, h) = (0.6 * p.size, 1.1 * p.size);
    let z = ground_min - depth + h;
    // A perfect maze (randomised depth-first search) plus a few loops.
    let mut seen = vec![false; n * n];
    let mut stack = vec![(n / 2, n / 2)];
    seen[(n / 2) * n + n / 2] = true;
    let mut links: Vec<((usize, usize), (usize, usize))> = Vec::new();
    while let Some(&(i, j)) = stack.last() {
        let mut opts: Vec<(usize, usize)> = Vec::new();
        if i > 0 && !seen[j * n + i - 1] {
            opts.push((i - 1, j));
        }
        if i + 1 < n && !seen[j * n + i + 1] {
            opts.push((i + 1, j));
        }
        if j > 0 && !seen[(j - 1) * n + i] {
            opts.push((i, j - 1));
        }
        if j + 1 < n && !seen[(j + 1) * n + i] {
            opts.push((i, j + 1));
        }
        if opts.is_empty() {
            stack.pop();
            continue;
        }
        let k = ((rng.f() * opts.len() as f32) as usize).min(opts.len() - 1);
        let nx = opts[k];
        seen[nx.1 * n + nx.0] = true;
        links.push(((i, j), nx));
        stack.push(nx);
    }
    for j in 0..n {
        for i in 0..n {
            if i + 1 < n && rng.f() < 0.12 {
                links.push(((i, j), (i + 1, j)));
            }
            if j + 1 < n && rng.f() < 0.12 {
                links.push(((i, j), (i, j + 1)));
            }
        }
    }
    links.sort();
    links.dedup();
    let mut sys = CaveSystem::default();
    for (a, b) in links {
        let (pa, pb) = (at(a.0, a.1), at(b.0, b.1));
        sys.passages.push(passage(
            vec![[pa[0], pa[1], z], [pb[0], pb[1], z]],
            w,
            h,
            Profile::Arch,
        ));
    }
    // Ossuary rooms.
    for j in 0..n {
        for i in 0..n {
            if rng.f() < 0.05 {
                let q = at(i, j);
                sys.chambers.push(Chamber {
                    center: [q[0], q[1], z + 0.4],
                    radii_m: [cell * 0.4, cell * 0.4, h * 1.6],
                });
            }
        }
    }
    // The stair: a 35° incline from the surface down to the galleries.
    let zg = g.z(stair[0], stair[1]);
    let drop = zg - (z - h);
    let run = drop / 35f32.to_radians().tan();
    let c = at(n / 2, n / 2);
    let top = [c[0] - run, c[1]];
    let zt = g.z(top[0], top[1]);
    sys.passages.push(passage(
        vec![[top[0] - 2.0, top[1], zt + h], [c[0], c[1], z]],
        w * 1.1,
        h,
        Profile::Arch,
    ));
    sys.entrances.push(Entrance {
        at: [top[0], top[1], zt],
        kind: EntranceKind::Shaft,
        underwater: false,
    });
    Ok(sys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caves::{CaveField, SpeleothemKind, cave_mesh, entrance_holes};

    /// A 1 km mountain: a cone from 50 m at the edges to 450 m in the middle, on a 128² grid.
    fn hill() -> Heightfield {
        let mut hf = Heightfield::new(128, 128, 64, 2).expect("hf");
        hf.fill_with(|x, y| {
            let (dx, dy) = (x as f32 - 63.5, y as f32 - 63.5);
            let r = (dx * dx + dy * dy).sqrt() / 64.0;
            50.0 + 400.0 * (1.0 - r).max(0.0).powf(1.3)
        });
        hf
    }
    const EXT: [f64; 2] = [1024.0, 1024.0];

    fn make(p: CaveParams, hf: &Heightfield) -> Generated {
        generate_cave(
            &p,
            &Ground {
                hf,
                extent_m: EXT,
                water: None,
            },
        )
        .expect("generate")
    }

    #[test]
    fn karst_drains_sinkholes_to_a_spring_dry_above_flooded_below() {
        let hf = hill();
        let g = make(
            CaveParams {
                seed: 4,
                reach_m: 600.0,
                entrances: vec![[480.0, 560.0], [560.0, 470.0]],
                ..Default::default()
            },
            &hf,
        );
        let s = &g.system;
        let wl = s.water_level_m.expect("water table");
        // Sinkholes and one spring; the spring sits low.
        assert_eq!(
            s.entrances
                .iter()
                .filter(|e| e.kind == EntranceKind::Sinkhole)
                .count(),
            2
        );
        let spring = s
            .entrances
            .iter()
            .find(|e| e.kind == EntranceKind::Resurgence)
            .expect("spring");
        assert!(spring.at[2] < 120.0, "{spring:?}");
        assert!(s.length_m() > 300.0, "{}", s.length_m());
        // Phreatic tubes (wider than tall) at the water table, keyholes above it.
        assert!(s.passages.iter().any(|p| p.profile == Profile::Tube
            && p.half_width_m[0] > p.half_height_m[0]
            && p.points.iter().all(|q| q[2] < wl + 4.0)));
        assert!(s.passages.iter().any(|p| p.profile == Profile::Keyhole));
        // Speleothems, all above the water table.
        assert!(!s.speleothems.is_empty());
        assert!(
            s.speleothems
                .iter()
                .all(|x| x.kind != SpeleothemKind::Stalagmite || x.at[2] >= wl)
        );
        // Rock over the passages away from the entrances.
        let f = CaveField::new(s).expect("field");
        let buried = s
            .passages
            .iter()
            .flat_map(|p| p.points.iter())
            .filter(|q| {
                s.entrances
                    .iter()
                    .all(|e| (e.at[0] - q[0]).hypot(e.at[1] - q[1]) > 60.0)
            })
            .all(|q| ground(&hf, EXT, q[0], q[1]) - q[2] > 6.0);
        assert!(buried);
        // The terrain opens only near the entrances.
        let holes = entrance_holes(s, &hf, EXT).expect("holes");
        let cell = 8.0f32;
        for y in 0..128 {
            for x in 0..128 {
                if holes.get(x, y) > 0.5 {
                    let (u, v) = ((x as f32 + 0.5) * cell, (y as f32 + 0.5) * cell);
                    assert!(
                        s.entrances
                            .iter()
                            .any(|e| (e.at[0] - u).hypot(e.at[1] - v) < 30.0),
                        "hole at ({u}, {v})"
                    );
                }
            }
        }
        assert!(
            f.sdf([spring.at[0], spring.at[1], spring.at[2] - 0.5]) < 0.0,
            "the spring mouth is open"
        );
        let m = cave_mesh(s, &hf, EXT, 1.0).expect("mesh");
        assert!(m.indices.len() > 3000);
        // Same seed, same cave.
        let again = make(
            CaveParams {
                seed: 4,
                reach_m: 600.0,
                entrances: vec![[480.0, 560.0], [560.0, 470.0]],
                ..Default::default()
            },
            &hf,
        );
        assert_eq!(again.system, g.system);
    }

    #[test]
    fn a_lava_tube_runs_downhill_under_a_thin_roof_with_skylights() {
        let hf = hill();
        let g = make(
            CaveParams {
                kind: CaveKind::LavaTube,
                seed: 2,
                reach_m: 600.0,
                entrances: vec![[512.0, 600.0]],
                ..Default::default()
            },
            &hf,
        );
        let s = &g.system;
        let pts = &s.passages[0].points;
        assert!(pts[pts.len() - 1][2] < pts[0][2] - 100.0, "downhill");
        for q in pts.iter().step_by(5) {
            let roof = ground(&hf, EXT, q[0], q[1]) - (q[2] + s.passages[0].half_height_m[0]);
            assert!((0.5..30.0).contains(&roof), "roof {roof}");
        }
        assert!(
            s.entrances
                .iter()
                .filter(|e| e.kind == EntranceKind::Skylight)
                .count()
                >= 2
        );
        assert!(
            s.speleothems
                .iter()
                .filter(|x| x.kind == SpeleothemKind::Stalactite)
                .all(|x| x.length_m <= 0.25 + 1e-4),
            "lavacicles are small"
        );
    }

    #[test]
    fn sea_caves_open_at_sea_level_into_cliffs() {
        // The hill rising out of a sea at 120 m: cliffs where the cone crosses it.
        let mut hf = hill();
        hf.fill_with(|x, y| {
            let (dx, dy) = (x as f32 - 63.5, y as f32 - 63.5);
            let r = (dx * dx + dy * dy).sqrt();
            if r < 40.0 {
                200.0 + (40.0 - r) * 2.0
            } else {
                60.0
            }
        });
        let g = make(
            CaveParams {
                kind: CaveKind::SeaCave,
                seed: 5,
                sea_level_m: 120.0,
                complexity: 3,
                ..Default::default()
            },
            &hf,
        );
        let s = &g.system;
        assert!(!s.passages.is_empty());
        assert_eq!(s.water_level_m, Some(120.0));
        for p in &s.passages {
            let floor = p.points[0][2] - p.half_height_m[0];
            assert!((floor - 119.0).abs() < 1.0, "floor under the water {floor}");
            assert!(p.half_height_m[0] > p.half_width_m[0], "taller than wide");
        }
        assert!(s.speleothems.is_empty());
    }

    #[test]
    fn a_mine_drains_out_of_its_portal_and_its_shaft_reaches_the_surface() {
        let hf = hill();
        let g = make(
            CaveParams {
                kind: CaveKind::Mine,
                seed: 1,
                reach_m: 200.0,
                entrances: vec![[512.0, 250.0]],
                ..Default::default()
            },
            &hf,
        );
        let s = &g.system;
        let adit = &s.passages[0];
        let (a, b) = (adit.points[1], adit.points[adit.points.len() - 1]);
        let run = (b[0] - a[0]).hypot(b[1] - a[1]);
        assert!(((b[2] - a[2]) / run - 0.005).abs() < 1e-3, "1 in 200");
        assert_eq!(adit.profile, Profile::Arch);
        assert!((adit.half_width_m[0] * 2.0 - 2.4).abs() < 0.01);
        let shaft = s
            .passages
            .iter()
            .find(|p| p.profile == Profile::Rect)
            .expect("shaft");
        assert!(shaft.points[0][2] > ground(&hf, EXT, shaft.points[0][0], shaft.points[0][1]));
        assert!(s.entrances.iter().any(|e| e.kind == EntranceKind::Shaft));
        assert!(s.speleothems.is_empty());
        assert!(s.roughness_m < 0.2);
    }

    #[test]
    fn a_tunnel_joins_two_portals_and_flags_a_steep_grade() {
        let hf = hill();
        let g = make(
            CaveParams {
                kind: CaveKind::Tunnel,
                entrances: vec![[512.0, 200.0], [512.0, 820.0]],
                ..Default::default()
            },
            &hf,
        );
        assert_eq!(g.system.entrances.len(), 2);
        assert!(g.notes.is_empty(), "{:?}", g.notes);
        let p = &g.system.passages[0];
        assert!(
            p.points.iter().all(|q| (q[0] - 512.0).abs() < 1e-3),
            "straight"
        );
        let steep = make(
            CaveParams {
                kind: CaveKind::Tunnel,
                entrances: vec![[512.0, 200.0], [512.0, 420.0]],
                ..Default::default()
            },
            &hf,
        );
        assert!(
            steep.notes.iter().any(|n| n.contains("grade")),
            "{:?}",
            steep.notes
        );
        assert!(
            generate_cave(
                &CaveParams {
                    kind: CaveKind::Tunnel,
                    entrances: vec![[1.0, 1.0]],
                    ..Default::default()
                },
                &Ground {
                    hf: &hf,
                    extent_m: EXT,
                    water: None
                }
            )
            .is_err()
        );
    }

    #[test]
    fn catacombs_are_a_shallow_connected_maze_reached_by_a_stair() {
        let hf = hill();
        let g = make(
            CaveParams {
                kind: CaveKind::Catacombs,
                seed: 3,
                reach_m: 80.0,
                entrances: vec![[300.0, 300.0]],
                ..Default::default()
            },
            &hf,
        );
        let s = &g.system;
        let n = 10usize; // 80 m / 8 m
        assert!(
            s.passages.len() > n * n - 1,
            "a spanning maze plus loops: {}",
            s.passages.len()
        );
        let galleries: Vec<&Passage> = s
            .passages
            .iter()
            .filter(|p| p.points.len() == 2 && (p.points[0][2] - p.points[1][2]).abs() < 1e-3)
            .collect();
        let z = galleries[0].points[0][2];
        assert!(
            galleries.iter().all(|p| (p.points[0][2] - z).abs() < 1e-3),
            "one level"
        );
        assert!(s.entrances.iter().any(|e| e.kind == EntranceKind::Shaft));
    }

    #[test]
    fn a_spring_under_a_lake_makes_an_underwater_entrance() {
        let hf = hill();
        let mut water = Heightfield::new(128, 128, 64, 2).expect("w");
        // A lake 6 m deep over the low south-west corner.
        water.fill_with(|x, y| if x < 20 && y < 20 { 6.0 } else { 0.0 });
        let g = generate_cave(
            &CaveParams {
                seed: 8,
                reach_m: 900.0,
                entrances: vec![[400.0, 420.0]],
                ..Default::default()
            },
            &Ground {
                hf: &hf,
                extent_m: EXT,
                water: Some(&water),
            },
        )
        .expect("gen");
        let spring = g
            .system
            .entrances
            .iter()
            .find(|e| e.kind == EntranceKind::Resurgence)
            .expect("spring");
        assert!(spring.underwater, "{spring:?}");
        assert!(spring.at[0] < 170.0 && spring.at[1] < 170.0);
        let wl = g.system.water_level_m.expect("wl");
        assert!(
            (wl - (spring.at[2] + 6.0)).abs() < 0.6,
            "water table at the lake surface: {wl}"
        );
    }
}
