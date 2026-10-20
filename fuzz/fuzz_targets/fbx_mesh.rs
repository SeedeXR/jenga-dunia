#![no_main]
//! The binary FBX reader must never panic or hang on hostile input (nesting, array lengths and
//! zlib output are bounded in `jd_terrain::fbx`); an error is the only acceptable outcome.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = jd_terrain::fbx::read_fbx_mesh_bytes(data);
});
