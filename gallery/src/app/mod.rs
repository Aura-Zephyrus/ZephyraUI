//! Gallery 应用：RinUI 示例应用（gallery）的 Zephyra 重写版。
//!
//! # 状态从哪来
//!
//! `InputState` 只给 hover / pressed（见 `zephyra_fx::InputState`）。click 是
//! 按下与抬起的配对，而配对信息在 `on_input` 的两次调用之间 —— 所以这里自己
//! 记一份 `last_pressed`，在「上一帧按着、这一帧松了」时还原出 click。
//! 与 `InputRouter` 的 `RoutedEvent::Click` 同一条判据（同键才算点中），
//! 放在 View 侧不丢信息，也不需要改宿主。
//!
//! # 主题为什么是「意图」
//!
//! `&Theme` 由宿主持有（窗口里它是 `&'static`）。View 只发意图
//! （`wants_dark`），宿主每帧对账后执行 `set_theme` —— 于是"想切的"与
//! "在用的"永远可分开观测，不会出现"点了没反应但也不知道为什么"。

use zephyra_core::i18n::{Messages, Preferences, Tag};
use zephyra_core::id::ElementKey;
use zephyra_core::{context::Context, element::Element};
use zephyra_fx::{InputState, View};

pub mod glass;
pub mod pages;
pub mod shell;

/// 路由表：`(id, 标题, 图标)`。导航栏从左到右、Settings 置底。
///
/// # 为什么是 11 条而不是 70 条
///
/// 旧结构是 Rin-UI 的翻版：10 个页面 + 14 个分组索引 + 44 个控件展示页 + 设置。
/// 那套能查到每一个控件，代价是**导航树本身大到没人用** —— 44 个控件页里绝大
/// 多数一辈子不会被点开，而它们让 `catalog.rs`（340 行）与 `showcase.rs`
/// （453 行）成为两份几乎重复的模板代码。
///
/// 现在的取舍：**按控件的「族」分页**，每页把一族控件摆在一起，各自都带活样例。
/// 11 条路由 vs 70 条，导航一眼扫完；`showcase` 那套模板代码整个消失。
///
/// 代价要说清：**控件级的深度文档没有了**。控件层自己的 doc 注释是那件事的
/// 正统载体，gallery 的职责是「让人看见它长什么样、摸一摸」，不是当手册。
///
/// # 顺序与 [`pages::PAGES`] 必须一致
///
/// 导航栏遍历本表、首页遍历那张表，两者的 id 集合由门禁
/// `route_table_and_page_table_agree` 钉住 —— 加了一页忘了另一张时，症状是
/// 「点得进导航、进去却是首页」，既不报错也不崩。
///
/// # 图标名来自 core 的内置图标集
///
/// 拼错时 `ICON_NAME` 解析不到、图标**静默**消失 —— 门禁
/// `every_icon_name_the_tree_emits_is_registered` 盯着这条。
pub const ROUTES: &[(&str, &str, &str)] = &[
    ("guide", "Quick start", "home"),
    ("state", "State", "check"),
    ("styling", "Styling", "pen"),
    ("layout", "Layout", "grid"),
    ("theme", "Theme & i18n", "settings"),
    ("testing", "Testing", "bell"),
    ("reference", "Controls", "doc"),
    // 液态玻璃**提升为一等路由**：它有运行期可调的折射参数实验台，
    // 藏在第 8 族要点两下才看得到，而四个 `[−]/[+]` 步进器正是最不该
    // 藏起来的东西。图标选 `sliders` —— 这一页的核心是调参数。
    ("glass", "Liquid Glass", "sliders"),
];

/// 控件参考页的**族**（顺序即 segmented 的段顺序）。
///
/// # 为什么用下标而不是 `&'static str`
///
/// `view()` 每帧重建整棵树，`selected` 要喂给 segmented。存下标是 Copy 的、
/// 能直接比较，与 `combo` / `spin` 同一手法；存字符串则每帧要比较一次内容，
/// 而「选中的是哪一项」在画面上就是那个高亮的段。
///
/// # 与旧 `pages::PAGES` 的分工
///
/// 参考层不再是**路由**（9 条路由平铺进导航栏装不下 —— 二级导航
/// `nav_sub_item` 不折叠，9 个子项常驻会把 756 高的导航栏撑爆），而是
/// `reference` **页内**的 segmented 切换。十族内容复用现有页面模块的
/// `content()`，每族独立一屏，一屏装得下。
pub const REF_FAMILIES: &[&str] = &[
    "basics",
    "toggles",
    "layout",
    "collections",
    "menus",
    "navigation",
    "status",
    "dialogs",
    // ⚠ 新族一律**追加在末尾**，不要插在中间：`ref_family` 是下标，
    // 门禁与 `--family` 都用下标引用（插中间会静默改掉已有序号的含义，
    // 而画面上只是「点了这一族出现那族」）。
    //
    // ⚠ `effects`（液态玻璃）已不在其中 —— 它提升为一等路由（见 [`ROUTES`]）：
    // 四个 `[−]/[+]` 步进器是运行期调折射参数的实验台，藏在第 8 族要点两下
    // 才看得到，而那是本页最不该藏的东西。现在 9 族。
    "paging",
];

/// 基准画布尺寸（物理像素）= 宿主建窗口时的初值。
///
/// 它同时是[`Gallery`] 在 dump 路下的默认视口，于是静态图与窗口路共用同一套
/// 断点。Rin-UI 原版按桌面大窗口设计，所以基准落在官方"大"档（1120 epx）。
pub const BASE_VIEWPORT: (u32, u32) = (1400, 800);

/// 内容区两侧的页边距（布局单位）。与 `pages.rs` 里的 section 间距同源，
/// 由 `tests::gates::content_width_follows_breakpoint` 间接钉住。
pub const PAGE_GUTTER: f32 = 28.0;

/// 基准画布下的**网格列数**。
///
/// # 它现在只有一个消费者：响应式门禁
///
/// 旧结构里这批「44 张控件卡」铺在网格上，于是列数直接决定首页长什么样。
/// 重写后首页放的是活样例与分类入口（`pages::home`），网格只剩布局页与
/// 状态页里两三张卡 —— 而 core **没有**按列数折行的布局语义，所以列数对画面
/// 已经没有影响。
///
/// 那为什么不删？删掉之后 `content_width()` 与断点之间就少了一条可测的关系，
/// 而那正是 `docs/developer/responsive-and-touch.md` 记下的那笔账。把常量与
/// 断言留在门禁里，等 core 长出 wrap 语义时直接接上，而不是重新推一遍。
pub const BASE_COLUMNS: usize = 5;

/// 本 demo 支持的语言（有序，**第一项是默认/回退语言**）。
///
/// 顺序有语义：协商匹配不到任何用户偏好时回落第一项 —— 这与浏览器把默认
/// 语言放在 "supported locales" 同一形状（`Intl` 的 preference-based fallback
/// 即按这个顺序取第一个匹配）。
///
/// 顺带这也是 `Messages::select` 的 `fallback` 参数。
pub const SUPPORTED: &[&str] = &["en", "zh-Hans", "ja"];

/// 各语言的自称，下标与 [`SUPPORTED`] 对齐。
///
/// # 为什么**不**进文案表
///
/// 自称不是翻译：任何语言的用户看到的都是同一组"English / 中文 / 日本語"。
/// 放进表里就得给 3 种语言各填 3 条共 9 条，而那 9 条内容两两相同 ——
/// 表格里的重复迟早会被误改成"翻译"，而那正是错的。
///
/// 对齐关系由门禁 `every_language_name_is_counted_in_the_same_order` 钉住：
/// 下标错位会让用户选了"English"却得到日文，而那在画面上完全看不出原因。
pub const LANG_SELFS: &[&str] = &["English", "中文", "日本語"];

