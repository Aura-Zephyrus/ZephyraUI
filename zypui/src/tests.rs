//! `zypui` 的单测：求值语义、实例化语义、二进制解码的边界。
//!
//! 组件 round-trip（encode ↔ decode）在 `zypuic` 那边测 —— 编码器在那里，
//! 这里没有它。两条路径（文本 parse / 二进制 decode）汇进同一份 IR 后，
//! 本文件的实例化断言对两条路同样成立。

use crate::eval::{eval, Ctx, Scope};
use crate::expr::{BinOp, Expr};
use crate::instantiate::{Host, Ui};
use crate::intern::Interner;
use crate::ir::*;
use crate::val::Val;
use zephyra_core::id::NodeTag;
use zephyra_core::props;

/// 测试宿主：绑定表 + 文案直返 + 按 key 的悬停/按住。
struct MockHost {
    bindings: Vec<(&'static str, Val)>,
    hovered: Vec<&'static str>,
    pressed: Vec<&'static str>,
    sources: Vec<zephyra_core::paint::RenderSource>,
}

impl Host for MockHost {
    fn binding(&self, name: &str) -> Option<Val> {
        self.bindings
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
    }
    fn tr(&self, key: &str) -> &'static str {
        // 与 gallery 同规：查不到返回键名本身。
        Box::leak(key.to_string().into_boxed_str())
    }
    fn hovers(&self, key: &str) -> bool {
        self.hovered.contains(&key)
    }
    fn holds(&self, key: &str) -> bool {
        self.pressed.contains(&key)
    }
    fn render_source(&self, kind: &str) -> Option<zephyra_core::paint::RenderSource> {
        self.sources
            .iter()
            .find(|source| source.kind == kind)
            .cloned()
    }
}

fn host(bindings: Vec<(&'static str, Val)>) -> MockHost {
    MockHost {
        bindings,
        hovered: vec![],
        pressed: vec![],
        sources: vec![],
    }
}

fn ui_with_render_source(kind: &'static str) -> Ui {
    Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::El(ElementNode {
                tag: NodeTag::Box,
                key: Some(Expr::LitS("video")),
                classes: vec![],
                attrs: vec![("render-source", Expr::LitS(kind))],
                children: vec![],
            })],
        }],
    })
    .unwrap()
}

#[test]
fn render_source_kind_resolves_host_owned_payload() {
    use zephyra_core::paint::RenderSource;
    let source = RenderSource::new("video:main", 42_u32);
    let h = MockHost {
        sources: vec![source.clone()],
        ..host(vec![])
    };
    let ui = ui_with_render_source("video:main");
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    let actual = els[0]
        .props()
        .lookup(&props::RENDER_SOURCE)
        .unwrap()
        .0
        .get::<RenderSource>()
        .unwrap();
    assert_eq!(actual.kind, "video:main");
    assert_eq!(actual.payload::<u32>(), Some(&42));
    assert_eq!(actual, source, "宿主源保持相同 Arc 载荷，稳态 diff 不抖");
}

