//! `new` —— 生成一个可运行、带关键开发说明的示例工程。
//!
//! # 为什么不覆盖已存在的目录
//!
//! 覆盖等于**静默删掉别人的代码**。要覆盖就显式 `--force`，而且只删我们**自己**
//! 写过的那些文件（见 `TEMPLATES` 的名单），不碰目录里的其它东西。

use super::{CliError, Write};
use crate::args::Args;
use crate::ExitCode;

/// 模板文件名。`is_ours` 判定只认这些 —— 用来决定 `--force` 能删什么。
const TEMPLATES: &[&str] = &[
    "README.md",
    "Cargo.toml",
    "zephyra.toml",
    "ZephyraUI.json",
    "src/main.rs",
    "frontend/app.rs",
    "frontend/app.zstyle",
    "styles/app.zstyle",
];

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    let Some(name) = a.positional_at(0) else {
        return Err(CliError::usage(
            "new needs a project name. Try `zephyra new myapp`.",
        ));
    };
    if name.trim().is_empty() {
        return Err(CliError::usage("project name is empty"));
    }
    // crate 名只能是标识符片段；这里挡掉会生成**编译不过**的工程的那几种。
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        return Err(CliError::usage(format!(
            "`{name}` is not a valid crate name: it must start with a letter or `_`"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(CliError::usage(format!(
            "`{name}` is not a valid crate name: only letters, digits, `_` and `-`"
        )));
    }

    let root = match a.option("path") {
        Some(p) => std::path::PathBuf::from(p).join(name),
        None => std::path::PathBuf::from(name),
    };
    let force = a.has("force");

    if root.exists() {
        if !force {
            return Err(CliError::missing(format!(
                "{} already exists. Pass --force to overwrite the files `zephyra new` created.",
                root.display()
            )));
        }
        for rel in TEMPLATES {
            let p = root.join(rel);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| {
                    CliError::failure(format!("cannot remove {}: {e}", p.display()))
                })?;
            }
        }
    }

    std::fs::create_dir_all(root.join("src")).map_err(|e| {
        CliError::failure(format!("cannot create {}: {e}", root.join("src").display()))
    })?;
    std::fs::create_dir_all(root.join("frontend")).map_err(|e| {
        CliError::failure(format!(
            "cannot create {}: {e}",
            root.join("frontend").display()
        ))
    })?;

    write_file(&root.join("Cargo.toml"), &cargo_toml(name))?;
    write_file(&root.join("zephyra.toml"), &zephyra_toml(name))?;
    write_file(&root.join("ZephyraUI.json"), &app_config_json())?;
    write_file(&root.join("README.md"), &readme(name))?;
    write_file(&root.join("src/main.rs"), &main_rs(name))?;
    write_file(&root.join("frontend/app.rs"), &app_rs())?;
    write_file(&root.join("frontend/app.zstyle"), &app_zstyle())?;

    w.line(&format!("created {}", root.display()));
    w.line("next:");
    w.line(&format!("  cd {}", root.display()));
    w.line(&format!(
        "  zephyra check frontend/app.zstyle   # {}.zstyle",
        name
    ));
    w.line("  zephyra dev --styles frontend");
    Ok(ExitCode::Success)
}

fn write_file(p: &std::path::Path, contents: &str) -> Result<(), CliError> {
    std::fs::write(p, contents)
        .map_err(|e| CliError::failure(format!("cannot write {}: {e}", p.display())))
}

fn cargo_toml(name: &str) -> String {
    // 依赖面照抄 `examples`（那是本仓唯一被反复真跑过的窗口工程），并且
    // **锁默认走 D3D12** —— D3D11 那条路本仓的 GPU 门禁只覆盖到一部分，
    // 让新工程第一步就走完整的那条。
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false

[features]
default = ["d3d12"]
d3d12 = ["dep:zephyra-winit-d3d12"]
d3d11 = ["dep:zephyra-winit"]

[dependencies]
zephyra-core = {{ path = "{core}" }}
zephyra-fx = {{ path = "{fx}" }}
zephyra-style = {{ path = "{style}" }}
zstylec = {{ path = "{zstylec}" }}
winit = "0.30"
zephyra-winit-d3d12 = {{ path = "{d3d12}", optional = true }}
zephyra-winit = {{ path = "{winit_base}", optional = true }}
"#,
        core = crate_dep("zephyra/zephyra-core"),
        fx = crate_dep("zephyra/zephyra-fx"),
        style = crate_dep("zephyra/zephyra-style"),
        zstylec = crate_dep("zstylec"),
        d3d12 = crate_dep("zephyra/zephyra-base/zephyra-winit-d3d12"),
        winit_base = crate_dep("zephyra/zephyra-base/zephyra-winit"),
    )
}

