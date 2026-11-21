//! 统一帧契约（§4.1，2026-11-21 重设计）。
//!
//! 后端无关：一帧 = 描述符（[`FrameDesc`]）+ 来源（[`FrameSource`]）+ 有效期
//! （`valid_until`）。旧的 `Box<dyn Any>` 不透明句柄与 `ExternalTextureImporter`
//! trait 已删除：扩展资源由后端直接产出强类型 `ExtResource`，跨契约传递的只有
//! 元数据与拥有所有权的像素平面。
//!
//! CPU 平面用 `bytes::Bytes`：Arc-backed 引用计数、跨线程、`Vec::into()` 与
//! `slice` 都零拷贝，正是解码线程 → UI 线程交付视频帧需要的形状。采集端
//! （camera 的 NV12 平面）本就是 `Bytes`，可直接直达契约，无须再 memcpy 到 `Arc`。

use bytes::Bytes;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::sync::GpuFence;
use crate::types::{ColorSpace, ExternalTextureFormat, FrameId, RawHandle, SourceBackend};

// ---------------------------------------------------------------------------
// 新契约：拥有所有权的帧描述（跨线程、零拷贝、生命周期可判定）
// ---------------------------------------------------------------------------

/// CPU 多平面像素（拥有所有权）。
///
/// - `planes[0..3]`：平面 0=Y/单平面 RGBA，1=UV/U，2=V；
/// - `format` 决定平面数量与语义：RGBA 单平面、NV12/P010 两平面、I420 三平面；
/// - `row_pitch[i]` 与 `planes[i]` 一一对应（字节/行）。
///
/// 用 `Bytes` 而非 `Vec<u8>`：`Vec -> Bytes` 零拷贝转移分配，`Bytes::slice` 零拷贝
/// 切平面，且一份解码帧可以同时递给多个消费者（Hook、渲染器、录制），不需要在 UI
/// 线程再拷一次。`Bytes` 也是 Arc-backed，clone 只加引用计数。
#[derive(Clone, Debug, Default)]
pub struct CpuPlanes {
    pub planes: [Option<Bytes>; 3],
    pub format: ExternalTextureFormat,
    pub row_pitch: [u32; 3],
}

impl CpuPlanes {
    /// 单平面 RGBA8。
    pub fn rgba(pixels: impl Into<Bytes>, row_pitch: u32) -> Self {
        Self {
            planes: [Some(pixels.into()), None, None],
            format: ExternalTextureFormat::Rgba8UnormSrgb,
            row_pitch: [row_pitch, 0, 0],
        }
    }

    /// 双平面 NV12（Y + 交错 UV）。
    pub fn nv12(y: impl Into<Bytes>, uv: impl Into<Bytes>, y_pitch: u32, uv_pitch: u32) -> Self {
        Self {
            planes: [Some(y.into()), Some(uv.into()), None],
            format: ExternalTextureFormat::Nv12,
            row_pitch: [y_pitch, uv_pitch, 0],
        }
    }

    /// 三平面 I420（Y + U + V）。
    pub fn i420(
        y: impl Into<Bytes>,
        u: impl Into<Bytes>,
        v: impl Into<Bytes>,
        y_pitch: u32,
        uv_pitch: u32,
    ) -> Self {
        Self {
            planes: [Some(y.into()), Some(u.into()), Some(v.into())],
            format: ExternalTextureFormat::I420,
            row_pitch: [y_pitch, uv_pitch, uv_pitch],
        }
    }

    /// 第 `i` 个平面的字节切片。
    pub fn plane(&self, i: usize) -> Option<&[u8]> {
        self.planes.get(i).and_then(|p| p.as_deref())
    }

    /// 平面数量（由 `format` 决定，最多 3）。
    pub fn plane_count(&self) -> usize {
        match self.format {
            ExternalTextureFormat::Nv12 | ExternalTextureFormat::P010 => 2,
            ExternalTextureFormat::I420 | ExternalTextureFormat::I420P10 => 3,
            _ => 1,
        }
    }
}

