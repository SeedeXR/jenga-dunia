//! Windows (MSVC): embed `windows/studio.manifest` into jd-studio.exe, so the loader gives it
//! Common Controls 6 — the menu bar (muda) needs it, and without it the exe does not start.
//! Elsewhere there is nothing to do.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !(target.contains("windows") && target.contains("msvc")) {
        return;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows/studio.manifest");
    println!("cargo:rerun-if-changed=windows/studio.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
}
