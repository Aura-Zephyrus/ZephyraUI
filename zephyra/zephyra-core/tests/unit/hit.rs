//! 命中测试（[`crate::hit`]）的行为断言。
//!
//! 这批测试补的是骨架阶段留下的洞：原 `hit.rs` 从不把任何节点标成 `interactive`
//! （`HitNode.interactive` 恒为 `false`），也就是说命中测试永远答不出"点到了哪个
//! 控件"。更根本的是它**没人调用**——而 hover/click 正是保留式 UI 的地基。
//!
//! 每条断言都对应一个"曾经会静默出错"的具体故障。
//!
//! ⚠ 原本这些断言与路由断言混在 `src/input_tests.rs` 里。拆开是为了**按模块
//! 定位**：改命中只跑 `cargo test -p zephyra-core hit`，改路由只跑
//! `cargo test -p zephyra-core input::router`。

use crate::core::{Point, Rect};
use crate::element::Element;
use crate::hit::{hit_test, HitResult, HitTestRequest, MAX_DEPTH};
use crate::id::NodeTag;
use crate::layout::{layout, Direction, Position};
use crate::node::Node;
use crate::property::PropertyValue;
use crate::props;
use crate::test_support::{ctx, hitbox, tree, two_buttons};

/// 在某个点做一次命中测试。
fn hit_at(node: &Node, x: f32, y: f32) -> HitResult {
    let mut out = HitResult::default();
    hit_test(
        node,
        HitTestRequest {
            point: Point::new(x, y),
        },
        ctx(),
        &mut out,
    );
    out
}

#[test]
fn an_interactive_node_is_actually_reported_as_interactive() {
    // 这条钉的是那个恒为 false 的 bug：`interactive` 永远假 ⇒ target() 永远 None
    // ⇒ 所有控件都收不到点击，而且没有任何报错。
    let node = tree(&two_buttons());
    let result = hit_at(&node, 10.0, 10.0);
    let target = result.target().expect("落在可交互按钮上时必须有 target");
    assert_eq!(target.id, Some(crate::id::ElementKey::Name("a")));
    assert!(target.interactive);
}

#[test]
fn a_container_is_on_the_path_but_is_not_the_target() {
    // 事件冒泡需要容器的矩形，但处理目标必须是按钮本身。
    let node = tree(&two_buttons());
    let result = hit_at(&node, 10.0, 10.0);
    assert!(result
        .path
        .iter()
        .any(|n| n.id == Some(crate::id::ElementKey::Name("root"))));
    assert_eq!(
        result.target().map(|n| &n.id),
        Some(&Some(crate::id::ElementKey::Name("a")))
    );
}

#[test]
fn sibling_rects_never_overlap_under_the_current_layout() {
    // 这条是给**将来**埋的雷：目前 `Stack` 布局按顺序摆放兄弟，所以兄弟矩形永不相交，
    // 命中测试不存在"多个候选"的歧义。
    //
    // 一旦加入绝对定位或浮层，兄弟就会重叠，那时 `descend` 里"反向遍历 = 后加入的
    // 在上层"这条约定才开始承重。这条断言会在那时失败——那正是它存在的意义：
    // 提醒实现者"重叠下的 z 序现在有人依赖了"。
    let node = tree(&two_buttons());
    let rects: Vec<Rect> = node.children().iter().map(|c| c.rect()).collect();
    assert!(
        rects[0].right() <= rects[1].x || rects[1].right() <= rects[0].x,
        "Stack 布局下兄弟不应重叠，实得 {rects:?}"
    );
}

#[test]
fn a_point_in_the_root_but_on_no_control_has_no_target() {
    // 复用 `out` 的关键风险：上一次命中的路径残留下来，被下一次误读为命中了控件。
    //
    // 注意 (150,150) 仍在根矩形内，所以 `hit_any()` 为真——"命中了容器"与
    // "命中了控件"是两回事，混为一谈就会把点击吞掉。
    let node = tree(&two_buttons());
    let mut out = HitResult::default();
    assert!(hit_test(
        &node,
        HitTestRequest {
            point: Point::new(10.0, 10.0),
        },
        ctx(),
        &mut out
    ));
    assert!(out.target().is_some());

    assert!(hit_test(
        &node,
        HitTestRequest {
            point: Point::new(150.0, 150.0),
        },
        ctx(),
        &mut out
    ));
    assert!(
        out.target().is_none(),
        "落在容器空白处不应命中任何控件，实得 {:?}",
        out.path
    );
}

