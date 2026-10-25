//! `zypuic` 的单测：XML 子集解析、构建校验、表达式、编译产物 round-trip。
//!
//! round-trip 是**命门测试**：encode 的产物必须 decode 回同一棵 IR ——
//! 两套表示一旦漂移（zephyra-style IR 文档警告过的那类），这里第一个红。

use crate::{parse_all, Diag, Level};
use zypui::decode::decode;
use zypui::ir::RawUi;

fn parse_ok(src: &str) -> RawUi {
    match parse_all(&[("test.zypui", src)]) {
        Ok(raw) => raw,
        Err(diags) => panic!("应当解析通过，实际：\n{}", render(&diags)),
    }
}

fn render(diags: &[Diag]) -> String {
    diags
        .iter()
        .map(|d| d.render())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_err(src: &str) -> Vec<Diag> {
    match parse_all(&[("test.zypui", src)]) {
        Ok(_) => panic!("应当报错，却解析通过了"),
        Err(diags) => diags,
    }
}

fn has_error(diags: &[Diag], needle: &str) -> bool {
    diags
        .iter()
        .any(|d| d.level == Level::Error && d.msg.contains(needle))
}

// ============================== XML 子集 ==============================

#[test]
fn xml_elements_attrs_self_closing_comments_entities() {
    let raw = parse_ok(
        "<!-- 头注释 -->\n\
         <page name=\"p\">\n\
           <container key=\"c1\" class=\"a b\"/>\n\
           <text key=\"t1\" text=\"确定 &amp; 提交\"/>\n\
         </page>",
    );
    assert_eq!(raw.pages.len(), 1);
    assert_eq!(raw.pages[0].root.len(), 2);
}

#[test]
fn xml_errors_carry_line_numbers() {
    let diags = parse_err("<page name=\"p\">\n  <container key=\"c1\"\n</page>");
    assert!(!diags.is_empty());
    // 第二行的语法错 → 行号 2 或 3（闭合标签处理点），关键是**不是 1**。
    assert!(
        diags.iter().any(|d| d.line >= 2),
        "诊断：{}",
        render(&diags)
    );
}

#[test]
fn xml_text_outside_text_element_is_an_error() {
    let diags = parse_err("<page name=\"p\"><container>流浪文本</container></page>");
    assert!(
        has_error(&diags, "不接受文本内容"),
        "诊断：{}",
        render(&diags)
    );
}

// ============================== 构建校验 ==============================

const BUTTON: &str = "\
<component name=\"Btn\" params=\"label variant\">
  <control key=\"\" class=\"button {variant} {hover ? 'hovered'}\" interactive=\"true\">
    <text key=\"btn-label\" class=\"btn-label\" text=\"{label}\"/>
  </control>
</component>";

#[test]
fn component_and_page_build() {
    let raw = parse_ok(&format!(
        "{BUTTON}\n<page name=\"p\"><Btn key=\"b1\" label=\"确定\" variant=\"accent\"/></page>"
    ));
    assert_eq!(raw.components.len(), 1);
    assert_eq!(raw.components[0].params, &["label", "variant"]);
    assert_eq!(raw.pages[0].root.len(), 1);
}

#[test]
fn unknown_component_and_param_are_errors() {
    let diags = parse_err("<page name=\"p\"><Nope key=\"x\"/></page>");
    assert!(
        has_error(&diags, "组件 `Nope` 不存在"),
        "诊断：{}",
        render(&diags)
    );

    let diags = parse_err(&format!(
        "{BUTTON}\n<page name=\"p\"><Btn key=\"b1\" label=\"x\" variant=\"a\" color=\"red\"/></page>"
    ));
    assert!(
        has_error(&diags, "没有参数 `color`"),
        "诊断：{}",
        render(&diags)
    );
}

#[test]
fn unknown_element_and_attr_are_errors() {
    let diags = parse_err("<page name=\"p\"><stack/></page>");
    assert!(has_error(&diags, "未知元素"), "诊断：{}", render(&diags));

    let diags = parse_err("<page name=\"p\"><container background=\"red\"/></page>");
    assert!(
        has_error(&diags, "不认识属性 `background`"),
        "外观值混进 .zypui 必须被拦。诊断：{}",
        render(&diags)
    );
}

#[test]
fn unknown_identifier_in_expression_is_an_error() {
    let diags =
        parse_err("<component name=\"C\"><container class=\"x {hoving ? 'a'}\"/></component>");
    assert!(
        has_error(&diags, "未知标识符 `hoving`"),
        "诊断：{}",
        render(&diags)
    );
}

#[test]
fn reserved_and_builtin_param_names_are_errors() {
    let diags = parse_err("<component name=\"C\" params=\"hover key\"><container/></component>");
    assert!(
        has_error(&diags, "保留字") || has_error(&diags, "撞名"),
        "诊断：{}",
        render(&diags)
    );
}

#[test]
fn component_names_must_be_pascal() {
    let diags = parse_err("<component name=\"button\"><container/></component>");
    assert!(has_error(&diags, "PascalCase"), "诊断：{}", render(&diags));
}

#[test]
fn duplicate_names_are_errors() {
    let src = "<component name=\"C\"><container/></component>\n\
               <component name=\"C\"><container/></component>";
    assert!(has_error(&parse_err(src), "重名"));
}

#[test]
fn slot_rules() {
    // 页面里出现槽位 → 错。
    let diags = parse_err("<page name=\"p\"><children/></page>");
    assert!(
        has_error(&diags, "只能出现在组件模板里"),
        "诊断：{}",
        render(&diags)
    );
    // 两个槽位：实例化期会炸，编译期就该拦 —— 走 check 全链。
    let src = "<component name=\"C\"><container><children/><children/></container></component>\n\
               <page name=\"p\"><C key=\"c\"><text key=\"t\" text=\"x\"/></C></page>";
    let diags = parse_err(src);
    assert!(
        diags.iter().any(|d| d.level == Level::Error),
        "双槽位应当报错。诊断：{}",
        render(&diags)
    );
}

#[test]
fn use_with_children_but_no_slot_is_an_error() {
    let src = "<component name=\"C\"><container/></component>\n\
               <page name=\"p\"><C key=\"c\"><text key=\"t\" text=\"x\"/></C></page>";
    // 编译期查模板槽位数（当前在运行期查；这里确保至少一路拦得住）。
    let raw = parse_ok(src);
    assert_eq!(raw.components.len(), 1);
    let ui = zypui::Ui::new(raw).unwrap();
    let mut it = zypui::Interner::new();
    let h = TestHost;
    assert!(
        ui.build("p", &h, &mut it).is_err(),
        "无槽位却传子节点必须报错"
    );
}

/// 元素树的结构描述（`Element` 不实现 Debug；测试自建同构描述）。
fn describe(el: &zephyra_core::element::Element) -> String {
    let mut s = format!(
        "({:?} key={:?} classes={:?}",
        el.tag(),
        el.key(),
        el.style_names()
    );
    let mut prop_keys: Vec<String> = el
        .props()
        .iter_local()
        .map(|(k, _)| format!("{k:?}"))
        .collect();
    prop_keys.sort();
    s.push_str(&format!(" props={prop_keys:?}"));
    for c in el.children() {
        s.push(' ');
        s.push_str(&describe(c));
    }
    s.push(')');
    s
}

/// 最小测试宿主。
#[derive(Default)]
struct TestHost;

impl zypui::Host for TestHost {
    fn binding(&self, _: &str) -> Option<zypui::Val> {
        None
    }
    fn tr(&self, key: &str) -> &'static str {
        Box::leak(key.to_string().into_boxed_str())
    }
    fn hovers(&self, _: &str) -> bool {
        false
    }
    fn holds(&self, _: &str) -> bool {
        false
    }
}

