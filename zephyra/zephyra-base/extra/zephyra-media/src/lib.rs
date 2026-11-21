#![forbid(unsafe_code)]

//! 媒体播放桥接层：把 `zephyra-video` 的解码能力接进 ZephyraUI 的渲染 / 音频管线。
//!
//! 默认构建只有零依赖的词汇表（错误、时钟、轨道信息）；demux、解码、纹理导入、
//! 音频输出全部按 feature 选择编译，且不进 workspace `default-members`。

pub mod audio_frames;
pub mod clock;
#[cfg(any(feature = "video", feature = "audio"))]
pub mod decode_thread;
#[cfg(any(feature = "video", feature = "audio"))]
pub mod demux;
pub mod error;
#[cfg(feature = "audio")]
pub mod output;
#[cfg(any(feature = "video", feature = "audio"))]
pub mod pipeline;
#[cfg(feature = "video")]
pub mod video;
#[cfg(not(feature = "video"))]
pub mod video_stub;

pub use clock::PresentationClock;
pub use error::{MediaError, Result};
#[cfg(feature = "audio")]
pub use output::AudioSink;
#[cfg(any(feature = "video", feature = "audio"))]
pub use pipeline::{MediaPlayer, MediaTrackInfo, PlayerOptions};
#[cfg(feature = "video")]
pub use video::{VideoFrameSource, VideoTexture};
#[cfg(not(feature = "video"))]
pub use video_stub::VideoFrameSource;
