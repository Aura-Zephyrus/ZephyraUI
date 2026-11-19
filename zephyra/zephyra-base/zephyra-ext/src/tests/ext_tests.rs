use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use zephyra_render_ext::prelude::{
    AdapterIdentity, BackendCapabilities, DrawHook, ExternalTextureFormat, ExternalTextureSource,
    FrameId, HookContextExt, RawHandle, Rect, RenderSource,
};

use crate::ext_context::{ExtContextHandle, ExtContextImpl, ExtError, TextureRegistration};
use crate::texture_builder::{
    CpuTextureDescOwned, ExternalTextureBuilder, SyncInput, TextureSourceInput,
};

#[derive(Default)]
struct MockState {
    textures: HashMap<String, (u32, u32)>,
    hooks: HashSet<String>,
    window_hooks: HashMap<(u64, String), ()>,
    unregister_count: Rc<Cell<u32>>,
    hook_unregister_count: Rc<Cell<u32>>,
    updates: Vec<(String, Vec<u8>, u32)>,
}

struct MockExt {
    state: std::rc::Rc<std::cell::RefCell<MockState>>,
    caps: BackendCapabilities,
    frame: FrameId,
    fail_hooks: bool,
}

impl MockExt {
    fn new() -> (Self, std::rc::Rc<std::cell::RefCell<MockState>>) {
        let state = std::rc::Rc::new(std::cell::RefCell::new(MockState::default()));
        (
            Self {
                state: state.clone(),
                caps: BackendCapabilities::all(),
                frame: FrameId(1),
                fail_hooks: false,
            },
            state,
        )
    }
}

impl ExtContextImpl for MockExt {
    fn register_texture(
        &mut self,
        builder: ExternalTextureBuilder,
    ) -> Result<TextureRegistration, ExtError> {
        let (key, parts) = builder.into_parts();
        let mut s = self.state.borrow_mut();
        let replaced = s.textures.insert(key.into_owned(), parts.size).is_some();
        Ok(if replaced {
            TextureRegistration::Replaced
        } else {
            TextureRegistration::Inserted
        })
    }
    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError> {
        let mut s = self.state.borrow_mut();
        let c = s.unregister_count.clone();
        c.set(c.get() + 1);
        s.textures.remove(key);
        Ok(())
    }
    fn update_texture(&mut self, key: &str, pixels: &[u8], row_pitch: u32) -> Result<(), ExtError> {
        if !self.state.borrow().textures.contains_key(key) {
            return Err(ExtError::Backend(format!("texture not registered: {key}")));
        }
        self.state
            .borrow_mut()
            .updates
            .push((key.to_string(), pixels.to_vec(), row_pitch));
        Ok(())
    }
    fn register_hook(
        &mut self,
        kind: Cow<'static, str>,
        _hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        if self.fail_hooks {
            return Err(ExtError::HooksOnD3D12RequireForWindow);
        }
        self.state.borrow_mut().hooks.insert(kind.into_owned());
        Ok(())
    }
    fn register_hook_for_window(
        &mut self,
        window: u64,
        kind: Cow<'static, str>,
        _hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        self.state
            .borrow_mut()
            .window_hooks
            .insert((window, kind.into_owned()), ());
        Ok(())
    }
    fn unregister_hook(&mut self, kind: &str, window: Option<u64>) -> bool {
        let mut s = self.state.borrow_mut();
        let c = s.hook_unregister_count.clone();
        c.set(c.get() + 1);
        match window {
            None => s.hooks.remove(kind),
            Some(w) => s.window_hooks.remove(&(w, kind.to_string())).is_some(),
        }
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.caps.clone()
    }
    fn frame_id(&self) -> FrameId {
        self.frame
    }
}

fn handle(mock: MockExt) -> ExtContextHandle {
    ExtContextHandle::new(Box::new(mock))
}

