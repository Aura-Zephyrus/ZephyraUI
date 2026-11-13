//! 静态手势识别：轻点（Tap）/ 双击（DoubleTap）/ 长按（Holding）。
//!
//! # 官方指南里的位置
//!
//! 触控指南把手势分成三档：
//!
//! | 档 | 事件 | 本模块 |
//! |---|---|---|
//! | 静态手势 | Tap / DoubleTap / RightTap / Holding | ✅ 本模块（RightTap 见下） |
//! | 指针事件 | Pressed / Released / Moved / … | [`super::pointer`] |
//! | 操作事件 | ManipulationStarted / Delta / Inertia / Completed | [`super::manipulation`] |
//!
//! # 计时判据的取舍
//!
//! 指南说「尽量不要使用计时交互（适当用例：**长按**）」，同时又明确把
//! Tap / DoubleTap / Holding 列进 Windows 的标准手势集。所以：
//!
//! - **轻点**（Tap）不是「计时交互」—— 它是「按下与抬起落在同一处」的
//!   **空间**判据；时长只是**上限**（按住不动超过上限就不再是轻点）。
//! - **双击**（DoubleTap）天然是计时的 —— 两次轻点的间隔窗口。
//! - **长按**（Holding）是指南明确认可的计时交互。
//!
//! # 为什么不按「手指数量」区分
//!
//! 指南：「不应按所用手指数来区分操作。」所以本模块**只跟踪一根**主指针
//! （最先按下的那根），多指时**不**产生轻点 —— 但也**不**因为「多指」而
//! 改变判据；多指的操作数据走 [`super::manipulation`]。
//!
//! # 阈值
//!
//! 数值取自 WinUI 的默认手势参数（那是 Windows 平台自己认的数）：
//!
//! - 轻点位移容差 **10 px**：指尖抖动不该让轻点变成「拖」。
//! - 轻点时长上限 **500 ms**：与长按阈值同值（超过就不是轻点了）。
//! - 长按时长 **500 ms**。
//! - 双击间隔 **300 ms**，双击位移容差 **24 px**（比轻点宽 ——
//!   两次轻点之间手会抖得更厉害）。
//!
//! ⚠️ 阈值是**物理像素**。指南的「接触几何」一节指出：指尖的接触区域
//! 大于单一 x-y 坐标 —— 所以容差就是那圈「手指比针尖粗」的补偿。

use super::pointer::{PointerEvent, PointerPhase};

/// 轻点位移容差（物理 px）。
pub const TAP_SLOP_PX: f32 = 10.0;

/// 轻点时长上限（ms）；超过即不是轻点。
pub const TAP_MAX_DURATION_MS: u64 = 500;

/// 长按时长（ms）。
pub const HOLD_DURATION_MS: u64 = 500;

/// 双击间隔窗口（ms）。
pub const DOUBLE_TAP_INTERVAL_MS: u64 = 300;

/// 双击两次落点之间的最大距离（px）。
pub const DOUBLE_TAP_SLOP_PX: f32 = 24.0;

/// 一次静态手势的产出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticGesture {
    /// 轻点（按下与抬起落在同一处，且未超时）。
    Tapped,
    /// 双击（两次轻点，间隔与距离都在窗口内）。
    DoubleTapped,
    /// 长按（按住超过 [`HOLD_DURATION_MS`] 且没挪动）。
    Holding,
}

/// 静态手势状态机。
#[derive(Clone, Default)]
pub(crate) struct StaticGestures {
    /// 主指针按下时的状态。
    active: Option<Active>,
    /// 上一次轻点（用于双击判定）。
    last_tap: Option<TapRecord>,
    /// 本次按下已经报过长按（长按只报一次）。
    held_reported: bool,
}

#[derive(Clone, Copy)]
struct Active {
    id: u64,
    x: f32,
    y: f32,
    down_ms: u64,
    /// 是否已经挪出了容差（挪出后不再是轻点/长按候选）。
    moved: bool,
}

