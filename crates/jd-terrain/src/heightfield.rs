//! Tiled f32 heightfield.
//!
//! Storage: a grid of square tiles (`tile_size` power of two) each holding
//! `(tile_size + 2·halo)²` samples row-major (y-major), including a halo ring
//! copied from neighbours so stencil kernels never read across tiles. The
//! field's logical size need not be a multiple of the tile size; edge tiles are
//! padded with edge-clamped values. Pixel (0,0) is the south-west corner
//! (`+x` east, `+y` north); format adapters handle any flipping.

use jd_types::{ErrorCode, JdError, JdResult, WorldSettings};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Tile coordinate in the tile grid (lod 0 only for now).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TileCoord {
    /// Column.
    pub x: u32,
    /// Row.
    pub y: u32,
}

/// One tile with halo.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Position in the tile grid.
    pub coord: TileCoord,
    /// Interior edge length in pixels.
    pub size: u32,
    /// Halo width in pixels.
    pub halo: u32,
    /// `(size + 2·halo)²` samples, row-major.
    pub data: Vec<f32>,
}

impl Tile {
    /// Padded edge length.
    pub fn stride(&self) -> usize {
        (self.size + 2 * self.halo) as usize
    }

    /// Sample at interior pixel `(ix, iy)` (may index into the halo with negative offsets via `get_padded`).
    pub fn get(&self, ix: u32, iy: u32) -> f32 {
        self.get_padded(ix as i64, iy as i64)
    }

    /// Sample at padded coordinates where `-halo ..= size+halo-1` are valid.
    pub fn get_padded(&self, ix: i64, iy: i64) -> f32 {
        let h = self.halo as i64;
        let s = self.stride() as i64;
        let (px, py) = (ix + h, iy + h);
        debug_assert!(
            px >= 0 && py >= 0 && px < s && py < s,
            "padded index out of range"
        );
        self.data[(py * s + px) as usize]
    }

    /// Mutable interior sample.
    pub fn set(&mut self, ix: u32, iy: u32, v: f32) {
        let h = self.halo as usize;
        let s = self.stride();
        self.data[(iy as usize + h) * s + ix as usize + h] = v;
    }

    /// blake3 hash of the interior samples (halo excluded) — the cache key component.
    pub fn content_hash(&self) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();
        let h = self.halo as usize;
        let s = self.stride();
        // ponytail: per-row byte copy instead of an unsafe reinterpret (unsafe is forbidden workspace-wide).
        let mut row_bytes = Vec::with_capacity(self.size as usize * 4);
        for iy in 0..self.size as usize {
            row_bytes.clear();
            for v in &self.data[(iy + h) * s + h..(iy + h) * s + h + self.size as usize] {
                row_bytes.extend_from_slice(&v.to_le_bytes());
            }
            hasher.update(&row_bytes);
        }
        hasher.finalize()
    }
}

/// Summary statistics over the whole field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HeightfieldStats {
    /// Minimum height.
    pub min: f32,
    /// Maximum height.
    pub max: f32,
    /// Mean height.
    pub mean: f64,
    /// Count of non-finite samples (must be 0 for a valid field).
    pub non_finite: u64,
}

/// A tiled f32 heightfield.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightfield {
    width: u32,
    height: u32,
    tile_size: u32,
    halo: u32,
    tiles_x: u32,
    tiles_y: u32,
    tiles: Vec<Tile>,
}

impl Heightfield {
    /// A zero-filled field.
    pub fn new(width: u32, height: u32, tile_size: u32, halo: u32) -> JdResult<Self> {
        if width == 0 || height == 0 {
            return Err(JdError::invalid(
                "resolution_px",
                "heightfield dimensions must be positive",
            ));
        }
        if !tile_size.is_power_of_two() || halo > tile_size / 2 {
            return Err(JdError::invalid(
                "tile_size",
                "tile_size must be a power of two and halo ≤ tile_size/2",
            ));
        }
        let tiles_x = width.div_ceil(tile_size);
        let tiles_y = height.div_ceil(tile_size);
        let stride = (tile_size + 2 * halo) as usize;
        let tiles = (0..tiles_y)
            .flat_map(|y| (0..tiles_x).map(move |x| TileCoord { x, y }))
            .map(|coord| Tile {
                coord,
                size: tile_size,
                halo,
                data: vec![0.0; stride * stride],
            })
            .collect();
        Ok(Self {
            width,
            height,
            tile_size,
            halo,
            tiles_x,
            tiles_y,
            tiles,
        })
    }

