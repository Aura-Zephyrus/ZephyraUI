//! 统一导出（§3.6）。

pub use crate::debug;
pub use crate::ext_context::{
    ExtContext, ExtContextHandle, ExtContextImpl, ExtError, TextureRegistration,
};
pub use crate::handles::{ExternalTextureHandle, HookHandle};
pub use crate::hook_registry::{HookRegistry, WindowHookRegistry};
pub use crate::texture_builder::{
    CpuTextureDescOwned, ExternalTextureBuilder, SyncInput, TextureSourceInput,
};

pub use zephyra_render_ext::prelude::{
    AdapterIdentity, BackendCapabilities, Capability, ColorSpace, CpuTextureDesc, DrawHook,
    ExtensionFactory, ExternalTextureDesc, ExternalTextureFormat, ExternalTextureImporter,
    ExternalTextureSource, FrameId, GpuFence, HdrMetadata, HookContext, HookContextExt,
    ImportError, ImportedTexture, LifecycleContext, RawHandle, Rect, RenderExtension,
    RenderLifecycle, RenderSource, SourceBackend, SyncError,
};

#[cfg(windows)]
pub use crate::platform::d3d11::D3D11HookContext;
#[cfg(windows)]
pub use crate::platform::d3d12::D3D12HookContext;
#[cfg(target_os = "linux")]
pub use crate::platform::vulkan::VulkanHookContext;
