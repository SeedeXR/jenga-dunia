//! On-disk layer cache (`docs/architecture/06-jobs.md`, "Cache").
//!
//! Layers produced by `terrain.generate` / `graph.evaluate` are stored under
//! `<project>/cache/layers/` as content-addressed blobs (`<blake3>.jdl`: magic, header, f32 LE
//! rows) with a small `index.json` mapping layer name → hash. This is what lets one CLI
//! invocation evaluate a graph and the next one export the result, and what lets the desktop
//! app reopen a project with its layers warm. The cache is disposable: deleting `cache/` never
//! breaks a project (ADR-0011).
//!
//! Eviction: with a byte limit (`settings.cache_limit_bytes`, applied by the core) every `put`
//! runs [`LayerCache::gc`], which drops blobs oldest-first by modification time — unreferenced
//! blobs before indexed ones — until the store fits; `get` touches a blob so recently read layers
//! survive. The newest blob is never evicted, so a single oversized layer still round-trips.
//!
//! ponytail: whole-layer blobs, JSON index, mtime as the LRU clock — tile-level blobs and a
//! SQLite index (06-jobs.md) when layers stop fitting in RAM.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use jd_terrain::Heightfield;
use jd_types::{ErrorCode, JdError, JdResult};

const MAGIC: &[u8; 4] = b"JDL1";
const HEADER: usize = 4 + 4 * 4; // magic + width, height, tile_size, halo (u32 LE)

/// A project's layer store.
#[derive(Debug, Clone)]
pub struct LayerCache {
    dir: PathBuf,
    limit_bytes: Option<u64>,
}

/// What [`LayerCache::gc`] found and did.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct GcReport {
    /// Blobs kept.
    pub blobs: u64,
    /// Bytes kept.
    pub bytes: u64,
    /// Blobs removed.
    pub evicted_blobs: u64,
    /// Bytes removed.
    pub evicted_bytes: u64,
    /// The limit applied (None = report only).
    pub limit_bytes: Option<u64>,
}

impl LayerCache {
    /// The store of the project rooted at `project_root` (nothing is created until `put`).
    pub fn open(project_root: &Path) -> Self {
        Self {
            dir: project_root.join("cache").join("layers"),
            limit_bytes: None,
        }
    }

    /// Evict oldest blobs after every `put` so the store stays within `bytes`.
    pub fn with_limit(mut self, bytes: u64) -> Self {
        self.limit_bytes = Some(bytes);
        self
    }

