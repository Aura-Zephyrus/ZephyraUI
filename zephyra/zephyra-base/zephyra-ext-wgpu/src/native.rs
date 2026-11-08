//! 原生句柄提取（`feature = "native"`，包含 unsafe）：wgpu 纹理 → 平台/后端句柄。
//!
//! # 重要限制
//!
//! - **wgpu 纹理默认不可共享**：DX12/Vulkan 的跨 API/跨进程共享通常要求创建时带共享标志。
//!   wgpu 公共 API 不保证分配共享资源；能否导出 NT handle / external memory 取决于纹理创建方式与后端。
//! - **wgpu-hal API 随版本变化**：本模块按 wgpu/wgpu-hal 24 系列的常见形态编写；若编译失败，
//!   请对照实际 `wgpu-hal` 版本调整 `as_hal`/原生类型访问。
//! - 提取到的句柄/图像用于 [`zephyra_render_ext::ExternalTextureSource`]，再由 Zephyra 后端
//!   [`ExternalTextureImporter`](zephyra_render_ext::external_texture::ExternalTextureImporter) 导入。

use zephyra_render_ext::external_texture::ExternalTextureSource;
use zephyra_render_ext::types::{RawHandle, SourceBackend};

/// 尝试从 wgpu 纹理提取可用于 Zephyra 扩展导入的原生来源（尽力而为）。
///
/// 默认返回 `None`：自动导出需要资源具备共享标志且生命周期/同步契约明确。
/// 应用可使用：
/// - Windows + `native-dx12`: [`dx12::export_nt_handle_source`]（unsafe，需兼容设备+共享资源）
/// - Vulkan/dmabuf: 应用层通过 wgpu-hal + 平台 API 导出，再构造 [`ExternalTextureSource`]
pub fn try_extract_source(_texture: &wgpu::Texture) -> Option<ExternalTextureSource> {
    None
}

/// Vulkan：尝试读取底层 `VkImage` 原始句柄（不导出外部内存）。
///
/// 仅提供图像句柄不足以在独立后端安全导入（通常还需 memory/fd + layout + 同步）；
/// 返回值可用于诊断或配合应用层完整导出流程。
///
/// 注意：wgpu-hal 24 的 Vulkan 纹理句柄类型与 ash 版本相关；此处保留接线点，
/// 实际提取请在应用层按所用 `wgpu-hal`/`ash` 版本实现（避免在 interop crate 锁死 ash 依赖）。
pub fn try_extract_vulkan_image_raw(_texture: &wgpu::Texture) -> Option<RawHandle> {
    None
}

#[cfg(all(windows, feature = "native-dx12"))]
pub mod dx12 {
    use super::*;
    use windows::Win32::Graphics::Direct3D12::ID3D12Device;
    use windows::Win32::Security::SECURITY_ATTRIBUTES;

    /// NT 共享句柄导出策略（§15.1）。
    ///
    /// 默认安全：不传显式 `SECURITY_ATTRIBUTES`（避免 null DACL），句柄不可继承语义由系统默认处理。
    /// 需要跨进程共享时由调用方显式 opt-in，并自行提供符合安全模型的 DACL/属性。
    #[derive(Debug, Clone, Copy, Default)]
    pub struct NtHandleExportPolicy {
        /// 允许子进程继承句柄。默认 `false`。
        pub inherit_handle: bool,
        /// 显式 opt-in：使用调用方提供的安全描述符指针（可为 null 仅用于受控环境；生产慎用）。
        /// `None` = 不传 `SECURITY_ATTRIBUTES`（推荐默认）。
        pub security_descriptor: Option<*mut core::ffi::c_void>,
    }

    // Safety: caller controls descriptor lifetime/validity when set.
    unsafe impl Send for NtHandleExportPolicy {}
    unsafe impl Sync for NtHandleExportPolicy {}

    /// 从 DX12 后端 wgpu 纹理提取 `ID3D12Resource` 并导出 NT 共享句柄（需要纹理/资源可共享）。
    ///
    /// # Safety
    /// - `device` 必须与创建该 wgpu 纹理的 D3D12 设备兼容/同源。
    /// - 底层资源必须允许共享（wgpu 默认创建路径通常不保证）；否则 `CreateSharedHandle` 失败。
    /// - 返回的 `HANDLE` 由调用方负责生命周期/复制策略；传入 Zephyra 时通常复制或转移所有权语义需与后端约定一致。
    pub unsafe fn export_nt_handle_source(
        texture: &wgpu::Texture,
        device: &ID3D12Device,
    ) -> Option<ExternalTextureSource> {
        export_nt_handle_source_with_policy(texture, device, NtHandleExportPolicy::default())
    }

    /// 同 [`export_nt_handle_source`]，但允许显式安全策略（§15.1 opt-in）。
    ///
    /// # Safety
    /// 同 [`export_nt_handle_source`]；当 `policy.security_descriptor` 为 `Some` 时，调用方必须保证
    /// 描述符指针在调用期间有效且语义正确。避免在生产默认路径使用 null DACL。
    pub unsafe fn export_nt_handle_source_with_policy(
        texture: &wgpu::Texture,
        device: &ID3D12Device,
        policy: NtHandleExportPolicy,
    ) -> Option<ExternalTextureSource> {
        // wgpu 24: `Texture::as_hal<A, F, R>` 通常返回 `Option<R>`（非该后端时 None）。
        // 回调签名：`FnOnce(Option<&A::Texture>) -> R`。若你使用的 wgpu/wgpu-hal 版本签名不同，
        // 请按实际 API 调整（这是 feature-gated 接线点）。
        texture.as_hal::<wgpu_hal::api::Dx12, _, _>(|opt| {
            let t = opt?;
            let resource = t.raw_resource();
            let attrs = policy.security_descriptor.map(|sd| SECURITY_ATTRIBUTES {
                nLength: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: policy.inherit_handle.into(),
            });
            // windows 0.58: CreateSharedHandle 返回 Result<HANDLE>。
            // GENERIC_ALL = 0x10000000。
            let pattrs = attrs
                .as_ref()
                .map(|a| a as *const windows::Win32::Security::SECURITY_ATTRIBUTES);
            let out = device
                .CreateSharedHandle(resource, pattrs, 0x1000_0000, windows::core::PCWSTR::null())
                .ok()?;
            if out.is_invalid() {
                return None;
            }
            Some(ExternalTextureSource::NtHandle {
                handle: RawHandle(out.0 as usize),
                source_backend: Some(SourceBackend::Wgpu),
            })
        })
    }

    /// 旧入口：保留兼容，转发到无设备版本不可实现时返回 `None`。
    pub fn try_extract_nt_handle_source(_texture: &wgpu::Texture) -> Option<ExternalTextureSource> {
        None
    }
}
