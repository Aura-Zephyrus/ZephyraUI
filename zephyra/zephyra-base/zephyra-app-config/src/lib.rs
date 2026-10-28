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
///
/// `ZEPHYRA_FRAMELESS` (`1`/`0`) overrides [`WindowConfig::frameless`] at runtime,
/// taking priority over any file. `window.frameless` is a general setting; this
/// override exists so apps that draw their own titlebar (where frameless is required,
/// not optional) can pin it independent of cwd — the file is resolved relative to the
/// working directory, so running `target/debug/<exe>` from anywhere but the project
/// root silently drops the setting and the OS titlebar reappears on top of the app's
/// own. All three window backends read frameless through this function, so one
/// override covers every backend.
pub fn load_from_current_dir() -> AppConfig {
    let mut config = load_resolved();
    let override_value = std::env::var("ZEPHYRA_FRAMELESS").ok();
    apply_frameless_override(&mut config, override_value.as_deref());
    config
}

/// Apply a `ZEPHYRA_FRAMELESS` override: `Some("1")` forces frameless, `Some("0")`
/// forces decorated, anything else leaves the config untouched.
///
/// Split out so the override logic is testable without mutating process-global
/// environment — env-var tests race when the harness runs tests in parallel.
fn apply_frameless_override(config: &mut AppConfig, value: Option<&str>) {
    match value {
        Some("1") => config.window.frameless = true,
        Some("0") => config.window.frameless = false,
        _ => {}
    }
}

/// Resolve the embedded or working-directory config file (no env override).
fn load_resolved() -> AppConfig {
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
