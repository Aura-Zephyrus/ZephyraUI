//! 窗口集合与事件泵。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use zephyra_core::styler::Styler;
use zephyra_core::theme::ThemeVariant;
use zephyra_fx::View;

use crate::backend::Backend;
use crate::handle::{Queue, WindowCommand, WindowHandle};
use crate::host::{HostEventOutcome, HostTick, WindowHost};
use crate::spec::{WindowKey, WindowSpec};

/// 最后一个窗口关闭后事件循环怎么办。
///
/// 对应 WinUI 的 `DispatcherShutdownMode`：
/// - [`Self::OnLastWindowClosed`] ≈ `OnLastWindowClose`；
/// - [`Self::Explicit`] ≈ `OnExplicitShutdown`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShutdownMode {
    /// 最后一个窗口关闭 ⇒ 退出事件循环（默认）。
    #[default]
    OnLastWindowClosed,
    /// 窗口全关也不退出；应用自己调 `WindowHandle::exit` 或 `ActiveEventLoop::exit`。
    Explicit,
}

struct Entry {
    key: WindowKey,
    winit_id: Option<WindowId>,
    owner: Option<WindowKey>,
    host: Box<dyn WindowHost>,
    alive: Rc<Cell<bool>>,
    /// 与 `WindowHandle` 的 `AppWaker` 共享的存活标志（P0.3 关窗时一并失效）。
    waker_alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// 每线程一个的窗口协调者。
///
/// 它**就是** `ApplicationHandler`：`WindowSet::run` 负责建 `EventLoop` 并把它跑起来。
///
/// # 事件分发
///
/// `window_event` 按 `WindowId` 找到宿主再转发。原实现里 `_id` 被忽略，多窗口下
/// 所有事件都会落到同一个窗口上 —— 症状是"第二个窗口点不动、第一个窗口乱动"。
///
/// # 关闭
///
/// - `CloseRequested` 只关**当前**窗口；
/// - owned 窗口级联关闭（工具窗口跟随主窗口）；
/// - 最后一个窗口关闭时按 [`ShutdownMode`] 决定是否 `exit()`。
pub struct WindowSet<B: Backend> {
    backend: B,
    windows: Vec<Entry>,
    queue: Queue,
    shutdown: ShutdownMode,
    next_key: u64,
    on_start: Option<StartHandler<B>>,
    on_tick: Option<TickHandler<B>>,
    on_window_closed: Option<CloseHandler<B>>,
    /// 事件循环代理（P0.3）：只在 `run()` 里拿得到，`WindowSet::new` 的测试路径为 None。
    waker_proxy: Option<winit::event_loop::EventLoopProxy<()>>,
}

/// 首次 `resumed` 的处理器。抽成别名只为了让字段类型可读（clippy 的
/// `type_complexity` 也是这么判的）。
///
/// `B: Backend` 这条界在别名上不受检查（Rust 的 `type_alias_bounds`），但
/// `WindowSet<B>` 本身要求它，所以保留界只为可读性。
#[allow(type_alias_bounds)]
type StartHandler<B: Backend> = Box<dyn FnOnce(&mut WindowSet<B>, &ActiveEventLoop)>;
/// 每 tick 的处理器。
#[allow(type_alias_bounds)]
type TickHandler<B: Backend> = Box<dyn FnMut(&mut WindowSet<B>, &ActiveEventLoop)>;
/// 窗口关闭后的处理器。
#[allow(type_alias_bounds)]
type CloseHandler<B: Backend> = Box<dyn FnMut(&mut WindowSet<B>, WindowKey)>;

impl<B: Backend> WindowSet<B> {
    /// 建一个空集合。
    pub fn new(backend: B, shutdown: ShutdownMode) -> Self {
        Self {
            backend,
            windows: Vec::new(),
            queue: Rc::new(RefCell::new(Vec::new())),
            shutdown,
            next_key: 0,
            on_start: None,
            on_tick: None,
            on_window_closed: None,
            waker_proxy: None,
        }
    }

    /// 建 `EventLoop`、调用一次 `on_start`、然后跑消息循环。
    ///
    /// # 线程
    ///
    /// 事件循环在**调用线程**上建立。想再开一个线程的窗口集合，就在那个线程里
    /// 再调一次本函数 —— GPU 设备/图集由后端进程级共享。Linux 上 winit 只允许
    /// 主线程拥有事件循环，见 crate 文档。
    pub fn run<F>(
        backend: B,
        shutdown: ShutdownMode,
        on_start: F,
    ) -> Result<(), winit::error::EventLoopError>
    where
        F: FnOnce(&mut Self, &ActiveEventLoop) + 'static,
    {
        let event_loop = EventLoop::new()?;
        let mut set = Self::new(backend, shutdown);
        set.waker_proxy = Some(event_loop.create_proxy());
        set.on_start = Some(Box::new(on_start));
        event_loop.run_app(&mut set)
    }