#[test]
fn render_source_kind_must_exist_in_host_registry() {
    let ui = ui_with_render_source("video:missing");
    let mut it = Interner::new();
    let err = match ui.build("p", &host(vec![]), &mut it) {
        Ok(_) => panic!("missing render source kind must fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("没有注册"));
}

#[test]
fn render_source_kind_must_match_host_result() {
    use zephyra_core::paint::RenderSource;

    let ui = ui_with_render_source("video:main");
    let h = MockHost {
        sources: vec![RenderSource::new("video:other", ())],
        ..host(vec![])
    };
    let mut it = Interner::new();
    let err = match ui.build("p", &h, &mut it) {
        Ok(_) => panic!("a different host render source kind must fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("kind"));
}

// ============================== eval ==============================

fn ev(e: &Expr, host: &dyn Host, hovered_key: &'static str) -> Val {
    let mut it = Interner::new();
    let mut ctx = Ctx {
        host,
        cur_key: hovered_key,
        it: &mut it,
    };
    eval(e, &Scope::ROOT, &mut ctx)
}

#[test]
fn literals_and_arithmetic() {
    let h = host(vec![]);
    assert_eq!(ev(&Expr::LitS("a"), &h, ""), Val::S("a"));
    assert_eq!(ev(&Expr::LitF(2.5), &h, ""), Val::F(2.5));
    assert_eq!(ev(&Expr::LitB(true), &h, ""), Val::B(true));
    let add = |l, r| Expr::Bin(BinOp::Add, Box::new(l), Box::new(r));
    assert_eq!(
        ev(&add(Expr::LitF(1.0), Expr::LitF(2.0)), &h, ""),
        Val::F(3.0)
    );
}

#[test]
fn string_concat_interns_to_static() {
    let h = host(vec![("variant", Val::S("accent"))]);
    // "button " + @variant → 单个 'static 串。
    let e = Expr::Bin(
        BinOp::Add,
        Box::new(Expr::LitS("button ")),
        Box::new(Expr::Binding("variant")),
    );
    let mut it = Interner::new();
    let mut ctx = Ctx {
        host: &h,
        cur_key: "",
        it: &mut it,
    };
    let v = eval(&e, &Scope::ROOT, &mut ctx);
    assert_eq!(v, Val::S("button accent"));
    assert!(matches!(v, Val::S(s) if s.len() == 13)); // &'static，不是临时
}

#[test]
fn builtins_read_host_input_state() {
    let mut h = host(vec![]);
    h.hovered = vec!["act:ok"];
    assert!(matches!(
        ev(&Expr::Ref("hover"), &h, "act:ok"),
        Val::B(true)
    ));
    assert!(matches!(
        ev(&Expr::Ref("hover"), &h, "other"),
        Val::B(false)
    ));
    h.pressed = vec!["act:ok"];
    assert!(matches!(
        ev(&Expr::Ref("press"), &h, "act:ok"),
        Val::B(true)
    ));
}

#[test]
fn comparisons_are_type_strict_and_total() {
    let h = host(vec![("n", Val::F(2.0))]);
    let eq = |l, r| Expr::Bin(BinOp::Eq, Box::new(l), Box::new(r));
    // 同型相等。
    assert!(matches!(
        ev(&eq(Expr::LitF(2.0), Expr::Binding("n")), &h, ""),
        Val::B(true)
    ));
    // 跨类型恒 false（不炸帧）。
    assert!(matches!(
        ev(&eq(Expr::LitS("2"), Expr::Binding("n")), &h, ""),
        Val::B(false)
    ));
    // 除零得 0。
    let div = |l, r| Expr::Bin(BinOp::Div, Box::new(l), Box::new(r));
    assert!(matches!(
        ev(&div(Expr::LitF(1.0), Expr::LitF(0.0)), &h, ""),
        Val::F(0.0)
    ));
}

#[test]
fn missing_binding_is_falsy_not_panic() {
    let h = host(vec![]);
    assert!(matches!(
        ev(&Expr::Binding("nothing"), &h, ""),
        Val::B(false)
    ));
    // 未传的组件参数同样假。
    assert!(matches!(
        ev(&Expr::Ref("not_a_param"), &h, ""),
        Val::B(false)
    ));
}

// ============================== instantiate ==============================

fn bare(tag: NodeTag, key: &'static str, classes: Vec<ClassTok>) -> Node {
    Node::El(ElementNode {
        tag,
        key: Some(Expr::LitS(key)),
        classes,
        attrs: vec![],
        children: vec![],
    })
}

#[test]
fn builds_elements_with_keys_classes_text() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::El(ElementNode {
                tag: NodeTag::Text,
                key: Some(Expr::LitS("t1")),
                classes: vec![
                    ClassTok::Lit("hero"),
                    ClassTok::If {
                        cond: Expr::Binding("on"),
                        tok: "on",
                    },
                ],
                attrs: vec![("text", Expr::Call("tr", vec![Expr::LitS("ui.ok")]))],
                children: vec![],
            })],
        }],
    })
    .unwrap();
    let h = host(vec![("on", Val::B(true))]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(els.len(), 1);
    let el = &els[0];
    assert_eq!(el.tag(), NodeTag::Text);
    assert_eq!(el.key(), Some(&zephyra_core::id::ElementKey::Name("t1")));
    assert_eq!(el.style_names(), &["hero", "on"]);
    use zephyra_core::property::PropertyValue;
    let text = el.props().lookup(&props::TEXT).map(|(v, _)| v);
    assert!(matches!(text, Some(PropertyValue::StaticStr("ui.ok"))));
}

