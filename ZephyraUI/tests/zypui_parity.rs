//! parity 验收：`.zypui` 组件模板 vs Rust 控件函数，同参数同状态，产出
//! 的元素树必须**全等**（tag / key / 分类名序列 / 内容属性 / 子树）。
//!
//! 这是控件层迁移的命门测试：`.zypui` 是 Rust 控件的逐字翻译，翻译错了
//! （类序、key、属性、条件词漏一个分支）在这里第一个红。
//!
//! # 怎么跑
//!
//! ```text
//! cargo test -p zephyra-ui --test zypui_parity
//! ```
//!
//! # 数据流
//!
//! - `.zypui` 侧：`ui/controls.zypui`（组件）+ `ui/parity.zypui`（每控件
//!   一个 page）走 **dev 路径**（zypuic 直接 parse 源文件）。
//! - Rust 侧：同参数调用 `controls::*` 构造函数。
//! - 输入态约定：实例 key 以 `:h` 结尾 = hovered、`:p` = pressed
//!   （两侧同用，key 本身进对照）。
//! - 动态参数走绑定（`@on` / `@v01` / `@seg_items` …），两侧同一数值。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_ui::controls;
use zypui::ir::RawUi;
use zypui::{Host, Interner, Ui, Val};

// ──────────────────────────── 对照基础设施 ────────────────────────────

/// 元素树的**全量**结构描述：tag、key、分类名序列（有序！）、内容属性
/// （按属性键名排序）、子树。任何一侧的差异都会显形。
fn describe(el: &Element) -> String {
    let mut s = format!(
        "({tag:?} key={key:?} classes={classes:?}",
        tag = el.tag(),
        key = el.key().map(zephyra_core::id::ElementKey::as_name),
        classes = el.style_names(),
    );
    let mut props: Vec<String> = el
        .props()
        .iter_local()
        .map(|(k, v)| prop_line(k, v))
        .collect();
    props.sort();
    s.push_str(&format!(" props=[{}]", props.join(", ")));
    for c in el.children() {
        s.push(' ');
        s.push_str(&describe(c));
    }
    s.push(')');
    s
}

fn prop_line(k: &'static zephyra_core::property::PropertyKey, v: &PropertyValue) -> String {
    let val = match v {
        PropertyValue::StaticStr(s) => format!("s({s:?})"),
        PropertyValue::Bool(b) => format!("b({b})"),
        PropertyValue::Float(f) => format!("f({f:.4})"),
        PropertyValue::UvRect(r) => format!("uv({},{},{},{})", r.x, r.y, r.width, r.height),
        other => format!("other({other:?})"),
    };
    format!("{}={val}", k_name(k))
}

/// 属性键的身份：`PropertyKey` 相等按地址，跨构造点只能按名字对账。
fn k_name(k: &zephyra_core::property::PropertyKey) -> String {
    // PropertyKey 没有名字访问器；Debug 里带名字 —— 用 Debug 文本。
    format!("{k:?}")
}

/// parity 页共用的宿主。输入态按 key 后缀（`:h` / `:p`），绑定表集中在
/// `Bindings`。
struct ParityHost;

const HOVER_SUFFIX: &str = ":h";
const PRESS_SUFFIX: &str = ":p";

impl Host for ParityHost {
    fn binding(&self, name: &str) -> Option<Val> {
        bindings(name)
    }

    fn tr(&self, key: &'static str) -> &'static str {
        // parity 页不用 tr（Rust 侧也是字面量参数）；返回键本身。
        key
    }

    fn hovers(&self, key: &str) -> bool {
        key.ends_with(HOVER_SUFFIX)
    }

    fn holds(&self, key: &str) -> bool {
        key.ends_with(PRESS_SUFFIX)
    }

    fn prop_key(&self, name: &str) -> Option<&'static zephyra_core::property::PropertyKey> {
        if name == "url" {
            Some(&zephyra_ui::controls::HYPERLINK_URL)
        } else {
            None
        }
    }
}

