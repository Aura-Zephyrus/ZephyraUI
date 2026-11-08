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

#[cfg(test)]
mod ext_context_tests {
    use std::borrow::Cow;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use zephyra_ext::{CpuTextureDescOwned, ExtError, ExternalTextureBuilder};
    use zephyra_render_ext::prelude::{
        DrawHook, ExternalTextureFormat, HookContext, RawHandle, RenderSource,
    };
    use zephyra_render_ext::types::Rect;

    use crate::ext::{new_ext_handle, ExtPool, HookTable, VulkanExtContext};
    use zephyra_ext::ExtContextImpl;

    fn make_ctx() -> (
        zephyra_ext::ExtContextHandle,
        Rc<RefCell<ExtPool>>,
        Rc<RefCell<HookTable>>,
    ) {
        let pool = Rc::new(RefCell::new(ExtPool::default()));
        let hooks = Rc::new(RefCell::new(HookTable::default()));
        let frame = Rc::new(Cell::new(0u64));
        let handle = new_ext_handle(pool.clone(), hooks.clone(), frame);
        (handle, pool, hooks)
    }

    #[test]
    fn cpu_texture_registration_stores_pending_and_unregisters() {
        let (handle, pool, _hooks) = make_ctx();
        let cpu = CpuTextureDescOwned {
            width: 2,
            height: 2,
            row_pitch: 8,
            format: ExternalTextureFormat::Rgba8UnormSrgb,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        };
        let tex = handle
            .register_texture(ExternalTextureBuilder::new("vk.cpu", (2, 2)).source_cpu(cpu))
            .expect("register cpu");
        assert_eq!(tex.key(), "vk.cpu");
        {
            let p = pool.borrow();
            let res = p.get("vk.cpu").expect("stored");
            assert!(res.pending_cpu.is_some());
            assert_eq!((res.width, res.height), (2, 2));
        }
        tex.unregister().expect("unregister");
        assert!(pool.borrow().get("vk.cpu").is_none());
    }

    #[test]
    fn cpu_texture_replacement_returns_replaced_registration() {
        let pool = Rc::new(RefCell::new(ExtPool::default()));
        let hooks = Rc::new(RefCell::new(HookTable::default()));
        let frame = Rc::new(Cell::new(0u64));
        let mut ctx = VulkanExtContext::new(pool.clone(), hooks, frame);
        let cpu = CpuTextureDescOwned {
            width: 2,
            height: 2,
            row_pitch: 8,
            format: ExternalTextureFormat::Rgba8UnormSrgb,
            pixels: vec![0; 16],
        };
        let r1 = ctx
            .register_texture(ExternalTextureBuilder::new("vk.cpu", (2, 2)).source_cpu(cpu))
            .expect("insert");
        assert!(matches!(r1, zephyra_ext::TextureRegistration::Inserted));
        let cpu2 = CpuTextureDescOwned {
            width: 1,
            height: 1,
            row_pitch: 4,
            format: ExternalTextureFormat::Rgba8UnormSrgb,
            pixels: vec![1, 2, 3, 4],
        };
        let r2 = ctx
            .register_texture(ExternalTextureBuilder::new("vk.cpu", (1, 1)).source_cpu(cpu2))
            .expect("replace");
        assert!(matches!(r2, zephyra_ext::TextureRegistration::Replaced));
        ctx.unregister_texture("vk.cpu").expect("unregister");
        assert!(pool.borrow().get("vk.cpu").is_none());
    }

    #[test]
    fn vk_image_metadata_registration_stores_handles() {
        let pool = Rc::new(RefCell::new(ExtPool::default()));
        let hooks = Rc::new(RefCell::new(HookTable::default()));
        let frame = Rc::new(Cell::new(7u64));
        let mut ctx = VulkanExtContext::new(pool.clone(), hooks, frame);
        let b = ExternalTextureBuilder::new("vk.img", (64, 32))
            .format(ExternalTextureFormat::Bgra8Unorm)
            .source_auto(zephyra_ext::texture_builder::TextureSourceInput::VkImage {
                image: RawHandle(0x1234),
                layout: 5,
                memory: Some(RawHandle(0x5678)),
            });
        let r = ctx.register_texture(b).expect("vk image meta");
        assert!(matches!(r, zephyra_ext::TextureRegistration::Inserted));
        let p = pool.borrow();
        let res = p.get("vk.img").expect("stored");
        assert_eq!(res.vk_image_raw, Some(0x1234));
        assert_eq!(res.vk_layout, 5);
        assert_eq!(res.vk_memory_raw, Some(0x5678));
        assert_eq!((res.width, res.height), (64, 32));
    }

    #[test]
    fn nt_handle_import_reports_external_memory_requirement() {
        let pool = Rc::new(RefCell::new(ExtPool::default()));
        let hooks = Rc::new(RefCell::new(HookTable::default()));
        let frame = Rc::new(Cell::new(0u64));
        let mut ctx = VulkanExtContext::new(pool, hooks, frame);
        let b = ExternalTextureBuilder::new("vk.nt", (4, 4)).source_auto(
            zephyra_ext::texture_builder::TextureSourceInput::NtHandle(RawHandle(42)),
        );
        let err = ctx.register_texture(b).unwrap_err();
        match err {
            ExtError::Import(zephyra_render_ext::error::ImportError::Backend(msg)) => {
                assert!(msg.contains("VK_KHR_external_memory_win32"));
            }
            other => panic!("expected backend import error, got {other:?}"),
        }
    }

    #[test]
    fn hook_registration_and_window_scoping() {
        let (handle, _pool, hooks) = make_ctx();
        let hook: Box<DrawHook> =
            Box::new(|_ctx: &mut dyn HookContext, _r: Rect, _s: &RenderSource<'_>| {});
        let h1 = handle
            .register_hook(Cow::Borrowed("custom.vk"), hook)
            .expect("global hook");
        assert!(hooks.borrow().global.contains_key("custom.vk"));
        let hook2: Box<DrawHook> = Box::new(|_ctx, _r, _s| {});
        let h2 = handle
            .register_hook_for_window(7, Cow::Borrowed("custom.vk.win"), hook2)
            .expect("window hook");
        assert!(hooks
            .borrow()
            .window
            .contains_key(&(7, "custom.vk.win".to_string())));
        drop(h1);
        assert!(!hooks.borrow().global.contains_key("custom.vk"));
        drop(h2);
        assert!(!hooks
            .borrow()
            .window
            .contains_key(&(7, "custom.vk.win".to_string())));
    }
}