    /// Evict blobs (oldest modification time first; unreferenced before indexed) until the store
    /// fits the limit, dropping index entries of evicted blobs. Without a limit, only reports.
    pub fn gc(&self) -> JdResult<GcReport> {
        let mut blobs: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        match fs::read_dir(&self.dir) {
            Ok(rd) => {
                for entry in rd {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("jdl") {
                        continue;
                    }
                    let meta = entry.metadata()?;
                    blobs.push((
                        path,
                        meta.len(),
                        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    ));
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut report = GcReport {
            blobs: blobs.len() as u64,
            bytes: blobs.iter().map(|b| b.1).sum(),
            limit_bytes: self.limit_bytes,
            ..GcReport::default()
        };
        let Some(limit) = self.limit_bytes else {
            return Ok(report);
        };
        if report.bytes <= limit {
            return Ok(report);
        }
        let mut index = self.read_index()?;
        let referenced: std::collections::BTreeSet<String> = index.values().cloned().collect();
        let newest = blobs.iter().map(|b| b.2).max();
        let hash_of = |p: &Path| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned()
        };
        // Unreferenced first, then by age; the newest blob is never a candidate.
        blobs.sort_by_key(|b| (referenced.contains(&hash_of(&b.0)), b.2));
        for (path, len, modified) in blobs {
            if report.bytes <= limit {
                break;
            }
            if Some(modified) == newest {
                continue;
            }
            if let Err(e) = fs::remove_file(&path)
                && path.exists()
            {
                return Err(e.into());
            }
            let hash = hash_of(&path);
            index.retain(|_, h| *h != hash);
            report.bytes -= len;
            report.blobs -= 1;
            report.evicted_bytes += len;
            report.evicted_blobs += 1;
        }
        if report.evicted_blobs > 0 {
            let index_bytes = serde_json::to_vec_pretty(&index)?;
            write_atomic(&self.index_path(), |out| Ok(out.write_all(&index_bytes)?))?;
        }
        Ok(report)
    }

    /// Store `hf` under `name`; returns its content hash (hex). Identical fields share one blob.
    pub fn put(&self, name: &str, hf: &Heightfield) -> JdResult<String> {
        fs::create_dir_all(&self.dir)?;
        let hash = hf.content_hash().to_hex().to_string();
        let blob = self.blob_path(&hash);
        if !blob.exists() {
            // Streamed row by row: a 16k² layer is 1 GiB and must not be copied to write it.
            write_atomic(&blob, |out| {
                out.write_all(MAGIC)?;
                for v in [hf.width(), hf.height(), hf.tile_size(), hf.halo()] {
                    out.write_all(&v.to_le_bytes())?;
                }
                let mut row = vec![0.0f32; hf.width() as usize];
                for y in 0..hf.height() {
                    hf.row(y, &mut row);
                    for v in &row {
                        out.write_all(&v.to_le_bytes())?;
                    }
                }
                Ok(())
            })?;
        }
        let mut index = self.read_index()?;
        index.insert(name.to_owned(), hash.clone());
        let index_bytes = serde_json::to_vec_pretty(&index)?;
        write_atomic(&self.index_path(), |out| Ok(out.write_all(&index_bytes)?))?;
        if self.limit_bytes.is_some() {
            self.gc()?;
        }
        Ok(hash)
    }

    /// Load the layer stored under `name`, if any (a stale index entry counts as absent).
    pub fn get(&self, name: &str) -> JdResult<Option<Heightfield>> {
        let Some(hash) = self.read_index()?.remove(name) else {
            return Ok(None);
        };
        let blob = self.blob_path(&hash);
        let file = match fs::File::open(&blob) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        // LRU clock: a read counts as use (best effort — a read-only cache still works).
        if let Ok(f) = fs::File::options().write(true).open(&blob) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        let corrupt = || {
            JdError::new(ErrorCode::E_INTERNAL, format!("cache blob for layer `{name}` is corrupt"))
                .next("Delete the project's cache/ directory — it is disposable — and re-run the operation.")
                .detail("layer", name)
        };
        let len = file.metadata()?.len();
        let mut input = std::io::BufReader::with_capacity(1 << 20, file);
        let mut head = [0u8; HEADER];
        if input.read_exact(&mut head).is_err() || &head[..4] != MAGIC {
            return Err(corrupt());
        }
        let u32_at =
            |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let (w, h, tile, halo) = (u32_at(4), u32_at(8), u32_at(12), u32_at(16));
        if len != HEADER as u64 + 4 * w as u64 * h as u64 {
            return Err(corrupt());
        }
        let mut hf = Heightfield::new(w, h, tile, halo).map_err(|_| corrupt())?;
        // Streamed row by row into the tiles — no whole-field byte buffer.
        let mut bytes = vec![0u8; 4 * w as usize];
        let mut row = vec![0.0f32; w as usize];
        for y in 0..h {
            input.read_exact(&mut bytes).map_err(|_| corrupt())?;
            for (v, b) in row.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *v = f32::from_le_bytes(*b);
            }
            hf.set_row(y, &row);
        }
        hf.rebuild_halos();
        Ok(Some(hf))
    }

    /// The preview of a stored layer straight from its blob — bit-identical to
    /// `jd_terrain::preview::preview_u16` on the loaded field (full-field min/max, every
    /// `2^lod`-th sample), but reading rows instead of holding the layer: a 16k² layer is 1 GiB
    /// in memory and a few hundred kilobytes on the way to the viewport.
    pub fn preview(&self, name: &str, lod: u8) -> JdResult<Option<jd_terrain::preview::Preview>> {
        let Some(hash) = self.read_index()?.remove(name) else {
            return Ok(None);
        };
        let file = match fs::File::open(self.blob_path(&hash)) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let corrupt = || {
            JdError::new(ErrorCode::E_INTERNAL, format!("cache blob for layer `{name}` is corrupt"))
                .next("Delete the project's cache/ directory — it is disposable — and re-run the operation.")
                .detail("layer", name)
        };
        let len = file.metadata()?.len();
        let mut input = std::io::BufReader::with_capacity(1 << 20, file);
        let mut head = [0u8; HEADER];
        if input.read_exact(&mut head).is_err() || &head[..4] != MAGIC {
            return Err(corrupt());
        }
        let u32_at =
            |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let (w, h) = (u32_at(4), u32_at(8));
        if len != HEADER as u64 + 4 * w as u64 * h as u64 || w == 0 || h == 0 {
            return Err(corrupt());
        }
        let step = 1u32 << lod.min(12);
        let (pw, ph) = (w.div_ceil(step).max(1), h.div_ceil(step).max(1));
        // One sequential pass: exact min / max over every sample, the strided rows kept as f32.
        let (mut min, mut max) = (f32::MAX, f32::MIN);
        let mut rows: Vec<f32> = Vec::with_capacity((pw * ph) as usize);
        let mut bytes = vec![0u8; 4 * w as usize];
        for y in 0..h {
            input.read_exact(&mut bytes).map_err(|_| corrupt())?;
            let vals = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b));
            if y.is_multiple_of(step) {
                for (x, v) in vals.enumerate() {
                    min = min.min(v);
                    max = max.max(v);
                    if (x as u32).is_multiple_of(step) {
                        rows.push(v);
                    }
                }
            } else {
                for v in vals {
                    min = min.min(v);
                    max = max.max(v);
                }
            }
        }
        let (min, max) = if max > min {
            (min, max)
        } else {
            (min, min + 1.0)
        };
        let scale = 65535.0 / (max - min);
        let data = rows
            .iter()
            .map(|v| ((v - min) * scale).round().clamp(0.0, 65535.0) as u16)
            .collect();
        Ok(Some(jd_terrain::preview::Preview {
            width: pw,
            height: ph,
            min,
            max,
            data,
        }))
    }

    fn blob_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.jdl"))
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    /// Every name in the index (layers and evaluator node ports), sorted.
    pub fn names(&self) -> JdResult<Vec<String>> {
        Ok(self.read_index()?.into_keys().collect())
    }

    fn read_index(&self) -> JdResult<BTreeMap<String, String>> {
        match fs::read(self.index_path()) {
            // A garbled index is a cache miss, not an error: the cache is disposable and the next
            // `put` rewrites it (existing blobs are simply forgotten).
            Ok(b) => Ok(serde_json::from_slice(&b).unwrap_or_default()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Write via a uniquely named temporary file and rename, so an interrupted write never leaves a
/// torn blob/index and two writers (two processes, or two evaluations) never share a temp file.
/// Write `path` through a temp file next to it, fsync, then rename into place — the one
/// recipe for blobs (streamed) and the index (one `write_all`).
fn write_atomic(
    path: &Path,
    body: impl FnOnce(&mut std::io::BufWriter<fs::File>) -> JdResult<()>,
) -> JdResult<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{seq}.tmp", std::process::id()));
    let mut out = std::io::BufWriter::with_capacity(1 << 20, fs::File::create(&tmp)?);
    body(&mut out)?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_from_the_blob_matches_the_in_memory_preview_bit_for_bit() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = LayerCache::open(dir.path());
        let hf = field(0.3);
        cache.put("height", &hf).expect("put");
        for lod in [0u8, 1, 3] {
            let streamed = cache
                .preview("height", lod)
                .expect("preview")
                .expect("some");
            let loaded = jd_terrain::preview::preview_u16(&hf, lod);
            assert_eq!(streamed, loaded, "lod {lod}");
        }
        assert!(cache.preview("missing", 1).expect("none").is_none());
    }

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
    fn limit_evicts_oldest_blobs_first_and_reads_refresh_them() {
        let dir = tempfile::tempdir().expect("tmp");
        let blob_bytes = (HEADER + 4 * 64 * 64) as u64; // one 64² layer
        let cache = LayerCache::open(dir.path()).with_limit(blob_bytes * 3 + 10);
        let age = |hash: &str, secs: u64| {
            let f = fs::File::options()
                .write(true)
                .open(cache.blob_path(hash))
                .expect("open");
            f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
                .expect("mtime");
        };
        let h_a = cache.put("a", &field(1.0)).expect("put");
        age(&h_a, 100);
        let h_b = cache.put("b", &field(2.0)).expect("put");
        age(&h_b, 200);
        let h_c = cache.put("c", &field(3.0)).expect("put");
        age(&h_c, 300);
        // Reading `a` makes it the most recently used.
        assert!(cache.get("a").expect("get").is_some());
        // A fourth layer pushes the store over 3 blobs: `b` (oldest untouched) goes, `a` stays.
        let _h_d = cache.put("d", &field(4.0)).expect("put");
        assert!(cache.get("b").expect("get").is_none(), "b evicted");
        assert!(
            cache.get("a").expect("get").is_some(),
            "a refreshed by the read"
        );
        assert!(cache.get("c").expect("get").is_some());
        assert!(cache.get("d").expect("get").is_some());
        let r = cache.gc().expect("gc");
        assert_eq!((r.blobs, r.evicted_blobs), (3, 0));
        assert!(r.bytes <= blob_bytes * 3 + 10);
        // Unreferenced blobs go before indexed ones; an oversized single layer is still kept.
        let tiny = LayerCache::open(dir.path()).with_limit(1);
        let r = tiny.gc().expect("gc");
        assert_eq!(
            r.blobs, 1,
            "the newest blob survives a limit it cannot meet"
        );
        assert!(tiny.get("d").expect("get").is_some());
        assert_eq!(
            LayerCache::open(dir.path()).gc().expect("gc").limit_bytes,
            None
        );
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
        // A garbled index reads as empty and heals on the next put.
        fs::write(dir.path().join("cache/layers/index.json"), b"{ not json").expect("write");
        assert!(cache.get("b").expect("miss").is_none());
        cache.put("c", &field(3.0)).expect("put heals");
        assert!(cache.get("c").expect("get").is_some());
        // A blob with an impossible header is corrupt, not an input error.
        let hash = cache.put("d", &field(4.0)).expect("put");
        let blob = dir.path().join("cache/layers").join(format!("{hash}.jdl"));
        let mut bad = fs::read(&blob).expect("read");
        bad[12..16].copy_from_slice(&7u32.to_le_bytes()); // tile_size 7: not a power of two
        fs::write(&blob, &bad).expect("write");
        assert_eq!(
            cache.get("d").expect_err("corrupt header").code,
            ErrorCode::E_INTERNAL
        );
    }
}
