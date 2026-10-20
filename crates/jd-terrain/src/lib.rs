//! Terrain data and generators.
//!
//! [`Heightfield`] is an f32 raster stored as tiles with halos (ADR-0005) so
//! every kernel runs per tile with bounded memory. Generators are deterministic
//! for a given seed and independent of tiling or thread count (ADR-0007).
//! Export writes the formats engines actually consume (`docs/architecture/08-io-formats.md`).

pub mod aeolian;
pub mod climate;
pub mod coastal;
pub mod droplet;
pub mod export;
pub mod fbx;
pub mod flow;
pub mod heightfield;
pub mod import;
pub mod mesh;
pub mod noise;
pub mod preview;
pub mod profiles;
pub mod scatter;
pub mod sculpt;
pub mod sim;

pub use aeolian::{AeolianParams, aeolian_transport};
pub use climate::{SnowParams, snow_cover};
pub use coastal::{CoastalParams, coastal_erode};
pub use droplet::{DropletParams, DropletResult, droplet_erode};
pub use export::{
    ExportOptions, GeoRef, HeightmapFormat, epsg_code, export_heightmap, export_heightmap_geo,
};
pub use flow::{
    FlowMethod, carve_rivers, d8_receivers, fill_depressions, flow_accumulation,
    flow_accumulation_with, sea_level, wetness_index,
};
pub use heightfield::{Heightfield, HeightfieldStats, Tile, TileCoord};
pub use import::{ForeignImport, GeoTags, geotiff_tags, read_foreign, resample_to_world};
pub use mesh::{
    BakedMaps, GridMesh, MeshExportReport, auto_step, bake_maps, export_glb, export_mesh,
    export_obj, export_ply, export_stl, export_usda, export_usdz,
};
pub use noise::{FractalKind, NoiseKind, NoiseParams, generate_noise};
pub use profiles::{EngineProfile, ProfileReport, ProfileSettings, export_profile};
pub use scatter::{ScatterParams, ScatterPoint, scatter_points, write_points};
pub use sculpt::{Stroke, StrokeMode, apply_strokes};
pub use sim::{
    HydraulicParams, HydraulicResult, StreamPowerParams, THERMAL_MAX_RATE, hydraulic_erode,
    slope_mask, stream_power_erode, thermal_erode,
};
