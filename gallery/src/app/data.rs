//! 宿主绑定的数据面：`.zypui` 模板经 `@绑定` 引用的**全部**数据表与查表函数。
//!
//! # 为什么集中在这一处
//!
//! `Gallery::binding` 的大匹配表是「模板引用了哪些名字」的唯一清单；
//! 而这些名字背后的数据表（菜单条目、日历、语言段……）原来散在各页面
//! 模块里。集中到这里之后，绑定的数据只有一份真相 —— `click` 的分派
//! （`list:` / `dpc:` 查表）也引用这里的表，页面与分派共享同一份 id 清单。
//!
//! # 零分配纪律不变
//!
//! 全部表是 `&'static` 常量；数值文本全查表（不 `format!` —— 每帧泄漏）。
//! 每帧构造 `Val` 的 Rec/L 是 `Vec` 分配，但这与 Rust 版每帧 `collect()`
//! 的形状相同（树本身每帧重建），不是新增泄漏。

use zypui::Val;

/// s/rec/item：构造绑定值的小助手。
pub(crate) fn s(v: &'static str) -> Val {
    Val::S(v)
}

pub(crate) fn rec(fields: Vec<(&'static str, Val)>) -> Val {
    Val::Rec(fields)
}

pub(crate) fn item(id: &'static str, label: &'static str) -> Val {
    rec(vec![("id", s(id)), ("label", s(label))])
}

// ---- 基础输入（basics 族） ----

/// ComboBox 演示选项（与 `Gallery::click` 的推进表同源，见 `COMBO_OPTIONS`）。
pub(crate) const COMBO_OPTIONS: &[&str] = &["Mica", "Acrylic", "Tabbed", "None"];

/// ComboBox 当前值的文字（越界回落第一项 —— `combo` 是公开字段）。
pub(crate) fn combo_text(i: usize) -> &'static str {
    COMBO_OPTIONS.get(i).copied().unwrap_or("Mica")
}

/// state 章ComboBox 的当前值文字（教学页自己的表，One/Two/Three ——
/// 与 basics 页的材质名表是两回事）。
pub(crate) fn demo_combo_text(i: usize) -> &'static str {
    match i {
        0 => "One",
        1 => "Two",
        _ => "Three",
    }
}

/// SpinBox 的显示文本（值域 0..=20 与 `click` 的边界同步）。
pub(crate) fn spin_text(n: i32) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        10 => "10",
        11 => "11",
        12 => "12",
        13 => "13",
        14 => "14",
        15 => "15",
        16 => "16",
        17 => "17",
        18 => "18",
        19 => "19",
        _ => "20",
    }
}

/// 滑条右侧的数值文本（basics 与 settings 共用 —— 两个页面的函数体一致）。
pub(crate) fn slider_text(v: f32) -> &'static str {
    if v < 0.2 {
        "0"
    } else if v < 0.4 {
        "25"
    } else if v < 0.6 {
        "50"
    } else if v < 0.8 {
        "75"
    } else {
        "100"
    }
}

/// 文本域的三行（文案走 CATALOG）。
pub(crate) fn area_lines(g: &super::Gallery) -> Val {
    Val::L(vec![
        s(g.tr("ui.multi-line-notes")),
        s(g.tr("ui.second-line")),
        s(g.tr("ui.third-line")),
    ])
}

// ---- 选择（toggles 族） ----

/// 分段控件的项（id 与 `Gallery::click` 的三条 `seg:*` 分支一一对应）。
pub(crate) const SEGS: &[(&str, &str)] = &[
    ("seg:grid", "Grid"),
    ("seg:list", "List"),
    ("seg:card", "Card"),
];

/// 单选组的选项（id 与 `click` 的 `radio:*` 分支对应）。
pub(crate) const RADIO_ITEMS: &[(&str, &str)] = &[
    ("radio:a", "Option A"),
    ("radio:b", "Option B"),
    ("radio:c", "Option C"),
];

// ---- 集合（collections 族） ----

/// ListView 的数据（`click` 的 `list:` 分支按 id 查回下标）。
pub(crate) const LIST_ITEMS: &[(&str, &str)] = &[
    ("list:a", "Acrylic"),
    ("list:b", "Mica"),
    ("list:c", "Solid"),
];

/// TableView 的表头与行。数据行不翻译（内容数据，与 COMBO_OPTIONS 同处理）。
pub(crate) const TABLE_ROWS: &[&[&str]] = &[
    &["Mica", "Low", "Win11"],
    &["Acrylic", "High", "Win10+"],
    &["Solid", "None", "All"],
];

