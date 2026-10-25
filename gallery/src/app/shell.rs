//! 外壳**常量**：画布、标题栏、导航栏、内容卡的基准尺寸。
//!
//! # 迁移后这里没有结构
//!
//! 窗口外壳的**结构**（标题栏 + 导航 + 内容卡）住在 `ui/shell.zypui` 的
//! `Window` 页里；这里是它引用的尺寸常量 —— 它们同时是布局几何与门禁
//! （`shell_metrics_match_stylesheet`）比对的**真相来源**：
//! 「代码里 240、样式表里 220」这种两处真相靠那条门禁挡住。
//!
//! # "置底"怎么实现
//!
//! core 没有 flex-grow。RinUI 把 Settings 钉在导航栏底部，这里用一个
//! **定高占位**把 Settings 推到底 —— 占位高度是布局几何（不是外观），
//! 且有一条几何门禁 (`settings_lands_near_the_bottom`) 真去量布局后的矩形，
//! 偏差过大就红。改导航项数量时改常量，门禁会告诉你改得对不对。

/// 画布宽（RinUI gallery 的窗口宽）。
///
/// ⚠️ 它只是**基准画布**的宽度，不是窗口的宽度。窗口宽度由宿主按 `inner_size()`
/// 给（见 `Gallery::viewport`），外壳的宽度方向全部靠 `cross-align: stretch`
/// 铺满 —— 这里保留常量只为让基准尺寸有名字可引用。
pub const CANVAS_W: f32 = 1400.0;
/// 画布高。同 [`CANVAS_W`]：基准值，不是窗口高。
pub const CANVAS_H: f32 = 800.0;
/// 标题栏高（Apple HIG toolbar 高度）。
pub const TITLEBAR_H: f32 = 44.0;
/// 导航栏宽（Apple HIG 侧栏宽度）。
pub const NAV_W: f32 = 260.0;
/// **紧凑档**的导航栏宽：只留图标列（WinUI NavigationView 的 `LeftCompact`）。
///
/// 出处见 `docs/developer/responsive-and-touch.md` §6 引 `responsive-design` 页。取 56
/// 而不是更小：图标列本身是 32 宽（**4 的倍数**，官方要求基单位为 4 epx），
/// 加两侧 padding 后不能把图标压到 32 以下。
pub const NAV_W_COMPACT: f32 = 56.0;
/// 导航栏高 = 画布高 - 标题栏高。
pub const NAV_H: f32 = CANVAS_H - TITLEBAR_H;
/// 内容卡宽 = 画布宽 - 导航宽 - 1px 接缝（RinUI StackView 的 leftMargin）。
pub const CONTENT_W: f32 = CANVAS_W - NAV_W - 1.0;
/// 把置底项推到页面底部的定高占位（布局几何，由几何门禁校准）。
///
/// # 校准公式
///
/// ```text
/// NAV_H - 2*padding - (NAV_ITEMS*item_h + NAV_ITEMS*spacing) - item_h - 底部留白
/// ```
///
/// 当前 8 个导航项（7 路由 + Settings 置底）、项高 40、间距 6、上下 padding 各 12：
/// `756 - 24 - 7*40 - 6*6 - 40 - 44 = 364`（7 项之间只有 6 个间距）。
///
/// ⚠ 改路由数量时**必须**重算这里：差一项（46）时 `shell_layout_geometry`
/// 会立刻红，而症状在画面上只是「置底项浮在半空」，不看几何门禁很容易归咎
/// 成「间距不对」而去改样式表。
pub const NAV_SPACER: f32 = 324.0;

/// 导航项数（= `ROUTES` 的长度 + Settings 置底项）。
///
/// 显式写成一个常量而不是 `ROUTES.len()`：几何门禁要比对**算出来的数**与
/// **样式表里的数**，用 `.len()` 会让两边永远相等，于是改路由数量时门禁失去作用。
pub const NAV_ITEMS: usize = 8;
