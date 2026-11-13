//! 触控语义的路由门禁。
//!
//! 判据对着官方触控指南（Windows apps · 触控交互开发人员指南）：
//!
//! - **触控是双状态的**（没有悬停）；
//! - **指针取消不是抬起**（取消不提交点击）；
//! - 静态手势（轻点 / 双击 / 长按）与「点击」并存而不同。

use super::*;
use crate::input::pointer::PointerSource;
use crate::test_support::{route, tree, two_buttons};

/// 造一个触控指针事件。
fn touch(id: u64, x: f32, y: f32, phase: PointerPhase, t: u64) -> PointerEvent {
    PointerEvent {
        id,
        x,
        y,
        phase,
        source: PointerSource::Touch,
        time_ms: t,
    }
}

/// 造一个鼠标指针事件。
fn mouse(id: u64, x: f32, y: f32, phase: PointerPhase, t: u64) -> PointerEvent {
    PointerEvent {
        id,
        x,
        y,
        phase,
        source: PointerSource::Mouse,
        time_ms: t,
    }
}

/// ⚠ **触控不产生悬停**。
///
/// 官方指南：「触控使用双状态模式：显示设备的触控表面要么处于触控状态（打开），
/// 要么处于非触控状态（关闭）。**没有可触发其他视觉反馈的悬停状态。**」
///
/// 症状（若触控也写悬停）：控件同时亮出 hover 与 pressed 两种外观 —— 而触屏上
/// 根本没有「悬停」这回事。
#[test]
fn touch_never_produces_hover() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();

    // 手指按下并移动到另一个按钮上。
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    route(
        &mut router,
        &node,
        touch(0, 60.0, 10.0, PointerPhase::Move, 16),
    );

    assert!(
        router.hovered().is_none(),
        "触控不该有悬停，实得 {:?}",
        router.hovered()
    );
}

/// 鼠标**仍然**产生悬停（双状态是触控特有的）。
#[test]
fn mouse_still_produces_hover() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        mouse(0, 10.0, 10.0, PointerPhase::Move, 0),
    );
    assert!(router.hovered().is_some(), "鼠标移动应当有悬停");
}

/// ⚠ **取消不是抬起**：取消**不**产出点击。
///
/// 官方指南把「指针取消」单列为一个事件：「在**异常失去**触控交互时发生」。
/// Windows 11 把三指 / 四指手势交给系统时，应用收到的就是取消。
///
/// 若把取消当抬起：用户手指还在屏幕上，应用却以为用户松手了 ⇒
/// **误触发一次点击**。
#[test]
fn a_cancelled_touch_does_not_click() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();

    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    let evs = route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Cancel, 16),
    );

    assert!(
        !evs.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "取消不该产出点击：{evs:?}"
    );
    // 按下态必须被清掉 —— 否则下一次点击会被判成「抬起」。
    assert!(router.pressed().is_none(), "取消后不该还按着");
}

/// 取消要产出 `Release`（让控件把按下态清掉），但 `released` 是 `None`
/// —— 「松开了但没松在任何控件上」。
#[test]
fn cancel_releases_without_a_release_target() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    let evs = route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Cancel, 16),
    );
    let release = evs
        .iter()
        .find_map(|e| match e {
            RoutedEvent::Release { pressed, released } => Some((pressed.clone(), released.clone())),
            _ => None,
        })
        .expect("取消应产出 Release");
    assert!(
        release.1.is_none(),
        "取消没有「松在哪个控件上」：{release:?}"
    );
}

/// 触控的轻点：按下 + 抬起落在同一处 ⇒ 既有 `Click` 也有 `Tapped`。
///
/// 两者**并存**而不是替代：点击是「按下与抬起落在同一节点」，轻点还多了
/// 位移容差与时长上限两条判据。
#[test]
fn a_touch_tap_produces_both_click_and_tapped() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    let evs = route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Up, 80),
    );

    assert!(
        evs.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "应产出 Click：{evs:?}"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, RoutedEvent::Static(StaticGesture::Tapped, _))),
        "应产出轻点：{evs:?}"
    );
}

/// **两指并存**：两根手指各自登记，按下态仍是「最后那根」。
///
/// 官方指南：「不应按所用手指数来区分操作」—— 多指不该改变「谁被按下」的语义。
#[test]
fn two_fingers_are_tracked_independently() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    route(
        &mut router,
        &node,
        touch(1, 60.0, 10.0, PointerPhase::Down, 16),
    );
    assert_eq!(router.pointer_count(), 2, "两根手指都该在册");

    // 抬起第一根 ⇒ 还剩一根。
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Up, 32),
    );
    assert_eq!(router.pointer_count(), 1);
}

