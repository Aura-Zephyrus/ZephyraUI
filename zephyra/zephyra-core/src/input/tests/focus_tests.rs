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
    let el = Element::new(NodeTag::Box).with_key("root").with_children([
        Element::new(NodeTag::Control)
            .with_key("a")
            .with(&FOCUSABLE, PropertyValue::bool(true)),
        Element::new(NodeTag::Box)
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
fn collects_focusable_nodes_depth_first() {
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
fn non_focusable_nodes_excluded() {
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
fn tab_without_focus_focuses_first() {
    let mut root = tree();
    let mut fs = FocusState::new();
    assert!(fs.tab(&mut root, false, ctx()));
    let c = ctx();
    assert_eq!(focused_key(&root, c), Some("a"));
    assert_eq!(fs.focused(), Some(&crate::id::ElementKey::Name("a")));
}

#[test]
fn shift_tab_without_focus_focuses_last() {
    let mut root = tree();
    let mut fs = FocusState::new();
    assert!(fs.tab(&mut root, true, ctx()));
    assert_eq!(focused_key(&root, ctx()), Some("d"));
}

#[test]
fn tab_writes_exactly_one_focused() {
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
fn tab_moves_forward_in_document_order() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.tab(&mut root, false, ctx());
    assert_eq!(focused_key(&root, ctx()), Some("c"));
    fs.tab(&mut root, false, ctx());
    assert_eq!(focused_key(&root, ctx()), Some("d"));
}

#[test]
fn tab_wraps_from_last_to_first() {
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
fn shift_tab_moves_backward() {
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
fn tab_clears_previous_node() {
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
fn manual_focus_on_focusable_node() {
    let mut root = tree();
    let mut fs = FocusState::new();
    let c = ctx();
    assert!(fs.focus(&mut root, Some(&crate::id::ElementKey::Name("c")), c));
    let c = ctx();
    assert!(node_named(&root, "c").unwrap().is_focused(c));
}

#[test]
fn manual_focus_on_unfocusable_or_missing_clears() {
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
fn manual_focus_on_missing_key_clears() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.focus(&mut root, Some(&crate::id::ElementKey::Name("nope")), ctx());
    let c = ctx();
    assert_eq!(count_focused(&root, c), 0);
}

#[test]
fn manual_focus_clears_old_tab_focus() {
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
fn tab_without_focusable_does_nothing() {
    let el = Element::new(NodeTag::Box).with_key("root").with_children([
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
fn single_node_tab_focuses_and_wraps_to_self() {
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
fn clear_removes_focus_and_writes_false() {
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
fn tab_after_clear_starts_over() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx()); // a
    fs.clear(Some(&mut root));
    fs.tab(&mut root, false, ctx()); // 应该重新从 a 开始，而不是跳到 c
    assert_eq!(focused_key(&root, ctx()), Some("a"));
}

#[test]
fn clear_also_clears_focused_in_subtree() {
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
fn revision_increments_on_focus_change() {
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
fn revision_stable_when_tab_finds_nothing() {
    let el = Element::new(NodeTag::Box).with_key("root");
    let mut root = Node::from_element(&el);
    let mut fs = FocusState::new();
    let r0 = fs.revision();
    fs.tab(&mut root, false, ctx());
    assert_eq!(fs.revision(), r0, "没发生焦点变化，revision 不该动");
}

// ───────────────────────── 焦点环可见性（focus-visible） ─────────────────────────

#[test]
fn tab_focus_shows_the_ring() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    assert!(fs.is_visible(), "Tab 引入的焦点必须让焦点环可见");
}

#[test]
fn mouse_focus_does_not_show_the_ring() {
    let mut root = tree();
    let mut fs = FocusState::new();
    let c = ctx();
    fs.tab(&mut root, false, ctx()); // 先让环可见
    assert!(fs.is_visible());
    // 鼠标点同一节点：焦点不变，但环必须收掉。
    fs.focus(&mut root, Some(&crate::id::ElementKey::Name("a")), c);
    assert!(fs.focused().is_some(), "鼠标点击仍然设置焦点（行为未变）");
    assert!(!fs.is_visible(), "鼠标点击不该留下焦点环");
    assert!(
        node_named(&root, "a").unwrap().is_focused(ctx()),
        "FOCUSED 属性照常写入（激活 / 无障碍读它）"
    );
}

#[test]
fn clicking_the_same_focused_node_still_clears_the_ring_and_bumps_revision() {
    let mut root = tree();
    let mut fs = FocusState::new();
    let c = ctx();
    fs.tab(&mut root, false, ctx()); // a，可见
    let r = fs.revision();
    fs.focus(&mut root, Some(&crate::id::ElementKey::Name("a")), c);
    assert!(fs.revision() > r, "可见性变化必须让宿主知道要重绘");
}

#[test]
fn clear_also_hides_the_ring() {
    let mut root = tree();
    let mut fs = FocusState::new();
    fs.tab(&mut root, false, ctx());
    fs.clear(Some(&mut root));
    assert!(!fs.is_visible());
}

#[test]
fn no_focus_means_ring_not_visible() {
    let fs = FocusState::new();
    assert!(!fs.is_visible());
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
