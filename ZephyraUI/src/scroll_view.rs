//! 滚动容器（ScrollView）控件的状态模型。
//!
//! # 蓝本（WinUI `ScrollViewer` / Android `ScrollView` / CSS `overflow: scroll`）
//!
//! 滚动容器允许内容超出视口时通过拖动或滚轮查看隐藏部分。它不参与布局流的
//! 尺寸分配 —— 容器按**视口**尺寸（显式 `width` / `height`），子内容按**内容**
//! 尺寸，超出部分通过 `scroll_offset` 控制可见区域。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 滚动偏移、内容尺寸、视口尺寸、拖动/滚轮处理、钳位 | 本模块（`ScrollView`） |
//! | 结构（节点树、分类名、滚动条槽位） | `ui/controls.zypui` |
//! | 颜色与尺寸（轨道、thumb、圆角、背景） | `controls-*.zstyle` |
//! | `scroll_offset` 的**存留**（写回宿主状态） | 宿主应用 |
//!
//! # 坐标系契约
//!
//! - `scroll_offset` 是**内容相对于视口的偏移**。`scroll_offset = (0, 0)` 时
//!   内容左上角与视口左上角重合；`scroll_offset.y > 0` 表示内容**向上**平移
//!   （用户向下滚了，看到的是内容的下半段）。
//! - 绘制趟（`paint::PaintState::descend_children`）按 `-scroll_offset` 平移
//!   **子节点**；命中测试（`crate::hit`）走同一条规则（`visual::enter`），
//!   所以「看得见的」与「点得到的」永远是同一批元素。核心层已处理，
//!   本模块只负责维护 `scroll_offset` 的值。
//! - `scroll_offset` 被钳位到 `[0, max_scroll]`，其中
//!   `max_scroll = content_size - viewport_size`（各轴独立，负值视为 0）。
//!
//! # 钳位为什么住在控件层而不是核心层
//!
//! 核心层不知道「内容有多大」—— 它只负责把偏移施加到子节点上。内容尺寸是
//! **布局**的产物，而布局在核心层、控件状态在控件层，两边不共享数据。
//! 于是钳位只能由持有内容尺寸的那一侧做，也就是这里。核心层拿到的偏移
//! 永远是「已经钳位过」的（宿主写回时用的就是本模块的 `scroll_offset()`）。

use zephyra_core::core::Point;

/// 滚动容器的状态模型。
#[derive(Debug, Clone)]
pub struct ScrollView {
    key: &'static str,
    /// 是否禁用（禁用时不可拖动、不可滚轮）。
    disabled: bool,
    /// 内容尺寸（DIP）。由宿主根据实际内容设置。
    content_size: Point,
    /// 视口尺寸（DIP）。由宿主根据容器布局矩形设置。
    viewport_size: Point,
    /// 当前滚动偏移（DIP）。每轴独立钳位到 `[0, max_scroll]`。
    scroll_offset: Point,
    /// 是否正在拖拽（按下且移动超过 slop）。
    dragging: bool,
    /// 拖拽开始时的鼠标位置（DIP，屏幕/客户区坐标）。
    drag_start_pos: Point,
    /// 拖拽开始时的 `scroll_offset`（DIP）。
    drag_start_offset: Point,
    /// 上一帧的鼠标位置（DIP）。越过 slop 之后按**增量**累加。
    last_pos: Point,
    /// X 轴是否已越过 slop（越过之前该轴一动不动）。
    axis_active_x: bool,
    /// Y 轴是否已越过 slop。
    axis_active_y: bool,
    /// X 轴累计的有效位移（DIP，已扣掉 slop）。
    acc_x: f32,
    /// Y 轴累计的有效位移（DIP，已扣掉 slop）。
    acc_y: f32,
}

/// 桌面指针点击抖动的拖拽分界（DIP）。净位移越过本阈值之前，滚动**完全不动**，
/// 避免点击被误判为拖动。
///
/// 4 DIP 与 [`crate::toggle::KNOB_DRAG_SLOP`] / [`crate::slider::DRAG_SLOP`] 同值：
/// 实测桌面指针单点击抖 ≤3 物理像素（125% 缩放下 ≈2.4 DIP），4 DIP 能吞掉它，
/// 而真想拖的人会移动远超 4 DIP。
pub const SCROLL_DRAG_SLOP: f32 = 4.0;

