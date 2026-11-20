//! 跨帧 click 判据的边界测试。

use super::{clicks_from, ClickTracker};
use crate::id::ElementKey;

fn key(name: &'static str) -> ElementKey {
    ElementKey::Name(name)
}

#[test]
fn press_then_release_produces_one_click() {
    let k = key("ok");
    assert_eq!(clicks_from(std::slice::from_ref(&k), &[], false), vec![k]);
}

#[test]
fn held_key_does_not_click_every_frame() {
    let k = key("ok");
    assert!(clicks_from(std::slice::from_ref(&k), std::slice::from_ref(&k), false).is_empty());
    assert!(clicks_from(&[], &[k], false).is_empty());
}

#[test]
fn cancelled_frame_suppresses_click() {
    // 系统抢走手势时 pressed 也会被清成空 —— 不是抬起，绝不产生点击。
    let k = key("ok");
    assert!(clicks_from(&[k], &[], true).is_empty());
}

#[test]
fn multi_pointer_first_release_is_a_click() {
    let a = key("a");
    let b = key("b");
    // 两指各按一个，先抬 a：集合差必须把 a 算成点击，而不是丢掉。
    assert_eq!(
        clicks_from(&[a.clone(), b.clone()], std::slice::from_ref(&b), false),
        vec![a]
    );
}

#[test]
fn a_released_key_counts_even_if_another_is_pressed() {
    // 集合差是判据：上帧按 a、这帧只剩 b ⇒ a 被释放，仍是一次点击。
    // 「按下与抬起是否落在同一节点」由 router 的捕获机制保证，不在本层。
    assert_eq!(clicks_from(&[key("a")], &[key("b")], false), vec![key("a")]);
}

#[test]
fn tracker_remembers_last_frame() {
    let a = key("a");
    let mut tracker = ClickTracker::default();
    assert_eq!(
        tracker.update(std::slice::from_ref(&a), false),
        Vec::<ElementKey>::new()
    );
    assert_eq!(tracker.last_pressed(), std::slice::from_ref(&a));
    assert_eq!(tracker.update(&[], false), vec![a]);
    assert!(tracker.last_pressed().is_empty());
}

#[test]
fn tracker_cancelled_frame_advances_state_without_clicking() {
    let a = key("a");
    let mut tracker = ClickTracker::default();
    tracker.update(std::slice::from_ref(&a), false);
    assert!(tracker.update(&[], true).is_empty());
    // 状态照常前进：下一次按下不会被误当成那次取消的点击。
    assert!(tracker.last_pressed().is_empty());
}
