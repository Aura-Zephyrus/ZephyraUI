//! RinUI 目录里尚未有 Rust 状态模型的补充控件（`https://ui.rinlit.cn/zh/components/`）。
//!
//! 这些控件的**结构**（`controls.zypui`）与**外观**（`controls-*.zstyle`）早已存在，
//! 缺的是「值 + 状态流转」。本模块只放纯状态与规则，不碰颜色 / 尺寸 / 绘制 ——
//! 宿主把 getter 喂给 zypui `@绑定`，把点击分派到 setter，与 `Checkbox` / `ToggleSwitch`
//! / `Slider` 的现行分工完全一致。
//!
//! | 控件 | 状态 | 模板参数 |
//! |---|---|---|
//! | [`Progress`] | 进度 0..=1 + 状态词 | `ProgressBar value01 state` / `ProgressRing value state show_value` |
//! | [`InfoBarState`] | 严重级 + 打开与否 | `InfoBar title text severity closable` |
//! | [`ColorPickerState`] | 选中色号 0..15 | `ColorPicker sel` |
//! | [`SpinBoxState`] | 数值 + 上下界 | `SpinBox value` |
//! | [`BusyIndicatorState`] | 帧序号 | `BusyIndicator frame` |
//! | [`ExpanderState`] | 展开与否 | `Expander title expanded` |
//! | [`SelectorBarState`] | 选中项 | `SelectorBar items sel` |
//! | [`AvatarState`] | 缩写 + 强调 | `Avatar initials accent` |

/// 进度值（0..=1）与状态词。ProgressBar / ProgressRing 共用。
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    value: f32,
    state: &'static str,
}

impl Progress {
    /// 以给定比例（0..=1）建进度，状态为空（正常）。
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            state: "",
        }
    }

    pub fn set_value(&mut self, value: f32) {
        self.value = value.clamp(0.0, 1.0);
    }

    /// 给模板 `value01` 的比例。
    pub fn value01(&self) -> f32 {
        self.value
    }

    /// 百分比整数（给环形进度的 `value` 文本）。
    pub fn percent(&self) -> u32 {
        (self.value * 100.0).round() as u32
    }

    /// 状态分类词：`""` / `"paused"` / `"error"`。
    pub fn state_word(&self) -> &'static str {
        self.state
    }

    pub fn set_state(&mut self, state: &'static str) {
        self.state = state;
    }
}

/// InfoBar 的状态：严重级分类词 + 是否打开。
#[derive(Debug, Clone, PartialEq)]
pub struct InfoBarState {
    severity: &'static str,
    open: bool,
}

impl InfoBarState {
    pub fn new(severity: &'static str) -> Self {
        Self {
            severity,
            open: true,
        }
    }

    /// `"info"` / `"success"` / `"caution"` / `"error"`。
    pub fn severity(&self) -> &'static str {
        self.severity
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 关闭按钮：`bar:close` 点击后调用；重复关闭无副作用。
    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn reopen(&mut self) {
        self.open = true;
    }
}

/// 8×2 色板的选中下标（0..=15）。
#[derive(Debug, Clone, PartialEq)]
pub struct ColorPickerState {
    selected: usize,
}

impl ColorPickerState {
    pub fn new(selected: usize) -> Self {
        Self {
            selected: selected.min(15),
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        if index < 16 {
            self.selected = index;
        }
    }
}

/// 数字框：值 + 上下界。上/下按钮越过边界时为 no-op。
#[derive(Debug, Clone, PartialEq)]
pub struct SpinBoxState {
    value: i32,
    min: i32,
    max: i32,
}

impl SpinBoxState {
    pub fn new(value: i32, min: i32, max: i32) -> Self {
        debug_assert!(min <= max);
        Self {
            value: value.clamp(min, max),
            min,
            max,
        }
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn increment(&mut self) {
        self.value = (self.value + 1).min(self.max);
    }

    pub fn decrement(&mut self) {
        self.value = (self.value - 1).max(self.min);
    }
}

/// RinUI `BusyIndicator`（≡ `ProgressRing` 不确定态）的帧序号。
///
/// 原版是 `startAngle` 两段各 1033ms、`sweepAngle` 0→180→0，`Utils` 的
/// `progressBarAnimationSpeed = 1550ms / 1.5` —— 一个完整循环 ≈ 2066ms。
/// 这里按 24 帧采样（每帧 ≈ 86ms），每帧对应一枚预生成的弧线 SVG。
#[derive(Debug, Clone, PartialEq)]
pub struct BusyIndicatorState {
    frame: u32,
    elapsed_ms: f32,
}

/// 一帧的毫秒数：2066 / 24 ≈ 86。
pub const BUSY_FRAME_MS: f32 = 2066.0 / 24.0;
/// 帧数（与 `zephyra_core::icon::assets` 里 `busy-arc-N` 一一对应）。
pub const BUSY_FRAME_COUNT: u32 = 24;

impl BusyIndicatorState {
    pub fn new() -> Self {
        Self {
            frame: 0,
            elapsed_ms: 0.0,
        }
    }

