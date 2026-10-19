//! Environment diagnostics (`jd diag env`, MCP `diag.env`).

use serde::{Deserialize, Serialize};

/// Presence of an optional external tool on `PATH`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToolStatus {
    /// Tool name.
    pub name: String,
    /// Resolved path, if found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Snapshot of the host environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EnvInfo {
    /// Jenga Dunia version.
    pub app_version: String,
    /// OS name and version.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Logical CPU count.
    pub cpus: usize,
    /// Total physical memory in bytes.
    pub memory_total_bytes: u64,
    /// Currently available memory in bytes.
    pub memory_available_bytes: u64,
    /// Current working directory.
    pub cwd: String,
    /// Optional external tools (validators, DCCs) detected on PATH.
    pub tools: Vec<ToolStatus>,
}

const OPTIONAL_TOOLS: &[&str] = &[
    "blender",
    "usdchecker",
    "usdcat",
    "usdzip",
    "gltf-validator",
    "gltf-transform",
    "toktx",
];

fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
        // Windows: honour PATHEXT so npm `.cmd` shims and `.bat` wrappers are found too.
        if cfg!(windows) {
            let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
            for ext in exts.split(';').filter(|e| !e.is_empty()) {
                let with_ext = dir.join(format!("{name}{}", ext.to_ascii_lowercase()));
                if with_ext.is_file() {
                    return Some(with_ext.display().to_string());
                }
            }
        }
    }
    None
}

/// Collect environment information. Never fails; unknown values are reported as such.
pub fn env_info() -> EnvInfo {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    EnvInfo {
        app_version: crate::APP_VERSION.to_owned(),
        os: format!(
            "{} {}",
            sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.to_owned()),
            sysinfo::System::os_version().unwrap_or_default()
        )
        .trim()
        .to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        cpus: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        memory_total_bytes: sys.total_memory(),
        memory_available_bytes: sys.available_memory(),
        cwd: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        tools: OPTIONAL_TOOLS
            .iter()
            .map(|t| ToolStatus {
                name: (*t).to_owned(),
                path: which(t),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn env_info_reports_sane_values() {
        let e = super::env_info();
        assert!(e.cpus >= 1);
        assert!(e.memory_total_bytes > 0);
        assert_eq!(e.tools.len(), super::OPTIONAL_TOOLS.len());
    }
}