#[test]
fn absolute_child_is_hittable_even_when_parent_collapsed() {
    // 弹出菜单的形状：菜单项 `position:absolute` 按窗口坐标摆位，父容器因为
    // 子项脱离正常流而塌成 0×0。父矩形不含点**不能**代表整棵子树不可命中，
    // 否则菜单画得出来却点不到——症状是「点了没反应」，而画面毫无异常。
    let slot = Element::new(NodeTag::Box)
        .with_key("slot")
        .with(&props::WIDTH, PropertyValue::float(0.0))
        .with(&props::HEIGHT, PropertyValue::float(0.0))
        .with_children([hitbox("menu", 30.0, 20.0)
            .with(&props::POSITION, PropertyValue::position(Position::Root))
            .with(
                &props::ANCHOR,
                PropertyValue::anchor(crate::layout::Anchor::Center),
            )
            .with(&props::LEFT, PropertyValue::float(120.0))
            .with(&props::TOP, PropertyValue::float(40.0))]);
    let root = Element::new(NodeTag::Box).with_children([slot]);
    let node = tree(&root);

    let mut out = HitResult::default();
    assert!(hit_test(
        &node,
        HitTestRequest {
            point: Point::new(120.0, 40.0),
        },
        ctx(),
        &mut out
    ));
    let target = out.target().expect("绝对定位子项必须可命中");
    assert_eq!(
        target.id,
        Some(crate::id::ElementKey::Name("menu")),
        "命中的应是菜单项而不是塌陷的父容器"
    );
}

#[test]
fn a_point_outside_the_root_leaves_the_buffer_empty() {
    let node = tree(&two_buttons());
    let mut out = HitResult::default();
    assert!(!hit_test(
        &node,
        HitTestRequest {
            point: Point::new(500.0, 500.0),
        },
        ctx(),
        &mut out
    ));
    assert!(
        out.path.is_empty(),
        "完全未命中时路径必须被清空（供跨帧复用），实得 {:?}",
        out.path
    );
}

#[test]
fn collapsed_nodes_are_not_hittable() {
    // 折叠 = 不占位不绘制。若仍可命中，会出现"看不见但点得到"的幽灵控件。
    let e = two_buttons();
    let mut node = tree(&e);
    node.children_mut()[1].set_prop(&props::COLLAPSED, PropertyValue::bool(true));
    let result = hit_at(&node, 60.0, 10.0);
    assert!(result.target().is_none(), "折叠的按钮不该被命中");
}

#[test]
fn a_deep_tree_truncates_but_still_answers_correctly() {
    // 超过 MAX_DEPTH 时路径被截断，但"命中了谁"必须照常给出——
    // 截断只该影响冒泡链，不该影响答案。
    let mut element = hitbox("deep", 100.0, 100.0);
    for _ in 0..(MAX_DEPTH + 5) {
        element = Element::new(NodeTag::Box)
            .with_key("l")
            .with(
                &props::DIRECTION,
                PropertyValue::direction(Direction::Vertical),
            )
            .with(&props::PADDING, PropertyValue::float(0.0))
            .with_children([element]);
    }
    let mut node = Node::from_element(&element);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 200.0), ctx());
    let result = hit_at(&node, 50.0, 50.0);
    assert!(result.truncated, "超深应标记为截断");
    assert!(result.hit_any());
}

// ============================== 滚动容器 ==============================

