//! Water for engines and the viewport, as meshes in the terrain export frame (x = u − extent/2
//! east, y up, z = extent/2 − v south; vertices at cell centres, as [`GridMesh::build`]):
//! the **surface** of standing water (lakes, rivers, wetlands, plunge pools — any depth layer)
//! and the **sheets** of waterfalls.
//!
//! ponytail: the surface is the terrain grid lifted by the depth, so it follows the ground's
//! resolution (no decimation); a fall is a ribbon, not a particle system — engines add spray.

use jd_types::{JdError, JdResult};

use crate::Heightfield;
use crate::mesh::GridMesh;

/// Water shallower than this (metres) is dry ground for the surface mesh.
pub const WET_M: f32 = 0.05;

/// Rows of a fall's ribbon from lip to foot.
pub const FALL_ROWS: u32 = 10;

/// Water leaving a lip horizontally, m/s. INFERRED: a typical mountain-stream velocity (rivers
/// run at ~1–3 m/s).
pub const JET_MPS: f32 = 1.5;

/// A waterfall to draw: lip and foot `[u, v, z]` in metres from the world's south-west corner
/// (`hydrology.waterfalls`), and its width across the river.
#[derive(Debug, Clone, PartialEq)]
pub struct FallSheet {
    /// Top of the fall.
    pub lip: [f32; 3],
    /// Foot of the fall.
    pub base: [f32; 3],
    /// Width across the river, metres.
    pub width_m: f32,
}

impl FallSheet {
    /// Downstream direction (u, v) and how far out the jet lands: a jet leaving the lip
    /// horizontally at [`JET_MPS`] has covered `JET_MPS·√(2·drop/g)` when it reaches the foot,
    /// and lands at least as far out as the foot.
    pub fn reach(&self) -> ([f32; 2], f32) {
        let (du, dv) = (self.base[0] - self.lip[0], self.base[1] - self.lip[1]);
        let run = du.hypot(dv);
        let dir = if run > 1e-3 {
            [du / run, dv / run]
        } else {
            [0.0, -1.0]
        };
        let drop = (self.lip[2] - self.base[2]).max(0.0);
        (dir, run.max(JET_MPS * (2.0 * drop / 9.81).sqrt()))
    }
}

/// The surface of standing water: `ground` lifted by `depth` (both on the same grid), keeping
/// the triangles that touch a wet cell (≥ [`WET_M`]); their dry corners stay on the ground, so
/// the water meets its banks. UVs span the world (0..1, v = 0 on the south edge). `None` when
/// nothing is wet.
pub fn water_surface(
    ground: &Heightfield,
    depth: &Heightfield,
    extent_m: [f64; 2],
) -> JdResult<Option<GridMesh>> {
    if (ground.width(), ground.height()) != (depth.width(), depth.height()) {
        return Err(JdError::invalid("depth", "must match the terrain's size"));
    }
    let mut surface = ground.clone();
    surface.fill_with(|x, y| ground.get(x, y) + depth.get(x, y).max(0.0));
    let full = GridMesh::build(&surface, extent_m, 1)?;
    let wet: Vec<bool> = depth.to_vec().iter().map(|d| *d >= WET_M).collect();
    let mut remap = vec![u32::MAX; wet.len()];
    let mut m = GridMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        grid: full.grid,
        height_range: [f32::MAX, f32::MIN],
    };
    for t in full.indices.as_chunks::<3>().0 {
        if !t.iter().any(|i| wet[*i as usize]) {
            continue;
        }
        for &i in t {
            let i = i as usize;
            if remap[i] == u32::MAX {
                remap[i] = (m.positions.len() / 3) as u32;
                m.positions
                    .extend_from_slice(&full.positions[i * 3..i * 3 + 3]);
                m.normals.extend_from_slice(&full.normals[i * 3..i * 3 + 3]);
                m.uvs.extend_from_slice(&full.uvs[i * 2..i * 2 + 2]);
                let y = full.positions[i * 3 + 1];
                m.height_range = [m.height_range[0].min(y), m.height_range[1].max(y)];
            }
            m.indices.push(remap[i]);
        }
    }
    Ok((!m.indices.is_empty()).then_some(m))
}

