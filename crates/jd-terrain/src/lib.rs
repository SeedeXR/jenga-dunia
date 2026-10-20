//! Terrain data and generators.
//!
//! [`Heightfield`] is an f32 raster stored as tiles with halos (ADR-0005) so
//! every kernel runs per tile with bounded memory. Generators are deterministic
//! for a given seed and independent of tiling or thread count (ADR-0007).
//! Export writes the formats engines actually consume (`docs/architecture/08-io-formats.md`).

pub mod aeolian;
pub mod bathymetry;
pub mod climate;
pub mod coastal;
pub mod decimate;
pub mod droplet;
pub mod export;
pub mod fbx;
pub mod flow;
pub mod heightfield;
pub mod import;
pub mod karst;
pub mod mesh;
pub mod noise;
pub mod ocean;
pub mod preview;
pub mod profiles;
pub mod rocks;
pub mod scatter;
pub mod sculpt;
pub mod seabed;
pub mod sim;
pub mod sky;
pub mod spline;
pub mod strata;
pub mod yardang;

pub use aeolian::{AeolianParams, aeolian_transport};
pub use bathymetry::{BathymetryParams, bathymetry};
pub use climate::{SnowParams, snow_cover};
pub use coastal::{CoastalParams, coastal_erode};
pub use decimate::decimate;
pub use droplet::{DropletParams, DropletResult, droplet_erode};
pub use export::{
    ExportOptions, GeoRef, HeightmapFormat, epsg_code, export_heightmap, export_heightmap_geo,
};
pub use flow::{
    FlowMethod, carve_rivers, d8_receivers, fill_depressions, flow_accumulation,
    flow_accumulation_with, sea_level, wetness_index, wetness_index_with,
};
pub use heightfield::{Heightfield, HeightfieldStats, Tile, TileCoord};
pub use import::{ForeignImport, GeoTags, geotiff_tags, read_foreign, resample_to_world};
pub use karst::{KarstParams, Sinkhole, SinkholeProfile, karst_sinkholes};
pub use mesh::{
    BakedMaps, GridMesh, MeshExportReport, auto_step, bake_maps, export_glb, export_mesh,
    export_mesh_lods, export_obj, export_ply, export_stl, export_usda, export_usdz,
};
pub use noise::{FractalKind, NoiseKind, NoiseParams, generate_noise};
pub use ocean::{
    WaterLook, WaterMaps, WaveComponent, WaveParams, WaveSpectrum, bake_water, bake_wave_normal,
    water_colour, wave_spectrum,
};
pub use profiles::{EngineProfile, ProfileReport, ProfileSettings, export_profile};
pub use rocks::{RockParams, rock_mesh};
pub use scatter::{
    ScatterParams, ScatterPoint, exclude_near, read_points_csv, scatter_points, write_points,
    write_points_with,
};
pub use sculpt::{Stroke, StrokeMode, apply_strokes};
pub use seabed::{SeabedParams, classify_seabed};
pub use sim::{
    HydraulicParams, HydraulicResult, StreamPowerParams, THERMAL_MAX_RATE, hydraulic_erode,
    hydraulic_erode_with, slope_mask, stream_power_erode, stream_power_erode_with, thermal_erode,
    thermal_erode_with,
};
pub use sky::{Sky, SkyParams, bake_sky_hdri, sky_preview_png};
pub use spline::{SplineMode, SplineParams, apply_spline};
pub use strata::{StrataParams, strata_hardness};
pub use yardang::{YardangParams, wind_abrasion};
