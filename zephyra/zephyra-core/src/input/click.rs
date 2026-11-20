//! 跨帧的「按下 → 抬起」点击还原。
//!
//! # 为什么判据在框架里
//!
//! 宿主给应用的 `pressed` 只是「**这一帧**按住谁」，而 `View::on_input` 每帧只调一次。
//! 「点击」这件事跨越两帧（上帧按着、这帧松了），所以每个消费者都得自己记一份上帧
//! 状态才能还原。在没有通用判据之前，gallery、无障碍桥、以及 UIA 的测试各抄了一遍 ——
//! 而「取消不是抬起」「多指按不同控件时先抬那个也要算点击」这些边界最容易被抄漏。
//!
//! 本模块把这段判据收进核心：纯函数 [`clicks_from`] 给需要自己管状态的场景，
//! [`ClickTracker`] 给只想拿点击的场景。

use crate::id::ElementKey;

/// 从两帧的按下集合还原**本帧**产生的点击。
///
/// - `last`：上一帧按着的 key；
/// - `now`：这一帧按着的 key；
/// - `cancelled`：本帧是否发生了指针取消。
///
/// 返回「上一帧按着、这一帧不再按着」的 key，顺序与 `last` 一致。
///
/// # 边界
///
/// - **取消不是抬起**：系统抢走手势时 `pressed` 同样被清成空，但用户手指还在屏幕上。
///   不区分就会误触发一次点击，所以 `cancelled == true` 时整帧不产生点击。
/// - **多指用集合差**：两指各按一个控件、先抬的那根也要算一次点击 —— 判据必须是
///   `last - now`，而不是「上帧那一个 key」。
pub fn clicks_from(last: &[ElementKey], now: &[ElementKey], cancelled: bool) -> Vec<ElementKey> {
    if cancelled {
        return Vec::new();
    }
    last.iter().filter(|k| !now.contains(k)).cloned().collect()
}

/// 跨帧点击状态持有者。
///
/// 只持有一份 `last`：每帧 [`Self::update`] 喂入当前按下集合，返回本帧的点击。
/// 应用若已经自己存了 `last`（比如要画按下态），直接用 [`clicks_from`] 即可，不必
/// 再存一份。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ClickTracker {
    last: Vec<ElementKey>,
}

impl ClickTracker {
    /// 喂入本帧状态，返回本帧新产生的点击。
    pub fn update(&mut self, now: &[ElementKey], cancelled: bool) -> Vec<ElementKey> {
        let clicks = clicks_from(&self.last, now, cancelled);
        self.last = now.to_vec();
        clicks
    }

    /// 上一帧喂入的按下集合（供画「刚松开」反馈用）。
    pub fn last_pressed(&self) -> &[ElementKey] {
        &self.last
    }
}

#[cfg(test)]
#[path = "../../tests/unit/input/click.rs"]
mod tests;
