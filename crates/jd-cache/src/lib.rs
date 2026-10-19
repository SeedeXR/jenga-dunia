//! On-disk layer cache (`docs/architecture/06-jobs.md`, "Cache").
//!
//! Layers produced by `terrain.generate` / `graph.evaluate` are stored under
//! `<project>/cache/layers/` as content-addressed blobs (`<blake3>.jdl`: magic, header, f32 LE
//! rows) with a small `index.json` mapping layer name → hash. This is what lets one CLI
//! invocation evaluate a graph and the next one export the result, and what lets the desktop
//! app reopen a project with its layers warm. The cache is disposable: deleting `cache/` never
//! breaks a project (ADR-0011).
//!
//! ponytail: whole-layer blobs, JSON index, no eviction — tile-level blobs, SQLite index and
//! LRU (06-jobs.md) when layers stop fitting in RAM or the cap in `settings.cache_limit_bytes`
//! starts to matter.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use jd_terrain::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult};

const MAGIC: &[u8; 4] = b"JDL1";
const HEADER: usize = 4 + 4 * 4; // magic + width, height, tile_size, halo (u32 LE)

/// A project's layer store.
#[derive(Debug, Clone)]
pub struct LayerCache {
    dir: PathBuf,
}

impl LayerCache {
    /// The store of the project rooted at `project_root` (nothing is created until `put`).
    pub fn open(project_root: &Path) -> Self {
        Self {
            dir: project_root.join("cache").join("layers"),
        }
    }

    /// Store `hf` under `name`; returns its content hash (hex). Identical fields share one blob.
    pub fn put(&self, name: &str, hf: &Heightfield) -> JdResult<String> {
        fs::create_dir_all(&self.dir)?;
        let hash = hf.content_hash().to_hex().to_string();
        let blob = self.blob_path(&hash);
        if !blob.exists() {
            let mut bytes = Vec::with_capacity(HEADER + 4 * (hf.width() * hf.height()) as usize);
            bytes.extend_from_slice(MAGIC);
            for v in [hf.width(), hf.height(), hf.tile_size(), hf.halo()] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            for v in hf.to_vec() {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            write_atomic(&blob, &bytes)?;
        }
        let mut index = self.read_index()?;
        index.insert(name.to_owned(), hash.clone());
        write_atomic(&self.index_path(), &serde_json::to_vec_pretty(&index)?)?;
        Ok(hash)
    }

    /// Load the layer stored under `name`, if any (a stale index entry counts as absent).
    pub fn get(&self, name: &str) -> JdResult<Option<Heightfield>> {
        let Some(hash) = self.read_index()?.remove(name) else {
            return Ok(None);
        };
        let bytes = match fs::read(self.blob_path(&hash)) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let corrupt = || {
            JdError::new(ErrorCode::E_INTERNAL, format!("cache blob for layer `{name}` is corrupt"))
                .next("Delete the project's cache/ directory — it is disposable — and re-run the operation.")
                .detail("layer", name)
        };
        if bytes.len() < HEADER || &bytes[..4] != MAGIC {
            return Err(corrupt());
        }
        let u32_at =
            |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        let (w, h, tile, halo) = (u32_at(4), u32_at(8), u32_at(12), u32_at(16));
        let (rows, rest) = bytes[HEADER..].as_chunks::<4>();
        if !rest.is_empty() || rows.len() != (w as usize) * (h as usize) {
            return Err(corrupt());
        }
        let mut hf = Heightfield::new(w, h, tile, halo)?;
        hf.fill_with(|x, y| f32::from_le_bytes(rows[(y * w + x) as usize]));
        Ok(Some(hf))
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.jdl"))
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn read_index(&self) -> JdResult<BTreeMap<String, String>> {
        match fs::read(self.index_path()) {
            Ok(b) => Ok(serde_json::from_slice(&b)?),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Write via a temporary file and rename so an interrupted write never leaves a torn blob/index.
fn write_atomic(path: &Path, bytes: &[u8]) -> JdResult<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(seed: f32) -> Heightfield {
        let mut hf = Heightfield::new(70, 50, 32, 4).expect("hf");
        hf.fill_with(|x, y| (x as f32 * 0.5 + y as f32 * 1.25 + seed).sin() * 100.0);
        hf
    }

    #[test]
    fn round_trip_preserves_content_and_tiling() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = LayerCache::open(dir.path());
        assert_eq!(cache.get("height").expect("get"), None, "empty cache");
        let hf = field(1.0);
        let hash = cache.put("height", &hf).expect("put");
        let back = cache.get("height").expect("get").expect("present");
        assert_eq!(back.content_hash().to_hex().to_string(), hash);
        assert_eq!(
            (back.width(), back.height(), back.tile_size(), back.halo()),
            (70, 50, 32, 4)
        );
        assert_eq!(back.get(69, 49), hf.get(69, 49));
    }

    #[test]
    fn identical_fields_share_one_blob_and_stale_index_reads_as_absent() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = LayerCache::open(dir.path());
        let a = cache.put("a", &field(2.0)).expect("put");
        let b = cache.put("b", &field(2.0)).expect("put");
        assert_eq!(a, b);
        let blobs = fs::read_dir(dir.path().join("cache/layers"))
            .expect("dir")
            .filter(|e| {
                e.as_ref()
                    .map(|e| e.path().extension().is_some_and(|x| x == "jdl"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(blobs, 1);
        fs::remove_file(dir.path().join("cache/layers").join(format!("{a}.jdl"))).expect("rm");
        assert!(
            cache.get("a").expect("get").is_none(),
            "missing blob is just a miss"
        );
        fs::write(
            dir.path().join("cache/layers").join(format!("{a}.jdl")),
            b"JDL1garbage",
        )
        .expect("write");
        assert_eq!(
            cache.get("a").expect_err("corrupt").code,
            ErrorCode::E_INTERNAL
        );
    }
}
