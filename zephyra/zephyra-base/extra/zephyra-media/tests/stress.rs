//! 压力与边界：多次 open/drop、随机访问、音视频帧丰富度。

use zephyra_media::{MediaPlayer, PlayerOptions};

fn fixture() -> std::path::PathBuf {
    let p = std::env::temp_dir()
        .join("zephyra-media-test")
        .join("test.mp4");
    if !p.exists() {
        // 与 playback.rs 共享：若不存在则由该测试先生成。
        let dir = p.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x48:rate=5",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "0.4",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
                "-f",
                "mp4",
                p.to_str().unwrap(),
                "-y",
            ])
            .status()
            .ok();
    }
    p
}

#[test]
fn repeated_open_drop_does_not_hang_or_leak_threads() {
    let path = fixture();
    for _ in 0..20 {
        let player = MediaPlayer::open(&path, PlayerOptions::default()).unwrap();
        drop(player);
    }
}

#[test]
fn video_frames_have_geometrically_valid_cpu_planes() {
    let path = fixture();
    let player = MediaPlayer::open(&path, PlayerOptions::default()).unwrap();
    let mut checked = 0;
    for _ in 0..500 {
        if let Some(zephyra_media::VideoFrameSource::Cpu(f)) = player.poll_video_frame() {
            assert_eq!(f.coded_size().width, 64);
            assert_eq!(f.coded_size().height, 48);
            assert_eq!(f.num_planes(), 3);
            // I420: Y 平面 w*h，U/V 各 w*h/4。用帧布局给出的几何校验。
            for (i, plane) in f.planes().iter().enumerate() {
                let geom = f.layout().planes()[i];
                let rows = f.layout().plane_rows(i);
                assert!(plane.cpu_data().unwrap().len() >= geom.stride * rows);
            }
            checked += 1;
        }
        if checked >= 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(checked > 0);
}

#[test]
fn audio_frames_have_expected_sample_rate_and_nonzero_samples() {
    let path = fixture();
    let player = MediaPlayer::open(&path, PlayerOptions::default()).unwrap();
    let mut checked = 0;
    for _ in 0..500 {
        if let Some(f) = player.poll_audio_frame() {
            assert!(f.sample_rate() == 48000);
            assert!(f.frames() > 0);
            assert!(f.planes().iter().any(|p| p.iter().any(|&b| b != 0)));
            checked += 1;
        }
        if checked >= 3 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(checked > 0);
}

#[test]
fn cpu_video_desc_maps_all_supported_formats() {
    use zephyra_render_ext::prelude::ExternalTextureFormat;
    use zephyra_video::{VideoFrame, VideoPixelFormat};

    let size = zephyra_video::Size::new(64, 48);
    let rect = zephyra_video::Rect::new(0, 0, 64, 48);
    // 所有 4:2:0 格式都必须能转成 D3D11 importer 接受的 NV12。
    let cases = [
        VideoPixelFormat::I420,
        VideoPixelFormat::NV12,
        VideoPixelFormat::NV21,
        VideoPixelFormat::YUV420P10,
        VideoPixelFormat::P010LE,
    ];
    for fmt in cases {
        let bpp = if matches!(fmt, VideoPixelFormat::YUV420P10 | VideoPixelFormat::P010LE) {
            2
        } else {
            1
        };
        let plane_sizes = if matches!(
            fmt,
            VideoPixelFormat::NV12 | VideoPixelFormat::NV21 | VideoPixelFormat::P010LE
        ) {
            vec![64 * 48 * bpp, 64 * 24 * bpp]
        } else {
            vec![64 * 48 * bpp, 32 * 24 * bpp, 32 * 24 * bpp]
        };
        let frame = VideoFrame::from_cpu(
            fmt,
            size,
            rect,
            size,
            plane_sizes.into_iter().map(|n| vec![0u8; n]).collect(),
            0,
            0,
        )
        .unwrap();
        let source = zephyra_media::video::from_video_frame(frame);
        let desc = zephyra_media::import_video_frame(&source).expect("FrameDesc");
        assert_eq!(desc.format, ExternalTextureFormat::Nv12, "{:?}", fmt);
        assert_eq!(desc.size, (64, 48), "{:?}", fmt);
        let planes = desc.cpu_planes().expect("CPU planes");
        assert_eq!(planes.plane_count(), 2);
        assert_eq!(planes.plane(0).unwrap().len(), 64 * 48, "{:?}", fmt);
        assert_eq!(planes.plane(1).unwrap().len(), 32 * 48, "{:?}", fmt);
        // 两平面以 Arc<[u8]> 拥有所有权交付，不再 flatten 成单缓冲。
        assert_eq!(planes.row_pitch[0], 64);
        assert_eq!(planes.row_pitch[1], 64);
    }
}

#[test]
fn present_video_frame_never_drops_future_frames_permanently() {
    let path = fixture();
    let player = MediaPlayer::open(&path, PlayerOptions::default()).unwrap();
    // 时钟还没走到任何帧的 PTS 时，应返回 None 但把帧缓存起来，
    // 下一次（推进时钟后）应能取到同一帧，而不是永久丢弃。
    let mut got = None;
    for _ in 0..200 {
        if let Some(f) = player.present_video_frame() {
            got = Some(f);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(got.is_some(), "no frame ever became presentable");
}
