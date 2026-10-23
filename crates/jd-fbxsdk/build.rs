//! Build the C shim against the Autodesk FBX SDK when it is installed (`FBXSDK_DIR`, else the
//! default install under /Applications or /usr), and tell the crate so with `cfg(fbxsdk)`;
//! otherwise the crate compiles to a stub. Nothing from the SDK is ever copied or shipped.
use std::path::{Path, PathBuf};

fn sdk_dir() -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = std::env::var_os("FBXSDK_DIR")
        .map(PathBuf::from)
        .into_iter()
        .chain(
            [
                "/Applications/Autodesk/FBX SDK/2020.3.9",
                "/usr/local/fbxsdk",
                "/opt/fbxsdk",
            ]
            .iter()
            .map(PathBuf::from),
        )
        .collect();
    candidates
        .into_iter()
        .find(|d| d.join("include/fbxsdk.h").is_file())
}

fn lib_dir(sdk: &Path) -> Option<PathBuf> {
    ["lib/clang/release", "lib/gcc/release", "lib/release", "lib"]
        .iter()
        .map(|p| sdk.join(p))
        .find(|d| d.join("libfbxsdk.a").is_file() || d.join("libfbxsdk-mt.lib").is_file())
}

fn main() {
    println!("cargo:rerun-if-env-changed=FBXSDK_DIR");
    println!("cargo:rerun-if-changed=cpp/shim.cpp");
    println!("cargo:rustc-check-cfg=cfg(fbxsdk)");
    let Some(sdk) = sdk_dir() else {
        println!(
            "cargo:warning=jd-fbxsdk: no Autodesk FBX SDK found (set FBXSDK_DIR); building the stub"
        );
        return;
    };
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("apple-darwin") {
        println!(
            "cargo:warning=jd-fbxsdk: the shim is wired for macOS only today ({target}); building the stub"
        );
        return;
    }
    let Some(libs) = lib_dir(&sdk) else {
        println!(
            "cargo:warning=jd-fbxsdk: {} has no release libfbxsdk; building the stub",
            sdk.display()
        );
        return;
    };
    let mut build = cc::Build::new();
    build
        .warnings(false)
        .cpp(true)
        .flag("-std=c++17")
        .flag("-Wno-nontrivial-memcall")
        .flag("-mmacosx-version-min=11.0")
        .include(sdk.join("include"))
        .file("cpp/shim.cpp");
    build.compile("jdfbxshim");
    // Link the static archive only: with `-lfbxsdk` against the SDK's lib dir Apple's linker picks
    // libfbxsdk.dylib and every binary then needs the dylib at run time. A search directory that
    // holds nothing but a symlink to the archive makes the choice deterministic, and the search
    // path + lib propagate to every crate that links this one (a `rustc-link-arg` would not).
    let archive = libs.join("libfbxsdk.a");
    #[cfg(not(unix))]
    {
        println!(
            "cargo:warning=jd-fbxsdk: linking {} is wired for unix hosts only; building the stub",
            archive.display()
        );
        return;
    }
    #[cfg(unix)]
    {
        let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
            println!("cargo:warning=jd-fbxsdk: no OUT_DIR; building the stub");
            return;
        };
        let link = out.join("libfbxsdk.a");
        let _ = std::fs::remove_file(&link);
        if let Err(e) = std::os::unix::fs::symlink(&archive, &link) {
            println!(
                "cargo:warning=jd-fbxsdk: cannot link to the SDK archive ({e}); building the stub"
            );
            return;
        }
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=fbxsdk");
        for l in ["xml2", "z", "iconv", "c++"] {
            println!("cargo:rustc-link-lib={l}");
        }
        for f in ["CoreFoundation", "CoreServices", "SystemConfiguration"] {
            println!("cargo:rustc-link-lib=framework={f}");
        }
        println!("cargo:rustc-cfg=fbxsdk");
    }
}