/// 造一棵滚动容器：视口 100×100，内容 100×150（两项 + 50 间距）。
///
/// 布局（纵��）：`item1` 占 0..50，`item2` 占 100..150。
/// 滚动偏移 `offset_y` 把**内容**整体上移 —— 于是 `item2` 的命中矩形
/// 变成 100-offset .. 150-offset。
fn scroll_tree(offset_y: f32) -> Node {
    let item = |id: &'static str| {
        Element::new(NodeTag::Control)
            .with_key(id)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .with(&props::WIDTH, PropertyValue::float(100.0))
            .with(&props::HEIGHT, PropertyValue::float(50.0))
    };
    let content = Element::new(NodeTag::Box)
        .with_key("content")
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Vertical),
        )
        .with(&props::SPACING, PropertyValue::float(50.0))
        .with_children([item("item1"), item("item2")]);
    let scroll = Element::new(NodeTag::Box)
        .with_key("scroll")
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Vertical),
        )
        .with(&props::WIDTH, PropertyValue::float(100.0))
        .with(&props::HEIGHT, PropertyValue::float(100.0))
        .with(
            &props::OVERFLOW,
            PropertyValue::overflow(crate::layout::Overflow::Scroll),
        )
        .with(&props::SCROLL_OFFSET_Y, PropertyValue::float(offset_y))
        .with_children([content]);
    let root = Element::new(NodeTag::Box).with_children([scroll]);
    let mut node = Node::from_element(&root);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 200.0), ctx());
    node
}

/// 滚过的内容按**滚后的位置**命中。
///
/// # 这条在钉什么
///
/// 命中测试必须与绘制用**同一条**平移规则（内容按 `-scroll_offset` 平移）。
/// 若命中测试忘了反向平移，症状是「滚下去之后点不到那一项」——
/// 而画面上明明有一项在那儿。
#[test]
fn scrolled_content_is_hittable_at_its_shifted_position() {
    let node = scroll_tree(50.0);
    // 滚了 50：`item2` 从 100..150 变成 50..100，落在视口内。
    let result = hit_at(&node, 10.0, 75.0);
    assert_eq!(
        result.target().map(|n| &n.id),
        Some(&Some(crate::id::ElementKey::Name("item2"))),
        "滚过之后应命中滚后位置上的那一项"
    );
    // 没滚过的位置（100..150）此刻是空的 —— 命中测试不能还按老位置答。
    let old = hit_at(&node, 10.0, 125.0);
    assert!(
        old.target().is_none(),
        "滚后的旧位置不该再命中：{:?}",
        old.target().map(|n| n.id.clone())
    );
}

/// 视口外的内容**不可命中**（它已经被裁掉了）。
///
/// # 为什么这条重要
///
/// 绘制趟把视口外的内容裁掉，命中测试若不裁，就会出现「看不见却点得到」：
/// 用户点一个看不见的按钮，应用却执行了它的动作。
#[test]
fn content_clipped_by_the_scroll_viewport_is_not_hittable() {
    let node = scroll_tree(0.0);
    // `item2` 在 100..150，视口只有 0..100 —— 它被裁掉了。
    let result = hit_at(&node, 10.0, 125.0);
    assert!(
        result.target().is_none(),
        "视口外被裁掉的内容不该可命中：{:?}",
        result.target().map(|n| n.id.clone())
    );
    // 视口内的 `item1` 仍然可命中。
    let inside = hit_at(&node, 10.0, 25.0);
    assert_eq!(
        inside.target().map(|n| &n.id),
        Some(&Some(crate::id::ElementKey::Name("item1"))),
        "视口内的内容必须照常可命中"
    );
}

/// 滚动容器**自身**的盒子不跟着内容滚。
///
/// 症状（若错了）：滚一下，容器的背景 / 边框跟着往上跑 —— 用户看到的是
/// 「整个面板在动」而不是「内容在动」。
#[test]
fn the_scroll_viewport_itself_does_not_move_with_the_content() {
    let node = scroll_tree(50.0);
    let scroll = node
        .children()
        .iter()
        .find(|n| n.key().and_then(|k| k.as_name()) == Some("scroll"))
        .expect("树里应当有 scroll 容器");
    assert_eq!(
        scroll.rect(),
        Rect::new(0.0, 0.0, 100.0, 100.0),
        "滚动容器的盒子（视口）必须留在原位"
    );
}
