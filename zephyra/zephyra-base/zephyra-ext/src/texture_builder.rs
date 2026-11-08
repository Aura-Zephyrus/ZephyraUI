//! 外部纹理构建器（§3.3，S4/S6/S7）。

use std::borrow::Cow;
use std::sync::Arc;

use zephyra_render_ext::prelude::{
    ColorSpace, ExternalTextureDesc, ExternalTextureFormat, ExternalTextureSource, GpuFence,
    HdrMetadata, RawHandle,
};

/// 构建器层 CPU 描述（持有，便于延迟注册）。
#[derive(Clone, Debug)]
pub struct CpuTextureDescOwned {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: ExternalTextureFormat,
}

/// 来源输入：构建器层（可含 `Auto`），注册时解析。
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum TextureSourceInput {
    #[default]
    Auto,
    NtHandle(RawHandle),
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
    CpuOwned(CpuTextureDescOwned),
}

/// 同步输入（S4）。
#[derive(Clone, Default)]
pub enum SyncInput {
    #[default]
    Auto,
    Fence(Arc<dyn GpuFence>),
    None,
}

impl core::fmt::Debug for SyncInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SyncInput::Auto => write!(f, "Auto"),
            SyncInput::Fence(_) => write!(f, "Fence(..)"),
            SyncInput::None => write!(f, "None"),
        }
    }
}

/// 外部纹理构建器：链式 API。
pub struct ExternalTextureBuilder {
    pub(crate) key: Cow<'static, str>,
    pub(crate) size: (u32, u32),
    pub(crate) format: ExternalTextureFormat,
    pub(crate) source: TextureSourceInput,
    pub(crate) sync: SyncInput,
    pub(crate) color_space: Option<ColorSpace>,
    pub(crate) hdr_metadata: Option<HdrMetadata>,
}

impl ExternalTextureBuilder {
    pub fn new(key: impl Into<Cow<'static, str>>, size: (u32, u32)) -> Self {
        Self {
            key: key.into(),
            size,
            format: ExternalTextureFormat::Rgba8UnormSrgb,
            source: TextureSourceInput::Auto,
            sync: SyncInput::Auto,
            color_space: None,
            hdr_metadata: None,
        }
    }
    pub fn format(mut self, fmt: ExternalTextureFormat) -> Self {
        self.format = fmt;
        self
    }
    pub fn source_auto(mut self, source: TextureSourceInput) -> Self {
        self.source = source;
        self
    }
    pub fn source_cpu(mut self, desc: CpuTextureDescOwned) -> Self {
        self.source = TextureSourceInput::CpuOwned(desc);
        self
    }
    pub fn sync_auto(mut self, sync: SyncInput) -> Self {
        self.sync = sync;
        self
    }
    pub fn color_space(mut self, cs: ColorSpace) -> Self {
        self.color_space = Some(cs);
        self
    }
    pub fn hdr_metadata(mut self, meta: HdrMetadata) -> Self {
        self.hdr_metadata = Some(meta);
        self
    }

    /// 供后端 `ExtContextImpl` 消费（导入/注册）。
    pub fn into_parts(self) -> (Cow<'static, str>, BuilderParts) {
        (
            self.key,
            BuilderParts {
                size: self.size,
                format: self.format,
                source: self.source,
                sync: self.sync,
                color_space: self.color_space,
                hdr_metadata: self.hdr_metadata,
            },
        )
    }
}

/// 构建器解析后的部件（供后端实现使用）。
#[derive(Clone, Debug)]
pub struct BuilderParts {
    pub size: (u32, u32),
    pub format: ExternalTextureFormat,
    pub source: TextureSourceInput,
    pub sync: SyncInput,
    pub color_space: Option<ColorSpace>,
    pub hdr_metadata: Option<HdrMetadata>,
}

impl BuilderParts {
    /// 解析为契约层 `ExternalTextureDesc`（CPU 源需后端走 `import_cpu`；此处仅映射句柄源）。
    pub fn try_into_handle_desc(&self, key: Cow<'static, str>) -> Option<ExternalTextureDesc> {
        let (source, fence) = match &self.source {
            TextureSourceInput::NtHandle(h) => (
                ExternalTextureSource::NtHandle {
                    handle: *h,
                    source_backend: None,
                },
                self.sync_fence(),
            ),
            TextureSourceInput::VkImage {
                image,
                layout,
                memory,
            } => (
                ExternalTextureSource::VkImage {
                    image: *image,
                    layout: *layout,
                    memory: *memory,
                },
                self.sync_fence(),
            ),
            #[cfg(target_os = "linux")]
            TextureSourceInput::Dmabuf {
                fd,
                modifier,
                planes,
                offsets,
                strides,
            } => (
                ExternalTextureSource::Dmabuf {
                    fd: *fd,
                    modifier: *modifier,
                    planes: *planes,
                    offsets: *offsets,
                    strides: *strides,
                },
                self.sync_fence(),
            ),
            TextureSourceInput::Auto | TextureSourceInput::CpuOwned(_) => return None,
        };
        Some(ExternalTextureDesc {
            key,
            size: self.size,
            format: self.format,
            source,
            sync: fence,
        })
    }
    pub fn sync_fence(&self) -> Option<Arc<dyn GpuFence>> {
        match &self.sync {
            SyncInput::Fence(f) => Some(f.clone()),
            SyncInput::Auto | SyncInput::None => None,
        }
    }
    pub fn cpu_owned(&self) -> Option<&CpuTextureDescOwned> {
        match &self.source {
            TextureSourceInput::CpuOwned(c) => Some(c),
            _ => None,
        }
    }
}
