//! `fbx.sdk_check`: the Autodesk FBX SDK's own view of an FBX file (ADR-0015) — what a DCC or
//! engine built on the SDK will see of our export. A build without the SDK answers
//! `E_DEPENDENCY_MISSING`, so every client can tell the two apart.

use std::path::PathBuf;

use jd_types::JdResult;
use serde::{Deserialize, Serialize};

use crate::ops::{Registry, Scope};

/// `fbx.sdk_check` params.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SdkCheck {
    /// The FBX file (absolute, or relative to the project's `exports/`).
    pub path: PathBuf,
}

/// `fbx.sdk_check` result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SdkReport {
    /// The SDK's version string.
    pub sdk: String,
    /// The file's version as the SDK reports it (7400, 7500, …).
    pub file_version: i32,
    /// Meshes in the scene; the rest describes the first.
    pub meshes: u32,
    /// Control points.
    pub vertices: u32,
    /// Polygons before the SDK's triangulation.
    pub polygons: u32,
    /// Triangles after it.
    pub triangles: u32,
    /// Bounds of the control points in file units, `[min, max]`.
    pub bounds: [[f64; 3]; 2],
    /// Centimetres per file unit (1 = cm, 100 = m).
    pub unit_scale: f64,
    /// `Y` or `Z`.
    pub up_axis: String,
    /// A normal layer element exists.
    pub has_normals: bool,
    /// A UV layer element exists.
    pub has_uvs: bool,
    /// First material's name.
    pub material: Option<String>,
    /// First diffuse texture's relative file name.
    pub texture: Option<String>,
}

/// Register `fbx.sdk_check`.
pub fn register(r: &mut Registry) {
    r.register::<SdkCheck, SdkReport, _>(
        "fbx.sdk_check",
        "Read an FBX file through the Autodesk FBX SDK (when this build carries it) and report what the SDK sees: version, meshes, control points, polygons and triangles, bounds, units, up axis, normals, UVs, material and texture — the reference view of our own FBX writer",
        Scope::ProjectRead,
        |ctx, p| {
            let path = if p.path.is_absolute() { p.path.clone() } else { ctx.with_project(|proj| Ok(proj.root().join("exports").join(&p.path)))? };
            let m = jd_fbxsdk::read(&path)?;
            let (lo, hi) = m.bounds();
            let report: JdResult<SdkReport> = Ok(SdkReport {
                sdk: jd_fbxsdk::version().unwrap_or_default(),
                file_version: m.file_version,
                meshes: m.meshes,
                vertices: m.positions.len() as u32,
                polygons: m.polygons,
                triangles: m.triangles.len() as u32,
                bounds: [lo, hi],
                unit_scale: m.unit_scale,
                up_axis: if m.up_axis == 2 { "Z".into() } else { "Y".into() },
                has_normals: m.has_normals,
                has_uvs: m.has_uvs,
                material: m.material,
                texture: m.texture,
            });
            Ok((report?, Vec::new()))
        },
    );
}
