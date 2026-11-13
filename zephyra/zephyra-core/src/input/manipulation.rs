//! 操作（manipulation）识别：多指平移 / 缩放 / 旋转 + 速度 + 惯性。
//!
//! # 为什么在核心里
//!
//! 官方触控指南把「操作事件」单列一节，并明确：「如果需要支持多个手指交互
//! 或要求使用高速数据的交互，请在应用中使用操作事件」。
//!
//! 而操作事件的**数据**与「是哪种交互」是**分开的** —— 指南原文：「操作事件
//! 提供的信息不会标识执行的交互形式，而是包括位置、平移增量和速度等数据。
//! 可以使用此触控数据来确定需要执行的交互类型。」
//!
//! 所以本模块只产出**原始数据**（平移增量 / 缩放比 / 转角 / 速度），
//! 「这是捏合还是旋转」由应用决定 —— 这正是指南要的那种分层。
//!
//! # 与捏合识别器（[`super::pinch`]）的关系
//!
//! [`super::pinch`] 是**两指**的窄口径缩放增量（增量语义 + 激活容差），
//! 本模块是**通用**的操作数据（任意指数 + 旋转 + 速度）。
//!
//! 两者并存而不是合并：捏合那条路的语义（增量比、批次门控、平方域容差）
//! 已经被现有应用依赖，而指南要的是「不标识交互形式」的原始数据。
//! 应用要缩放可以用捏合，要完整的手势（双指旋转、平移+缩放复合）用本模块。
//!
//! # 零分配
//!
//! 与指针表同款约束：状态是几个标量 + 两个指位，热路径不分配。

use super::pointer::PointerTable;

/// 操作增量（`Copy`，零分配）。
///
/// # 字段语义与指南一致
///
/// 「包括位置、平移增量和速度等数据」—— 所以这里**只有数据**，
/// 没有「本次是捏合」这种**形式**判断。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ManipulationDelta {
    /// 平移增量（屏幕 px）：所有活跃指针**质心**的位移。
    pub translation_x: f32,
    /// 平移增量（屏幕 px）。
    pub translation_y: f32,
    /// 缩放增量（比值）：参考指对的间距相对上一帧的变化（1.0 = 无缩放）。
    ///
    /// 只有 ≥2 指时才有意义；单指恒为 1.0。
    pub scale: f32,
    /// 旋转增量（弧度）：参考指对的连线相对上一帧的转角。
    ///
    /// 只有 ≥2 指时才有意义；单指恒为 0。
    pub rotation: f32,
    /// 质心当前位置（屏幕 px）。
    pub center_x: f32,
    /// 质心当前位置（屏幕 px）。
    pub center_y: f32,
    /// 平移速度（px/秒）。
    ///
    /// 用来在手指抬起后**续上惯性**（指南的 `ManipulationInertiaStarting`
    /// 携带的正是速度）。
    pub velocity_x: f32,
    /// 平移速度（px/秒）。
    pub velocity_y: f32,
}

impl ManipulationDelta {
    /// 无变化的增量（单指且没动）。
    pub const IDLE: ManipulationDelta = ManipulationDelta {
        translation_x: 0.0,
        translation_y: 0.0,
        scale: 1.0,
        rotation: 0.0,
        center_x: 0.0,
        center_y: 0.0,
        velocity_x: 0.0,
        velocity_y: 0.0,
    };

    /// 本次增量是否**什么都没变**。
    #[inline]
    pub fn is_idle(&self) -> bool {
        self.translation_x == 0.0
            && self.translation_y == 0.0
            && self.scale == 1.0
            && self.rotation == 0.0
    }
}

/// 「没有参考指针」的哨兵。
const NO_ID: u64 = u64::MAX;

