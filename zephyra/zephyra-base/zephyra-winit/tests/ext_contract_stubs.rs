//! §12/133 extension contract stubs (GPU required).

#[test]
#[ignore = "requires Windows + D3D11 GPU"]
fn register_external_texture_nt_handle() {}

#[test]
#[ignore = "requires Windows + D3D11 GPU"]
fn render_hook_d3d11_context() {}

#[test]
#[ignore = "requires Windows + D3D11 GPU"]
fn external_texture_cpu_fallback() {}

#[test]
#[ignore = "requires Windows + D3D11 GPU"]
fn cross_backend_texture_import() {}

#[test]
#[ignore = "requires Windows + D3D11 GPU; device removal scenario"]
fn device_lost_recovery() {}
