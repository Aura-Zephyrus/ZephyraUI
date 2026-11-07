//! Gallery 前端应用状态：RinUI 示例应用（gallery）的 Zephyra 重写版。
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

use std::sync::OnceLock;

use zephyra_core::i18n::{Messages, Preferences, Tag};
use zephyra_core::id::ElementKey;
use zephyra_core::{context::Context, element::Element};
use zephyra_fx::{InputState, PointerInputPhase, View};

pub mod data;
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
/// 背景照片的**纹理名**。
///
/// 它同时出现在两处，两处必须一致：
///
/// - 这里：向宿主注册纹理时用的名字；
/// - `frontend/pages.zstyle` 的 `image-texture: "bg-mountain"` —— 那里写的是
///   **同一个字符串**，后端按名字查采样视图。
///
/// ⚠ 写错的后果不是「图片不出来」这么温和：名字对不上时 `quad::expand`
/// 把这条图像命令**跳过**并计入 `missing_textures`，而画面上只表现为
/// 「背景是空的」—— 与「照片解码失败」完全同形。
pub const MOUNTAIN_TEXTURE: &str = "bg-mountain";

pub const ROUTES: &[(&str, &str, &str)] = &[
    ("guide", "Quick start", "home"),
    ("state", "State", "check"),
    ("styling", "Styling", "pen"),
    ("layout", "Layout", "grid"),
    ("theme", "Theme & i18n", "settings"),
    ("testing", "Testing", "bell"),
    ("reference", "Controls", "doc"),
    // 材质专区**提升为一等路由**，并把液态玻璃**并进来**：材质（实心 / 磨砂 /
    // 液态玻璃）本来只是按钮族里的一行按钮，而液态玻璃是运行期可调的折射
    // 实验台 —— 两件事讲的是同一张皮的不同侧面，合成一页比分成两页强。
    //
    // 合并之后这一页的核心是「材质在真实照片上是什么样」，所以图标取 `image`
    // （照片板就是这一页的背景）；四个 `[−]/[+]` 步进器仍在页内。
    ("materials", "Materials", "image"),
    ("slider", "Slider", "sliders"),
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
/// `reference` **页内**的 segmented 切换。两族内容复用 `frontend/shell.zypui` 里各页面组件的
/// 组件，每族独立一屏，一屏装得下。
pub const REF_FAMILIES: &[&str] = &[
    // ⚠ 2026-11-01：gallery 只演示**真正接了原生交互行为**的控件 ——
    // Button 族与复选框族。其余控件（Dialog / Menu / DatePicker / TableView …）
    // 仍是控件库的一部分，但不再在这里演示：它们多数只画得出样子、交互不成立，
    // 摆在「控件参考」里等于教错。
    //
    // ⚠ 新族一律**追加在末尾**，不要插在中间：`ref_family` 是下标，
    // 门禁与 `--family` 都用下标引用（插中间会静默改掉已有序号的含义，
    // 而画面上只是「点了这一族出现那族」）。
    "buttons", "checkbox", "switches",
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
    ("en", "route.guide", "Quick start"),
    ("en", "route.state", "State"),
    ("en", "route.styling", "Styling"),
    ("en", "route.layout", "Layout"),
    ("en", "route.theme", "Theme & i18n"),
    ("en", "route.testing", "Testing"),
    ("en", "route.reference", "Controls"),
    ("en", "route.materials", "Materials"),
    ("en", "route.slider", "Slider"),
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
    ("en", "action.feedback", "Feedback"),
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
    ("en", "ui.language", "Language"),
    ("en", "ui.theme", "Theme"),
    ("en", "ui.route", "Route"),
    ("en", "ui.button", "Button"),
    ("en", "ui.variants", "Variants"),
    ("en", "ui.standard", "Standard"),
    ("en", "ui.accent", "Accent"),
    ("en", "ui.subtle", "Subtle"),
    ("en", "ui.materials", "Materials"),
    ("en", "ui.solid", "Solid"),
    ("en", "ui.frosted", "Frosted"),
    ("en", "ui.glass", "Glass"),
    ("en", "ui.primary", "Primary"),
    ("en", "ui.pill", "Pill"),
    ("en", "ui.with-icon-round", "With icon / round"),
    ("en", "ui.liquid-button", "Liquid button"),
    ("en", "ui.options", "Options"),
    ("en", "ui.option.alpha", "Alpha"),
    ("en", "ui.option.beta", "Beta"),
    ("en", "ui.option.gamma", "Gamma"),
    ("en", "ui.dropdown-button", "Dropdown button"),
    ("en", "ui.liquid.demo", "LiquidButton — press & drag, solid / glass (LiquidButton.kt 1:1)"),
    ("en", "ui.send", "Send"),
    ("en", "ui.click-me", "Click me"),
    ("en", "ui.counter", "Counter"),
    ("en", "ui.multiple", "Multiple"),
    ("en", "ui.checkbox", "CheckBox"),
    ("en", "ui.token-colored-text", "Token-colored text"),
    ("en", "ui.spacing-demo", "Spacing demo"),
    ("en", "ui.what-they-guard", "What they guard"),
    ("en", "ui.you-are-here", "You are here"),
    ("en", "ui.check-a-stylesheet", "Check a stylesheet"),
    (
        "en",
        "ui.click-the-button-the-count-goes-up-that-is-the-whole-pipeline",
        "Click the button. The count goes up. That is the whole pipeline.",
    ),
    ("en", "ui.dump-current-route", "Dump current route"),
    ("en", "ui.dump-a-specific-route", "Dump a specific route"),
    ("en", "ui.run-all-gallery-gates", "Run all gallery gates"),
    ("en", "ui.current-breakpoint", "Current breakpoint"),
    ("en", "ui.44-epx-correct", "44 epx (correct)"),
    ("en", "ui.28-epx-too-small", "28 epx (too small)"),
    ("en", "ui.materials-on-a-real-photo", "Materials on a real photo"),
    ("en", "ui.materials-and-liquid-glass", "Materials & Liquid Glass"),
    ("en", "ui.thin", "Thin"),
    ("en", "ui.normal", "Normal"),
    ("en", "ui.thick", "Thick"),
    ("en", "ui.decorations", "Decorations"),
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
    // ---- 按钮 / 复选框族（label_key 通道）+ 页面散文 ----
    ("en", "ui.check.toggle-me", "Toggle me"),
    ("en", "ui.check.enable-feature", "Enable feature"),
    ("en", "ui.check.send-analytics", "Send analytics"),
    ("en", "ui.check.beta-channel", "Beta channel"),
    ("en", "ui.check.locked-option", "Locked option"),
    ("en", "ui.big-enough", "Big enough"),
    ("en", "ui.tiny", "Tiny"),
    ("en", "ui.wired-controls-change-state-on-click-display-only-controls-dont-thats-a-core-boundary-not-a-bug", "Wired controls change state on click. Display-only controls don't — that's a core boundary, not a bug."),
    ("en", "ui.wired-to-state", "Wired to state"),
    ("en", "ui.appearance-lives-in-zstyle-source-sends-keys-and-class-names-only-no-colors-no-sizes", "Appearance lives in .zstyle. Source sends keys and class names only — no colors, no sizes."),
    ("en", "ui.colors-come-from-theme-tokens", "Colors come from theme tokens"),
    ("en", "ui.this-color-is-content-secondary", "This color is @content_secondary"),
    ("en", "ui.class-names-are-selectors", "Class names are selectors"),
    ("en", "ui.layout-affecting-properties", "Layout-affecting properties"),
    ("en", "ui.spacing-is-declared-affects-layout-so-color-only-changes-skip-relayout", "Spacing is declared affects_layout so color-only changes skip relayout"),
    ("en", "ui.breakpoints-small-medium-large", "Breakpoints: small / medium / large"),
    ("en", "ui.touch-targets-need-44-epx", "Touch targets need 44 epx"),
    ("en", "ui.theme-is-an-intent-not-a-reference-language-negotiates-from-system-prefs-then-manual-choice-wins", "Theme is an intent, not a reference. Language negotiates from system prefs, then manual choice wins."),
    ("en", "ui.language-en-zh-hans-ja", "Language (en / zh-Hans / ja)"),
    ("en", "ui.current-locale", "Current locale"),
    ("en", "ui.three-layers-of-self-verification-from-fastest-to-slowest-each-guards-a-different-class-of-silent-failure", "Three layers of self-verification, from fastest to slowest. Each guards a different class of silent failure."),
    ("en", "ui.current-state", "Current state"),
    ("en", "ui.1-stylesheet-syntax", "1. Stylesheet syntax"),
    ("en", "ui.2-cpu-raster-dump", "2. CPU raster dump"),
    ("en", "ui.dump-one-reference-family", "Dump one reference family"),
    ("en", "ui.3-tree-level-gates", "3. Tree-level gates"),
    ("en", "ui.style-tokens-dead-rules-hit-test-breakpoints-no-appearance-in-source", "Style tokens, dead rules, hit-test, breakpoints, no appearance in source"),
    ("en", "ui.try-it-switch-language-same-state-as-lang", "Try it: switch language (same state as --lang)"),
    ("en", "ui.control-reference-pick-a-family-to-see-its-controls", "Control reference — pick a family to see its controls."),
    // ---- 开关族（第三族）：分区标题 + 开关 / 单选钮标签 ----
    ("en", "ui.switch", "Switch"),
    ("en", "ui.toggle", "Toggle"),
    ("en", "ui.radio", "Radio"),
    ("en", "ui.switch.airplane-mode", "Airplane mode"),
    ("en", "ui.radio.off", "Off"),
    ("en", "ui.radio.on", "On"),
    ("en", "ui.radio.auto", "Auto"),
    ("en", "ui.slider", "Slider"),
    ("en", "ui.slider.volume", "Volume"),
    ("en", "ui.slider.brightness", "Brightness"),
    ("en", "ui.slider.demo", "Liquid slider — drag or tap, solid / glass"),
    // ---- zh-Hans ----
    ("zh-Hans", "route.guide", "快速开始"),
    ("zh-Hans", "route.state", "状态"),
    ("zh-Hans", "route.styling", "样式"),
    ("zh-Hans", "route.layout", "布局"),
    ("zh-Hans", "route.theme", "主题与多语言"),
    ("zh-Hans", "route.testing", "测试"),
    ("zh-Hans", "route.reference", "控件参考"),
    ("zh-Hans", "route.materials", "材质与玻璃"),
    ("zh-Hans", "route.slider", "滑块"),
    (
        "zh-Hans",
        "ui.small-nav-collapsed-to-icons",
        "小 — 导航收成图标列",
    ),
    ("zh-Hans", "ui.medium-nav-shows-labels", "中 — 导航显示文字"),
    ("zh-Hans", "ui.large-full-layout", "大 — 完整布局"),
    ("zh-Hans", "action.feedback", "反馈"),
    // ---- 页面正文（同 en 组，键序一致）----
    ("zh-Hans", "ui.zephyraui-gallery", "ZephyraUI 示例库"),
    (
        "zh-Hans",
        "ui.search-controls-and-samples",
        "搜索控件与示例……",
    ),
    ("zh-Hans", "ui.language", "语言"),
    ("zh-Hans", "ui.theme", "主题"),
    ("zh-Hans", "ui.route", "路由"),
    ("zh-Hans", "ui.button", "按钮"),
    ("zh-Hans", "ui.variants", "变体"),
    ("zh-Hans", "ui.standard", "标准"),
    ("zh-Hans", "ui.accent", "强调"),
    ("zh-Hans", "ui.subtle", "低调"),
    ("zh-Hans", "ui.materials", "材质"),
    ("zh-Hans", "ui.solid", "纯色"),
    ("zh-Hans", "ui.frosted", "磨砂"),
    ("zh-Hans", "ui.glass", "玻璃"),
    ("zh-Hans", "ui.primary", "主要"),
    ("zh-Hans", "ui.pill", "胶囊"),
    ("zh-Hans", "ui.with-icon-round", "带图标 / 圆形"),
    ("zh-Hans", "ui.liquid-button", "液态胶囊按钮"),
    ("zh-Hans", "ui.options", "选项"),
    ("zh-Hans", "ui.option.alpha", "选项一"),
    ("zh-Hans", "ui.option.beta", "选项二"),
    ("zh-Hans", "ui.option.gamma", "选项三"),
    ("zh-Hans", "ui.dropdown-button", "下拉按钮"),
    ("zh-Hans", "ui.liquid.demo", "LiquidButton — 按压并拖拽，实色 / 玻璃（LiquidButton.kt 1:1）"),
    ("zh-Hans", "ui.send", "发送"),
    ("zh-Hans", "ui.click-me", "点我"),
    ("zh-Hans", "ui.counter", "计数器"),
    ("zh-Hans", "ui.multiple", "多选"),
    ("zh-Hans", "ui.checkbox", "复选框"),
    ("zh-Hans", "ui.token-colored-text", "令牌着色的文字"),
    ("zh-Hans", "ui.spacing-demo", "间距演示"),
    ("zh-Hans", "ui.what-they-guard", "它们守住什么"),
    ("zh-Hans", "ui.you-are-here", "你在这里"),
    ("zh-Hans", "ui.check-a-stylesheet", "校验样式表"),
    (
        "zh-Hans",
        "ui.click-the-button-the-count-goes-up-that-is-the-whole-pipeline",
        "点一下按钮，计数加一 —— 整条管线就这些。",
    ),
    ("zh-Hans", "ui.dump-current-route", "导出当前路由"),
    ("zh-Hans", "ui.dump-a-specific-route", "导出指定路由"),
    ("zh-Hans", "ui.run-all-gallery-gates", "跑全部画廊门禁"),
    ("zh-Hans", "ui.current-breakpoint", "当前断点"),
    ("zh-Hans", "ui.44-epx-correct", "44 epx（正确）"),
    ("zh-Hans", "ui.28-epx-too-small", "28 epx（太小）"),
    ("zh-Hans", "ui.materials-on-a-real-photo", "真实照片上的材质"),
    ("zh-Hans", "ui.materials-and-liquid-glass", "材质与液态玻璃"),
    ("zh-Hans", "ui.thin", "薄"),
    ("zh-Hans", "ui.normal", "普通"),
    ("zh-Hans", "ui.thick", "厚"),
    ("zh-Hans", "ui.decorations", "装饰"),
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
    // ---- 按钮 / 复选框族（label_key 通道）+ 页面散文 ----
    ("zh-Hans", "ui.check.toggle-me", "切换我"),
    ("zh-Hans", "ui.check.enable-feature", "启用功能"),
    ("zh-Hans", "ui.check.send-analytics", "发送统计"),
    ("zh-Hans", "ui.check.beta-channel", "测试通道"),
    ("zh-Hans", "ui.check.locked-option", "锁定的选项"),
    ("zh-Hans", "ui.big-enough", "足够大"),
    ("zh-Hans", "ui.tiny", "太小"),
    ("zh-Hans", "ui.wired-controls-change-state-on-click-display-only-controls-dont-thats-a-core-boundary-not-a-bug", "接线类控件点击即改状态；纯展示类控件不改 —— 那是核心的边界，不是缺陷。"),
    ("zh-Hans", "ui.wired-to-state", "已接状态"),
    ("zh-Hans", "ui.appearance-lives-in-zstyle-source-sends-keys-and-class-names-only-no-colors-no-sizes", "外观住在 .zstyle 里。源码只发 key 与分类名 —— 没有颜色，没有尺寸。"),
    ("zh-Hans", "ui.colors-come-from-theme-tokens", "颜色来自主题令牌"),
    ("zh-Hans", "ui.this-color-is-content-secondary", "这个颜色是 @content_secondary"),
    ("zh-Hans", "ui.class-names-are-selectors", "分类名就是选择器"),
    ("zh-Hans", "ui.layout-affecting-properties", "影响布局的属性"),
    ("zh-Hans", "ui.spacing-is-declared-affects-layout-so-color-only-changes-skip-relayout", "间距声明为 affects_layout，因此只改颜色的更新会跳过重排"),
    ("zh-Hans", "ui.breakpoints-small-medium-large", "断点：小 / 中 / 大"),
    ("zh-Hans", "ui.touch-targets-need-44-epx", "触控目标需要 44 epx"),
    ("zh-Hans", "ui.theme-is-an-intent-not-a-reference-language-negotiates-from-system-prefs-then-manual-choice-wins", "主题是意图而非引用。语言先按系统偏好协商，再由手动选择优先。"),
    ("zh-Hans", "ui.language-en-zh-hans-ja", "语言（en / zh-Hans / ja）"),
    ("zh-Hans", "ui.current-locale", "当前区域"),
    ("zh-Hans", "ui.three-layers-of-self-verification-from-fastest-to-slowest-each-guards-a-different-class-of-silent-failure", "三层自验，从快到慢。每一层挡住一类不同的静默失效。"),
    ("zh-Hans", "ui.current-state", "当前状态"),
    ("zh-Hans", "ui.1-stylesheet-syntax", "1. 样式表语法"),
    ("zh-Hans", "ui.2-cpu-raster-dump", "2. CPU 光栅出图"),
    ("zh-Hans", "ui.dump-one-reference-family", "导出一个参考族"),
    ("zh-Hans", "ui.3-tree-level-gates", "3. 树级门禁"),
    ("zh-Hans", "ui.style-tokens-dead-rules-hit-test-breakpoints-no-appearance-in-source", "样式令牌、死规则、命中测试、断点、源码里没有外观"),
    ("zh-Hans", "ui.try-it-switch-language-same-state-as-lang", "试试：切换语言（与 --lang 同一状态）"),
    ("zh-Hans", "ui.control-reference-pick-a-family-to-see-its-controls", "控件参考 —— 选一个族看它的控件。"),
    // ---- 开关族（第三族） ----
    ("zh-Hans", "ui.switch", "开关"),
    ("zh-Hans", "ui.toggle", "切换"),
    ("zh-Hans", "ui.radio", "单选"),
    ("zh-Hans", "ui.switch.airplane-mode", "飞行模式"),
    ("zh-Hans", "ui.radio.off", "关"),
    ("zh-Hans", "ui.radio.on", "开"),
    ("zh-Hans", "ui.radio.auto", "自动"),
    ("zh-Hans", "ui.slider", "滑块"),
    ("zh-Hans", "ui.slider.volume", "音量"),
    ("zh-Hans", "ui.slider.brightness", "亮度"),
    ("zh-Hans", "ui.slider.demo", "液态滑块 — 拖拽或点按，实色 / 玻璃"),
    // ---- ja ----
    ("ja", "route.guide", "クイックスタート"),
    ("ja", "route.state", "状態"),
    ("ja", "route.styling", "スタイリング"),
    ("ja", "route.layout", "レイアウト"),
    ("ja", "route.theme", "テーマと多言語"),
    ("ja", "route.testing", "テスト"),
    ("ja", "route.reference", "コントロール"),
    ("ja", "route.materials", "マテリアルとガラス"),
    ("ja", "route.slider", "スライダー"),
    (
        "ja",
        "ui.small-nav-collapsed-to-icons",
        "小 — ナビはアイコン列に折りたたみ",
    ),
    ("ja", "ui.medium-nav-shows-labels", "中 — ナビにラベル表示"),
    ("ja", "ui.large-full-layout", "大 — フルレイアウト"),
    ("ja", "action.feedback", "フィードバック"),
    // ---- ページ本文（en と同順）----
    ("ja", "ui.zephyraui-gallery", "ZephyraUI ギャラリー"),
    (
        "ja",
        "ui.search-controls-and-samples",
        "コントロールとサンプルを検索…",
    ),
    ("ja", "ui.language", "言語"),
    ("ja", "ui.theme", "テーマ"),
    ("ja", "ui.route", "ルート"),
    ("ja", "ui.button", "ボタン"),
    ("ja", "ui.variants", "バリエーション"),
    ("ja", "ui.standard", "標準"),
    ("ja", "ui.accent", "アクセント"),
    ("ja", "ui.subtle", "サブトル"),
    ("ja", "ui.materials", "マテリアル"),
    ("ja", "ui.solid", "ソリッド"),
    ("ja", "ui.frosted", "フロスト"),
    ("ja", "ui.glass", "ガラス"),
    ("ja", "ui.primary", "プライマリ"),
    ("ja", "ui.pill", "ピル"),
    ("ja", "ui.with-icon-round", "アイコン付き / 丸"),
    ("ja", "ui.liquid-button", "リキッドボタン"),
    ("ja", "ui.options", "オプション"),
    ("ja", "ui.option.alpha", "アルファ"),
    ("ja", "ui.option.beta", "ベータ"),
    ("ja", "ui.option.gamma", "ガンマ"),
    ("ja", "ui.dropdown-button", "ドロップダウンボタン"),
    ("ja", "ui.liquid.demo", "LiquidButton — 押下/ドラッグ、ソリッド/グラス（LiquidButton.kt 1:1）"),
    ("ja", "ui.send", "送信"),
    ("ja", "ui.click-me", "クリックして"),
    ("ja", "ui.counter", "カウンター"),
    ("ja", "ui.multiple", "複数選択"),
    ("ja", "ui.checkbox", "チェックボックス"),
    ("ja", "ui.token-colored-text", "トークンで着色した文字"),
    ("ja", "ui.spacing-demo", "間隔のデモ"),
    ("ja", "ui.what-they-guard", "何を守っているか"),
    ("ja", "ui.you-are-here", "現在地"),
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
    ("ja", "ui.current-breakpoint", "現在のブレークポイント"),
    ("ja", "ui.44-epx-correct", "44 epx（正しい）"),
    ("ja", "ui.28-epx-too-small", "28 epx（小さすぎる）"),
    ("ja", "ui.materials-on-a-real-photo", "実写写真の上のマテリアル"),
    ("ja", "ui.materials-and-liquid-glass", "マテリアルとリキッドグラス"),
    ("ja", "ui.thin", "薄い"),
    ("ja", "ui.normal", "普通"),
    ("ja", "ui.thick", "厚い"),
    ("ja", "ui.decorations", "装飾"),
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
    // ---- 按钮 / 复选框族（label_key 通道）+ 页面散文 ----
    ("ja", "ui.check.toggle-me", "私を切り替え"),
    ("ja", "ui.check.enable-feature", "機能を有効にする"),
    ("ja", "ui.check.send-analytics", "分析を送信する"),
    ("ja", "ui.check.beta-channel", "ベータチャンネル"),
    ("ja", "ui.check.locked-option", "ロックされたオプション"),
    ("ja", "ui.big-enough", "十分大きい"),
    ("ja", "ui.tiny", "小さすぎる"),
    ("ja", "ui.wired-controls-change-state-on-click-display-only-controls-dont-thats-a-core-boundary-not-a-bug", "配線されたコントロールはクリックで状態が変わります。表示専用のコントロールは変わりません —— それはコアの境界であり、バグではありません。"),
    ("ja", "ui.wired-to-state", "状態に接続済み"),
    ("ja", "ui.appearance-lives-in-zstyle-source-sends-keys-and-class-names-only-no-colors-no-sizes", "外観は .zstyle にあります。ソースはキーとクラス名だけを送ります —— 色もサイズもありません。"),
    ("ja", "ui.colors-come-from-theme-tokens", "色はテーマトークンから"),
    ("ja", "ui.this-color-is-content-secondary", "この色は @content_secondary です"),
    ("ja", "ui.class-names-are-selectors", "クラス名はセレクタです"),
    ("ja", "ui.layout-affecting-properties", "レイアウトに影響するプロパティ"),
    ("ja", "ui.spacing-is-declared-affects-layout-so-color-only-changes-skip-relayout", "間隔は affects_layout として宣言されるため、色だけの変更は再レイアウトをスキップします"),
    ("ja", "ui.breakpoints-small-medium-large", "ブレークポイント：小 / 中 / 大"),
    ("ja", "ui.touch-targets-need-44-epx", "タッチターゲットには 44 epx が必要"),
    ("ja", "ui.theme-is-an-intent-not-a-reference-language-negotiates-from-system-prefs-then-manual-choice-wins", "テーマは参照ではなく意図です。言語はシステム設定から調整され、次に手動選択が優先されます。"),
    ("ja", "ui.language-en-zh-hans-ja", "言語（en / zh-Hans / ja）"),
    ("ja", "ui.current-locale", "現在のロケール"),
    ("ja", "ui.three-layers-of-self-verification-from-fastest-to-slowest-each-guards-a-different-class-of-silent-failure", "三層の自己検証。速いものから順に、それぞれ別の種類の静かな失敗を防ぎます。"),
    ("ja", "ui.current-state", "現在の状態"),
    ("ja", "ui.1-stylesheet-syntax", "1. スタイルシートの構文"),
    ("ja", "ui.2-cpu-raster-dump", "2. CPU ラスタダンプ"),
    ("ja", "ui.dump-one-reference-family", "参照ファミリーを一つ書き出す"),
    ("ja", "ui.3-tree-level-gates", "3. ツリー層のゲート"),
    ("ja", "ui.style-tokens-dead-rules-hit-test-breakpoints-no-appearance-in-source", "スタイルトークン、死んだルール、ヒットテスト、ブレークポイント、ソースに外観なし"),
    ("ja", "ui.try-it-switch-language-same-state-as-lang", "試してください：言語を切り替える（--lang と同じ状態）"),
    ("ja", "ui.control-reference-pick-a-family-to-see-its-controls", "コントロール参照 —— ファミリーを選んで控件を確認します。"),
    // ---- 开关族（第三族） ----
    ("ja", "ui.switch", "スイッチ"),
    ("ja", "ui.toggle", "トグル"),
    ("ja", "ui.radio", "ラジオ"),
    ("ja", "ui.switch.airplane-mode", "機内モード"),
    ("ja", "ui.radio.off", "オフ"),
    ("ja", "ui.radio.on", "オン"),
    ("ja", "ui.radio.auto", "自動"),
    ("ja", "ui.slider", "スライダー"),
    ("ja", "ui.slider.volume", "音量"),
    ("ja", "ui.slider.brightness", "明るさ"),
    ("ja", "ui.slider.demo", "リキッドスライダー — ドラッグ/タップ、ソリッド/グラス"),
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

/// 应用内的消息表（**控件层的默认文案已并进来**）。
///
/// # 为什么文案在这里而不在控件里
///
/// 源码里**没有面向用户的字符串** —— 与"源码零外观"是同一条纪律：
/// 控件发分类名、不发颜色；页面发 key、不发文案。漏翻由门禁枚举
/// （`every_tr_key_used_in_source_exists_in_every_language`），不靠肉眼。
///
/// # 为什么是 `&'static`（`OnceLock` 缓存）
///
/// `tr()` 在**每帧**被调（模板里 `tr('…')` 是逐元素的），而 `Messages` 的
/// 内部是 `BTreeMap` —— 每次重建就是 200 多次节点分配 + 一次整表分配。
/// 一次 `tr()` 一次整表，等于**每个文字元素都重建一遍文案表**。
///
/// 文案表是**编译期常量**（`CATALOG` 是 `&'static` 切片），运行期不会变 ⇒
/// 建一次就够。`OnceLock` 保证「只建一次」在多线程下也成立。
///
/// ⚠ 这不是微优化：控件模板里的 `tr(label_key)` 把每帧的 `tr()` 次数翻了一
/// 倍（按钮与复选框的标签各一次），不缓存的话热路径开销同比翻倍。
pub fn messages() -> &'static Messages {
    static MESSAGES: OnceLock<Messages> = OnceLock::new();
    MESSAGES.get_or_init(|| {
        let mut m = Messages::new();
        for (locale, key, value) in CATALOG {
            m.insert(locale, key, value);
        }
        // 控件层**自身语义**的文案（复选框组组头的「全选」）并进来。
        // 方向是单向的：应用条目优先 ⇒ 应用能改控件的默认说法。
        zephyra_ui::i18n::merge_builtin(&mut m);
        m
    })
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
    // 静态回退：id 是 `&'static`（来自 `ROUTES`），
    // 所以这里不需要额外分配。若调用方传了非静态串，门禁会先拦住。
    ROUTES
        .iter()
        .find(|(rid, _, _)| *rid == id)
        .map(|(rid, _, _)| *rid)
        .unwrap_or("route.unknown")
}

/// 解析一个 key 是否为合法路由，是则返回该路由 id（`&'static str`）。
///
/// # 路由来源只剩一处
///
/// 旧结构有三处（10 页 + 14 分组 + 44 控件页），后两处都来自
/// `catalog.rs`。那个模块随重写一并删掉，设置页也整页撤下，所以这里只剩
/// [`ROUTES`] 一个来源。
///
/// 一处也是**查表**：新增 id 只改那一张表，不用回来改这个函数。
/// 枚举式（`match`）会把新路由漏在 `_ => home` 里，静默回首页。
///
/// 返回 `&'static str`：路由要存进 [`Gallery::route`]（`&'static`），
/// 不能在运行时拼字符串 —— 那会每帧泄漏。
pub fn resolve_route(key: &str) -> Option<&'static str> {
    ROUTES.iter().map(|(id, _, _)| *id).find(|id| *id == key)
}

