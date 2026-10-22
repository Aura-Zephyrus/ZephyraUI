//! 液态玻璃：折射（`Filter::Refraction`）+ 参数实验台。
//!
//! # 为什么提升为一等路由
//!
//! 液态玻璃是 core 滤镜能力最有说服力的展示：折射参数**运行期可调**，
//! 点 `[−]/[+]` 就能看到位移实时变化。藏在 `reference` 页的第 8 族时要点两下
//! 才看得到，而它的四个步进器正是本页最不该藏起来的东西。
//!
//! # 这一页要展示的是「透过玻璃看到后面是什么」
//!
//! 所以玻璃块**必须压在有内容的地方**。放一块纯色底板上的折射等于什么都不显示
//! —— 折射采样的是**背景**，背景没东西就看不出任何位移。
//!
//! 底下铺的那几行字不是装饰：它们是**折射的输入**。删掉它们，这一页
//! 就变成"三个灰色方块"，而那看起来像折射没实现。
//!
//! # 参数是**运行期可调**的，不是三档静态值
//!
//! 玻璃块上的 `filter` 由 `gallery::app::glass::Glass` 逐帧算出（见其
//! `filter` 方法），面板上的 `[−] [+]` 直接改它。所以这一页同时是**说明**与**实验台**：
//! 把厚度调到 0 会看到玻璃变成纯透镜板，把 IOR 调到 1.00 会看到位移完全消失
//! —— 后者是最直接的「公式被改坏了吗」诊断。
//!
//! # 后端支持
//!
//! 折射在 **D3D11 / D3D12** 上实现；Vulkan 的滤镜管线尚未接通（那边
//! `fs_blur_h` / `fs_composite` 还没被调用），所以在 Linux 上这一页
//! 看起来与"没设滤镜"相同。`--dump-cpu` 那条路也不画滤镜。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_ui::controls;

use crate::app::glass::{Param, PANE_THICKNESS_OFFSETS};
use crate::app::pages;
use crate::app::Gallery;

/// 玻璃**后面**铺的内容 —— **折射的输入**，不是装饰。
///
/// ⚠ 这是函数而不是 `const`：三行走 [`Gallery::tr`]，而 `g` 只在运行期有。
fn backdrop_text(g: &Gallery) -> [&'static str; 3] {
    [
        g.tr("ui.the-quick-brown-fox-jumps-over-the-lazy-dog"),
        g.tr("ui.refraction-samples-what-is-already-on-screen"),
        g.tr("ui.remove-these-lines-and-the-glass-looks-broken"),
    ]
}

/// 三块玻璃的 key（纯身份，不参与路由；标题是文案，走 [`Gallery::tr`]）。
const PANE_KEYS: &[&str] = &["glass:a", "glass:b", "glass:c"];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "glass",
        "page-glass",
        vec![
            controls::text_label(
                "glass-blurb",
                g.tr("ui.liquid-glass-refraction-parameters-are-runtime-adjustable"),
                "text-body",
            ),
            pages::section(
                "sec-params",
                g.tr("ui.refraction-parameters"),
                vec![params(g), stage(g)],
            ),
            // ⚠ 这里**曾经**还有一节 `sec-filters`，内容是「两个进度条」。
            //   那是旧「效果页」的遗留（那时这一页叫 effects、演示的是各种效果），
            //   页面改成「液态玻璃」之后它没被删掉 —— 于是在液态玻璃页上出现
            //   一节标题写着「Filters」、内容却是进度条的演示。标题与内容对不上，
            //   而两样都与本页无关。
        ],
    )
}

/// 参数面板：四组 `[−] 名称 值 [+]` 横排。
///
/// # 为什么一行放四组而不是四行
///
/// 内容卡（`shell::CONTENT_H` 756）减去页头与标题只剩 400 出头，而舞台本身
/// 就占 255。参数面板只能占一行 —— 而一行四组在 1139 宽下绰绰有余（每组约
/// 240px），竖排则要 160px，直接把舞台挤出客户区。
///
/// # 为什么用 `[−] [+] 而不是滑条
///
/// core 还没把滑条做成**输入设备**（`Gallery::click` 里它仍属「展示型」，
/// 见 `docs/dev-log/README.md`）。用滑条做这个面板会得到一个**看得见摸不动**
/// 的控件 —— 而那正是本页最不该出现的东西：整个页面就是讲「参数可调」的。
fn params(g: &Gallery) -> Element {
    let (t_text, b_text, d_text, i_text) = g.glass.texts();
    let values = [t_text, b_text, d_text, i_text];

    pages::page_root(
        "glass-params",
        "glass-params",
        Param::ALL
            .iter()
            .enumerate()
            .map(|(i, p)| stepper(g, *p, values[i]))
            .collect(),
    )
}