/// 操作识别状态。
///
/// 状态量：参考指对、上一帧的质心 / 指距 / 转角 / 时刻。
#[derive(Clone)]
pub(crate) struct ManipulationState {
    /// 是否已开始（第一根手指按下即开始）。
    pub(crate) active: bool,
    /// 几何参考用的**两根指针**（按下最早的那两根）。
    ///
    /// # 为什么是「固定的一对」而不是「当前的任意两根」
    ///
    /// 缩放与旋转都要一个**连续**的参考量。若每次都取「当前的前两根」，
    /// 一根手指抬起、槽位被另一根补上时，参考量会**跳变** —— 于是一个
    /// 稳定的手势会突然冒出一大段缩放或转角。
    ///
    /// 固定一对指针就解决了这件事：手指的身份不变，参考量也不变。
    ///
    /// ⚠ 用「两指连线」算朝向（而不是点集的二阶矩）是有意的：二阶矩给出的是
    /// **轴**（周期 π），于是「转 180°」与「没转」分不出来 —— 而方向是有
    /// 2π 周期的。
    ref_ids: [u64; 2],
    /// 上一帧的质心。
    prev_center: (f32, f32),
    /// 上一帧的指距。
    prev_distance: f32,
    /// 上一帧的转角（弧度）。
    prev_angle: f32,
    /// 上一帧的时刻（ms）。
    prev_time_ms: u64,
    /// 上一帧的平移速度（px/秒），用于抬手时的惯性。
    last_velocity: (f32, f32),
    /// 本次操作**是否真的动过**。
    ///
    /// 判据用来决定抬手时要不要报「操作结束」：一次**原地**的点击（按下即抬起）
    /// 不算一次操作 —— 报「结束，���度 0」只是噪音。
    produced_delta: bool,
}

impl ManipulationState {
    pub(crate) fn new() -> Self {
        Self {
            active: false,
            ref_ids: [NO_ID; 2],
            prev_center: (0.0, 0.0),
            prev_distance: 0.0,
            prev_angle: 0.0,
            prev_time_ms: 0,
            last_velocity: (0.0, 0.0),
            produced_delta: false,
        }
    }

    /// 当前是否处于操作中（≥1 指按住）。
    #[inline]
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    /// 抬手瞬间的速度（px/秒）—— 惯性的初值。
    ///
    /// 只给**测试**看：生产路径上的速度由 [`Self::on_up`] 直接交出去
    /// （免得「抬手后速度被别处改了」这种不一致）。
    #[cfg(test)]
    #[inline]
    pub(crate) fn velocity(&self) -> (f32, f32) {
        self.last_velocity
    }

    /// 指针按下：开始（若尚未开始）。
    pub(crate) fn on_down(&mut self, table: &PointerTable, _time_ms: u64) {
        if !self.active {
            self.active = true;
            self.ref_ids = [NO_ID; 2];
            self.produced_delta = false;
        }
        self.anchor(table);
        self.reanchor(table);
    }

    /// 指针移动：返回本帧的操作增量。
    ///
    /// # 缩放与旋转为什么**相对上一帧**
    ///
    /// 指南的操作事件是**增量**语义（`ManipulationDelta` 携带「平移增量」），
    /// 而「缩放 / 旋转」同理必须是增量比 / 增量角 —— 否则应用无法把它接到
    /// 自己的变换上（累计变换需要的是增量）。
    pub(crate) fn on_move(&mut self, table: &PointerTable, time_ms: u64) -> ManipulationDelta {
        if !self.active {
            return ManipulationDelta::IDLE;
        }
        let (cx, cy) = centroid(table);
        let (d, a) = self.geometry(table);
        let dt_ms = time_ms.saturating_sub(self.prev_time_ms);
        let translation_x = cx - self.prev_center.0;
        let translation_y = cy - self.prev_center.1;
        // 缩放比：参考距离为 0（指距退化）时按「无缩放」处理。
        let scale = if self.prev_distance > 0.0 && d > 0.0 {
            d / self.prev_distance
        } else {
            1.0
        };
        // 转角差要归一到 (-π, π] —— 否则从 179° 转到 -179° 会被算成 -358°。
        let rotation = normalize_angle(a - self.prev_angle);
        // 速度：位移 / 时间。dt 为 0（同一毫秒内的两个事件）时沿用上一次的
        // 速度 —— 否则一次同毫秒的事件会把速度打成 0，手感上是「一顿」。
        let velocity = if dt_ms > 0 {
            (
                translation_x * 1000.0 / dt_ms as f32,
                translation_y * 1000.0 / dt_ms as f32,
            )
        } else {
            self.last_velocity
        };
        self.prev_center = (cx, cy);
        self.prev_distance = d;
        self.prev_angle = a;
        self.prev_time_ms = time_ms;
        self.last_velocity = velocity;
        let delta = ManipulationDelta {
            translation_x,
            translation_y,
            scale,
            rotation,
            center_x: cx,
            center_y: cy,
            velocity_x: velocity.0,
            velocity_y: velocity.1,
        };
        if !delta.is_idle() {
            self.produced_delta = true;
        }
        delta
    }