/// 滚轮每格的滚动增量（DIP）。
///
/// 一格 = 48 DIP 是 Windows 的标准滚轮步长（`WHEEL_DELTA` = 120，
/// 一格三行，所以每行 40；这里取 48 是为了在触控板上更跟手）。
pub const SCROLL_WHEEL_DELTA: f32 = 48.0;

impl ScrollView {
    /// 建一个空滚动容器（内容尺寸和视口尺寸均为 0）。
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            disabled: false,
            content_size: Point::ZERO,
            viewport_size: Point::ZERO,
            scroll_offset: Point::ZERO,
            dragging: false,
            drag_start_pos: Point::ZERO,
            drag_start_offset: Point::ZERO,
            last_pos: Point::ZERO,
            axis_active_x: false,
            axis_active_y: false,
            acc_x: 0.0,
            acc_y: 0.0,
        }
    }

    /// 滚动容器的稳定 key（事件路由身份）。
    pub const fn key(&self) -> &'static str {
        self.key
    }

    /// 是否禁用。
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 设置禁用。
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// 内容尺寸（DIP）。
    pub const fn content_size(&self) -> Point {
        self.content_size
    }

    /// 设置内容尺寸。设置后会重新钳位 `scroll_offset`。
    pub fn set_content_size(&mut self, size: Point) {
        self.content_size = Point::new(finite(size.x), finite(size.y));
        self.scroll_offset = self.clamp_point(self.scroll_offset);
    }

    /// 视口尺寸（DIP）。
    pub const fn viewport_size(&self) -> Point {
        self.viewport_size
    }

    /// 设置视口尺寸。设置后会重新钳位 `scroll_offset`。
    pub fn set_viewport_size(&mut self, size: Point) {
        self.viewport_size = Point::new(finite(size.x), finite(size.y));
        self.scroll_offset = self.clamp_point(self.scroll_offset);
    }

    /// 当前滚动偏移（DIP）。
    pub const fn scroll_offset(&self) -> Point {
        self.scroll_offset
    }

    /// 设置滚动偏移（DIP）。会被钳位到有效范围。
    pub fn set_scroll_offset(&mut self, offset: Point) {
        self.scroll_offset = self.clamp_point(offset);
    }

    /// 某轴的最大可滚动距离（DIP）。`content_size - viewport_size`，负值视为 0。
    fn max_scroll(&self, axis_max: f32, viewport_max: f32) -> f32 {
        (axis_max - viewport_max).max(0.0)
    }

    /// X 轴最大可滚动距离（DIP）。
    pub fn max_scroll_x(&self) -> f32 {
        self.max_scroll(self.content_size.x, self.viewport_size.x)
    }

    /// Y 轴最大可滚动距离（DIP）。
    pub fn max_scroll_y(&self) -> f32 {
        self.max_scroll(self.content_size.y, self.viewport_size.y)
    }

    /// 将偏移钳位到有效范围 `[0, max_scroll]`（各轴独立）。
    fn clamp_point(&self, p: Point) -> Point {
        Point::new(
            self.clamp_axis(p.x, self.content_size.x, self.viewport_size.x),
            self.clamp_axis(p.y, self.content_size.y, self.viewport_size.y),
        )
    }

    fn clamp_axis(&self, v: f32, content: f32, viewport: f32) -> f32 {
        let max = self.max_scroll(content, viewport);
        if !v.is_finite() {
            return 0.0;
        }
        v.clamp(0.0, max)
    }

    /// 内容是否在 X 轴可滚动（内容宽度 > 视口宽度）。
    pub fn can_scroll_x(&self) -> bool {
        self.content_size.x > self.viewport_size.x
    }

    /// 内容是否在 Y 轴可滚动（内容高度 > 视口高度）。
    pub fn can_scroll_y(&self) -> bool {
        self.content_size.y > self.viewport_size.y
    }

    /// 滚轮滚动。`delta_y` 正数表示**向下滚**（内容向上移，`scroll_offset.y` 增大）。
    /// 返回是否真的滚动了。
    pub fn scroll_wheel(&mut self, delta_y: f32) -> bool {
        if self.disabled || !delta_y.is_finite() {
            return false;
        }
        let delta = delta_y * SCROLL_WHEEL_DELTA;
        let max_y = self.max_scroll_y();
        let new_y = (self.scroll_offset.y + delta).clamp(0.0, max_y);
        let changed = (new_y - self.scroll_offset.y).abs() > f32::EPSILON;
        self.scroll_offset.y = new_y;
        changed
    }

    /// 按指定的**绝对**偏移滚动（DIP，已钳位）。滚动条拖动走这条路。
    pub fn scroll_to(&mut self, offset: Point) -> bool {
        let clamped = self.clamp_point(offset);
        let changed = (clamped.x - self.scroll_offset.x).abs() > f32::EPSILON
            || (clamped.y - self.scroll_offset.y).abs() > f32::EPSILON;
        self.scroll_offset = clamped;
        changed
    }

    /// 开始拖拽。记录按下点和当前偏移，作为后续增量的基准。
    pub fn drag_start(&mut self, pos: Point) {
        if self.disabled {
            return;
        }
        self.dragging = true;
        self.drag_start_pos = pos;
        self.drag_start_offset = self.scroll_offset;
        self.last_pos = pos;
        self.axis_active_x = false;
        self.axis_active_y = false;
        self.acc_x = 0.0;
        self.acc_y = 0.0;
    }

    /// 拖拽更新。根据鼠标位移更新 `scroll_offset`，并钳位。
    /// 返回是否真的滚动了。
    ///
    /// # slop 的处理（逐轴）
    ///
    /// 每个轴**独立**越过 slop：某轴的净位移不足 [`SCROLL_DRAG_SLOP`] 时该轴
    /// 一动不动（点击不该被当成拖动）；越过的那一刻**只应用超出 slop 的部分**
    /// （Compose 的 `overSlop` 同款），之后按增量累加。
    ///
    /// 逐轴而不是整体：斜着拖时两个轴都该动，但**先**动的那一轴不该因为
    /// 另一轴还没越过阈值而卡住。
    pub fn drag_update(&mut self, pos: Point) -> bool {
        if !self.dragging || self.disabled {
            return false;
        }
        let travel_x = pos.x - self.drag_start_pos.x;
        let travel_y = pos.y - self.drag_start_pos.y;

        if !self.axis_active_x {
            if travel_x.abs() >= SCROLL_DRAG_SLOP {
                self.axis_active_x = true;
                self.acc_x = travel_x - SCROLL_DRAG_SLOP * travel_x.signum();
            }
        } else {
            self.acc_x += pos.x - self.last_pos.x;
        }
        if !self.axis_active_y {
            if travel_y.abs() >= SCROLL_DRAG_SLOP {
                self.axis_active_y = true;
                self.acc_y = travel_y - SCROLL_DRAG_SLOP * travel_y.signum();
            }
        } else {
            self.acc_y += pos.y - self.last_pos.y;
        }
        self.last_pos = pos;

        // 内容跟着手指走：手指向下 ⇒ 内容向上 ⇒ 偏移增大。
        let target = Point::new(
            self.drag_start_offset.x - self.acc_x,
            self.drag_start_offset.y - self.acc_y,
        );
        let clamped = self.clamp_point(target);
        let changed = (clamped.x - self.scroll_offset.x).abs() > f32::EPSILON
            || (clamped.y - self.scroll_offset.y).abs() > f32::EPSILON;
        self.scroll_offset = clamped;
        changed
    }

    /// 结束拖拽。
    pub fn drag_end(&mut self) {
        self.dragging = false;
        self.axis_active_x = false;
        self.axis_active_y = false;
        self.acc_x = 0.0;
        self.acc_y = 0.0;
    }

    /// 是否正在拖拽。
    pub const fn is_dragging(&self) -> bool {
        self.dragging
    }
}

impl Default for ScrollView {
    fn default() -> Self {
        Self::new("scroll-view")
    }
}

/// 非有限值归零（`NaN` / `∞` 不能进几何计算）。
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "../tests/unit/scroll_view_tests.rs"]
mod tests;
