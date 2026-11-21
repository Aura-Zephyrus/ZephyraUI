//! frameless 窗口拖动 / 边缘缩放的纯逻辑测试（不碰 GPU / 窗口）。

use super::{hit_drag_region, resize_edge, ResizeEdge};
use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props::{HEIGHT, WIDTH, WINDOW_DRAG};
use zephyra_core::theme::{Theme, ThemeVariant};

fn ctx() -> Context<'static> {
    Context::new(Box::leak(Box::new(Theme::builtin(ThemeVariant::Dark))))
}

#[test]
fn resize_edge_detects_all_eight_directions() {
    let (w, h, i) = (100.0, 80.0, 4.0);
    assert_eq!(resize_edge(50.0, 1.0, w, h, i), Some(ResizeEdge::North));
    assert_eq!(resize_edge(50.0, 79.0, w, h, i), Some(ResizeEdge::South));
    assert_eq!(resize_edge(1.0, 40.0, w, h, i), Some(ResizeEdge::West));
    assert_eq!(resize_edge(99.0, 40.0, w, h, i), Some(ResizeEdge::East));
    assert_eq!(resize_edge(1.0, 1.0, w, h, i), Some(ResizeEdge::NorthWest));
    assert_eq!(resize_edge(99.0, 1.0, w, h, i), Some(ResizeEdge::NorthEast));
    assert_eq!(resize_edge(1.0, 79.0, w, h, i), Some(ResizeEdge::SouthWest));
    assert_eq!(
        resize_edge(99.0, 79.0, w, h, i),
        Some(ResizeEdge::SouthEast)
    );
    assert_eq!(resize_edge(50.0, 40.0, w, h, i), None);
}

#[test]
fn hit_drag_region_marks_titlebar() {
    let root = Element::new(NodeTag::Box).with_children([Element::new(NodeTag::Box)
        .with_key("titlebar")
        .with(&WINDOW_DRAG, PropertyValue::bool(true))
        .with(&WIDTH, PropertyValue::float(100.0))
        .with(&HEIGHT, PropertyValue::float(20.0))]);
    let mut node = Node::from_element(&root);
    zephyra_core::layout::layout(
        &mut node,
        zephyra_core::core::Rect::new(0.0, 0.0, 100.0, 100.0),
        ctx(),
    );
    assert!(hit_drag_region(&node, 50.0, 1.0, ctx()));
}

#[test]
fn hit_drag_region_child_button_is_exempt() {
    // 按钮嵌在标题栏里但没标 WINDOW_DRAG：点按钮不该拖窗口。
    let element = Element::new(NodeTag::Box).with_children([Element::new(NodeTag::Box)
        .with_key("titlebar")
        .with(&WINDOW_DRAG, PropertyValue::bool(true))
        .with(&WIDTH, PropertyValue::float(100.0))
        .with(&HEIGHT, PropertyValue::float(20.0))
        .with_children([Element::new(NodeTag::Control)
            .with_key("close")
            .with(&WIDTH, PropertyValue::float(20.0))
            .with(&HEIGHT, PropertyValue::float(20.0))])]);
    let mut node = Node::from_element(&element);
    zephyra_core::layout::layout(
        &mut node,
        zephyra_core::core::Rect::new(0.0, 0.0, 100.0, 100.0),
        ctx(),
    );
    // 点在标题栏空白处 ⇒ 拖动区。
    assert!(hit_drag_region(&node, 50.0, 10.0, ctx()));
    // 点在最内层的 close 按钮上 ⇒ 它不是拖动区（子元素豁免）。
    assert!(!hit_drag_region(&node, 10.0, 10.0, ctx()));
}