/// 文案表：`(locale, key, value)` 三元组，一行一条。
///
/// # 为什么是一张平表而不是 `HashMap<locale, HashMap<key, value>>`
///
/// 两种都能取到值，但平表让「门禁枚举漏翻」变成一行 `CATALOG.iter()`
/// —— 而漏翻是 i18n 唯一真正危险的东西（其余问题最多是显示难看）。
/// 分语言分文件是**将来**运行期加载时才需要的形态，见 `docs/developer/i18n.md` §4。
///
/// # 覆盖范围与未覆盖范围（不要误读这张表）
///
/// 已接进视图：路由标题、页头、应用名、设置页、以及各页正文的
/// demo 标签 / 控件标签。仍**未接**：少数分区标题与按钮标签
/// （`guide/layout.rs` 的 "Breakpoints: small / medium / large" 之类）
/// —— 那些仍是源码里的英文硬编码，是**记账的债**，不是"已支持"。
///
/// 键名是英文原文的 slug：[`Gallery::tr`] 找不到键时回退显示**键名本身**，
/// 于是漏加条目时画面上是可读英文而不是乱码。这条覆盖由门禁
/// `every_tr_key_used_in_source_exists_in_every_language` 逐语言查
/// （它扫源码里每个 `g.tr("…")`，再反查 `catalog_keys()` —— 两个方向都查）。
const CATALOG: &[(&str, &str, &str)] = &[
    // ---- en（默认语言）----
    ("en", "app.title", "ZephyraUI Gallery"),
    ("en", "route.guide", "Quick start"),
    ("en", "route.state", "State"),
    ("en", "route.styling", "Styling"),
    ("en", "route.layout", "Layout"),
    ("en", "route.theme", "Theme & i18n"),
    ("en", "route.testing", "Testing"),
    ("en", "route.reference", "Controls"),
    ("en", "route.glass", "Liquid Glass"),
    (
        "en",
        "ui.small-nav-collapsed-to-icons",
        "Small — nav collapsed to icons",
    ),
    (
        "en",
        "ui.medium-nav-shows-labels",
        "Medium — nav shows labels",
    ),
    ("en", "ui.large-full-layout", "Large — full layout"),
    ("en", "route.settings", "Settings"),
    ("en", "action.feedback", "Feedback"),
    ("en", "action.language", "Language"),
    ("en", "set.theme.title", "Dark mode"),
    (
        "en",
        "set.theme.desc",
        "Switching theme only swaps token values",
    ),
    ("en", "set.volume.title", "Volume"),
    ("en", "set.volume.desc", "Fill width = value x track width"),
    ("en", "set.notify.title", "Notifications"),
    (
        "en",
        "set.notify.desc",
        "Selected state travels as a class name",
    ),
    ("en", "set.adv.title", "Advanced"),
    ("en", "set.adv.logging", "Diagnostic logging"),
    ("en", "set.adv.hwaccel", "Hardware acceleration"),
    ("en", "set.adv.shortcuts", "Shortcuts"),
    // ---- 页面正文（各页 demo 标签 / 控件标签 / 分区标题）----
    //
    // 键名是英文原文的 slug：`tr()` 找不到键时**回退显示键名本身**，于是
    // 漏加条目时画面上出现的是可读的英文而不是乱码 —— 这也是键不另起
    // 语义名（如 `ui.card-title-1`）的唯一理由。
    ("en", "ui.zephyraui-gallery", "ZephyraUI Gallery"),
    (
        "en",
        "ui.search-controls-and-samples",
        "Search controls and samples...",
    ),
    ("en", "ui.interface", "Interface"),
    ("en", "ui.language", "Language"),
    ("en", "ui.theme", "Theme"),
    ("en", "ui.route", "Route"),
    ("en", "ui.button", "Button"),
    ("en", "ui.variants", "Variants"),
    ("en", "ui.standard", "Standard"),
    ("en", "ui.accent", "Accent"),
    ("en", "ui.subtle", "Subtle"),
    ("en", "ui.primary", "Primary"),
    ("en", "ui.pill", "Pill"),
    ("en", "ui.with-icon-round", "With icon / round"),
    ("en", "ui.send", "Send"),
    ("en", "ui.click-me", "Click me"),
    ("en", "ui.counter", "Counter"),
    ("en", "ui.one-state", "One state"),
    ("en", "ui.multiple", "Multiple"),
    ("en", "ui.single", "Single"),
    ("en", "ui.pick-one", "Pick one"),
    ("en", "ui.text-input", "Text input"),
    ("en", "ui.text-input-display", "Text input (display)"),
    ("en", "ui.your-name", "Your name"),
    ("en", "ui.multi-line-notes", "Multi-line notes"),
    ("en", "ui.second-line", "second line"),
    ("en", "ui.third-line", "third line"),
    ("en", "ui.value-pickers", "Value pickers"),
    ("en", "ui.value", "Value"),
    ("en", "ui.slider", "Slider"),
    ("en", "ui.slider-display", "Slider (display)"),
    ("en", "ui.colorpicker", "ColorPicker"),
    ("en", "ui.combobox", "ComboBox"),
    ("en", "ui.checkbox", "CheckBox"),
    ("en", "ui.toggle", "Toggle"),
    ("en", "ui.progressbar", "ProgressBar"),
    ("en", "ui.ring", "Ring"),
    ("en", "ui.bar", "Bar"),
    ("en", "ui.busy", "Busy"),
    ("en", "ui.list", "List"),
    ("en", "ui.table", "Table"),
    ("en", "ui.selected-cell", "Selected cell"),
    ("en", "ui.avatar", "Avatar"),
    ("en", "ui.sizes", "Sizes"),
    ("en", "ui.dialog", "Dialog"),
    ("en", "ui.pickers", "Pickers"),
    ("en", "ui.date", "Date"),
    ("en", "ui.time", "Time"),
    ("en", "ui.segmented", "Segmented"),
    ("en", "ui.states", "States"),
    ("en", "ui.position", "Position"),
    ("en", "ui.range", "Range"),
    ("en", "ui.tools", "Tools"),
    ("en", "ui.severities", "Severities"),
    ("en", "ui.sample", "Sample"),
    ("en", "ui.backdrop", "Backdrop"),
    ("en", "ui.cost", "Cost"),
    ("en", "ui.available", "Available"),
    ("en", "ui.card", "Card"),
    ("en", "ui.translucent-card", "Translucent card"),
    ("en", "ui.translucent", "Translucent"),
    ("en", "ui.a-plain-surface", "A plain surface."),
    (
        "en",
        "ui.same-class-other-content",
        "Same class, other content.",
    ),
    ("en", "ui.token-colored-text", "Token-colored text"),
    ("en", "ui.spacing-demo", "Spacing demo"),
    ("en", "ui.what-they-guard", "What they guard"),
    ("en", "ui.you-are-here", "You are here"),
    ("en", "ui.tooltip", "ToolTip"),
    ("en", "ui.tool-tip", "Tool tip"),
    ("en", "ui.a-tooltip", "A tooltip"),
    ("en", "ui.check-a-stylesheet", "Check a stylesheet"),
    (
        "en",
        "ui.click-the-button-the-count-goes-up-that-is-the-whole-pipeline",
        "Click the button. The count goes up. That is the whole pipeline.",
    ),
    ("en", "ui.dump-current-route", "Dump current route"),
    ("en", "ui.dump-a-specific-route", "Dump a specific route"),
    ("en", "ui.run-all-gallery-gates", "Run all gallery gates"),
    ("en", "ui.toggle-theme", "Toggle theme"),
    ("en", "ui.toggle-mica", "Toggle Mica"),
    ("en", "ui.restore", "Restore"),
    ("en", "ui.current-breakpoint", "Current breakpoint"),
    ("en", "ui.card-1", "Card 1"),
    ("en", "ui.card-2", "Card 2"),
    ("en", "ui.card-3", "Card 3"),
    ("en", "ui.first", "First"),
    ("en", "ui.second", "Second"),
    ("en", "ui.third", "Third"),
    ("en", "ui.fits-inside-the-card", "Fits inside the card"),
    ("en", "ui.also-fits", "Also fits"),
    (
        "en",
        "ui.watch-me-when-the-window-shrinks",
        "Watch me when the window shrinks",
    ),
    ("en", "ui.hidden-until-expanded", "Hidden until expanded."),
    (
        "en",
        "ui.no-scroll-overflow-is-clipped",
        "No scroll — overflow is clipped",
    ),
    ("en", "ui.44-epx-correct", "44 epx (correct)"),
    ("en", "ui.28-epx-too-small", "28 epx (too small)"),
    (
        "en",
        "ui.the-quick-brown-fox-jumps-over-the-lazy-dog",
        "The quick brown fox jumps over the lazy dog.",
    ),
    (
        "en",
        "ui.refraction-samples-what-is-already-on-screen",
        "Refraction samples what is already on screen.",
    ),
    (
        "en",
        "ui.remove-these-lines-and-the-glass-looks-broken",
        "Remove these lines and the glass looks broken.",
    ),
    (
        "en",
        "ui.rendered-inline-the-core-has-no-overlay-layer-yet",
        "Rendered inline — the core has no overlay layer yet.",
    ),
    ("en", "ui.see-the-effects-page", "See the effects page."),
    (
        "en",
        "ui.liquid-glass-refraction-parameters-are-runtime-adjustable",
        "Liquid glass: refraction parameters are runtime adjustable",
    ),
    ("en", "ui.refraction-parameters", "Refraction parameters"),
    ("en", "ui.thin", "Thin"),
    ("en", "ui.normal", "Normal"),
    ("en", "ui.thick", "Thick"),
    // ---- 布局页：flex ----
    (
        "en",
        "ui.flex-rows-that-grow-and-wrap-and-the-one-boundary-that-is-still-real-no-scroll",
        "Flex rows that grow and wrap — and the one boundary that is still real: no scroll.",
    ),
    (
        "en",
        "ui.flexbox-rows-that-grow-and-wrap",
        "Flexbox: rows that grow and wrap",
    ),
    (
        "en",
        "ui.flex-grow-1-fills-the-row-and-flex-wrap-packs-items-into-lines",
        "flex-grow: 1 fills the row · flex-wrap packs items into lines",
    ),
    ("en", "ui.fixed", "Fixed"),
    ("en", "ui.grows", "Grows"),
    // ---- zh-Hans ----
    ("zh-Hans", "app.title", "ZephyraUI 示例库"),
    ("zh-Hans", "route.guide", "快速开始"),
    ("zh-Hans", "route.state", "状态"),
    ("zh-Hans", "route.styling", "样式"),
    ("zh-Hans", "route.layout", "布局"),
    ("zh-Hans", "route.theme", "主题与多语言"),
    ("zh-Hans", "route.testing", "测试"),
    ("zh-Hans", "route.reference", "控件参考"),
    ("zh-Hans", "route.glass", "液态玻璃"),
    (
        "zh-Hans",
        "ui.small-nav-collapsed-to-icons",
        "小 — 导航收成图标列",
    ),
    ("zh-Hans", "ui.medium-nav-shows-labels", "中 — 导航显示文字"),
    ("zh-Hans", "ui.large-full-layout", "大 — 完整布局"),
    ("zh-Hans", "route.settings", "设置"),
    ("zh-Hans", "action.feedback", "反馈"),
    ("zh-Hans", "action.language", "语言"),
    ("zh-Hans", "set.theme.title", "深色模式"),
    ("zh-Hans", "set.theme.desc", "换主题只换令牌值"),
    ("zh-Hans", "set.volume.title", "音量"),
    ("zh-Hans", "set.volume.desc", "填充宽度 = 值 × 轨道宽"),
    ("zh-Hans", "set.notify.title", "通知"),
    ("zh-Hans", "set.notify.desc", "选中态走分类名"),
    ("zh-Hans", "set.adv.title", "高级"),
    ("zh-Hans", "set.adv.logging", "诊断日志"),
    ("zh-Hans", "set.adv.hwaccel", "硬件加速"),
    ("zh-Hans", "set.adv.shortcuts", "快捷键"),
    // ---- 页面正文（同 en 组，键序一致）----
    ("zh-Hans", "ui.zephyraui-gallery", "ZephyraUI 示例库"),
    (
        "zh-Hans",
        "ui.search-controls-and-samples",
        "搜索控件与示例……",
    ),
    ("zh-Hans", "ui.interface", "界面"),
    ("zh-Hans", "ui.language", "语言"),
    ("zh-Hans", "ui.theme", "主题"),
    ("zh-Hans", "ui.route", "路由"),
    ("zh-Hans", "ui.button", "按钮"),
    ("zh-Hans", "ui.variants", "变体"),
    ("zh-Hans", "ui.standard", "标准"),
    ("zh-Hans", "ui.accent", "强调"),
    ("zh-Hans", "ui.subtle", "低调"),
    ("zh-Hans", "ui.primary", "主要"),
    ("zh-Hans", "ui.pill", "胶囊"),
    ("zh-Hans", "ui.with-icon-round", "带图标 / 圆形"),
    ("zh-Hans", "ui.send", "发送"),
    ("zh-Hans", "ui.click-me", "点我"),
    ("zh-Hans", "ui.counter", "计数器"),
    ("zh-Hans", "ui.one-state", "单选"),
    ("zh-Hans", "ui.multiple", "多选"),
    ("zh-Hans", "ui.single", "单选"),
    ("zh-Hans", "ui.pick-one", "选一个"),
    ("zh-Hans", "ui.text-input", "文本输入"),
    ("zh-Hans", "ui.text-input-display", "文本输入（展示）"),
    ("zh-Hans", "ui.your-name", "你的名字"),
    ("zh-Hans", "ui.multi-line-notes", "多行笔记"),
    ("zh-Hans", "ui.second-line", "第二行"),
    ("zh-Hans", "ui.third-line", "第三行"),
    ("zh-Hans", "ui.value-pickers", "取值选择器"),
    ("zh-Hans", "ui.value", "值"),
    ("zh-Hans", "ui.slider", "滑块"),
    ("zh-Hans", "ui.slider-display", "滑块（展示）"),
    ("zh-Hans", "ui.colorpicker", "颜色选择器"),
    ("zh-Hans", "ui.combobox", "组合框"),
    ("zh-Hans", "ui.checkbox", "复选框"),
    ("zh-Hans", "ui.toggle", "开关"),
    ("zh-Hans", "ui.progressbar", "进度条"),
    ("zh-Hans", "ui.ring", "环形"),
    ("zh-Hans", "ui.bar", "条"),
    ("zh-Hans", "ui.busy", "忙碌"),
    ("zh-Hans", "ui.list", "列表"),
    ("zh-Hans", "ui.table", "表格"),
    ("zh-Hans", "ui.selected-cell", "选中单元格"),
    ("zh-Hans", "ui.avatar", "头像"),
    ("zh-Hans", "ui.sizes", "尺寸"),
    ("zh-Hans", "ui.dialog", "对话框"),
    ("zh-Hans", "ui.pickers", "选择器"),
    ("zh-Hans", "ui.date", "日期"),
    ("zh-Hans", "ui.time", "时间"),
    ("zh-Hans", "ui.segmented", "分段"),
    ("zh-Hans", "ui.states", "状态"),
    ("zh-Hans", "ui.position", "位置"),
    ("zh-Hans", "ui.range", "区间"),
    ("zh-Hans", "ui.tools", "工具"),
    ("zh-Hans", "ui.severities", "严重级别"),
    ("zh-Hans", "ui.sample", "示例"),
    ("zh-Hans", "ui.backdrop", "背景"),
    ("zh-Hans", "ui.cost", "成本"),
    ("zh-Hans", "ui.available", "可用"),
    ("zh-Hans", "ui.card", "卡片"),
    ("zh-Hans", "ui.translucent-card", "半透明卡片"),
    ("zh-Hans", "ui.translucent", "半透明"),
    ("zh-Hans", "ui.a-plain-surface", "一块纯色底板。"),
    (
        "zh-Hans",
        "ui.same-class-other-content",
        "同一个分类名，另一段内容。",
    ),
    ("zh-Hans", "ui.token-colored-text", "令牌着色的文字"),
    ("zh-Hans", "ui.spacing-demo", "间距演示"),
    ("zh-Hans", "ui.what-they-guard", "它们守住什么"),
    ("zh-Hans", "ui.you-are-here", "你在这里"),
    ("zh-Hans", "ui.tooltip", "提示"),
    ("zh-Hans", "ui.tool-tip", "提示气泡"),
    ("zh-Hans", "ui.a-tooltip", "一个提示气泡"),
    ("zh-Hans", "ui.check-a-stylesheet", "校验样式表"),
    (
        "zh-Hans",
        "ui.click-the-button-the-count-goes-up-that-is-the-whole-pipeline",
        "点一下按钮，计数加一 —— 整条管线就这些。",
    ),
    ("zh-Hans", "ui.dump-current-route", "导出当前路由"),
    ("zh-Hans", "ui.dump-a-specific-route", "导出指定路由"),
    ("zh-Hans", "ui.run-all-gallery-gates", "跑全部画廊门禁"),
    ("zh-Hans", "ui.toggle-theme", "切换主题"),
    ("zh-Hans", "ui.toggle-mica", "切换 Mica"),
    ("zh-Hans", "ui.restore", "恢复"),
    ("zh-Hans", "ui.current-breakpoint", "当前断点"),
    ("zh-Hans", "ui.card-1", "卡片 1"),
    ("zh-Hans", "ui.card-2", "卡片 2"),
    ("zh-Hans", "ui.card-3", "卡片 3"),
    ("zh-Hans", "ui.first", "第一"),
    ("zh-Hans", "ui.second", "第二"),
    ("zh-Hans", "ui.third", "第三"),
    ("zh-Hans", "ui.fits-inside-the-card", "装得进卡片"),
    ("zh-Hans", "ui.also-fits", "也放得下"),
    (
        "zh-Hans",
        "ui.watch-me-when-the-window-shrinks",
        "窗口变小时看我",
    ),
    ("zh-Hans", "ui.hidden-until-expanded", "展开前看不到。"),
    (
        "zh-Hans",
        "ui.no-scroll-overflow-is-clipped",
        "无滚动 — 超出即裁剪",
    ),
    ("zh-Hans", "ui.44-epx-correct", "44 epx（正确）"),
    ("zh-Hans", "ui.28-epx-too-small", "28 epx（太小）"),
    (
        "zh-Hans",
        "ui.the-quick-brown-fox-jumps-over-the-lazy-dog",
        "敏捷的棕色狐狸跃过懒狗。",
    ),
    (
        "zh-Hans",
        "ui.refraction-samples-what-is-already-on-screen",
        "折射采样的是屏幕上已有的内容。",
    ),
    (
        "zh-Hans",
        "ui.remove-these-lines-and-the-glass-looks-broken",
        "删掉这几行，玻璃就像坏了一样。",
    ),
    (
        "zh-Hans",
        "ui.rendered-inline-the-core-has-no-overlay-layer-yet",
        "内联渲染 —— core 还没有覆盖层。",
    ),
    ("zh-Hans", "ui.see-the-effects-page", "看效果页。"),
    (
        "zh-Hans",
        "ui.liquid-glass-refraction-parameters-are-runtime-adjustable",
        "液态玻璃：折射参数运行期可调",
    ),
    ("zh-Hans", "ui.refraction-parameters", "折射参数"),
    ("zh-Hans", "ui.thin", "薄"),
    ("zh-Hans", "ui.normal", "普通"),
    ("zh-Hans", "ui.thick", "厚"),
    // ---- 布局页：flex ----
    (
        "zh-Hans",
        "ui.flex-rows-that-grow-and-wrap-and-the-one-boundary-that-is-still-real-no-scroll",
        "会填充、会折行的 flex 行 —— 而仍然真实的边界只剩一条：无滚动。",
    ),
    (
        "zh-Hans",
        "ui.flexbox-rows-that-grow-and-wrap",
        "Flexbox：会填充、会折行的行",
    ),
    (
        "zh-Hans",
        "ui.flex-grow-1-fills-the-row-and-flex-wrap-packs-items-into-lines",
        "左：flex-grow: 1 把整行填满 · 右：flex-wrap 把内容折成多行",
    ),
    ("zh-Hans", "ui.fixed", "固定"),
    ("zh-Hans", "ui.grows", "填充"),
    // ---- ja ----
    ("ja", "app.title", "ZephyraUI ギャラリー"),
    ("ja", "route.guide", "クイックスタート"),
    ("ja", "route.state", "状態"),
    ("ja", "route.styling", "スタイリング"),
    ("ja", "route.layout", "レイアウト"),
    ("ja", "route.theme", "テーマと多言語"),
    ("ja", "route.testing", "テスト"),
    ("ja", "route.reference", "コントロール"),
    ("ja", "route.glass", "リキッドグラス"),
    (
        "ja",
        "ui.small-nav-collapsed-to-icons",
        "小 — ナビはアイコン列に折りたたみ",
    ),
    ("ja", "ui.medium-nav-shows-labels", "中 — ナビにラベル表示"),
    ("ja", "ui.large-full-layout", "大 — フルレイアウト"),
    ("ja", "route.settings", "設定"),
    ("ja", "action.feedback", "フィードバック"),
    ("ja", "action.language", "言語"),
    ("ja", "set.theme.title", "ダークモード"),
    (
        "ja",
        "set.theme.desc",
        "テーマを切り替えてもトークンの値だけが変わる",
    ),
    ("ja", "set.volume.title", "音量"),
    ("ja", "set.volume.desc", "塗りの幅 = 値 x トラックの幅"),
    ("ja", "set.notify.title", "通知"),
    ("ja", "set.notify.desc", "選択状態はクラス名で表す"),
    ("ja", "set.adv.title", "詳細"),
    ("ja", "set.adv.logging", "診断ログ"),
    ("ja", "set.adv.hwaccel", "ハードウェアアクセラレーション"),
    ("ja", "set.adv.shortcuts", "ショートカット"),
    // ---- ページ本文（en と同順）----
    ("ja", "ui.zephyraui-gallery", "ZephyraUI ギャラリー"),
    (
        "ja",
        "ui.search-controls-and-samples",
        "コントロールとサンプルを検索…",
    ),
    ("ja", "ui.interface", "インターフェース"),
    ("ja", "ui.language", "言語"),
    ("ja", "ui.theme", "テーマ"),
    ("ja", "ui.route", "ルート"),
    ("ja", "ui.button", "ボタン"),
    ("ja", "ui.variants", "バリエーション"),
    ("ja", "ui.standard", "標準"),
    ("ja", "ui.accent", "アクセント"),
    ("ja", "ui.subtle", "サブトル"),
    ("ja", "ui.primary", "プライマリ"),
    ("ja", "ui.pill", "ピル"),
    ("ja", "ui.with-icon-round", "アイコン付き / 丸"),
    ("ja", "ui.send", "送信"),
    ("ja", "ui.click-me", "クリックして"),
    ("ja", "ui.counter", "カウンター"),
    ("ja", "ui.one-state", "単一選択"),
    ("ja", "ui.multiple", "複数選択"),
    ("ja", "ui.single", "単一"),
    ("ja", "ui.pick-one", "ひとつ選択"),
    ("ja", "ui.text-input", "テキスト入力"),
    ("ja", "ui.text-input-display", "テキスト入力（表示）"),
    ("ja", "ui.your-name", "お名前"),
    ("ja", "ui.multi-line-notes", "複数行のメモ"),
    ("ja", "ui.second-line", "2行目"),
    ("ja", "ui.third-line", "3行目"),
    ("ja", "ui.value-pickers", "値のピッカー"),
    ("ja", "ui.value", "値"),
    ("ja", "ui.slider", "スライダー"),
    ("ja", "ui.slider-display", "スライダー（表示）"),
    ("ja", "ui.colorpicker", "カラーピッカー"),
    ("ja", "ui.combobox", "コンボボックス"),
    ("ja", "ui.checkbox", "チェックボックス"),
    ("ja", "ui.toggle", "トグル"),
    ("ja", "ui.progressbar", "プログレスバー"),
    ("ja", "ui.ring", "リング"),
    ("ja", "ui.bar", "バー"),
    ("ja", "ui.busy", "ビジー"),
    ("ja", "ui.list", "リスト"),
    ("ja", "ui.table", "テーブル"),
    ("ja", "ui.selected-cell", "選択したセル"),
    ("ja", "ui.avatar", "アバター"),
    ("ja", "ui.sizes", "サイズ"),
    ("ja", "ui.dialog", "ダイアログ"),
    ("ja", "ui.pickers", "ピッカー"),
    ("ja", "ui.date", "日付"),
    ("ja", "ui.time", "時刻"),
    ("ja", "ui.segmented", "セグメント"),
    ("ja", "ui.states", "状態"),
    ("ja", "ui.position", "位置"),
    ("ja", "ui.range", "範囲"),
    ("ja", "ui.tools", "ツール"),
    ("ja", "ui.severities", "重要度"),
    ("ja", "ui.sample", "サンプル"),
    ("ja", "ui.backdrop", "背景"),
    ("ja", "ui.cost", "コスト"),
    ("ja", "ui.available", "利用可能"),
    ("ja", "ui.card", "カード"),
    ("ja", "ui.translucent-card", "半透明カード"),
    ("ja", "ui.translucent", "半透明"),
    ("ja", "ui.a-plain-surface", "ただのサーフェス。"),
    (
        "ja",
        "ui.same-class-other-content",
        "同じクラス、別の内容。",
    ),
    ("ja", "ui.token-colored-text", "トークンで着色した文字"),
    ("ja", "ui.spacing-demo", "間隔のデモ"),
    ("ja", "ui.what-they-guard", "何を守っているか"),
    ("ja", "ui.you-are-here", "現在地"),
    ("ja", "ui.tooltip", "ツールヒント"),
    ("ja", "ui.tool-tip", "ツールチップ"),
    ("ja", "ui.a-tooltip", "ツールチップひとつ"),
    ("ja", "ui.check-a-stylesheet", "スタイルシートを検証"),
    (
        "ja",
        "ui.click-the-button-the-count-goes-up-that-is-the-whole-pipeline",
        "ボタンを押すとカウントが増える。それだけがこのパイプラインです。",
    ),
    ("ja", "ui.dump-current-route", "現在のルートをダンプ"),
    ("ja", "ui.dump-a-specific-route", "指定ルートをダンプ"),
    (
        "ja",
        "ui.run-all-gallery-gates",
        "ギャラリーの全ゲートを実行",
    ),
    ("ja", "ui.toggle-theme", "テーマを切り替え"),
    ("ja", "ui.toggle-mica", "Mica を切り替え"),
    ("ja", "ui.restore", "復元"),
    ("ja", "ui.current-breakpoint", "現在のブレークポイント"),
    ("ja", "ui.card-1", "カード 1"),
    ("ja", "ui.card-2", "カード 2"),
    ("ja", "ui.card-3", "カード 3"),
    ("ja", "ui.first", "1つ目"),
    ("ja", "ui.second", "2つ目"),
    ("ja", "ui.third", "3つ目"),
    ("ja", "ui.fits-inside-the-card", "カードに収まる"),
    ("ja", "ui.also-fits", "こちらも収まる"),
    (
        "ja",
        "ui.watch-me-when-the-window-shrinks",
        "ウィンドウを縮めると起きる",
    ),
    ("ja", "ui.hidden-until-expanded", "展開するまで隠れる。"),
    (
        "ja",
        "ui.no-scroll-overflow-is-clipped",
        "スクロールなし — はみ出しは切られる",
    ),
    ("ja", "ui.44-epx-correct", "44 epx（正しい）"),
    ("ja", "ui.28-epx-too-small", "28 epx（小さすぎる）"),
    (
        "ja",
        "ui.the-quick-brown-fox-jumps-over-the-lazy-dog",
        "素早い茶色のキツネがのろまな犬を飛び越える。",
    ),
    (
        "ja",
        "ui.refraction-samples-what-is-already-on-screen",
        "屈折は画面に既にあるものをサンプリングする。",
    ),
    (
        "ja",
        "ui.remove-these-lines-and-the-glass-looks-broken",
        "この行を消すとガラスが壊れて見える。",
    ),
    (
        "ja",
        "ui.rendered-inline-the-core-has-no-overlay-layer-yet",
        "インライン描画 — core にはまだオーバーレイ層がない。",
    ),
    ("ja", "ui.see-the-effects-page", "エフェクトページを見る。"),
    (
        "ja",
        "ui.liquid-glass-refraction-parameters-are-runtime-adjustable",
        "リキッドグラス：屈折のパラメータは実行時に変えられる",
    ),
    ("ja", "ui.refraction-parameters", "屈折のパラメータ"),
    ("ja", "ui.thin", "薄い"),
    ("ja", "ui.normal", "普通"),
    ("ja", "ui.thick", "厚い"),
    // ---- レイアウト頁：flex ----
    (
        "ja",
        "ui.flex-rows-that-grow-and-wrap-and-the-one-boundary-that-is-still-real-no-scroll",
        "伸びて折り返す flex 行 —— それでも残る境界は「スクロールなし」だけ。",
    ),
    (
        "ja",
        "ui.flexbox-rows-that-grow-and-wrap",
        "Flexbox：伸びる行と折り返す行",
    ),
    (
        "ja",
        "ui.flex-grow-1-fills-the-row-and-flex-wrap-packs-items-into-lines",
        "左：flex-grow: 1 が行を埋める · 右：flex-wrap が項目を折り返す",
    ),
    ("ja", "ui.fixed", "固定"),
    ("ja", "ui.grows", "伸びる"),
];

