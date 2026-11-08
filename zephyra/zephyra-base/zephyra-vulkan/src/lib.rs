//! ZephyraUI 的裸 Vulkan(ash) 渲染后端：窗口 + 渲染器 + 帧循环（Linux/Windows）。
//!
//! # 模块划分
//!
//! - `surface`：Instance / Device / Queue / Surface / Swapchain / 读回
//! - `renderer`：顶点流 → GPU 一帧
//! - `host`：窗口、事件循环、帧调度
//! - `ext`：后端扩展桥接（纹理/Hook，按 `zephyra-ext` / `zephyra-render-ext` 契约）
//!
//! # 为什么三个渲染后端互不依赖
//!
//! D3D11 / D3D12 / Vulkan 是三套不同的词汇，硬并会让 `#[cfg]` 分支淹没真正的
//! 逻辑。后端由**特性开关**选，同一个二进制只会有一个；跨后端共享的只有
//! `zephyra-fx`（`DisplayList → 顶点流` 等后端无关部分）。

mod ext;
mod host;
mod renderer;
mod surface;

#[cfg(test)]
#[path = "tests/backend_contract.rs"]
mod backend_contract;

pub use host::{HostWindow, VulkanBackend};
pub use renderer::{HookCtx, RenderCtx, RenderHook, Renderer, TextureTable};
pub use surface::{Device, Surface, SurfaceError};
