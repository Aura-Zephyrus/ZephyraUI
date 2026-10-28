// 本文件由 focus.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use crate::context::{dark_context, Context};
use crate::element::Element;
use crate::id::NodeTag;
use crate::node::Node;
use crate::property::PropertyValue;
use crate::props::FOCUSABLE;

use super::{collect_focusable, FocusState};

/// 造一棵带焦点序的树：
///
/// ```text
/// root
///   a  (FOCUSABLE)
///   b  (不可聚焦)
///     c  (FOCUSABLE)
///   d  (FOCUSABLE)
/// ```
fn tree() -> Node {
    let el = Element::new(NodeTag::Container)
        .with_key("root")
        .with_children([
            Element::new(NodeTag::Control)
                .with_key("a")
                .with(&FOCUSABLE, PropertyValue::bool(true)),
            Element::new(NodeTag::Container)
                .with_key("b")
                .with_children([Element::new(NodeTag::Control)
                    .with_key("c")
                    .with(&FOCUSABLE, PropertyValue::bool(true))]),
            Element::new(NodeTag::Control)
                .with_key("d")
                .with(&FOCUSABLE, PropertyValue::bool(true)),
        ]);
    Node::from_element(&el)
}

fn ctx() -> Context<'static> {
    dark_context()
}

fn focused_key<'a>(root: &'a Node, c: Context<'_>) -> Option<&'a str> {
    // 遍历找 `is_focused` 的节点。测试用字符串 key 好断言。
    fn walk<'a>(n: &'a Node, c: Context<'_>, found: &mut Option<&'a str>) {
        if n.is_focused(c) {
            *found = n.key().and_then(|k| match k {
                crate::id::ElementKey::Name(s) => Some(*s),
                _ => None,
            });
        }
        for child in n.children() {
            walk(child, c, found);
        }
    }
    let mut found = None;
    walk(root, c, &mut found);
    found
}

// ─────────────────────────── Tab 顺序（深度优先） ───────────────────────────

#[test]
fn 深度优先收集可聚焦节点() {
    let root = tree();
    let c = ctx();
    let order = collect_focusable(&root, c);
    let names: Vec<&str> = order
        .iter()
        .filter_map(|k| match k {
            crate::id::ElementKey::Name(s) => Some(*s),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        ["a", "c", "d"],
        "顺序必须按文档序深度优先：a → c（b 的子）→ d"
    );
}

#[test]
fn 不可聚焦节点不进入顺序() {
    let root = tree();
    let c = ctx();
    let order = collect_focusable(&root, c);
    assert_eq!(order.len(), 3);
    assert!(!order
        .iter()
        .any(|k| matches!(k, crate::id::ElementKey::Name("b"))));
}

// ─────────────────────────────── 初始聚焦 ───────────────────────────────

#[test]
fn 没有焦点时_tab_聚焦第一个() {
    let mut root = tree();
    let mut fs = FocusState::new();
    assert!(fs.tab(&mut root, false, ctx()));
    let c = ctx();
    assert_eq!(focused_key(&root, c), Some("a"));
    assert_eq!(fs.focused(), Some(&crate::id::ElementKey::Name("a")));
}

#[test]
fn 没有焦点时_shift_tab_聚焦最后一个() {
    let mut root = tree();
    let mut fs = FocusState::new();
    assert!(fs.tab(&mut root, true, ctx()));
    assert_eq!(focused_key(&root, ctx()), Some("d"));
}

#[test]
fn tab_只写一个_focused() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    // 整棵树里必须恰好一个节点 is_focused。
    let c = ctx();
    let count = count_focused(&root, c);
    assert_eq!(count, 1, "焦点必须恰好落在一个节点上");
}

fn count_focused(root: &Node, c: Context<'_>) -> usize {
    fn walk(n: &Node, c: Context<'_>, acc: &mut usize) {
        if n.is_focused(c) {
            *acc += 1;
        }
        for child in n.children() {
            walk(child, c, acc);
        }
    }
    let mut acc = 0;
    walk(root, c, &mut acc);
    acc
}

// ─────────────────────────────── Tab 移动 ───────────────────────────────

#[test]
fn tab_前进_按文档序移动() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.tab(&mut root, false, ctx());
    assert_eq!(focused_key(&root, ctx()), Some("c"));
    fs.tab(&mut root, false, ctx());
    assert_eq!(focused_key(&root, ctx()), Some("d"));
}

#[test]
fn tab_循环_末端绕回第一个() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.tab(&mut root, false, ctx());
    fs.tab(&mut root, false, ctx());
    fs.tab(&mut root, false, ctx());
    // a → c → d → a（圈闭）
    assert_eq!(focused_key(&root, ctx()), Some("a"));
}

#[test]
fn shift_tab_反向移动() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, true, ctx()); // → d
    fs.tab(&mut root, true, ctx()); // → c
    assert_eq!(focused_key(&root, ctx()), Some("c"));
    fs.tab(&mut root, true, ctx()); // → a
    fs.tab(&mut root, true, ctx()); // → d（圈闭）
    assert_eq!(focused_key(&root, ctx()), Some("d"));
}

#[test]
fn tab_移动后_旧节点被清掉() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx()); // a
    let c = ctx();
    assert!(node_named(&root, "a").unwrap().is_focused(c));
    fs.tab(&mut root, false, ctx()); // c

    let c = ctx();
    let a = node_named(&root, "a").unwrap();
    assert!(!a.is_focused(c), "焦点移走后旧节点必须清掉 FOCUSED=false");
    assert!(node_named(&root, "c").unwrap().is_focused(c));
}