/// 仓库里某个子目录的**绝对**路径。
///
/// # 为什么是绝对而不是相对
///
/// 工程是用 `zephyra new` 建在**任意**目录下的（`--path` 给什么就在哪），
/// 而它依赖的是本机这一份 ZephyraUI checkout。相对路径（`../../ZephyraUI/...`）
/// 看起来更「可移植」，实测是**错的**：工程挪一下位置、或者被建在 ZephyraUI
/// 里面的某处，相对路径立刻指到别的地方去 —— 而失败方式是「找不到依赖」，
/// 与「我写错了相对路径」之间没有任何提示关联。
///
/// 绝对路径代价是这份 Cargo.toml 不能被别人直接复用。接受：那本来就要改版本号，
/// 而「`cargo check` 直接过」比「文件好看」值钱。
fn crate_dep(rel: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR"); // .../ZephyraUI/ZephyraCLI
    std::path::Path::new(root)
        .parent() // .../ZephyraUI
        .unwrap_or_else(|| std::path::Path::new(root))
        .join(rel)
        .to_string_lossy()
        .replace('\\', "/")
}
fn zephyra_toml(name: &str) -> String {
    format!(
        r#"[app]
name = "{name}"
# 窗口初始尺寸（物理像素）。改这里就能看到 live resize 的效果。
width = 1000
height = 640

[styles]
# 相对工程根。改完跑 `zephyra check frontend/app.zstyle`。
files = ["frontend/app.zstyle"]
"#
    )
}

fn app_config_json() -> String {
    r#"{
  "window": {
    "frameless": false,
    "transparent": false
  }
}
"#
    .to_string()
}

fn readme(name: &str) -> String {
    format!(
        r#"# {name}

由 `zephyra new` 生成的 ZephyraUI 示例工程。

## 运行

```text
cargo run
```

默认启用 Windows D3D12。选择 D3D11：

```text
cargo run --no-default-features --features d3d11
```

## 项目文件

- `src/main.rs`：窗口宿主入口和 winit 事件循环接线。
- `frontend/app.rs`：可交互示例 View，展示应用状态、稳定节点 key 和元素结构。
- `frontend/app.zstyle`：外观和布局样式；源码描述元素与状态，不硬编码颜色或字号。
- `zephyra.toml`：应用名、初始物理像素尺寸和样式表入口。
- `ZephyraUI.json`：窗口选项。通过 `zephyra run`、`dev`、`build` 生成的程序会嵌入它；
  `window.frameless: true` 会移除系统标题栏，自绘标题栏和拖动行为由应用实现；
  `window.transparent: true` 开启窗口透明模式（逐像素 alpha，含 frameless；
  见 docs/developer/win/07-window-transparent.md）。
- `Cargo.toml`：后端 feature 与本机 ZephyraUI 源码依赖路径；迁移到其他机器时应调整路径。

## 常用开发命令

```text
zephyra dev --styles frontend
zephyra check frontend/app.zstyle
zephyra props text
```

`dev` 会输出帧 trace，并在样式表变化时重启应用。编辑 `.rs` 后仍由 Cargo 重新编译。

## 扩展示例

1. 在 `frontend/app.rs` 的 `App::on_input` 读取 `InputState`，把输入转换为应用自己的状态。
2. 在 `App::view` 根据状态构造 `Element` 树；交互节点使用固定 `.with_key(...)`，不要用每帧变化的 key。
3. 在 `frontend/app.zstyle` 为节点声明样式；属性名可用 `zephyra props` 查询，改后用 `zephyra check` 验证。
4. 窗口只能在 winit `ApplicationHandler::resumed` 中创建；`window_event` 必须转发给 `HostWindow`。
5. 布局和鼠标坐标使用物理像素。有效像素断点需通过 `InputState.scale` 换算。
"#
    )
}