/// 本表里出现过的**全部**键（按首次出现去重）。
///
/// 门禁拿它当"应该存在的键集合"：一张平表天然可以枚举，这是选平表而不是
/// 分语言分桶的**唯一**理由（见 `CATALOG` 的说明）。
pub fn catalog_keys() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (_, k, _) in CATALOG {
        if !out.contains(k) {
            out.push(k);
        }
    }
    out
}

/// 应用内的消息表。
///
/// # 为什么文案在这里而不在控件里
///
/// 源码里**没有面向用户的字符串** —— 与"源码零外观"是同一条纪律：
/// 控件发分类名、不发颜色；页面发 key、不发文案。漏翻由门禁枚举
/// （`every_tr_key_used_in_source_exists_in_every_language`），不靠肉眼。
pub fn messages() -> Messages {
    let mut m = Messages::new();
    for (locale, key, value) in CATALOG {
        m.insert(locale, key, value);
    }
    m
}
/// 路由 id → 文案键（`home` ⇒ `route.home`）。
///
/// # 为什么是**查表**而不是 `concat!("route.{id}")`
///
/// `view()` 每帧重建整棵树，任何运行期拼出来的字符串都会变成泄漏
/// （`Box::leak` 每帧一次，见 `element.rs` 的记录）。而键必须 `&'static`
/// 才能喂给 [`Gallery::tr`]。查表天然返回 `&'static str`。
///
/// 找不到时**返回 id 本身**而不是 `panic!` 或空串：调用方随后会拿它当键去
/// `tr()`，而 `tr()` 的最后一档正是"返回键名"，于是画面上出现路由 id ——
/// 看得见的降级。真正拦住这件事的是门禁
/// `every_route_id_is_translated_in_every_supported_language`（新增路由忘了加译文会红）。
pub fn route_key(id: &str) -> &'static str {
    // 只在默认语言那一组里找键 —— 键集合与语言无关，看第一组就够且更快。
    for (locale, key, _) in CATALOG {
        if *locale != SUPPORTED[0] {
            continue;
        }
        if let Some(rest) = key.strip_prefix("route.") {
            if rest == id {
                return key;
            }
        }
    }
    // 静态回退：id 是 `&'static`（来自 `ROUTES` / `SETTINGS_ROUTE`），
    // 所以这里不需要额外分配。若调用方传了非静态串，门禁会先拦住。
    ROUTES
        .iter()
        .chain(std::iter::once(&SETTINGS_ROUTE))
        .find(|(rid, _, _)| *rid == id)
        .map(|(rid, _, _)| *rid)
        .unwrap_or("route.unknown")
}

