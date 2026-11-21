//! 未开 video feature 时的占位类型，让 audio-only 构建的 pipeline 仍可编译。

/// 占位视频帧来源（不会真的产生）。
#[derive(Debug, Clone)]
pub struct VideoFrameSource;

impl VideoFrameSource {
    pub fn timestamp(&self) -> i64 {
        0
    }
    pub fn duration(&self) -> i64 {
        0
    }
    pub fn as_external_source(&self) -> Option<zephyra_render_ext::prelude::ExternalTextureSource> {
        None
    }
}
