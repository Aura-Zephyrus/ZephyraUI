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
    /// Create the window with per-pixel alpha (content can be translucent).
    ///
    /// This is the **main-window default**; a single window can override it at
    /// creation time via [`WindowOpts`]. Transparent implies frameless: the
    /// system titlebar would be an opaque rectangle on top of an alpha window.
    /// Delivery is backend's business (DComp / winit transparent attribute /
    /// Vulkan PRE_MULTIPLIED probe) — see `docs/developer/win/07-window-transparent.md`.
    pub transparent: bool,
}

/// Per-window options resolved **at creation time** (not from the config file).
///
/// # Why it lives here instead of in each backend
///
/// All three backends must share the same semantics ("different windows can have
/// different definitions"), and they already depend on this crate — one shared
/// definition makes that structural rather than incidental.
///
/// # Multi-window (TODO, not this round)
///
/// A future `windows` section in `ZephyraUI.json` may define named window
/// states; newly created windows pick a definition by key and fall back to the
/// main-window section (`window`). This struct is the shape that path plugs
/// into: the named definitions would carry the same fields resolved here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowOpts {
    /// Transparency of this window. `None` = follow the main-window default
    /// ([`WindowConfig::transparent`], and `Default` for this struct means
    /// exactly that); `Some(_)` overrides it — that is the "per-window
    /// definition" entry point.
    pub transparent: Option<bool>,
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
///
/// `ZEPHYRA_TRANSPARENT` (`1`/`0`) does the same for [`WindowConfig::transparent`]
/// — the main-window default, still overridable per window via
/// [`WindowOpts`]. It is the manual verification switch for the transparent
/// window mode (`ZEPHYRA_TRANSPARENT=1 cargo run …` docks onto any live app);
/// per-window definitions live in [`WindowOpts`], not here.
pub fn load_from_current_dir() -> AppConfig {
    let mut config = load_resolved();
    apply_frameless_override(
        &mut config,
        std::env::var("ZEPHYRA_FRAMELESS").ok().as_deref(),
    );
    apply_transparent_override(
        &mut config,
        std::env::var("ZEPHYRA_TRANSPARENT").ok().as_deref(),
    );
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

/// Apply a `ZEPHYRA_TRANSPARENT` override; same contract as
/// [`apply_frameless_override`].
fn apply_transparent_override(config: &mut AppConfig, value: Option<&str>) {
    match value {
        Some("1") => config.window.transparent = true,
        Some("0") => config.window.transparent = false,
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
