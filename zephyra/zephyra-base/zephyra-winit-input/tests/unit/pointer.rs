//! 本文件由 `zephyra::zephyra-base::zephyra-winit-input::src::pointer.rs` 的 `#[cfg(test)] mod tests` 抽出 —— 测试代码与生产代码分离。
//!
//! 模块路径不变（生产文件里仍用 `#[path = ...]` 声明），所以 `use super::*`
//! 与对父模块私有项的访问照旧。

use super::*;

#[test]
fn touch_phases_map_to_the_core_vocabulary() {
    let t = PointerTranslator::new();
    let ev = t.touch(winit::event::TouchPhase::Started, 7, 10.0, 20.0);
    assert_eq!(ev.phase, PointerPhase::Down);
    assert_eq!(ev.id, 7, "多点触控靠 id 区分手指");
    assert_eq!(ev.source, PointerSource::Touch);
    assert_eq!((ev.x, ev.y), (10.0, 20.0));

    assert_eq!(
        t.touch(winit::event::TouchPhase::Moved, 7, 10.0, 20.0)
            .phase,
        PointerPhase::Move
    );
    assert_eq!(
        t.touch(winit::event::TouchPhase::Ended, 7, 10.0, 20.0)
            .phase,
        PointerPhase::Up
    );
    // ⚠ 取消 ≠ 抬起。
    assert_eq!(
        t.touch(winit::event::TouchPhase::Cancelled, 7, 10.0, 20.0)
            .phase,
        PointerPhase::Cancel
    );
}

#[test]
fn the_clock_is_monotonic_and_never_goes_backwards() {
    let t = PointerTranslator::new();
    let a = t.now_ms();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let b = t.now_ms();
    assert!(b >= a, "单调时钟不能倒退：{a} → {b}");
}

#[test]
fn mouse_events_carry_the_mouse_source_and_id_zero() {
    let t = PointerTranslator::new();
    assert_eq!(t.mouse_move(1.0, 2.0).source, PointerSource::Mouse);
    assert_eq!(t.mouse_move(1.0, 2.0).id, 0);
    assert_eq!(t.mouse_button(true, 1.0, 2.0).phase, PointerPhase::Down);
    assert_eq!(t.mouse_button(false, 1.0, 2.0).phase, PointerPhase::Up);
}