#[derive(Clone, Copy)]
struct TapRecord {
    x: f32,
    y: f32,
    up_ms: u64,
}

impl StaticGestures {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 喂一个指针事件，返回它产出的静态手势（至多一个）。
    pub(crate) fn on_pointer(&mut self, ev: PointerEvent) -> Option<StaticGesture> {
        match ev.phase {
            PointerPhase::Down => {
                // 新按下开始跟踪。⚠ **已有主指针时不换**：多指场景下第二根手指
                // 不能顶掉第一根 —— 否则「第一根按住、第二根点了别处」会把
                // 主指针换掉，第一根的轻点就丢了。
                if self.active.is_none() {
                    self.active = Some(Active {
                        id: ev.id,
                        x: ev.x,
                        y: ev.y,
                        down_ms: ev.time_ms,
                        moved: false,
                    });
                }
                self.held_reported = false;
                None
            }
            PointerPhase::Move => {
                if let Some(active) = self.active.as_mut() {
                    if active.id == ev.id && !active.moved {
                        let dx = ev.x - active.x;
                        let dy = ev.y - active.y;
                        if (dx * dx + dy * dy).sqrt() > TAP_SLOP_PX {
                            active.moved = true;
                        }
                    }
                }
                None
            }
            PointerPhase::Up => {
                let active = self.active.take()?;
                if active.id != ev.id {
                    // 不是主指针抬起（多指场景）：主指针还在，别动它。
                    self.active = Some(active);
                    return None;
                }
                // 取消/长按都不算轻点。
                if active.moved {
                    self.last_tap = None;
                    return None;
                }
                let duration = ev.time_ms.saturating_sub(active.down_ms);
                if duration > TAP_MAX_DURATION_MS {
                    // 按太久 ⇒ 这是长按，不是轻点。双击窗口也要断掉。
                    self.last_tap = None;
                    return None;
                }
                // 双击判定：上一次轻点存在、间隔在窗口内、两次落点够近。
                let is_double = match self.last_tap {
                    Some(prev) => {
                        let dt = ev.time_ms.saturating_sub(prev.up_ms);
                        let dx = ev.x - prev.x;
                        let dy = ev.y - prev.y;
                        dt <= DOUBLE_TAP_INTERVAL_MS
                            && (dx * dx + dy * dy).sqrt() <= DOUBLE_TAP_SLOP_PX
                    }
                    None => false,
                };
                if is_double {
                    // 双击吃掉这次轻点 —— 不重复报两次。
                    self.last_tap = None;
                    Some(StaticGesture::DoubleTapped)
                } else {
                    self.last_tap = Some(TapRecord {
                        x: ev.x,
                        y: ev.y,
                        up_ms: ev.time_ms,
                    });
                    Some(StaticGesture::Tapped)
                }
            }
            PointerPhase::Cancel => {
                // 取消 ⇒ 这一下**不算数**：清掉轻点窗口（用户是被系统抢走的，
                // 后续的轻点与它无关）。
                if self.active.map(|a| a.id == ev.id).unwrap_or(false) {
                    self.active = None;
                    self.held_reported = false;
                    self.last_tap = None;
                }
                None
            }
        }
    }

    /// 每帧调用：检查主指针是否已经按住够久，够久则报一次长按。
    ///
    /// # 为什么要有这个「每帧」入口
    ///
    /// 长按是**时间到了**才发生的事，而「时间到了」那一刻**没有任何输入事件**
    /// —— 用户的手指一动不动。所以它不能靠事件驱动，只能靠**时钟**驱动。
    ///
    /// 宿主每帧调一次（零成本：没按下时立刻返回）。
    pub(crate) fn tick(&mut self, time_ms: u64) -> Option<StaticGesture> {
        if self.held_reported {
            return None;
        }
        let active = self.active.as_ref()?;
        if active.moved {
            return None;
        }
        if time_ms.saturating_sub(active.down_ms) < HOLD_DURATION_MS {
            return None;
        }
        self.held_reported = true;
        Some(StaticGesture::Holding)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/gesture.rs"]
mod tests;
