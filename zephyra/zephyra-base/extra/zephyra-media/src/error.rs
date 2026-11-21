//! 媒体播放错误类型。

/// 播放链路中可能失败的各层。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// 文件打不开或读取失败。
    Io(String),
    /// 容器解复用失败（格式不支持、损坏）。
    Demux(String),
    /// 解码失败（码流损坏、配置不支持）。
    Decoder(String),
    /// 动态加载的解码库缺失 / 版本不匹配。
    ///
    /// 内容来自 `zephyra_video::DynamicLoadReport`，由主程序决定如何处理。
    DecoderLibrary(String),
    /// 平台能力缺失（例如没有音频设备）。
    Platform(String),
    /// 流里没有要求的轨道。
    NoStream(&'static str),
    /// 不支持的格式 / 操作。
    Unsupported(&'static str),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::Io(s) => write!(f, "I/O error: {s}"),
            MediaError::Demux(s) => write!(f, "demux error: {s}"),
            MediaError::Decoder(s) => write!(f, "decoder error: {s}"),
            MediaError::DecoderLibrary(s) => write!(f, "decoder library unavailable: {s}"),
            MediaError::Platform(s) => write!(f, "platform error: {s}"),
            MediaError::NoStream(s) => write!(f, "no {s} stream"),
            MediaError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for MediaError {}

pub type Result<T> = std::result::Result<T, MediaError>;