/// Each fall as a ribbon of falling water from its lip to where its jet lands
/// ([`FallSheet::reach`]), bowed out along the free-fall arc (clear of the rock face), `width_m`
/// across. Two-sided as geometry — a back copy with flipped normals — since an engine material
/// is one-sided by default. UV: u across (0..1), v down the fall (0 at the lip, 1 at the foot).
pub fn fall_sheets(falls: &[FallSheet], extent_m: [f32; 2]) -> GridMesh {
    let mut m = GridMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        grid: [0, 0],
        height_range: [f32::MAX, f32::MIN],
    };
    let per = 2 * (FALL_ROWS + 1);
    for f in falls {
        let (dir, reach_m) = f.reach();
        let side = [-dir[1] * f.width_m / 2.0, dir[0] * f.width_m / 2.0];
        let n = {
            let l = dir[0].hypot(dir[1]).max(f32::EPSILON);
            [dir[0] / l, 0.0, -dir[1] / l]
        };
        for back in [false, true] {
            let first = (m.positions.len() / 3) as u32;
            for k in 0..=FALL_ROWS {
                let t = k as f32 / FALL_ROWS as f32;
                let r = reach_m * t.sqrt();
                let (u, v, z) = (
                    f.lip[0] + dir[0] * r,
                    f.lip[1] + dir[1] * r,
                    f.lip[2] + (f.base[2] - f.lip[2]) * t,
                );
                for (s, across) in [(-1.0, 0.0), (1.0, 1.0)] {
                    let (uu, vv) = (u + s * side[0], v + s * side[1]);
                    m.positions.extend_from_slice(&[
                        uu - extent_m[0] / 2.0,
                        z,
                        extent_m[1] / 2.0 - vv,
                    ]);
                    let sign = if back { -1.0 } else { 1.0 };
                    m.normals.extend_from_slice(&n.map(|c| c * sign));
                    m.uvs.extend_from_slice(&[across, t]);
                    m.height_range = [m.height_range[0].min(z), m.height_range[1].max(z)];
                }
            }
            debug_assert_eq!((m.positions.len() / 3) as u32 - first, per);
            for k in 0..FALL_ROWS {
                let (a, b) = (first + 2 * k, first + 2 * k + 2);
                if back {
                    m.indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
                } else {
                    m.indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
                }
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_surface_lifts_wet_cells_by_their_depth_and_meets_the_banks() {
        // A 10 × 10 world of 10 m cells at 100 m, a 2 m pond on cells (4..6, 4..6).
        let mut ground = Heightfield::new(10, 10, 64, 0).expect("hf");
        ground.fill_with(|_, _| 100.0);
        let mut depth = ground.clone();
        depth.fill_with(|x, y| {
            if (4..6).contains(&x) && (4..6).contains(&y) {
                2.0
            } else {
                0.0
            }
        });
        let m = water_surface(&ground, &depth, [100.0, 100.0])
            .expect("ok")
            .expect("wet");
        let ys: Vec<f32> = m
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| p[1])
            .collect();
        // The 2 × 2 pond and the bank vertices of the triangles touching it: the 4 × 4 ring less
        // the two corners whose quads split along the other diagonal (their triangles are dry).
        assert_eq!(ys.iter().filter(|y| **y == 102.0).count(), 4);
        assert_eq!(ys.iter().filter(|y| **y == 100.0).count(), 10);
        assert_eq!(ys.len(), 14);
        assert_eq!(m.height_range, [100.0, 102.0]);
        // Cell (4, 4)'s centre: 45 m east, 45 m north → x = −5, z = +5.
        assert!(
            m.positions
                .as_chunks::<3>()
                .0
                .iter()
                .any(|p| *p == [-5.0, 102.0, 5.0])
        );
        let mut dry = depth.clone();
        dry.fill_with(|_, _| 0.01);
        assert!(
            water_surface(&ground, &dry, [100.0, 100.0])
                .expect("ok")
                .is_none()
        );
        let small = Heightfield::new(5, 5, 64, 0).expect("hf");
        assert!(water_surface(&ground, &small, [100.0, 100.0]).is_err());
    }

    #[test]
    fn a_fall_sheet_runs_from_its_lip_and_lands_beyond_its_foot_on_both_sides() {
        // A 20 m fall in a 100 m world, flowing south over 2 m, 6 m wide.
        let f = FallSheet {
            lip: [50.0, 52.0, 40.0],
            base: [50.0, 50.0, 20.0],
            width_m: 6.0,
        };
        let (dir, reach) = f.reach();
        assert_eq!(dir, [0.0, -1.0]);
        // √(2·20/9.81) s at 1.5 m/s ≈ 3.03 m: farther out than the 2 m run.
        assert!(
            (reach - 1.5 * (40.0f32 / 9.81).sqrt()).abs() < 1e-4,
            "{reach}"
        );
        let m = fall_sheets(std::slice::from_ref(&f), [100.0, 100.0]);
        let per = 2 * (FALL_ROWS + 1) as usize;
        assert_eq!(m.positions.len() / 3, 2 * per);
        assert_eq!(m.indices.len(), 2 * FALL_ROWS as usize * 6);
        let p = m.positions.as_chunks::<3>().0;
        // Lip row at the lip (v 52 → z = −2), last row at the foot height, `reach` downstream.
        assert_eq!(p[0], [-3.0, 40.0, -2.0]);
        assert_eq!(p[1], [3.0, 40.0, -2.0]);
        let last = p[per - 1];
        assert!(
            (last[1] - 20.0).abs() < 1e-4 && (last[2] - (-2.0 + reach)).abs() < 1e-4,
            "{last:?}"
        );
        // The back copy faces the other way.
        let n = m.normals.as_chunks::<3>().0;
        assert_eq!(n[0].map(|c| -c), n[per]);
        assert_eq!(m.height_range, [20.0, 40.0]);
    }
}
