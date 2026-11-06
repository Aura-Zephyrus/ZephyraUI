#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
//! 窗口集合：每线程一个协调者，持有多个顶层窗口。
//!
//! # 形状（照 WinUI 的 per-thread core）
//!
//! ```text
//! WindowSet<B>                 ≈ DXamlCore + 消息泵
//! ├─ B (Backend)               ≈ 设备/队列/图集/纹理表的持有者（进程级共享）
//! └─ windows: Vec<Box<dyn WindowHost>>
//!    ├─ HostWindow<Gallery>    ≈ DesktopWindowImpl + 内容树
//!    └─ HostWindow<Inspector>  ≈ DesktopWindowImpl + 另一棵内容树
//! ```
//!
//! 三条与后端无关的判据住在这里：
//!
//! 1. **事件按窗口分发** —— 宿主不再忽略 `WindowId`；
//! 2. **owned 窗口级联关闭** —— 工具窗口跟随主窗口；
//! 3. **最后窗口关闭退出事件循环**（[`ShutdownMode`]）。
//!
//! # 线程模型
//!
//! [`WindowSet`] 是**线程亲和**的：它包含 `Rc`，因此 `!Send` / `!Sync`，
//! 编译器会阻止你把它搬到别的线程。这是有意的 —— WinUI 的 `DXamlCore`、
//! `CCoreServices` 与所有 Xaml 对象同样是线程亲和的。
//!
//! 想跨线程开窗口时，在**另一个线程**上再建一个 [`WindowSet`]（它自己的
//! `EventLoop`）。GPU 设备/图集/纹理表由各后端做成**进程级**共享
//! （`OnceLock<Arc<Mutex<...>>>`），所以两个线程不会各建一套设备。
//!
//! ⚠ winit 0.30 在 Linux（X11/Wayland）上只允许**主线程**拥有事件循环；
//! Windows 允许每线程一个。跨线程用法在 Linux 上会 panic —— 这是 winit 的
//! 限制，不是本层的取舍，记账见 `docs/dev-log/2026-11-multi-window.md`。

mod backend;
mod handle;
mod host;
mod set;
mod spec;

pub use backend::Backend;
pub use handle::{WindowCommand, WindowHandle};
pub use host::{HostEventOutcome, HostTick, WindowHost};
pub use set::{ShutdownMode, WindowSet};
pub use spec::{WindowKey, WindowSpec};

/// 重新导出，方便后端 crate 实现 [`Backend`] 时不必各自依赖配置 crate。
pub use zephyra_app_config::{AppConfig, WindowOpts};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