/// 帧来源。
///
/// `Native` 只携带平台句柄（契约层不解释）；`Cpu` 携带拥有所有权的平面，
/// 供三后端的 NV12 / RGBA 上传路径消费。
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FrameSource {
    Native(ExternalTextureSource),
    Cpu(Arc<CpuPlanes>),
}

/// 跨平台统一帧描述符。
///
/// `key` 与 `valid_until` 由 `submit_frame(key, desc, valid_until)` 单独给出 ——
/// 描述符只回答「这一帧长什么样、从哪来、什么时候同步」，不回答「放在哪个槽、
/// 活到哪一帧」。这样同一份 `FrameDesc` 可以重用到不同槽位。
#[derive(Clone)]
pub struct FrameDesc {
    pub size: (u32, u32),
    pub format: ExternalTextureFormat,
    pub source: FrameSource,
    pub sync: Option<Arc<dyn GpuFence>>,
    pub color_space: ColorSpace,
}

impl FrameDesc {
    /// 从平台原生句柄构造（无同步、sRGB）。
    pub fn native(
        size: (u32, u32),
        format: ExternalTextureFormat,
        source: ExternalTextureSource,
    ) -> Self {
        Self {
            size,
            format,
            source: FrameSource::Native(source),
            sync: None,
            color_space: ColorSpace::Srgb,
        }
    }

    /// 从 CPU 平面构造（无同步、sRGB）。
    pub fn cpu(size: (u32, u32), planes: CpuPlanes) -> Self {
        let format = planes.format;
        Self {
            size,
            format,
            source: FrameSource::Cpu(Arc::new(planes)),
            sync: None,
            color_space: ColorSpace::Srgb,
        }
    }

    /// RGBA8 单平面便捷构造。
    pub fn rgba(pixels: impl Into<Bytes>, width: u32, height: u32, row_pitch: u32) -> Self {
        Self::cpu((width, height), CpuPlanes::rgba(pixels, row_pitch))
    }

    /// NV12 双平面便捷构造。
    pub fn nv12(
        y: impl Into<Bytes>,
        uv: impl Into<Bytes>,
        width: u32,
        height: u32,
        y_pitch: u32,
        uv_pitch: u32,
    ) -> Self {
        Self::cpu((width, height), CpuPlanes::nv12(y, uv, y_pitch, uv_pitch))
    }

    /// CPU 平面引用（若来源是 CPU）。
    pub fn cpu_planes(&self) -> Option<&CpuPlanes> {
        match &self.source {
            FrameSource::Cpu(p) => Some(p),
            FrameSource::Native(_) => None,
        }
    }
}

impl core::fmt::Debug for FrameDesc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameDesc")
            .field("size", &self.size)
            .field("format", &self.format)
            .field("source", &self.source)
            .field("sync", &self.sync.as_ref().map(|_| "<fence>"))
            .field("color_space", &self.color_space)
            .finish()
    }
}

/// 一次 `submit_frame` 的结果。
///
/// `valid_until` 是**真正生效**的值：渲染器每帧用它拒绝过期纹理，过期后 SRV
/// 从纹理解析中消失，图像不再出现，并计入 `FrameDiagnostics::expired_frames`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredTexture {
    pub key: Cow<'static, str>,
    pub valid_until: FrameId,
}

/// 帧诊断计数（跨后端一致的失败可见性）。
///
/// `RenderReport`（位于 `zephyra-vector` git 依赖）已有 `missing_textures`；
/// 新增计数放在本契约层，由后端在帧边界累加，debug 构建下同时打进调试桥。
#[derive(Debug, Default)]
pub struct FrameDiagnostics {
    missing_textures: AtomicU32,
    expired_frames: AtomicU32,
    import_failures: AtomicU32,
    unsupported_format: AtomicU32,
}