/// 列出全部合法路由（`--list-routes` 用）。
///
/// 之所以要“列出来”而不是让人猜：出图/调试时拼错一个 id 的后果是
/// **静默回到 home**（历史上已发生过）。把全集打印出来，拼错立刻可见。
pub fn list_routes() -> Vec<&'static str> {
    ROUTES.iter().map(|(id, _, _)| *id).collect()
}

// ---- 交互 key 常量表 ----
//
// # 为什么这批 key 住在 mod.rs 而不是各页面
//
// `Gallery::click` 收到的 key 是运行期 `&str`（`on_input` 里 `to_string()`
// 转交），而选中态字段是 `&'static str` —— 直接赋值存不下来。把 key 表
// 定义在这里，页面发的是表里那几项、click 按表还原，**同一份常量**服务
// 两处：key 表就是「哪些 key 会改状态」的清单，与门禁
// `every_interactive_key_is_wired_or_whitelisted` 的白名单共用。
//
// ⚠ state 章的 segmented 与 navigation 族的子项常量已随控件退场删掉 ——
// 那几族不再是演示内容，留着的常量会让「key 表 = 现存演示」的约定失真。

/// 触发类按钮（basics / layout / styling / menus / 外壳页头共用 `last_action`）。
///
/// ⚠ 新增触发按钮时把 key 加进这张表——漏加的症状是「点了没反应」，
/// 而门禁 `every_interactive_key_is_wired_or_whitelisted` 会先一步红。
pub const ACT_IDS: [&str; 16] = [
    "act:standard",
    "act:accent",
    "act:subtle",
    "act:solid",
    "act:frosted",
    "act:glass",
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

/// 复选框组的四个子项 id（选择页）。
///
/// 第四项是**禁用**的 —— 它演示三件事：禁用态画成灰的、点它没反应、
/// 「全选」不捎带改它（禁用项也不参与组头聚合，见
/// `zephyra_ui::CheckboxGroup::header_state` 的文档）。
pub const CHECKS: [&str; 4] = ["check:alpha", "check:beta", "check:gamma", "check:locked"];

/// 复选框组四个子项的**语义 key**（不是文案）。
///
/// 与 [`CHECKS`] 一一对应 —— 模板不再逐个写死 `label`，改由宿主把
/// `id / label_key / state / disabled` 打包成记录列表喂给 `CheckBoxGroup`；
/// 译文由控件模板内部 `tr()` 从文案表取。
///
/// ⚠ 这里存的是 **key** 而不是英文原文：`Checkbox` 的字段是 `label_key`
/// （见 `ZephyraUI/src/checkbox.rs`）。存原文的话查表必然落空，画面上会
/// 显示 key 本身 —— **看得见的降级**，但那是「漏翻」而不是「翻译」。
pub const CHECK_LABEL_KEYS: [&str; 4] = [
    "ui.check.enable-feature",
    "ui.check.send-analytics",
    "ui.check.beta-channel",
    "ui.check.locked-option",
];

/// 复选框组的**组头** key —— 点它 = 全选 / 全部不选。
///
/// 它同时是组头那颗复选框在**树上的 key**（宿主按它把点击分派回来）。
///
/// ⚠ 模板里它走 `CheckBoxGroup` 的 `gkey` **实参**而不是 Use 的 `key`
/// 属性 —— 否则 zypui 的「实例补丁」会把它打在组件体的第一个元素（外层
/// 容器）上，于是容器与组头**同 key**：按 key 找节点是深度优先第一个命中，
/// 焦点会写到容器上（焦点环画不到复选框），Space/Enter 也闩不住组头。
/// 详见 `ZephyraUI/ui/controls.zypui` 里 `CheckBoxGroup` 的注释。
pub const CHECK_GROUP_KEY: &str = "group:checks";

/// 参考页开关族（第三族）的演示数据。
///
/// # 为什么是常量而不是散在 `Default` 里
///
/// 门禁要按 key **逐个点击**并回读状态（`every_interactive_key_is_wired_or_whitelisted`），
/// 而"哪些 key 存在"这件事必须只有一个真相 —— 散在构造与分派两处，
/// 加一项时就会漏改一边，症状是"点了没反应"。
pub const SWITCH_DEMO_KEY: &str = "switch:demo";
pub const GLASS_DECOR_KEY: &str = "glass:decor";
pub const SWITCH_SOLID_KEY: &str = "switch:solid";
pub const SWITCH_GLASS_KEY: &str = "switch:glass";
pub const MAT_SOLID_KEY: &str = "mat:solid";
pub const MAT_GLASS_KEY: &str = "mat:glass";
/// 滑块（LiquidSlider 1:1）演示：实色 / 玻璃各一枚。
pub const SLIDER_SOLID_KEY: &str = "slider:solid";
pub const SLIDER_GLASS_KEY: &str = "slider:glass";
/// 胶囊液态按钮（LiquidButton 1:1）演示：实色 / 玻璃各一枚。
pub const LIQUID_BTN_SOLID_KEY: &str = "liquid:btn:solid";
pub const LIQUID_BTN_GLASS_KEY: &str = "liquid:btn:glass";
/// 下拉按钮（液态材质）演示：实色 / 玻璃各一枚。
pub const LIQUID_DD_SOLID_KEY: &str = "liquid:dd:solid";
pub const LIQUID_DD_GLASS_KEY: &str = "liquid:dd:glass";

/// 该 key 是否属于某个下拉按钮（本体或其菜单项）。
///
/// Down 的「点外部收起」与 Click 的「不要误收」共用这一条判据：分开写两份时，
/// 改一处漏一处会让菜单要么点自己就消失、要么点外面关不掉。
pub fn is_dd_key(key: &str) -> bool {
    key == LIQUID_DD_SOLID_KEY
        || key == LIQUID_DD_GLASS_KEY
        || key.starts_with("liquid:dd:solid:opt")
        || key.starts_with("liquid:dd:glass:opt")
}
/// 装饰开关的标签语义 key。
pub const GLASS_DECOR_LABEL_KEY: &str = "ui.decorations";
/// 开关的标签语义 key（译文由控件模板内部 `tr()` 取）。
pub const SWITCH_LABEL_KEY: &str = "ui.switch.airplane-mode";
/// 单选组的三个选项（顺序即显示顺序）。
pub const RADIO_KEYS: [&str; 3] = ["radio:off", "radio:on", "radio:auto"];
/// 三个单选选项的标签语义 key，与 [`RADIO_KEYS`] 一一对应。
pub const RADIO_LABEL_KEYS: [&str; 3] = ["ui.radio.off", "ui.radio.on", "ui.radio.auto"];
/// ComboBox 演示选项（`'static`：控件签名收静态切片，与 demo 的常量同手法）。
///
/// 取的是本仓**应用内**已有的材质名（纯色 / 磨砂 / 液态玻璃）：DWM 系统材质
/// （Mica/Acrylic）已于 2026-11 移除，演示数据不该再引用一个不存在的功能。
pub const COMBO_OPTIONS: &[&str] = &["Solid", "Frosted", "Glass", "None"];

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
    /// 当前拥有键盘焦点的 key（来自 InputState）。模板据此发 `focused` 类。
    pub focused: Option<ElementKey>,
    /// 想要深色主题（意图；宿主执行）。
    pub wants_dark: bool,
    /// 主题意图的共享格（窗口路用；dump 路直接读 `wants_dark`）。
    pub theme_intent: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
    /// 选择页的**复选框组**（组头全选 + 三个各自独立的子项）。
    ///
    /// # 为什么用控件层给的状态模型，而不是 `[bool; 3]`
    ///
    /// 组头的「勾 / 半 / 不勾」是**从子项聚合派生**的，不是另存一份。
    /// 另存一份就会出现两份真相 —— 子项变了、组头忘了改，而那种不一致在
    /// 画面上完全看不出来：头还是画成「全选」，实际却只选了两个。
    ///
    /// 流转规则（半选点一下 = 全选、全不选 = 未选中、全选 = 已选中）
    /// 与聚合规则都住在 `zephyra_ui::checkbox` 里，本页只负责「点哪个 key」。
    pub check_group: zephyra_ui::CheckboxGroup,
    /// 参考页开关族（第三族）的**开关**（两态）。
    ///
    /// 用控件层的 `ToggleSwitch` 而不是裸 `bool`：开关的两态流转与「禁用不翻转」
    /// 都在 `zephyra_ui::toggle` 里，本页只负责「点了哪个 key」。
    pub switch_demo: zephyra_ui::ToggleSwitch,
    /// 参考页开关族（第三族）的**单选组**（至多一项选中）。
    pub radio_group: zephyra_ui::RadioGroup,
    /// 液态玻璃页的四个折射参数（**运行期可调**）。
    ///
    /// # 为什么它住在应用状态而不是样式表
    ///
    /// 参数写进 `.zstyle` 就只剩三档静态值，而这个页面的职责恰恰是让人**调**
    /// 它们。见 [`glass::Glass`] 的文档 —— `FILTER` 不在门禁的 APPEARANCE
    /// 列表里是**有意的**：它的四个参数是物理量（厚度 / 折射率 / 色散 / 倒角
    /// 宽度），既没有 `@令牌` 对应物，也不随主题变化。
    pub glass: crate::app::glass::Glass,
    /// 玻璃装饰开关（玻璃页）。
    ///
    /// 控制三块玻璃的 `decoration`（`glass` ↔ `none`），**默认开**。
    /// 开/关都走同一个控件模型 —— 与拨杆的把手动画同一条路径。
    pub glass_decor: zephyra_ui::ToggleSwitch,
    pub switch_solid: zephyra_ui::ToggleSwitch,
    pub switch_glass: zephyra_ui::ToggleSwitch,
    pub mat_solid: zephyra_ui::ToggleSwitch,
    pub mat_glass: zephyra_ui::ToggleSwitch,
    /// 滑块演示（LiquidSlider 原版语义）：实色 / 玻璃各一枚。
    pub slider_solid: zephyra_ui::Slider,
    pub slider_glass: zephyra_ui::Slider,
    /// 胶囊液态按钮演示（LiquidButton 原版语义）：实色 / 玻璃各一枚。
    pub liquid_btn_solid: zephyra_ui::LiquidButton,
    pub liquid_btn_glass: zephyra_ui::LiquidButton,
    /// 下拉按钮（液态材质）演示：实色 / 玻璃各一枚。
    pub liquid_dd_solid: zephyra_ui::LiquidButton,
    pub liquid_dd_glass: zephyra_ui::LiquidButton,
    pub liquid_dd_solid_open: bool,
    pub liquid_dd_glass_open: bool,
    /// 菜单开合动画进度（0=收起, 1=展开），由 advance 按 dt 逼近目标。
    pub liquid_dd_solid_anim: f32,
    pub liquid_dd_glass_anim: f32,
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
    /// state 章 checkbox（独立于选择页的组：教学页要自证）。
    ///
    /// 用控件层给的 `Checkbox` 而不是裸 `bool`：这一章教的是「状态怎么存」，
    /// 而「复选框的状态是个枚举、两态框只会用到其中两个值」正是要教的事。
    pub demo_check: zephyra_ui::Checkbox,
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

    /// **强制**指定语言（命令行 `--lang` 用）。
    ///
    /// # 为什么必须走这个方法，而不能直接改 `lang`
    ///
    /// 直接改 `lang` 的话，窗口路第一帧的**系统协商**会把它改回系统语言 ——
    /// 本机系统语言是 zh-Hans，于是 `--lang en` 也显示中文。
    ///
    /// 症状特别难查：命令行说「强制」，界面不听；而在**系统语言恰好就是
    /// 目标语言**的机器上（大多数中文开发机都是），这个 bug 完全看不出来 ——
    /// 只有换一台系统语言不同的机器才复现。
    ///
    /// 标记 `lang_negotiated` = 「语言已经定了，别再协商」—— 与用户**手动**
    /// 点「日本語」走的是同一条路（见 `click` 里的 `lang:` 分支）。
    pub fn force_language(&mut self, idx: usize) {
        self.lang = idx.min(SUPPORTED.len() - 1);
        self.lang_negotiated = true;
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
            focused: None,
            wants_dark: true,
            theme_intent: None,
            ref_family: 0,
            guide_count: 0,
            last_action: "",
            pill_on: false,
            // ⚠ 第二个实参是**语义 key** 不是文案 —— 控件模板内部 `tr()` 翻译它。
            demo_check: zephyra_ui::Checkbox::new("state-check", "ui.check.toggle-me")
                .with_state(zephyra_ui::CheckState::Checked),
            check_group: {
                // 组头 + 四个子项。初始勾上第一个（对应老版本 `[true, false, false]`），
                // 于是组头一开始就是「半选」—— 一进页面就看得见三态。
                //
                // 第四项是**禁用**的：它不参与组头聚合，也不被「全选」捎带改。
                // ⚠ 组头的标签 key **不传** —— 它是**控件语义**（组头 = 全选 /
                // 全部不选），由控件层自带默认译文（`zephyra.checkbox.select-all`，
                // 见 `zephyra_ui::i18n`）。应用想改说法才调 `with_header_label_key`。
                let mut g = zephyra_ui::CheckboxGroup::new(CHECK_GROUP_KEY);
                for (i, (key, label_key)) in CHECKS.iter().zip(CHECK_LABEL_KEYS).enumerate() {
                    g.push(key, label_key);
                    let item = g.item_mut(key).expect("刚 push 进去的子项必然存在");
                    if i == 0 {
                        item.set_state(zephyra_ui::CheckState::Checked);
                    }
                    if *key == "check:locked" {
                        item.set_disabled(true);
                    }
                }
                g
            },
            glass: crate::app::glass::Glass::default(),
            // 装饰默认**开**（与 `Decorations::glass()` 同款；`none` 是
            // 恒等，画面零差别只是不进离屏 pass）。
            glass_decor: zephyra_ui::ToggleSwitch::new(GLASS_DECOR_KEY, GLASS_DECOR_LABEL_KEY)
                .with_checked(true)
                .with_material(zephyra_core::filter::Material::Glass),
            // 开关默认**关**着；单选组默认选中第一项（「关」）—— 一进页面就看得见
            // 两种状态，不必先点一下才知道控件长什么样。
            switch_demo: zephyra_ui::ToggleSwitch::new(SWITCH_DEMO_KEY, SWITCH_LABEL_KEY)
                .with_material(zephyra_core::filter::Material::Glass),
            switch_solid: zephyra_ui::ToggleSwitch::new(SWITCH_SOLID_KEY, "ui.solid"),
            switch_glass: zephyra_ui::ToggleSwitch::new(SWITCH_GLASS_KEY, "ui.glass")
                .with_material(zephyra_core::filter::Material::Glass),
            mat_solid: zephyra_ui::ToggleSwitch::new(MAT_SOLID_KEY, "ui.solid"),
            mat_glass: zephyra_ui::ToggleSwitch::new(MAT_GLASS_KEY, "ui.glass")
                .with_material(zephyra_core::filter::Material::Glass),
            // 滑块：0..100，visibilityThreshold 0.01（原版 SliderContent）；初值 50。
            slider_solid: zephyra_ui::Slider::new(
                SLIDER_SOLID_KEY,
                "ui.slider.volume",
                0.0,
                100.0,
                0.01,
            )
            .with_value(50.0),
            slider_glass: zephyra_ui::Slider::new(
                SLIDER_GLASS_KEY,
                "ui.slider.brightness",
                0.0,
                100.0,
                0.01,
            )
            .with_value(50.0)
            .with_material(zephyra_core::filter::Material::Glass),
            liquid_btn_solid: zephyra_ui::LiquidButton::new(LIQUID_BTN_SOLID_KEY)
                .with_size(180.0, 48.0),
            liquid_btn_glass: zephyra_ui::LiquidButton::new(LIQUID_BTN_GLASS_KEY)
                .with_size(180.0, 48.0)
                .with_material(zephyra_core::filter::Material::Glass),
            liquid_dd_solid: zephyra_ui::LiquidButton::new(LIQUID_DD_SOLID_KEY)
                .with_size(160.0, 36.0),
            liquid_dd_glass: zephyra_ui::LiquidButton::new(LIQUID_DD_GLASS_KEY)
                .with_size(160.0, 36.0)
                .with_material(zephyra_core::filter::Material::Glass),
            liquid_dd_solid_open: false,
            liquid_dd_glass_open: false,
            liquid_dd_solid_anim: 0.0,
            liquid_dd_glass_anim: 0.0,
            radio_group: {
                let mut g = zephyra_ui::RadioGroup::new();
                for (key, label_key) in RADIO_KEYS.iter().zip(RADIO_LABEL_KEYS) {
                    g.push(key, label_key);
                }
                g.select(RADIO_KEYS[0]);
                g
            },
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
    pub fn is_focused(&self, key: &str) -> bool {
        self.focused.as_ref().and_then(|k| k.as_name()) == Some(key)
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
        // 点外部关闭两个下拉菜单（与下拉无关的点击）。
        let dd_related = is_dd_key(key);
        if !dd_related {
            self.liquid_dd_solid_open = false;
            self.liquid_dd_glass_open = false;
        }
        match key {
            // 主题意图的入口是页头的 tool_button（`theme-toggle`）——
            // 2026-11-01 之前还有设置页的总开关（`switch:master`），
            // 随设置页一起撤下了。
            "theme-toggle" => {
                self.wants_dark = !self.wants_dark;
                // 窗口路：把意图写进共享格，Bootstrap 每帧对账后执行。
                if let Some(cell) = &self.theme_intent {
                    cell.store(
                        if self.wants_dark { 0 } else { 1 },
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
            // 复选框组：组头 = 全选 / 全部不选；子项 = 只改自己。
            //
            // ⚠ 子项走**前缀分支**：`toggle_item` 认 key，找不到就返回 false
            // （不改任何状态）。漏接的症状是「点了没反应」，而门禁
            // `every_interactive_key_is_wired_or_whitelisted` 正是要消灭它。
            CHECK_GROUP_KEY => self.check_group.toggle_all(),
            _ if key.starts_with("check:") => {
                self.check_group.toggle_item(key);
            }
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
            // basics 页的 pill（可保持按下）。
            //
            // round-a / round-b 是 round_button 的**两种外观**（普通 / checked），
            // 演示的是「同一个按钮的长相」，不是可切换状态 —— 没有分派分支，
            // 门禁白名单里注明。
            "pill-a" => self.pill_on = !self.pill_on,
            // ---- state 章：教学页必须自证 ----
            //
            // 这一章的职责是教「哪些控件真的接了状态」。若「Wired to state」
            // 区里的控件自己点了没反应，这一页就在说谎——比别页的空壳更糟。
            "state-check" => self.demo_check.toggle(),
            // ---- 参考页开关族（第三族） ----
            //
            // 开关：两态翻转；单选钮：选中自己，同组其余由 `RadioGroup` 取消。
            // 两条分派都只调控件层的方法 —— 「怎么变」住在 `zephyra_ui::toggle`。
            SWITCH_DEMO_KEY => {
                if let Some(target) = self.switch_demo.take_drag_completed_target_checked() {
                    self.switch_demo.set_checked(target);
                } else {
                    self.switch_demo.toggle();
                }
            }
            // 玻璃页「装饰」开关：控制三块玻璃的 `decoration`（glass ↔ none）。
            GLASS_DECOR_KEY => {
                if let Some(target) = self.glass_decor.take_drag_completed_target_checked() {
                    self.glass_decor.set_checked(target);
                } else {
                    self.glass_decor.toggle();
                }
            }
            SWITCH_SOLID_KEY => {
                if let Some(target) = self.switch_solid.take_drag_completed_target_checked() {
                    self.switch_solid.set_checked(target);
                } else {
                    self.switch_solid.toggle();
                }
            }
            SWITCH_GLASS_KEY => {
                if let Some(target) = self.switch_glass.take_drag_completed_target_checked() {
                    self.switch_glass.set_checked(target);
                } else {
                    self.switch_glass.toggle();
                }
            }
            MAT_SOLID_KEY => {
                if let Some(target) = self.mat_solid.take_drag_completed_target_checked() {
                    self.mat_solid.set_checked(target);
                } else {
                    self.mat_solid.toggle();
                }
            }
            MAT_GLASS_KEY => {
                if let Some(target) = self.mat_glass.take_drag_completed_target_checked() {
                    self.mat_glass.set_checked(target);
                } else {
                    self.mat_glass.toggle();
                }
            }
            // 滑块：拖拽提交由 take_committed_value 体现；点按跳转在 process_pointer_events
            // （track_left 已知时）。无位置/拖拽上下文的激活（click/键盘）做步进回退，保证可见反馈。
            SLIDER_SOLID_KEY => {
                if self.slider_solid.take_committed_value().is_none() {
                    let v = self.slider_solid.value();
                    let next = if v >= 95.0 { 0.0 } else { v + 5.0 };
                    self.slider_solid.set_value_immediate(next);
                }
            }
            SLIDER_GLASS_KEY => {
                if self.slider_glass.take_committed_value().is_none() {
                    let v = self.slider_glass.value();
                    let next = if v >= 95.0 { 0.0 } else { v + 5.0 };
                    self.slider_glass.set_value_immediate(next);
                }
            }
            LIQUID_BTN_SOLID_KEY | LIQUID_BTN_GLASS_KEY => {
                // 胶囊按钮：激活即点击（原版 onClick）；切换 pill 状态给出可见反馈（满足接线门禁）。
                self.pill_on = !self.pill_on;
            }
            LIQUID_DD_SOLID_KEY => {
                let next = !self.liquid_dd_solid_open;
                self.liquid_dd_solid_open = next;
                if next {
                    self.liquid_dd_glass_open = false;
                }
            }
            LIQUID_DD_GLASS_KEY => {
                let next = !self.liquid_dd_glass_open;
                self.liquid_dd_glass_open = next;
                if next {
                    self.liquid_dd_solid_open = false;
                }
            }
            _ if key.starts_with("liquid:dd:solid:opt") => {
                self.liquid_dd_solid_open = false;
                self.last_action = "liquid-dd-solid";
            }
            _ if key.starts_with("liquid:dd:glass:opt") => {
                self.liquid_dd_glass_open = false;
                self.last_action = "liquid-dd-glass";
            }
            _ if key.starts_with("radio:") => {
                self.radio_group.select(key);
            }
            // ---- 控件参考页的族切换 ----
            //
            // key 形如 `ref:buttons`。走 `REF_FAMILIES` 查表而不是逐族 `match`：
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
        // 先把本帧的 hover/pressed/focus 同步给模型——Down 判断要读 pressed；
        // `last_pressed` 则在点击判据用完后更新（见后）。
        self.hovered = state.hovered.clone();
        self.pressed = state.pressed.clone();
        self.focused = state.focused.clone();

        // 点外部收起（含**空白处**）。不能只靠下面的 click 分派：Click 由
        // 「按下与抬起同键」配对出来，而空白处没有可交互目标 ⇒ router 不产生
        // Click ⇒ 只靠 click() 的判据会让菜单「点外面关不掉」。
        //
        // Down 时就用 hovered 判归属：按在下拉本体/菜单项上不动，按在别处立即收。
        // 按在菜单项上时，收起与记录动作留给 Up 的 click（那里才知道选的是哪项）。
        if state
            .pointer_events
            .iter()
            .any(|e| e.phase == PointerInputPhase::Down)
        {
            let related = state
                .hovered
                .as_ref()
                .and_then(|k| k.as_name())
                .is_some_and(is_dd_key);
            if !related {
                self.liquid_dd_solid_open = false;
                self.liquid_dd_glass_open = false;
            }
        }

        // 顺序要紧：先**同步 pressed 目标**并让模型处理指针事件（Up 会把拖拽
        // 提交的目标暂存起来），再跑激活漏斗 —— 反过来的话 `click()` 会在模型
        // 还没看到 Up 时先 toggle，拖拽永远走不到「按阈值提交」那条路。
        let switch_now_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(SWITCH_DEMO_KEY);
        self.switch_demo.set_pressed(switch_now_pressed);
        self.switch_demo
            .process_pointer_events(&state.pointer_events, state.scale);

        // 玻璃页装饰开关，同一顺序。
        let decor_now_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(GLASS_DECOR_KEY);
        self.glass_decor.set_pressed(decor_now_pressed);
        self.glass_decor
            .process_pointer_events(&state.pointer_events, state.scale);

        let solid_now_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(SWITCH_SOLID_KEY);
        self.switch_solid.set_pressed(solid_now_pressed);
        self.switch_solid
            .process_pointer_events(&state.pointer_events, state.scale);

        let glass_now_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(SWITCH_GLASS_KEY);
        self.switch_glass.set_pressed(glass_now_pressed);
        self.switch_glass
            .process_pointer_events(&state.pointer_events, state.scale);

        let mat_solid_now_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(MAT_SOLID_KEY);
        self.mat_solid.set_pressed(mat_solid_now_pressed);
        self.mat_solid
            .process_pointer_events(&state.pointer_events, state.scale);

        let mat_glass_now_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(MAT_GLASS_KEY);
        self.mat_glass.set_pressed(mat_glass_now_pressed);
        self.mat_glass
            .process_pointer_events(&state.pointer_events, state.scale);

        let slider_solid_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(SLIDER_SOLID_KEY);
        self.slider_solid.set_pressed(slider_solid_pressed);
        self.slider_solid
            .process_pointer_events(&state.pointer_events, state.scale);
        let slider_glass_pressed =
            self.pressed.as_ref().and_then(|k| k.as_name()) == Some(SLIDER_GLASS_KEY);
        self.slider_glass.set_pressed(slider_glass_pressed);
        self.slider_glass
            .process_pointer_events(&state.pointer_events, state.scale);

        let lb_s = self.pressed.as_ref().and_then(|k| k.as_name()) == Some(LIQUID_BTN_SOLID_KEY);
        self.liquid_btn_solid.set_pressed(lb_s);
        self.liquid_btn_solid
            .process_pointer_events(&state.pointer_events, state.scale);
        let lb_g = self.pressed.as_ref().and_then(|k| k.as_name()) == Some(LIQUID_BTN_GLASS_KEY);
        self.liquid_btn_glass.set_pressed(lb_g);
        self.liquid_btn_glass
            .process_pointer_events(&state.pointer_events, state.scale);
        let dd_s = self.pressed.as_ref().and_then(|k| k.as_name()) == Some(LIQUID_DD_SOLID_KEY);
        self.liquid_dd_solid.set_pressed(dd_s);
        self.liquid_dd_solid
            .process_pointer_events(&state.pointer_events, state.scale);
        let dd_g = self.pressed.as_ref().and_then(|k| k.as_name()) == Some(LIQUID_DD_GLASS_KEY);
        self.liquid_dd_glass.set_pressed(dd_g);
        self.liquid_dd_glass
            .process_pointer_events(&state.pointer_events, state.scale);

        // click 还原：上一帧按着、这一帧松了，且 key 相同（按住期间不放）。
        // 在模型处理 Up 之后再判，拖拽提交才能走 `take_drag_completed_target_checked()`。
        let click_key = match (&self.last_pressed, &state.pressed) {
            (Some(prev), None) => Some(prev.as_name().unwrap_or_default().to_string()),
            _ => None,
        };
        if let Some(key) = click_key {
            self.click(&key);
        }
        self.last_pressed = state.pressed.clone();

        self.switch_demo.advance(dt);
        self.glass_decor.advance(dt);
        self.switch_solid.advance(dt);
        self.switch_glass.advance(dt);
        self.mat_solid.advance(dt);
        self.mat_glass.advance(dt);
        self.slider_solid.advance(dt);
        self.slider_glass.advance(dt);
        self.liquid_btn_solid.advance(dt);
        self.liquid_btn_glass.advance(dt);
        self.liquid_dd_solid.advance(dt);
        self.liquid_dd_glass.advance(dt);
        // 下拉菜单开合动画（指数逼近，~180ms 视觉量级；目标由 *_open 决定）。
        fn dd_anim_toward(v: &mut f32, target: f32, dt_ms: f32) {
            if !v.is_finite() {
                *v = target;
                return;
            }
            let dt = if dt_ms.is_finite() && dt_ms > 0.0 {
                dt_ms.min(64.0)
            } else {
                return;
            };
            let k = 1.0 - (-dt / 55.0_f32).exp();
            *v += (target - *v) * k;
            if (*v - target).abs() < 0.001 {
                *v = target;
            }
            *v = v.clamp(0.0, 1.0);
        }
        dd_anim_toward(
            &mut self.liquid_dd_solid_anim,
            if self.liquid_dd_solid_open { 1.0 } else { 0.0 },
            dt,
        );
        dd_anim_toward(
            &mut self.liquid_dd_glass_anim,
            if self.liquid_dd_glass_open { 1.0 } else { 0.0 },
            dt,
        );

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

    /// 还有动画在跑吗 —— 宿主据此决定要不要继续泵帧。
    ///
    /// # 为什么这件事必须在应用侧回答
    ///
    /// 宿主的保留式节奏是「有事件才画」：静止时 `tick()` 连一帧都不跑、
    /// 事件循环 `Wait`。可动画是**时间**驱动的，不是事件驱动的 —— 只有
    /// 有人回答「还没收敛」，宿主才会在动画期间保持 Poll，把中间帧泵出来。
    /// 缺了它，把手/入场/玻璃的弹簧只在鼠标动一下时推进一格，随后循环
    /// 睡死（症状：动画看起来是"跳"过去的、几乎看不见曲线）。
    ///
    /// 三处是本页的**全部**动画源：
    ///
    /// - `enter_ms`：入场动画时钟，跑满 `ENTER_MS` 后停在端点（见 `on_input`）；
    /// - `switch_demo` / `glass_decor`：开关模型的弹簧
    ///   （[`zephyra_ui::ToggleSwitch::is_animating`] —— 与 `advance` 的钉死
    ///   规则配对，收敛后精确为 false）。
    ///
    /// ⚠ 不能用「`render` 统计变了没有」代替：动画收敛到端点后属性 diff 确实
    /// 恒为零，但那要到**下一帧**才看得见 —— 那时循环已经睡了。
    fn wants_frames(&self) -> bool {
        self.enter_ms < zephyra_style::anim::ENTER_MS
            || self.switch_demo.is_animating()
            || self.glass_decor.is_animating()
            || self.switch_solid.is_animating()
            || self.switch_glass.is_animating()
            || self.mat_solid.is_animating()
            || self.mat_glass.is_animating()
            || self.slider_solid.is_animating()
            || self.slider_glass.is_animating()
            || self.liquid_btn_solid.is_animating()
            || self.liquid_btn_glass.is_animating()
            || self.liquid_dd_solid.is_animating()
            || self.liquid_dd_glass.is_animating()
            || (self.liquid_dd_solid_anim > 0.001 && self.liquid_dd_solid_anim < 0.999)
            || (self.liquid_dd_glass_anim > 0.001 && self.liquid_dd_glass_anim < 0.999)
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        crate::ui::window(self)
    }
}

// ───────────────────────── zypui 宿主 ─────────────────────────
//
// 每帧一棵树：`Window` 页从 `frontend/shell.zypui` 实例化，模板经 @绑定 读这里
// 的状态、经 tr('…') 读文案、经 hover/press 内置词读输入态。绑定名与
// shell.zypui 里的 @引用一一对应 —— 模板引用了不存在的名字时求值为假
// （画面直接可见），真拦住漂移的是 parity 测试。

impl zypui::Host for Gallery {
    fn binding(&self, name: &str) -> Option<zypui::Val> {
        use crate::app::data::*;
        use zypui::Val::{B, F, L, S};
        let g = self;
        Some(match name {
            // ---- 外壳 ----
            "route" => S(g.route),
            "compact" => B(g.is_compact()),
            "enter_class" => S(g.enter_class().unwrap_or("")),
            "route_title" => S(g.tr(route_key(g.route))),
            "last_action" => S(g.last_action),
            "caps" => L(CAPTION_BUTTONS
                .iter()
                .map(|(key, icon, close)| {
                    rec(vec![
                        ("key", S(key)),
                        ("icon", S(icon)),
                        ("close", B(*close)),
                    ])
                })
                .collect()),
            "routes" => L(ROUTES
                .iter()
                .map(|(id, _, icon)| {
                    rec(vec![
                        ("id", S(id)),
                        ("label", S(g.tr(route_key(id)))),
                        ("icon", S(icon)),
                    ])
                })
                .collect()),

            "cnt_text" => S(count_text(g.guide_count)),
            "demo_check" => S(g.demo_check.class_word()),
            // 复选框组：**两个**绑定撑起整组 —— 组头的聚合态 + 子项列表。
            //
            // ⚠ 状态以**分类词**（"" / "checked" / "indeterminate"）过界，
            // 不是布尔：模板要区分三个态，而布尔装不下第三个。分类词由
            // `zephyra_ui::CheckState::class_word()` 产出，与样式表的
            // `.checked` / `.indeterminate` 同名 —— 两处同改。
            "check_group_state" => S(g.check_group.header_state().class_word()),
            // ⚠ 子项给的是 **label_key**（语义 key），不是文案 ——
            // 译文由控件模板内部 `tr()` 产生。给文案的话切语言时子项
            // 标签不会跟着变，而**组头**变了 —— 一组里两种语言，最难查。
            "check_group_items" => L(g
                .check_group
                .items()
                .iter()
                .map(|i| {
                    rec(vec![
                        ("id", S(i.key())),
                        ("label_key", S(i.label_key())),
                        ("state", S(i.class_word())),
                        ("disabled", B(i.is_disabled())),
                    ])
                })
                .collect()),
            // 组头的标签 key 来自**控件层**的默认（`zephyra.checkbox.select-all`）。
            // Rust 模型是唯一真相源 ⇒ 模板不必再写一遍 key，也就不会出现
            // 「模型说 A、模板说 B」这种两处不一致（画面上只表现为文案不对）。
            "check_group_header_key" => S(g.check_group.header_label_key()),

            "bp_text" => S(bp_text(g)),
            "wants_dark" => B(g.wants_dark),

            "lang" => F(g.lang as f32),
            "langs" => langs(),

            "locale_name" => S(g.locale()),

            "theme_text" => S(if g.wants_dark { "Dark" } else { "Light" }),
            // ---- 参考层 ----
            "ref_segs" => ref_segs(),
            "ref_sel" => S(REF_FAMILIES
                .get(g.ref_family)
                .copied()
                .map(seg_key)
                .unwrap_or("ref:buttons")),
            "ref_family" => F(g.ref_family as f32),
            // ---- buttons ----
            "pill_on" => B(g.pill_on),

            // ---- checkbox ----

            // ---- switches（参考页第三族） ----
            //
            // 开关与单选钮的**状态**都从控件层模型派生：模板只收"这个词 / 真假"，
            // 不认识「两态」「单选互斥」是什么。
            "switch_demo_on" => B(g.switch_demo.is_checked()),
            // 把手：位移 = fraction × 行程、按压缩放 = 1 + 0.5 × press。
            // 与 `decoration` 同一条动态属性路径。
            "switch_knob_x" => F(g.switch_demo.knob_x()),
            "switch_knob_scale_x" => F(g.switch_demo.knob_scale_x()),
            "switch_knob_scale_y" => F(g.switch_demo.knob_scale_y()),
            // 1:1 复刻 LiquidToggle：把手与轨道的每帧「物理量」绑定。
            // 颜色也是逐帧插值的（原版 drawRect(lerp(...))），不走两个静态分类名。
            "switch_knob_bg" => switch_knob_bg_value(&g.switch_demo),
            "switch_knob_filter" => switch_knob_filter_value(&g.switch_demo),
            // 把手玻璃装饰（高光 / 把手投影 / 内阴影）随**按压进度**逐帧重建：
            // 松开时 control_with_progress(0) 的高光 / 内阴影全为 0（恒等），
            // 按住时渐显到 control 档。装饰落在把手上（原版 drawBackdrop 挂在
            // knob 上）。
            "switch_knob_decoration" => switch_knob_decoration_value(&g.switch_demo),
            "radio_items" => L(g
                .radio_group
                .options()
                .iter()
                .map(|o| {
                    rec(vec![
                        ("id", S(o.key())),
                        ("label_key", S(o.label_key())),
                        ("checked", B(g.radio_group.is_selected(o.key()))),
                    ])
                })
                .collect()),

            // ---- glass ----
            "glass_params" => {
                let (t, bz, d, i) = g.glass.texts();
                let values = [t, bz, d, i];
                L(crate::app::glass::Param::ALL
                    .iter()
                    .enumerate()
                    .map(|(n, p)| {
                        rec(vec![
                            ("label", S(p.label())),
                            ("dec", S(glass_step_key(*p, false))),
                            ("inc", S(glass_step_key(*p, true))),
                            ("value", S(values[n])),
                        ])
                    })
                    .collect())
            }
            "glass_filter_0" => zypui::Val::Raw(glass_filter_value(g, 0)),
            "glass_filter_1" => zypui::Val::Raw(glass_filter_value(g, 1)),
            "glass_filter_2" => zypui::Val::Raw(glass_filter_value(g, 2)),
            // ---- glass：装饰开关 ----
            "glass_decor_on" => B(g.glass_decor.is_checked()),
            "glass_decor_knob_x" => F(g.glass_decor.knob_x()),
            "glass_decor_knob_scale_x" => F(g.glass_decor.knob_scale_x()),
            "glass_decor_knob_scale_y" => F(g.glass_decor.knob_scale_y()),
            "glass_decor_knob_bg" => switch_knob_bg_value(&g.glass_decor),
            "glass_decor_knob_filter" => switch_knob_filter_value(&g.glass_decor),
            "glass_decor_knob_decoration" => switch_knob_decoration_value(&g.glass_decor),
            // ---- 材质对比开关（PageSwitches / MaterialBoard 各一对） ----
            "switch_solid_on" => B(g.switch_solid.is_checked()),
            "switch_solid_x" => F(g.switch_solid.knob_x()),
            "switch_solid_knob_bg" => solid_knob_bg_value(),
            "switch_glass_on" => B(g.switch_glass.is_checked()),
            "switch_glass_x" => F(g.switch_glass.knob_x()),
            "switch_glass_scale_x" => F(g.switch_glass.knob_scale_x()),
            "switch_glass_scale_y" => F(g.switch_glass.knob_scale_y()),
            "switch_glass_knob_bg" => switch_knob_bg_value(&g.switch_glass),
            "switch_glass_knob_filter" => switch_knob_filter_value(&g.switch_glass),
            "switch_glass_knob_decoration" => switch_knob_decoration_value(&g.switch_glass),
            "mat_solid_on" => B(g.mat_solid.is_checked()),
            "mat_solid_x" => F(g.mat_solid.knob_x()),
            "mat_solid_knob_bg" => solid_knob_bg_value(),
            "mat_glass_on" => B(g.mat_glass.is_checked()),
            "mat_glass_x" => F(g.mat_glass.knob_x()),
            "mat_glass_scale_x" => F(g.mat_glass.knob_scale_x()),
            "mat_glass_scale_y" => F(g.mat_glass.knob_scale_y()),
            "mat_glass_knob_bg" => switch_knob_bg_value(&g.mat_glass),
            "mat_glass_knob_filter" => switch_knob_filter_value(&g.mat_glass),
            "mat_glass_knob_decoration" => switch_knob_decoration_value(&g.mat_glass),
            // 装饰本体：`glass` ↔ `none`。走 `Val::Raw`（结构值）——
            // 与控件把手 `knob_decoration` 的 Raw 通道同一条路；`none` 是恒等，
            // 关时三块玻璃不进离屏 pass。
            // ---- slider（LiquidSlider 原版，实色/玻璃对比）----
            "slider_solid_fill_w" => F(g.slider_solid.fill_width()),
            // 拇指在水平流中排在 fill 后面：translate-x 从布局位置起算，
            // 需减去 fill 宽度才能对齐到轨道原点（thumb_x() 是相对轨道左缘的绝对偏移）。
            "slider_solid_thumb_x" => F(g.slider_solid.thumb_x() - g.slider_solid.fill_width()),
            "slider_solid_thumb_sx" => F(g.slider_solid.thumb_scale_x()),
            "slider_solid_thumb_sy" => F(g.slider_solid.thumb_scale_y()),
            "slider_solid_thumb_bg" => solid_thumb_bg_value(),
            "slider_solid_value" => S(slider_value_text(g.slider_solid.value())),
            "slider_glass_fill_w" => F(g.slider_glass.fill_width()),
            "slider_glass_thumb_x" => F(g.slider_glass.thumb_x() - g.slider_glass.fill_width()),
            "slider_glass_thumb_sx" => F(g.slider_glass.thumb_scale_x()),
            "slider_glass_thumb_sy" => F(g.slider_glass.thumb_scale_y()),
            "slider_glass_thumb_bg" => slider_thumb_bg_value(&g.slider_glass),
            "slider_glass_thumb_filter" => slider_thumb_filter_value(&g.slider_glass),
            "slider_glass_thumb_decoration" => slider_thumb_decoration_value(&g.slider_glass),
            "slider_glass_value" => S(slider_value_text(g.slider_glass.value())),
            // ---- liquid button / dropdown（LiquidButton.kt 1:1，实色/玻璃对比）----
            "lbtn_s_sx" => F(g.liquid_btn_solid.scale().0),
            "lbtn_s_sy" => F(g.liquid_btn_solid.scale().1),
            "lbtn_s_tx" => F(g.liquid_btn_solid.translation().0),
            "lbtn_s_ty" => F(g.liquid_btn_solid.translation().1),
            "lbtn_g_sx" => F(g.liquid_btn_glass.scale().0),
            "lbtn_g_sy" => F(g.liquid_btn_glass.scale().1),
            "lbtn_g_tx" => F(g.liquid_btn_glass.translation().0),
            "lbtn_g_ty" => F(g.liquid_btn_glass.translation().1),
            "lbtn_g_bg" => zypui::Val::Raw(zephyra_core::property::PropertyValue::color(
                zephyra_core::core::Color::rgba(1.0, 1.0, 1.0, 0.18),
            )),
            "lbtn_g_filter" => zypui::Val::Raw(zephyra_ui::liquid_button::glass_filter_value()),
            "lbtn_g_decoration" => {
                zypui::Val::Raw(zephyra_ui::liquid_button::glass_decoration_value(
                    g.liquid_btn_glass.press_progress(),
                ))
            }
            "ldd_s_sx" => F(g.liquid_dd_solid.scale().0),
            "ldd_s_sy" => F(g.liquid_dd_solid.scale().1),
            "ldd_s_tx" => F(g.liquid_dd_solid.translation().0),
            "ldd_s_ty" => F(g.liquid_dd_solid.translation().1),
            "ldd_s_open" => B(g.liquid_dd_solid_open),
            "ldd_s_visible" => B(g.liquid_dd_solid_anim > 0.001),
            // 菜单 morph 的 draw-time scale/translate **刻意不用**：`paint` 的文字路径
            // 不吃 `scale-x/scale-y`，缩放容器会让文字溢出/错位（关闭动画最明显）。
            // 形态切换改由 `morph-menu`（绝对菜单矩形）+ `show_items` 驱动；连续形变
            // 需要文字也参与缩放或改走布局动画，当前引擎不具备。
            "ldd_s_menu_sx" => F(1.0),
            "ldd_s_menu_sy" => F(1.0),
            "ldd_s_menu_tx" => F(0.0),
            "ldd_s_menu_ty" => F(0.0),
            // 开：过阈值再揭示（避免未 morph 完文字溢出）；关：衰减期保持可见，
            // 避免全尺寸菜单壳子先空出来再消失（配合无 scale 变换的形态切换）。
            "ldd_s_reveal" => B(if g.liquid_dd_solid_open {
                g.liquid_dd_solid_anim > 0.85
            } else {
                g.liquid_dd_solid_anim > 0.001
            }),
            "ldd_s_entries" => L(vec![
                rec(vec![
                    ("key", S("liquid:dd:solid:opt1")),
                    ("label", S(g.tr("ui.option.alpha"))),
                ]),
                rec(vec![
                    ("key", S("liquid:dd:solid:opt2")),
                    ("label", S(g.tr("ui.option.beta"))),
                ]),
                rec(vec![
                    ("key", S("liquid:dd:solid:opt3")),
                    ("label", S(g.tr("ui.option.gamma"))),
                ]),
            ]),
            "ldd_g_sx" => F(g.liquid_dd_glass.scale().0),
            "ldd_g_sy" => F(g.liquid_dd_glass.scale().1),
            "ldd_g_tx" => F(g.liquid_dd_glass.translation().0),
            "ldd_g_ty" => F(g.liquid_dd_glass.translation().1),
            "ldd_g_open" => B(g.liquid_dd_glass_open),
            "ldd_g_visible" => B(g.liquid_dd_glass_anim > 0.001),
            "ldd_g_menu_sx" => F(1.0),
            "ldd_g_menu_sy" => F(1.0),
            "ldd_g_menu_tx" => F(0.0),
            "ldd_g_menu_ty" => F(0.0),
            "ldd_g_reveal" => B(if g.liquid_dd_glass_open {
                g.liquid_dd_glass_anim > 0.85
            } else {
                g.liquid_dd_glass_anim > 0.001
            }),
            "ldd_g_entries" => L(vec![
                rec(vec![
                    ("key", S("liquid:dd:glass:opt1")),
                    ("label", S(g.tr("ui.option.alpha"))),
                ]),
                rec(vec![
                    ("key", S("liquid:dd:glass:opt2")),
                    ("label", S(g.tr("ui.option.beta"))),
                ]),
                rec(vec![
                    ("key", S("liquid:dd:glass:opt3")),
                    ("label", S(g.tr("ui.option.gamma"))),
                ]),
            ]),
            "ldd_g_bg" => {
                let a = if g.liquid_dd_glass_open { 0.30 } else { 0.18 };
                zypui::Val::Raw(zephyra_core::property::PropertyValue::color(
                    zephyra_core::core::Color::rgba(1.0, 1.0, 1.0, a),
                ))
            }
            "ldd_g_filter" => zypui::Val::Raw(zephyra_ui::liquid_button::glass_filter_value()),
            "ldd_g_decoration" => {
                zypui::Val::Raw(zephyra_ui::liquid_button::glass_decoration_value(
                    g.liquid_dd_glass.press_progress(),
                ))
            }
            "glass_decorations" => zypui::Val::Raw(
                zephyra_core::property::PropertyValue::decorations(if g.glass_decor.is_checked() {
                    zephyra_core::decoration::Decorations::glass()
                } else {
                    zephyra_core::decoration::Decorations::none()
                }),
            ),
            _ => return None,
        })
    }

    fn tr(&self, key: &'static str) -> &'static str {
        Gallery::tr(self, key)
    }

    fn hovers(&self, key: &str) -> bool {
        Gallery::hovers(self, key)
    }

    fn holds(&self, key: &str) -> bool {
        Gallery::holds(self, key)
    }

    fn is_focused(&self, key: &str) -> bool {
        Gallery::is_focused(self, key)
    }
}
