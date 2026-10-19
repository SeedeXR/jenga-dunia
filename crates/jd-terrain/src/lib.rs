//! Terrain data and generators.
//!
//! [`Heightfield`] is an f32 raster stored as tiles with halos (ADR-0005) so
//! every kernel runs per tile with bounded memory. Generators are deterministic
//! for a given seed and independent of tiling or thread count (ADR-0007).
//! Export writes the formats engines actually consume (`docs/architecture/08-io-formats.md`).

pub mod export;
pub mod heightfield;
pub mod noise;
pub mod preview;

pub use export::{HeightmapFormat, export_heightmap};
pub use heightfield::{Heightfield, HeightfieldStats, Tile, TileCoord};
pub use noise::{FractalKind, NoiseKind, NoiseParams, generate_noise};