impl FrameDiagnostics {
    pub fn note_missing_texture(&self) {
        self.missing_textures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn note_expired(&self, n: u32) {
        self.expired_frames.fetch_add(n, Ordering::Relaxed);
    }
    pub fn note_import_failure(&self) {
        self.import_failures.fetch_add(1, Ordering::Relaxed);
    }
    pub fn note_unsupported_format(&self) {
        self.unsupported_format.fetch_add(1, Ordering::Relaxed);
    }

    pub fn missing_textures(&self) -> u32 {
        self.missing_textures.load(Ordering::Relaxed)
    }
    pub fn expired_frames(&self) -> u32 {
        self.expired_frames.load(Ordering::Relaxed)
    }
    pub fn import_failures(&self) -> u32 {
        self.import_failures.load(Ordering::Relaxed)
    }
    pub fn unsupported_format(&self) -> u32 {
        self.unsupported_format.load(Ordering::Relaxed)
    }

    /// 只读快照（不清零，供调试桥随时读取）。
    pub fn snapshot(&self) -> FrameDiagnosticsSnapshot {
        FrameDiagnosticsSnapshot {
            missing_textures: self.missing_textures.load(Ordering::Relaxed),
            expired_frames: self.expired_frames.load(Ordering::Relaxed),
            import_failures: self.import_failures.load(Ordering::Relaxed),
            unsupported_format: self.unsupported_format.load(Ordering::Relaxed),
        }
    }

    /// 采样并清零（每帧调用一次，得到本帧计数）。
    pub fn take(&self) -> FrameDiagnosticsSnapshot {
        FrameDiagnosticsSnapshot {
            missing_textures: self.missing_textures.swap(0, Ordering::Relaxed),
            expired_frames: self.expired_frames.swap(0, Ordering::Relaxed),
            import_failures: self.import_failures.swap(0, Ordering::Relaxed),
            unsupported_format: self.unsupported_format.swap(0, Ordering::Relaxed),
        }
    }
}

/// 帧有效期账本（三后端共用的契约级逻辑，零平台依赖、可无 GPU 单测）。
///
/// 后端把 `submit_frame` 的 `valid_until` 记进来，每帧调 [`Self::begin_frame`]：
/// 过期的 key 被剔除并计入 `FrameDiagnostics::expired_frames`，此后 `is_live` 为假，
/// 纹理解析自然拿不到它 —— 图像消失、可见地失败，而不是画一帧旧内容。
#[derive(Debug, Default)]
pub struct FrameLifetime {
    valid_until: HashMap<String, FrameId>,
    current: FrameId,
}

impl FrameLifetime {
    /// 登记/替换一个 key 的有效期，返回是否为新 key。
    pub fn register(&mut self, key: impl Into<String>, valid_until: FrameId) -> bool {
        self.valid_until.insert(key.into(), valid_until).is_none()
    }

    pub fn remove(&mut self, key: &str) {
        self.valid_until.remove(key);
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.valid_until.contains_key(key)
    }

    pub fn is_live(&self, key: &str) -> bool {
        self.valid_until
            .get(key)
            .is_some_and(|until| *until > self.current)
    }

    pub fn current(&self) -> FrameId {
        self.current
    }

