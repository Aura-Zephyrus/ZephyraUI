//! 操作（manipulation）识别的门禁。
//!
//! 判据对着官方触控指南的「操控事件」一节：操作事件提供**数据**
//! （位置、平移增量、速度），**不**标识交互形式。

use super::*;

/// 直接构造一张指针表（绕过 `Recognizer`）—— 本模块只关心几何。
fn table(points: &[(u64, f32, f32)]) -> PointerTable {
    let mut t = PointerTable::new();
    for &(id, x, y) in points {
        t.down(id, x, y);
    }
    t
}

#[test]
fn a_single_finger_translates_without_scaling_or_rotating() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 100.0, 100.0)]);
    s.on_down(&t, 0);

    t.move_pointer(0, 130.0, 90.0);
    let d = s.on_move(&t, 16);
    assert_eq!(d.translation_x, 30.0);
    assert_eq!(d.translation_y, -10.0);
    assert_eq!(d.scale, 1.0, "单指没有缩放");
    assert_eq!(d.rotation, 0.0, "单指没有旋转");
    assert_eq!(d.center_x, 130.0);
}

#[test]
fn two_fingers_spreading_produce_a_scale_delta() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 100.0, 100.0), (1, 200.0, 100.0)]);
    s.on_down(&t, 0);

    // 两指拉开 2 倍：质心不动，距离翻倍 ⇒ scale = 2。
    t.move_pointer(0, 50.0, 100.0);
    t.move_pointer(1, 250.0, 100.0);
    let d = s.on_move(&t, 16);
    assert!(
        d.scale > 1.9 && d.scale < 2.1,
        "scale 应当约为 2，实得 {}",
        d.scale
    );
    assert!(d.translation_x.abs() < 1.0, "质心没动，不该有平移");
}

#[test]
fn two_fingers_rotating_produce_a_rotation_delta() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 100.0, 100.0), (1, 200.0, 100.0)]);
    s.on_down(&t, 0);

    // 绕质心转 90°：(−50,0)→(0,−50)，(50,0)→(0,50)。
    t.move_pointer(0, 150.0, 50.0);
    t.move_pointer(1, 150.0, 150.0);
    let d = s.on_move(&t, 16);
    let expected = std::f32::consts::FRAC_PI_2;
    assert!(
        (d.rotation - expected).abs() < 0.01,
        "rotation 应当约为 π/2，实得 {}",
        d.rotation
    );
}

#[test]
fn rotation_is_normalized_so_it_never_jumps_by_more_than_pi() {
    let mut s = ManipulationState::new();
    // 初始方向约 179°。
    let mut t = table(&[(0, 100.0, 100.0), (1, 100.0 + 100.0 * 0.01745, 100.0)]);
    s.on_down(&t, 0);
    // 转过一点点到 −179°：朴素算法会算出 −358°。
    t.move_pointer(0, 100.0, 100.0);
    t.move_pointer(1, 100.0 - 100.0 * 0.01745, 100.0);
    let d = s.on_move(&t, 16);
    assert!(
        d.rotation.abs() <= std::f32::consts::PI + 0.01,
        "转角不该跳过一整圈，实得 {}",
        d.rotation
    );
}

#[test]
fn velocity_is_translation_over_time() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 0.0, 0.0)]);
    s.on_down(&t, 0);
    t.move_pointer(0, 20.0, 0.0);
    let d = s.on_move(&t, 100);
    // 20 px / 100 ms = 200 px/s。
    assert!(
        (d.velocity_x - 200.0).abs() < 0.5,
        "velocity_x 应当约为 200，实得 {}",
        d.velocity_x
    );
}

#[test]
fn lifting_the_last_finger_ends_the_manipulation_with_the_velocity() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 0.0, 0.0)]);
    s.on_down(&t, 0);
    t.move_pointer(0, 20.0, 0.0);
    s.on_move(&t, 100);
    assert!(s.is_active());

    t.up(0);
    let v = s.on_up(&t).expect("最后一根手指抬起 ⇒ 操作结束");
    assert!((v.0 - 200.0).abs() < 0.5, "速度应当延续，实得 {:?}", v);
    assert!(!s.is_active());
}

#[test]
fn a_second_finger_keeps_the_manipulation_alive() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 0.0, 0.0), (1, 100.0, 0.0)]);
    s.on_down(&t, 0);
    // 一根手指抬起，另一根还按着 ⇒ 操作**继续**（不是结束）。
    t.up(0);
    assert!(s.on_up(&t).is_none(), "还有一根手指时不结束");
    assert!(s.is_active());
}

#[test]
fn cancel_does_not_produce_inertia() {
    let mut s = ManipulationState::new();
    let mut t = table(&[(0, 0.0, 0.0)]);
    s.on_down(&t, 0);
    t.move_pointer(0, 40.0, 0.0);
    s.on_move(&t, 16);
    t.up(0);
    // ⚠ 取消**不**给惯性：用户是被系统抢走的，不是「甩手」。
    // 续上惯性的后果是内容在用户没打算的情况下继续滑。
    s.on_cancel(&t);
    assert!(!s.is_active());
    assert_eq!(s.velocity(), (0.0, 0.0), "取消后不该留下速度");
}

#[test]
fn an_idle_delta_reports_itself_as_idle() {
    let mut s = ManipulationState::new();
    let t = table(&[(0, 10.0, 10.0)]);
    s.on_down(&t, 0);
    let d = s.on_move(&t, 16);
    assert!(d.is_idle(), "没动过就不该产出增量");
}
