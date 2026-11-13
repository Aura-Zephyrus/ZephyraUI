//! 本文件由 `zephyra::zephyra-core::src::visual.rs` 的 `#[cfg(test)] mod tests` 抽出 —— 测试代码与生产代码分离。
//!
//! 模块路径不变（生产文件里仍用 `#[path = ...]` 声明），所以 `use super::*`
//! 与对父模块私有项的访问照旧。

use super::*;
use crate::context::dark_context;
use crate::element::Element;
use crate::id::NodeTag;
use crate::layout::layout;
use crate::property::PropertyValue;
use crate::props;

#[test]
fn root_position_clears_ancestor_translate_for_visual_bounds() {
    let root = Element::new(NodeTag::Box)
        .with(&props::TRANSLATE_X, PropertyValue::float(50.0))
        .with_children([Element::new(NodeTag::Box)
            .with(&props::WIDTH, PropertyValue::float(10.0))
            .with(&props::HEIGHT, PropertyValue::float(10.0))
            .with(
                &props::POSITION,
                PropertyValue::position(crate::layout::Position::Root),
            )
            .with(&props::LEFT, PropertyValue::float(5.0))
            .with(&props::TOP, PropertyValue::float(7.0))]);
    let mut node = Node::from_element(&root);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), dark_context());
    let child = &node.children()[0];
    // Parent translate (50) must not affect root-positioned child (D11).
    let (state, tr) = enter(&node, dark_context(), VisualState::ROOT);
    let (child_state, ctr) = enter(child, dark_context(), state);
    let _ = child_state;
    let vr = visual_rect(child, dark_context(), ctr);
    assert!((vr.x - 5.0).abs() < 0.001, "x={}", vr.x);
    assert!((vr.y - 7.0).abs() < 0.001, "y={}", vr.y);
    assert!((tr.x - 50.0).abs() < 0.001);
}
