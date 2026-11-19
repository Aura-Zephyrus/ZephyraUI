//! P1.2 / P0.3 纯逻辑门禁。

use zephyra_core::core::Rect;
use zephyra_core::id::ElementKey;

use super::{InputState, View};

#[test]
fn rect_of_finds_snapshotted_key_and_returns_none_for_unknown() {
    let key = ElementKey::Name("slider");
    let state = InputState {
        rects: vec![(key.clone(), Rect::new(10.0, 20.0, 220.0, 36.0))],
        ..InputState::default()
    };
    assert_eq!(
        state.rect_of(&key),
        Some(Rect::new(10.0, 20.0, 220.0, 36.0))
    );
    assert_eq!(state.rect_of(&ElementKey::Name("missing")), None);
}

/// 默认空实现：现有 `View` 实现者无需改任何代码。
#[test]
fn view_default_wake_hooks_are_noops() {
    struct Minimal;
    impl View for Minimal {
        fn on_input(&mut self, _state: &InputState) {}
        fn view(
            &self,
            _ctx: &zephyra_core::context::Context<'_>,
        ) -> zephyra_core::element::Element {
            zephyra_core::element::Element::new(zephyra_core::id::NodeTag::Box)
        }
    }
    let mut v = Minimal;
    v.on_wake(42);
    v.on_message(&mut 7u32);
}
