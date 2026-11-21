//! 音频帧类型统一出口。

#[cfg(feature = "audio")]
pub use zephyra_video::AudioFrame;

/// 未开 audio feature 时的占位类型（pipeline 的方法签名仍需可编译）。
#[cfg(not(feature = "audio"))]
#[derive(Debug, Clone)]
pub struct AudioFrame;