    /// A `w × h` window starting at `(x0, y0)` as its own field (same tile size and halo).
    pub fn crop(&self, x0: u32, y0: u32, w: u32, h: u32) -> JdResult<Self> {
        if w == 0 || h == 0 || x0 + w > self.width() || y0 + h > self.height() {
            return Err(JdError::invalid("crop", "window must lie inside the field"));
        }
        let mut out = Self::new(w, h, self.tile_size(), self.halo())?;
        out.fill_with(|x, y| self.get(x0 + x, y0 + y));
        Ok(out)
    }

    /// A field sized from world settings.
    pub fn from_world(world: &WorldSettings) -> JdResult<Self> {
        Self::new(
            world.resolution_px[0],
            world.resolution_px[1],
            world.tile_size,
            world.halo,
        )
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Tile edge length.
    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }
    /// Halo width.
    pub fn halo(&self) -> u32 {
        self.halo
    }
    /// Tiles, row-major.
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
    /// Number of tiles along x and y.
    pub fn tile_counts(&self) -> (u32, u32) {
        (self.tiles_x, self.tiles_y)
    }

    /// Sample at field pixel `(x, y)`; out of range is clamped to the edge.
    pub fn get(&self, x: u32, y: u32) -> f32 {
        let x = x.min(self.width - 1);
        let y = y.min(self.height - 1);
        let t = &self.tiles[((y / self.tile_size) * self.tiles_x + x / self.tile_size) as usize];
        t.get(x % self.tile_size, y % self.tile_size)
    }

    /// Write one interior sample (no halo update: call [`Self::rebuild_halos`] after the last).
    pub fn set(&mut self, x: u32, y: u32, v: f32) {
        debug_assert!(x < self.width && y < self.height, "sample out of range");
        let (ts, tx) = (self.tile_size, self.tiles_x);
        let t = &mut self.tiles[((y / ts) * tx + x / ts) as usize];
        t.set(x % ts, y % ts, v);
    }

    /// Fill every interior sample in parallel from `f(x, y)` (field pixel coordinates), then rebuild halos.
    /// Deterministic: the result depends only on `f`, never on scheduling.
    pub fn fill_with<F>(&mut self, f: F)
    where
        F: Fn(u32, u32) -> f32 + Sync,
    {
        let (ts, w, h) = (self.tile_size, self.width, self.height);
        self.tiles.par_iter_mut().for_each(|t| {
            let (ox, oy) = (t.coord.x * ts, t.coord.y * ts);
            for iy in 0..ts {
                for ix in 0..ts {
                    // Edge tiles beyond the field are clamped to the last valid pixel.
                    let x = (ox + ix).min(w - 1);
                    let y = (oy + iy).min(h - 1);
                    t.set(ix, iy, f(x, y));
                }
            }
        });
        self.rebuild_halos();
    }

    /// Copy neighbouring interiors into every tile's halo (edge-clamped at the field border).
    /// Two phases so no whole-field clone is needed: gather every ring from the interiors
    /// (read-only, parallel), then write the rings (mutable, parallel). Deterministic.
    pub fn rebuild_halos(&mut self) {
        if self.halo == 0 {
            return;
        }
        let (ts, halo) = (self.tile_size as i64, self.halo as i64);
        let (w, h) = (self.width as i64, self.height as i64);
        let ring_cells = |t: &Tile| {
            let s = t.stride() as i64;
            (0..s)
                .flat_map(move |py| (0..s).map(move |px| (px, py)))
                .filter(move |(px, py)| {
                    let (ix, iy) = (px - halo, py - halo);
                    !(ix >= 0 && iy >= 0 && ix < ts && iy < ts)
                })
        };
        let rings: Vec<Vec<f32>> = self
            .tiles
            .par_iter()
            .map(|t| {
                let (ox, oy) = (t.coord.x as i64 * ts, t.coord.y as i64 * ts);
                ring_cells(t)
                    .map(|(px, py)| {
                        let fx = (ox + px - halo).clamp(0, w - 1) as u32;
                        let fy = (oy + py - halo).clamp(0, h - 1) as u32;
                        self.get(fx, fy)
                    })
                    .collect()
            })
            .collect();
        self.tiles.par_iter_mut().zip(rings).for_each(|(t, ring)| {
            let s = t.stride() as i64;
            let (halo, ts) = (t.halo as i64, t.size as i64);
            let mut k = 0;
            for py in 0..s {
                for px in 0..s {
                    let (ix, iy) = (px - halo, py - halo);
                    if ix >= 0 && iy >= 0 && ix < ts && iy < ts {
                        continue;
                    }
                    t.data[(py * s + px) as usize] = ring[k];
                    k += 1;
                }
            }
        });
    }

