//! 跨线程唤醒与消息回投（P0.3）。
//!
//! [`WindowHandle`] 是线程亲和的（`Rc`），采集线程 / 文件对话框回调拿不到它。
//! [`AppWaker`] 则相反：`Send + Sync + Clone`，可以 `move` 进 `thread::spawn`。
//! 它持有事件循环的代理（只负责"醒一下"）和一条线程安全消息队列；真正的消息
//! 由宿主在 `about_to_wait` 排空，按投递顺序交给 `View::on_message`。

use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use winit::event_loop::EventLoopProxy;

/// 一次排空的结果：`wake` 的序号列表（对应 `View::on_wake`）与消息列表。
pub struct Drained {
    /// 本次唤醒的序号（逐条对应 `View::on_wake`）。
    pub wakes: Vec<u64>,
    /// 本次投递的消息（FIFO，逐条对应 `View::on_message`）。
    pub messages: Vec<Box<dyn Any + Send>>,
}

struct Inner {
    /// 代理只在 `WindowSet::run` 里能拿到；测试/单窗口兼容路径是无代理模式。
    proxy: Mutex<Option<EventLoopProxy<()>>>,
    posted: Mutex<VecDeque<Box<dyn Any + Send>>>,
    wakes: Mutex<VecDeque<u64>>,
    next_token: AtomicU64,
    alive: Arc<AtomicBool>,
}

/// 可跨线程克隆的唤醒器（P0.3）。
///
/// 窗口关闭后 `wake` / `post` 静默丢弃，不 panic —— 与 `WindowHandle` 的
/// "关掉的窗口再调方法不抛"一致。`Clone` 只增加 `Arc` 引用计数。
#[derive(Clone)]
pub struct AppWaker {
    inner: Arc<Inner>,
}

impl AppWaker {
    pub(crate) fn new(proxy: Option<EventLoopProxy<()>>, alive: Arc<AtomicBool>) -> Self {
        Self {
            inner: Arc::new(Inner {
                proxy: Mutex::new(proxy),
                posted: Mutex::new(VecDeque::new()),
                wakes: Mutex::new(VecDeque::new()),
                next_token: AtomicU64::new(0),
                alive,
            }),
        }
    }

    /// 只唤醒事件循环，不投消息。
    pub fn wake(&self) {
        if !self.inner.alive.load(Ordering::Relaxed) {
            return;
        }
        let token = self.inner.next_token.fetch_add(1, Ordering::Relaxed) + 1;
        self.inner.wakes.lock().unwrap().push_back(token);
        // EventLoopProxy 在 Windows 上是 Send 但未必 Sync —— 放进 Mutex 里就够了。
        // send_event 失败（循环已退出）时静默忽略，与「关闭后 post 静默丢弃」同律。
        if let Some(proxy) = self.inner.proxy.lock().unwrap().as_ref() {
            let _ = proxy.send_event(());
        }
    }

    /// 唤醒事件循环并投递一条消息，供 `View::on_message` 消费。
    ///
    /// `T` 必须 `Send + 'static`（消息要跨线程搬运），不要求 `Sync`（只在一个
    /// 线程里被取出并 `&mut` 使用）。
    pub fn post<T: Any + Send>(&self, msg: T) {
        if !self.inner.alive.load(Ordering::Relaxed) {
            return;
        }
        self.inner.posted.lock().unwrap().push_back(Box::new(msg));
        self.wake();
    }

    /// 宿主在 `about_to_wait` 调用：取走唤醒序号与消息（FIFO）。
    pub fn drain(&self) -> Drained {
        let wakes = self.inner.wakes.lock().unwrap().drain(..).collect();
        let messages = self.inner.posted.lock().unwrap().drain(..).collect();
        Drained { wakes, messages }
    }
}
