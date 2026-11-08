//! §12/133 extension contract stubs (Vulkan ICD required).

#[test]
#[ignore = "requires Linux/Windows Vulkan ICD"]
fn register_external_texture_vk_image() {}

#[test]
#[ignore = "requires Vulkan ICD"]
fn render_hook_vk_command_buffer() {}

#[test]
#[ignore = "requires Vulkan ICD + external memory"]
fn vk_image_zero_copy_import() {}

#[test]
#[ignore = "requires Vulkan ICD"]
fn cross_backend_texture_import() {}

#[test]
#[ignore = "requires Vulkan ICD; VK_ERROR_DEVICE_LOST scenario"]
fn device_lost_recovery() {}
