//! 视频帧 → 渲染契约描述符的转换。
//!
//! 软件后端输出 CPU 平面（I420 / NV12 / YUV420P10），硬件后端（DXVA2）输出
//! 平台原生句柄。两类分别映射成 `zephyra-render-ext` 的 CPU 多平面描述符与
//! `ExternalTextureSource::NtHandle`。
//!
//! **关键约束**：D3D11 importer 只接受 NV12（8bit）。因此 `CpuVideoFrame`
//! 统一把 4:2:0 格式转换成 NV12（Y 平面 + 交错 UV），10bit 先降到 8bit。
//! 这样 `import_video_frame()` 在所有后端都有一条真正可行的路径。

use zephyra_render_ext::external_texture::{CpuVideoTextureDesc, ImportedTexture};
use zephyra_render_ext::prelude::{
    CpuTextureDesc, ExternalTextureFormat, ExternalTextureImporter, ExternalTextureSource, FrameId,
    ImportError, RawHandle,
};
use zephyra_video::{VideoFrame, VideoPixelFormat, VideoPlane};

/// 一帧已导入到渲染后端的纹理。
pub type VideoTexture = ImportedTexture;

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

/// 已转换成 NV12、可直接上传的 owned CPU 帧。
///
/// 持有自己的像素缓冲，因此 `desc()` 可以安全地借出 `CpuTextureDesc`。
pub struct CpuVideoFrame {
    /// Y 平面（w*h 字节）。
    y: Vec<u8>,
    /// 交错 UV 平面（w*h/2 字节，U/V 交替）。
    uv: Vec<u8>,
    width: u32,
    height: u32,
}

impl CpuVideoFrame {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 借用两个平面生成 D3D11 importer 可消费的多平面描述。
    pub fn desc(&self) -> CpuVideoTextureDesc<'_> {
        let y_pitch = self.width;
        let uv_pitch = self.width; // 交错 UV 一行 w 字节（w/2 对）
        let y_desc = CpuTextureDesc {
            pixels: &self.y,
            width: self.width,
            height: self.height,
            row_pitch: y_pitch,
            format: ExternalTextureFormat::Nv12,
        };
        let uv_desc = CpuTextureDesc {
            pixels: &self.uv,
            width: self.width / 2,
            height: self.height.div_ceil(2),
            row_pitch: uv_pitch,
            format: ExternalTextureFormat::Nv12,
        };
        CpuVideoTextureDesc {
            planes: [Some(y_desc), Some(uv_desc), None],
            format: ExternalTextureFormat::Nv12,
        }
    }
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

    /// 转成渲染层的外部纹理来源（仅原生帧有句柄）。
    pub fn as_external_source(&self) -> Option<ExternalTextureSource> {
        match self {
            VideoFrameSource::Native { handle, .. } => Some(ExternalTextureSource::NtHandle {
                handle: *handle,
                source_backend: None,
            }),
            VideoFrameSource::Cpu(_) => None,
        }
    }

    /// 把 CPU 帧转换成可直接上传的 NV12。
    pub fn as_cpu_upload(&self) -> Option<CpuVideoFrame> {
        match self {
            VideoFrameSource::Cpu(f) => to_nv12(f),
            VideoFrameSource::Native { .. } => None,
        }
    }
}