/// 解析一个 key 是否为合法路由，是则返回该路由 id（`&'static`）。
///
/// # 路由来源有三处，缺一不可
///
/// 1. 10 个页面路由（[`ROUTES`]）—— 导航栏项；
/// 2. 14 个分组索引路由（`catalog::GROUP_ROUTES`）—— All 页的分组条；
/// 3. 44 个控件卡（`catalog::ENTRIES` 的 `page`）—— 卡片点击进该控件页。
///
/// 三处都用**查表**而非 `match` 列举：任一处新增 id 都只改那一处表，
/// 不用回来改这个函数（枚举式会把新路由漏在 match 的 `_` 分支里，静默回 Home）。
///
/// 返回 `&'static str`：路由要存进 [`Gallery::route`]（`&'static`），
/// 解析一个 key 是否为合法路由，是则返回该路由 id（`&'static str`）。
///
/// # 路由来源只剩两处
///
/// 旧结构有三处（10 页 + 14 分组 + 44 控件页），后两处都来自
/// `catalog.rs`。那个模块随重写一并删掉，所以这里只剩 [`ROUTES`] 与
/// [`SETTINGS_ROUTE`] 两个来源。
///
/// 两处都用**查表**：新增 id 只改那一个表，不用回来改这个函数。
/// 枚举式（`match`）会把新路由漏在 `_ => home` 里，静默回首页。
///
/// 返回 `&'static str`：路由要存进 [`Gallery::route`]（`&'static`），
/// 不能在运行时拼字符串 —— 那会每帧泄漏。
pub fn resolve_route(key: &str) -> Option<&'static str> {
    ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .find(|id| *id == key)
}

/// 列出全部合法路由（`--list-routes` 用）。
///
/// 之所以要“列出来”而不是让人猜：出图/调试时拼错一个 id 的后果是
/// **静默回到 home**（历史上已发生过）。把全集打印出来，拼错立刻可见。
pub fn list_routes() -> Vec<&'static str> {
    ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect()
}