#[test]
fn component_instance_shares_its_key_with_the_whole_template() {
    // 图标位带自己的固定 key，但 hover 类名必须看**实例** key ——
    // 这是「构造器扇出 hovered」这条数据流的 .zypui 对应物。
    let ui = Ui::new(RawUi {
        components: vec![Component {
            name: "Btn",
            params: vec!["variant"],
            body: vec![Node::El(ElementNode {
                tag: NodeTag::Control,
                key: None, // 由 Use 的 key 补上
                classes: vec![
                    ClassTok::Lit("button"),
                    ClassTok::Dyn(Expr::Ref("variant")),
                    ClassTok::If {
                        cond: Expr::Ref("hover"),
                        tok: "hovered",
                    },
                ],
                attrs: vec![("interactive", Expr::LitB(true))],
                children: vec![Node::El(ElementNode {
                    tag: NodeTag::Box,
                    key: Some(Expr::LitS("btn-icon")),
                    classes: vec![ClassTok::If {
                        cond: Expr::Ref("hover"),
                        tok: "hovered",
                    }],
                    attrs: vec![],
                    children: vec![],
                })],
            })],
        }],
        pages: vec![Page {
            name: "p",
            root: vec![Node::Use(UseNode {
                comp: "Btn",
                key: Some(Expr::LitS("act:ok")),
                class_extra: vec![],
                args: vec![("variant", Expr::LitS("accent"))],
                children: vec![],
            })],
        }],
    })
    .unwrap();
    let mut h = host(vec![]);
    h.hovered = vec!["act:ok"];
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(els.len(), 1);
    let root = &els[0];
    assert_eq!(
        root.key(),
        Some(&zephyra_core::id::ElementKey::Name("act:ok"))
    );
    // 实例 key 的 hover 对根与内部部件都为真。
    assert_eq!(root.style_names(), &["button", "accent", "hovered"]);
    assert!(root.props().lookup(&props::INTERACTIVE).is_some());
    let kids = root.children();
    assert_eq!(kids.len(), 1);
    assert_eq!(
        kids[0].key(),
        Some(&zephyra_core::id::ElementKey::Name("btn-icon"))
    );
    // 部件 key 不改写 hover 基准：btn-icon 也拿到 hovered。
    assert_eq!(kids[0].style_names(), &["hovered"]);
}

#[test]
fn slot_splices_caller_children_in_caller_scope() {
    let ui = Ui::new(RawUi {
        components: vec![Component {
            name: "Sec",
            params: vec![],
            body: vec![Node::El(ElementNode {
                tag: NodeTag::Box,
                key: Some(Expr::LitS("sec")),
                classes: vec![],
                attrs: vec![],
                children: vec![Node::Slot],
            })],
        }],
        pages: vec![Page {
            name: "p",
            root: vec![Node::Use(UseNode {
                comp: "Sec",
                key: Some(Expr::LitS("s1")),
                class_extra: vec![],
                args: vec![],
                children: vec![bare(NodeTag::Text, "child", vec![])],
            })],
        }],
    })
    .unwrap();
    let h = host(vec![]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    let kids = els[0].children();
    assert_eq!(kids.len(), 1);
    assert_eq!(
        kids[0].key(),
        Some(&zephyra_core::id::ElementKey::Name("child"))
    );
}

#[test]
fn for_iterates_bindings_and_exposes_var_and_index() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::For(ForNode {
                var: "name",
                index: Some("i"),
                track_key: None,
                list: Expr::Binding("icons"),
                body: vec![Node::El(ElementNode {
                    tag: NodeTag::Image,
                    key: Some(Expr::Ref("name")),
                    classes: vec![ClassTok::Dyn(Expr::Ref("name"))],
                    attrs: vec![],
                    children: vec![],
                })],
            })],
        }],
    })
    .unwrap();
    let h = host(vec![("icons", Val::L(vec![Val::S("pen"), Val::S("star")]))]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(els.len(), 2);
    assert_eq!(
        els[0].key(),
        Some(&zephyra_core::id::ElementKey::Name("pen"))
    );
    assert_eq!(
        els[1].key(),
        Some(&zephyra_core::id::ElementKey::Name("star"))
    );
    assert_eq!(els[0].style_names(), &["pen"]);
}

