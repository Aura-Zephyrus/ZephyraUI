//! demo 绑定的数据面：`ui/demo.zypui` 经 `@绑定` 引用的**全部**数据表。
//!
//! `Demo::binding` 的匹配表是「模板引用了哪些名字」的唯一清单；这些名字
//! 背后的数据表集中在这里 —— 页面（模板）与数据共享同一份 id / 文本清单。
//!
//! # 零分配纪律不变
//!
//! 全部表是 `&'static` 常量；每帧构造 `Val` 的 Rec/L 是 `Vec` 分配，但这与
//! Rust 版每帧 `collect()` 的形状相同（树本身每帧重建），不是新增泄漏。
//! 滤镜链 / 混合模式同理：Rust 版 `effects()` 也是每帧构造。

use zephyra_core::core::{Color, Point};
use zephyra_core::filter::{BlendMode, Filter, FilterChain};
use zephyra_core::property::PropertyValue;
use zypui::Val;

use super::{BUTTONS, ICON_NAMES};

/// s：构造绑定值的小助手。
fn s(v: &'static str) -> Val {
    Val::S(v)
}

/// 工具栏按钮：`name` 即元素 key 与图标名；`compact` = 名字长用小一档字号
/// （判断「名字长不长」是数据的，用哪一档是样式表的 `Text.label.compact`）。
pub(crate) fn buttons() -> Val {
    Val::L(
        BUTTONS
            .iter()
            .map(|name| Val::Rec(vec![("name", s(name)), ("compact", Val::B(name.len() > 6))]))
            .collect(),
    )
}

/// 演示纹理名（注册时用同一个名字，属性值与宿主的注册名对上）。
pub(crate) fn icons() -> Val {
    Val::L(ICON_NAMES.iter().map(|n| s(n)).collect())
}

/// 内容区的文本行。真实的 UTF-8 文本 —— 顺带验证非 ASCII 路径（`TextEngine`
/// 按 `char` 而非字节处理，中文与标点不该算错字数）。
const LINE_TEXTS: [&str; 4] = [
    "ZephyraUI 核心",
    "DisplayList 是唯一契约",
    "换主题不需要遍历控件树",
    "Steady state = zero rebuild",
];

/// 与 [`LINE_TEXTS`] 一一对应的 key（`ElementKey` 只接受编译期常量或 `u64`）。
const LINE_KEYS: [&str; 4] = ["line0", "line1", "line2", "line3"];

pub(crate) fn lines() -> Val {
    Val::L(
        LINE_KEYS
            .iter()
            .zip(LINE_TEXTS)
            .map(|(k, t)| Val::Rec(vec![("key", s(k)), ("text", s(t))]))
            .collect(),
    )
}

/// 演示用的裁剪区域（`x, y, w, h`，归一化）。
///
/// 依次是：整张、右半、左下四分之一、以及一个**越界**区域。
/// 最后一项是故意的 —— 核心应把它退化成整张，画面看起来与第一项相同。
const SRC_CROPS: [(f32, f32, f32, f32); 4] = [
    (0.0, 0.0, 1.0, 1.0),
    (0.5, 0.0, 0.5, 1.0),
    (0.5, 0.5, 0.5, 0.5),
    (0.9, 0.0, 0.5, 1.0),
];

/// 与 [`SRC_CROPS`] 一一对应的 key。长度不同则是编译错误，不会静默错位。
const SRC_CROP_KEYS: [&str; 4] = ["crop:full", "crop:right", "crop:br", "crop:overflow"];

/// uv 以四元数列表给 —— `uv` 属性分支收 `Val::L`（见 `zypui::instantiate::uv_of`）。
pub(crate) fn crops() -> Val {
    Val::L(
        SRC_CROPS
            .iter()
            .zip(SRC_CROP_KEYS)
            .map(|(uv, key)| {
                Val::Rec(vec![
                    ("key", s(key)),
                    (
                        "uv",
                        Val::L(vec![Val::F(uv.0), Val::F(uv.1), Val::F(uv.2), Val::F(uv.3)]),
                    ),
                ])
            })
            .collect(),
    )
}

/// E4 的效果展台：模糊 / 投影 / 多级链，外加两个非默认混合模式。
///
/// 三件事各自都能被肉眼分辨，但**必须有对照**才不会误判：
///
/// - 模糊 —— 看边缘有没有过渡带；
/// - 投影 —— 看**偏移方向**上有没有第二个更暗的形状；
/// - 多级链 —— 三趟同参数的高斯 ≈ 一趟 σ=2.83，过渡带**宽度**不同。
///
/// 最后两个样本不带滤镜、只带混合模式：「混合模式生效」与「离屏 pass 生效」
/// 互不干扰 —— 否则一个模式下错，会被另一个的偏移掩盖。
pub(crate) fn effects() -> Val {
    // mk：一个样本的记录。滤镜链与混合模式是**结构值**（`Val::Raw` 透传）——
    // 表达式语言没有它们的字面量，宿主绑定是唯一来源。
    let mk = |id: &'static str,
              caption: &'static str,
              fill: &'static str,
              chain: FilterChain,
              blend: BlendMode| {
        Val::Rec(vec![
            ("id", s(id)),
            ("caption", s(caption)),
            ("fill", s(fill)),
            ("filter", Val::Raw(PropertyValue::filter(chain))),
            ("blend", Val::Raw(PropertyValue::blend_mode(blend))),
        ])
    };

    // 三趟同参数的高斯 ≈ 一趟 σ=2.83，所以过渡带明显更宽 —— 这就是判据。
    let triple = FilterChain::new([
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
    ]);

    Val::L(vec![
        mk(
            "fx-none",
            "对照",
            "accent",
            FilterChain::none(),
            BlendMode::SrcOver,
        ),
        mk(
            "fx-blur",
            "模糊 σ3",
            "accent",
            FilterChain::single(Filter::Blur {
                sigma_x: 3.0,
                sigma_y: 3.0,
            }),
            BlendMode::SrcOver,
        ),
        mk(
            "fx-shadow",
            "投影 σ3",
            "accent",
            FilterChain::single(Filter::DropShadow {
                color: Color::rgba(0.0, 0.0, 0.0, 0.85),
                offset: Point::new(6.0, 6.0),
                sigma: 3.0,
            }),
            BlendMode::SrcOver,
        ),
        mk(
            "fx-triple",
            "三级模糊",
            "accent",
            triple,
            BlendMode::SrcOver,
        ),
        mk(
            "fx-multiply",
            "Multiply",
            "warm",
            FilterChain::none(),
            BlendMode::Multiply,
        ),
        mk(
            "fx-screen",
            "Screen",
            "warm",
            FilterChain::none(),
            BlendMode::Screen,
        ),
    ])
}