/// 绑定表：名字 → 值。与 parity.zypui 里的 `@…` 一一对应。
fn bindings(name: &str) -> Option<Val> {
    fn b(v: bool) -> Val {
        Val::B(v)
    }
    fn f(v: f32) -> Val {
        Val::F(v)
    }
    fn s(v: &'static str) -> Val {
        Val::S(v)
    }
    fn rec(fields: Vec<(&'static str, Val)>) -> Val {
        Val::Rec(fields)
    }
    fn item(id: &'static str, label: &'static str) -> Val {
        rec(vec![("id", s(id)), ("label", s(label))])
    }
    fn entry(key: &'static str, label: &'static str) -> Val {
        rec(vec![("key", s(key)), ("label", s(label))])
    }
    match name {
        "on" => Some(b(true)),
        "off" => Some(b(false)),
        "v01" => Some(f(0.42)),
        "v02" => Some(f(0.8)),
        "frame" => Some(f(1.0)),
        "pips" => Some(f(4.0)),
        "cur" => Some(f(2.0)),
        "sw_sel" => Some(f(9.0)),
        "ratio" => Some(f(0.5)),
        "sug_sel" => Some(f(1.0)),
        "dp_rows" => Some(f(5.0)),
        "dp_sel" => Some(f(15.0)),
        "menu_open" => Some(f(1.0)),
        "tp_hour" => Some(s("08:00")),
        "tp_minute" => Some(s("05")),
        "tf_val" => Some(s("输入内容")),
        "empty" => Some(s("")),
        "seg_sel" => Some(s("warm")),
        "sel_sel" => Some(s("b")),
        "list_sel" => Some(s("l2")),
        "sw_idx" => Some(f(1.0)),
        "seg_items" => Some(Val::L(vec![item("warm", "暖"), item("cool", "冷")])),
        "sel_items" => Some(Val::L(vec![item("a", "甲"), item("b", "乙")])),
        "list_items" => Some(Val::L(vec![item("l1", "第一"), item("l2", "第二")])),
        "menu_labels" => Some(Val::L(vec![s("文件"), s("编辑")])),
        "menu_entries" => Some(Val::L(vec![
            rec(vec![("label", s("复制")), ("key", s("copy"))]),
            rec(vec![("label", s("")), ("key", s(""))]),
            rec(vec![("label", s("粘贴")), ("key", s("paste"))]),
        ])),
        "tbl_heads" => Some(Val::L(vec![s("名称"), s("数量"), s("状态")])),
        "tbl_rows" => Some(Val::L(vec![
            rec(vec![("cells", Val::L(vec![s("苹果"), s("3"), s("在库")]))]),
            rec(vec![("cells", Val::L(vec![s("梨"), s("7"), s("缺货")]))]),
        ])),
        "sw_pages" => Some(Val::L(vec![
            rec(vec![("title", s("第一页")), ("desc", s("说明一"))]),
            rec(vec![("title", s("第二页")), ("desc", s("说明二"))]),
        ])),
        "dlg_buttons" => Some(Val::L(vec![
            rec(vec![
                ("id", s("cancel")),
                ("label", s("取消")),
                ("primary", b(false)),
            ]),
            rec(vec![
                ("id", s("ok")),
                ("label", s("确定")),
                ("primary", b(true)),
            ]),
        ])),
        "dp_heads" => Some(Val::L(vec![s("日"), s("一")])),
        // 35 格：1..=15 有日期、16..=35 空格（覆盖 selected=15 与空格两路）。
        "dp_cells" => Some(Val::L(
            (1..=35)
                .map(|d| {
                    let day = if d <= 15 { f(d as f32) } else { f(-1.0) };
                    rec(vec![("day", day)])
                })
                .collect(),
        )),
        // TimePicker 的条目形状是 (key, label)（与 Menu 条目一致）。
        "tp_hours" => Some(Val::L(vec![
            entry("h1", "07:00"),
            entry("h2", "08:00"),
            entry("h3", "09:00"),
        ])),
        "tp_minutes" => Some(Val::L(vec![entry("m1", "05"), entry("m2", "15")])),
        "area_lines" => Some(Val::L(vec![s("第一行"), s("第二行")])),
        "sugs" => Some(Val::L(vec![s("甲"), s("乙"), s("丙")])),
        "mi_items" => Some(Val::L(vec![s("剪切"), s("复制")])),
        _ => None,
    }
}

/// 编译（dev 路径）+ 实例化一页。
fn build_zypui(page: &str) -> Vec<Element> {
    let controls = include_str!("../ui/controls.zypui");
    let parity = include_str!("../ui/parity.zypui");
    let raw: RawUi = zypuic::parse_all(&[("controls.zypui", controls), ("parity.zypui", parity)])
        .expect("parity 源应当编译通过");
    let ui = Ui::new(raw).expect("组件重名");
    let mut it = Interner::new();
    ui.build(page, &ParityHost, &mut it)
        .unwrap_or_else(|e| panic!("页 `{page}` 实例化失败：{e}"))
}

/// 对照一页：zypui 产物 vs Rust 构造函数产物。
fn assert_parity(page: &str, rust_side: Vec<Element>) {
    let zy = build_zypui(page);
    let a: Vec<String> = zy.iter().map(describe).collect();
    let b: Vec<String> = rust_side.iter().map(describe).collect();
    assert_eq!(a.len(), b.len(), "页 `{page}` 顶层元素数不一致");
    for (i, (x, y)) in a.into_iter().zip(b).enumerate() {
        assert_eq!(x, y, "页 `{page}` 第 {i} 个元素树不一致");
    }
}

// ──────────────────────────── 各控件 ────────────────────────────

#[test]
fn button_matches_rust() {
    assert_parity(
        "button",
        vec![
            controls::button("b1:h", "确定", Some("pen"), "accent", true, false),
            controls::button("b2", "取消", None, "standard", false, false),
        ],
    );
}

#[test]
fn tool_button_matches_rust() {
    assert_parity(
        "tool_button",
        vec![controls::tool_button("t1:p", "pen", false, true)],
    );
}

#[test]
fn pill_button_matches_rust() {
    assert_parity(
        "pill_button",
        vec![
            controls::pill_button("p1:h", "标签", Some("pen"), true, true, false),
            controls::pill_button("p2", "标签", None, false, false, false),
        ],
    );
}

#[test]
fn badges_match_rust() {
    assert_parity(
        "badges",
        vec![
            controls::info_badge("ib1", "新", "info", true),
            controls::info_badge("ib2", "旧", "error", false),
            controls::dot_badge("db1", "caution"),
        ],
    );
}

#[test]
fn cards_match_rust() {
    assert_parity(
        "card",
        vec![
            controls::card(
                "c1",
                "link-card",
                vec![zephyra_core::element::Element::new(NodeTag::Text)
                    .with_key("inner")
                    .with(
                        &zephyra_core::props::TEXT,
                        PropertyValue::static_str("内容"),
                    )],
            ),
            controls::card(
                "c2",
                "",
                vec![zephyra_core::element::Element::new(NodeTag::Text)
                    .with_key("inner2")
                    .with(&zephyra_core::props::TEXT, PropertyValue::static_str("x"))],
            ),
        ],
    );
}

#[test]
fn setting_card_matches_rust() {
    assert_parity(
        "setting_card",
        vec![controls::setting_card(
            "sc1",
            "标题",
            "描述",
            controls::toggle_switch("sw1", true, false, false),
        )],
    );
}

#[test]
fn clip_card_matches_rust() {
    assert_parity(
        "clip_card",
        vec![controls::clip_card("cc1", "pen", "题", "述")],
    );
}

#[test]
fn expander_matches_rust() {
    assert_parity(
        "expander",
        vec![
            controls::expander("exp1", "展开", true, vec![text_leaf("c1", "内容")]),
            controls::expander("exp2", "收起", false, vec![text_leaf("c2", "内容")]),
        ],
    );
}

fn text_leaf(key: &'static str, text: &'static str) -> Element {
    Element::new(NodeTag::Text)
        .with_key(key)
        .with(&zephyra_core::props::TEXT, PropertyValue::static_str(text))
}

#[test]
fn text_matches_rust() {
    assert_parity(
        "text",
        vec![
            controls::text_label("tl1", "大标题", "text-title"),
            controls::caption("cp1", "说明"),
        ],
    );
}

#[test]
fn toggles_match_rust() {
    assert_parity(
        "toggles",
        vec![
            controls::check_box("ck1:h", "复选", true, true, false),
            controls::radio_button("rb1", "单选", false, false, false),
            controls::toggle_switch("sw1:p", true, false, true),
        ],
    );
}

#[test]
fn progress_matches_rust() {
    assert_parity(
        "progress",
        vec![
            controls::slider("sl1:h", 0.42, Some("42"), true, false),
            controls::slider("sl2", 0.42, None, false, false),
            controls::progress_bar("pb1", 0.8, "error"),
        ],
    );
}

#[test]
fn segmented_matches_rust() {
    // 逐项 hover/press 都为假（parity 页的行 key 不带后缀）。
    assert_parity(
        "segmented",
        vec![controls::segmented(
            "seg1",
            &[("warm", "暖"), ("cool", "冷")],
            Some("warm"),
            None,
            None,
        )],
    );
}

#[test]
fn nav_matches_rust() {
    assert_parity(
        "nav",
        vec![
            controls::nav_item("n1:h", "首页", "home", true, true, false, false),
            controls::nav_item("n2", "设置", "gear", false, false, false, true),
            controls::nav_sub_item("ns1:p", "子项", Some("pen"), false, false, true),
            controls::nav_sub_item("ns2", "子项2", None, true, false, false),
        ],
    );
}

#[test]
fn menu_matches_rust() {
    assert_parity(
        "menu",
        vec![
            controls::menu_bar("mb1", &["文件", "编辑"], Some(1)),
            controls::menu("m1", &[("复制", "copy"), ("", ""), ("粘贴", "paste")]),
            controls::menu_item("mi1:h", "复制", false, false, true, false),
        ],
    );
}

#[test]
fn selector_bar_matches_rust() {
    assert_parity(
        "selector",
        vec![controls::selector_bar(
            "sb1",
            &[("a", "甲"), ("b", "乙")],
            Some("b"),
            None,
            None,
        )],
    );
}

#[test]
fn error_page_matches_rust() {
    assert_parity(
        "error_page",
        vec![controls::error_page(
            "ep1",
            "出错了",
            "网络不可用",
            "重试",
            "retry:h",
            true,
            false,
        )],
    );
}

#[test]
fn status_matches_rust() {
    assert_parity(
        "status",
        vec![
            controls::progress_ring("pr1", "72", "running", true),
            controls::progress_ring("pr2", "", "paused", false),
            controls::info_bar("if1", "注意", "有更新", "caution", true),
            controls::info_bar("if2", "错误", "失败", "error", false),
            controls::toast("to1", "已保存", "草稿", "success"),
            controls::tool_tip("tt1", "提示内容"),
            controls::busy_indicator("bi1", 1),
            controls::page_indicator("pi1", 4, 2),
        ],
    );
}

#[test]
fn forms_match_rust() {
    assert_parity(
        "forms",
        vec![
            controls::round_button("rb1:h", "圆", Some("pen"), true, true, false),
            controls::hyperlink("hl1", "链接", "https://example.com", false),
            controls::drop_down_button("dd1:p", "下拉", Some("pen"), true, false, true),
            controls::combo_box("cb1:h", "第二项", true, false),
            controls::spin_box("sp1", "5", false, false, false, false, false, false),
            controls::color_picker("col1", 9, None, None),
        ],
    );
}

#[test]
fn collections_match_rust() {
    assert_parity(
        "collections",
        vec![
            controls::list_view(
                "lv1",
                &[("l1", "第一"), ("l2", "第二")],
                Some("l2"),
                None,
                None,
            ),
            controls::table_view(
                "tv1",
                &["名称", "数量", "状态"],
                &[&["苹果", "3", "在库"], &["梨", "7", "缺货"]],
                None,
                None,
            ),
            controls::swipe_view(
                "svp1",
                &[("第一页", "说明一"), ("第二页", "说明二")],
                1,
                None,
                None,
            ),
            controls::avatar("av1", "Z", true),
            controls::avatar("av2", "ab", false),
            controls::setting_item("si1", "gear", "标题", "描述"),
        ],
    );
}

#[test]
fn setting_expander_matches_rust() {
    assert_parity(
        "setting_expander",
        vec![controls::setting_expander(
            "se1",
            "标题",
            "描述",
            true,
            vec![text_leaf("c1", "内容")],
        )],
    );
}

#[test]
fn dialogs_match_rust() {
    assert_parity(
        "dialog",
        vec![
            controls::acrylic_card("ac1", vec![text_leaf("c1", "内容")]),
            controls::rounded_corner_overlay("co1", "r-lg", vec![text_leaf("c2", "内容")]),
            controls::dialog(
                "dg1",
                "确认",
                "确定要删吗",
                &[("cancel", "取消", false), ("ok", "确定", true)],
                None,
                None,
            ),
        ],
    );
}

#[test]
fn date_time_matches_rust() {
    // cells 的形状是 `&'static [Option<u8>]`（控件签名如此）：1..=15 有
    // 日期、16..=35 空格 —— 覆盖 selected=15 与空格两路。
    const CELLS: [Option<u8>; 35] = {
        let mut out: [Option<u8>; 35] = [None; 35];
        let mut d: usize = 1;
        while d <= 15 {
            out[d - 1] = Some(d as u8);
            d += 1;
        }
        out
    };
    assert_parity(
        "date_time",
        vec![
            controls::date_picker("dp1", &["日", "一"], &CELLS, 15, None, None),
            controls::time_picker(
                "tp1",
                &[("h1", "07:00"), ("h2", "08:00"), ("h3", "09:00")],
                "08:00",
                &[("m1", "05"), ("m2", "15")],
                "05",
                None,
                None,
            ),
        ],
    );
}

#[test]
fn text_input_matches_rust() {
    assert_parity(
        "text_input",
        vec![
            controls::text_field("tf1:h", Some("输入内容"), "请输入", true, true, false),
            controls::text_field("tf2", None, "占位", false, false, false),
            controls::text_area("ta1", &["第一行", "第二行"]),
            controls::scrollable_text_area("sta1", &["第一行", "第二行"], 0.5),
            controls::auto_suggest_box(
                "as1:h",
                Some("输入内容"),
                "搜索",
                &["甲", "乙", "丙"],
                1,
                true,
                true,
            ),
            controls::auto_suggest_box("as2", None, "搜索", &["甲", "乙", "丙"], 1, false, false),
            controls::text_input_menu("tim1", &["剪切", "复制"]),
        ],
    );
}