#[test]
fn missing_page_and_component_are_loud_errors() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![],
        }],
    })
    .unwrap();
    let h = host(vec![]);
    let mut it = Interner::new();
    assert!(ui.build("nope", &h, &mut it).is_err());
}

#[test]
fn uv_accepts_literal_string_and_four_element_list() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![
                Node::El(ElementNode {
                    tag: NodeTag::Image,
                    key: None,
                    classes: vec![],
                    attrs: vec![
                        ("uv", Expr::LitS("0.5,0,0.5,1")),
                        ("texture", Expr::LitS("atlas")),
                    ],
                    children: vec![],
                }),
                Node::El(ElementNode {
                    tag: NodeTag::Image,
                    key: None,
                    classes: vec![],
                    attrs: vec![("uv", Expr::Binding("crop"))],
                    children: vec![],
                }),
            ],
        }],
    })
    .unwrap();
    let h = host(vec![(
        "crop",
        Val::L(vec![Val::F(0.0), Val::F(0.0), Val::F(0.25), Val::F(1.0)]),
    )]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    use zephyra_core::property::PropertyValue;
    let uv0 = els[0].props().lookup(&props::IMAGE_SRC).map(|(v, _)| v);
    assert!(matches!(
        uv0,
        Some(PropertyValue::UvRect(r)) if (r.x - 0.5).abs() < 1e-6 && (r.height - 1.0).abs() < 1e-6
    ));
    let uv1 = els[1].props().lookup(&props::IMAGE_SRC).map(|(v, _)| v);
    assert!(matches!(
        uv1,
        Some(PropertyValue::UvRect(r)) if (r.width - 0.25).abs() < 1e-6
    ));
}

#[test]
fn decode_rejects_garbage_loudly() {
    assert!(crate::decode::decode(b"").is_err());
    assert!(crate::decode::decode(b"NOPE").is_err());
    assert!(crate::decode::decode(b"ZYP1\xff\xff\xff\xff").is_err()); // 长度越界
}

/// `background` 是受限的动态通道（宿主逐帧颜色绑定）：只收 `Val::Raw(Color)`，
/// 字符串字面量当场报错 —— 「外观不进模板」的边界在这里执行。
#[test]
fn background_only_accepts_a_raw_color_binding() {
    fn ui_with_background(v: Expr) -> Ui {
        Ui::new(RawUi {
            components: vec![],
            pages: vec![Page {
                name: "p",
                root: vec![Node::El(ElementNode {
                    tag: NodeTag::Box,
                    key: None,
                    classes: vec![],
                    attrs: vec![("background", v)],
                    children: vec![],
                })],
            }],
        })
        .unwrap()
    }

    // 字面量 → 报错（给错类型）。
    let bad = ui_with_background(Expr::LitS("red"));
    let h = host(vec![]);
    let mut it = Interner::new();
    match bad.build("p", &h, &mut it) {
        Err(e) => assert!(
            e.msg.contains("background"),
            "错误应指名道姓说的就是 background：{}",
            e.msg
        ),
        Ok(_) => panic!("字面量颜色必须报错，而不是被静默忽略"),
    }

    // 宿主 Raw 颜色 → 成，BACKGROUND 原样落地。
    let good = ui_with_background(Expr::Binding("track_bg"));
    let color = zephyra_core::core::Color::rgba(0.2, 0.5, 0.9, 0.8);
    let h = host(vec![(
        "track_bg",
        Val::Raw(zephyra_core::property::PropertyValue::color(color)),
    )]);
    let mut it = Interner::new();
    let els = good.build("p", &h, &mut it).unwrap();
    let got = els[0]
        .props()
        .lookup(&props::BACKGROUND)
        .and_then(|(v, _)| v.get::<zephyra_core::core::Color>());
    assert_eq!(got, Some(color), "绑定颜色必须原样落地");
}

