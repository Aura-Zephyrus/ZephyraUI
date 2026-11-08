#![forbid(unsafe_code)]
//! 统一门面扩展 API（`zephyra-ext`）。
//!
//! 应用只依赖本 crate + `zephyra-render-ext` 的公共类型；平台差异由框架内部路由。
//! Phase 1：`ExtContextHandle` 单线程（`!Send + !Sync`），仅 UI/渲染线程使用。

pub mod debug;
pub mod ext_context;
pub mod fallback;
pub mod handles;
pub mod hook_registry;
pub mod platform;
pub mod prelude;
pub mod texture_builder;

pub use ext_context::{
    ExtContext, ExtContextHandle, ExtContextImpl, ExtError, TextureRegistration,
};
pub use fallback::{Backend, BackendScore, DeviceCapabilityCache, FallbackPolicy, FeatureStatus};
pub use handles::{ExternalTextureHandle, HookHandle};
pub use hook_registry::HookRegistry;
pub use texture_builder::{
    BuilderParts, CpuTextureDescOwned, ExternalTextureBuilder, SyncInput, TextureSourceInput,
};

#[cfg(test)]
#[path = "tests/ext_tests.rs"]
mod ext_tests;