/// Settings 路由（置底那一项）。
pub const SETTINGS_ROUTE: (&str, &str, &str) = ("settings", "Settings", "settings");

// ---- 参考层与指南章的**交互 key 常量表** ----
//
// # 为什么这批 key 住在 mod.rs 而不是各页面
//
// `Gallery::click` 收到的 key 是运行期 `&str`（`on_input` 里 `to_string()`
// 转交），而选中态字段是 `&'static str` —— 直接赋值存不下来。把 key 表
// 定义在这里，页面发的是表里那几项、click 按表还原，**同一份常量**服务
// 两处：key 表就是「哪些 key 会改状态」的清单，与门禁
// `every_interactive_key_is_wired_or_whitelisted` 的白名单共用。

/// state 章 segmented 的三段。
pub const DEMO_SEGS: [&str; 3] = ["stseg:a", "stseg:b", "stseg:c"];
/// navigation 族 nav_sub_item 的两个子项。
pub const SUBDEMO_IDS: [&str; 2] = ["subdemo:a", "subdemo:b"];
/// navigation 族 selector_bar 的三段。
pub const SELDEMO_IDS: [&str; 3] = ["seldemo:day", "seldemo:week", "seldemo:month"];
/// 触发类按钮（basics / layout / styling / menus / 外壳页头共用 `last_action`）。
///
/// ⚠ 新增触发按钮时把 key 加进这张表——漏加的症状是「点了没反应」，
/// 而门禁 `every_interactive_key_is_wired_or_whitelisted` 会先一步红。
pub const ACT_IDS: [&str; 13] = [
    "act:standard",
    "act:accent",
    "act:subtle",
    "act:send",
    "act:touch-ok",
    "act:touch-bad",
    "act:style-primary",
    "act:style-subtle",
    "act:style-standard",
    "act:feedback",
    "act:tool-save",
    "act:tool-search",
    "act:tool-gear",
];

/// 把运行期 key 解析回编译期常量原身。
///
/// # 为什么需要它
///
/// 见上面这批常量表的说明。`Option` 而不是直接下标：收到表外的 key 时
/// **不动状态** —— 那比 panic 好（一个 key 拼错不该崩掉整个应用），也比
/// 「静默写进一个错误的值」好（字段内容仍是合法项，画面不出现怪态）。
fn static_key<'a>(candidates: &[&'a str], key: &str) -> Option<&'a str> {
    candidates.iter().copied().find(|c| *c == key)
}

/// time_picker 的 item key（`tph:13` / `tpm:05`）→ 显示值（`"13"` / `"05"`）。
///
/// 与 dialogs.rs 的 `HOURS` / `MINUTES` 常量对齐：改那边必须改这里 ——
/// 门禁 `reference_page_has_all_families` 与交互回归会踩到不一致。
fn time_part(key: &str) -> &'static str {
    match key {
        "tph:13" => "13",
        "tph:14" => "14",
        "tph:15" => "15",
        "tpm:00" => "00",
        "tpm:05" => "05",
        "tpm:10" => "10",
        _ => "",
    }
}

/// 演示开关的三个 id（Basic Input 页）。
pub const CHECKS: [&str; 3] = ["check:alpha", "check:beta", "check:gamma"];

/// ComboBox 演示选项（`'static`：控件签名收静态切片，与 demo 的常量同手法）。
pub const COMBO_OPTIONS: &[&str] = &["Mica", "Acrylic", "Tabbed", "None"];

/// Gallery 应用状态。
#[derive(Debug, Clone)]
pub struct Gallery {
    /// 当前路由 id。
    pub route: &'static str,
    /// 当前 hover 的 key（来自 InputState）。
    pub hovered: Option<ElementKey>,
    /// 当前按住的 key（捕获语义）。
    pub pressed: Option<ElementKey>,
    /// 上一帧的 pressed —— 用来还原 click。
    pub last_pressed: Option<ElementKey>,
    /// 想要深色主题（意图；宿主执行）。
    pub wants_dark: bool,
    /// 主题意图的共享格（窗口路用；dump 路直接读 `wants_dark`）。
    pub theme_intent: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
    /// Home 页分段选中项。
    pub seg: &'static str,
    /// 选择页的三个复选框（各自独立开关）。
    pub checked: [bool; 3],
    /// 单选组的**选中下标**（落在 `RADIO_ITEMS` 里）。
    ///
    /// # 为什么单选还要单独一个字段，不能复用 `checked`
    ///
    /// 单选的语义是「**有且仅有一个**被选中」，多选是「多个独立开关」。若拿那个布尔
    /// 数组表示单选，就得额外维护一套「什么时候清掉别的那个」规则，而规则一变就
    /// 出现「两个同时选中」—— 那正是画面上可见、且不报错的失败。
    pub radio: usize,
    /// 液态玻璃页的四个折射参数（**运行期可调**）。
    ///
    /// # 为什么它住在应用状态而不是样式表
    ///
    /// 参数写进 `.zstyle` 就只剩三档静态值，而这个页面的职责恰恰是让人**调**
    /// 它们。见 [`glass::Glass`] 的文档 —— `FILTER` 不在门禁的 APPEARANCE
    /// 列表里是**有意的**：它的四个参数是物理量（厚度 / 折射率 / 色散 / 倒角
    /// 宽度），既没有 `@令牌` 对应物，也不随主题变化。
    pub glass: crate::app::glass::Glass,
    /// 设置页主题总开关。
    pub switch_on: bool,
    /// 设置页通知开关。与主题开关**分开**：共用一个字段的话，勾掉通知会把
    /// 深色模式一起关掉 —— 两个毫不相干的设置被绑在一起，而画面上毫无异样。
    pub notify_on: bool,
    /// 设置页滑条值（0..1）。
    pub volume: f32,
    /// 设置页展开器是否展开。
    pub expanded: bool,
    /// Basic Input 页：ComboBox 选中项下标。
    pub combo: usize,
    /// Basic Input 页：SpinBox 的数值。
    pub spin: i32,
    /// Basic Input 页：DropDownButton 的展开态。
    pub dd_open: bool,
    /// 是否请求 Mica 材质外观（标题栏半透明）。true = 应用想要；false = 实色。
    /// 平台能不能真的开由 zephyra-platform-win 回答，不影响这里。
    pub mica: bool,
    /// Menus 演示：菜单栏当前展开项（None = 全收起）。
    pub menu_open: Option<usize>,
    /// 客户区视口，**物理像素**。宿主每帧经 `on_input` 推进来。
    ///
    /// 初始值取基准画布尺寸（见 [`crate::app::BASE_VIEWPORT`]）：dump 路没有宿主，
    /// 于是静态渲染下的断点与 1400×800 窗口一致 —— 否则 `--dump-cpu` 出的图
    /// 会按"小型窗口"排，与窗口路对不上，而那条对照正是参考图的全部价值。
    pub viewport: (f32, f32),
    /// 视口缩放比。它与 [`Self::viewport`] 一起才能算出有效像素（断点口径）。
    /// 初始 1.0 = 「dump 路当作未缩放处理」，与上面同源。
    pub scale: f32,
    /// 控件参考页当前选中的**族下标**（落在 [`REF_FAMILIES`] 里）。
    ///
    /// # 为什么单独一个字段而不是复用 `seg`
    ///
    /// `seg` 是 toggles 页的分段，`ref_family` 是参考页的分段 —— 两者
    /// 各管一页。共用一个字段会让「在参考页选了 Layout」污染「toggles 页
    /// 的分段选中项」，而画面上两页各自的高亮是对的、只有切回去才发现不对。
    pub ref_family: usize,
    /// 快速开始章的按钮计数。
    ///
    /// # 为什么单独一个字段而不是复用 `spin`
    ///
    /// `spin` 是 Basic Input 页 SpinBox 的值，语义是「0..100 的整数输入」。
    /// 复用它会让「点快速开始章的按钮」去改 SpinBox 的显示 —— 而那个
    /// SpinBox 在另一页。开发者会看到「点这个按钮，那个页面的某个东西变了」，
    /// 而画面上没有任何线索把两者连起来。
    pub guide_count: u32,
    /// 最近一次**触发类按钮**的动作名（basics / layout / styling 章共用）。
    ///
    /// # 为什么按钮需要它
    ///
    /// 按钮的语义是「触发动作」。示例项目里的按钮如果没有可见反馈，
    /// 教给开发者的就是「这个框架的按钮是假的」。所以每个触发类按钮点了
    /// 都把动作名写进来，basics 页顶部的状态行显示它 —— 「点击 → 状态 →
    /// 视图」这条链当场可验。
    pub last_action: &'static str,
    /// basics 页 pill_button 的选中态（pill = 可保持按下的按钮）。
    pub pill_on: bool,
    /// basics 页 color_picker 选中色格下标。
    pub color_idx: usize,
    /// state 章 checkbox（独立于 toggles 页的 `checked`：教学页要自证）。
    pub demo_check: bool,
    /// state 章 segmented 选中段（`stseg:a` / `stseg:b` / `stseg:c`）。
    pub demo_seg: &'static str,
    /// state 章 combo 循环下标。
    pub demo_combo: usize,
    /// navigation 族 nav_sub_item 选中项（`subdemo:a` / `subdemo:b`）。
    pub subsel: &'static str,
    /// navigation 族 selector_bar 选中段（`seldemo:day/week/month`）。
    pub selbar_sel: &'static str,
    /// collections 族 list 选中行下标（落在 LIST_ITEMS 里）。
    pub list_sel: usize,
    /// collections 族 table 选中格下标（行 ×4 + 列）。
    pub table_cell: usize,
    /// collections 族 swipe 当前页下标。
    pub swipe_page: usize,
    /// dialogs 族 date_picker 选中日（`dpc:RC` 的格子序号）。
    pub date_sel: u8,
    /// dialogs 族 time_picker 小时（`tph:XX`）。
    pub time_hour: &'static str,
    /// dialogs 族 time_picker 分钟（`tpm:XX`）。
    pub time_minute: &'static str,
    /// status 族 info bar 是否展开（`bar:close` 关闭后 = false）。
    pub infobar_open: bool,
    /// layout 族 setting_expander 展开态。
    pub ref_exp: bool,
    /// settings 页「高级」区 expander 展开态。
    pub adv_expanded: bool,
    /// 当前语言在 [SUPPORTED] 里的**下标**。
    ///
    /// 存下标而不是 Tag：Tag 每帧都要重新构造，而下标是 Copy 且能直接当
    /// selected 传给控件 —— 与 combo / spin 同一手法。
    pub lang: usize,
    /// 用户的系统语言偏好（有序，最偏好的在前）。
    ///
    /// 首帧由宿主经 on_input 推进来。空表 = 读不到 ⇒ 回落 [SUPPORTED] 第一项，
    /// **不**把界面钉死在某一门语言上（与「读不到系统主题就不跟随」同一纪律）。
    pub prefs: Preferences,
    /// 系统语言协商**已经做过**（成功或放弃）。
    ///
    /// # 为什么不能拿 `prefs.is_empty()` 当这个判据
    ///
    /// 协商**可能得到空表**：宿主给的语言串若不是合法的 BCP 47 标签
    /// （平台层误给 LCID 串时就是如此），`Preferences::from_owned` 会把
    /// 每一项都丢掉 —— 空表。于是「空表」有两种含义：「还没协商」与
    /// 「协商了但没结果」，拿同一个判据的后果是**每帧重跑协商**。
    ///
    /// 而每帧协商的结果是「回落 [SUPPORTED] 第一项」⇒ **每一帧都把用户
    /// 刚选的语言改回英文**。症状：点「中文」毫无反应 —— 点击其实生效了，
    /// 同一帧末尾又被改回去了。
    ///
    /// 所以这里用一个**与协商结果无关**的独立标志。
    pub lang_negotiated: bool,
    /// 入场动画已过去的**毫秒**数。
    ///
    /// # 为什么是应用层的字段而不是 core 的
    ///
    /// 动画是**状态**（与 `route` / `hovered` 同层），而 core 的每帧入口只有
    /// `view()` 与 `on_input()`。所以时钟必须是 `Gallery` 的字段，由
    /// `View::on_input` 用 `InputState.dt_ms` 推进 —— 那条路径**每帧必调**，
    /// 不需要给 core 加"每帧回调"。
    ///
    /// # 为什么按**路由**归零
    ///
    /// 换路由 = 换内容 = 重放入场，这正是官方页面刷新的定义（`docs/developer/animation.md` §1）。
    /// 自己额外记一个"上一页 id"就要多一个字段与一次比较，而那样还会漏掉
    /// "同一路由内内容变了"（测试页的状态更新就不该重放入场）。
    pub enter_ms: f32,
    /// 上一次推进时钟时的路由（`&'static`，与 `route` 同类）。
    ///
    /// 存在的唯一理由是**要在推进之前**发现路由变了。归零后把它记成新路由，
    /// 下一帧才不会又归零一次（症状是"入场动画一帧都不播"）。
    /// 不存它就得靠"进入的这一刻"传参，而 `on_input` 每帧只被调一次、拿不到
    /// "事件"——只有状态。
    entered_route: &'static str,
}

