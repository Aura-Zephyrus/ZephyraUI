//! D3D11 扩展桥接：`ExtContextImpl` + 外部纹理导入（NT handle / CPU / NV12）。
#![cfg(windows)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};

use zephyra_ext::prelude::*;
use zephyra_render_ext::capability::BackendCapabilities;
use zephyra_render_ext::prelude::{
    CpuPlanes, ExternalTextureFormat, ExternalTextureSource, FrameDesc, FrameLifetime, FrameSource,
};

use crate::texture::{create_nv12_texture, create_rgba_texture, create_srv};

/// 已导入的外部纹理资源（SRV + 底层纹理）。强类型，不再是 `Box<dyn Any>`。
pub struct ExtResource {
    pub srv: ID3D11ShaderResourceView,
    pub texture: ID3D11Texture2D,
}

/// 扩展纹理池。
///
/// 2026-11-21：它会同时被 Hook（`srv(key)`）与**纹理解析**（`TextureTable::contains/srv`）
/// 查询 —— 后者是让 `Command::Image` 真正画出扩展纹理的关键。生命周期由
/// `valid_until` 真正在帧边界生效：`begin_frame` 剔除过期项并计入诊断。
#[derive(Default)]
pub struct ExtPool {
    pub textures: HashMap<String, ExtResource>,
    lifetime: FrameLifetime,
    diagnostics: Arc<FrameDiagnostics>,
}

impl ExtPool {
    pub fn diagnostics(&self) -> &Arc<FrameDiagnostics> {
        &self.diagnostics
    }

    pub fn get(&self, key: &str) -> Option<&ExtResource> {
        if self.contains(key) {
            self.textures.get(key)
        } else {
            None
        }
    }

    /// 帧边界调用：剔除所有已过期纹理。
    pub fn begin_frame(&mut self, frame: FrameId) {
        for key in self.lifetime.begin_frame(frame, &self.diagnostics) {
            self.textures.remove(&key);
        }
    }

    /// key 当前是否可解析（存在且未过期）。
    pub fn contains(&self, key: &str) -> bool {
        self.textures.contains_key(key) && self.lifetime.is_live(key)
    }

    pub fn srv(&self, key: &str) -> Option<&ID3D11ShaderResourceView> {
        if !self.contains(key) {
            return None;
        }
        self.textures.get(key).map(|r| &r.srv)
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.lifetime.remove(key);
        self.textures.remove(key).is_some()
    }

    pub fn insert(
        &mut self,
        key: String,
        srv: ID3D11ShaderResourceView,
        tex: ID3D11Texture2D,
        valid_until: FrameId,
    ) -> TextureRegistration {
        let res = ExtResource { srv, texture: tex };
        let was_present = self.textures.insert(key.clone(), res).is_some();
        let fresh = self.lifetime.register(key, valid_until);
        if was_present || !fresh {
            TextureRegistration::Replaced
        } else {
            TextureRegistration::Inserted
        }
    }
}

/// D3D11 资源导入器：按 [`FrameSource`] 分发，直接产出 [`ExtResource`]。
pub struct D3D11Importer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

impl D3D11Importer {
    pub fn new(device: ID3D11Device, context: ID3D11DeviceContext) -> Self {
        Self { device, context }
    }

    /// 导入原生句柄帧，返回强类型资源。
    pub fn import_frame(
        &mut self,
        source: &ExternalTextureSource,
    ) -> Result<ExtResource, ExtError> {
        match source {
            ExternalTextureSource::NtHandle { handle, .. } => {
                let h = HANDLE(handle.0 as *mut core::ffi::c_void);
                let mut tex: Option<ID3D11Texture2D> = None;
                // SAFETY: 调用方保证句柄有效且指向可共享的 D3D11/DXGI 资源；打开后由 COM 管理生命周期。
                unsafe { self.device.OpenSharedResource(h, &mut tex) }
                    .map_err(|e| ExtError::Backend(format!("OpenSharedResource: {e:?}")))?;
                let tex = tex
                    .ok_or_else(|| ExtError::Backend("OpenSharedResource returned null".into()))?;
                let srv = create_srv(&self.device, &tex)
                    .map_err(|e| ExtError::Backend(format!("create_srv: {e:?}")))?;
                Ok(ExtResource { srv, texture: tex })
            }
            _ => Err(ExtError::UnsupportedSource),
        }
    }

