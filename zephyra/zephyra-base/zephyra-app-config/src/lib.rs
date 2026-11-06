#![deny(missing_docs)]
//! Runtime configuration stored in the consuming ZephyraUI project's root.
//!
//! CLI-built applications use the embedded project-root file; direct Cargo runs
//! resolve it relative to the process working directory.

use std::collections::HashMap;
use std::path::Path;

const CONFIG_FILE: &str = "ZephyraUI.json";

include!(concat!(env!("OUT_DIR"), "/embedded_config.rs"));

/// ZephyraUI settings from the consumer project's `ZephyraUI.json`.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    /// Window creation settings — the **main-window defaults**.
    pub window: WindowConfig,
    /// Named window definitions, keyed by the name passed to
    /// [`WindowOpts::named`] / the window crate's `WindowSpec::named`.
    ///
    /// Every field is optional: anything not written here falls back to
    /// [`AppConfig::window`], which itself falls back to the built-in default.
    /// That is what makes a named definition a *delta* rather than a complete
    /// copy — adding one new knob does not force every profile to restate it.
    pub windows: HashMap<String, WindowProfile>,
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
    /// Main-window title. `None` = the backend's built-in title.
    pub title: Option<String>,
    /// Main-window initial client size in **physical pixels**.
    pub width: Option<u32>,
    /// Main-window initial client height in **physical pixels**.
    pub height: Option<u32>,
    /// Whether the main window can be resized. `None` = resizable.
    pub resizable: Option<bool>,
    /// Keep the main window above normal windows.
    pub always_on_top: Option<bool>,
    /// Mark the main window as a tool window (no taskbar entry, small
    /// titlebar). Usually only meaningful for secondary windows.
    pub tool_window: Option<bool>,
}

/// One named entry of the `windows` section.
///
/// Every field is `Option`; `None` means "inherit from [`WindowConfig`]".
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WindowProfile {
    /// Window title.
    pub title: Option<String>,
    /// Client width in physical pixels.
    pub width: Option<u32>,
    /// Client height in physical pixels.
    pub height: Option<u32>,
    /// Override [`WindowConfig::frameless`].
    pub frameless: Option<bool>,
    /// Override [`WindowConfig::transparent`].
    pub transparent: Option<bool>,
    /// Override [`WindowConfig::resizable`].
    pub resizable: Option<bool>,
    /// Override [`WindowConfig::always_on_top`].
    pub always_on_top: Option<bool>,
    /// Override [`WindowConfig::tool_window`].
    pub tool_window: Option<bool>,
}

/// Per-window options resolved **at creation time** (not from the config file).
///
/// # Why it lives here instead of in each backend
///
/// All three backends must share the same semantics ("different windows can have
/// different definitions"), and they already depend on this crate — one shared
/// definition makes that structural rather than incidental.
///
/// # Resolution order
///
/// 1. Fields explicitly set here win.
/// 2. Otherwise the named profile (`windows.<name>`) applies.
/// 3. Otherwise [`AppConfig::window`] applies.
/// 4. Otherwise the built-in default.
///
/// [`WindowOpts::named`] performs step 2+3 for you; passing `WindowOpts::default()`
/// means "everything follows the main-window section".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowOpts {
    /// Name of the `windows.<name>` profile this window was created from.
    ///
    /// Kept for diagnostics (and so a backend can log which definition it used).
    pub name: Option<String>,
    /// Title of this window. `None` = inherit.
    pub title: Option<String>,
    /// Initial client size in physical pixels. `None` = inherit.
    pub size: Option<(u32, u32)>,
    /// Transparency of this window. `None` = follow the main-window default
    /// ([`WindowConfig::transparent`]); `Some(_)` overrides it.
    pub transparent: Option<bool>,
    /// Frameless (no OS decorations). `None` = follow the main-window default.
    pub frameless: Option<bool>,
    /// Resizable. `None` = follow the main-window default.
    pub resizable: Option<bool>,
    /// Always on top. `None` = follow the main-window default.
    pub always_on_top: Option<bool>,
    /// Tool-window style. `None` = follow the main-window default.
    pub tool_window: Option<bool>,
}

impl WindowOpts {
    /// Resolve the definition named `name` from `config`, layered over the
    /// main-window section.
    ///
    /// An unknown name is **not** an error: it resolves to the main-window
    /// defaults but keeps the name, so a typo is visible in the log rather than
    /// silently opening a differently-shaped window.
    pub fn named(config: &AppConfig, name: &str) -> Self {
        let mut opts = Self {
            name: Some(name.to_string()),
            title: config.window.title.clone(),
            size: match (config.window.width, config.window.height) {
                (Some(w), Some(h)) => Some((w, h)),
                _ => None,
            },
            transparent: Some(config.window.transparent),
            frameless: Some(config.window.frameless),
            resizable: config.window.resizable,
            always_on_top: config.window.always_on_top,
            tool_window: config.window.tool_window,
        };
        if let Some(profile) = config.windows.get(name) {
            if profile.title.is_some() {
                opts.title = profile.title.clone();
            }
            if let (Some(w), Some(h)) = (profile.width, profile.height) {
                opts.size = Some((w, h));
            }
            if profile.frameless.is_some() {
                opts.frameless = profile.frameless;
            }
            if profile.transparent.is_some() {
                opts.transparent = profile.transparent;
            }
            if profile.resizable.is_some() {
                opts.resizable = profile.resizable;
            }
            if profile.always_on_top.is_some() {
                opts.always_on_top = profile.always_on_top;
            }
            if profile.tool_window.is_some() {
                opts.tool_window = profile.tool_window;
            }
        }
        opts
    }

    /// The main-window definition (`window` section, no named profile).
    pub fn main(config: &AppConfig) -> Self {
        Self {
            name: None,
            title: config.window.title.clone(),
            size: match (config.window.width, config.window.height) {
                (Some(w), Some(h)) => Some((w, h)),
                _ => None,
            },
            transparent: Some(config.window.transparent),
            frameless: Some(config.window.frameless),
            resizable: config.window.resizable,
            always_on_top: config.window.always_on_top,
            tool_window: config.window.tool_window,
        }
    }

    /// Transparent after resolving against `config`.
    pub fn resolve_transparent(&self, config: &AppConfig) -> bool {
        self.transparent.unwrap_or(config.window.transparent)
    }

    /// Frameless after resolving against `config`.
    ///
    /// Transparent implies frameless: an opaque system titlebar over a
    /// per-pixel-alpha window has no coherent meaning.
    pub fn resolve_frameless(&self, config: &AppConfig) -> bool {
        self.resolve_transparent(config) || self.frameless.unwrap_or(config.window.frameless)
    }

    /// Resizable after resolving against `config`.
    pub fn resolve_resizable(&self, config: &AppConfig) -> bool {
        self.resizable.or(config.window.resizable).unwrap_or(true)
    }

    /// Always-on-top after resolving against `config`.
    pub fn resolve_always_on_top(&self, config: &AppConfig) -> bool {
        self.always_on_top
            .or(config.window.always_on_top)
            .unwrap_or(false)
    }

    /// Tool-window after resolving against `config`.
    pub fn resolve_tool_window(&self, config: &AppConfig) -> bool {
        self.tool_window
            .or(config.window.tool_window)
            .unwrap_or(false)
    }

    /// Title after resolving against `fallback`.
    pub fn resolve_title<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.title.as_deref().unwrap_or(fallback)
    }
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
