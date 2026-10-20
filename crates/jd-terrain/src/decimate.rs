//! Quadric edge-collapse decimation (Garland & Heckbert 1997) for the terrain grid mesh: flat
//! ground collapses to a few large triangles, ridges and gullies keep theirs. Every vertex
//! carries the sum of its faces' plane quadrics; the cheapest edge (error of its optimal merged
//! position) collapses first; the field border is pinned with perpendicular boundary planes so
//! the footprint never shrinks; a collapse that would flip a face is skipped.
//!
//! Attributes: positions come out of the quadric solve, normals are recomputed from the faces,
//! UVs are the planar terrain mapping of the position (u east, v north from the south edge), so
//! nothing has to be interpolated along the collapse.
//!
//! ponytail: no vertex-cost aggregation tricks, no memoryless variant, one global heap. 1 M
//! triangles → 100 k takes seconds in a release build; `step` bounds the input.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

use jd_types::{JdError, JdResult};

use crate::mesh::GridMesh;

/// 4×4 symmetric quadric, upper triangle (10 coefficients).
#[derive(Clone, Copy, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    fn plane(a: f64, b: f64, c: f64, d: f64) -> Self {
        Quadric([
            a * a,
            a * b,
            a * c,
            a * d,
            b * b,
            b * c,
            b * d,
            c * c,
            c * d,
            d * d,
        ])
    }
    fn add(&self, o: &Quadric) -> Quadric {
        let mut q = self.0;
        for (a, b) in q.iter_mut().zip(o.0) {
            *a += b;
        }
        Quadric(q)
    }
    fn scaled(&self, k: f64) -> Quadric {
        Quadric(self.0.map(|v| v * k))
    }
    /// vᵀ Q v for v = (x, y, z, 1).
    fn error(&self, p: [f64; 3]) -> f64 {
        let q = &self.0;
        let [x, y, z] = p;
        q[0] * x * x
            + 2.0 * q[1] * x * y
            + 2.0 * q[2] * x * z
            + 2.0 * q[3] * x
            + q[4] * y * y
            + 2.0 * q[5] * y * z
            + 2.0 * q[6] * y
            + q[7] * z * z
            + 2.0 * q[8] * z
            + q[9]
    }
    /// Position minimising the error, when the 3×3 block is invertible.
    fn optimum(&self) -> Option<[f64; 3]> {
        let q = &self.0;
        let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if det.abs() < 1e-12 {
            return None;
        }
        let b = [-q[3], -q[6], -q[8]];
        let inv = |r: usize, c: usize| {
            let (i0, i1) = ((r + 1) % 3, (r + 2) % 3);
            let (j0, j1) = ((c + 1) % 3, (c + 2) % 3);
            (m[j0][i0] * m[j1][i1] - m[j0][i1] * m[j1][i0]) / det
        };
        let x = inv(0, 0) * b[0] + inv(0, 1) * b[1] + inv(0, 2) * b[2];
        let y = inv(1, 0) * b[0] + inv(1, 1) * b[1] + inv(1, 2) * b[2];
        let z = inv(2, 0) * b[0] + inv(2, 1) * b[1] + inv(2, 2) * b[2];
        [x, y, z].iter().all(|v| v.is_finite()).then_some([x, y, z])
    }
}

#[derive(PartialEq)]
struct Candidate {
    cost: f64,
    a: u32,
    b: u32,
    /// Versions of both vertices when the cost was computed; stale entries are skipped.
    va: u32,
    vb: u32,
    target: [f64; 3],
}

impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> Ordering {
        // Min-heap on cost.
        o.cost.partial_cmp(&self.cost).unwrap_or(Ordering::Equal)
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Collapse edges of `mesh` until at most `target_triangles` remain (or no safe collapse is left).
/// Returns an irregular mesh (`grid` = [0, 0]) in the same frame with recomputed normals and
/// planar UVs over `extent_m`.
pub fn decimate(mesh: &GridMesh, extent_m: [f64; 2], target_triangles: u32) -> JdResult<GridMesh> {
    let n_faces = mesh.indices.len() / 3;
    if target_triangles == 0 {
        return Err(JdError::invalid("target_triangles", "must be ≥ 1"));
    }
    if n_faces as u32 <= target_triangles {
        return Ok(GridMesh {
            positions: mesh.positions.clone(),
            normals: mesh.normals.clone(),
            uvs: mesh.uvs.clone(),
            indices: mesh.indices.clone(),
            grid: [0, 0],
            height_range: mesh.height_range,
        });
    }
    let nv = mesh.positions.len() / 3;
    let mut pos: Vec<[f64; 3]> = mesh
        .positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
        .collect();
    let mut faces: Vec<[u32; 3]> = mesh.indices.as_chunks::<3>().0.to_vec();
    let mut alive = vec![true; faces.len()];
    let mut vfaces: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (fi, f) in faces.iter().enumerate() {
        for &v in f {
            vfaces[v as usize].push(fi as u32);
        }
    }
    // Vertex quadrics from face planes (area-weighted), plus border pins.
    let mut quad = vec![Quadric::default(); nv];
    let face_plane = |f: &[u32; 3], pos: &[[f64; 3]]| -> Option<([f64; 3], f64, f64)> {
        let (a, b, c) = (pos[f[0] as usize], pos[f[1] as usize], pos[f[2] as usize]);
        let n = cross(sub(b, a), sub(c, a));
        let len = norm(n);
        if len < 1e-18 {
            return None;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        Some((n, -dot(n, a), len * 0.5))
    };
    for f in &faces {
        if let Some((n, d, area)) = face_plane(f, &pos) {
            let q = Quadric::plane(n[0], n[1], n[2], d).scaled(area);
            for &v in f {
                quad[v as usize] = quad[v as usize].add(&q);
            }
        }
    }
    // Edge set and boundary edges (edges with one face). Boundary vertices get a heavy plane
    // perpendicular to each boundary edge so the footprint stays put.
    let mut edge_faces: std::collections::HashMap<(u32, u32), u32> =
        std::collections::HashMap::with_capacity(faces.len() * 3);
    for f in &faces {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            *edge_faces.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    let diag = (extent_m[0] * extent_m[0] + extent_m[1] * extent_m[1]).sqrt();
    let pin_weight = diag * diag * 1e3;
    let mut boundary = vec![false; nv];
    for (&(a, b), &count) in &edge_faces {
        if count == 1 {
            boundary[a as usize] = true;
            boundary[b as usize] = true;
            // Find the face to orient the perpendicular plane: n_face × edge.
            let fi = vfaces[a as usize]
                .iter()
                .find(|&&fi| faces[fi as usize].contains(&b))
                .copied();
            if let Some(fi) = fi
                && let Some((n, _, _)) = face_plane(&faces[fi as usize], &pos)
            {
                let e = sub(pos[b as usize], pos[a as usize]);
                let p = cross(n, e);
                let len = norm(p);
                if len > 1e-18 {
                    let p = [p[0] / len, p[1] / len, p[2] / len];
                    let q = Quadric::plane(p[0], p[1], p[2], -dot(p, pos[a as usize]))
                        .scaled(pin_weight);
                    quad[a as usize] = quad[a as usize].add(&q);
                    quad[b as usize] = quad[b as usize].add(&q);
                }
            }
        }
    }
    // The footprint: an optimal position may not leave it (the terrain ends at its extent).
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mut version = vec![0u32; nv];
    let mut heap = BinaryHeap::with_capacity(edge_faces.len());
    let candidate = |a: u32,
                     b: u32,
                     pos: &[[f64; 3]],
                     quad: &[Quadric],
                     version: &[u32],
                     boundary: &[bool]|
     -> Candidate {
        let q = quad[a as usize].add(&quad[b as usize]);
        let (pa, pb) = (pos[a as usize], pos[b as usize]);
        let mid = [
            (pa[0] + pb[0]) * 0.5,
            (pa[1] + pb[1]) * 0.5,
            (pa[2] + pb[2]) * 0.5,
        ];
        // Border vertices only slide along the border: keep one endpoint rather than the solve.
        let mut options: Vec<[f64; 3]> = Vec::with_capacity(4);
        if !(boundary[a as usize] || boundary[b as usize])
            && let Some(o) = q.optimum()
        {
            options.push([
                o[0].clamp(lo[0], hi[0]),
                o[1].clamp(lo[1], hi[1]),
                o[2].clamp(lo[2], hi[2]),
            ]);
        }
        options.extend([pa, pb, mid]);
        let (target, cost) = options
            .into_iter()
            .map(|p| (p, q.error(p)))
            .min_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(Ordering::Equal))
            .unwrap_or((mid, f64::MAX));
        Candidate {
            cost,
            a,
            b,
            va: version[a as usize],
            vb: version[b as usize],
            target,
        }
    };
    for &(a, b) in edge_faces.keys() {
        heap.push(candidate(a, b, &pos, &quad, &version, &boundary));
    }
    let mut remaining = n_faces as u32;
    let mut merged_into: Vec<u32> = (0..nv as u32).collect();
    let find = |mut v: u32, merged: &[u32]| {
        while merged[v as usize] != v {
            v = merged[v as usize];
        }
        v
    };
    while remaining > target_triangles {
        let Some(c) = heap.pop() else { break };
        let (a, b) = (find(c.a, &merged_into), find(c.b, &merged_into));
        if a == b || c.va != version[c.a as usize] || c.vb != version[c.b as usize] {
            continue;
        }
        // Both endpoints on the border but not along one border edge: collapsing would cut a corner.
        if boundary[a as usize]
            && boundary[b as usize]
            && edge_faces.get(&(a.min(b), a.max(b))).copied() != Some(1)
        {
            continue;
        }
        // Flip test: faces around a and b (that do not die) must keep their orientation.
        let dying: HashSet<u32> = vfaces[a as usize]
            .iter()
            .filter(|&&fi| alive[fi as usize] && faces[fi as usize].contains(&b))
            .copied()
            .collect();
        let mut flips = false;
        for &fi in vfaces[a as usize].iter().chain(vfaces[b as usize].iter()) {
            if !alive[fi as usize] || dying.contains(&fi) {
                continue;
            }
            let f = faces[fi as usize];
            let before = face_plane(&f, &pos).map(|p| p.0);
            let moved: [u32; 3] = f.map(|v| if v == a || v == b { a } else { v });
            let get = |v: u32| if v == a { c.target } else { pos[v as usize] };
            let (p0, p1, p2) = (get(moved[0]), get(moved[1]), get(moved[2]));
            let n = cross(sub(p1, p0), sub(p2, p0));
            let len = norm(n);
            // Stay a heightfield: every face keeps facing up (n.y > 0) and near its old normal.
            if len < 1e-12
                || n[1] <= 1e-9
                || before.is_none_or(|bn| dot(bn, [n[0] / len, n[1] / len, n[2] / len]) < 0.2)
            {
                flips = true;
                break;
            }
        }
        if flips {
            continue;
        }
        // Collapse b into a.
        pos[a as usize] = c.target;
        quad[a as usize] = quad[a as usize].add(&quad[b as usize]);
        boundary[a as usize] |= boundary[b as usize];
        merged_into[b as usize] = a;
        version[a as usize] += 1;
        version[b as usize] += 1;
        for &fi in &dying {
            alive[fi as usize] = false;
            remaining -= 1;
        }
        let moved_faces: Vec<u32> = vfaces[b as usize]
            .iter()
            .filter(|&&fi| alive[fi as usize])
            .copied()
            .collect();
        for &fi in &moved_faces {
            for v in &mut faces[fi as usize] {
                if *v == b {
                    *v = a;
                }
            }
            vfaces[a as usize].push(fi);
        }
        vfaces[b as usize].clear();
        vfaces[a as usize].retain(|&fi| alive[fi as usize]);
        // Boundary bookkeeping for the merged vertex's edges and new candidates around it.
        let mut neighbours: HashSet<u32> = HashSet::new();
        for &fi in &vfaces[a as usize] {
            for &v in &faces[fi as usize] {
                if v != a {
                    neighbours.insert(v);
                }
            }
        }
        for &v in &neighbours {
            let key = (a.min(v), a.max(v));
            let count = vfaces[a as usize]
                .iter()
                .filter(|&&fi| faces[fi as usize].contains(&v))
                .count() as u32;
            edge_faces.insert(key, count);
            version[v as usize] += 1;
            heap.push(candidate(a, v, &pos, &quad, &version, &boundary));
        }
    }
    // Compact: keep live vertices and faces.
    let mut remap = vec![u32::MAX; nv];
    let mut positions = Vec::new();
    let mut indices = Vec::with_capacity(remaining as usize * 3);
    for (fi, f) in faces.iter().enumerate() {
        if !alive[fi] || f[0] == f[1] || f[1] == f[2] || f[0] == f[2] {
            continue;
        }
        for &v in f {
            if remap[v as usize] == u32::MAX {
                remap[v as usize] = (positions.len() / 3) as u32;
                let p = pos[v as usize];
                positions.extend_from_slice(&[p[0] as f32, p[1] as f32, p[2] as f32]);
            }
            indices.push(remap[v as usize]);
        }
    }
    let n_out = positions.len() / 3;
    let mut normals = vec![0.0f32; n_out * 3];
    for f in indices.as_chunks::<3>().0 {
        let p = |i: u32| {
            let o = i as usize * 3;
            [
                positions[o] as f64,
                positions[o + 1] as f64,
                positions[o + 2] as f64,
            ]
        };
        let n = cross(sub(p(f[1]), p(f[0])), sub(p(f[2]), p(f[0])));
        for &v in f {
            for k in 0..3 {
                normals[v as usize * 3 + k] += n[k] as f32;
            }
        }
    }
    for n in normals.as_chunks_mut::<3>().0 {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        n[0] /= len;
        n[1] /= len;
        n[2] /= len;
    }
    let uvs: Vec<f32> = positions
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|p| {
            [
                ((p[0] as f64 + extent_m[0] / 2.0) / extent_m[0]) as f32,
                ((extent_m[1] / 2.0 - p[2] as f64) / extent_m[1]) as f32,
            ]
        })
        .collect();
    Ok(GridMesh {
        positions,
        normals,
        uvs,
        indices,
        grid: [0, 0],
        height_range: mesh.height_range,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Heightfield;

    #[test]
    fn decimation_keeps_the_footprint_and_the_surface_within_a_cell() {
        // A ridge across a plain: the plain should collapse to few triangles, the ridge keep many.
        let mut hf = Heightfield::new(65, 65, 64, 0).expect("hf");
        hf.fill_with(|x, y| {
            let r = ((x as f32 - 32.0).powi(2) + (y as f32 - 32.0).powi(2)).sqrt();
            100.0 + 40.0 * (-(r - 16.0).powi(2) / 18.0).exp()
        });
        let extent = [640.0, 640.0];
        let full = GridMesh::build(&hf, extent, 1).expect("grid");
        let target = 1200;
        let out = decimate(&full, extent, target).expect("decimate");
        let tris = out.indices.len() / 3;
        assert!(
            tris <= target as usize && tris > target as usize / 2,
            "triangles {tris}"
        );
        assert!(out.positions.len() / 3 < full.positions.len() / 3 / 3);
        // Footprint: still spans the original mesh's bounding box (vertices sit at cell centres,
        // 640 m / 65 cells, so the border is at ±315.08 m).
        let bbox = |m: &GridMesh, k: usize| {
            let v: Vec<f32> = m
                .positions
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| p[k])
                .collect();
            (
                v.iter().cloned().fold(f32::MAX, f32::min),
                v.iter().cloned().fold(f32::MIN, f32::max),
            )
        };
        assert_eq!(bbox(&out, 0), bbox(&full, 0), "x footprint");
        assert_eq!(bbox(&out, 2), bbox(&full, 2), "z footprint");
        // Surface error: every output vertex sits within a cell's height change of the field.
        let sx = 640.0 / 65.0;
        let mut worst = 0.0f32;
        for p in out.positions.as_chunks::<3>().0 {
            let gx = ((p[0] + 320.0) / sx - 0.5).round().clamp(0.0, 64.0) as u32;
            let gz = ((320.0 - p[2]) / sx - 0.5).round().clamp(0.0, 64.0) as u32;
            worst = worst.max((p[1] - hf.get(gx, gz)).abs());
        }
        assert!(worst < 6.0, "worst vertex height error {worst} m");
        // Every face is counter-clockwise from above (no flips) and non-degenerate; UVs in 0..1.
        for f in out.indices.as_chunks::<3>().0 {
            let p = |i: u32| {
                [
                    out.positions[i as usize * 3] as f64,
                    out.positions[i as usize * 3 + 2] as f64,
                ]
            };
            let (a, b, c) = (p(f[0]), p(f[1]), p(f[2]));
            // In the XZ plane (viewed from +Y, Z towards the viewer) counter-clockwise means a negative 2-D cross.
            let cross2 = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert!(cross2 < 0.0, "face {f:?} flipped or degenerate: {cross2}");
        }
        assert!(out.uvs.iter().all(|u| (0.0..=1.0).contains(u)));
        // Flat ground is where the triangles went: count faces touching the ridge annulus vs the rest.
        let on_ridge = out
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|f| {
                f.iter().any(|&i| {
                    let (x, z) = (
                        out.positions[i as usize * 3],
                        out.positions[i as usize * 3 + 2],
                    );
                    ((x * x + z * z).sqrt() - 160.0).abs() < 60.0
                })
            })
            .count();
        assert!(on_ridge > tris / 2, "ridge faces {on_ridge} of {tris}");
        assert!(decimate(&full, extent, 0).is_err());
    }
}