    /// 用显式尺寸导入 CPU 平面（NV12 走双平面直传，绝不 flatten 成 RGBA）。
    pub fn import_cpu_sized(
        &mut self,
        planes: &CpuPlanes,
        size: (u32, u32),
    ) -> Result<ExtResource, ExtError> {
        if planes.format == ExternalTextureFormat::Nv12 {
            let y = planes.plane(0).ok_or(ExtError::UnsupportedSource)?;
            let uv = planes.plane(1).ok_or(ExtError::UnsupportedSource)?;
            let tex = create_nv12_texture(
                &self.device,
                &self.context,
                y,
                planes.row_pitch[0],
                uv,
                planes.row_pitch[1],
                size.0,
                size.1,
            )
            .map_err(|e| ExtError::Backend(format!("create_nv12_texture: {e:?}")))?;
            // NV12 是视频格式：部分驱动拒绝显式 `D3D11_SHADER_RESOURCE_VIEW_DESC`
            // （E_INVALIDARG），必须从资源描述隐式派生视图格式。
            let mut srv = None;
            unsafe {
                self.device
                    .CreateShaderResourceView(&tex, None, Some(&mut srv as *mut _))
            }
            .map_err(|e| ExtError::Backend(format!("create_srv(NV12 implicit): {e:?}")))?;
            let srv =
                srv.ok_or_else(|| ExtError::Backend("create_srv(NV12) returned null".into()))?;
            Ok(ExtResource { srv, texture: tex })
        } else {
            let px = planes.plane(0).ok_or(ExtError::UnsupportedSource)?;
            let tex = create_rgba_texture(&self.device, &self.context, px, size.0, size.1)
                .map_err(|e| ExtError::Backend(format!("create_rgba_texture: {e:?}")))?;
            let srv = create_srv(&self.device, &tex)
                .map_err(|e| ExtError::Backend(format!("create_srv: {e:?}")))?;
            Ok(ExtResource { srv, texture: tex })
        }
    }
}

#[derive(Default)]
pub(crate) struct HookTable {
    pub global: HashMap<String, Box<DrawHook>>,
    pub window: HashMap<(u64, String), Box<DrawHook>>,
}

impl HookTable {
    #[allow(dead_code)]
    pub fn has(&self, kind: &str, window: Option<u64>) -> bool {
        match window {
            None => self.global.contains_key(kind),
            Some(w) => self.window.contains_key(&(w, kind.to_string())),
        }
    }
}

/// `ExtContextImpl`：唯一的资源动作是 submit_frame。
pub(crate) struct D3D11ExtContext {
    importer: D3D11Importer,
    pool: Arc<std::cell::RefCell<ExtPool>>,
    hooks: Arc<std::cell::RefCell<HookTable>>,
    frame: Arc<std::cell::Cell<u64>>,
}

impl D3D11ExtContext {
    pub(crate) fn new(
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        pool: Arc<std::cell::RefCell<ExtPool>>,
        hooks: Arc<std::cell::RefCell<HookTable>>,
        frame: Arc<std::cell::Cell<u64>>,
    ) -> Self {
        Self {
            importer: D3D11Importer::new(device, context),
            pool,
            hooks,
            frame,
        }
    }
}

impl ExtContextImpl for D3D11ExtContext {
    fn submit_frame(
        &mut self,
        key: Cow<'static, str>,
        desc: FrameDesc,
        valid_until: FrameId,
    ) -> Result<RegisteredTexture, ExtError> {
        let mut resource = match &desc.source {
            FrameSource::Cpu(_) => self
                .importer
                .import_cpu_sized(desc.cpu_planes().unwrap(), desc.size)?,
            FrameSource::Native(source) => self.importer.import_frame(source)?,
            _ => return Err(ExtError::UnsupportedSource),
        };
        if !matches!(
            desc.format,
            ExternalTextureFormat::Nv12
                | ExternalTextureFormat::Rgba8Unorm
                | ExternalTextureFormat::Rgba8UnormSrgb
                | ExternalTextureFormat::Bgra8Unorm
                | ExternalTextureFormat::Bgra8UnormSrgb
        ) {
            let _ = &mut resource;
            return Err(ExtError::UnsupportedSource);
        }
        let k = key.into_owned();
        self.pool
            .borrow_mut()
            .insert(k.clone(), resource.srv, resource.texture, valid_until);
        Ok(RegisteredTexture {
            key: Cow::Owned(k),
            valid_until,
        })
    }

    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError> {
        self.pool.borrow_mut().remove(key);
        Ok(())
    }

    fn has_texture(&self, key: &str) -> bool {
        self.pool.borrow().textures.contains_key(key)
    }

    fn register_hook(
        &mut self,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        self.hooks
            .borrow_mut()
            .global
            .insert(kind.into_owned(), hook);
        Ok(())
    }

    fn register_hook_for_window(
        &mut self,
        window: u64,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        self.hooks
            .borrow_mut()
            .window
            .insert((window, kind.into_owned()), hook);
        Ok(())
    }

    fn unregister_hook(&mut self, kind: &str, window: Option<u64>) -> bool {
        let mut h = self.hooks.borrow_mut();
        match window {
            None => h.global.remove(kind).is_some(),
            Some(w) => h.window.remove(&(w, kind.to_string())).is_some(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none().with(
            zephyra_render_ext::capability::Capability::CustomBasic,
            true,
        )
    }

    fn frame_id(&self) -> FrameId {
        FrameId(self.frame.get())
    }
}

pub(crate) fn new_ext_handle(
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    pool: Arc<std::cell::RefCell<ExtPool>>,
    hooks: Arc<std::cell::RefCell<HookTable>>,
    frame: Arc<std::cell::Cell<u64>>,
) -> ExtContextHandle {
    ExtContextHandle::new(Box::new(D3D11ExtContext::new(
        device, context, pool, hooks, frame,
    )))
}
