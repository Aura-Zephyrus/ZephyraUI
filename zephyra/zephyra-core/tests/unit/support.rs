//! 测试共用的构件（`cfg(test)`）。
//!
//! # 为什么单独一个文件
//!
//! 命中测试（[`crate::hit`]）与输入路由（[`crate::input::router`]）都要
//! 「一棵摆好的树」与「一个可交互的方块」。这两处以前共用
//! `src/input_tests.rs` 里的辅助函数 —— 那份文件后来长成了**混着两类断言**
//! 的大文件，改一处路由要重跑命中测试。
//!
//! 拆开之后每个测试文件只依赖它**自己那摊**的辅助函数，而跨模块共用的
//! 那几个（树 / 方块 / 上下文）住在这里。
//!
//! ⚠ 本模块**只在测试构建里存在**（`lib.rs` 里是 `#[cfg(test)] mod
//! test_support;`）—— 生产二进制里一个字节都不带。

use crate::context::Context;
use crate::core::{Color, Rect};
use crate::element::Element;
use crate::id::NodeTag;
use crate::input::pointer::{PointerEvent, PointerPhase, PointerSource};
use crate::input::router::{InputRouter, RoutedEvent, RoutedEvents};
use crate::layout::{layout, Direction};
use crate::node::Node;
use crate::property::PropertyValue;
use crate::props;

/// 深色上下文（测布局与命中都用同一个，省得每次重造）。
pub(crate) fn ctx() -> Context<'static> {
    crate::context::dark_context()
}

/// 一个可交互的矩形节点。
pub(crate) fn hitbox(id: &'static str, w: f32, h: f32) -> Element {
    Element::new(NodeTag::Control)
        .with_key(id)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .with(&props::WIDTH, PropertyValue::float(w))
        .with(&props::HEIGHT, PropertyValue::float(h))
        .with(
            &props::BACKGROUND,
            PropertyValue::color(Color::rgb(0.2, 0.2, 0.2)),
        )
}

/// 一行两个按钮：a 在左（0..40），b 在右（40..80）。
pub(crate) fn two_buttons() -> Element {
    Element::new(NodeTag::Box)
        .with_key("root")
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Horizontal),
        )
        .with(&props::SPACING, PropertyValue::float(0.0))
        .with(&props::PADDING, PropertyValue::float(0.0))
        .with_children([hitbox("a", 40.0, 40.0), hitbox("b", 40.0, 40.0)])
}

/// 摆好一棵 200×200 的树。
pub(crate) fn tree(element: &Element) -> Node {
    let mut node = Node::from_element(element);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 200.0), ctx());
    node
}

/// 一个**鼠标**指针事件（时刻按 id 递增，够计时判据用）。
pub(crate) fn ev(id: u64, x: f32, y: f32, phase: PointerPhase) -> PointerEvent {
    PointerEvent {
        id,
        x,
        y,
        phase,
        source: PointerSource::Mouse,
        time_ms: id * 16,
    }
}

/// 路由一个指针事件，返回它产出的事件。
pub(crate) fn route(router: &mut InputRouter, node: &Node, e: PointerEvent) -> Vec<RoutedEvent> {
    let mut out = RoutedEvents::new();
    router.on_pointer(node, e, ctx(), &mut out);
    out.iter().cloned().collect()
}