    /// 每个 tick 调一次（在窗口 tick 之前）。用于对账应用级意图。
    pub fn set_tick_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&mut Self, &ActiveEventLoop) + 'static,
    {
        self.on_tick = Some(Box::new(handler));
    }

    /// 首次 `resumed` 时调一次。调用方在这里 `create_window`。
    ///
    /// [`Self::run`] 内部也走它；直接 `event_loop.run_app(&mut set)` 的应用用
    /// 这个方法注册自己的建窗逻辑（还要注册 tick / close 处理器）。
    pub fn set_start_handler<F>(&mut self, handler: F)
    where
        F: FnOnce(&mut Self, &ActiveEventLoop) + 'static,
    {
        self.on_start = Some(Box::new(handler));
    }

    /// 某个窗口关闭后调一次。
    pub fn set_close_handler<F>(&mut self, handler: F)
    where
        F: FnMut(&mut Self, WindowKey) + 'static,
    {
        self.on_window_closed = Some(Box::new(handler));
    }

    /// 创建一个窗口。
    ///
    /// `make_view` 收到该窗口的句柄 —— View 可以把句柄存进自己的状态，
    /// 于是按钮里能直接 `handle.close()` 而不必绕一个全局表。
    pub fn create_window<V, F>(
        &mut self,
        event_loop: &ActiveEventLoop,
        spec: WindowSpec,
        make_view: F,
    ) -> Result<WindowHandle, String>
    where
        V: View + 'static,
        F: FnOnce(WindowHandle) -> V,
    {
        let key = self.allocate_key();
        let alive = Rc::new(Cell::new(true));
        let waker_alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let handle = WindowHandle::new(
            key,
            self.queue.clone(),
            alive.clone(),
            self.waker_proxy.clone(),
            waker_alive.clone(),
        );
        let view = make_view(handle.clone());
        let host = self
            .backend
            .create(event_loop, view, &spec, handle.clone())?;
        let winit_id = host.winit_id();
        let owner = spec.owner;
        self.windows.push(Entry {
            key,
            winit_id,
            owner,
            host: Box::new(host),
            alive,
            waker_alive,
        });
        // ⚠ 必须在 `Box` 完成**之后**调 `start()`：live-resize / UIA 子类化
        // 持有指向宿主的裸指针，地址必须先稳定。
        let start_result = self
            .windows
            .last_mut()
            .map(|entry| entry.host.start())
            .unwrap_or(Ok(()));
        start_result?;
        Ok(handle)
    }

    /// 窗口数量。
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    /// 当前所有窗口的标识（按创建顺序）。
    pub fn keys(&self) -> Vec<WindowKey> {
        self.windows.iter().map(|e| e.key).collect()
    }