/// P0.2：`update_texture` 原地更新 —— 已注册 key 成功入队、不改变注册表，
/// 未注册 key 报错；快照的字节与行距原样送达后端。
#[test]
fn update_texture_enqueues_without_reregistering() {
    let (mock, state) = MockExt::new();
    let h = handle(mock);
    let b = ExternalTextureBuilder::new("cam", (640, 480)).source_cpu(CpuTextureDescOwned {
        pixels: vec![0; 640 * 480 * 4],
        width: 640,
        height: 480,
        row_pitch: 640 * 4,
        format: ExternalTextureFormat::Rgba8UnormSrgb,
    });
    let _tex = h.register_texture(b).unwrap();

    assert_eq!(
        h.update_texture("cam", &[1, 2, 3, 4], 2560)
            .err()
            .map(|e| e.to_string()),
        None
    );
    let s = state.borrow();
    assert_eq!(s.textures.len(), 1, "更新不应新建/替换纹理注册表项");
    assert_eq!(s.updates.len(), 1);
    assert_eq!(s.updates[0].0, "cam");
    assert_eq!(s.updates[0].1, vec![1, 2, 3, 4]);
    assert_eq!(s.updates[0].2, 2560);
    drop(s);

    assert!(
        h.update_texture("missing", &[0; 4], 4).is_err(),
        "未注册的 key 必须报错而不是静默成功"
    );
}

#[test]
fn register_texture_and_raii_unregister() {
    let (mock, state) = MockExt::new();
    let h = handle(mock);
    {
        let _tex = h
            .register_texture(ExternalTextureBuilder::new("video", (1920, 1080)))
            .expect("register");
        assert!(state.borrow().textures.contains_key("video"));
    }
    assert!(!state.borrow().textures.contains_key("video"));
    assert_eq!(state.borrow().unregister_count.get(), 1);
}

#[test]
fn same_key_replaces_texture() {
    let (mock, state) = MockExt::new();
    let h = handle(mock);
    let tex = h
        .register_texture(ExternalTextureBuilder::new("f", (1, 1)))
        .expect("register");
    h.register_texture(ExternalTextureBuilder::new("f", (2, 3)))
        .expect("replace");
    assert_eq!(state.borrow().textures.get("f"), Some(&(2, 3)));
    tex.update(ExternalTextureBuilder::new("ignored", (4, 5)))
        .expect("update");
    assert_eq!(state.borrow().textures.get("f"), Some(&(4, 5)));
}

#[test]
fn explicit_unregister_and_context_dropped() {
    let (mock, state) = MockExt::new();
    let h = handle(mock);
    let tex = h
        .register_texture(ExternalTextureBuilder::new("t", (1, 1)))
        .expect("register");
    tex.try_unregister().expect("unregister");
    assert!(!state.borrow().textures.contains_key("t"));
    drop(h);
    // handle dropped; creating a new handle from old weak isn't possible, but API surfaces ContextDropped on stale handles
}

#[test]
fn hook_registry_generic_and_window_scoped() {
    let (mock, state) = MockExt::new();
    let h = handle(mock);
    let _g = h.hooks().generic("debug", |_ctx, _r, _s| {}).expect("hook");
    assert!(state.borrow().hooks.contains("debug"));
    let _w = h
        .hooks()
        .for_window(7)
        .generic("per-win", |_ctx, _r, _s| {})
        .expect("window hook");
    assert!(state
        .borrow()
        .window_hooks
        .contains_key(&(7, "per-win".to_string())));
}

#[test]
fn capabilities_and_frame_id() {
    let (mock, _) = MockExt::new();
    let h = handle(mock);
    let caps = h.ctx().capabilities();
    assert!(caps.has(zephyra_render_ext::prelude::Capability::CustomBasic));
    assert_eq!(h.ctx().frame_id(), FrameId(1));
}

