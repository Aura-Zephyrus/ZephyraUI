//! 滚动条（ScrollBar）控件的状态模型。
//!
//! # 蓝本（WinUI `ScrollBar` / CSS `::-webkit-scrollbar`）
//!
//! 滚动条 = **轨道**（track）+ **拇指**（thumb）。拇指的长度反映
//! 「视口 / 内容」的比例，拇指的位置反映当前滚动偏移。
//!
//! # 分工
//!
//! | 关注 | 住在哪 |
//! |---|---|
//! | 拇指几何、拖动换算、滚轮翻页 | 本模块（`ScrollBar`） |
//! | 结构（轨道 / 拇指两个盒子、分类名） | `ui/controls.zypui` |
//! | 颜色、圆角、悬停态 | `controls-*.zstyle` |
//! | 与 `ScrollView` 的**绑定**（谁驱动谁） | 宿主应用 |
//!
//! # 与 `ScrollView` 的关系
//!
//! 两者**不共享状态**：滚动条只持有一份「内容长度 / 视口长度 / 当前偏移」的
//! **快照**，拖动时产出新的偏移，由宿主写回 `ScrollView`。这样两个控件都能
//! 单独存在（滚动条可以挂在别的滚动容器上），也让「谁是权威」这件事只有一处：
//! **`ScrollView` 是权威**，滚动条每帧从它同步。
//!
//! # 换算
//!
//! 设轨道长 `T`、内容长 `C`、视口长 `V`、当前偏移 `o`：
//!
//! - 拇指长 `thumb = max(MIN_THUMB, T * V / C)`；
//! - 拇指起点 `p = o / (C - V) * (T - thumb)`；
//! - 拖动时 `o' = o0 + (pointer - pointer0) * (C - V) / (T - thumb)`。
//!
//! `C <= V`（内容装得下）⇒ 没有可滚的距离 ⇒ 拇指铺满轨道、偏移恒为 0。

use zephyra_core::core::Point;

/// 滚动条的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollBarAxis {
    /// 竖直滚动条（内容沿 Y 方向滚动）。
    Vertical,
    /// 水平滚动条（内容沿 X 方向滚动）。
    Horizontal,
}

impl ScrollBarAxis {
    /// 取该轴上的长度（DIP）。
    const fn of(&self, size: Point) -> f32 {
        match self {
            Self::Vertical => size.y,
            Self::Horizontal => size.x,
        }
    }
}

/// 拇指的最小长度（DIP）。
///
/// 内容远长于视口时，按比例算出来的拇指会细到点不中。WinUI 的最小值是 16 DIP，
/// 这里取 24 —— 与本仓其它可拖元素的命中尺寸同量级（拇指太细比太长更难用）。
pub const MIN_THUMB_LENGTH: f32 = 24.0;

/// 轨道点按翻页的步长（DIP）占视口的比例。
///
/// 点在轨道空白处 → 翻一「页」。页 = 视口的 90%（留 10% 重叠，让连续点按有
/// 连贯感 —— 每页正好等于视口时，两次点按之间的内容会跳过一整屏）。
pub const PAGE_FRACTION: f32 = 0.9;

/// 滚动条的状态模型。
#[derive(Debug, Clone)]
pub struct ScrollBar {
    key: &'static str,
    axis: ScrollBarAxis,
    /// 轨道长度（DIP）。宿主按实际布局设置。
    track_length: f32,
    /// 内容长度（DIP）。
    content_length: f32,
    /// 视口长度（DIP）。
    viewport_length: f32,
    /// 当前滚动偏移（DIP）。宿主每帧从 `ScrollView` 同步。
    offset: f32,
    /// 是否正在拖拽拇指。
    dragging: bool,
    /// 拖拽开始时指针在该轴上的位置（DIP，绝对坐标）。
    drag_start_pointer: f32,
    /// 拖拽开始时的偏移（DIP）。
    drag_start_offset: f32,
    /// 固定拇指长度（`None` = 按比例算）。
    fixed_thumb_length: Option<f32>,
}