/// 把任意 4:2:0 平面格式转换成 NV12（8bit）。
///
/// - I420（Y/U/V）→ Y + 交错 UV
/// - NV12/NV21     → 直接拷贝（NV21 交换 U/V）
/// - YUV420P10     → 每采样右移 2 位降到 8bit，再交错
/// - P010LE        → 16bit 交错 UV 右移 2 位降到 8bit
fn to_nv12(frame: &VideoFrame) -> Option<CpuVideoFrame> {
    let w = frame.coded_size().width as usize;
    let h = frame.coded_size().height as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let layout = frame.layout();
    let planes: Vec<&[u8]> = frame
        .planes()
        .iter()
        .map(|p| match p {
            VideoPlane::Cpu { data } => Some(data.as_slice()),
            VideoPlane::Native { .. } => None,
        })
        .collect::<Option<Vec<_>>>()?;

    let bytes = |i: usize| layout.planes()[i].stride;
    let shift = match frame.format() {
        VideoPixelFormat::I420 | VideoPixelFormat::NV12 | VideoPixelFormat::NV21 => 0,
        VideoPixelFormat::YUV420P10 | VideoPixelFormat::P010LE => 2,
        _ => return None,
    };
    // 16bit little-endian → 8bit：低字节的高 8 位 + 高字节的低 2 位补到高位。
    let sample = |plane: &[u8], i: usize| -> u8 {
        let lo = plane.get(i).copied().unwrap_or(128);
        if shift == 0 {
            lo
        } else {
            let hi = plane.get(i + 1).copied().unwrap_or(0);
            (lo >> 2) | (hi << 6)
        }
    };

    let mut y_out = Vec::with_capacity(w * h);
    let y_stride = bytes(0);
    for row in 0..h {
        for col in 0..w {
            let i = row * y_stride + col * (1 << shift);
            y_out.push(sample(planes[0], i));
        }
    }

    let mut uv = Vec::with_capacity(cw * ch * 2);
    match frame.format() {
        VideoPixelFormat::I420 | VideoPixelFormat::YUV420P10 => {
            let (u, v) = (planes[1], planes[2]);
            let stride = bytes(1);
            for row in 0..ch {
                for col in 0..cw {
                    let i = row * stride + col * 2 * (1 << shift);
                    uv.push(sample(u, i));
                    uv.push(sample(v, i));
                }
            }
        }
        VideoPixelFormat::NV12 | VideoPixelFormat::P010LE => {
            let src = planes[1];
            let stride = bytes(1);
            for row in 0..ch {
                for col in 0..cw {
                    let i = row * stride + col * 2 * (1 << shift);
                    uv.push(sample(src, i));
                    uv.push(sample(src, i + (1 << shift)));
                }
            }
        }
        VideoPixelFormat::NV21 => {
            let src = planes[1];
            let stride = bytes(1);
            for row in 0..ch {
                for col in 0..cw {
                    let i = row * stride + col * 2;
                    uv.push(sample(src, i + 1)); // V
                    uv.push(sample(src, i)); // U
                }
            }
        }
        _ => return None,
    }

    Some(CpuVideoFrame {
        y: y_out,
        uv,
        width: w as u32,
        height: h as u32,
    })
}

/// 把一帧真正导入渲染后端，返回 `VideoTexture`。
///
/// 这是 `VideoTexture` 的生产者：原生帧走共享句柄，CPU 帧统一转 NV12 后走
/// 多平面上传。D3D11/D3D12/Vulkan 的 importer 都实现了对应的入口。
pub fn import_video_frame<I: ExternalTextureImporter>(
    importer: &mut I,
    frame: &VideoFrameSource,
) -> Result<VideoTexture, ImportError> {
    if let Some(source) = frame.as_external_source() {
        let desc = zephyra_render_ext::prelude::ExternalTextureDesc {
            key: format!("video-{}", frame.timestamp()).into(),
            size: (0, 0), // 句柄导入用 desc.source，尺寸由资源自身决定
            format: ExternalTextureFormat::Nv12,
            source,
            sync: None,
        };
        return importer.import(desc);
    }
    let cpu = frame
        .as_cpu_upload()
        .ok_or(ImportError::UnsupportedSource)?;
    importer.import_video_cpu(cpu.desc())
}

/// 把解码帧包装成统一的来源类型。
pub fn from_video_frame(frame: VideoFrame) -> VideoFrameSource {
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

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_video::{Rect, Size};

    fn frame(fmt: VideoPixelFormat, w: u32, h: u32, shift: usize) -> VideoFrame {
        let cw = w as usize / 2;
        let ch = h as usize / 2;
        let b = 1usize << shift;
        let y = vec![0x80u8; w as usize * h as usize * b];
        let u = vec![0x40u8; cw * ch * b];
        let v = vec![0xc0u8; cw * ch * b];
        VideoFrame::from_cpu(
            fmt,
            Size::new(w, h),
            Rect::new(0, 0, w, h),
            Size::new(w, h),
            vec![y, u, v],
            0,
            0,
        )
        .unwrap()
    }

    #[test]
    fn i420_and_10bit_become_nv12() {
        for (fmt, shift) in [
            (VideoPixelFormat::I420, 0),
            (VideoPixelFormat::YUV420P10, 2),
        ] {
            let src = VideoFrameSource::Cpu(frame(fmt, 64, 48, shift));
            let cpu = src.as_cpu_upload().unwrap();
            assert_eq!((cpu.width(), cpu.height()), (64, 48));
            let d = cpu.desc();
            assert_eq!(d.format, ExternalTextureFormat::Nv12);
            assert!(d.planes[0].is_some());
            assert!(d.planes[1].is_some());
            assert_eq!(d.planes[0].unwrap().pixels.len(), 64 * 48);
            assert_eq!(d.planes[1].unwrap().pixels.len(), 32 * 48);
            // UV 是 U,V 交替；10bit 采样右移 2 位降到 8bit。
            let uv = d.planes[1].unwrap().pixels;
            let (expect_u, expect_v) = if shift == 0 {
                (0x40u8, 0xc0u8)
            } else {
                (0x40 >> 2, 0xc0 >> 2)
            };
            assert_eq!((uv[0], uv[1]), (expect_u, expect_v));
        }
    }
}
