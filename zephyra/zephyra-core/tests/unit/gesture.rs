//! 静态手势（轻点 / 双击 / 长按）的门禁。
//!
//! 判据对着官方触控指南的「静态手势事件」一档：
//! Tap / DoubleTap / Holding。

use super::*;
use crate::input::pointer::PointerSource;

/// 造一个指针事件。
fn ev(id: u64, x: f32, y: f32, phase: PointerPhase, time_ms: u64) -> PointerEvent {
    PointerEvent {
        id,
        x,
        y,
        phase,
        source: PointerSource::Touch,
        time_ms,
    }
}

fn down(id: u64, x: f32, y: f32, t: u64) -> PointerEvent {
    ev(id, x, y, PointerPhase::Down, t)
}

fn up(id: u64, x: f32, y: f32, t: u64) -> PointerEvent {
    ev(id, x, y, PointerPhase::Up, t)
}

fn move_to(id: u64, x: f32, y: f32, t: u64) -> PointerEvent {
    ev(id, x, y, PointerPhase::Move, t)
}

#[test]
fn a_quick_press_and_release_is_a_tap() {
    let mut g = StaticGestures::new();
    assert_eq!(g.on_pointer(down(0, 10.0, 10.0, 0)), None);
    assert_eq!(
        g.on_pointer(up(0, 10.0, 10.0, 80)),
        Some(StaticGesture::Tapped)
    );
}

/// ⚠ 超过时长上限 ⇒ **不是**轻点。
///
/// 指南把长按列为独立的静态手势 —— 所以「按住很久再松开」不能同时报轻点。
#[test]
fn a_long_press_is_not_a_tap() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    assert_eq!(
        g.on_pointer(up(0, 10.0, 10.0, 800)),
        None,
        "按住 800 ms 不该报轻点"
    );
}

/// 位移超出容差 ⇒ **不是**轻点（指尖抖动不该让轻点变成「拖」）。
#[test]
fn moving_beyond_the_slop_cancels_the_tap() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    g.on_pointer(move_to(0, 40.0, 10.0, 30));
    assert_eq!(g.on_pointer(up(0, 40.0, 10.0, 80)), None);
}

/// 抖动在容差内 ⇒ 仍是轻点。
#[test]
fn jitter_within_the_slop_keeps_the_tap() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    g.on_pointer(move_to(0, 14.0, 13.0, 30));
    assert_eq!(
        g.on_pointer(up(0, 14.0, 13.0, 80)),
        Some(StaticGesture::Tapped)
    );
}

/// 两次轻点、间隔与距离都在窗口内 ⇒ 双击，且**不**重复报两次轻点。
#[test]
fn two_quick_taps_are_a_double_tap() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    assert_eq!(
        g.on_pointer(up(0, 10.0, 10.0, 60)),
        Some(StaticGesture::Tapped)
    );
    g.on_pointer(down(0, 12.0, 11.0, 200));
    assert_eq!(
        g.on_pointer(up(0, 12.0, 11.0, 250)),
        Some(StaticGesture::DoubleTapped),
        "第二次轻点应当报双击，而不是又报一次轻点"
    );
}

/// 间隔太长 ⇒ 两次独立的轻点，不是双击。
#[test]
fn taps_too_far_apart_are_two_single_taps() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    g.on_pointer(up(0, 10.0, 10.0, 60));
    g.on_pointer(down(0, 10.0, 10.0, 1000));
    assert_eq!(
        g.on_pointer(up(0, 10.0, 10.0, 1060)),
        Some(StaticGesture::Tapped)
    );
}

/// 两次轻点离得太远 ⇒ 不是双击（双击要求在同一处附近）。
#[test]
fn taps_too_far_apart_in_space_are_two_single_taps() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    g.on_pointer(up(0, 10.0, 10.0, 60));
    g.on_pointer(down(0, 200.0, 200.0, 150));
    assert_eq!(
        g.on_pointer(up(0, 200.0, 200.0, 200)),
        Some(StaticGesture::Tapped)
    );
}

/// 长按是**时钟驱动**的：手指不动 ⇒ 没有事件 ⇒ 只能靠 tick 报出来。
///
/// 这正是「长按」与「拖动」的根本区别：拖动靠 Move 事件，长按靠时间。
#[test]
fn holding_is_reported_by_the_clock_not_by_an_event() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    assert_eq!(g.tick(100), None, "还没到时长");
    assert_eq!(g.tick(HOLD_DURATION_MS - 1), None, "还差 1 ms");
    assert_eq!(g.tick(HOLD_DURATION_MS), Some(StaticGesture::Holding));
    // 长按**只报一次**。
    assert_eq!(g.tick(HOLD_DURATION_MS + 500), None);
}

/// 挪动过之后不再算长按（那是拖动）。
#[test]
fn moving_cancels_the_hold() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    g.on_pointer(move_to(0, 60.0, 10.0, 50));
    assert_eq!(g.tick(HOLD_DURATION_MS + 100), None);
}

/// ⚠ **取消不是抬起**：取消之后**不**报轻点。
///
/// 官方指南：「指针取消：在异常失去触控交互时发生」。Windows 11 默认把
/// 三指/四指手势交给系统 —— 那时应用收到的是取消。若当成抬起，用户
/// 没抬手也会被判成「轻点」，于是按钮被误触发。
#[test]
fn a_cancelled_touch_produces_no_tap() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    assert_eq!(
        g.on_pointer(ev(0, 10.0, 10.0, PointerPhase::Cancel, 60)),
        None
    );
    // 取消之后紧接的一次抬起也不该报轻点（那一根手指已经不算数了）。
    assert_eq!(g.on_pointer(up(0, 10.0, 10.0, 80)), None);
}

/// 取消会断掉双击窗口：被系统抢走的那一下与后续的轻点无关。
#[test]
fn cancel_breaks_the_double_tap_window() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    g.on_pointer(up(0, 10.0, 10.0, 60));
    // 第二次按下后被取消 ⇒ 双击窗口断掉。
    g.on_pointer(down(0, 10.0, 10.0, 150));
    g.on_pointer(ev(0, 10.0, 10.0, PointerPhase::Cancel, 200));
    g.on_pointer(down(0, 10.0, 10.0, 250));
    assert_eq!(
        g.on_pointer(up(0, 10.0, 10.0, 300)),
        Some(StaticGesture::Tapped),
        "取消之后的轻点应当是独立的一次轻点"
    );
}

/// 多指时**不**产生轻点 —— 但也不是「按手指数区分操作」。
///
/// 指南：「不应按所用手指数来区分操作。」这里的意思是：多指的操作数据
/// 走 manipulation，而**不是**说「多指 = 另一种点击」。
#[test]
fn a_second_finger_does_not_break_the_first_ones_tap() {
    let mut g = StaticGestures::new();
    g.on_pointer(down(0, 10.0, 10.0, 0));
    // 第二根手指按下又抬起 —— 主指针还在。
    g.on_pointer(down(1, 200.0, 200.0, 10));
    assert_eq!(g.on_pointer(up(1, 200.0, 200.0, 20)), None);
    // 主指针抬起 ⇒ 仍是轻点。
    assert_eq!(
        g.on_pointer(up(0, 10.0, 10.0, 60)),
        Some(StaticGesture::Tapped)
    );
}
