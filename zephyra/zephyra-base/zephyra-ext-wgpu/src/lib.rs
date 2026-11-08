//! wgpu 互操作（可选）：让**使用 wgpu 开发的扩展/应用**能接入 Zephyra 的扩展契约。
//!
//! # 设计边界（S5 / P0-4）
//!
//! `zephyra-render-ext` / `zephyra-ext` **不依赖 wgpu**。wgpu 的转换与 glue 放在本
//! crate（或应用自己的 feature-gated 层），避免把 wgpu 依赖树强加给所有后端。
//!
//! # 提供能力
//!
//! - [`adapter_identity_from_wgpu`]：从 `wgpu::Adapter`/`AdapterInfo` 提取
//!   [`zephyra_render_ext::AdapterIdentity`]（尽力映射 vendor/device/backend；UUID/LUID
//!   在公共 API 不可用时为 `None`）。
//! - [`backend_from_wgpu`]：映射 `wgpu::Backend` → [`zephyra_render_ext::SourceBackend`]。
//!
//! # 用法
//!
//! ```ignore
//! use zephyra_ext_wgpu::adapter_identity_from_wgpu;
//! let id = adapter_identity_from_wgpu(&adapter);
//! // 将 id 传给需要 AdapterIdentity 的 interop/匹配逻辑（应用层决策）。
//! ```

#![deny(unsafe_code)]

use zephyra_render_ext::types::{AdapterIdentity, SourceBackend};

#[cfg(feature = "native")]
#[allow(unsafe_code)]
pub mod native;

/// `wgpu::Backend` → 契约层 `SourceBackend`。
pub fn backend_from_wgpu(backend: wgpu::Backend) -> SourceBackend {
    match backend {
        wgpu::Backend::Vulkan => SourceBackend::Vulkan,
        wgpu::Backend::Dx12 => SourceBackend::D3D12,
        #[allow(unreachable_patterns)]
        _ => SourceBackend::Wgpu,
    }
}

/// 从 `wgpu::Adapter` 提取 [`AdapterIdentity`]（尽力而为）。
///
/// wgpu 24.x 的公共 `AdapterInfo` 不总是暴露 LUID/UUID；这些字段在不可得时为 `None`。
/// 需要更强匹配时，应用可在自己的 feature-gated/hal 层补充 `device_uuid`/`luid`。
pub fn adapter_identity_from_wgpu(adapter: &wgpu::Adapter) -> AdapterIdentity {
    adapter_identity_from_info(adapter.get_info())
}

/// 从 `wgpu::AdapterInfo` 提取 [`AdapterIdentity`]（尽力而为）。
pub fn adapter_identity_from_info(info: wgpu::AdapterInfo) -> AdapterIdentity {
    AdapterIdentity {
        vendor_id: info.vendor,
        device_id: info.device,
        luid: None,
        device_uuid: None,
        driver_uuid: None,
        name: if info.name.is_empty() {
            None
        } else {
            Some(info.name.into())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_mapping_compiles() {
        assert_eq!(
            backend_from_wgpu(wgpu::Backend::Vulkan),
            SourceBackend::Vulkan
        );
        assert_eq!(backend_from_wgpu(wgpu::Backend::Dx12), SourceBackend::D3D12);
        // wgpu 24: other backends map to Wgpu/unknown category in contract terms.
        assert_eq!(backend_from_wgpu(wgpu::Backend::Gl), SourceBackend::Wgpu);
    }

    #[test]
    fn adapter_identity_from_info_maps_fields() {
        let info = wgpu::AdapterInfo {
            name: "test".into(),
            vendor: 0x1234,
            device: 0x5678,
            device_type: wgpu::DeviceType::DiscreteGpu,
            driver: "drv".into(),
            driver_info: "info".into(),
            backend: wgpu::Backend::Vulkan,
        };
        let id = adapter_identity_from_info(info);
        assert_eq!(id.vendor_id, 0x1234);
        assert_eq!(id.device_id, 0x5678);
        assert_eq!(id.name.as_deref(), Some("test"));
        assert!(id.luid.is_none());
    }
}
