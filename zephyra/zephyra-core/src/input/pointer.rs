//! 指针事件词汇表与固定容量指针表。
//!
//! 设计约束（性能预算）：
//! - [`PointerEvent`] 为 32 字节 `Copy` 值，按值传递零开销；
//! - 指针表为固定 8 槽数组：无 `HashMap`、无堆分配、无哈希计算，缓存友好；
//!   单次事件处理为 O(MAX_POINTERS) 线性扫描（≤8 次比较，分支可预测）。
//!
//! # 与官方触控指南的对应
//!
//! 指针事件取自 Windows 触控交互开发人员指南的「指针事件」一节：
//! **按下 / 抬起 / 移动 / 进入 / 退出 / 取消 / 捕获丢失 / 滚轮**。
//! 本模块覆盖其中「按下 / 抬起 / 移动 / 取消」四类 —— 「进入 / 退出」在
//! 保留式 UI 里由命中测试的悬停变化表达（「捕获丢失」由路由器的按下捕获
//! 语义表达），滚轮走另一条通道（[`crate::input::router`] 之外的 scroll）。
//!
//! 指南的两条硬要求落在本模块上：
//!
//! 1. **触控是双状态的**（没有悬停）—— 见 [`PointerSource::Touch`]。
//! 2. **取消必须区别于抬起** —— 见 [`PointerPhase::Cancel`]。

/// 指针来源（OS 层判定）。
///
/// # 触控没有悬停
///
/// 指南：「触控使用双状态模式：显示设备的触控表面要么处于触控状态（打开），
/// 要么处于非触控状态（关闭）。**没有可触发其他视觉反馈的悬停状态**。」
///
/// 而鼠标/笔是三状态（向上 / 向下 / 悬停）。所以「这个来源能不能产生悬停」
/// 不是样式问题而是**输入语义**问题 —— 触控按下时给的是**按下**反馈，
/// 不是悬停反馈。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerSource {
    /// 触摸屏。
    ///
    /// 没有悬停状态；也没有「右键」—— 右键由系统手势产生，不走这里。
    Touch,
    /// 笔 / 触笔。
    ///
    /// 三状态（悬停 + 按下），但**笔尖接触前**的悬停应当被应用忽略
    /// （笔尖落下时不应触发「悬停 ⇒ 点击」这类副作用）。
    Pen,
    /// 鼠标。
    Mouse,
}

impl PointerSource {
    /// 这个来源**能不能**产生悬停状态。
    ///
    /// 触控不能（双状态，见 [`PointerSource`] 的文档）。
    #[inline]
    pub fn can_hover(self) -> bool {
        !matches!(self, PointerSource::Touch)
    }
}

/// 指针相位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    /// 交互被**异常取消**。
    ///
    /// # 为什么必须有它
    ///
    /// 指南把「指针取消」列为指针事件之一：「在**异常失去**触控交互时发生」。
    /// Windows 11 默认把三指 / 四指手势交给系统（窗口切换、虚拟桌面…）——
    /// 那时应用收到的是**取消**，不是抬起。
    ///
    /// ⚠️ **取消不是抬起**。若把它当抬起处理：手指还没抬起、系统就接管了
    /// 手势 ⇒ 应用以为用户松手了 ⇒ **误提交一次点击**（按下的按钮被点掉）。
    ///
    /// 正确的语义是「**这一下不算数**」：清掉按下态、**不**产出点击。
    Cancel,
}

/// 单个指针事件（`Copy`，按值传递零开销）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    /// 指针身份。同一根手指在按下→移动→抬起之间**不变**。
    ///
    /// ⚠️ 触控的 id **允许多点并存**（每根手指一个）—— 这正是多点触控的
    /// 基础。鼠标恒为 0。
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub phase: PointerPhase,
    pub source: PointerSource,
    /// 事件发生时刻（**单调**毫秒）。
    ///
    /// # 为什么指针事件要带时刻
    ///
    /// 静态手势（轻点 / 双击 / 长按）是**计时**的 —— 指南把它们列进 Windows
    /// 的标准手势集（Tap / DoubleTap / Holding）。计时判据必须有一个**统一**
    /// 的时钟：宿主各有各的时钟（`Instant` / `GetTickCount`），而识别器住在
    /// 核心里、不认识宿主。把时刻放进事件里，识别器就不必知道宿主是谁。
    ///
    /// 单调时钟的另一个理由：系统时间被改（对时、时区切换）不该让手势判据
    /// 突然失效。
    pub time_ms: u64,
}

/// 同时跟踪的指针上限。触控设备实际 ≤10 点；手势消费 ≤2 点。
pub const MAX_POINTERS: usize = 8;

#[derive(Clone, Copy)]
struct Slot {
    id: u64,
    x: f32,
    y: f32,
}

/// 固定容量指针表：以 O(n)（n ≤ 8）数组扫描替代 `HashMap`，
/// 热路径零哈希、零分配、零 rehash。
#[derive(Clone)]
pub(crate) struct PointerTable {
    slots: [Option<Slot>; MAX_POINTERS],
    count: u8,
}

impl PointerTable {
    pub(crate) fn new() -> Self {
        Self {
            slots: [None; MAX_POINTERS],
            count: 0,
        }
    }

    /// 当前活跃指针数（维护的计数，O(1)）。
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.count as usize
    }

    /// Down：同 ID 原位覆盖（缺失 Up 的重复 Down），否则占用空槽；表满时忽略（防御兜底）。
    pub(crate) fn down(&mut self, id: u64, x: f32, y: f32) {
        // 先查同 ID（避免两遍扫描顺序导致同 ID 重复占槽）
        for slot in self.slots.iter_mut() {
            if let Some(s) = *slot {
                if s.id == id {
                    *slot = Some(Slot { id, x, y });
                    return;
                }
            }
        }
        for slot in self.slots.iter_mut() {
            if slot.is_none() {
                *slot = Some(Slot { id, x, y });
                self.count += 1;
                return;
            }
        }
    }

    /// Move：仅当指针已在表中才更新（未知 ID 的 Move 被忽略，与旧语义一致）。
    #[inline]
    pub(crate) fn move_pointer(&mut self, id: u64, x: f32, y: f32) -> bool {
        for slot in self.slots.iter_mut() {
            if let Some(s) = *slot {
                if s.id == id {
                    *slot = Some(Slot { id, x, y });
                    return true;
                }
            }
        }
        false
    }

    /// Up / Cancel：移除指针；返回该 ID 是否存在（不存在则调用方跳过手势通知）。
    pub(crate) fn up(&mut self, id: u64) -> bool {
        for slot in self.slots.iter_mut() {
            if let Some(s) = *slot {
                if s.id == id {
                    *slot = None;
                    self.count -= 1;
                    return true;
                }
            }
        }
        false
    }

    /// 按 ID 查询位置。
    #[inline]
    pub(crate) fn position_of(&self, id: u64) -> Option<(f32, f32)> {
        for s in self.slots.iter().flatten() {
            if s.id == id {
                return Some((s.x, s.y));
            }
        }
        None
    }

    /// 第 n 个活跃指针（槽位序，确定性）。n 越界 → `None`。
    #[inline]
    pub(crate) fn nth(&self, n: usize) -> Option<(u64, f32, f32)> {
        self.slots.iter().flatten().nth(n).map(|s| (s.id, s.x, s.y))
    }

    /// 全部活跃指针（槽位序，确定性）。
    #[inline]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u64, f32, f32)> + '_ {
        self.slots.iter().flatten().map(|s| (s.id, s.x, s.y))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/pointer.rs"]
mod tests;