/// 一组步进器。
fn stepper(g: &Gallery, param: Param, value: &'static str) -> Element {
    pages::page_root(
        "cell",
        "glass-param",
        vec![
            controls::text_label(param.label(), param.label(), "glass-param-label"),
            controls::round_button(
                dec_key(param),
                "-",
                None,
                false,
                g.hovers(dec_key(param)),
                g.holds(dec_key(param)),
            ),
            controls::text_label("v", value, "glass-param-value"),
            controls::round_button(
                inc_key(param),
                "+",
                None,
                false,
                g.hovers(inc_key(param)),
                g.holds(inc_key(param)),
            ),
        ],
    )
}

/// 折射的**舞台**：一块深色底板 + 压在它上面的三块玻璃。
///
/// # 为什么玻璃块要靠 `translate-y` 压上去，而不是当兄弟节点摆
///
/// 折射采样的是「**已经画在屏幕上**的东西」，所以玻璃块必须与它要折射的图案
/// **在几何上重叠**。而 core 没有浮层、没有负 margin、没有绝对定位（属性表里
/// 只有 `translate-x/y`，见 `zstylec/src/props_map.rs`）—— 于是重叠只有一条路：
/// 负 `translate-y`。
///
/// ⚠ 这是**有意为之的脆弱**：`.glass-panes` 的位移量必须与目标条带的高度
/// 对得上，否则玻璃块会滑出条带 —— 症状是「折射不生效」，而真正的原因是
/// **玻璃块下面没有内容可折射**。改条带高度时记得一起改样式表；
/// 门禁 `glass_panes_cover_the_refraction_target` 会替你盯着这件事。
///
/// # 为什么底板要挑这种深蓝而不是深灰
///
/// 折射位移在**色差大**的地方才看得见。深灰底上的深色玻璃，位移 20 像素与
/// 位移 0 像素画出来几乎一样 —— 于是「折射没生效」与「折射生效但看不出来」
/// 没法区分，而后者会被当成前者去查着色器。深蓝 + 白竖条把这个问题挡在门外。
fn stage(g: &Gallery) -> Element {
    pages::page_root(
        "glass-stage",
        "glass-stage",
        vec![backdrop(g), target(), panes(g)],
    )
}

/// 三块玻璃的标题文案（走 [`Gallery::tr`]，三语）。
/// 与 [`panes`] 分开是因为 titles 需要 `g.tr()` 而 key 不需要。
fn pane_titles(g: &Gallery) -> Vec<&'static str> {
    vec![g.tr("ui.thin"), g.tr("ui.normal"), g.tr("ui.thick")]
}

/// 玻璃**背后**的内容。
///
/// ⚠ 这些行不是装饰，它们是折射要采样的东西。删掉之后玻璃块与纯色底板背景
/// 之间没有任何差别，于是「折射没生效」看起来像 bug，而真正的原因是
/// **没有可供折射的内容**。
fn backdrop(g: &Gallery) -> Element {
    pages::page_root(
        "glass-backdrop",
        "glass-backdrop",
        backdrop_text(g)
            .iter()
            .enumerate()
            .map(|(i, l)| controls::text_label(backdrop_key(i), l, "glass-bg-text"))
            .collect(),
    )
}

/// 折射的**主输入**：一排贯穿整个舞台的竖条。
///
/// # 为什么需要它（而不能只靠那三行字）
///
/// 折射采样的是「**已经画在屏幕上**的东西」，所以玻璃块下面必须有内容。
/// 早先这一页只有三行说明文字，而文字**只有 312px 宽**（三块玻璃加起来
/// 占 692px）—— 于是：
///
/// | 玻璃块 | 横向范围 | 下面有字吗 |
/// |---|---|---|
/// | `glass:a` | 308..528 | 有（整块都在字上） |
/// | `glass:b` | 544..764 | 只有 1/3（字到 620 就没了） |
/// | `glass:c` | 780..1000 | **完全没有** |
///
/// 第三块玻璃下面是**一整块纯色底板** —— 位移纯色等于没位移，于是那一块
/// 看上去「折射没生效」。而它其实是**没有可折射的东西**。
///
/// 竖条比文字更适合当折射输入：位移是十几像素量级，15px 的细笔画弯起来
/// 看不清，而 27px 宽的竖条边缘一弯就看得见 —— 这正是液态玻璃该有的样子。
///
/// # 尺寸为什么是 27px × 20 根
///
/// 舞台内容宽 692px。`27 × 20 + 8 × 19 = 540 + 152 = 692` —— **正好铺满**，
/// 不用留残边。（竖条的宽度是解出来的：`(692 + 8) / (n + 1) - 8` 对某个整数
/// n 是整数，取 n = 20 时 27px。）
fn target() -> Element {
    pages::page_root(
        "glass-target",
        "glass-target",
        (0..TARGET_BARS)
            .map(|i| {
                let bar = Element::new(NodeTag::Container)
                    .with_key(target_key(i))
                    .style("glass-bar");
                // ⚠ 第二个分类名必须用 `styles()` **分开加**：`style()` 只收**一个**
                // 词，不按空白切分。写成 `style("glass-bar alt")` 会得到一个
                // 名字里带空格的分类名 —— 它匹配不上任何规则，于是那根竖条
                // **宽度为 0**（`width: 27px` 那条规则没选中它）。
                // 症状是「隔一根竖条就消失一根」，而且不报错。
                if i % 2 == 0 {
                    bar
                } else {
                    bar.styles(["alt"])
                }
            })
            .collect(),
    )
}

