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
/// 语言段（语言选择器的那几项）。
///
/// # 文案为什么**不进**文案表
///
/// 语言名用**各自语言写自己**（English / 中文 / 日本語）—— 它们是**自称**，
/// 不是翻译：放进表里要填 3×3 = 9 条两两相同的内容，而那 9 条迟早会被误改成
/// 「翻译」，而那正是错的。详见 `docs/developer/i18n.md` §5.2。
///
/// 代价是模板里也有同样三个字面量（`DemoRow` 的行首标签 + `PillButton` 的
/// 标签）⇒ 它们在 i18n 硬门禁的**豁免表**里（`gallery/tests/gates.rs` 的
/// `LITERAL_ALLOWLIST`）。
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
        "switches" => "ref:switches",
        _ => "ref:unknown",
    }
}

/// 族的显示名（与 `REF_FAMILIES` 一一对应）。
pub(crate) fn family_label(id: &str) -> &'static str {
    match id {
        "buttons" => "Buttons",
        "checkbox" => "Checkbox",
        "switches" => "Switches",
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

// ---- 开关旋钮（LiquidToggle 一比一复刻的逐帧绑定） ----

/// 轨道颜色 = `lerp(switch_track, switch_accent, fraction)`（原版
/// 把手白罩 = `White @ (1 − press)`（原版 `onDrawSurface { drawRect(White.copy(alpha = 1f - progress)) }`）。
///
/// 按住时白罩消失，让出玻璃把手的折射面。
pub(crate) fn switch_knob_bg_value(model: &zephyra_ui::ToggleSwitch) -> zypui::Val {
    use zephyra_core::core::Color;
    zypui::Val::Raw(zephyra_core::property::PropertyValue::color(Color::rgba(
        1.0,
        1.0,
        1.0,
        1.0 - model.press_progress(),
    )))
}

/// 实色（solid）变体的把手 = 纯白不透明、不随 press 变化 ——「零动画」。
///
/// 模板上 `background` 必须求值为结构值（见 zypui instantiate.rs：绑错类型直接
/// 报错），所以实色实例也要显式绑一个 Raw 颜色——就是"纯白"这一个常量。
pub(crate) fn solid_knob_bg_value() -> zypui::Val {
    use zephyra_core::core::Color;
    zypui::Val::Raw(zephyra_core::property::PropertyValue::color(Color::rgb(
        1.0, 1.0, 1.0,
    )))
}

/// 把手滤镜 = `[BackdropBlur(8dp · (1−press)), Lens(5dp · press, 10dp · press)]`
/// —— 原版 `effects` 的 `blur(8.dp * (1 - progress))` 与
/// `lens(5.dp * progress, 10.dp * progress, chromaticAberration = true)`。
///
/// `chromaticAberration = true` 由 [`Filter::Lens`] 固定实现（7 通道分色），
/// 不另给开关 —— 原版液态开关把这一个参数写死为 true。
pub(crate) fn switch_knob_filter_value(model: &zephyra_ui::ToggleSwitch) -> zypui::Val {
    use zephyra_core::filter::{Filter, FilterChain};
    let p = model.press_progress();
    zypui::Val::Raw(zephyra_core::property::PropertyValue::filter(
        FilterChain::new(vec![
            Filter::BackdropBlur {
                sigma: 8.0 * (1.0 - p),
            },
            Filter::Lens {
                refraction_height: 5.0 * p,
                refraction_amount: 10.0 * p,
            },
        ]),
    ))
}

/// 把手装饰 = `Decorations::control_with_progress(press)` —— 原版 `drawBackdrop`
/// 的 highlight / shadow / innerShadow 三件（`width/1.5` 高光、
/// `Shadow(4dp, Black@0.05)` 恒定、`InnerShadow(4dp · press, alpha = press)`）。
pub(crate) fn switch_knob_decoration_value(model: &zephyra_ui::ToggleSwitch) -> zypui::Val {
    zypui::Val::Raw(zephyra_core::property::PropertyValue::decorations(
        zephyra_core::decoration::Decorations::control_with_progress(model.press_progress()),
    ))
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

// ---- ����Ĵָ��LiquidSlider ԭ����֡�󶨣� ----

/// Ĵָ���� = `White @ (1 ? press)`��ԭ�� onDrawSurface����
pub(crate) fn slider_thumb_bg_value(model: &zephyra_ui::Slider) -> zypui::Val {
    use zephyra_core::core::Color;
    zypui::Val::Raw(zephyra_core::property::PropertyValue::color(Color::rgba(
        1.0,
        1.0,
        1.0,
        1.0 - model.press_progress(),
    )))
}

pub(crate) fn solid_thumb_bg_value() -> zypui::Val {
    solid_knob_bg_value()
}

/// Ĵָ�˾���ԭ�� `blur(8.dp*(1-p)) + lens(10.dp*p, 14.dp*p, chromaticAberration=true)`��
pub(crate) fn slider_thumb_filter_value(model: &zephyra_ui::Slider) -> zypui::Val {
    use zephyra_core::filter::{Filter, FilterChain};
    let p = model.press_progress();
    zypui::Val::Raw(zephyra_core::property::PropertyValue::filter(
        FilterChain::new(vec![
            Filter::BackdropBlur {
                sigma: 8.0 * (1.0 - p),
            },
            Filter::Lens {
                refraction_height: 10.0 * p,
                refraction_amount: 14.0 * p,
            },
        ]),
    ))
}

/// Ĵָװ�Σ��߹�/ͶӰ/����Ӱ�� press����ԭ�� highlight/shadow/innerShadow ͬ�Σ���
pub(crate) fn slider_thumb_decoration_value(model: &zephyra_ui::Slider) -> zypui::Val {
    zypui::Val::Raw(zephyra_core::property::PropertyValue::decorations(
        zephyra_core::decoration::Decorations::control_with_progress(model.press_progress()),
    ))
}

/// ����ֵ����ʾ�ı������ + ��Ƶй©���ף�������·�� format!����
pub(crate) fn slider_value_text(v: f32) -> &'static str {
    let n = v.round().clamp(0.0, 100.0) as i32;
    match n {
        0 => "0",
        10 => "10",
        20 => "20",
        25 => "25",
        30 => "30",
        40 => "40",
        50 => "50",
        60 => "60",
        70 => "70",
        75 => "75",
        80 => "80",
        90 => "90",
        100 => "100",
        _ => Box::leak(n.to_string().into_boxed_str()),
    }
}