    /// 当前帧（0..23）。
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// 对应的图标名（`busy-arc-N`）。
    pub fn icon_name(&self) -> String {
        format!("busy-arc-{}", self.frame)
    }

    /// 当前在一个完整循环里的相位（毫秒，0..2066）。
    ///
    /// 连续相位是 Canvas 路径动画的输入；`frame()` / `icon_name()` 是它离散化后的
    /// 兼容视图（供还没有路径能力的模板使用）。
    pub fn phase_ms(&self) -> f32 {
        self.elapsed_ms + self.frame as f32 * BUSY_FRAME_MS
    }

    /// `startAngle`（度），照 RinUI `ProgressRing.qml:90-94` 的两段线性动画。
    pub fn start_angle(&self) -> f32 {
        let t = self.phase_ms() / 1033.0;
        if t < 1.0 {
            450.0 * t
        } else {
            450.0 + 630.0 * (t - 1.0).min(1.0)
        }
    }

    /// `sweepAngle`（度），照 RinUI `ProgressRing.qml:96-100` 的呼吸式伸缩。
    pub fn sweep_angle(&self) -> f32 {
        let t = self.phase_ms() / 1033.0;
        if t < 1.0 {
            180.0 * t
        } else {
            180.0 * (1.0 - (t - 1.0).min(1.0))
        }
    }

    /// 是否还在动（宿主据此决定 `wants_frames`）。
    pub fn is_animating(&self) -> bool {
        true
    }

    pub fn advance(&mut self, dt_ms: f32) {
        self.elapsed_ms += dt_ms.max(0.0);
        while self.elapsed_ms >= BUSY_FRAME_MS {
            self.elapsed_ms -= BUSY_FRAME_MS;
            self.frame = (self.frame + 1) % BUSY_FRAME_COUNT;
        }
    }
}

impl Default for BusyIndicatorState {
    fn default() -> Self {
        Self::new()
    }
}

/// 展开器的开合状态。
#[derive(Debug, Clone, PartialEq)]
pub struct ExpanderState {
    expanded: bool,
}

impl ExpanderState {
    pub fn new(expanded: bool) -> Self {
        Self { expanded }
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }
}

/// 选择器栏的选中项下标。
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorBarState {
    selected: usize,
    count: usize,
}

impl SelectorBarState {
    pub fn new(selected: usize, count: usize) -> Self {
        Self {
            selected: selected.min(count.saturating_sub(1)),
            count,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 选中项 id（模板 `sel` 按 id 比对）。
    pub fn selected_id(&self) -> String {
        format!("sel:{}", self.selected)
    }

    pub fn select(&mut self, index: usize) {
        if index < self.count {
            self.selected = index;
        }
    }
}

/// 头像的缩写与强调标志。
#[derive(Debug, Clone, PartialEq)]
pub struct AvatarState {
    initials: &'static str,
    accent: bool,
}

impl AvatarState {
    pub fn new(initials: &'static str, accent: bool) -> Self {
        Self { initials, accent }
    }

    pub fn initials(&self) -> &'static str {
        self.initials
    }

    pub fn is_accent(&self) -> bool {
        self.accent
    }
}

/// ToggleButton / PillButton 的按下状态（RinUI：Button/RoundButton + `checkable`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToggleState {
    checked: bool,
}

impl ToggleState {
    pub fn new(checked: bool) -> Self {
        Self { checked }
    }

    pub fn is_checked(&self) -> bool {
        self.checked
    }

