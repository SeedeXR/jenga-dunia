//! Studio's crash reports (the desktop's `reportCrash`).

use std::sync::Arc;

/// A panic on the window's thread is logged and written as a crash report (`diag.crash_report`:
/// beside the project's ops log, else the temp dir), as the desktop's `reportCrash`. Engine
/// operations catch their own panics and report them (`write_crash_report`), so only the main
/// thread is reported here — the engine may hold its locks on the others.
pub fn install(server: Arc<jd_ipc::Server>) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            let message = info.payload_as_str().unwrap_or("panic").to_owned();
            let at = info.location().map(ToString::to_string).unwrap_or_default();
            let stack = format!("at {at}\n{}", std::backtrace::Backtrace::force_capture());
            tracing::error!(%at, "Studio panicked: {message}");
            let r = server.invoke(jd_core::Request::new(
                "diag.crash_report",
                serde_json::json!({ "message": message, "stack": stack, "source": "studio" }),
            ));
            match r.result {
                Some(v) => tracing::error!(report = %v["path"], "crash report written"),
                None => tracing::error!("could not write the crash report"),
            }
        }
        default(info);
    }));
}
