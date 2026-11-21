#![forbid(unsafe_code)]

//! Render extension contract crate（Phase 1，§17 对齐）。
//! 零平台 API、零 wgpu 依赖；`forbid(unsafe_code)`。

pub mod capability;
pub mod error;
pub mod external_texture;
pub mod lifecycle;
pub mod registration;
pub mod sync;
pub mod types;

pub mod prelude {
    pub use crate::capability::{BackendCapabilities, Capability, CapabilityProvider};
    pub use crate::error::{map_hresult_to_import_error, ImportError};
    pub use crate::external_texture::{
        CpuTextureDesc, CpuVideoTextureDesc, ExternalTextureDesc, ExternalTextureImporter,
        ExternalTextureSource, ImportedTexture,
    };
    pub use crate::lifecycle::{LifecycleContext, RenderLifecycle};
    pub use crate::registration::{
        DrawHook, ExtensionFactory, HookContext, HookContextExt, RenderExtension, RenderSource,
    };
    pub use crate::sync::{FenceHandle, FenceHandleKind, GpuFence, SyncError, SyncInput};
    pub use crate::types::{
        AdapterIdentity, ColorSpace, ExternalTextureFormat, FrameId, HdrMetadata, RawHandle, Rect,
        SourceBackend,
    };
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
