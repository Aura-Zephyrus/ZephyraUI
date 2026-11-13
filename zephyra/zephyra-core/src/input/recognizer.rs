//! 统一手势状态机：指针表 + 两指捏合 + 静态手势 + 操作数据，单入口
//! [`Recognizer::on_pointer`]。
//!
//! 每个 OS 指针事件恰好一次 O(MAX_POINTERS) 调用，产出 [`Gesture`]（`Copy`，
//! 零分配）。消费方无需再自行维护指针表或串联多个识别器。

use super::gesture::{StaticGesture, StaticGestures};
use super::manipulation::{ManipulationDelta, ManipulationState};
use super::pinch::{PinchDelta, PinchState};
use super::pointer::{PointerEvent, PointerPhase, PointerTable};

/// 单个指针事件的识别输出（`Copy`）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gesture {
    /// 捏合增量：Down 激活时为 1.0 锚点增量（消费方仅取 center）；
    /// Move 为增量比；Up 恒为 `None`。
    pub pinch: Option<PinchDelta>,
    /// 本次事件是否刚激活捏合（仅 Down 可能）。
    pub pinch_started: bool,
    /// 事件处理后的捏合状态。
    pub is_pinching: bool,
    /// 事件处理后的活跃指针数。
    pub pointers: u8,
    /// 静态手势（轻点 / 双击 / 长按）。
    ///
    /// 至多一个 —— 一次事件只可能构成一种静态手势。
    pub static_gesture: Option<StaticGesture>,
    /// 操作增量（多指平移 / 缩放 / 旋转 + 速度）。
    ///
    /// 单指时也有（就是那根手指的位移），所以**拖动**可以用它。
    pub manipulation: Option<ManipulationDelta>,
    /// 操作是否刚刚开始（第一根手指按下）。
    pub manipulation_started: bool,
    /// 操作是否刚刚结束（最后一根手指抬起），值是抬手瞬间的速度
    /// —— 惯性的初值（指南的 `ManipulationInertiaStarting`）。
    pub manipulation_ended: Option<(f32, f32)>,
}

/// 手势识别器：固定指针表 + 两指捏合 + 静态手势 + 操作数据。
#[derive(Clone)]
pub struct Recognizer {
    table: PointerTable,
    pinch: PinchState,
    gestures: StaticGestures,
    manipulation: ManipulationState,
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer {
    pub fn new() -> Self {
        Self {
            table: PointerTable::new(),
            pinch: PinchState::new(),
            gestures: StaticGestures::new(),
            manipulation: ManipulationState::new(),
        }
    }

    /// 当前是否处于捏合中。
    #[inline]
    pub fn is_pinching(&self) -> bool {
        self.pinch.is_pinching()
    }

    /// 当前活跃指针数。
    #[inline]
    pub fn pointer_count(&self) -> usize {
        self.table.len()
    }

    /// 当前是否处于操作中（≥1 指按住）。
    #[inline]
    pub fn is_manipulating(&self) -> bool {
        self.manipulation.is_active()
    }

    /// 每帧调用：检查长按是否到期。
    ///
    /// 长按是**时间驱动**的（手指不动时没有任何事件），所以只能靠时钟驱动。
    /// 没按下时立刻返回 —— 空闲零开销。
    #[inline]
    pub fn tick(&mut self, time_ms: u64) -> Option<StaticGesture> {
        self.gestures.tick(time_ms)
    }

    /// 单一入口：喂入一个指针事件，返回识别结果。
    ///
    /// - Down：登记指针；活跃 ≥2 时激活捏合并产出 1.0 锚点增量；
    /// - Move：仅更新已登记指针（未知 ID 忽略，与旧语义一致）；
    /// - Up：注销指针；捏合指抬起时结束，剩余 ≥2 指则无缝 regrab；
    /// - Cancel：注销指针，**不**产出轻点 / 点击类手势（见
    ///   [`PointerPhase::Cancel`]）。
    #[inline]
    pub fn on_pointer(&mut self, ev: PointerEvent) -> Gesture {
        let static_gesture = self.gestures.on_pointer(ev);
        let mut manipulation = None;
        let mut manipulation_started = false;
        let mut manipulation_ended = None;
        match ev.phase {
            PointerPhase::Down => {
                self.table.down(ev.id, ev.x, ev.y);
                let pinch = self.pinch.on_down(&self.table);
                if !self.manipulation.is_active() {
                    manipulation_started = true;
                }
                self.manipulation.on_down(&self.table, ev.time_ms);
                Gesture {
                    pinch,
                    pinch_started: pinch.is_some(),
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                    static_gesture,
                    manipulation,
                    manipulation_started,
                    manipulation_ended,
                }
            }
            PointerPhase::Move => {
                let tracked = self.table.move_pointer(ev.id, ev.x, ev.y);
                let pinch = if tracked {
                    self.pinch.on_move(&self.table, ev.id)
                } else {
                    None
                };
                if tracked {
                    manipulation = Some(self.manipulation.on_move(&self.table, ev.time_ms));
                }
                Gesture {
                    pinch,
                    pinch_started: false,
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                    static_gesture,
                    manipulation,
                    manipulation_started,
                    manipulation_ended,
                }
            }
            PointerPhase::Up => {
                let had = self.table.up(ev.id);
                if had {
                    self.pinch.on_up(&self.table, ev.id);
                }
                manipulation_ended = self.manipulation.on_up(&self.table);
                Gesture {
                    pinch: None,
                    pinch_started: false,
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                    static_gesture,
                    manipulation,
                    manipulation_started,
                    manipulation_ended,
                }
            }
            PointerPhase::Cancel => {
                let had = self.table.up(ev.id);
                if had {
                    self.pinch.on_up(&self.table, ev.id);
                }
                // ⚠ 取消**不**产出惯性：用户是被系统抢走的，不是「甩手」。
                // 续上惯性的后果是内容在用户没打算的情况下继续滑。
                self.manipulation.on_cancel(&self.table);
                Gesture {
                    pinch: None,
                    pinch_started: false,
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                    static_gesture,
                    manipulation,
                    manipulation_started,
                    manipulation_ended,
                }
            }
        }
    }

    /// 全量复位（窗口失焦等异常恢复路径用）。
    pub fn reset(&mut self) {
        self.table = PointerTable::new();
        self.pinch = PinchState::new();
        self.gestures = StaticGestures::new();
        self.manipulation = ManipulationState::new();
    }
}

#[cfg(test)]
#[path = "../../tests/unit/recognizer.rs"]
mod tests;
