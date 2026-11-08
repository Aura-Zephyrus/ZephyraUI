//! 回退/设备能力缓存契约（§8，Phase 1 仅定义类型，不实现探测）。

use std::collections::HashMap;

/// 后端标识（占位；与实际后端枚举在接线时对齐）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Backend {
    D3D12,
    D3D11,
    Vulkan,
    Cpu,
}

/// 特性状态（参考 Chromium GpuFeatureStatus）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FeatureStatus {
    Enabled,
    Blocklisted,
    Disabled,
    Software,
}

/// 后端评分（契约字段；Phase 2+ 填充实测）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackendScore {
    pub timeline_fence: bool,
    pub shared_texture: bool,
    pub upload_bandwidth_mbps: f32,
    pub submit_latency_ms: f32,
    pub tdr_count: u32,
    pub driver_bug_workarounds: u32,
    pub score: f32,
}

/// 设备能力缓存（Phase 1 契约）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceCapabilityCache {
    pub timestamp: u64,
    pub vendor_id: u32,
    pub device_id: u32,
    pub revision: u32,
    pub driver_version: String,
    pub driver_date: Option<String>,
    pub backend_scores: HashMap<Backend, BackendScore>,
    pub selected_backend: Option<Backend>,
    pub fallback_chain: Vec<Backend>,
    pub applied_blocklist_entries: Vec<u32>,
    pub applied_workarounds: Vec<u32>,
}

/// 回退策略（应用可控；`Custom` 在 Phase 1 仅类型占位）。
#[derive(Default)]
pub enum FallbackPolicy {
    #[default]
    Automatic,
    Disable(Backend),
    Force(Backend),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_contract_types() {
        let mut c = DeviceCapabilityCache::default();
        c.backend_scores
            .insert(Backend::Cpu, BackendScore::default());
        c.fallback_chain = vec![Backend::D3D12, Backend::D3D11, Backend::Cpu];
        assert_eq!(c.fallback_chain.len(), 3);
        assert!(matches!(
            FallbackPolicy::default(),
            FallbackPolicy::Automatic
        ));
        assert_eq!(FeatureStatus::Enabled, FeatureStatus::Enabled);
    }

    #[test]
    fn fallback_policy_variants() {
        assert!(matches!(
            FallbackPolicy::Disable(Backend::D3D12),
            FallbackPolicy::Disable(Backend::D3D12)
        ));
        assert!(matches!(
            FallbackPolicy::Force(Backend::Vulkan),
            FallbackPolicy::Force(Backend::Vulkan)
        ));
    }

    #[test]
    fn backend_score_defaults() {
        let s = BackendScore::default();
        assert!(!s.timeline_fence);
        assert_eq!(s.tdr_count, 0);
        assert_eq!(s.score, 0.0);
    }
}
