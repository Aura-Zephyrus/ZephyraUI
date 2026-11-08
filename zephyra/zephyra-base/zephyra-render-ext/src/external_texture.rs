//! 外部纹理契约（§4.1，按 §17 P0-1/S1/S6/S9 修正）。

use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;

use crate::error::ImportError;
use crate::sync::GpuFence;
use crate::types::{ExternalTextureFormat, FrameId, RawHandle, SourceBackend};

/// CPU 纹理上传描述（借用；后端按需拷贝）。
#[derive(Clone, Copy, Debug)]
pub struct CpuTextureDesc<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: ExternalTextureFormat,
}

/// 跨平台统一描述符。
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

/// 源类型：平台句柄使用不透明 `RawHandle`（P0-1）。
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

/// 导入结果（帧级租借；`valid_until` 语义见 S9）。
pub struct ImportedTexture {
    pub handle: Box<dyn Any + Send + Sync>,
    pub valid_until: FrameId,
}

impl ImportedTexture {
    pub fn new(handle: impl Any + Send + Sync, valid_until: FrameId) -> Self {
        Self {
            handle: Box::new(handle),
            valid_until,
        }
    }
}

/// 统一入口：后端按 source 分发（S1/P0-1）。
pub trait ExternalTextureImporter {
    fn import(&mut self, desc: ExternalTextureDesc) -> Result<ImportedTexture, ImportError>;
    fn import_cpu(&mut self, desc: CpuTextureDesc<'_>) -> Result<ImportedTexture, ImportError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RawHandle;

    #[test]
    fn cpu_desc_copy() {
        let px = [0u8; 4];
        let d = CpuTextureDesc {
            pixels: &px,
            width: 1,
            height: 1,
            row_pitch: 4,
            format: ExternalTextureFormat::Rgba8Unorm,
        };
        let d2 = d;
        assert_eq!(d2.width, 1);
    }

    #[test]
    fn imported_texture_any() {
        let t = ImportedTexture::new(42u32, FrameId(7));
        assert_eq!(t.valid_until, FrameId(7));
        assert_eq!(t.handle.downcast_ref::<u32>(), Some(&42));
    }

    #[test]
    fn raw_handle_null() {
        assert!(RawHandle::null().is_null());
        assert!(!RawHandle(1).is_null());
    }

    #[test]
    fn importer_contract_mock() {
        use std::borrow::Cow;
        struct MockImporter;
        impl ExternalTextureImporter for MockImporter {
            fn import(
                &mut self,
                desc: ExternalTextureDesc,
            ) -> Result<ImportedTexture, ImportError> {
                match desc.source {
                    ExternalTextureSource::NtHandle { .. }
                    | ExternalTextureSource::VkImage { .. } => {
                        Ok(ImportedTexture::new(desc.key.into_owned(), FrameId(1)))
                    }
                    #[cfg(target_os = "linux")]
                    ExternalTextureSource::Dmabuf { .. } => {
                        Ok(ImportedTexture::new((), FrameId(1)))
                    }
                }
            }
            fn import_cpu(
                &mut self,
                _desc: CpuTextureDesc<'_>,
            ) -> Result<ImportedTexture, ImportError> {
                Ok(ImportedTexture::new("cpu".to_string(), FrameId(2)))
            }
        }
        let mut m = MockImporter;
        let tex = m
            .import(ExternalTextureDesc {
                key: Cow::Borrowed("k"),
                size: (1, 1),
                format: ExternalTextureFormat::Rgba8Unorm,
                source: ExternalTextureSource::NtHandle {
                    handle: RawHandle(1),
                    source_backend: Some(SourceBackend::Wgpu),
                },
                sync: None,
            })
            .unwrap();
        assert_eq!(
            tex.handle.downcast_ref::<String>().map(String::as_str),
            Some("k")
        );
        let vk = m
            .import(ExternalTextureDesc {
                key: Cow::Borrowed("vk"),
                size: (2, 2),
                format: ExternalTextureFormat::Bgra8Unorm,
                source: ExternalTextureSource::VkImage {
                    image: RawHandle(9),
                    layout: 0,
                    memory: Some(RawHandle(10)),
                },
                sync: None,
            })
            .unwrap();
        assert_eq!(
            vk.handle.downcast_ref::<String>().map(String::as_str),
            Some("vk")
        );
        let cpu = m
            .import_cpu(CpuTextureDesc {
                pixels: &[0, 0, 0, 255],
                width: 1,
                height: 1,
                row_pitch: 4,
                format: ExternalTextureFormat::Rgba8Unorm,
            })
            .unwrap();
        assert_eq!(cpu.valid_until, FrameId(2));
    }

    #[test]
    fn external_texture_desc_debug_hides_fence() {
        let desc = ExternalTextureDesc {
            key: Cow::Borrowed("k"),
            size: (1, 1),
            format: ExternalTextureFormat::Rgba8Unorm,
            source: ExternalTextureSource::VkImage {
                image: RawHandle(1),
                layout: 0,
                memory: None,
            },
            sync: None,
        };
        let s = format!("{desc:?}");
        assert!(s.contains("ExternalTextureDesc"));
        assert!(s.contains("sync"));
    }
}
