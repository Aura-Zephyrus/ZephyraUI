#![deny(missing_docs)]
//! Runtime configuration stored in the consuming ZephyraUI project's root.
//!
//! CLI-built applications use the embedded project-root file; direct Cargo runs
//! resolve it relative to the process working directory.

use std::path::Path;

const CONFIG_FILE: &str = "ZephyraUI.json";

include!(concat!(env!("OUT_DIR"), "/embedded_config.rs"));

/// ZephyraUI settings from the consumer project's `ZephyraUI.json`.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    /// Window creation settings.
    pub window: WindowConfig,
}

/// Window creation settings.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WindowConfig {
    /// Create the window without operating-system decorations.
    pub frameless: bool,
}

/// Load configuration from a project root.
///
/// A missing file means default settings. Invalid files return an error so callers
/// can report the issue rather than silently treating malformed configuration as
/// an intentional default.
pub fn load_from_project_root(root: &Path) -> Result<AppConfig, String> {
    let path = root.join(CONFIG_FILE);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppConfig::default());
        }
        Err(error) => {
            return Err(format!("读取 {} 失败：{error}", path.display()));
        }
    };

    parse_config(&contents, &path.display().to_string())
}

/// Load embedded configuration, or resolve it from the process working directory.
///
/// A missing file uses defaults. An unreadable or invalid file is reported to stderr
/// and also uses defaults so window creation can proceed.
pub fn load_from_current_dir() -> AppConfig {
    if let Some((contents, source)) = EMBEDDED_CONFIG {
        return match parse_config(contents, source) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("[zephyra] {error}；使用默认窗口配置");
                AppConfig::default()
            }
        };
    }

    let root = match std::env::current_dir() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("[zephyra] 获取应用项目工作目录失败：{error}");
            return AppConfig::default();
        }
    };

    match load_from_project_root(&root) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[zephyra] {error}；使用默认窗口配置");
            AppConfig::default()
        }
    }
}

fn parse_config(contents: &str, source: &str) -> Result<AppConfig, String> {
    serde_json::from_str(contents).map_err(|error| format!("解析 {source} 失败：{error}"))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
