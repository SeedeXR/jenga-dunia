//! Every file in `fixtures/malformed/` must fail cleanly (structured error, no panic).
use jd_terrain::{ForeignImport, read_foreign};
use jd_types::ErrorCode;
use std::path::PathBuf;

#[test]
fn malformed_corpus_fails_with_structured_errors() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/malformed");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("fixtures/malformed exists") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            continue;
        }
        seen += 1;
        let opts = ForeignImport {
            range_m: Some([0.0, 100.0]),
            size_px: Some([2, 2]),
            ..Default::default()
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_owned();
        let Ok(result) = std::panic::catch_unwind(|| read_foreign(&path, &opts)) else {
            panic!("{name}: importer panicked");
        };
        match result {
            Ok((v, w, h)) => panic!("{name}: accepted as {w}×{h} with {} samples", v.len()),
            Err(e) => assert!(
                matches!(
                    e.code,
                    ErrorCode::E_FORMAT_CORRUPT
                        | ErrorCode::E_FORMAT_UNSUPPORTED
                        | ErrorCode::E_INPUT_INVALID
                ),
                "{name}: unexpected error code {:?}: {}",
                e.code,
                e.message
            ),
        }
    }
    assert!(seen >= 10, "corpus has {seen} files");
}