impl Gallery {
    /// 有效像素宽度（官方断点的口径）。
    ///
    /// 宿主给的视口是物理像素，这里除 scale。判据见
    /// `docs/developer/responsive-and-touch.md` §2 与 [`zephyra_core::responsive::WidthClass`]。
    pub fn width_epx(&self) -> f32 {
        let scale = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        };
        self.viewport.0 / scale
    }

    /// 当前断点档位。
    pub fn width_class(&self) -> zephyra_core::responsive::WidthClass {
        zephyra_core::responsive::WidthClass::of_viewport(self.viewport, self.scale)
    }

    /// 是否为紧凑档（小屏，导航折叠、单列布局）。
    ///
    /// 这一条是**结构**判据，不是外观判据：折叠不折叠、几列、显不显示某块，
    /// 都是树拓扑，属于视图职责；外观值仍在 zstyle（见
    /// `docs/developer/responsive-and-touch.md` §7）。
    pub fn is_compact(&self) -> bool {
        self.width_class().is_compact()
    }

    /// 内容区在**布局单位**下的可用宽度。
    ///
    /// 基准画布下 = 1400(视口) − [`crate::app::shell::NAV_W`] − 28(页边距) = 1136，
    /// 与现有卡片网格 5 列一致（`columns_for(1136, 200, 12) == 5`）。
    ///
    /// 导航折叠时导航宽度归零（内容区扩大到满宽）—— 这也是官方「重新排列」的做法：
    /// 小屏不把 [`NAV_W`](crate::app::shell::NAV_W) 单位浪费在一个看不见的侧栏上。
    ///
    /// # 为什么导航宽要引用常量而不是写死
    ///
    /// 这里曾经写死 `236.0`，而 `shell::NAV_W` 实际是 232。列数于是按错 4 单位算 ——
    /// 而 4 单位小到看不出，只在某些宽度上让「差一点点放不下第 5 列」。
    /// 那条错是被 `tests::gates::content_width_follows_breakpoint` 抓到的，不是被眼看出来的。
    pub fn content_width(&self) -> f32 {
        let nav = if self.is_compact() {
            0.0
        } else {
            crate::app::shell::NAV_W
        };
        (self.viewport.0 - nav - PAGE_GUTTER).max(0.0)
    }

    /// 卡片网格列数：按可用宽度算，且不小于当前档位的下限。
    ///
    /// 卡宽取 200（本仓现有值），因此基准画布下仍是 **5 列** —— 这一条
    /// 既是"没破坏基准外观"的证据，也是列数边界值的回归：
    /// 见 `responsive.rs` 的 `columns_for(1136, 200, 12) == 5`。
    pub fn card_columns(&self) -> usize {
        use zephyra_core::responsive::columns_for;
        let n = columns_for(self.content_width(), 200.0, 12.0);
        n.max(self.width_class().min_columns())
    }

    /// 入场动画的**档位**（`Some(n)` = 发 `.enter-<n>`；`None` = 不发分类名）。
    ///
    /// # 外观值一个都不在这里
    ///
    /// 代码只决定"第几档"，第 n 档长什么样（opacity / translate-y）全在 zstyle
    /// 的 `.enter-1` … `.enter-8` 里。这与"hover 走 `.hovered`"是同一条模式的
    /// 延伸，也是不违反 `tests::gates::源码里没有外观` 的前提。
    ///
    /// # 为什么返回 `Option` 而不是 `u32`
    ///
    /// 两个"没有分类名"的情形都要表达：**进度为 0**（这一档与常驻外观没有差异，
    /// 多发一个分类名会让样式表多一条永不命中的规则，而门禁会要求它存在）与
    /// **已经放完**（`anim::step(1.0)` 会返回 `Some(8)`，不单独判就会永远停在
    /// 0.91 不透明度上）。
    pub fn enter_class(&self) -> Option<&'static str> {
        // ⚠ 必须先判"放完"再算进度。
        //
        // `anim::step(1.0)` 返回 `Some(8)`（`ceil(1.0 × 8)` = 8），所以不加这一句，
        // 动画结束后分类名会**永远**停在 `.enter-8` —— 而那一档是不透明度 0.91，
        // 症状是"页面始终蒙着一层灰"，且没有任何报错。
        //
        // 撤掉分类名之后画面回到 1.0，于是最后一跳是 0.91 → 1.0（9%）。
        // 这是 8 档量化的固有代价：要么最后跳 9%，要么 `.enter-8` 写 1.0
        // 而从 `.enter-7`(0.78) 跳 22%。选前者，理由写在
        // `ZephyraUI/tests/anim_wiring_tests.rs` 的模块文档里。
        if self.enter_ms >= zephyra_style::anim::ENTER_MS {
            return None;
        }
        let p = zephyra_style::anim::progress(
            self.enter_ms,
            zephyra_style::anim::ENTER_MS,
            zephyra_style::anim::Easing::PointToPoint,
        );
        zephyra_style::anim::step(p).map(|n| match n {
            1 => "enter-1",
            2 => "enter-2",
            3 => "enter-3",
            4 => "enter-4",
            5 => "enter-5",
            6 => "enter-6",
            7 => "enter-7",
            _ => "enter-8",
        })
    }

    /// 当前语言的 BCP 47 标签（`zh-Hans` 这类）。
    pub fn locale(&self) -> &'static str {
        SUPPORTED[self.lang.min(SUPPORTED.len() - 1)]
    }

    /// 协商出初始语言：按 [`zephyra_core::i18n::negotiate`] 的用户优先序匹配可用表。
    ///
    /// # 为什么在 `on_input` 里做而不是构造时做
    ///
    /// 宿主才知道系统语言偏好（视图拿不到窗口），而 `on_input` 每帧必调 ——
    /// 于是首帧就能定下来，不必另加一条启动钩子。
    ///
    /// 协商不出来时回落**第一项**：空表会让整个界面全空白，而那比
    /// "界面是英文"严重得多。
    fn pick_initial_language(prefs: &Preferences) -> usize {
        let available: Vec<Tag> = SUPPORTED.iter().filter_map(|s| Tag::parse(s)).collect();
        match zephyra_core::i18n::negotiate(prefs, &available) {
            Some(tag) => SUPPORTED
                .iter()
                .position(|s| *s == tag.as_str())
                .unwrap_or(0),
            None => 0,
        }
    }

    /// 当前语言对应的偏好表。
    ///
    /// # 为什么不直接复用系统偏好
    ///
    /// 用户手动切过语言之后，系统那个表就**不再是**他要的了。手动选择优先。
    ///
    /// 只含**一项**：当前语言。回退由 [`zephyra_core::i18n::Messages::select`] 内部的
    /// 默认语言那一档负责，不需要在偏好表里堆回退项 ——
    /// 把回退项塞进偏好表会让 `negotiate` 也去匹配它们，
    /// 而协商与取文案是两件事，各有一条链更清楚。
    pub fn prefs_for_current(&self) -> Preferences {
        let mut p = Preferences::new();
        if let Some(t) = Tag::parse(self.locale()) {
            p.push(t);
        }
        p
    }

    /// 取一条文案。**所有**面向用户的字符串都走这里 —— 源码里不出文案。
    ///
    /// # 返回 `&'static str` 而不是 `String`
    ///
    /// `view()` 每帧重建整棵树，返回 `String` 就等于每帧一次分配 + 一次
    /// 泄漏（`Box::leak`），而文案在编译期就定好了、根本不是运行期数据。
    /// 于是链路终点是 `PropertyValue::static_str` —— 属性系统为此专门留了一个
    /// 变体（见 `property.rs`），整条路零分配。
    ///
    /// # 找不到时返回键名本身
    ///
    /// 三级回退（见 [`Messages::select`]）：当前语言 → 语言段 → 默认语言。
    /// 连默认语言也没有才轮到这里，返回**键名**而不是空串：空串会让"漏翻"
    /// 看起来像"这里本来就是空的"，而键名在画面上立刻可辨。
    /// 真正发现漏翻的是门禁，不是肉眼（`every_tr_key_used_in_source_exists_in_every_language`）。
    pub fn tr(&self, key: &'static str) -> &'static str {
        messages()
            .select(&self.prefs_for_current(), SUPPORTED[0], key)
            .unwrap_or(key)
    }

    /// 触控目标边长换算成**布局单位**（= 视觉上的 epx × scale）。
    ///
    /// 只在触控路径上用：鼠标路径的目标尺寸不变（本仓基准未动，见
    /// `docs/developer/responsive-and-touch.md` §5）。
    pub fn touch_target(&self, epx: f32) -> f32 {
        zephyra_core::responsive::touch_px(epx, self.scale)
    }
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            route: "guide",
            hovered: None,
            pressed: None,
            last_pressed: None,
            wants_dark: true,
            theme_intent: None,
            seg: "seg:grid",
            ref_family: 0,
            guide_count: 0,
            last_action: "",
            pill_on: false,
            color_idx: 0,
            demo_check: true,
            demo_seg: "stseg:b",
            demo_combo: 0,
            subsel: "subdemo:a",
            selbar_sel: "seldemo:week",
            list_sel: 1,
            table_cell: 0,
            swipe_page: 1,
            date_sel: 15,
            time_hour: "14",
            time_minute: "05",
            infobar_open: true,
            ref_exp: false,
            adv_expanded: false,
            checked: [true, false, false],
            radio: 0,
            glass: crate::app::glass::Glass::default(),
            switch_on: true,
            notify_on: false,
            volume: 0.62,
            expanded: false,
            combo: 0,
            spin: 7,
            dd_open: false,
            mica: true,
            menu_open: None,
            // dump 路默认走基准画布：静态图必须与 1400×800 窗口路同基准，
            // 否则参考图和真实画面始终差一档，而没人会发现差在哪。
            viewport: (BASE_VIEWPORT.0 as f32, BASE_VIEWPORT.1 as f32),
            scale: 1.0,
            lang: 0,
            prefs: Preferences::new(),
            lang_negotiated: false,
            enter_ms: 0.0,
            // 初值 = `route` 的初值（"home"），否则第一帧会被判成"路由变了"
            // 而归零一次 —— 那恰好是想要的，所以两者必须**相同**。
            entered_route: ROUTES[0].0,
        }
    }
}