/// 多指的**操作增量**：两根手指分开 ⇒ 缩放比 ≠ 1。
///
/// 官方指南的操作事件**只给数据、不给形式** —— 所以这里给的是
/// `scale` / `translation`，由应用决定「这是捏合还是别的」。
#[test]
fn two_fingers_produce_a_manipulation_delta() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    route(
        &mut router,
        &node,
        touch(1, 60.0, 10.0, PointerPhase::Down, 16),
    );
    let evs = route(
        &mut router,
        &node,
        touch(1, 100.0, 10.0, PointerPhase::Move, 32),
    );

    let delta = evs
        .iter()
        .find_map(|e| match e {
            RoutedEvent::Manipulation(d) => Some(*d),
            _ => None,
        })
        .expect("多指移动应产出操作增量");
    assert!(
        delta.scale > 1.0,
        "两指分开 ⇒ 缩放比应当 > 1，实得 {}",
        delta.scale
    );
}

/// 一次**原地**点击不产出「操作结束」—— 那只是噪音。
#[test]
fn a_stationary_click_produces_no_manipulation_end() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    let evs = route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Up, 16),
    );
    assert!(
        !evs.iter()
            .any(|e| matches!(e, RoutedEvent::ManipulationEnded { .. })),
        "没动过就不该报「操作结束」：{evs:?}"
    );
}

/// 拖动过 ⇒ 抬手时产出「操作结束」+ 速度（惯性的初值）。
#[test]
fn a_drag_produces_the_inertia_velocity() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    route(
        &mut router,
        &node,
        touch(0, 40.0, 10.0, PointerPhase::Move, 16),
    );
    let evs = route(
        &mut router,
        &node,
        touch(0, 40.0, 10.0, PointerPhase::Up, 32),
    );
    let ended = evs
        .iter()
        .find_map(|e| match e {
            RoutedEvent::ManipulationEnded { velocity_x, .. } => Some(*velocity_x),
            _ => None,
        })
        .expect("拖动后抬手应产出「操作结束」");
    assert!(ended > 0.0, "右移 ⇒ 横向速度应当为正，实得 {ended}");
}

/// ⚠ **两指各按一个控件**：两个按下都必须各自收到一次抬起。
///
/// # 为什么这条重要
///
/// 按下态曾经是**一个槽**（`pressed: Option<ElementKey>`）—— 第二根手指按下
/// 会**覆盖**第一根的。而抬起时 `pressed.take()` 只取走**一个** ⇒ 第一根手指
/// 抬起时**收不到 `Release`**。
///
/// 症状：**第一个按钮卡在按下态**（一直高亮），而它的 `Release` 从此消失 ——
/// 控件永远回不到正常外观。触屏上「用两根手指同时按两个按钮」是很常见的
/// 手势（多指缩放页面时手指常常各按一处）。
#[test]
fn two_fingers_pressing_two_controls_each_get_their_release() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();

    // 第一根手指按在 a 上（a 在 0..40）。
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    // 第二根手指按在 b 上（b 在 40..80）。
    route(
        &mut router,
        &node,
        touch(1, 60.0, 10.0, PointerPhase::Down, 16),
    );

    // 第一根手指抬起：必须收到 a 的 Release。
    let evs = route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Up, 32),
    );
    let released_a = evs.iter().any(
        |e| matches!(e, RoutedEvent::Release { pressed, .. } if pressed.as_name() == Some("a")),
    );
    assert!(
        released_a,
        "第一根手指抬起应收到 a 的 Release，实得 {evs:?}"
    );

    // 第二根手指抬起：必须收到 b 的 Release。
    let evs = route(
        &mut router,
        &node,
        touch(1, 60.0, 10.0, PointerPhase::Up, 48),
    );
    let released_b = evs.iter().any(
        |e| matches!(e, RoutedEvent::Release { pressed, .. } if pressed.as_name() == Some("b")),
    );
    assert!(
        released_b,
        "第二根手指抬起应收到 b 的 Release，实得 {evs:?}"
    );
}

/// 两指各按一个控件 ⇒ **两个**节点都在按下态里。
///
/// 应用要画**两个**按下态，所以「全部按下态」这个集合不能只留一个。
#[test]
fn two_pressed_controls_are_both_reported() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();

    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Down, 0),
    );
    route(
        &mut router,
        &node,
        touch(1, 60.0, 10.0, PointerPhase::Down, 16),
    );

    let pressed: Vec<&str> = router.pressed_keys().filter_map(|k| k.as_name()).collect();
    assert_eq!(pressed.len(), 2, "两个控件都该在按下态里，实得 {pressed:?}");
    assert!(pressed.contains(&"a"), "{pressed:?}");
    assert!(pressed.contains(&"b"), "{pressed:?}");

    // 抬起一根 ⇒ 只剩一个。
    route(
        &mut router,
        &node,
        touch(0, 10.0, 10.0, PointerPhase::Up, 32),
    );
    let pressed: Vec<&str> = router.pressed_keys().filter_map(|k| k.as_name()).collect();
    assert_eq!(
        pressed,
        vec!["b"],
        "抬起 a 之后应当只剩 b，实得 {pressed:?}"
    );
}