/// 选中数据格的内容（下标 = 行 ×4 + 列，与 `click` 的 `td:` 分支同步）。
pub(crate) fn table_cell_text(i: usize) -> &'static str {
    const FLAT: [&str; 9] = [
        "Mica / Low",
        "Low",
        "Win11",
        "Acrylic / High",
        "High",
        "Win10+",
        "Solid / None",
        "None",
        "All",
    ];
    FLAT.get(i).copied().unwrap_or("Mica / Low")
}

// ---- 菜单（menus 族） ----

/// 菜单栏的三项。
pub(crate) const BAR_ITEMS: &[&str] = &["File", "Edit", "View"];

/// 每个菜单里的条目（`label` 为空 = 一条分隔线）。
pub(crate) const MENU_ENTRIES: &[&[(&str, &str)]] = &[
    &[
        ("New", "m:new"),
        ("", ""),
        ("Open…", "m:open"),
        ("Save", "m:save"),
    ],
    &[
        ("Undo", "m:undo"),
        ("Redo", "m:redo"),
        ("", ""),
        ("Cut", "m:cut"),
    ],
    &[
        ("Zoom In", "m:zoom-in"),
        ("Zoom Out", "m:zoom-out"),
        ("", ""),
        ("Reset", "m:reset"),
    ],
];

/// 工具栏图标按钮（key 用 `act:` 前缀，动作名写进 `last_action`）。
pub(crate) const TOOLS: &[(&str, &str)] = &[
    ("act:tool-save", "doc"),
    ("act:tool-search", "search"),
    ("act:tool-gear", "settings"),
];

// ---- 对话框（dialogs 族） ----

/// TimePicker 的可见窗口。
pub(crate) const HOURS: &[(&str, &str)] = &[("tph:13", "13"), ("tph:14", "14"), ("tph:15", "15")];
pub(crate) const MINUTES: &[(&str, &str)] = &[("tpm:00", "00"), ("tpm:05", "05"), ("tpm:10", "10")];

/// DatePicker 的星期头（ASCII 单字母，各日历控件的通行写法）。
pub(crate) const WEEK_HEADS: &[&str] = &["S", "M", "T", "W", "T", "F", "S"];

/// DatePicker 的格子（7 列 × 3 周；`click` 的 `dpc:` 分支查这张表换算日期值）。
pub(crate) const CALENDAR: &[Option<u8>] = &[
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(7),
    Some(8),
    Some(9),
    Some(10),
    Some(11),
    Some(12),
    Some(13),
    Some(14),
    Some(15),
    Some(16),
    Some(17),
    Some(18),
    Some(19),
    Some(20),
    Some(21),
];

// ---- 分页（paging 族） ----

/// SwipeView 的页面（`(标题, 说明)`）。
pub(crate) const SWIPE: &[(&str, &str)] = &[
    ("First card", "Click a dot below to change pages."),
    (
        "Second card",
        "The core has no drag gesture; paging is by key.",
    ),
    ("Third card", "State lives in the app, not in the control."),
];

// ---- 状态（status 族） ----

/// Toast 的三条内容（severity 按下标 info / caution / success）。
pub(crate) const TOASTS: &[(&str, &str)] = &[
    ("Saved", "Your changes are on disk."),
    ("Offline", "No network connection."),
    ("Synced", "Everything is up to date."),
];
pub(crate) const TOAST_SEVERITIES: [&str; 3] = ["info", "caution", "success"];

// ---- 导航（navigation 族） ----

/// 一组 NavItem（第一个选中 —— 外观展示，点击不变是**有意的**）。
pub(crate) const NAV_DEMO: &[(&str, &str, &str)] = &[
    ("navdemo:home", "Home", "home"),
    ("navdemo:star", "Starred", "star"),
    ("navdemo:archive", "Archive", "doc"),
];

/// SelectorBar 的项（id 与 `SELDEMO_IDS` 对齐）。
pub(crate) const SELECTOR: &[(&str, &str)] = &[
    ("seldemo:day", "Day"),
    ("seldemo:week", "Week"),
    ("seldemo:month", "Month"),
];

// ---- 外壳（shell） ----

/// 窗口按钮（点击 no-op，hover/pressed 视觉是真的；close 加 `.close`）。
pub(crate) const CAPTION_BUTTONS: &[(&str, &str, bool)] = &[
    ("win:min", "minimize", false),
    ("win:max", "move", false),
    ("win:close", "close", true),
];

