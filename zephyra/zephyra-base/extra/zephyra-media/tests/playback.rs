//! 端到端回放测试：打开真实 MP4（H.264 + AAC），验证轨道识别与出帧。

use std::io::Write;

use zephyra_media::{MediaPlayer, PlayerOptions};

fn fixture_path() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("zephyra-media-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.mp4");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(include_bytes!("data/test.mp4")).unwrap();
    drop(f);
    path
}

#[test]
fn opens_mp4_and_reports_tracks() {
    let path = fixture_path();
    let player = MediaPlayer::open(&path, PlayerOptions::default()).unwrap();

    let tracks = player.tracks();
    assert!(!tracks.is_empty());
    assert!(tracks
        .iter()
        .any(|t| t.codec.to_ascii_lowercase().contains("h264")));
    assert!(tracks
        .iter()
        .any(|t| t.codec.to_ascii_lowercase().contains("aac")));
    let video = tracks.iter().find(|t| t.width.is_some()).unwrap();
    assert_eq!((video.width, video.height), (Some(64), Some(48)));
}

#[test]
fn decodes_video_and_audio_frames() {
    let path = fixture_path();
    let player = MediaPlayer::open(&path, PlayerOptions::default()).unwrap();

    let mut video_frames = 0;
    let mut audio_frames = 0;
    for _ in 0..200 {
        if player.poll_video_frame().is_some() {
            video_frames += 1;
        }
        if player.poll_audio_frame().is_some() {
            audio_frames += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        if video_frames > 0 && audio_frames > 0 {
            break;
        }
    }
    assert!(video_frames > 0, "no video frames decoded");
    assert!(audio_frames > 0, "no audio frames decoded");
}
