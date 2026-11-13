//! 输入路由（[`crate::input::router`]）的行为断言。
//!
//! 命中测试在 `tests/unit/hit.rs`；触控语义在 `tests/unit/input/touch.rs`。
//! 这里只管**鼠标**路径：悬停 / 按下 / 抬起 / 点击 / 复位。
//!
//! ⚠ 原本这些断言与命中断言混在 `src/input_tests.rs` 里。拆开是为了**按模块
//! 定位**：改路由只跑 `cargo test -p zephyra-core input::router`，改命中只跑
//! `cargo test -p zephyra-core hit`。

use crate::input::pointer::PointerPhase;
use crate::input::router::{InputRouter, RoutedEvent};
use crate::test_support::{ev, route, tree, two_buttons};

#[test]
fn pressing_and_releasing_both_bump_the_revision() {
    // 宿主靠 `revision` 变化决定「这一帧要不要重绘」，而按下态与 click 之后
    // 的状态变化**都是画出来的**。漏掉 `revision++` 时症状极其安静：
    //
    // - Down 不 bump ⇒ 按下时没有按下样式，鼠标一动才出现；
    // - Up 不 bump ⇒ **第一次点击「没反应」**，把鼠标移开再移回来才生效。
    //
    // 第二条最难查：画面没有任何异常（该亮的亮、该画的画），只是「少了一帧」。
    // 判据刻意不让 hover 同时变化（鼠标不动）—— 那正是点按钮时的常态。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();

    // 先悬停上去，让 hover 稳定下来。
    route(&mut router, &node, ev(0, 10.0, 10.0, PointerPhase::Move));
    let before_down = router.revision();

    route(&mut router, &node, ev(0, 10.0, 10.0, PointerPhase::Down));
    assert_ne!(
        router.revision(),
        before_down,
        "按下必须 bump revision（否则按下态没有样式，且宿主不重绘）"
    );

    let before_up = router.revision();
    route(&mut router, &node, ev(0, 10.0, 10.0, PointerPhase::Up));
    assert_ne!(
        router.revision(),
        before_up,
        "抬起必须 bump revision —— click 之后应用改了状态，不 bump 就整帧不重绘"
    );
}

// ============================== 路由：悬停 ==============================

#[test]
fn moving_onto_a_button_changes_the_revision() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    let before = router.revision();

    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    assert_eq!(
        router.hovered().and_then(|k| k.as_name()),
        Some("a"),
        "指针应悬停在 a 上"
    );
    assert_ne!(router.revision(), before, "悬停变化必须递增 revision");
}

#[test]
fn moving_between_buttons_keeps_the_same_revision_for_one_event() {
    // 一次 Move 内部完成了 a→b 的切换；revision 只该 +1（一个状态变更），
    // 而事件流里不该出现两条 hover 事件——宿主比较 revision 就够了。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    let before = router.revision();
    route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Move));
    assert_eq!(router.hovered().and_then(|k| k.as_name()), Some("b"));
    assert_eq!(
        router.revision(),
        before + 1,
        "一次移动只应产生一次状态变更"
    );
}

#[test]
fn hovering_blank_space_clears_the_hover() {
    // 指针移出所有按钮后，高亮必须消失——否则按钮会"粘"在亮起状态。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Move));
    assert!(router.hovered().is_none());
}

#[test]
fn the_same_position_does_not_bump_the_revision() {
    // 静止时指针可能仍送来 Move。若每次都 +1，宿主就会永远重绘。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    let before = router.revision();
    for _ in 0..5 {
        route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    }
    assert_eq!(router.revision(), before, "位置未变就不该要求重绘");
}

// ============================== 路由：按下/抬起/点击 ==============================

#[test]
fn press_and_release_on_the_same_button_produce_a_click() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    let mut events = route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    assert!(matches!(events.first(), Some(RoutedEvent::Press(_))));
    events = route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Up));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RoutedEvent::Click(k) if k.as_name() == Some("a"))),
        "同点按下抬起应产生 click，实得 {events:?}"
    );
}

#[test]
fn dragging_off_before_release_does_not_click() {
    // 这条是最容易出真 bug 的一条：按下 a、拖出去、在空白处松手。
    // 若按"抬起时命中谁"来判定，就会误触发——而用户根本没点它。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    let events = route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Up));
    assert!(
        !events.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "拖出后松手不得触发 click，实得 {events:?}"
    );
}

#[test]
fn dragging_from_a_onto_b_does_not_click_b() {
    // 捕获语义的核心断言：按下时是谁，就归谁。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    let events = route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Up));
    assert!(
        !events.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "a 按下、b 抬起不得触发 click，实得 {events:?}"
    );
    match events
        .iter()
        .find(|e| matches!(e, RoutedEvent::Release { .. }))
    {
        Some(RoutedEvent::Release { pressed, released }) => {
            assert_eq!(pressed.as_name(), Some("a"), "press 应归按下者");
            assert_eq!(
                released.as_ref().and_then(|k| k.as_name()),
                Some("b"),
                "released 应如实报告抬起时命中的节点"
            );
        }
        other => panic!("应有 Release 事件，实得 {other:?}"),
    }
}

#[test]
fn hover_is_frozen_while_dragging() {
    // 拖动过程中悬停若跟随指针，按钮会在手指经过时逐个闪亮。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    let hovered = router.hovered().cloned();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Move));
    assert_eq!(router.hovered(), hovered.as_ref(), "按下期间悬停不应跟随");
    // 松手后悬停立刻追上当前位置
    route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Up));
    assert_eq!(router.hovered().and_then(|k| k.as_name()), Some("b"));
}

#[test]
fn pressing_on_blank_space_presses_nothing() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    let events = route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Down));
    assert!(events.is_empty(), "空白处按下不应产出事件");
    assert!(router.pressed().is_none());
    let events = route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Up));
    assert!(!events.iter().any(|e| matches!(e, RoutedEvent::Click(_))));
}

#[test]
fn reset_clears_the_press_capture() {
    // 失焦复位必须同时清掉按下状态。残留的后果很具体：失焦后第一次点击会被判为
    // "抬起"，从而误触发一个用户以为已经取消了的操作。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    assert!(router.pressed().is_some());
    router.reset();
    assert!(router.pressed().is_none());
    let events = route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Up));
    assert!(
        events.is_empty(),
        "复位后第一次抬起不得被当作 click，实得 {events:?}"
    );
}

// ============================== 捏合 ==============================

#[test]
fn a_two_finger_pinch_is_reported_alongside_hover_changes() {
    // 手势与 hover 是正交通道：捏合时界面高亮也该更新到当前手指位置。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        crate::input::pointer::PointerEvent {
            id: 1,
            x: 10.0,
            y: 10.0,
            phase: PointerPhase::Down,
            source: crate::input::pointer::PointerSource::Touch,
            time_ms: 0,
        },
    );
    let events = route(
        &mut router,
        &node,
        crate::input::pointer::PointerEvent {
            id: 2,
            x: 60.0,
            y: 10.0,
            phase: PointerPhase::Down,
            source: crate::input::pointer::PointerSource::Touch,
            time_ms: 16,
        },
    );
    assert!(
        events.iter().any(|e| matches!(e, RoutedEvent::Gesture(_))),
        "第二指按下应激活捏合并产出手势，实得 {events:?}"
    );
    assert_eq!(router.pointer_count(), 2);
}
