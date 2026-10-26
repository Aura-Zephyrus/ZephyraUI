use zephyra_core::context::dark_context;
use zephyra_core::core::Rect;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::layout::layout;
use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{node_at_path, tree_dump_of};

#[test]
fn tree_dump_keeps_nested_rectangles_in_absolute_coordinates() {
    let root = Element::new(NodeTag::Container)
        .with(
            &props::DIRECTION,
            PropertyValue::direction(zephyra_core::layout::Direction::Horizontal),
        )
        .with_children([
            Element::new(NodeTag::Container)
                .with(&props::WIDTH, PropertyValue::float(20.0))
                .with(&props::HEIGHT, PropertyValue::float(100.0)),
            Element::new(NodeTag::Container)
                .with(&props::WIDTH, PropertyValue::float(100.0))
                .with(&props::HEIGHT, PropertyValue::float(100.0))
                .with_children([Element::new(NodeTag::Container)
                    .with(&props::WIDTH, PropertyValue::float(10.0))
                    .with(&props::HEIGHT, PropertyValue::float(10.0))]),
        ]);
    let mut node = Node::from_element(&root);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), dark_context());

    assert!(
        tree_dump_of(&node).contains("0.1.0 Container - 20,0 10x10"),
        "nested node coordinates must not add the parent's offset twice"
    );
    assert_eq!(
        node_at_path(&node, &[0, 1, 0])
            .expect("tree path must address the nested node")
            .arranged_rect()
            .x,
        20.0
    );
    assert!(node_at_path(&node, &[1]).is_none());
}
