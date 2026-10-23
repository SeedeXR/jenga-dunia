//! The Autodesk FBX SDK, in process, as a test oracle (ADR-0015): read what our writer wrote
//! and write files our reader must read. The SDK is found on the machine at build time
//! (`build.rs`: `FBXSDK_DIR` or the default install) and is never copied or shipped; without it
//! every call returns `E_DEPENDENCY_MISSING` and [`available`] is false. Binaries built with the
//! SDK present contain Autodesk code and carry the SDK licence's notice — see the ADR.

use std::path::Path;

use jd_types::{JdError, JdResult};

/// What the SDK read from a file (the first mesh of the scene).
#[derive(Debug, Clone, PartialEq)]
pub struct SdkMesh {
    /// Control points in the file's units.
    pub positions: Vec<[f64; 3]>,
    /// Triangles after the SDK's own triangulation.
    pub triangles: Vec<[u32; 3]>,
    /// Polygons before triangulation.
    pub polygons: u32,
    /// Meshes in the scene.
    pub meshes: u32,
    /// A normal layer element exists.
    pub has_normals: bool,
    /// A UV layer element exists.
    pub has_uvs: bool,
    /// File version as the SDK reports it (7400, 7500, …).
    pub file_version: i32,
    /// Centimetres per file unit (`FbxSystemUnit::GetScaleFactor`): 1 = cm, 100 = m.
    pub unit_scale: f64,
    /// 1 = Y up, 2 = Z up.
    pub up_axis: i32,
    /// First material's name.
    pub material: Option<String>,
    /// First diffuse file texture's relative name.
    pub texture: Option<String>,
}

impl SdkMesh {
    /// Axis-aligned bounds of the control points.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in &self.positions {
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        (lo, hi)
    }
}

/// Whether this build carries the SDK.
pub fn available() -> bool {
    cfg!(fbxsdk)
}

#[cfg(not(fbxsdk))]
fn missing() -> JdError {
    JdError::new(jd_types::ErrorCode::E_DEPENDENCY_MISSING, "this build has no Autodesk FBX SDK")
        .next("Install the FBX SDK (2020.3.x) and rebuild with FBXSDK_DIR pointing at it; the SDK is never bundled.")
}

#[cfg(fbxsdk)]
mod ffi {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::path::Path;

    use jd_types::{ErrorCode, JdError, JdResult};

    use super::SdkMesh;

    #[repr(C)]
    struct Mesh {
        positions: *mut f64,
        n_positions: u32,
        triangles: *mut u32,
        n_triangles: u32,
        n_polygons: u32,
        n_meshes: u32,
        has_normals: i32,
        has_uvs: i32,
        file_version: i32,
        unit_scale: f64,
        up_axis: i32,
        material: *mut c_char,
        texture: *mut c_char,
    }

    unsafe extern "C" {
        fn jd_fbxsdk_version() -> *const c_char;
        fn jd_fbxsdk_read(path: *const c_char, out: *mut *mut Mesh, err: *mut *mut c_char)
        -> c_int;
        fn jd_fbxsdk_write(
            path: *const c_char,
            positions: *const f64,
            n_positions: u32,
            triangles: *const u32,
            n_triangles: u32,
            version: c_int,
            err: *mut *mut c_char,
        ) -> c_int;
        fn jd_fbxsdk_free(m: *mut Mesh);
        fn jd_fbxsdk_free_str(s: *mut c_char);
    }

    fn take_err(err: *mut c_char, what: &str, rc: c_int) -> JdError {
        let msg = if err.is_null() {
            format!("{what}: rc {rc}")
        } else {
            // SAFETY: the shim hands back a NUL-terminated malloc'd string we free right after.
            let s = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { jd_fbxsdk_free_str(err) };
            format!("{what}: {s}")
        };
        JdError::new(ErrorCode::E_FORMAT_CORRUPT, msg)
    }