    pub fn toggle(&mut self) {
        self.checked = !self.checked;
    }

    /// 模板状态词（class 用）。
    pub fn class_word(&self) -> &'static str {
        if self.checked {
            "checked"
        } else {
            ""
        }
    }
}

/// ToolButton：`highlighted` 决定图标取强调色还是常规色。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolButtonState {
    highlighted: bool,
}

impl ToolButtonState {
    pub fn new(highlighted: bool) -> Self {
        Self { highlighted }
    }

    pub fn is_highlighted(&self) -> bool {
        self.highlighted
    }

    pub fn toggle(&mut self) {
        self.highlighted = !self.highlighted;
    }
}

/// Hyperlink：点过之后记录 visited（文案不变，只用于状态演示）。
#[derive(Debug, Clone, PartialEq)]
pub struct HyperlinkState {
    visited: bool,
}

impl HyperlinkState {
    pub fn new() -> Self {
        Self { visited: false }
    }

    pub fn is_visited(&self) -> bool {
        self.visited
    }

    pub fn visit(&mut self) {
        self.visited = true;
    }
}

impl Default for HyperlinkState {
    fn default() -> Self {
        Self::new()
    }
}

/// InfoBadge：计数 + 严重级 + 实心/描边。
#[derive(Debug, Clone, PartialEq)]
pub struct InfoBadgeState {
    count: i32,
    severity: &'static str,
    solid: bool,
    max: i32,
}

impl InfoBadgeState {
    pub fn new(count: i32, severity: &'static str, solid: bool) -> Self {
        Self {
            count,
            severity,
            solid,
            max: 99,
        }
    }

    /// 超上限显示 `99+`，负数显示空（`count = -1` 是 RinUI 的 dot 约定）。
    pub fn text(&self) -> String {
        if self.count < 0 {
            String::new()
        } else if self.count > self.max {
            format!("{}+", self.max)
        } else {
            self.count.to_string()
        }
    }

    pub fn severity(&self) -> &'static str {
        self.severity
    }

    pub fn is_solid(&self) -> bool {
        self.solid
    }

    pub fn set_count(&mut self, count: i32) {
        self.count = count;
    }
}

/// ToolTip：显隐（演示里由「点我」切换，RinUI 里由 hover 触发）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolTipState {
    visible: bool,
}

impl ToolTipState {
    pub fn new(visible: bool) -> Self {
        Self { visible }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

/// PageIndicator：点数与当前页（RinUI：当前点 6px、其它 4px）。
#[derive(Debug, Clone, PartialEq)]
pub struct PageIndicatorState {
    count: u32,
    current: u32,
}

impl PageIndicatorState {
    pub fn new(count: u32, current: u32) -> Self {
        Self {
            count: count.max(1),
            current: current.min(count.saturating_sub(1)),
        }
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    /// 点第 i 个点：循环到下一页（到末尾回 0）。
    pub fn next(&mut self) {
        self.current = (self.current + 1) % self.count;
    }

    pub fn set_current(&mut self, i: u32) {
        if i < self.count {
            self.current = i;
        }
    }
}

/// SettingItem：是否可点（RinUI `clickable` 默认 false）。
#[derive(Debug, Clone, PartialEq)]
pub struct SettingItemState {
    clickable: bool,
}

impl SettingItemState {
    pub fn new(clickable: bool) -> Self {
        Self { clickable }
    }

    pub fn is_clickable(&self) -> bool {
        self.clickable
    }

    pub fn toggle(&mut self) {
        self.clickable = !self.clickable;
    }
}

/// SettingExpander：设置页风格的展开器开合。
#[derive(Debug, Clone, PartialEq)]
pub struct SettingExpanderState {
    expanded: bool,
}

impl SettingExpanderState {
    pub fn new(expanded: bool) -> Self {
        Self { expanded }
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }
}

/// Card：可点卡片的选中态（RinUI `Card` ≡ 可点 Frame）。
#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    selected: bool,
}

impl CardState {
    pub fn new(selected: bool) -> Self {
        Self { selected }
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }

    pub fn toggle(&mut self) {
        self.selected = !self.selected;
    }
}

#[cfg(test)]
#[path = "../tests/unit/catalog_tests.rs"]
mod tests;