    /// Overwrite interior row `y` from `row` (`width` samples) without touching the halos;
    /// call [`Self::rebuild_halos`] once after the last row. Lets a reader stream a field in
    /// without a whole-field buffer.
    pub fn set_row(&mut self, y: u32, row: &[f32]) {
        assert_eq!(row.len(), self.width as usize, "row length");
        let (ts, halo) = (self.tile_size as usize, self.halo as usize);
        let (tx_count, ty) = (self.tiles_x as usize, (y / self.tile_size) as usize);
        let iy = (y % self.tile_size) as usize;
        for tx in 0..tx_count {
            let t = &mut self.tiles[ty * tx_count + tx];
            let s = t.stride();
            let x0 = tx * ts;
            let n = ts.min(self.width as usize - x0);
            let at = (iy + halo) * s + halo;
            t.data[at..at + n].copy_from_slice(&row[x0..x0 + n]);
            // Edge tiles beyond the field repeat the last valid pixel (as fill_with does)…
            let last = row[x0 + n - 1];
            for v in &mut t.data[at + n..at + ts] {
                *v = last;
            }
            // …and the last row repeats down the rest of the bottom tiles.
            if y == self.height - 1 {
                for pad in iy + 1..ts {
                    let to = (pad + halo) * s + halo;
                    let (src, dst) = t.data.split_at_mut(to);
                    dst[..ts].copy_from_slice(&src[at..at + ts]);
                }
            }
        }
    }

    /// Copy interior row `y` into `row` (`width` samples) — the streaming counterpart of
    /// [`Self::set_row`].
    pub fn row(&self, y: u32, row: &mut [f32]) {
        assert_eq!(row.len(), self.width as usize, "row length");
        let (ts, halo) = (self.tile_size as usize, self.halo as usize);
        let (tx_count, ty) = (self.tiles_x as usize, (y / self.tile_size) as usize);
        let iy = (y % self.tile_size) as usize;
        for tx in 0..tx_count {
            let t = &self.tiles[ty * tx_count + tx];
            let s = t.stride();
            let x0 = tx * ts;
            let n = ts.min(self.width as usize - x0);
            let at = (iy + halo) * s + halo;
            row[x0..x0 + n].copy_from_slice(&t.data[at..at + n]);
        }
    }