    /// 按标识找宿主。
    pub fn host(&self, key: WindowKey) -> Option<&dyn WindowHost> {
        self.windows
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.host.as_ref())
    }

    /// 按标识找宿主（可变）。
    pub fn host_mut(&mut self, key: WindowKey) -> Option<&mut (dyn WindowHost + '_)> {
        let position = self.windows.iter().position(|e| e.key == key)?;
        Some(&mut *self.windows[position].host)
    }

    /// 后端（例如注册共享纹理）。
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// 后端（可变）。
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// 切换全部窗口的主题。
    pub fn set_theme_all(&mut self, variant: ThemeVariant) {
        for entry in &mut self.windows {
            entry.host.set_theme(variant);
        }
    }

    /// 给某个窗口装样式注入点。
    pub fn set_styler(&mut self, key: WindowKey, styler: Box<dyn Styler>) -> bool {
        match self.windows.iter_mut().find(|e| e.key == key) {
            Some(entry) => {
                entry.host.set_styler(styler);
                true
            }
            None => false,
        }
    }

    /// 关闭一个窗口（owned 窗口级联关闭）。
    pub fn close_window(&mut self, key: WindowKey) {
        let mut to_close = vec![key];
        let mut index = 0;
        while index < to_close.len() {
            let parent = to_close[index];
            for entry in &self.windows {
                if entry.owner == Some(parent) && !to_close.contains(&entry.key) {
                    to_close.push(entry.key);
                }
            }
            index += 1;
        }

        for key in to_close {
            let Some(position) = self.windows.iter().position(|e| e.key == key) else {
                continue;
            };
            let mut entry = self.windows.remove(position);
            entry.host.teardown();
            entry.alive.set(false);
            entry
                .waker_alive
                .store(false, std::sync::atomic::Ordering::Relaxed);
            // 宿主（含 winit Window）在这里析构：交换链先死、窗口最后死，
            // 顺序由各后端自己的字段声明保证。
            drop(entry);
            if let Some(mut handler) = self.on_window_closed.take() {
                handler(self, key);
                self.on_window_closed = Some(handler);
            }
        }
    }

    /// 关闭全部窗口。
    pub fn close_all(&mut self) {
        let keys: Vec<WindowKey> = self.windows.iter().map(|e| e.key).collect();
        for key in keys {
            self.close_window(key);
        }
    }

    fn allocate_key(&mut self) -> WindowKey {
        let key = WindowKey(self.next_key);
        self.next_key += 1;
        key
    }

    /// 测试用：把一个已经建好的宿主放进集合。
    #[cfg(test)]
    pub(crate) fn push_host(
        &mut self,
        key: WindowKey,
        host: Box<dyn WindowHost>,
        owner: Option<WindowKey>,
    ) -> WindowHandle {
        let alive = Rc::new(Cell::new(true));
        let waker_alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let handle = WindowHandle::new(
            key,
            self.queue.clone(),
            alive.clone(),
            self.waker_proxy.clone(),
            waker_alive.clone(),
        );
        let winit_id = host.winit_id();
        self.windows.push(Entry {
            key,
            winit_id,
            owner,
            host,
            alive,
            waker_alive,
        });
        handle
    }

    #[cfg(test)]
    pub(crate) fn allocate_key_for_test(&mut self) -> WindowKey {
        self.allocate_key()
    }

    /// 执行排队命令；`exit` 被置位表示调用方应当退出事件循环。
    pub(crate) fn drain_commands(&mut self, exit: &mut bool) {
        let commands: Vec<(WindowKey, WindowCommand)> = self.queue.borrow_mut().drain(..).collect();
        for (key, command) in commands {
            match command {
                WindowCommand::Close => self.close_window(key),
                WindowCommand::SetTitle(title) => {
                    if let Some(entry) = self.windows.iter_mut().find(|e| e.key == key) {
                        entry.host.set_title(&title);
                    }
                }
                WindowCommand::SetTheme(variant) => {
                    if let Some(entry) = self.windows.iter_mut().find(|e| e.key == key) {
                        entry.host.set_theme(variant);
                    }
                }
                WindowCommand::RequestRedraw => {
                    if let Some(entry) = self.windows.iter().find(|e| e.key == key) {
                        entry.host.request_redraw();
                    }
                }
                WindowCommand::Exit => *exit = true,
            }
        }
    }

    /// 聚合所有窗口的 tick 结论，决定这一次控制流。
    pub(crate) fn aggregate_tick(&mut self) -> Aggregate {
        let mut aggregate = Aggregate::default();
        for entry in &mut self.windows {
            let tick: HostTick = entry.host.tick();
            aggregate.busy |= tick.busy;
            aggregate.device_dead |= tick.device_dead;
            aggregate.wants_poll |= tick.wants_poll;
            if let Some(wake) = tick.wake_at {
                aggregate.wake_at = Some(match aggregate.wake_at {
                    Some(current) => current.min(wake),
                    None => wake,
                });
            }
        }
        aggregate
    }
}

/// 所有窗口 tick 的聚合结果。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Aggregate {
    pub busy: bool,
    pub device_dead: bool,
    pub wants_poll: bool,
    pub wake_at: Option<Instant>,
}

impl<B: Backend> ApplicationHandler for WindowSet<B> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(start) = self.on_start.take() {
            start(self, event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(key) = self
            .windows
            .iter()
            .find(|entry| entry.winit_id == Some(window_id))
            .map(|entry| entry.key)
        else {
            return;
        };
        let outcome = match self.windows.iter_mut().find(|entry| entry.key == key) {
            Some(entry) => entry.host.handle_event(event),
            None => return,
        };
        if outcome == HostEventOutcome::Close {
            self.close_window(key);
        }
        let mut exit = false;
        self.drain_commands(&mut exit);
        if exit {
            self.close_all();
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut exit = false;
        self.drain_commands(&mut exit);

        if let Some(mut handler) = self.on_tick.take() {
            handler(self, event_loop);
            self.on_tick = Some(handler);
        }
        self.drain_commands(&mut exit);

        if exit {
            self.close_all();
            event_loop.exit();
            return;
        }

        // P0.3：先排空跨线程投递（各宿主据此调 View::on_wake / on_message 并
        // 标脏），再聚合 tick —— 消息当帧可见，不会被 `aggregate_tick` 的
        // 「静止 → Wait」判断漏掉。
        for entry in &mut self.windows {
            entry.host.drain_posted();
        }

        let aggregate = self.aggregate_tick();

        if self.windows.is_empty() && self.shutdown == ShutdownMode::OnLastWindowClosed {
            event_loop.exit();
            return;
        }

        if aggregate.wants_poll || aggregate.busy {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else if let Some(wake) = aggregate.wake_at {
            event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
        } else if aggregate.device_dead {
            // 设备恢复在宿主的 `tick` 里按间隔重试，而空闲时事件循环不会自己醒。
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}
