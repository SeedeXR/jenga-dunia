//! A panic on Studio's window thread leaves a crash report (its own process: the hook is global).

use std::sync::Arc;

use jd_core::{Context, Registry};

#[test]
fn a_panic_on_the_window_thread_writes_a_crash_report() {
    let server = Arc::new(
        jd_ipc::Server::bind(Registry::with_builtins(), Context::default()).expect("bind"),
    );
    jd_studio::crash::install(server);
    let dir = std::env::temp_dir().join("jenga-dunia").join("crashes");
    let before: Vec<_> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    // The window's thread is `main`; a worker's panic (the engine's own) is not reported here.
    let _ = std::thread::Builder::new()
        .name("worker".into())
        .spawn(|| panic!("not the window"))
        .unwrap()
        .join();
    let _ = std::thread::Builder::new()
        .name("main".into())
        .spawn(|| panic!("kittest boom"))
        .unwrap()
        .join();
    let new: Vec<_> = std::fs::read_dir(&dir)
        .expect("crash dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| !before.contains(p))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(
        new[0].contains("kittest boom") && new[0].contains("ui:studio"),
        "{}",
        new[0]
    );
}