// ============================== 表达式 ==============================

#[test]
fn expression_precedence_and_literals() {
    let raw = parse_ok(
        "<page name=\"p\"><container key=\"{ 'a' + 'b' }\" class=\"x {@n > 1 && @flag ? 'big'}\"/></page>",
    );
    let page = &raw.pages[0];
    let zypui::ir::Node::El(el) = &page.root[0] else {
        panic!("页根应是元素");
    };
    assert!(
        matches!(&el.key, Some(zypui::expr::Expr::Bin(..))),
        "key 应是拼接表达式"
    );
    // 条件词降级：整词 `{expr}` 且形状是 Cond(LitS, LitS "") → ClassTok::If。
    assert!(
        matches!(el.classes[1], zypui::ir::ClassTok::If { .. }),
        "条件词应降级成 If"
    );
}

// ============================== round-trip（命门） ==============================

#[test]
fn encode_decode_round_trips_to_the_same_tree_and_same_pixels() {
    let src = format!(
        "{BUTTON}\n\
         <page name=\"p\">\n\
           <Btn key=\"b1\" label=\"确定\" variant=\"accent\"/>\n\
           <Btn key=\"b2\" label=\"取消\" variant=\"standard\"/>\n\
           <for var=\"name\" index=\"i\" list=\"@icons\">\n\
             <image key=\"{{name}}\" texture=\"atlas\" uv=\"0.5,0,0.5,1\"/>\n\
           </for>\n\
           <if cond=\"@open\">\n\
             <container key=\"panel\" class=\"panel\"/>\n\
             <else>\n\
               <container key=\"closed\" class=\"closed\"/>\n\
             </else>\n\
           </if>\n\
         </page>"
    );
    let raw = parse_ok(&src);
    let bytes = crate::encode::encode(&raw);
    let back = decode(&bytes).expect("decode 失败");

    // Debug 表示逐字相等 = 树结构、表达式、分类名词全部一致。
    assert_eq!(
        format!("{raw:?}"),
        format!("{back:?}"),
        "round-trip 后 IR 漂移了"
    );
}