    /// Row-major copy of the whole field (no halos). Allocates `width·height` floats.
    pub fn to_vec(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; (self.width * self.height) as usize];
        out.par_chunks_mut(self.width as usize)
            .enumerate()
            .for_each(|(y, row)| {
                for (x, v) in row.iter_mut().enumerate() {
                    *v = self.get(x as u32, y as u32);
                }
            });
        out
    }

    /// Statistics; reduction order is fixed (tile order, row order) so results are bit-stable across thread counts.
    pub fn stats(&self) -> HeightfieldStats {
        let partials: Vec<(f32, f32, f64, u64)> = self
            .tiles
            .par_iter()
            .map(|t| {
                let (mut mn, mut mx, mut sum, mut nf) =
                    (f32::INFINITY, f32::NEG_INFINITY, 0.0f64, 0u64);
                let (ox, oy) = (t.coord.x * self.tile_size, t.coord.y * self.tile_size);
                for iy in 0..self.tile_size {
                    if oy + iy >= self.height {
                        break;
                    }
                    for ix in 0..self.tile_size {
                        if ox + ix >= self.width {
                            break;
                        }
                        let v = t.get(ix, iy);
                        if v.is_finite() {
                            mn = mn.min(v);
                            mx = mx.max(v);
                            sum += v as f64;
                        } else {
                            nf += 1;
                        }
                    }
                }
                (mn, mx, sum, nf)
            })
            .collect();
        let (mut mn, mut mx, mut sum, mut nf) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f64, 0u64);
        for (a, b, c, d) in partials {
            mn = mn.min(a);
            mx = mx.max(b);
            sum += c;
            nf += d;
        }
        let n = (self.width as u64 * self.height as u64)
            .saturating_sub(nf)
            .max(1) as f64;
        HeightfieldStats {
            min: mn,
            max: mx,
            mean: sum / n,
            non_finite: nf,
        }
    }

    /// Fail with `E_NUMERIC_UNSTABLE` when any sample is NaN/Inf.
    pub fn validate_finite(&self) -> JdResult<()> {
        let s = self.stats();
        if s.non_finite > 0 {
            Err(JdError::new(
                ErrorCode::E_NUMERIC_UNSTABLE,
                format!("{} non-finite height samples", s.non_finite),
            )
            .why("NaN/Inf heights corrupt meshes and exports.")
            .next("Check generator parameters (frequency, gain) and input data."))
        } else {
            Ok(())
        }
    }

    /// blake3 of all tile content hashes in order — the field's cache identity.
    pub fn content_hash(&self) -> blake3::Hash {
        let mut h = blake3::Hasher::new();
        for t in &self.tiles {
            h.update(t.content_hash().as_bytes());
        }
        h.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_stream_in_and_out_and_halos_match_the_interiors() {
        let mut a = Heightfield::new(70, 50, 32, 4).expect("a");
        a.fill_with(|x, y| (x as f32 * 0.5 + y as f32 * 1.25).sin() * 100.0);
        let mut b = Heightfield::new(70, 50, 32, 4).expect("b");
        let mut row = vec![0.0f32; 70];
        for y in 0..50 {
            a.row(y, &mut row);
            b.set_row(y, &row);
        }
        b.rebuild_halos();
        assert_eq!(a, b, "tiles, edge padding and halos all match");
        assert_eq!(a.content_hash(), b.content_hash());
        // The halo of the middle tile holds its neighbours' interiors (clone-free rebuild).
        let t = &a.tiles()[4]; // tile (1, 1) of the 3 × 2 grid
        assert_eq!(t.get_padded(-1, 0), a.get(31, 32));
        assert_eq!(t.get_padded(32, 5), a.get(64, 37));
        assert_eq!(t.get_padded(3, -2), a.get(35, 30));
    }

    #[test]
    fn fill_get_and_halos_are_consistent_for_non_multiple_sizes() {
        let mut hf = Heightfield::new(100, 70, 64, 4).expect("field");
        assert_eq!(hf.tile_counts(), (2, 2));
        hf.fill_with(|x, y| (x * 1000 + y) as f32);
        assert_eq!(hf.get(99, 69), 99_069.0);
        assert_eq!(hf.get(63, 0), 63_000.0);
        assert_eq!(hf.get(64, 0), 64_000.0);
        // Halo of tile (0,0) at padded (-1, 0) clamps to x=0; at (64, 0) reads tile (1,0)'s x=64.
        let t0 = &hf.tiles()[0];
        assert_eq!(t0.get_padded(-1, 0), 0.0);
        assert_eq!(t0.get_padded(64, 0), 64_000.0);
        assert_eq!(t0.get_padded(65, 3), 65_003.0);
        // Beyond the field edge on the right of tile (1,0): clamps to x=99.
        let t1 = &hf.tiles()[1];
        assert_eq!(t1.get_padded(36, 0), 99_000.0);
        assert_eq!(t1.get_padded(40, 0), 99_000.0);
        let v = hf.to_vec();
        assert_eq!(v.len(), 7000);
        assert_eq!(v[69 * 100 + 99], 99_069.0);
    }

    #[test]
    fn stats_are_thread_count_independent_and_detect_nan() {
        let mut hf = Heightfield::new(300, 200, 128, 2).expect("field");
        hf.fill_with(|x, y| ((x as f32) * 0.37).sin() + ((y as f32) * 0.11).cos());
        let a = hf.stats();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("pool");
        let b = pool.install(|| hf.stats());
        assert_eq!(a, b);
        assert!(hf.validate_finite().is_ok());
        hf.fill_with(|x, y| if x == 7 && y == 9 { f32::NAN } else { 1.0 });
        assert_eq!(hf.stats().non_finite, 1);
        assert_eq!(
            hf.validate_finite().expect_err("nan").code,
            ErrorCode::E_NUMERIC_UNSTABLE
        );
    }

    #[test]
    fn content_hash_ignores_halo_and_tracks_interior() {
        let mut a = Heightfield::new(64, 64, 32, 4).expect("a");
        a.fill_with(|x, y| (x + y) as f32);
        let mut b = Heightfield::new(64, 64, 32, 0).expect("b");
        b.fill_with(|x, y| (x + y) as f32);
        assert_eq!(
            a.content_hash(),
            b.content_hash(),
            "halo must not affect identity"
        );
        b.fill_with(|x, y| (x + y) as f32 + 1.0);
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn invalid_dimensions_are_rejected() {
        assert!(Heightfield::new(0, 10, 64, 1).is_err());
        assert!(Heightfield::new(10, 10, 48, 1).is_err());
        assert!(Heightfield::new(10, 10, 64, 40).is_err());
    }
}
