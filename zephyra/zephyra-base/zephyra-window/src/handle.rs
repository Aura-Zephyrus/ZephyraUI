//! 应用侧窗口句柄与命令队列。
//!
//! # 为什么是队列而不是直接调 `&mut WindowSet`
//!
//! 应用代码在 `View::view` / 事件处理里拿到的是 `&self`，而"关掉这个窗口"
//! 需要 `&mut WindowSet`。两者同时存在就借不过。WinUI 里 `Window.Close()`
//! 也只是把消息投进 DispatcherQueue，真正动作发生在消息泵里 —— 本模块照这个形状。
//!
//! 队列本身是 `Rc<RefCell<...>>`，所以 [`WindowHandle`] **线程亲和**：
//! 把它塞进 `thread::spawn` 会编译失败。这是有意的。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use zephyra_core::theme::ThemeVariant;

use crate::spec::WindowKey;
use crate::waker::AppWaker;

/// 投给 [`crate::WindowSet`] 的一条命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowCommand {
    /// 关闭目标窗口（owned 窗口会级联关闭）。
    Close,
    /// 设置标题。
    SetTitle(String),
    /// 切换主题。
    SetTheme(ThemeVariant),
    /// 请求重绘。
    RequestRedraw,
    /// 退出整个事件循环（先关掉全部窗口）。
    Exit,
}

pub(crate) type Queue = Rc<RefCell<Vec<(WindowKey, WindowCommand)>>>;

/// 应用侧窗口句柄。
///
/// 克隆是廉价的（`Rc`）。窗口关闭后所有克隆都变为失效：
/// [`WindowHandle::is_alive`] 返回 `false`，后续命令被静默丢弃 ——
/// 与 WinUI 里"关掉的 Window 再调 Close 不抛"一致。
#[derive(Clone)]
pub struct WindowHandle {
    key: WindowKey,
    queue: Queue,
    alive: Rc<Cell<bool>>,
    /// 跨线程唤醒器（P0.3）：与 `WindowSet` 共享存活标志，关窗时一并失效。
    waker: AppWaker,
}

impl WindowHandle {
    pub(crate) fn new(
        key: WindowKey,
        queue: Queue,
        alive: Rc<Cell<bool>>,
        proxy: Option<EventLoopProxy<()>>,
        waker_alive: Arc<AtomicBool>,
    ) -> Self {
        let waker = AppWaker::new(proxy, waker_alive);
        Self {
            key,
            queue,
            alive,
            waker,
        }
    }

    /// 这个句柄对应的窗口标识。
    pub fn key(&self) -> WindowKey {
        self.key
    }

    /// 窗口是否还活着。
    pub fn is_alive(&self) -> bool {
        self.alive.get()
    }

    /// 可跨线程 clone 的唤醒器（P0.3）。
    ///
    /// 采集线程 / 文件对话框回调可以拿它 `post(msg)`，宿主在下一帧把消息交给
    /// `View::on_message`。窗口关闭后 `wake` / `post` 静默丢弃。
    pub fn waker(&self) -> AppWaker {
        self.waker.clone()
    }

    /// 请求关闭窗口。
    pub fn close(&self) {
        self.push(WindowCommand::Close);
    }

    /// 设置标题。
    pub fn set_title(&self, title: impl Into<String>) {
        self.push(WindowCommand::SetTitle(title.into()));
    }

    /// 切换本窗口主题。
    pub fn set_theme(&self, variant: ThemeVariant) {
        self.push(WindowCommand::SetTheme(variant));
    }

    /// 请求重绘。
    pub fn request_redraw(&self) {
        self.push(WindowCommand::RequestRedraw);
    }

    /// 请求退出整个事件循环（先关掉全部窗口）。
    pub fn exit(&self) {
        self.push(WindowCommand::Exit);
    }

    fn push(&self, command: WindowCommand) {
        if !self.alive.get() {
            return;
        }
        self.queue.borrow_mut().push((self.key, command));
    }
}

impl std::fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowHandle")
            .field("key", &self.key)
            .field("alive", &self.alive.get())
            .finish()
    }
}
