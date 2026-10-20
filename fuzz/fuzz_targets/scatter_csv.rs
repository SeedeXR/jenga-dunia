#![no_main]
//! Scatter CSV text (`u,v,z,r`) from an earlier layer: parse errors, never panics.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = jd_terrain::parse_points_csv(text);
    }
});
