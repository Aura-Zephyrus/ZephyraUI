//! 动画接入层的单测：预制解析、progress→帧、paint 属性写入。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_ui::animation::{AnimDriver, PREFADE_SOURCES};

#[test]
fn all_prefabs_parse_and_produce_a_frame() {
    let driver = AnimDriver::new();
    assert_eq!(PREFADE_SOURCES.len(), 6);
    for (id, _) in PREFADE_SOURCES {
        let frame = driver
            .frame(id, 0.5)
            .unwrap_or_else(|| panic!("{id} 应能取帧"));
        assert!(!frame.paths.is_empty(), "{id} 应至少有一条路径");
    }
}

#[test]
fn driver_writes_only_paint_properties() {
    let driver = AnimDriver::new();
    let element = Element::new(NodeTag::Box)
        .with(&props::ANIMATION, PropertyValue::StaticStr("spinner"))
        .with(&props::ANIMATION_PROGRESS, PropertyValue::float(0.5));
    let mut node = Node::from_element(&element);

    driver.update(&mut node);

    assert!(
        node.props().lookup(&props::PATH_DATA).is_some(),
        "动画帧应写入 PATH_DATA"
    );
    assert!(
        node.props().lookup(&props::OPACITY).is_some(),
        "动画帧应写入 OPACITY"
    );
    // 布局属性一个都不许被动画写：affects_layout 门禁在 core 侧也钉着。
    assert!(node.props().lookup(&props::WIDTH).is_none());
    assert!(node.props().lookup(&props::HEIGHT).is_none());
}

#[test]
fn paused_state_keeps_the_node_untouched() {
    let driver = AnimDriver::new();
    let element = Element::new(NodeTag::Box)
        .with(&props::ANIMATION, PropertyValue::StaticStr("spinner"))
        .with(&props::ANIMATION_PROGRESS, PropertyValue::float(0.5))
        .with(&props::ANIMATION_PLAY_STATE, PropertyValue::bool(false));
    let mut node = Node::from_element(&element);

    driver.update(&mut node);

    assert!(
        node.props().lookup(&props::PATH_DATA).is_none(),
        "play-state=false 时不应写任何动画属性"
    );
}

#[test]
fn paused_animation_survives_the_next_diff() {
    // 钉住冲突 1：动画帧写在 styled 层，diff 的 retain_local_props 不会拆掉它。
    // 若写回 local 层，下一帧 diff 会清空 PATH_DATA，暂停动画直接变空白。
    use zephyra_core::diff::diff;

    let driver = AnimDriver::new();
    let element = Element::new(NodeTag::Box)
        .with(&props::ANIMATION, PropertyValue::StaticStr("spinner"))
        .with(&props::ANIMATION_PROGRESS, PropertyValue::float(0.5));
    let mut node = Node::from_element(&element);
    driver.update(&mut node);
    assert!(
        node.props().lookup(&props::PATH_DATA).is_some(),
        "第一帧应写出 PATH_DATA"
    );

    // 下一帧：声明树仍没有 PATH_DATA，diff 对账；随后暂停的钩子不写任何值。
    let paused = Element::new(NodeTag::Box)
        .with(&props::ANIMATION, PropertyValue::StaticStr("spinner"))
        .with(&props::ANIMATION_PROGRESS, PropertyValue::float(0.5))
        .with(&props::ANIMATION_PLAY_STATE, PropertyValue::bool(false));
    let _ = diff(&mut node, &paused);
    driver.update(&mut node);

    assert!(
        node.props().lookup(&props::PATH_DATA).is_some(),
        "跨过一次 diff 后，暂停动画的 PATH_DATA 必须还在"
    );
}
