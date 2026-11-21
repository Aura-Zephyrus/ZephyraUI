//! 视频帧 → 渲染契约描述符的转换（2026-11-21 重设计）。
//!
//! 软件后端输出 CPU 平面（I420 / NV12 / YUV420P10），硬件后端输出平台原生句柄。
//! 两类分别映射成 `zephyra-render-ext` 的 `FrameSource::Cpu(Arc<CpuPlanes>)` 与
//! `FrameSource::Native`。**不再**产出不透明 `ImportedTexture`，也不再持有 importer：
//! 本层只负责把解码帧变成纯数据 `FrameDesc`，由应用层 `submit_frame(key, desc, valid_until)`
//! 提交到具体后端。这样 media 与三后端完全解耦，且 `bytes::Bytes` 让解码线程零拷贝交付。

use bytes::Bytes;

use zephyra_render_ext::prelude::{
    ColorSpace, CpuPlanes, ExternalTextureFormat, ExternalTextureSource, FrameDesc, ImportError,
    RawHandle,
};
use zephyra_video::{VideoFrame, VideoPixelFormat, VideoPlane};

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
/// 两个平面都是 `Bytes`：`Vec -> Bytes` 零拷贝转移分配，可跨线程交给渲染器，
/// 也便于一帧同时被多个消费者持有（clone 只加引用计数）。契约层的 `CpuPlanes`
/// 同样以 `Bytes` 存储平面，因此 `planes()` 是一次零拷贝的所有权交付。
pub struct CpuVideoFrame {
    y: Bytes,
    uv: Bytes,
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
    pub fn y_plane(&self) -> &[u8] {
        &self.y
    }
    pub fn uv_plane(&self) -> &[u8] {
        &self.uv
    }

    /// 生成拥有所有权的 NV12 平面（行距：Y = width，交错 UV = width）。
    pub fn planes(&self) -> CpuPlanes {
        CpuPlanes::nv12(self.y.clone(), self.uv.clone(), self.width, self.width)
    }

    /// 生成统一 `FrameDesc`。
    pub fn frame_desc(&self) -> FrameDesc {
        FrameDesc::cpu((self.width, self.height), self.planes())
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

    pub fn as_external_source(&self) -> Option<ExternalTextureSource> {
        match self {
            VideoFrameSource::Native { handle, .. } => Some(ExternalTextureSource::NtHandle {
                handle: *handle,
                source_backend: None,
            }),
            VideoFrameSource::Cpu(_) => None,
        }
    }

    pub fn as_cpu_upload(&self) -> Option<CpuVideoFrame> {
        match self {
            VideoFrameSource::Cpu(f) => to_nv12(f),
            VideoFrameSource::Native { .. } => None,
        }
    }
}

/// 该帧建议使用的纹理 key（应用调用 `submit_frame` 时传入）。
pub fn video_key(frame: &VideoFrameSource) -> String {
    format!("video-{}", frame.timestamp())
}

/// 把任意 4:2:0 平面格式转换成 NV12（8bit）。
///
/// - I420（Y/U/V）→ Y + 交错 UV
/// - NV12/NV21 → 直接拷贝（NV21 交换 U/V）
/// - YUV420P10 → 每采样右移 2 位降到 8bit，再交错
/// - P010LE → 16bit 交错 UV 右移 2 位降到 8bit
fn to_nv12(frame: &VideoFrame) -> Option<CpuVideoFrame> {
    let w = frame.coded_size().width as usize;
    let h = frame.coded_size().height as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let layout = frame.layout();

    // 快速通道：源已是 NV12 且行距紧凑 → 把 video 的 Arc 平面经 `VideoPlane::bytes()`
    // （内部 `Bytes::from_owner`）零拷贝交给契约，Y/UV 都不重新编码。slice 也零拷贝，
    // 仅把可能带填充的缓冲裁到平面语义长度。
    if frame.format() == VideoPixelFormat::NV12
        && layout.planes()[0].stride == w
        && layout.planes()[1].stride == w
    {
        let y_src = frame.planes().first()?.bytes()?;
        let uv_src = frame.planes().get(1)?.bytes()?;
        return Some(CpuVideoFrame {
            y: y_src.slice(0..w * h),
            uv: uv_src.slice(0..cw * ch * 2),
            width: w as u32,
            height: h as u32,
        });
    }

    // `as_ref()` 读取 video 侧的 Arc<[u8]> 平面（无需拷贝）。
    let planes: Vec<&[u8]> = frame
        .planes()
        .iter()
        .map(|p| match p {
            VideoPlane::Cpu { data } => Some(data.as_ref()),
            VideoPlane::Native { .. } => None,
        })
        .collect::<Option<Vec<_>>>()?;

    let bytes = |i: usize| layout.planes()[i].stride;
    let shift = match frame.format() {
        VideoPixelFormat::I420 | VideoPixelFormat::NV12 | VideoPixelFormat::NV21 => 0,
        VideoPixelFormat::YUV420P10 | VideoPixelFormat::P010LE => 2,
        _ => return None,
    };
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
        // Vec -> Bytes 零拷贝转移分配；下游 planes() 再零拷贝交给契约层。
        y: Bytes::from(y_out),
        uv: Bytes::from(uv),
        width: w as u32,
        height: h as u32,
    })
}

