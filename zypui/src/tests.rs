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
                tag: NodeTag::Container,
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
                    tag: NodeTag::Container,
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
                tag: NodeTag::Container,
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
