# ADR-0014 — Geo data over the network: one client, allowlisted hosts, adaptive and cached

**Status:** Accepted · 2026-10-23 (owner decision: "design a robust internet-efficient policy that adapts to internet speed")

## Context

`jd-geo` had CRS transforms and GeoTIFF placement but no way to fetch data: no HTTP client was compiled into any binary, `Scope::NetworkGeo` existed with no op behind it, and `project.json → attribution[]` was never written or exported. The research (`docs/research/geo-vision.md` A1–A5, binding rules in `docs/architecture/13-geo-vision.md`) fixed *which* sources may be used and what they demand; what was missing was *how* the product talks to them: client, hosts, concurrency, retries, caching, offline behaviour, keys, and the attribution trail.

## Decision

1. **One client.** `jd_geo::fetch::UreqHttp` — `ureq` 3 (blocking) with rustls and the webpki roots, gzip, redirects capped at 4, HTTP status never an error. It is the only HTTP code in the domain crates; ops inject it through the `Http` trait, tests script a `FakeHttp`. No tokio, no OpenSSL (`deny.toml` bans `openssl-sys`).
2. **HTTPS to allowlisted hosts only.** A request names a provider; the fetcher refuses any URL whose host is not that provider's (`Policy.allow_hosts`, exact or parent domain). `http://` is accepted for loopback only, so tests and the e2e suite run a local tile server. Today's hosts: `elevation-tiles-prod.s3.amazonaws.com` (Terrarium), `copernicus-dem-30m.s3.amazonaws.com` (Copernicus GLO-30), `portal.opentopography.org` (OpenTopography, bring your own key), `overpass-api.de` (OSM). Excluded sources stay excluded (Google, Earth Engine, Cesium, MapTiler, Mapbox Terrain-DEM).
3. **Adapts to the link.** The fetcher keeps an exponentially weighted throughput estimate and a connection count between `min_conn` (1) and `max_conn` (4, the Overpass/AWS etiquette). Every clean response at ≥ 1 MiB/s, or inside 400 ms, earns one more connection; every timeout, connection failure, 429 or 5xx halves the count, backs off (`Retry-After` honoured, else 0.5 s doubling, 30 s cap) and retries up to 4 times; a 4xx is final. Per-request deadlines are `base (8 s) + 2 × expected bytes / measured throughput`, floored at 100 KB/s and capped at 120 s, so a slow link gets patience and a dead one a quick retry. Bodies are capped at 64 MiB. Batches run on a scoped thread pool gated by the live connection count; the first failure cancels the rest; progress and cancel go through the jobs system.
4. **Cached on disk, validated by ETag.** `<project>/cache/geo/<provider>/<z>/<x>/<y>` (Terrarium) and `<provider>/<tile>.tif` (Copernicus) with a `.meta` holding the ETag; re-fetches are conditional and a 304 serves the cached body. Bounded by `cache_limit_bytes` with the oldest entries evicted first. OpenTopography responses carry the key in the URL and are never cached or logged. Offline mode (`offline: true`) serves the cache only and fails a miss with `E_NETWORK`.
5. **Keys are the user's.** OpenTopography is bring-your-own-key: the key is an op parameter (or `JD_OPENTOPOGRAPHY_KEY`), never written to `project.json`, the journal, the cache or a diagnostic bundle, and never shipped.
6. **Attribution is automatic.** Every fetch appends the provider's exact notice to `project.json → attribution[]` (journaled `project.add_attribution`, deduplicated by source), and every export writes `ATTRIBUTION.txt` beside its files when the list is non-empty. Copernicus carries its 6(b) notice and 6(c) liability sentence; Terrarium the registry citation and Mapzen's per-source credits; OSM "© OpenStreetMap contributors, ODbL" and keeps its features in a separately labelled GeoJSON file (a derivative database under ODbL).
7. **The placed DEM is an asset.** A fetch ends as an f32 GeoTIFF with tags in `assets/`, read by `jd.input.heightmap@1` through the existing reprojection, so a project stays reproducible after `cache/` is deleted and the determinism contract (ADR-0007) sees the asset hash.

## Alternatives rejected

- `reqwest` + an async runtime: already in the lockfile through Tauri, but would pull tokio and hyper into the domain crates for a few dozen requests.
- A fixed connection count: too few on a fast link, abusive on a slow one or a rate-limited host; the public Overpass instance rejects bursts with 429.
- Caching under `assets/`: assets are reproducibility inputs; the tile cache is disposable by design (ADR-0011).
- A shipped OpenTopography key: forbidden by their terms for commercial integration and a secret in a public repository.

## Consequences

- MCP exposes the `geo.*` ops only under `--scope network:geo` (or `all`); the CLI and desktop have them.
- CI and the gates never touch the network: unit tests script responses, the integration test spins a loopback server, and the real-bucket checks run only with `JD_GEO_ONLINE=1`.
- Adding a provider means a host on the allowlist, its attribution text, and a decoder — nothing else changes.

## Evidence

`crates/jd-geo/src/fetch.rs` tests (allowlist, 304/offline, backoff and recovery, pooled batch); `crates/jd-geo/tests/online.rs` (2026-10-23, Nairobi: 16 Terrarium tiles 572 KB in 6.6 s at ~100 KB/s, Copernicus S02 E036 39 MB in 18.8 s at 2.1 MB/s, both decoding to 1 611–1 774 m); research `docs/research/geo-vision.md` A1.8, A1.11, A2.2, A4.4.