    /// 指针抬起：若已无活跃指针则结束操作。返回结束时的速度（惯性初值）。
    pub(crate) fn on_up(&mut self, table: &PointerTable) -> Option<(f32, f32)> {
        if !self.active {
            return None;
        }
        // 抬起的是参考指针 ⇒ 那个槽位空出来，重新锚定（否则参考量会跳变）。
        if table.position_of(self.ref_ids[0]).is_none() {
            self.ref_ids[0] = NO_ID;
        } else if table.position_of(self.ref_ids[1]).is_none() {
            self.ref_ids[1] = NO_ID;
        }
        if table.len() > 0 {
            // 还有手指按着 ⇒ 操作继续（重新锚定，避免用陈旧的参考量算增量）。
            self.anchor(table);
            self.reanchor(table);
            return None;
        }
        self.active = false;
        self.ref_ids = [NO_ID; 2];
        let v = self.last_velocity;
        self.last_velocity = (0.0, 0.0);
        // ⚠ **没动过**就不算一次操作 —— 一次原地点击不该报「操作结束，
        // 速度 0」，那只是噪音。
        if self.produced_delta {
            self.produced_delta = false;
            Some(v)
        } else {
            None
        }
    }

    /// 取消（系统接管手势 / 失焦）：结束操作，**不**产出惯性。
    ///
    /// ⚠ 取消时**不能**续惯性 —— 用户是被系统抢走的，不是「甩手」。
    /// 续上惯性的后果是：内容在用户没打算的情况下继续滑。
    pub(crate) fn on_cancel(&mut self, table: &PointerTable) {
        if table.len() > 0 {
            self.anchor(table);
            self.reanchor(table);
            return;
        }
        self.active = false;
        self.ref_ids = [NO_ID; 2];
        self.last_velocity = (0.0, 0.0);
    }

    /// 补齐参考指对里空着的槽位。
    fn anchor(&mut self, table: &PointerTable) {
        for slot in 0..2 {
            if self.ref_ids[slot] != NO_ID {
                continue;
            }
            // 跳过已被另一槽占用的指针。
            let taken = self.ref_ids[1 - slot];
            for (id, _, _) in table.iter() {
                if id != taken {
                    self.ref_ids[slot] = id;
                    break;
                }
            }
        }
    }

    /// 把「上一帧的参考量」重设成当前值（重新锚定时用）。
    fn reanchor(&mut self, table: &PointerTable) {
        let (cx, cy) = centroid(table);
        self.prev_center = (cx, cy);
        let (d, a) = self.geometry(table);
        self.prev_distance = d;
        self.prev_angle = a;
    }

    /// 参考指对的「指距」与「朝向」。
    fn geometry(&self, table: &PointerTable) -> (f32, f32) {
        if self.ref_ids[0] == NO_ID || self.ref_ids[1] == NO_ID {
            // 只有一根指针 ⇒ 没有指距也没有朝向。
            return (0.0, 0.0);
        }
        let (Some((x0, y0)), Some((x1, y1))) = (
            table.position_of(self.ref_ids[0]),
            table.position_of(self.ref_ids[1]),
        ) else {
            return (0.0, 0.0);
        };
        let dx = x1 - x0;
        let dy = y1 - y0;
        ((dx * dx + dy * dy).sqrt(), dy.atan2(dx))
    }
}

/// 活跃指针的质心。
fn centroid(table: &PointerTable) -> (f32, f32) {
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut n = 0.0;
    for (_, x, y) in table.iter() {
        sx += x;
        sy += y;
        n += 1.0;
    }
    if n == 0.0 {
        (0.0, 0.0)
    } else {
        (sx / n, sy / n)
    }
}

/// 角度归一到 `(-π, π]`。
fn normalize_angle(a: f32) -> f32 {
    use std::f32::consts::PI;
    let mut a = a;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a <= -PI {
        a += 2.0 * PI;
    }
    a
}

#[cfg(test)]
#[path = "../../tests/unit/manipulation.rs"]
mod tests;
