//! 指针翻译：winit 的指针事件 → core 的 [`PointerEvent`]。
//!
//! # 为什么在这里
//!
//! 三个宿主（D3D11 / D3D12 / Vulkan）**互不依赖** —— 那是仓库的硬边界。
//! 但「winit 的 `WindowEvent::Touch` 怎么变成 core 的 `PointerEvent`」与
//! 后端**无关**，抄三份就意味着三份会漂移（键盘那条路已经漂移过一次：
//! Vulkan 宿主把 `key_events` 写死成空值，Linux 上键盘从未接通过，
//! 而没有任何东西会报错）。
//!
//! 触控这条路的漂移后果更严重：**触控完全不通**（三个宿主都不处理
//! `WindowEvent::Touch`），而 winit 又已经 `RegisterTouchWindow` ——
//! 那会**关掉 Windows 的「触摸提升为鼠标」**，于是触摸事件**既不是触摸
//! 也不是鼠标**，被整个丢掉。
//!
//! # 时刻从哪来
//!
//! winit 的 `Touch` 事件**不带时间戳**（`KeyboardInput` 才有）。
//! 而静态手势（轻点 / 双击 / 长按）是**计时**的 —— 所以翻译器自带一个
//! **单调**时钟（[`std::time::Instant`]）。
//!
//! 为什么不用系统时间：系统时间被改（对时、时区切换、夏令时）会让手势
//! 判据突然失效（「轻点」变成「双击」或反过来）。单调时钟没有这个问题。

use std::time::Instant;

use zephyra_core::input::{PointerEvent, PointerPhase, PointerSource};

/// 指针翻译器：winit 事件 → core 指针事件。
///
/// # 零分配
///
/// 它只持一个 [`Instant`] 与两个标量，构造一次之后**不再分配**。
#[derive(Debug, Clone)]
pub struct PointerTranslator {
    /// 单调时钟的起点（构造时刻）。
    start: Instant,
}

impl Default for PointerTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl PointerTranslator {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// 当前时刻（**单调**毫秒，自构造起算）。
    #[inline]
    pub fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// 一次触摸事件 → core 指针事件。
    ///
    /// # 相位映射
    ///
    /// | winit | core | 含义 |
    /// |---|---|---|
    /// | `Started` | [`PointerPhase::Down`] | 手指落下 |
    /// | `Moved` | [`PointerPhase::Move`] | 手指移动 |
    /// | `Ended` | [`PointerPhase::Up`] | 手指抬起 |
    /// | `Cancelled` | [`PointerPhase::Cancel`] | **被系统抢走**（异常结束）|
    ///
    /// ⚠ `Cancelled` **不是** `Ended`：官方指南把「指针取消」单列为一个事件
    /// ——「在**异常失去**触控交互时发生」。Windows 11 默认把三指 / 四指手势
    /// 交给系统，那时应用收到的就是取消。
    ///
    /// 若把它当抬起：用户还没抬手、系统就接管了 ⇒ 应用以为用户松手了 ⇒
    /// **误提交一次点击**。
    ///
    /// # 坐标单位
    ///
    /// `x` / `y` 是**物理像素**（与 core 里其它坐标同一套）。
    #[inline]
    pub fn touch(&self, phase: winit::event::TouchPhase, id: u64, x: f32, y: f32) -> PointerEvent {
        let phase = match phase {
            winit::event::TouchPhase::Started => PointerPhase::Down,
            winit::event::TouchPhase::Moved => PointerPhase::Move,
            winit::event::TouchPhase::Ended => PointerPhase::Up,
            winit::event::TouchPhase::Cancelled => PointerPhase::Cancel,
        };
        PointerEvent {
            id,
            x,
            y,
            phase,
            source: PointerSource::Touch,
            time_ms: self.now_ms(),
        }
    }

    /// 鼠标移动 → core 指针事件。
    #[inline]
    pub fn mouse_move(&self, x: f32, y: f32) -> PointerEvent {
        PointerEvent {
            // 鼠标只有一个指针 —— id 恒为 0。
            id: 0,
            x,
            y,
            phase: PointerPhase::Move,
            source: PointerSource::Mouse,
            time_ms: self.now_ms(),
        }
    }

    /// 鼠标按键（按下 / 抬起）→ core 指针事件。
    #[inline]
    pub fn mouse_button(&self, pressed: bool, x: f32, y: f32) -> PointerEvent {
        PointerEvent {
            id: 0,
            x,
            y,
            phase: if pressed {
                PointerPhase::Down
            } else {
                PointerPhase::Up
            },
            source: PointerSource::Mouse,
            time_ms: self.now_ms(),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/pointer.rs"]
mod tests;