impl ScrollBar {
    /// 新建滚动条（竖直）。
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            axis: ScrollBarAxis::Vertical,
            track_length: 0.0,
            content_length: 0.0,
            viewport_length: 0.0,
            offset: 0.0,
            dragging: false,
            drag_start_pointer: 0.0,
            drag_start_offset: 0.0,
            fixed_thumb_length: None,
        }
    }

    /// 设置方向。
    pub fn set_axis(&mut self, axis: ScrollBarAxis) {
        self.axis = axis;
    }

    /// 当前方向。
    pub const fn axis(&self) -> ScrollBarAxis {
        self.axis
    }

    /// 滚动条的稳定 key（事件路由身份）。
    pub const fn key(&self) -> &'static str {
        self.key
    }

    /// 设置固定拇指长度（`None` = 按比例算）。
    pub fn set_fixed_thumb_length(&mut self, len: Option<f32>) {
        self.fixed_thumb_length = len.filter(|v| v.is_finite() && *v > 0.0);
    }

    /// 设置几何：轨道长 / 内容长 / 视口长（DIP）。
    pub fn set_geometry(&mut self, track_length: f32, content_length: f32, viewport_length: f32) {
        self.track_length = finite(track_length);
        self.content_length = finite(content_length);
        self.viewport_length = finite(viewport_length);
    }

    /// 从一个二维尺寸对设置几何（取该轴上的长度）。
    pub fn set_geometry_from_sizes(&mut self, track: Point, content: Point, viewport: Point) {
        self.set_geometry(
            axis_len(&self.axis, track),
            axis_len(&self.axis, content),
            axis_len(&self.axis, viewport),
        );
    }

    /// 同步当前偏移（DIP）。宿主每帧从 `ScrollView` 拿。
    pub fn set_offset(&mut self, offset: f32) {
        self.offset = self.clamp_offset(offset);
    }

    /// 当前偏移（DIP）。
    pub const fn offset(&self) -> f32 {
        self.offset
    }

    /// 该轴上的最大可滚动距离（DIP）。
    pub fn max_offset(&self) -> f32 {
        (self.content_length - self.viewport_length).max(0.0)
    }

    /// 是否可滚动（内容比视口长）。
    pub fn is_scrollable(&self) -> bool {
        self.max_offset() > 0.0
    }

    /// 拇指长度（DIP）。
    pub fn thumb_length(&self) -> f32 {
        if let Some(fixed) = self.fixed_thumb_length {
            return fixed.min(self.track_length.max(0.0));
        }
        if self.content_length <= 0.0 || self.track_length <= 0.0 {
            return 0.0;
        }
        let ratio = (self.viewport_length / self.content_length).clamp(0.0, 1.0);
        (self.track_length * ratio).max(MIN_THUMB_LENGTH.min(self.track_length))
    }

    /// 拇指在轨道上的起点（DIP，从轨道起点算）。
    pub fn thumb_offset(&self) -> f32 {
        let max = self.max_offset();
        if max <= 0.0 {
            return 0.0;
        }
        let travel = (self.track_length - self.thumb_length()).max(0.0);
        (self.offset / max * travel).clamp(0.0, travel)
    }

    /// 拇指的终点（DIP，从轨道起点算）。
    pub fn thumb_end(&self) -> f32 {
        self.thumb_offset() + self.thumb_length()
    }

    /// 指针位置（DIP，绝对）是否落在拇指上。
    pub fn hits_thumb(&self, pointer: f32) -> bool {
        let t = self.thumb_offset();
        pointer >= t && pointer <= t + self.thumb_length()
    }

    /// 开始拖拽。`pointer` 是该轴上的**绝对**坐标（DIP）。
    pub fn drag_start(&mut self, pointer: f32) {
        self.dragging = true;
        self.drag_start_pointer = pointer;
        self.drag_start_offset = self.offset;
    }

    /// 拖拽更新。返回新的偏移（DIP，已钳位）。
    ///
    /// 换算：指针走了 `d` DIP，拇指能走的行程是 `track - thumb`，
    /// 而内容能走的行程是 `content - viewport` —— 两者按比例对应。
    pub fn drag_update(&mut self, pointer: f32) -> f32 {
        if !self.dragging {
            return self.offset;
        }
        let thumb_travel = (self.track_length - self.thumb_length()).max(0.0);
        let content_travel = self.max_offset();
        let new_offset = if thumb_travel <= 0.0 || content_travel <= 0.0 {
            self.drag_start_offset
        } else {
            let delta = pointer - self.drag_start_pointer;
            self.drag_start_offset + delta * (content_travel / thumb_travel)
        };
        self.offset = self.clamp_offset(new_offset);
        self.offset
    }

    /// 结束拖拽。
    pub fn drag_end(&mut self) {
        self.dragging = false;
    }

    /// 是否正在拖拽。
    pub const fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// 轨道点按：翻一页。`forward` = `true` 表示向下/向右翻。
    ///
    /// 返回新的偏移（DIP，已钳位）。
    pub fn page(&mut self, forward: bool) -> f32 {
        let step = (self.viewport_length * PAGE_FRACTION).max(1.0);
        let delta = if forward { step } else { -step };
        self.offset = self.clamp_offset(self.offset + delta);
        self.offset
    }

    /// 滚轮：翻一格。返回新的偏移（DIP，已钳位）。
    ///
    /// 竖直滚动条只吃 `delta_y`；水平滚动条只吃 `delta_x` —— 交叉的滚轮方向
    /// 不该把水平条也带动（那是内容自己的事）。
    ///
    /// # 方向约定：正数 = 向上（与 [`crate::scroll_view::ScrollView::scroll_wheel`] 同款）
    ///
    /// 正增量表示**向上滚** ⇒ 偏移**减小**（`offset` 是「内容向上移了多少」，
    /// 向上滚就是回到更靠前的内容）。符号搞反的话，滚动条会与滚动容器
    /// 朝**相反**方向动 —— 两个控件各滚各的，谁也说不清是谁错了。
    pub fn wheel(&mut self, delta_x: f32, delta_y: f32) -> f32 {
        let step = match self.axis {
            ScrollBarAxis::Vertical => delta_y,
            ScrollBarAxis::Horizontal => delta_x,
        };
        if !step.is_finite() || step == 0.0 {
            return self.offset;
        }
        let page = (self.viewport_length * PAGE_FRACTION).max(1.0);
        // ⚠ **不能**用 `step.signum()`：Rust 的 `f32::signum` 对 `+0.0` 返回
        // `1.0`（不是 `0.0`）—— 零增量会翻一整页。触控板惯性结束时正是零增量。
        //
        // ⚠ 符号：正增量 = 向上滚 = 偏移减小 ⇒ 与 `step` 反号。
        let delta = if step > 0.0 { -page } else { page };
        self.offset = self.clamp_offset(self.offset + delta);
        self.offset
    }

    fn clamp_offset(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return 0.0;
        }
        v.clamp(0.0, self.max_offset())
    }
}

impl Default for ScrollBar {
    fn default() -> Self {
        Self::new("scroll-bar")
    }
}

/// 取该轴上的长度（DIP）。
fn axis_len(axis: &ScrollBarAxis, size: Point) -> f32 {
    axis.of(size)
}

/// 非有限值归零。
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "../tests/unit/scroll_bar_tests.rs"]
mod tests;