    /// 帧边界：推进帧号、剔除过期项，返回被剔除的 key。
    pub fn begin_frame(&mut self, frame: FrameId, diagnostics: &FrameDiagnostics) -> Vec<String> {
        self.current = frame;
        let mut expired = Vec::new();
        self.valid_until.retain(|key, until| {
            if *until > frame {
                true
            } else {
                expired.push(key.clone());
                false
            }
        });
        if !expired.is_empty() {
            diagnostics.note_expired(expired.len() as u32);
        }
        expired
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameDiagnosticsSnapshot {
    pub missing_textures: u32,
    pub expired_frames: u32,
    pub import_failures: u32,
    pub unsupported_format: u32,
}

// ---------------------------------------------------------------------------
// 兼容视图：借用型描述（构建器 / 旧调用点仍用；新代码请用上面的 owned 类型）
// ---------------------------------------------------------------------------

/// 源类型：平台句柄使用不透明 `RawHandle`（契约层不解释其含义）。
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ExternalTextureSource {
    NtHandle {
        handle: RawHandle,
        source_backend: Option<SourceBackend>,
    },
    VkImage {
        image: RawHandle,
        layout: u32,
        memory: Option<RawHandle>,
    },
    #[cfg(target_os = "linux")]
    Dmabuf {
        fd: i32,
        modifier: u64,
        planes: u32,
        offsets: [u32; 4],
        strides: [u32; 4],
    },
}

/// 借用型 CPU 纹理描述（供构建器与旧入口使用）。
#[derive(Clone, Copy, Debug)]
pub struct CpuTextureDesc<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: ExternalTextureFormat,
}

/// 借用型多平面 CPU 描述。
#[derive(Clone, Copy, Debug)]
pub struct CpuVideoTextureDesc<'a> {
    pub planes: [Option<CpuTextureDesc<'a>>; 3],
    pub format: ExternalTextureFormat,
}

impl<'a> CpuVideoTextureDesc<'a> {
    /// 转成拥有所有权的 [`CpuPlanes`]（跨线程交付）。
    pub fn to_owned_planes(&self) -> Option<CpuPlanes> {
        if self.planes.iter().all(|p| p.is_none()) {
            return None;
        }
        let mut planes = [None, None, None];
        let mut pitch = [0u32; 3];
        for i in 0..3 {
            if let Some(p) = self.planes[i] {
                // 借用型 legacy 视图只能复制（输入是 &[u8]）；新调用点请直接构造
                // 拥有所有权的 `CpuPlanes`，避免这一步拷贝。
                planes[i] = Some(Bytes::copy_from_slice(p.pixels));
                pitch[i] = p.row_pitch;
            }
        }
        Some(CpuPlanes {
            planes,
            format: self.format,
            row_pitch: pitch,
        })
    }
}

/// 句柄型外部纹理描述（构建器 `try_into_handle_desc` 仍返回它）。
pub struct ExternalTextureDesc {
    pub key: Cow<'static, str>,
    pub size: (u32, u32),
    pub format: ExternalTextureFormat,
    pub source: ExternalTextureSource,
    pub sync: Option<Arc<dyn GpuFence>>,
}

impl core::fmt::Debug for ExternalTextureDesc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternalTextureDesc")
            .field("key", &self.key)
            .field("size", &self.size)
            .field("format", &self.format)
            .field("source", &self.source)
            .field("sync", &self.sync.as_ref().map(|_| "<fence>"))
            .finish()
    }
}