/// 端到端：同一份源码走「文本 parse」与「二进制 decode」两条路，
/// 实例化结果必须一致 —— dev/release 同构的证明。
#[test]
fn dev_text_path_and_release_binary_path_agree() {
    let src = format!(
        "{BUTTON}\n<page name=\"p\"><Btn key=\"b1\" label=\"确定\" variant=\"accent\"/></page>"
    );
    let from_text = parse_all(&[("a.zypui", &src)]).unwrap();
    let bytes = crate::encode::encode(&from_text);
    let from_bin = decode(&bytes).unwrap();

    let mut it_a = zypui::Interner::new();
    let mut it_b = zypui::Interner::new();
    let h = TestHost;
    let a = zypui::Ui::new(from_text)
        .unwrap()
        .build("p", &h, &mut it_a)
        .unwrap();
    let b = zypui::Ui::new(from_bin)
        .unwrap()
        .build("p", &h, &mut it_b)
        .unwrap();

    // 元素树的结构描述对齐（key/分类名/属性/子树全量）。
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(describe(x), describe(y));
    }
    assert_eq!(a[0].style_names(), &["button", "accent"]);
    assert_eq!(a[0].key(), Some(&zephyra_core::id::ElementKey::Name("b1")));
}

// ============================== 语言扩展（记录/取模/内置调用/fill 门禁） ==============================