/// state 章的分段（`stseg:*`）。
pub(crate) fn demo_segs() -> Val {
    Val::L(
        super::DEMO_SEGS
            .iter()
            .map(|id| match *id {
                "stseg:a" => item(id, "A"),
                "stseg:b" => item(id, "B"),
                _ => item(id, "C"),
            })
            .collect(),
    )
}

/// 语言段（testing 页的切换钮 —— key 带 `lang:` 前缀，`click` 两种形态都认）。
pub(crate) fn langs() -> Val {
    Val::L(vec![
        item("lang:en", "English"),
        item("lang:zh-Hans", "中文"),
        item("lang:ja", "日本語"),
    ])
}

/// 语言段（设置页 langbar —— 段 id 是裸语言标签，`selected` 按标签比对）。
pub(crate) fn lang_segs() -> Val {
    Val::L(vec![
        item("en", "English"),
        item("zh-Hans", "中文"),
        item("ja", "日本語"),
    ])
}

/// 参考页的族段（key 带 `ref:` 前缀 —— `click` 按 `strip_prefix("ref:")` 解析）。
pub(crate) fn ref_segs() -> Val {
    Val::L(
        super::REF_FAMILIES
            .iter()
            .map(|fam| item(seg_key(fam), family_label(fam)))
            .collect(),
    )
}

/// 族的 segmented key（`ref:<族名>`）。
pub(crate) fn seg_key(fam: &str) -> &'static str {
    match fam {
        "basics" => "ref:basics",
        "toggles" => "ref:toggles",
        "layout" => "ref:layout",
        "collections" => "ref:collections",
        "menus" => "ref:menus",
        "navigation" => "ref:navigation",
        "status" => "ref:status",
        "dialogs" => "ref:dialogs",
        "paging" => "ref:paging",
        _ => "ref:unknown",
    }
}

/// 族的显示名（与 `REF_FAMILIES` 一一对应）。
pub(crate) fn family_label(id: &str) -> &'static str {
    match id {
        "basics" => "Basic Input",
        "toggles" => "Selection",
        "layout" => "Layout",
        "collections" => "Collections",
        "menus" => "Menus",
        "navigation" => "Navigation",
        "status" => "Status",
        "dialogs" => "Dialogs",
        "paging" => "Paging",
        _ => "Unknown",
    }
}

/// 断点三档的显示文本（layout 章）。
pub(crate) fn bp_text(g: &super::Gallery) -> &'static str {
    use zephyra_core::responsive::WidthClass;
    match g.width_class() {
        WidthClass::Small => g.tr("ui.small-nav-collapsed-to-icons"),
        WidthClass::Medium => g.tr("ui.medium-nav-shows-labels"),
        WidthClass::Large => g.tr("ui.large-full-layout"),
    }
}

/// 快速开始章的计数文本（查表）。
pub(crate) fn count_text(n: u32) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        _ => "5+",
    }
}

// ---- 液态玻璃（glass 路由） ----

/// 玻璃步进按钮的 key（`glass-<参数>:dec|inc`，与 `click` 的解析器对齐）。
pub(crate) fn glass_step_key(p: crate::app::glass::Param, inc: bool) -> &'static str {
    use crate::app::glass::Param::*;
    match (p, inc) {
        (Thickness, false) => "glass-thickness:dec",
        (Thickness, true) => "glass-thickness:inc",
        (Bezel, false) => "glass-bezel:dec",
        (Bezel, true) => "glass-bezel:inc",
        (Dispersion, false) => "glass-dispersion:dec",
        (Dispersion, true) => "glass-dispersion:inc",
        (Ior, false) => "glass-ior:dec",
        (Ior, true) => "glass-ior:inc",
    }
}

/// 三块玻璃的逐帧滤镜值（结构值经 `Val::Raw` 透传进模板的 `filter` 属性；
/// 厚度档位偏移见 `PANE_THICKNESS_OFFSETS` —— 三块厚薄真的不同）。
pub(crate) fn glass_filter_value(
    g: &super::Gallery,
    pane: usize,
) -> zephyra_core::property::PropertyValue {
    zephyra_core::property::PropertyValue::filter(zephyra_core::filter::FilterChain::single(
        g.glass
            .filter_with_thickness_offset(super::glass::PANE_THICKNESS_OFFSETS[pane]),
    ))
}