    fn c_path(path: &Path) -> JdResult<CString> {
        CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| JdError::invalid("path", "contains a NUL byte"))
    }

    pub fn version() -> String {
        // SAFETY: a static string owned by the SDK.
        unsafe { CStr::from_ptr(jd_fbxsdk_version()) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn read(path: &Path) -> JdResult<SdkMesh> {
        let c = c_path(path)?;
        let mut out: *mut Mesh = std::ptr::null_mut();
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: the shim writes `out` on success and `err` on failure; both are freed here.
        let rc = unsafe { jd_fbxsdk_read(c.as_ptr(), &mut out, &mut err) };
        if rc != 0 || out.is_null() {
            return Err(take_err(err, "FBX SDK read", rc));
        }
        // SAFETY: `out` is a valid Mesh from the shim until jd_fbxsdk_free.
        let m = unsafe { &*out };
        let positions = (0..m.n_positions as usize)
            .map(|i| unsafe {
                [
                    *m.positions.add(i * 3),
                    *m.positions.add(i * 3 + 1),
                    *m.positions.add(i * 3 + 2),
                ]
            })
            .collect();
        let triangles = (0..m.n_triangles as usize)
            .map(|i| unsafe {
                [
                    *m.triangles.add(i * 3),
                    *m.triangles.add(i * 3 + 1),
                    *m.triangles.add(i * 3 + 2),
                ]
            })
            .collect();
        let str_of = |p: *mut c_char| {
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        };
        let mesh = SdkMesh {
            positions,
            triangles,
            polygons: m.n_polygons,
            meshes: m.n_meshes,
            has_normals: m.has_normals != 0,
            has_uvs: m.has_uvs != 0,
            file_version: m.file_version,
            unit_scale: m.unit_scale,
            up_axis: m.up_axis,
            material: str_of(m.material),
            texture: str_of(m.texture),
        };
        unsafe { jd_fbxsdk_free(out) };
        Ok(mesh)
    }

    pub fn write(
        path: &Path,
        positions: &[[f64; 3]],
        triangles: &[[u32; 3]],
        version: u16,
    ) -> JdResult<()> {
        let c = c_path(path)?;
        let flat_p: Vec<f64> = positions.iter().flatten().copied().collect();
        let flat_t: Vec<u32> = triangles.iter().flatten().copied().collect();
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: the slices outlive the call; the shim copies what it needs.
        let rc = unsafe {
            jd_fbxsdk_write(
                c.as_ptr(),
                flat_p.as_ptr(),
                positions.len() as u32,
                flat_t.as_ptr(),
                triangles.len() as u32,
                i32::from(version),
                &mut err,
            )
        };
        if rc != 0 {
            return Err(take_err(err, "FBX SDK write", rc));
        }
        Ok(())
    }
}

/// The SDK's version string, when built in.
pub fn version() -> Option<String> {
    #[cfg(fbxsdk)]
    {
        Some(ffi::version())
    }
    #[cfg(not(fbxsdk))]
    {
        None
    }
}

/// Read the first mesh of `path` through the SDK (triangulated by the SDK).
pub fn read(path: &Path) -> JdResult<SdkMesh> {
    #[cfg(fbxsdk)]
    {
        if !path.is_file() {
            return Err(JdError::invalid(
                "path",
                format!("{} is not a file", path.display()),
            ));
        }
        ffi::read(path)
    }
    #[cfg(not(fbxsdk))]
    {
        let _ = path;
        Err(missing())
    }
}

/// Write a triangle mesh (metres, Y up) as a binary FBX of `version` (7400 or 7500) through the
/// SDK, in centimetres with the SDK's compressed arrays.
pub fn write(
    path: &Path,
    positions: &[[f64; 3]],
    triangles: &[[u32; 3]],
    version: u16,
) -> JdResult<()> {
    if !matches!(version, 7400 | 7500) {
        return Err(JdError::invalid("version", "7400 or 7500"));
    }
    if triangles
        .iter()
        .flatten()
        .any(|&i| i as usize >= positions.len())
    {
        return Err(JdError::invalid("triangles", "an index is out of range"));
    }
    #[cfg(fbxsdk)]
    {
        ffi::write(path, positions, triangles, version)
    }
    #[cfg(not(fbxsdk))]
    {
        let _ = (path, positions, triangles);
        Err(missing())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jd_types::ErrorCode;

    #[test]
    fn a_square_round_trips_through_the_sdk_or_the_stub_says_so() {
        let dir = std::env::temp_dir().join(format!("jd-fbxsdk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("square.fbx");
        let positions = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 2.5, 10.0],
            [0.0, 0.0, 10.0],
        ];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        assert!(
            write(&path, &positions, &[[0, 1, 9]], 7500).is_err(),
            "index out of range"
        );
        assert!(
            write(&path, &positions, &triangles, 7300).is_err(),
            "unsupported version"
        );
        let r = write(&path, &positions, &triangles, 7500);
        if !available() {
            assert_eq!(r.unwrap_err().code, ErrorCode::E_DEPENDENCY_MISSING);
            assert_eq!(
                read(&path).unwrap_err().code,
                ErrorCode::E_DEPENDENCY_MISSING
            );
            assert!(version().is_none());
            eprintln!("no FBX SDK: skipping");
            return;
        }
        r.expect("write");
        assert!(version().expect("version").starts_with("2020."));
        let m = read(&path).expect("read");
        assert_eq!(
            (m.file_version, m.meshes, m.polygons, m.triangles.len()),
            (7500, 1, 2, 2)
        );
        assert_eq!(m.unit_scale, 1.0, "written in centimetres");
        assert_eq!(m.up_axis, 1);
        let (lo, hi) = m.bounds();
        assert_eq!(
            (lo, hi),
            ([0.0, 0.0, 0.0], [1000.0, 250.0, 1000.0]),
            "metres × 100"
        );
        assert!(m.has_normals);
        assert!(read(Path::new("/nonexistent.fbx")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
