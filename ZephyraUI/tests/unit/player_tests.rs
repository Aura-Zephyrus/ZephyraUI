//! 播放器状态模型单测：流转规则、派生几何、分类词与样式表对齐。

use super::*;

fn state() -> PlayerState {
    PlayerState::new("player-demo").with_duration_us(60_000_000)
}

#[test]
fn class_words_exist_in_the_video_stylesheet() {
    let sheet = crate::styles::stylesheet("video").expect("video 样式表必须在册");
    for word in [CLASS_PLAYING, CLASS_PAUSED, CLASS_BUFFERING, CLASS_ENDED] {
        assert!(
            sheet.contains(&format!(".{word}")),
            "样式表里没有 .{word} —— 模型发出的分类词会静默失效"
        );
    }
}

#[test]
fn toggle_play_and_advance_follow_the_rate() {
    let mut p = state();
    assert_eq!(p.state_class(), CLASS_PAUSED);
    p.toggle_play();
    assert!(p.is_playing());
    p.advance(1000.0);
    assert_eq!(p.current_us(), 1_000_000);
    p.cycle_rate();
    assert_eq!(p.rate(), 1.25);
    p.pause();
    p.advance(1000.0); // 暂停后不再走
    assert_eq!(p.current_us(), 1_000_000);
}

#[test]
fn end_clamps_time_and_play_restarts() {
    let mut p = state();
    p.play();
    p.advance(120_000.0); // 超过 60s
    assert_eq!(p.state_class(), CLASS_ENDED);
    assert_eq!(p.current_us(), p.duration_us());
    p.play();
    assert_eq!(p.current_us(), 0);
    assert_eq!(p.state_class(), CLASS_PLAYING);
}

#[test]
fn seek_and_volume_are_bounded_and_drive_geometry() {
    let mut p = state();
    p.set_seek_geometry(100.0, SEEK_TRACK_W);
    p.seek_at(100.0 + SEEK_TRACK_W / 2.0);
    assert!((p.progress01() - 0.5).abs() < 0.05);
    assert!((p.seek_fill_w() - SEEK_TRACK_W * 0.5).abs() < 12.0);
    p.seek_at(-1000.0);
    assert_eq!(p.current_us(), 0);
    p.seek_at(100_000.0);
    assert_eq!(p.current_us(), p.duration_us());

    p.set_volume_geometry(0.0, VOLUME_TRACK_W);
    p.set_volume(0.5);
    assert!((p.volume_fill_w() - VOLUME_TRACK_W * 0.5).abs() < 0.01);
    p.toggle_mute();
    assert_eq!(p.effective_volume(), 0.0);
    assert_eq!(p.mute_icon(), "volume-off");
    p.set_volume(2.0);
    assert_eq!(p.volume(), 1.0);
}

#[test]
fn time_formatting_is_language_neutral() {
    assert_eq!(PlayerState::format_time(0), "0:00");
    assert_eq!(PlayerState::format_time(65_000_000), "1:05");
    assert_eq!(PlayerState::format_time(3_661_000_000), "1:01:01");
}

#[test]
fn interaction_keys_are_unique() {
    let keys = [
        KEY_PLAY_PAUSE,
        KEY_SEEK,
        KEY_SEEK_FILL,
        KEY_VOLUME,
        KEY_VOLUME_FILL,
        KEY_MUTE,
        KEY_RATE,
        KEY_FULLSCREEN,
    ];
    for (i, a) in keys.iter().enumerate() {
        for b in keys.iter().skip(i + 1) {
            assert_ne!(a, b, "交互 key 撞名：{a}");
        }
    }
}

#[test]
fn emitted_icon_names_are_registered_in_the_ui_layer() {
    let p = state();
    let names = crate::icons::icon_set().names();
    for icon in [p.play_icon(), p.mute_icon(), p.fullscreen_icon()] {
        assert!(names.contains(&icon), "播放器用了未登记的图标 {icon}");
    }
}
