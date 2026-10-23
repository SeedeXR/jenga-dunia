//! Real-network checks, run only with `JD_GEO_ONLINE=1`: one Terrarium tile and one Copernicus
//! GLO-30 tile through the rustls client, decoded by the same code the ops use. Skipped
//! otherwise so CI and offline gates never touch the internet.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use jd_geo::dem::{
    Bbox, Provider, TerrariumPlan, copernicus_requests, decode_geotiff, mosaic_geographic,
};
use jd_geo::fetch::{DiskCache, Fetcher, Policy, UreqHttp};

fn online() -> bool {
    if std::env::var("JD_GEO_ONLINE").is_ok() {
        return true;
    }
    eprintln!("JD_GEO_ONLINE unset: skipping");
    false
}

fn fetcher(dir: &str) -> Fetcher {
    let policy = Policy {
        allow_hosts: vec![
            Provider::Terrarium.host().into(),
            Provider::Copernicus.host().into(),
        ],
        ..Policy::default()
    };
    let cache = DiskCache::new(std::env::temp_dir().join(dir), 512 << 20);
    Fetcher::new(
        Arc::new(UreqHttp::new(&policy.user_agent)),
        policy,
        Some(cache),
    )
}

#[test]
fn a_terrarium_mosaic_of_nairobi_comes_down_with_real_heights() {
    if !online() {
        return;
    }
    let bbox = Bbox {
        south: -1.32,
        west: 36.78,
        north: -1.26,
        east: 36.86,
    }; // Nairobi, ~1 700 m
    let plan = TerrariumPlan::new(
        &bbox,
        30.0,
        &format!("https://{}/terrarium", Provider::Terrarium.host()),
    )
    .unwrap();
    let f = fetcher("jd-geo-online");
    let cancel = AtomicBool::new(false);
    let (tiles, stats) = f
        .fetch_all(&plan.requests, &cancel, &mut |d, t| eprintln!("{d}/{t}"))
        .unwrap();
    eprintln!("terrarium: {stats:?}");
    let grid = plan
        .mosaic(
            &bbox,
            &tiles.iter().map(|t| t.bytes.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    let (lo, hi) = grid.stats();
    eprintln!(
        "grid {}×{} spacing {:.2} m, {lo}..{hi} m, voids {}",
        grid.width, grid.height, grid.spacing[0], grid.void_share
    );
    assert!(
        lo > 1_400.0 && hi < 2_200.0,
        "Nairobi sits at 1 600–1 900 m: {lo}..{hi}"
    );
    assert!(grid.void_share < 0.01);
    // Second pass: every tile is a cache hit (ETag or body).
    let (_, again) = f
        .fetch_all(&plan.requests, &cancel, &mut |_, _| {})
        .unwrap();
    assert_eq!(again.cache_hits, plan.tile_count(), "{again:?}");
}

#[test]
fn a_copernicus_cog_tile_decodes_with_the_tiff_crate() {
    if !online() {
        return;
    }
    let bbox = Bbox {
        south: -1.32,
        west: 36.78,
        north: -1.26,
        east: 36.86,
    };
    let reqs =
        copernicus_requests(&bbox, &format!("https://{}", Provider::Copernicus.host())).unwrap();
    assert_eq!(reqs.len(), 1);
    let f = fetcher("jd-geo-online");
    let cancel = AtomicBool::new(false);
    let (tiles, stats) = f.fetch_all(&reqs, &cancel, &mut |_, _| {}).unwrap();
    eprintln!("copernicus: {stats:?}");
    let grid = decode_geotiff(&tiles[0].bytes, "Copernicus S02 E036").unwrap();
    eprintln!(
        "tile {}×{} origin {:?} spacing {:?} voids {}",
        grid.width, grid.height, grid.origin, grid.spacing, grid.void_share
    );
    assert_eq!((grid.width, grid.height), (3600, 3600));
    assert!(
        (grid.origin[0] - 36.0).abs() < 1e-3 && (grid.origin[1] + 2.0).abs() < 1e-3,
        "{:?}",
        grid.origin
    );
    let crop = mosaic_geographic(&[grid], &bbox).unwrap();
    let (lo, hi) = crop.stats();
    eprintln!("crop {}×{} {lo}..{hi} m", crop.width, crop.height);
    assert!(lo > 1_400.0 && hi < 2_200.0, "{lo}..{hi}");
}
