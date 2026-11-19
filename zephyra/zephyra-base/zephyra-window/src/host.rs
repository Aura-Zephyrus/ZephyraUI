//! 单个窗口的宿主契约。
//!
//! 每个后端实现一次；[`crate::WindowSet`] 只认这个 trait，不认具体后端。

use std::any::Any;
use std::time::Instant;

use winit::event::WindowEvent;
use winit::window::WindowId;

use zephyra_core::styler::Styler;
use zephyra_core::theme::ThemeVariant;

use crate::spec::WindowKey;

/// 一次 `window_event` 的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEventOutcome {
    /// 窗口继续存活。
    Continue,
    /// 窗口要求关闭（`CloseRequested`，或应用在事件里投了 Close 命令）。
    Close,
}

/// 一次 `tick` 的结论。
///
/// # 为什么把控制流"上提"
///
/// 单窗口时每个宿主自己 `event_loop.set_control_flow(...)` 没问题。多窗口时
/// N 个窗口会互相覆盖 —— 最后一个说话的那个决定整个循环，而它可能恰好是
/// 空闲的那个。所以宿主只**报告**自己的需要，由 [`crate::WindowSet`] 聚合后
/// 设置一次。
#[derive(Debug, Clone, Copy, Default)]
pub struct HostTick {
    /// 本 tick 真的画了一帧（仅用于 trace / 统计）。
    pub painted: bool,
    /// 还有待处理的工作（脏标记 / profiler / 无障碍动作）。
    pub busy: bool,
    /// 设备不可用；需要按间隔唤醒重试。
    pub device_dead: bool,
    /// 调试桥有客户端：要求连续轮询。
    pub wants_poll: bool,
    /// 没有别的事情时，最早该被唤醒的时刻。
    pub wake_at: Option<Instant>,
}

/// 单个窗口的宿主。
///
/// 实现者负责：交换链 / DComp visual / 后缓冲、`FrameCache`、`View`、
/// 输入与焦点状态、以及本窗口的离屏渲染目标。
pub trait WindowHost {
    /// 本窗口的稳定标识。
    fn key(&self) -> WindowKey;

    /// 本窗口 id 的 `u64` 表示（P1.1：多窗口 ext hook 精确匹配用）。
    ///
    /// 默认由 [`Self::key`] 派生；真实宿主无需单独存储。
    fn window_id(&self) -> u64 {
        self.key().as_u64()
    }

    /// winit 的窗口 id（事件分发用）。
    ///
    /// `Option` 只为测试替身：真实宿主恒返回 `Some`。
    fn winit_id(&self) -> Option<WindowId>;

    /// 处理一个窗口事件。
    fn handle_event(&mut self, event: WindowEvent) -> HostEventOutcome;

    /// 排空跨线程投递（P0.3），把消息交给 `View`；返回是否有状态变化需要重画。
    ///
    /// 默认空实现：测试替身与无桥后端无需处理。真实宿主持有 `WindowHandle`，
    /// 从其 `AppWaker` 取走消息并按序调用 `View::on_wake` / `View::on_message`。
    fn drain_posted(&mut self) -> bool {
        false
    }

    /// 跑一次"该画就画"。
    fn tick(&mut self) -> HostTick;

    /// 窗口创建完成后的首次启动（补第一帧、装 live-resize / UIA 子类）。
    ///
    /// 由 [`crate::WindowSet`] 在宿主被放进 `Box` **之后**调用 —— 子类化钩子
    /// 持有指向宿主的裸指针，地址必须先稳定。
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// 切换主题。
    fn set_theme(&mut self, variant: ThemeVariant);

    /// 安装样式注入点。
    fn set_styler(&mut self, styler: Box<dyn Styler>);

    /// 设置标题。
    fn set_title(&mut self, title: &str);

    /// 请求重绘。
    fn request_redraw(&self);

    /// 请求关闭（由 [`crate::WindowSet`] 在收到 Close 命令时调用）。
    fn request_close(&mut self);

    /// 窗口销毁前的收尾（摘钩子、摘 UIA）。
    fn teardown(&mut self) {}

    /// 向下转型（给需要具体视图类型的应用用）。
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
