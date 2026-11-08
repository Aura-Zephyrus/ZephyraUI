//! §12/133 extension contract stubs (GPU/ICD required). These are intentionally ignored
//! in normal runs; they document the expected cross-backend/runtime matrix.

#[test]
#[ignore = "requires Windows + D3D12 GPU"]
fn register_external_texture_nt_handle() {}

#[test]
#[ignore = "requires Windows + D3D12 GPU + multi-window host"]
fn multi_window_hook_registration() {}

#[test]
#[ignore = "requires Windows + D3D12 GPU"]
fn multi_window_hook_isolation() {}

#[test]
#[ignore = "requires Windows + D3D12 GPU"]
fn cross_backend_texture_import() {}

#[test]
#[ignore = "requires Windows + D3D12 GPU; device removal scenario"]
fn device_lost_recovery() {}

#[test]
#[ignore = "requires Windows + D3D12 GPU; runtime hook panic isolation in render path"]
fn hook_panic_isolation_runtime() {}

#[test]
#[ignore = "requires Windows + D3D12 GPU; throughput baseline"]
fn cpu_fallback_throughput_baseline() {}
