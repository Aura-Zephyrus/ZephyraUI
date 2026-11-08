//! 平台内部实现路由骨架（Phase 1：类型/模块占位；具体后端接线 Phase 2-4）。

#[cfg(windows)]
pub mod d3d11 {
    /// D3D11 Hook 上下文标记（Phase 2 提供真实实现）。
    pub trait D3D11HookContext: zephyra_render_ext::prelude::HookContext {}
}
#[cfg(windows)]
pub mod d3d12 {
    /// D3D12 Hook 上下文标记（Phase 3）。
    pub trait D3D12HookContext: zephyra_render_ext::prelude::HookContext {}
}
#[cfg(target_os = "linux")]
pub mod vulkan {
    /// Vulkan Hook 上下文标记（Phase 4）。
    pub trait VulkanHookContext: zephyra_render_ext::prelude::HookContext {}
}