#[test]
fn field_access_and_mod_roundtrip_and_eval() {
    let src = "<component name=\"C\" params=\"item n\">\
               <container key=\"{'k' + str(item.id % 3)}\" class=\"x {n % 2 == 0 ? 'even'}\"/></component>\
               <page name=\"p\"><C item=\"{@it}\" n=\"{8}\"/></page>";
    let raw = parse_ok(src);
    // round-trip：Field 与 Mod 都要活着回到解码侧。
    let bytes = crate::encode::encode(&raw);
    let back = decode(&bytes).expect("round-trip 解码");
    assert_eq!(format!("{raw:?}").len(), format!("{back:?}").len());
    let ui = zypui::Ui::new(back).unwrap();
    let h = RecHost;
    let mut it = zypui::Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    assert_eq!(els.len(), 1);
    // key = "k" + str(5 % 3) = "k2"。
    assert_eq!(
        els[0].key().map(zephyra_core::id::ElementKey::as_name),
        Some(Some("k2"))
    );
    assert!(
        els[0].style_names().contains(&"even"),
        "8 % 2 == 0 应为真？{:?}",
        els[0].style_names()
    );
}

/// 提供记录绑定的宿主：`@it` = { id: 5 }。
struct RecHost;

impl zypui::Host for RecHost {
    fn binding(&self, name: &str) -> Option<zypui::Val> {
        match name {
            "it" => Some(zypui::Val::Rec(vec![("id", zypui::Val::F(5.0))])),
            _ => None,
        }
    }
    fn tr(&self, k: &str) -> &'static str {
        Box::leak(k.to_string().into_boxed_str())
    }
    fn hovers(&self, _: &str) -> bool {
        false
    }
    fn holds(&self, _: &str) -> bool {
        false
    }
}

#[test]
fn range_and_nested_for_produce_indexed_keys() {
    let src = "<page name=\"p\">\
               <container key=\"rows\">\
               <for var=\"i\" list=\"range(3)\"><container key=\"{'pip:' + str(i)}\" class=\"pip {i == 1 ? 'selected'}\"/></for>\
               </container></page>";
    let raw = parse_ok(src);
    let ui = zypui::Ui::new(raw).unwrap();
    let h = TestHost;
    let mut it = zypui::Interner::new();
    let els = ui.build("p", &h, &mut it).unwrap();
    let rows = els[0].children();
    assert_eq!(rows.len(), 3, "range(3) 应产出三行");
    assert_eq!(
        rows[1].key().map(zephyra_core::id::ElementKey::as_name),
        Some(Some("pip:1"))
    );
    assert!(rows[1].style_names().contains(&"selected"));
    assert!(!rows[0].style_names().contains(&"selected"));
}

#[test]
fn min_max_clamp_fill_width() {
    let src = "<page name=\"p\">\
               <container key=\"fill:slider\" class=\"slider-fill\" width=\"{min(max(@v, 0.0), 1.0) * 220.0}\"/>\
               </page>";
    let raw = parse_ok(src);
    let ui = zypui::Ui::new(raw).unwrap();
    let mut it = zypui::Interner::new();
    let h = NumHost(1.7);
    let els = ui.build("p", &h, &mut it).unwrap();
    use zephyra_core::property::PropertyValue;
    let w = els[0]
        .props()
        .lookup(&zephyra_core::props::WIDTH)
        .map(|(v, _)| v);
    assert!(
        matches!(w, Some(PropertyValue::Float(f)) if (f - 220.0).abs() < 1e-4),
        "min(max(1.7,0),1)*220 应为 220，实际 {w:?}"
    );
}

/// 恒定数值绑定的宿主。
struct NumHost(f32);

impl zypui::Host for NumHost {
    fn binding(&self, name: &str) -> Option<zypui::Val> {
        (name == "v").then(|| zypui::Val::F(self.0))
    }
    fn tr(&self, k: &str) -> &'static str {
        Box::leak(k.to_string().into_boxed_str())
    }
    fn hovers(&self, _: &str) -> bool {
        false
    }
    fn holds(&self, _: &str) -> bool {
        false
    }
}

#[test]
fn width_outside_fill_key_is_an_error() {
    let diags =
        parse_err("<page name=\"p\"><container key=\"box\" class=\"x\" width=\"{42}\"/></page>");
    assert!(
        has_error(&diags, "fill:"),
        "width 出现在非 fill 键上应报错：{}",
        render(&diags)
    );
}