// ─────────────────────────────── 手动聚焦 ───────────────────────────────

#[test]
fn 手动聚焦到存在的可聚焦节点() {
    let mut root = tree();
    let mut fs = FocusState::new();
    let c = ctx();
    assert!(fs.focus(&mut root, Some(&crate::id::ElementKey::Name("c")), c));
    let c = ctx();
    assert!(node_named(&root, "c").unwrap().is_focused(c));
}

#[test]
fn 手动聚焦到不可聚焦或不存在节点_清空焦点() {
    let mut root = tree();
    let mut fs = FocusState::new();
    // 先聚焦到 a。
    fs.tab(&mut root, false, ctx());
    // 再聚焦到一个不可聚焦的节点（b）。
    fs.focus(&mut root, Some(&crate::id::ElementKey::Name("b")), ctx());
    let c = ctx();
    assert_eq!(count_focused(&root, c), 0, "聚焦到不可聚焦节点 = 清空焦点");
    assert!(fs.focused().is_none());
}

#[test]
fn 手动聚焦到不存在的key_清空焦点() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.focus(&mut root, Some(&crate::id::ElementKey::Name("nope")), ctx());
    let c = ctx();
    assert_eq!(count_focused(&root, c), 0);
}

#[test]
fn 手动聚焦会清掉旧的_tab_焦点() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx()); // a
    fs.focus(&mut root, Some(&crate::id::ElementKey::Name("d")), ctx());
    let c = ctx();
    assert!(node_named(&root, "d").unwrap().is_focused(c));
    assert!(!node_named(&root, "a").unwrap().is_focused(c));
}

// ─────────────────────────────── 无焦点树 ───────────────────────────────

#[test]
fn 没有可聚焦节点时_tab_不做事() {
    let el = Element::new(NodeTag::Container)
        .with_key("root")
        .with_children([
            Element::new(NodeTag::Control).with_key("x"),
            Element::new(NodeTag::Control).with_key("y"),
        ]);
    let mut root = Node::from_element(&el);
    let mut fs = FocusState::new();
    assert!(
        !fs.tab(&mut root, false, ctx()),
        "没有可聚焦节点，Tab 不应报已移动"
    );
    assert!(fs.focused().is_none());
    assert_eq!(count_focused(&root, ctx()), 0);
}

#[test]
fn 单节点树_tab_聚焦它_再_tab_绕回它自己() {
    let el = Element::new(NodeTag::Control)
        .with_key("only")
        .with(&FOCUSABLE, PropertyValue::bool(true));
    let mut root = Node::from_element(&el);
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    assert_eq!(focused_key(&root, ctx()), Some("only"));
    fs.tab(&mut root, false, ctx());
    assert_eq!(focused_key(&root, ctx()), Some("only"));
    assert_eq!(count_focused(&root, ctx()), 1);
}

// ─────────────────────────────── 清空 / 复位 ───────────────────────────────

#[test]
fn clear_清掉焦点并写回_false() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx()); // a
    fs.clear(Some(&mut root));
    let c = ctx();
    assert_eq!(
        count_focused(&root, c),
        0,
        "clear 后树里不能残留 focused=true"
    );
    assert!(fs.focused().is_none());
}

#[test]
fn clear_后重新_tab_从头开始() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx()); // a
    fs.clear(Some(&mut root));
    fs.tab(&mut root, false, ctx()); // 应该重新从 a 开始，而不是跳到 c
    assert_eq!(focused_key(&root, ctx()), Some("a"));
}

#[test]
fn 焦点在子节点_clear_后父树的_focused_也清零() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.tab(&mut root, false, ctx()); // c（在 b 之下）
    let c = ctx();
    assert!(node_named(&root, "c").unwrap().is_focused(c));
    fs.clear(Some(&mut root));
    let c = ctx();
    assert_eq!(count_focused(&root, c), 0);
}

// ─────────────────────────────── revision ───────────────────────────────

#[test]
fn revision_在焦点变化时递增() {
    let mut root = tree();
    let mut fs = FocusState::new();
    let r0 = fs.revision();
    fs.tab(&mut root, false, ctx());
    assert!(fs.revision() > r0, "Tab 移动后 revision 必须递增");
    let r1 = fs.revision();
    fs.tab(&mut root, false, ctx());
    assert!(fs.revision() > r1);
}

#[test]
fn revision_在无焦点树_tab_时不递增() {
    let el = Element::new(NodeTag::Container).with_key("root");
    let mut root = Node::from_element(&el);
    let mut fs = FocusState::new();
    let r0 = fs.revision();
    fs.tab(&mut root, false, ctx());
    assert_eq!(fs.revision(), r0, "没发生焦点变化，revision 不该动");
}

// ─────────────────────────────── 帮助函数 ───────────────────────────────

fn node_named<'a>(root: &'a Node, name: &str) -> Option<&'a Node> {
    if let Some(key) = root.key() {
        if matches!(key, crate::id::ElementKey::Name(n) if *n == name) {
            return Some(root);
        }
    }
    for child in root.children() {
        if let Some(found) = node_named(child, name) {
            return Some(found);
        }
    }
    None
}