/// 竖条的数量 —— 与 [`target_key`] 的下标范围一致。
const TARGET_BARS: usize = 20;

/// 竖条的 key（**不能 `format!`**，见模块文档的每帧泄漏那条）。
const fn target_key(i: usize) -> &'static str {
    match i {
        0 => "gx:0",
        1 => "gx:1",
        2 => "gx:2",
        3 => "gx:3",
        4 => "gx:4",
        5 => "gx:5",
        6 => "gx:6",
        7 => "gx:7",
        8 => "gx:8",
        9 => "gx:9",
        10 => "gx:10",
        11 => "gx:11",
        12 => "gx:12",
        13 => "gx:13",
        14 => "gx:14",
        15 => "gx:15",
        16 => "gx:16",
        17 => "gx:17",
        18 => "gx:18",
        _ => "gx:19",
    }
}

/// 三块玻璃并排。
///
/// # `filter` 写在这里、而不是样式表 —— 见模块文档
///
/// 值来自 [`Gallery::glass`]，每帧重算。它是**逐帧**的（`view()` 每帧重建整棵
/// 树），但查表是零成本的：四次数组下标，没有分配、没有浮点运算。
///
/// # 三块的厚度**不同**（档位偏移）
///
/// 标题写着「薄 / 常规 / 厚」，所以三块必须真的厚薄不同 —— 否则标题在**说谎**。
/// 偏移量见 [`PANE_THICKNESS_OFFSETS`]；其余三个参数三块**共用**（拖一次面板，
/// 三块一起变）。
///
/// # 标签放在玻璃**下面**而不是上面
///
/// 这是为了让 `.glass-panes` 的 `translate-y` 恰好等于**目标条带的高度**
/// （一个整数），而不是「目标高度 + 标题高度 + 间距」—— 后者里的标题高度
/// 取决于**字体度量**，换个字体那个数就错位了，而错位的症状是「玻璃块没有
/// 压在竖条上」（画面上等于没有折射）。
fn panes(g: &Gallery) -> Element {
    // ⚠ 每块玻璃**各自重建**链，而不是建一次后复用：`FilterChain` 不是 `Copy`，
    // 而 `map` 的闭包是 `FnMut` —— 捕获一个要按值取用的变量会编译不过。
    // 重建的代价是四次数组下标，零分配。
    // 标题走 `pane_titles(g)`（三语）；key 走 `PANE_KEYS`（静态身份）。
    // 两者按下标对齐 —— 改块数时同步改 [`PANE_KEYS`] 与 [`pane_titles`]。
    let titles = pane_titles(g);
    pages::page_root(
        "glass-panes",
        "glass-panes",
        PANE_KEYS
            .iter()
            .zip(titles.iter())
            .enumerate()
            .map(|(i, (&key, title))| {
                pages::page_root(
                    "cell",
                    "glass-cell",
                    vec![
                        Element::new(NodeTag::Container)
                            .with_key(key)
                            .style("glass-pane")
                            .with(
                                &props::FILTER,
                                PropertyValue::filter(zephyra_core::filter::FilterChain::single(
                                    g.glass
                                        .filter_with_thickness_offset(PANE_THICKNESS_OFFSETS[i]),
                                )),
                            ),
                        controls::text_label(pane_title_key(key), title, "glass-title"),
                    ],
                )
            })
            .collect(),
    )
}

/// 减号按钮的 key（**不能 `format!`**，见模块文档的每帧泄漏那条）。
const fn dec_key(p: Param) -> &'static str {
    match p {
        Param::Thickness => "glass-thickness:dec",
        Param::Bezel => "glass-bezel:dec",
        Param::Dispersion => "glass-dispersion:dec",
        Param::Ior => "glass-ior:dec",
    }
}

/// 加号按钮的 key。
const fn inc_key(p: Param) -> &'static str {
    match p {
        Param::Thickness => "glass-thickness:inc",
        Param::Bezel => "glass-bezel:inc",
        Param::Dispersion => "glass-dispersion:inc",
        Param::Ior => "glass-ior:inc",
    }
}

/// 背景文字的 key。
const fn backdrop_key(i: usize) -> &'static str {
    match i {
        0 => "gb:0",
        1 => "gb:1",
        _ => "gb:2",
    }
}

/// 玻璃块标题的 key。
///
/// ⚠ 不是 `const fn`：`&str` 上的 `match` 在常量上下文里还不允许（E0658）。
/// 它每帧只调三次，运行期比较的开销可忽略 —— 而省下的是一次 `format!`。
fn pane_title_key(key: &str) -> &'static str {
    match key {
        "glass:a" => "gt:a",
        "glass:b" => "gt:b",
        _ => "gt:c",
    }
}
