//! 契约层基础类型：零平台依赖，不透明句柄。

use std::borrow::Cow;

/// 帧序号（单调递增）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FrameId(pub u64);

/// 不透明句柄：HANDLE / VkImage / fd / 平台资源包装。契约层不解释其含义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct RawHandle(pub usize);

impl RawHandle {
    pub const fn null() -> Self {
        Self(0)
    }
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// 外部纹理像素格式（跨后端统一枚举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ExternalTextureFormat {
    #[default]
    Rgba8UnormSrgb,
    Rgba8Unorm,
    Bgra8UnormSrgb,
    Bgra8Unorm,
    Rgba16Float,
    Rgba32Float,
}

/// 色彩空间。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorSpace {
    #[default]
    Srgb,
    Linear,
    Bt709,
    Bt2020,
    DisplayP3,
}

/// HDR 元数据（最小占位，Phase 2+ 细化）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HdrMetadata {
    pub max_luminance: f32,
    pub min_luminance: f32,
    pub max_content_light_level: f32,
    pub max_frame_average_light_level: f32,
}

/// 2D 矩形（契约层自有，避免强绑 core 内部类型）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

/// 适配器标识（跨平台；Windows 可含 LUID，Vulkan/Linux 可含 UUID）。
/// `zephyra-render-ext` 不依赖 wgpu；转换由应用/feature-gated 层提供（S5/P0-4）。
///
/// wgpu 24.x 应用层可从 `wgpu::AdapterInfo`/hal 映射：`vendor/device`、Windows `device_luid`→`luid`、
/// Vulkan `device_uuid/driver_uuid`。本类型仅承载值，便于框架/后端在同一物理适配器上工作。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AdapterIdentity {
    pub vendor_id: u32,
    pub device_id: u32,
    pub luid: Option<u64>,
    pub device_uuid: Option<[u8; 16]>,
    pub driver_uuid: Option<[u8; 16]>,
    pub name: Option<Cow<'static, str>>,
}

impl AdapterIdentity {
    pub fn matches_physical_device(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (self.luid, other.luid) {
            return a == b;
        }
        if let (Some(a), Some(b)) = (self.device_uuid, other.device_uuid) {
            if a == b {
                return true;
            }
        }
        self.vendor_id == other.vendor_id && self.device_id == other.device_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_identity_matches_luid_uuid() {
        let a = AdapterIdentity {
            luid: Some(1),
            vendor_id: 1,
            device_id: 2,
            ..Default::default()
        };
        let b = AdapterIdentity {
            luid: Some(1),
            vendor_id: 9,
            device_id: 9,
            ..Default::default()
        };
        assert!(a.matches_physical_device(&b));
        let c = AdapterIdentity {
            device_uuid: Some([3; 16]),
            ..Default::default()
        };
        let d = AdapterIdentity {
            device_uuid: Some([3; 16]),
            vendor_id: 4,
            device_id: 5,
            ..Default::default()
        };
        assert!(c.matches_physical_device(&d));
    }

    #[test]
    fn source_backend_has_wgpu() {
        let _ = SourceBackend::Wgpu;
    }

    #[test]
    fn frame_id_ord_and_default() {
        assert_eq!(FrameId::default(), FrameId(0));
        assert!(FrameId(1) < FrameId(2));
        assert_eq!(FrameId(5), FrameId(5));
    }

    #[test]
    fn raw_handle_null_and_eq() {
        assert!(RawHandle::null().is_null());
        assert!(!RawHandle(1).is_null());
        assert_eq!(RawHandle(42), RawHandle(42));
    }

    #[test]
    fn rect_new() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!((r.x, r.y, r.w, r.h), (1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn adapter_identity_vendor_device_fallback() {
        let a = AdapterIdentity {
            vendor_id: 0x10de,
            device_id: 0x1234,
            ..Default::default()
        };
        let b = AdapterIdentity {
            vendor_id: 0x10de,
            device_id: 0x1234,
            name: Some("n".into()),
            ..Default::default()
        };
        assert!(a.matches_physical_device(&b));
        let c = AdapterIdentity {
            vendor_id: 0x10de,
            device_id: 0x9999,
            ..Default::default()
        };
        assert!(!a.matches_physical_device(&c));
    }

    #[test]
    fn formats_and_colorspaces_are_copy() {
        let f = ExternalTextureFormat::Bgra8Unorm;
        let g = f;
        assert_eq!(f, g);
        let c = ColorSpace::DisplayP3;
        assert_eq!(c, ColorSpace::DisplayP3);
    }

    #[test]
    fn source_backends() {
        for b in [
            SourceBackend::D3D11,
            SourceBackend::D3D12,
            SourceBackend::Vulkan,
            SourceBackend::Wgpu,
            SourceBackend::Cpu,
        ] {
            let _ = format!("{b:?}");
        }
    }
}

/// 来源后端提示（可选）。`Wgpu` 表示应用层 wgpu 扩展导出的句柄（框架侧仍按 NT/VkImage/dmabuf 导入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceBackend {
    D3D11,
    D3D12,
    Vulkan,
    Wgpu,
    Cpu,
}