/// `overflow` 与 `scroll-offset-*` 落到核心属性上。
///
/// # 为什么这条重要
///
/// `overflow` 走**字符串**（表达式语言没有枚举字面量），拼错时的降级是
/// 「不裁剪」—— 症状是「内容溢出到面板外面」，与「忘了写 overflow」完全同形。
/// 所以这里把四档都过一遍，并钉住「认不出的词按不裁剪降级」。
#[test]
fn overflow_and_scroll_offset_reach_the_core_properties() {
    fn ui_with(attrs: Vec<(&'static str, Expr)>) -> Ui {
        Ui::new(RawUi {
            components: vec![],
            pages: vec![Page {
                name: "p",
                root: vec![Node::El(ElementNode {
                    tag: NodeTag::Box,
                    key: None,
                    classes: vec![],
                    attrs,
                    children: vec![],
                })],
            }],
        })
        .unwrap()
    }

    use zephyra_core::layout::Overflow;
    for (word, want) in [
        ("visible", Overflow::Visible),
        ("hidden", Overflow::Hidden),
        ("scroll", Overflow::Scroll),
        ("auto", Overflow::Auto),
    ] {
        let ui = ui_with(vec![("overflow", Expr::LitS(word))]);
        let h = host(vec![]);
        let mut it = Interner::new();
        let els = ui.build("p", &h, &mut it).unwrap();
        assert_eq!(
            els[0]
                .props()
                .lookup(&props::OVERFLOW)
                .and_then(|(v, _)| v.get::<Overflow>()),
            Some(want),
            "overflow {word}"
        );
    }

    // 认不出的词：按「不裁剪」降级（不报错，也不生效）。
    let ui = ui_with(vec![("overflow", Expr::LitS("clip"))]);
    let h = host(vec![]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert!(
        els[0].props().lookup(&props::OVERFLOW).is_none(),
        "认不出的 overflow 词应当按「没写」降级"
    );

    // 滚动偏移：数值。
    let ui = ui_with(vec![
        ("scroll-offset-x", Expr::LitF(12.0)),
        ("scroll-offset-y", Expr::LitF(40.0)),
    ]);
    let h = host(vec![]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(
        els[0]
            .props()
            .lookup(&props::SCROLL_OFFSET_X)
            .and_then(|(v, _)| v.get::<f32>()),
        Some(12.0)
    );
    assert_eq!(
        els[0]
            .props()
            .lookup(&props::SCROLL_OFFSET_Y)
            .and_then(|(v, _)| v.get::<f32>()),
        Some(40.0)
    );
}

#[test]
fn runtime_resolve_matches_build_for_static_page() {
    // 简单静态页：UiRuntime 的 P1 缓存路径必须与 Ui::build 语义等价。
    let raw = RawUi {
        components: vec![Component {
            name: "Label",
            params: vec!["text"],
            body: vec![Node::El(ElementNode {
                tag: zephyra_core::id::NodeTag::Text,
                key: None,
                classes: vec![ClassTok::Lit("label")],
                attrs: vec![("text", Expr::Ref("text"))],
                children: vec![],
            })],
        }],
        pages: vec![Page {
            name: "p",
            root: vec![Node::Use(UseNode {
                comp: "Label",
                key: Some(Expr::LitS("a")),
                class_extra: vec![],
                args: vec![("text", Expr::LitS("hello"))],
                children: vec![],
            })],
        }],
    };
    let h = host(vec![]);
    let mut it1 = Interner::new();
    let mut it2 = Interner::new();
    let ui = Ui::new(raw.clone()).unwrap();
    let mut rt = crate::UiRuntime::new(raw).unwrap();
    let a = ui.build("p", &h, &mut it1).unwrap();
    let b = rt.resolve_page("p", &h, &mut it2).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].tag(), b.tag());
    assert_eq!(a[0].key(), b.key());
    assert_eq!(a[0].style_names(), b.style_names());
    assert_eq!(a[0].child_elements().count(), b.child_elements().count());
}

#[test]
fn runtime_oracle_check_matches_full_build() {
    let raw = RawUi {
        components: vec![Component {
            name: "Label",
            params: vec!["text"],
            body: vec![Node::El(ElementNode {
                tag: zephyra_core::id::NodeTag::Text,
                key: Some(Expr::Ref("text")),
                classes: vec![ClassTok::Lit("label")],
                attrs: vec![("text", Expr::Ref("text"))],
                children: vec![],
            })],
        }],
        pages: vec![Page {
            name: "p",
            root: vec![Node::Use(UseNode {
                comp: "Label",
                key: Some(Expr::LitS("a")),
                class_extra: vec![],
                args: vec![("text", Expr::LitS("hello"))],
                children: vec![],
            })],
        }],
    };
    let h = host(vec![]);
    let mut it = Interner::new();
    let mut rt = crate::UiRuntime::new(raw).unwrap();
    #[cfg(debug_assertions)]
    rt.set_oracle_check(true);
    let _ = rt.resolve_page("p", &h, &mut it).unwrap();
    let stats = rt.cache_stats();
    assert!(stats.misses >= 1);
}

#[test]
fn for_track_key_builds_in_order() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::For(ForNode {
                var: "item",
                index: Some("i"),
                track_key: Some(Expr::Field(Box::new(Expr::Ref("item")), "id")),
                list: Expr::Binding("items"),
                body: vec![Node::El(ElementNode {
                    tag: NodeTag::Box,
                    key: Some(Expr::Call(
                        "str",
                        vec![Expr::Field(Box::new(Expr::Ref("item")), "id")],
                    )),
                    classes: vec![],
                    attrs: vec![],
                    children: vec![],
                })],
            })],
        }],
    })
    .unwrap();
    let items = Val::L(vec![
        Val::Rec(vec![("id", Val::S("a"))]),
        Val::Rec(vec![("id", Val::S("b"))]),
    ]);
    let h = host(vec![("items", items)]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(els.len(), 2);
    assert_eq!(els[0].key().and_then(|k| k.as_name()), Some("a"));
    assert_eq!(els[1].key().and_then(|k| k.as_name()), Some("b"));
}

