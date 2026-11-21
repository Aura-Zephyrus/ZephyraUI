//! 播放管线：demux → 解码线程 → 帧队列 → A/V 同步。
//!
//! 全部按 feature 选择编译：不开 `video` / `audio` 时本模块不编译。
//! demux 由 symphonia 完成（纯 Rust / MPL-2.0），裸码流交给 zephyra-video。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::clock::PresentationClock;
use crate::error::{MediaError, Result};
use crate::VideoFrameSource;

/// 轨道信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTrackInfo {
    pub id: u32,
    pub codec: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub duration_us: Option<i64>,
    /// 容器里的编解码器配置（MP4 的 avcC），用于 AVCC → Annex-B 转换。
    pub extra_data: Option<Vec<u8>>,
}

/// 播放选项。
#[derive(Debug, Clone)]
pub struct PlayerOptions {
    pub video: bool,
    pub audio: bool,
    /// 尝试 DXVA2 硬件解码（Windows + hw-dxva2 feature）。
    pub hardware: bool,
    /// 预加载共享库并报告失败（dynamic feature）。
    pub check_libraries: bool,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        Self {
            video: true,
            audio: true,
            hardware: false,
            check_libraries: true,
        }
    }
}

/// 解码线程向主线程上报的加载报告。
#[cfg(feature = "dynamic")]
pub type LibraryReport = zephyra_video::DynamicLoadReport;

pub(crate) enum Command {
    Seek,
    Reset,
}

/// 媒体播放器。
///
/// `open()` 只做探测与初始化；真正的解码在后台线程进行，主线程通过
/// `poll_video_frame()` / `poll_audio_frame()` 取帧。
pub struct MediaPlayer {
    video_rx: Option<Receiver<VideoFrameSource>>,
    audio_rx: Option<Receiver<crate::audio_frames::AudioFrame>>,
    /// 每个解码线程一个 sender：Stop 必须广播给全部线程，不能共享一个接收端。
    cmd_tx: Vec<Sender<Command>>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    clock: Arc<Mutex<PresentationClock>>,
    /// 尚未到显示时间的帧；保留它而不是丢弃，否则视频会大量丢帧。
    pending_video: Mutex<Option<VideoFrameSource>>,
    tracks: Vec<MediaTrackInfo>,
    #[cfg(feature = "dynamic")]
    library_report: LibraryReport,
}

impl MediaPlayer {
    /// 打开媒体文件。
    pub fn open(path: impl AsRef<Path>, options: PlayerOptions) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(MediaError::Io(format!("{} does not exist", path.display())));
        }

        #[cfg(feature = "dynamic")]
        let library_report = if options.check_libraries {
            zephyra_video::load_all()
        } else {
            zephyra_video::current_report()
        };
        // 缺库不阻止打开：主程序可通过 library_report() 看到具体失败并自行决定；
        // 真正用到的解码器会在 initialize() 时返回对应错误。

        let (tracks, packets) = crate::demux::probe(path)?;
        if tracks.is_empty() {
            return Err(MediaError::NoStream("playable"));
        }

        let clock = Arc::new(Mutex::new(PresentationClock::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let mut cmd_tx: Vec<Sender<Command>> = Vec::new();
        let mut threads = Vec::new();
        #[allow(unused_mut)]
        let mut video_rx = None;
        #[allow(unused_mut)]
        let mut audio_rx = None;

        #[cfg(feature = "video")]
        if options.video {
            let (tx, rx) = mpsc::channel();
            let (ctx, crx) = mpsc::channel();
            cmd_tx.push(ctx);
            video_rx = Some(rx);
            let packets = packets.clone();
            let tracks = tracks.clone();
            let clock = Arc::clone(&clock);
            let stop = Arc::clone(&stop);
            threads.push(std::thread::spawn(move || {
                crate::decode_thread::video_loop(packets, tracks, tx, clock, stop, crx);
            }));
        }

        #[cfg(feature = "audio")]
        if options.audio {
            let (tx, rx) = mpsc::channel();
            let (ctx, crx) = mpsc::channel();
            cmd_tx.push(ctx);
            audio_rx = Some(rx);
            let packets = packets.clone();
            let tracks = tracks.clone();
            let stop = Arc::clone(&stop);
            threads.push(std::thread::spawn(move || {
                crate::decode_thread::audio_loop(packets, tracks, tx, stop, crx);
            }));
        }

        Ok(Self {
            video_rx,
            audio_rx,
            cmd_tx,
            stop,
            threads,
            clock,
            pending_video: Mutex::new(None),
            tracks,
            #[cfg(feature = "dynamic")]
            library_report,
        })
    }

    pub fn tracks(&self) -> &[MediaTrackInfo] {
        &self.tracks
    }

    pub fn clock(&self) -> &Arc<Mutex<PresentationClock>> {
        &self.clock
    }

    #[cfg(feature = "dynamic")]
    pub fn library_report(&self) -> &LibraryReport {
        &self.library_report
    }

    pub fn poll_video_frame(&self) -> Option<VideoFrameSource> {
        self.video_rx.as_ref()?.try_recv().ok()
    }

    pub fn poll_audio_frame(&self) -> Option<crate::audio_frames::AudioFrame> {
        self.audio_rx.as_ref()?.try_recv().ok()
    }

    /// 按时钟取当前应显示的视频帧。
    ///
    /// 未到显示时间的帧会留在 `pending_video` 里，下次调用再判断；不再丢弃，
    /// 避免按 PTS 播放时几乎每帧都被误丢。
    pub fn present_video_frame(&self) -> Option<VideoFrameSource> {
        let mut pending = self.pending_video.lock().ok()?;
        let frame = match pending.take() {
            Some(f) => f,
            None => self.poll_video_frame()?,
        };
        let delta = {
            let c = self.clock.lock().ok()?;
            c.delta_us(frame.timestamp())
        };
        if delta > 0 {
            *pending = Some(frame);
            return None;
        }
        Some(frame)
    }

    pub fn seek(&self, media_us: i64) {
        if let Ok(mut c) = self.clock.lock() {
            c.seek(media_us);
        }
        for tx in &self.cmd_tx {
            let _ = tx.send(Command::Seek);
        }
    }

    pub fn pause(&self) {
        if let Ok(mut c) = self.clock.lock() {
            c.pause();
        }
    }

    pub fn resume(&self) {
        if let Ok(mut c) = self.clock.lock() {
            c.resume();
        }
    }

    pub fn reset(&self) {
        for tx in &self.cmd_tx {
            let _ = tx.send(Command::Reset);
        }
    }
}

impl Drop for MediaPlayer {
    fn drop(&mut self) {
        // 原子标志广播给所有线程；不再往共享 channel 发一条会被某个线程独吞的 Stop。
        self.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

pub(crate) fn is_stop(stop: &Arc<AtomicBool>) -> bool {
    stop.load(Ordering::Acquire)
}
