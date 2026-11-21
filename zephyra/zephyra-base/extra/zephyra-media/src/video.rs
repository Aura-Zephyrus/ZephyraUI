//! 视频帧 → 渲染契约描述符的转换。
//!
//! 软件后端输出 CPU 平面（I420 / NV12 / YUV420P10），硬件后端（DXVA2）输出
//! 平台原生句柄。两类分别映射成 `zephyra-render-ext` 的 CPU 多平面描述符与
//! `ExternalTextureSource::NtHandle`。

use zephyra_render_ext::external_texture::CpuVideoTextureDesc;
use zephyra_render_ext::prelude::{
    CpuTextureDesc, ExternalTextureFormat, ExternalTextureSource, FrameId, RawHandle,
};
use zephyra_video::{VideoFrame, VideoPixelFormat, VideoPlane};

/// 一帧已导入到渲染后端的纹理。
pub type VideoTexture = zephyra_render_ext::external_texture::ImportedTexture;

/// 视频帧来源（便于队列统一存放 CPU 与原生帧）。
pub enum VideoFrameSource {
    Cpu(VideoFrame),
    Native {
        handle: RawHandle,
        format: ExternalTextureFormat,
        width: u32,
        height: u32,
        owner: zephyra_video::NativeResourceOwner,
        timestamp: i64,
        duration: i64,
    },
}

impl VideoFrameSource {
    pub fn timestamp(&self) -> i64 {
        match self {
            VideoFrameSource::Cpu(f) => f.timestamp(),
            VideoFrameSource::Native { timestamp, .. } => *timestamp,
        }
    }

    pub fn duration(&self) -> i64 {
        match self {
            VideoFrameSource::Cpu(f) => f.duration(),
            VideoFrameSource::Native { duration, .. } => *duration,
        }
    }

    /// 转成渲染层的外部纹理来源；CPU 平面借用本帧的内存。
    pub fn as_external_source(&self) -> Option<ExternalTextureSource> {
        match self {
            VideoFrameSource::Native { handle, .. } => Some(ExternalTextureSource::NtHandle {
                handle: *handle,
                source_backend: None,
            }),
            VideoFrameSource::Cpu(_) => None,
        }
    }
}

/// 把 CPU 视频帧的平面映射为多平面上传描述。
///
/// 返回 `None` 表示该像素格式没有对应的渲染格式（例如 RGB 帧可直接走
/// 单平面 `CpuTextureDesc`）。
pub fn cpu_video_desc<'a>(frame: &'a VideoFrame) -> Option<CpuVideoTextureDesc<'a>> {
    let (format, count) = match frame.format() {
        VideoPixelFormat::I420 => (ExternalTextureFormat::I420, 3),
        VideoPixelFormat::NV12 | VideoPixelFormat::NV21 => (ExternalTextureFormat::Nv12, 2),
        VideoPixelFormat::YUV420P10 => (ExternalTextureFormat::I420P10, 3),
        VideoPixelFormat::P010LE => (ExternalTextureFormat::P010, 2),
        _ => return None,
    };
    let layout = frame.layout();
    let w = frame.coded_size().width;
    let h = frame.coded_size().height;
    // 亮度全分辨率，色度宽高各减半（4:2:0）。stride 只是行距，不能当宽度用。
    let mut planes = [None, None, None];
    for (i, plane) in frame.planes().iter().enumerate().take(count) {
        let data = match plane {
            VideoPlane::Cpu { data } => data.as_slice(),
            VideoPlane::Native { .. } => return None,
        };
        let stride = layout.planes()[i].stride as u32;
        let width = if i == 0 { w } else { w.div_ceil(2) };
        let height = if i == 0 { h } else { h.div_ceil(2) };
        planes[i] = Some(CpuTextureDesc {
            pixels: data,
            width,
            height,
            row_pitch: stride,
            format,
        });
    }
    Some(CpuVideoTextureDesc { planes, format })
}

/// 把解码帧包装成统一的来源类型。
pub fn from_video_frame(frame: VideoFrame) -> VideoFrameSource {
    // 先检查是否有原生平面。
    let native = frame.planes().iter().find_map(|p| match p {
        VideoPlane::Native { handle } => Some(*handle),
        VideoPlane::Cpu { .. } => None,
    });
    if let (Some(handle), Some(owner)) = (native, frame.native_owner().cloned()) {
        let (w, h) = (frame.coded_size().width, frame.coded_size().height);
        let format = match frame.format() {
            VideoPixelFormat::NV12 => ExternalTextureFormat::Nv12,
            VideoPixelFormat::P010LE => ExternalTextureFormat::P010,
            _ => ExternalTextureFormat::Rgba8Unorm,
        };
        return VideoFrameSource::Native {
            handle: RawHandle(handle as usize),
            format,
            width: w,
            height: h,
            owner,
            timestamp: frame.timestamp(),
            duration: frame.duration(),
        };
    }
    VideoFrameSource::Cpu(frame)
}

/// 帧的过期帧号。
pub fn frame_id(n: u64) -> FrameId {
    FrameId(n)
}