/// 宿主入口只负责连接应用 View 与窗口事件循环。
fn main_rs(name: &str) -> String {
    let _ = name;
    r#"#[path = "../frontend/app.rs"]
mod frontend;
use frontend::App;
#[cfg(all(windows, feature = "d3d12"))]
use zephyra_winit_d3d12::HostWindow;
#[cfg(all(windows, feature = "d3d11", not(feature = "d3d12")))]
use zephyra_winit::HostWindow;
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop};

#[cfg(all(windows, feature = "d3d12"))]
type Host = HostWindow<App>;
#[cfg(all(windows, feature = "d3d11", not(feature = "d3d12")))]
type Host = HostWindow<App>;

/// winit 0.30 要求在 `resumed` 中创建窗口，并将窗口事件转发给宿主。
#[cfg(any(
    all(windows, feature = "d3d12"),
    all(windows, feature = "d3d11", not(feature = "d3d12"))
))]
struct Bootstrap {
    inner: Option<Host>,
    app: Option<App>,
}

#[cfg(any(
    all(windows, feature = "d3d12"),
    all(windows, feature = "d3d11", not(feature = "d3d12"))
))]
impl ApplicationHandler for Bootstrap {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.inner.is_some() {
            return;
        }
        let app = self.app.take().expect("resumed called without an application");
        self.inner = Some(Host::new_lenient(event_loop, app));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let Some(app) = self.inner.as_mut() {
            app.window_event(event_loop, id, event);
        }
    }
}

#[cfg(any(
    all(windows, feature = "d3d12"),
    all(windows, feature = "d3d11", not(feature = "d3d12"))
))]
fn main() {
    let event_loop = EventLoop::new().expect("EventLoop::new");
    let mut boot = Bootstrap {
        inner: None,
        app: Some(App::default()),
    };
    if let Err(e) = event_loop.run_app(&mut boot) {
        eprintln!("event loop failed: {e}");
    }
}

#[cfg(not(any(
    all(windows, feature = "d3d12"),
    all(windows, feature = "d3d11", not(feature = "d3d12"))
)))]
fn main() {
    eprintln!("The generated window sample requires Windows with the d3d12 or d3d11 feature.");
}
"#
    .to_string()
}

/// 前端示例：状态、输入与声明树如何组成一帧。
fn app_rs() -> String {
    r#"use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::id::ElementKey;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_fx::{InputState, View};

const BUTTON_KEY: &str = "sample-button";

/// 输入更新应用状态，view 再把状态声明为下一帧的元素树。
#[derive(Default)]
pub struct App {
    hovered: bool,
    pressed: bool,
    was_pressed: bool,
    clicked: bool,
}

impl View for App {
    fn on_input(&mut self, state: &InputState) {
        let is_hovered = state.hovered.as_ref().and_then(ElementKey::as_name) == Some(BUTTON_KEY);
        let is_pressed = state.pressed.as_ref().and_then(ElementKey::as_name) == Some(BUTTON_KEY);

        if self.was_pressed && !is_pressed {
            self.clicked = !self.clicked;
        }
        self.hovered = is_hovered;
        self.pressed = is_pressed;
        self.was_pressed = is_pressed;
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        let label = if self.pressed {
            "Release to activate"
        } else if self.clicked {
            "Clicked! Click again to reset"
        } else if self.hovered {
            "Click me"
        } else {
            "Move pointer here, then click"
        };

        Element::new(NodeTag::Box)
            .with_key("root")
            .with_children([
                Element::new(NodeTag::Text)
                    .with_key(BUTTON_KEY)
                    .with(&props::TEXT, PropertyValue::static_str(label)),
            ])
    }
}
"#
    .to_string()
}

fn app_zstyle() -> String {
    r#"/* ZephyraUI stylesheet.
 *
 * 检查：`zephyra check frontend/app.zstyle`
 * 列全部受支持的属性：`zephyra props`
 */

text {
    text-color: #202020;
    text-size: 24;
}
"#
    .to_string()
}