#[test]
fn builder_cpu_owned_parts() {
    let b = ExternalTextureBuilder::new("cpu", (2, 2))
        .format(zephyra_render_ext::prelude::ExternalTextureFormat::Rgba8Unorm)
        .source_cpu(CpuTextureDescOwned {
            pixels: vec![0; 16],
            width: 2,
            height: 2,
            row_pitch: 8,
            format: zephyra_render_ext::prelude::ExternalTextureFormat::Rgba8Unorm,
        })
        .sync_auto(SyncInput::None);
    let (k, p) = b.into_parts();
    assert_eq!(k, "cpu");
    assert_eq!(p.size, (2, 2));
    assert!(p.cpu_owned().is_some());
}

#[test]
fn draw_hook_and_hook_context_ext() {
    struct Ctx(u64, (f32, f32));
    impl zephyra_render_ext::prelude::HookContext for Ctx {
        fn frame_index(&self) -> u64 {
            self.0
        }
        fn size(&self) -> (f32, f32) {
            self.1
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
    let mut hook: Box<DrawHook> = Box::new(|ctx, rect, src| {
        assert_eq!(ctx.frame_index(), 9);
        assert_eq!(rect.w, 10.0);
        assert_eq!(src.resource_key(), Some("k"));
        assert!(ctx.downcast_ref::<Ctx>().is_some());
    });
    let mut ctx = Ctx(9, (10.0, 10.0));
    hook(
        &mut ctx,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        &RenderSource::new(Some("k"), None),
    );
}

#[test]
fn d3d12_style_hook_registration_can_fail() {
    let (mut mock, _) = MockExt::new();
    mock.fail_hooks = true;
    let h = handle(mock);
    let r = h.hooks().generic("x", |_c, _r, _s| {});
    assert!(matches!(r, Err(ExtError::HooksOnD3D12RequireForWindow)));
}

#[test]
fn context_dropped_errors() {
    let (mock, _) = MockExt::new();
    let h = handle(mock);
    let tex = h
        .register_texture(ExternalTextureBuilder::new("t", (1, 1)))
        .expect("register");
    drop(h);
    let err = tex.try_unregister().unwrap_err();
    assert!(matches!(err, ExtError::ContextDropped));
}

#[test]
fn hook_handle_raii_unregisters() {
    let (mock, state) = MockExt::new();
    let h = handle(mock);
    {
        let _hook = h.register_hook("h1", Box::new(|_, _, _| {})).expect("hook");
        assert!(state.borrow().hooks.contains("h1"));
    }
    assert_eq!(state.borrow().hook_unregister_count.get(), 1);
}

#[test]
fn window_scoped_hook_allowed_when_global_d3d12_strategy() {
    let (mut mock, state) = MockExt::new();
    mock.fail_hooks = true; // global hooks rejected (D3D12 strategy)
    let h = handle(mock);
    {
        let _wh = h
            .hooks()
            .for_window(42)
            .generic("win", |_c, _r, _s| {})
            .expect("window hook");
        assert!(state
            .borrow()
            .window_hooks
            .contains_key(&(42, "win".to_string())));
    }
    assert!(!state
        .borrow()
        .window_hooks
        .contains_key(&(42, "win".to_string())));
}

#[test]
fn adapter_identity_and_builder_handle_desc_mapping() {
    let id = AdapterIdentity {
        vendor_id: 0x10de,
        device_id: 0x1234,
        luid: Some(99),
        device_uuid: Some([1; 16]),
        driver_uuid: None,
        name: Some("gpu".into()),
    };
    assert_eq!(id.vendor_id, 0x10de);
    assert_eq!(id.luid, Some(99));
    let id2 = AdapterIdentity {
        vendor_id: 0x10de,
        device_id: 0x1234,
        luid: Some(99),
        ..Default::default()
    };
    assert!(id.matches_physical_device(&id2));
    let wgpu_like = AdapterIdentity {
        vendor_id: 0x8086,
        device_id: 0x9bc4,
        device_uuid: Some([2; 16]),
        ..Default::default()
    };
    let wgpu_like2 = AdapterIdentity {
        vendor_id: 0,
        device_id: 0,
        device_uuid: Some([2; 16]),
        ..Default::default()
    };
    assert!(wgpu_like.matches_physical_device(&wgpu_like2));

    let b = ExternalTextureBuilder::new("nt", (64, 64))
        .format(ExternalTextureFormat::Bgra8Unorm)
        .source_auto(TextureSourceInput::NtHandle(RawHandle(0x1234)));
    let (k, p) = b.into_parts();
    let desc = p.try_into_handle_desc(k).expect("nt handle desc");
    match desc.source {
        ExternalTextureSource::NtHandle {
            handle,
            source_backend,
        } => {
            assert_eq!(handle, RawHandle(0x1234));
            assert_eq!(source_backend, None);
        }
        _ => panic!("expected NtHandle"),
    }
    assert_eq!(desc.size, (64, 64));

    let b = ExternalTextureBuilder::new("vk", (8, 8)).source_auto(TextureSourceInput::VkImage {
        image: RawHandle(7),
        layout: 5,
        memory: Some(RawHandle(8)),
    });
    let (k, p) = b.into_parts();
    let desc = p.try_into_handle_desc(k).expect("vk desc");
    match desc.source {
        ExternalTextureSource::VkImage {
            image,
            layout,
            memory,
        } => {
            assert_eq!(image, RawHandle(7));
            assert_eq!(layout, 5);
            assert_eq!(memory, Some(RawHandle(8)));
        }
        _ => panic!("expected VkImage"),
    }

    let b = ExternalTextureBuilder::new("cpu", (1, 1)).source_cpu(CpuTextureDescOwned {
        pixels: vec![0, 0, 0, 255],
        width: 1,
        height: 1,
        row_pitch: 4,
        format: ExternalTextureFormat::Rgba8Unorm,
    });
    let (k, p) = b.into_parts();
    assert!(p.try_into_handle_desc(k).is_none());
}

#[test]
fn hook_panic_isolation_pattern_is_catchable() {
    // §133/§15.5 contract (pure): backend dispatch isolates hook panics with catch_unwind.
    // Runtime integration on real render paths remains GPU/backend-specific.
    let mut hook: Box<zephyra_render_ext::prelude::DrawHook> = Box::new(|_, _, _| panic!("boom"));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Minimal fake context for the contract test.
        struct C;
        impl zephyra_render_ext::prelude::HookContext for C {
            fn frame_index(&self) -> u64 {
                0
            }
            fn size(&self) -> (f32, f32) {
                (1.0, 1.0)
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }
        let mut ctx = C;
        hook(
            &mut ctx,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            &RenderSource::none(),
        );
    }));
    assert!(r.is_err());
}

