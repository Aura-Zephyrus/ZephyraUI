//! `Backend` / 共享设备契约的编译期门禁。
//!
//! 这两条不需要窗口或 GPU —— 它们钉住的是**类型形状**，而类型形状一旦破了，
//! `WindowSet` 在 Linux 上根本编译不过（那时才发现，会比这里晚得多）。

use std::sync::{Arc, Mutex};

use zephyra_window::Backend;

use super::*;

fn assert_backend<B: Backend>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn vulkan_backend_satisfies_window_set_contract() {
    // `Backend::Window<V>` 的上界是 `WindowHost + 'static`，而 `WindowHost`
    // 的实现又要求 `V: View + 'static` —— 任何一条失效都会在这里红。
    assert_backend::<VulkanBackend>();
}

#[test]
fn shared_device_handle_is_send_and_sync() {
    // `Device` 只 `Send`（ash 的句柄里是裸指针），是 `Mutex` 让它变成 `Sync`。
    // 去掉 `unsafe impl Send for Device` 会让 `WindowSet` 的跨线程用法失效，
    // 而单线程窗口路完全看不出差别 —— 所以单独立一条。
    assert_send_sync::<Arc<Mutex<Device>>>();
}
