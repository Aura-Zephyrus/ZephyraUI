//! 宿主绑定的数据面：`.zypui` 模板经 `@绑定` 引用的**全部**数据表与查表函数。位于 `frontend/`。
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

// ---- 选择（toggles 族） ----

// ---- 集合（collections 族） ----

// ---- 菜单（menus 族） ----

// ---- 对话框（dialogs 族） ----

// ---- 分页（paging 族） ----

// ---- 状态（status 族） ----

// ---- 导航（navigation 族） ----

// ---- 外壳（shell） ----

/// 窗口按钮（点击 no-op，hover/pressed 视觉是真的；close 加 `.close`）。
pub(crate) const CAPTION_BUTTONS: &[(&str, &str, bool)] = &[
    ("win:min", "minimize", false),
    ("win:max", "move", false),
    ("win:close", "close", true),
];

/// 语言段（testing 页的切换钮 —— key 带 `lang:` 前缀，`click` 两种形态都认）。
pub(crate) fn langs() -> Val {
    Val::L(vec![
        item("lang:en", "English"),
        item("lang:zh-Hans", "中文"),
        item("lang:ja", "日本語"),
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
        "buttons" => "ref:buttons",
        "checkbox" => "ref:checkbox",
        _ => "ref:unknown",
    }
}

/// 族的显示名（与 `REF_FAMILIES` 一一对应）。
pub(crate) fn family_label(id: &str) -> &'static str {
    match id {
        "buttons" => "Buttons",
        "checkbox" => "Checkbox",
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