// `ImportError` 仍由 ext / 后端使用；在此重导出保持路径稳定。
pub use crate::error::ImportError as TextureImportError;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RawHandle, SourceBackend};

    #[test]
    fn cpu_planes_rgba_zero_copy_from_vec() {
        let v = vec![255u8; 16];
        let planes = CpuPlanes::rgba(v, 16);
        assert_eq!(planes.plane_count(), 1);
        assert_eq!(planes.plane(0).unwrap().len(), 16);
        assert_eq!(planes.row_pitch[0], 16);
        assert!(planes.plane(1).is_none());
    }

    #[test]
    fn cpu_planes_nv12_two_planes() {
        let planes = CpuPlanes::nv12(vec![0u8; 64], vec![128u8; 32], 8, 8);
        assert_eq!(planes.format, ExternalTextureFormat::Nv12);
        assert_eq!(planes.plane_count(), 2);
        assert_eq!(planes.plane(0).unwrap().len(), 64);
        assert_eq!(planes.plane(1).unwrap().len(), 32);
        assert!(planes.plane(2).is_none());
    }

    #[test]
    fn cpu_planes_i420_three_planes() {
        let planes = CpuPlanes::i420(vec![0u8; 64], vec![1u8; 16], vec![2u8; 16], 8, 4);
        assert_eq!(planes.format, ExternalTextureFormat::I420);
        assert_eq!(planes.plane_count(), 3);
        assert_eq!(planes.plane(2).unwrap()[0], 2);
    }

    #[test]
    fn frame_desc_cpu_and_native() {
        let d = FrameDesc::nv12(vec![0u8; 64], vec![0u8; 32], 8, 8, 8, 8);
        assert_eq!(d.size, (8, 8));
        assert_eq!(d.format, ExternalTextureFormat::Nv12);
        assert!(d.cpu_planes().is_some());
        let n = FrameDesc::native(
            (4, 4),
            ExternalTextureFormat::Bgra8Unorm,
            ExternalTextureSource::NtHandle {
                handle: RawHandle(1),
                source_backend: Some(SourceBackend::Wgpu),
            },
        );
        assert!(n.cpu_planes().is_none());
        assert!(format!("{n:?}").contains("FrameDesc"));
    }

    #[test]
    fn registered_texture_carries_lifetime() {
        let r = RegisteredTexture {
            key: Cow::Borrowed("k"),
            valid_until: FrameId(7),
        };
        assert_eq!(r.valid_until, FrameId(7));
    }

    #[test]
    fn diagnostics_counts_and_take() {
        let d = FrameDiagnostics::default();
        d.note_missing_texture();
        d.note_expired(2);
        d.note_import_failure();
        d.note_unsupported_format();
        let s = d.take();
        assert_eq!(s.missing_textures, 1);
        assert_eq!(s.expired_frames, 2);
        assert_eq!(s.import_failures, 1);
        assert_eq!(s.unsupported_format, 1);
        assert_eq!(d.take(), FrameDiagnosticsSnapshot::default());
    }

    #[test]
    fn legacy_view_converts_to_owned() {
        let y = [0u8; 4];
        let uv = [1u8; 2];
        let view = CpuVideoTextureDesc {
            planes: [
                Some(CpuTextureDesc {
                    pixels: &y,
                    width: 2,
                    height: 2,
                    row_pitch: 2,
                    format: ExternalTextureFormat::Nv12,
                }),
                Some(CpuTextureDesc {
                    pixels: &uv,
                    width: 1,
                    height: 2,
                    row_pitch: 2,
                    format: ExternalTextureFormat::Nv12,
                }),
                None,
            ],
            format: ExternalTextureFormat::Nv12,
        };
        let owned = view.to_owned_planes().unwrap();
        assert_eq!(owned.plane(0).unwrap(), &[0, 0, 0, 0]);
        assert_eq!(owned.plane(1).unwrap(), &[1, 1]);
    }

    #[test]
    fn raw_handle_null() {
        assert!(RawHandle::null().is_null());
        assert!(!RawHandle(1).is_null());
    }

    /// 生命周期负向测试（不依赖 GPU）：valid_until 设为第 10 帧，
    /// 推进到第 11 帧时资源过期、被剔除、计入 expired_frames。
    #[test]
    fn frame_lifetime_expires_on_next_frame() {
        let d = FrameDiagnostics::default();
        let mut life = FrameLifetime::default();
        life.register("cam", FrameId(10));
        assert!(life.is_live("cam"));
        for f in 1..10 {
            let expired = life.begin_frame(FrameId(f), &d);
            assert!(expired.is_empty(), "第 {f} 帧仍应有效");
        }
        let expired = life.begin_frame(FrameId(10), &d);
        assert_eq!(expired, vec!["cam".to_string()]);
        assert!(
            !life.is_live("cam"),
            "第 10 帧（== valid_until）必须不再可解析"
        );
        assert_eq!(d.snapshot().expired_frames, 1);
        assert!(life.begin_frame(FrameId(11), &d).is_empty());
    }

    /// 无 GPU 的 cpu_only 路径断言：1080p NV12 双平面走统一 FrameDesc，
    /// 参考 BT.709 软件采样中心像素非黑。
    ///
    /// 真机像素门禁（D3D11 1080p NV12 → submit_frame → IMAGE_TEXTURE=key →
    /// ZEPHYRA_FRAME_DUMP 回读）**待补**：需要可连接 GPU 的 Windows 机器与
    /// gallery 应用；本测试只保证跨契约像素与平面形状正确。
    #[test]
    fn cpu_only_1080p_nv12_center_pixel_non_black() {
        let (w, h) = (1920usize, 1080usize);
        let y = vec![128u8; w * h];
        let uv = vec![128u8; (w / 2) * (h / 2) * 2];
        let desc = FrameDesc::nv12(y, uv, w as u32, h as u32, w as u32, w as u32);
        assert_eq!(desc.size, (1920, 1080));
        let planes = desc.cpu_planes().unwrap();
        assert_eq!(planes.plane_count(), 2);
        assert_eq!(planes.plane(0).unwrap().len(), w * h);
        assert_eq!(planes.plane(1).unwrap().len(), (w / 2) * (h / 2) * 2);

        let (cx, cy) = (w / 2, h / 2);
        let yy = planes.plane(0).unwrap()[cy * w + cx] as f32;
        let uv_idx = (cy / 2) * w + cx;
        let u = planes.plane(1).unwrap()[uv_idx] as f32 - 128.0;
        let v = planes.plane(1).unwrap()[uv_idx + 1] as f32 - 128.0;
        let r = yy + 1.5748 * v;
        let g = yy - 0.1873 * u - 0.4681 * v;
        let b = yy + 1.8556 * u;
        assert!(r >= 16.0 && g >= 16.0 && b >= 16.0, "中心像素不能是黑色");
    }

    /// 1080p 稳态零分配（等价分配计数测试）：Bytes 克隆 1000 次仍指向同一份平面，
    /// 每帧提交不复制像素。
    #[test]
    fn frame_desc_bytes_clone_is_zero_copy() {
        let desc = FrameDesc::nv12(
            vec![0u8; 1920 * 1080],
            vec![128u8; 1920 * 540],
            1920,
            1080,
            1920,
            1920,
        );
        let first = desc.cpu_planes().unwrap().clone();
        for _ in 0..1000 {
            let next = first.clone();
            assert_eq!(
                first.planes[0].as_ref().unwrap().as_ptr(),
                next.planes[0].as_ref().unwrap().as_ptr()
            );
            assert_eq!(
                first.planes[1].as_ref().unwrap().as_ptr(),
                next.planes[1].as_ref().unwrap().as_ptr()
            );
        }
    }

    /// 指针同一性钉住零拷贝：Bytes 平面进入 `CpuPlanes` 后，`plane(i)` 的字节
    /// 首地址必须与原始 `Bytes::as_ptr()` 完全相同 —— 若将来有人把存储换回
    /// `Arc<[u8]>` 并在构造时 memcpy，这条测试立刻变红。
    #[test]
    fn cpu_planes_bytes_pointer_identity_is_zero_copy() {
        let y: Bytes = vec![128u8; 16].into();
        let uv: Bytes = vec![100u8; 8].into();
        let (y_ptr, uv_ptr) = (y.as_ptr(), uv.as_ptr());
        let planes = CpuPlanes::nv12(y.clone(), uv.clone(), 4, 4);
        assert_eq!(planes.plane(0).unwrap().as_ptr(), y_ptr);
        assert_eq!(planes.plane(1).unwrap().as_ptr(), uv_ptr);
        // clone 只加引用计数，指针仍不变。
        let cloned = planes.clone();
        assert_eq!(cloned.plane(0).unwrap().as_ptr(), y_ptr);
        assert_eq!(cloned.plane(1).unwrap().as_ptr(), uv_ptr);
    }

    /// `Bytes::from_owner(Arc<[u8]>)` 的指针同一性：把 Arc 的所有权交给 Bytes
    /// 不会产生任何 memcpy —— 这是 video 侧 `cpu_arc()` 直达契约的理论依据。
    #[test]
    fn bytes_from_owner_arc_preserves_pointer() {
        let arc: Arc<[u8]> = Arc::from(vec![7u8; 32]);
        let ptr = arc.as_ptr();
        let bytes = Bytes::from_owner(arc);
        assert_eq!(bytes.as_ptr(), ptr);
        let planes = CpuPlanes::rgba(bytes, 32);
        assert_eq!(planes.plane(0).unwrap().as_ptr(), ptr);
    }
}
