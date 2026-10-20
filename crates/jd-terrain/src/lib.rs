//! Terrain data and generators.
//!
//! [`Heightfield`] is an f32 raster stored as tiles with halos (ADR-0005) so
//! every kernel runs per tile with bounded memory. Generators are deterministic
//! for a given seed and independent of tiling or thread count (ADR-0007).
//! Export writes the formats engines actually consume (`docs/architecture/08-io-formats.md`).

pub mod export;
pub mod flow;
pub mod heightfield;
pub mod import;
pub mod mesh;
pub mod noise;
pub mod preview;
pub mod sculpt;
pub mod sim;

pub use export::{HeightmapFormat, export_heightmap};
pub use flow::{d8_receivers, fill_depressions, flow_accumulation};
pub use heightfield::{Heightfield, HeightfieldStats, Tile, TileCoord};
pub use import::{ForeignImport, read_foreign, resample_to_world};
pub use mesh::{MeshExportReport, auto_step, export_glb};
pub use noise::{FractalKind, NoiseKind, NoiseParams, generate_noise};
pub use sculpt::{Stroke, StrokeMode, apply_strokes};
pub use sim::{
    HydraulicParams, HydraulicResult, THERMAL_MAX_RATE, hydraulic_erode, slope_mask, thermal_erode,
};