/// 把一帧解码结果变成纯数据 [`FrameDesc`]（不再导入，不再有不透明句柄）。
///
/// 原生帧走共享句柄，CPU 帧统一转 NV12 双平面。应用随后调用
/// `ExtContextHandle::submit_frame(video_key(&frame), desc, valid_until)`。
pub fn import_video_frame(frame: &VideoFrameSource) -> Result<FrameDesc, ImportError> {
    if let Some(source) = frame.as_external_source() {
        let (w, h, format) = match frame {
            VideoFrameSource::Native {
                width,
                height,
                format,
                ..
            } => (*width, *height, *format),
            VideoFrameSource::Cpu(_) => unreachable!(),
        };
        let mut desc = FrameDesc::native((w, h), format, source);
        desc.color_space = ColorSpace::Bt709;
        return Ok(desc);
    }
    let cpu = frame
        .as_cpu_upload()
        .ok_or(ImportError::UnsupportedSource)?;
    let mut desc = cpu.frame_desc();
    desc.color_space = ColorSpace::Bt709;
    Ok(desc)
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

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_render_ext::prelude::FrameSource;
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
    fn i420_and_10bit_become_nv12_frame_desc() {
        for (fmt, shift) in [
            (VideoPixelFormat::I420, 0),
            (VideoPixelFormat::YUV420P10, 2),
        ] {
            let src = VideoFrameSource::Cpu(frame(fmt, 64, 48, shift));
            let desc = import_video_frame(&src).unwrap();
            assert_eq!(desc.size, (64, 48));
            assert_eq!(desc.format, ExternalTextureFormat::Nv12);
            let planes = desc.cpu_planes().unwrap();
            assert_eq!(planes.plane(0).unwrap().len(), 64 * 48);
            assert_eq!(planes.plane(1).unwrap().len(), 32 * 48);
            let uv = planes.plane(1).unwrap();
            let (expect_u, expect_v) = if shift == 0 {
                (0x40u8, 0xc0u8)
            } else {
                (0x40 >> 2, 0xc0 >> 2)
            };
            assert_eq!((uv[0], uv[1]), (expect_u, expect_v));
        }
    }

    /// media → 契约跨边界：产出的 FrameDesc 可被下游（mock importer）按 key 消费。
    #[test]
    fn media_frame_desc_crosses_contract_boundary() {
        let src = VideoFrameSource::Cpu(frame(VideoPixelFormat::I420, 64, 48, 0));
        let key = video_key(&src);
        let desc = import_video_frame(&src).unwrap();
        // mock importer：只接收 (key, FrameDesc)，断言字段跨边界完整。
        let received: Vec<(String, FrameDesc)> = vec![(key.clone(), desc.clone())];
        assert_eq!(received[0].0, "video-0");
        assert_eq!(received[0].1.format, ExternalTextureFormat::Nv12);
        match &received[0].1.source {
            FrameSource::Cpu(p) => {
                assert_eq!(p.plane_count(), 2);
                assert_eq!(p.plane(0).unwrap().len(), 64 * 48);
            }
            FrameSource::Native(_) => panic!("CPU frame must produce CPU source"),
            _ => panic!("unexpected source"),
        }
    }

    /// 指针同一性钉住 media → 契约的零拷贝：NV12 紧排行距走快速通道，
    /// video 的 Arc 平面（Bytes::from_owner）与 contract 的 plane(i) 指向同一分配。
    #[test]
    fn nv12_tight_passthrough_is_zero_copy_into_contract() {
        use std::sync::Arc;
        let (w, h) = (8usize, 8usize);
        // 直接以 Arc<[u8]> 造帧（Vec -> Arc 会另分配并拷贝，那一步不属于本路径）。
        let y: Arc<[u8]> = Arc::from(vec![128u8; w * h]);
        let uv: Arc<[u8]> = Arc::from(vec![100u8; w * h / 2]);
        let (y_ptr, uv_ptr) = (y.as_ptr(), uv.as_ptr());
        let src = VideoFrameSource::Cpu(
            VideoFrame::from_cpu(
                VideoPixelFormat::NV12,
                Size::new(w as u32, h as u32),
                Rect::new(0, 0, w as u32, h as u32),
                Size::new(w as u32, h as u32),
                vec![y, uv],
                0,
                0,
            )
            .unwrap(),
        );
        let vframe = match &src {
            VideoFrameSource::Cpu(f) => f,
            _ => unreachable!(),
        };
        let vy_ptr = vframe.planes()[0].bytes().unwrap().as_ptr();
        let vu_ptr = vframe.planes()[1].bytes().unwrap().as_ptr();
        // from_owner 复用原 Vec 的分配，Arc 持有后指针仍不变。
        assert_eq!(vy_ptr, y_ptr);
        assert_eq!(vu_ptr, uv_ptr);

        let desc = import_video_frame(&src).unwrap();
        let planes = desc.cpu_planes().unwrap();
        assert_eq!(planes.plane(0).unwrap().as_ptr(), vy_ptr);
        assert_eq!(planes.plane(1).unwrap().as_ptr(), vu_ptr);
        assert_eq!(planes.plane(0).unwrap().len(), w * h);
        assert_eq!(planes.plane(1).unwrap().len(), w * h / 2);
    }
}