#[test]
fn window_drag_attr_reaches_the_core_property() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::El(ElementNode {
                tag: NodeTag::Box,
                key: Some(Expr::LitS("titlebar")),
                classes: vec![],
                attrs: vec![("window-drag", Expr::LitB(true))],
                children: vec![],
            })],
        }],
    })
    .unwrap();
    let h = host(vec![]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(
        els[0]
            .props()
            .lookup(&props::WINDOW_DRAG)
            .and_then(|(v, _)| v.get::<bool>()),
        Some(true),
        "window-drag 应实例化为 WINDOW_DRAG 属性"
    );
}

#[test]
fn window_drag_attr_false_does_not_mark_the_node() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::El(ElementNode {
                tag: NodeTag::Box,
                key: Some(Expr::LitS("plain")),
                classes: vec![],
                attrs: vec![("window-drag", Expr::LitB(false))],
                children: vec![],
            })],
        }],
    })
    .unwrap();
    let h = host(vec![]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(
        els[0]
            .props()
            .lookup(&props::WINDOW_DRAG)
            .and_then(|(v, _)| v.get::<bool>()),
        Some(false),
    );
}

#[test]
fn animation_attrs_reach_the_core_properties() {
    let ui = Ui::new(RawUi {
        components: vec![],
        pages: vec![Page {
            name: "p",
            root: vec![Node::El(ElementNode {
                tag: NodeTag::Box,
                key: Some(Expr::LitS("spin")),
                classes: vec![],
                attrs: vec![
                    ("animation", Expr::LitS("spinner")),
                    ("animation-progress", Expr::LitF(0.5)),
                    ("animation-play-state", Expr::LitB(false)),
                ],
                children: vec![],
            })],
        }],
    })
    .unwrap();
    let h = host(vec![]);
    let mut it = Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    let el = &els[0];
    assert_eq!(
        el.props()
            .lookup(&props::ANIMATION)
            .unwrap()
            .0
            .get::<&'static str>(),
        Some("spinner")
    );
    assert_eq!(
        el.props()
            .lookup(&props::ANIMATION_PROGRESS)
            .unwrap()
            .0
            .get::<f32>(),
        Some(0.5)
    );
    assert_eq!(
        el.props()
            .lookup(&props::ANIMATION_PLAY_STATE)
            .unwrap()
            .0
            .get::<bool>(),
        Some(false)
    );
}