impl Gallery {
    /// 带主题意图格的构造（窗口路：View 与 Bootstrap 共享这一格）。
    pub fn with_theme_intent(
        theme_intent: std::sync::Arc<std::sync::atomic::AtomicU8>,
        dark: bool,
    ) -> Self {
        Self {
            theme_intent: Some(theme_intent),
            wants_dark: dark,
            ..Self::default()
        }
    }

    /// 某路由当前是否被选中。
    pub fn is_route(&self, id: &str) -> bool {
        self.route == id
    }

    /// 某 key 是否正被悬停。
    pub fn hovers(&self, key: &str) -> bool {
        self.hovered.as_ref().and_then(|k| k.as_name()) == Some(key)
    }

    /// 某 key 是否正被按住。
    /// 当前 hover 的 key 名字（给需要 Option<&str> 的控件用：列表/选择器/对话框）。
    pub fn hovered_name(&self) -> Option<&str> {
        self.hovered.as_ref().and_then(|k| k.as_name())
    }
    /// 当前按住的 key 名字（见 [Self::hovered_name]）。
    /// 当前 hover 的 key 名（无 hover 时提示语）。测试页状态行用。
    pub fn hovered_key(&self) -> &'static str {
        self.hovered
            .as_ref()
            .and_then(|k| k.as_name())
            .unwrap_or("hover anything")
    }
    pub fn pressed_name(&self) -> Option<&str> {
        self.pressed.as_ref().and_then(|k| k.as_name())
    }
    pub fn holds(&self, key: &str) -> bool {
        self.pressed.as_ref().and_then(|k| k.as_name()) == Some(key)
    }

    /// 应用一次 click（按下与抬起落在同一 key 上）。
    ///
    /// # 为什么是 `pub`
    ///
    /// 玻璃参数的分派是**运行期字符串解析**（剥 `glass-` 前缀 → 找
    /// `Param::slug()` → 看 `:dec` / `:inc` 后缀），编译器看不出「面板发的
    /// key 与解析器认识的 key 对不上」。门禁 `glass_param_keys_all_wired` 要
    /// **真的点一遍**才能证明接线，所以这里必须开。
    ///
    /// 门禁**不能**自己复刻一份分派逻辑再验它 —— 那样验的是门禁自己，
    /// 面板可以照样点不动。
    pub fn click(&mut self, key: &str) {
        // ---- 1) 导航（查表，不是 match 列举）----
        //
        // 首页的分类入口与左侧导航栏发的是**同一个 key**（路由 id），所以这一条
        // 同时接住两处。旧结构还要处理 14 个分组路由与 44 个控件卡，那两处都来自
        // `catalog.rs`，随重写删掉了。
        if let Some(route) = resolve_route(key) {
            self.route = route;
            return;
        }
        // ---- 2) 语言切换（设置页分段控件的段 id 就是语言标签）----
        //
        // ⚠️ 必须在 `resolve_route` **之后**、`match` **之前**：段 id 是
        // `zh-Hans` 这种形状，不带 `lang:` 前缀，所以这里按 `SUPPORTED`
        // 查表而不是切前缀 —— 切前缀要 `strip_prefix` 再 `concat!`，后者
        // 在每帧重建的视图里就是一次泄漏。
        //
        // 两种形态都要认：设置页发裸标签（`en`），主题章发 `lang:en`
        // （与其它页的 `前缀:名` 一致）。只认一种的话，另一种就是
        // 「key 在树上、点击没反应」——主题章的语言按钮曾因此全成空壳。
        let lang_key = key.strip_prefix("lang:").unwrap_or(key);
        if let Some(i) = SUPPORTED.iter().position(|s| *s == lang_key) {
            self.lang = i;
            // 手动选择**优先于**系统协商：把协商标记成「已做过」，
            // 这样即使这一帧恰好是系统语言首次到达（协商就发生在
            // `on_input` 末尾），也不会把用户刚选的语言改回去。
            self.lang_negotiated = true;
            return;
        }
        // ---- 3) 其余按 key 分派 ----
        match key {
            // 主题意图有两个入口：页头的 tool_button（`theme-toggle`）与设置页
            // 的总开关（`switch:master`）。两者必须改**同一个**字段 ——
            // 设置页的开关绑的是 `wants_dark`，这里若改别的字段，开关点了
            // 视觉不动（门禁 `every_interactive_key_is_wired_or_whitelisted`
            // 正是抓到这一处的）。
            "theme-toggle" | "switch:master" => {
                self.wants_dark = !self.wants_dark;
                // 窗口路：把意图写进共享格，Bootstrap 每帧对账后执行。
                if let Some(cell) = &self.theme_intent {
                    cell.store(
                        if self.wants_dark { 0 } else { 1 },
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
            // 选择页的分段。段 id 与 `pages::toggles::SEGS` 一一对应，改那边必须改这里 ——
            // 门禁 `segment_keys_are_all_wired` 就是钉这一条，改漏了它立刻红。
            "seg:grid" => self.seg = "seg:grid",
            "seg:list" => self.seg = "seg:list",
            "seg:card" => self.seg = "seg:card",
            // 复选框。
            "check:alpha" => self.checked[0] = !self.checked[0],
            "check:beta" => self.checked[1] = !self.checked[1],
            "check:gamma" => self.checked[2] = !self.checked[2],
            // 单选：点哪个就选哪个。
            //
            // ⚠️ 写成 `self.radio = if self.radio == i { usize::MAX } else { i }`
            // 会得到「点当前选中的那个可以取消」，而真实的单选组不允许选中项被取消
            // —— 画面上这不会报错，只是永远做不到「一个都不选」。
            "radio:a" => self.radio = 0,
            "radio:b" => self.radio = 1,
            "radio:c" => self.radio = 2,
            // toggles 页的演示开关（首页旧 `live-switch` 已随首页删除）。
            "switch:notify" => self.notify_on = !self.notify_on,
            "exp:details" => self.expanded = !self.expanded,
            // ---- 值输入类演示 ----
            // ⚠ key 是 `combo-material`（连字符），不是 `combo:material`——
            // 页面发的就是连字符版。两者差一个冒号时点击**静默**不切换，
            // 而画面上 ComboBox 完全正常（它一直显示第一项看起来像默认值）。
            // 这正是 `every_interactive_key_is_wired_or_whitelisted` 要消灭的
            // 失效类别：key 在树上、分派不认识。
            "combo-material" => self.combo = (self.combo + 1) % COMBO_OPTIONS.len(),
            // 值域与 `pages::basics::spin_text` 的查表同步：表的末项就是
            // 上限，越界回落会让「点 ＋」显示不变。（默认值也在这段值域内。）
            "spin:up" => self.spin = (self.spin + 1).min(20),
            "spin:down" => self.spin = (self.spin - 1).max(0),
            // round-a / round-b 是 round_button 的**两种外观**（普通 / checked），
            // 演示的是「同一个按钮的长相」，不是可切换状态 —— 没有分派分支，
            // 门禁白名单里注明。
            "dd:demo" => self.dd_open = !self.dd_open,
            // ---- 菜单栏 ----
            "bar:0" | "bar:1" | "bar:2" => {
                let idx = match key {
                    "bar:0" => 0,
                    "bar:1" => 1,
                    _ => 2,
                };
                // 再点一次当前项 = 收起（MenuBar 的常规行为）。
                self.menu_open = if self.menu_open == Some(idx) {
                    None
                } else {
                    Some(idx)
                };
            }
            // 菜单条目：core 的菜单是**内联卡**而不是浮层（见 `pages::menus`），
            // 所以「点一项」的最小诚实语义就是收起菜单。菜单里没有真动作，
            // 点完不收起才是看起来"点了没反应"。
            _ if key.starts_with("m:") => self.menu_open = None,
            // 窗口按钮（win:*）在这个构建里只做悬停/按下视觉：宿主没有把
            // 最小化/最大化/关闭暴露给 View（那是窗口能力，不是样式能力）。
            //
            // 展示型控件（滑条、文本输入、Tooltip、进度/徽章类指示器）同样
            // 没有分支：core 没有连续指针位置、没有文本输入，指示器本来
            // 就不是按钮。别在这里给它们编造一个假动作 —— 完整清单与每一项
            // 的理由在 `tests/gates.rs` 的 `DISPLAY_ONLY`。
            //
            // ---- 快速开始章的计数 ----
            //
            // 开发者点这个按钮时，他想看的是「状态变了 → 界面变了」。
            // 所以这里只做一件事：加一。渲染由 `view()` 每帧读状态重建。
            "guide-inc" => self.guide_count = self.guide_count.saturating_add(1),
            // ---- 触发类按钮的可见反馈 ----
            //
            // 按钮的语义是「触发动作」。示例里的按钮若点了没有任何可见变化，
            // 教给开发者的就是「按钮是假的」。所以触发类按钮统一发 `act:<名>`，
            // 点了把动作名写进 `last_action`，由**外壳页头**那行显示 ——
            // 放各页页首的话，其它路由上的按钮改的是看不见的树。
            //
            // ⚠ 分派走**前缀分支 + ACT_IDS 查表**：新增一个触发按钮时
            // 只要在页面上发 `act:` key、并把 key 加进 [`ACT_IDS`]。
            // 漏加表的症状是「点了没反应」，而这正是本门禁要消灭的失效类别。
            _ if key.starts_with("act:") => {
                if let Some(s) = static_key(&ACT_IDS, key) {
                    self.last_action = s;
                }
            }
            // basics 页的 pill（可保持按下）与色格（color_picker 的格子）。
            "pill-a" => self.pill_on = !self.pill_on,
            _ if key.starts_with("swatch:") => {
                // `swatch:0..15` 是常量表（`gen_swatch_key`），冒号后就是下标。
                self.color_idx = key[7..].parse().unwrap_or(0);
            }
            // ---- state 章：教学页必须自证 ----
            //
            // 这一章的职责是教「哪些控件真的接了状态」。若「Wired to state」
            // 区里的控件自己点了没反应，这一页就在说谎——比别页的空壳更糟。
            "state-check" => self.demo_check = !self.demo_check,
            // state 章演示的是「switch 接了什么」：与 settings 页共用
            // `switch_on`（旧首页 `live-switch` 的先例）——共享本身是教学
            // 内容：「同一个状态可以驱动两处视图」。
            "state-switch" => self.switch_on = !self.switch_on,
            _ if key.starts_with("stseg:") => {
                if let Some(s) = static_key(&DEMO_SEGS, key) {
                    self.demo_seg = s;
                }
            }
            "state-combo" => {
                self.demo_combo = (self.demo_combo + 1) % 3;
            }
            // ---- 主题 ----
            "theme-mica" => self.mica = !self.mica,
            // ---- 参考层：navigation / collections / dialogs 族的选中态 ----
            //
            // ⚠ 这些字段都是 `&'static str`，而 `click` 收到的 key 是 `&str`
            // （`on_input` 里 `to_string()` 转交）。直接 `self.x = key` 存不下来，
            // 必须经 [`static_key`] 查回常量原身——查表是「运行期 key →
            // 编译期常量」的唯一不泄漏通道。
            _ if key.starts_with("subdemo:") => {
                if let Some(s) = static_key(&SUBDEMO_IDS, key) {
                    self.subsel = s;
                }
            }
            _ if key.starts_with("seldemo:") => {
                if let Some(s) = static_key(&SELDEMO_IDS, key) {
                    self.selbar_sel = s;
                }
            }
            _ if key.starts_with("list:") => {
                if let Some(i) = crate::app::pages::collections::LIST_ITEMS
                    .iter()
                    .position(|(id, _)| *id == key)
                {
                    self.list_sel = i;
                }
            }
            _ if key.starts_with("td:") => {
                // `td:RC`（常量表 `cell_key`）：行 ×4 + 列 = 线性下标。
                // 下标位置：`t`0 `d`1 `:`2 `R`3 `C`4。
                let r = key[3..4].parse::<usize>().unwrap_or(0);
                let c = key[4..5].parse::<usize>().unwrap_or(0);
                self.table_cell = r * 4 + c;
            }
            _ if key.starts_with("sw:") => {
                self.swipe_page = key[3..].parse().unwrap_or(0);
            }
            _ if key.starts_with("dpc:") => {
                // `dpc:RC`（`dp_cell_key`）：格子序号 = 行 ×7 + 列。selected
                // 的语义是**日期值**，所以查 `CALENDAR[序号]` 的 `Some(n)` ——
                // 空白格（None）不响应：日历上空白格本来就不是日期。
                // 下标位置：`d`0 `p`1 `c`2 `:`3 `R`4 `C`5。
                let r = key[4..5].parse::<usize>().unwrap_or(0);
                let c = key[5..6].parse::<usize>().unwrap_or(0);
                if let Some(Some(day)) = crate::app::pages::dialogs::CALENDAR.get(r * 7 + c) {
                    self.date_sel = *day;
                }
            }
            _ if key.starts_with("tph:") => self.time_hour = time_part(key),
            _ if key.starts_with("tpm:") => self.time_minute = time_part(key), // ---- 信息条关闭 ----
            //
            // ⚠ `bar:close` 与菜单栏的 `bar:0/1/2` 只差后缀，必须放在菜单栏
            // 分支**之后**（或精确匹配），否则 `bar:close` 会被当成菜单索引。
            "bar:close" => self.infobar_open = false,
            "infobar-restore" => self.infobar_open = true,
            // ---- expander 类 ----
            "set-exp" => self.ref_exp = !self.ref_exp,
            "set-adv" => self.adv_expanded = !self.adv_expanded,
            // ---- 控件参考页的族切换 ----
            //
            // key 形如 `ref:basics`。走 `REF_FAMILIES` 查表而不是九段 `match`：
            // 加一族只改那张表。解析方式与玻璃参数同构（剥前缀 → 查表），
            // 而门禁 `reference_family_keys_all_wired` 钉住「表里有、树上也有」。
            _ if key.starts_with("ref:") => {
                let fam = &key[4..];
                if let Some(i) = REF_FAMILIES.iter().position(|f| *f == fam) {
                    self.ref_family = i;
                }
            }
            // ---- 玻璃参数步进 ----
            //
            // key 形如 `glass-thickness:dec`。解析方式是「剥前缀 → 找后缀」，
            // 而不是四段 `match` —— 加第五个参数时只改 [`Param::ALL`]，
            // 而漏改这里的话面板会显示却点不动（门禁 `glass_param_keys_all_wired`
            // 钉住这一点）。
            //
            // ⚠ 边界上的 no-op **刻意不产生任何状态变化**：那样「已达上限」与
            // 「点击没接线」在画面上无法区分。`Glass::nudge` 返回是否真的动了，
            // 这里忽略它 —— 我们只关心 `glass` 自身变没变。
            //
            // ⚠ 这个 `_ =>` 必须待在 `match` 的**最后**：写在中间会把后面所有分支
            // 变成「不可达」，而 clippy 只报 warning 不报错 —— 症状是圆角按钮、
            // 下拉、菜单栏全部静默失效。踩过一次。
            _ => {
                if let Some(rest) = key.strip_prefix("glass-") {
                    for param in crate::app::glass::Param::ALL {
                        let Some(tail) = rest.strip_prefix(param.slug()) else {
                            continue;
                        };
                        let delta = match tail {
                            ":dec" => -1,
                            ":inc" => 1,
                            _ => continue,
                        };
                        self.glass.nudge(param, delta);
                        break;
                    }
                }
            }
        }
    }
}

impl View for Gallery {
    fn on_input(&mut self, state: &InputState) {
        // 时钟输入。取一次，全函数共用 —— 同一帧里读两次 `state.dt_ms`
        // 不会得到不同值，但两处各写一遍会让"这个数字是哪来的"变成两处真相。
        let dt = state.dt_ms;
        // click 还原：上一帧按着、这一帧松了，且 key 相同（按住期间不放）。
        let click_key = match (&self.last_pressed, &state.pressed) {
            (Some(prev), None) => Some(prev.as_name().unwrap_or_default().to_string()),
            _ => None,
        };
        if let Some(key) = click_key {
            self.click(&key);
        }
        self.last_pressed = state.pressed.clone();
        self.hovered = state.hovered.clone();
        self.pressed = state.pressed.clone();

        // 入场动画时钟。
        //
        // ⚠ 顺序很要紧：**先**归零再累加，否则换路由的那一帧会从上一条路由的
        // 末尾接着走 —— 症状是"切页之后动画几乎不播"（尤其在两条路由之间反复切换
        // 时，`enter_ms` 已经很大，归零前先加就等于直接跳过入场）。
        let route_changed = self.entered_route != self.route;
        if route_changed {
            self.enter_ms = 0.0;
            self.entered_route = self.route;
        } else {
            self.enter_ms += dt;
        }
        // 夹到时长以上：动画放完之后不必再累加（`f32` 无限增长终会丢精度，
        // 而一个跑了几小时的窗口正是最可能暴露这件事的场景）。
        if self.enter_ms > zephyra_style::anim::ENTER_MS {
            self.enter_ms = zephyra_style::anim::ENTER_MS;
        }
        // 视口与缩放比：断点判据的唯一来源。宿主每帧开头推一次，于是 resize 之后
        // **下一帧**的树就按新断点造 —— 这正是 WinUI AdaptiveTrigger 的节奏
        // （状态改变 → 声明改变 → diff 落到保留树），不需要任何回调。
        self.viewport = state.viewport;
        self.scale = state.scale;
        // 系统语言偏好：**只协商一次**，之后用户手动切了就以其为准。
        //
        // ⚠ 判据是 `lang_negotiated` 而不是 `prefs.is_empty()`。
        //
        // 理由（这条曾经是「点中文无反应」的真凶）：协商**可能得到空表**
        // —— 宿主给的语言串不是合法 BCP 47 标签时（平台层误给 LCID 串），
        // `Preferences::from_owned` 会把每一项都丢掉。于是「空表」既表示
        // 「还没协商」也表示「协商了但没结果」，用空表当判据就会**每帧重跑**；
        // 而每帧的结果是「回落英文」⇒ 用户刚点的语言在同一帧末尾被改回去。
        //
        // 手动切换不修改 `prefs`、只改 `lang` —— 而且它自己也会把
        // `lang_negotiated` 置真（见 `click`），所以**任何**时机的系统语言
        // 都盖不过用户的选择。
        if !self.lang_negotiated && !state.languages.is_empty() {
            self.lang_negotiated = true;
            self.prefs = zephyra_core::i18n::Preferences::from_owned(&state.languages);
            self.lang = Self::pick_initial_language(&self.prefs);
        }
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        shell::window(self)
    }
}