#[test]
fn ext_error_display_variants() {
    assert!(ExtError::ContextDropped.to_string().contains("dropped"));
    assert!(ExtError::UnsupportedSource
        .to_string()
        .contains("unsupported"));
    assert!(ExtError::HookConflict("k".into())
        .to_string()
        .contains("conflict"));
    assert!(ExtError::HooksOnD3D12RequireForWindow
        .to_string()
        .contains("D3D12"));
    assert!(ExtError::Backend("b".into())
        .to_string()
        .contains("backend"));
    assert!(
        ExtError::Import(zephyra_render_ext::error::ImportError::DeviceLost)
            .to_string()
            .contains("import")
    );
}

#[test]
fn builder_try_into_handle_desc_maps_sources() {
    let (_, parts) = ExternalTextureBuilder::new("k", (2, 3))
        .source_auto(TextureSourceInput::NtHandle(RawHandle(7)))
        .into_parts();
    let d = parts.try_into_handle_desc("k".into()).expect("nt");
    assert_eq!(d.size, (2, 3));
    assert!(matches!(d.source, ExternalTextureSource::NtHandle { .. }));

    let (_, parts) = ExternalTextureBuilder::new("k", (4, 5))
        .source_auto(TextureSourceInput::VkImage {
            image: RawHandle(1),
            layout: 2,
            memory: Some(RawHandle(3)),
        })
        .into_parts();
    let d = parts.try_into_handle_desc("k".into()).expect("vk");
    assert!(matches!(d.source, ExternalTextureSource::VkImage { .. }));
}
