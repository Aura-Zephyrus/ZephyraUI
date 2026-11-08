//! D3D11 扩展桥接：`ExtContextImpl` + 外部纹理导入（NT handle / CPU）。
#![cfg(windows)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};

use zephyra_ext::prelude::*;
use zephyra_ext::{BuilderParts, CpuTextureDescOwned, ExternalTextureBuilder, TextureSourceInput};
use zephyra_render_ext::capability::BackendCapabilities;

use crate::texture::{create_rgba_texture, create_srv};

/// 已导入的外部纹理资源（SRV + 底层纹理）。
pub struct ExtResource {
    pub srv: ID3D11ShaderResourceView,
    pub texture: ID3D11Texture2D,
}

#[derive(Default)]
pub(crate) struct ExtPool {
    pub textures: HashMap<String, ExtResource>,
}

impl ExtPool {
    pub fn get(&self, key: &str) -> Option<&ExtResource> {
        self.textures.get(key)
    }
    pub fn srv(&self, key: &str) -> Option<&ID3D11ShaderResourceView> {
        self.textures.get(key).map(|r| &r.srv)
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.textures.remove(key).is_some()
    }
    pub fn insert(
        &mut self,
        key: String,
        srv: ID3D11ShaderResourceView,
        tex: ID3D11Texture2D,
    ) -> TextureRegistration {
        let res = ExtResource { srv, texture: tex };
        if self.textures.insert(key, res).is_some() {
            TextureRegistration::Replaced
        } else {
            TextureRegistration::Inserted
        }
    }
}

/// D3D11 `ExternalTextureImporter`（最小实现：CPU + NT 共享句柄）。
pub struct D3D11Importer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

impl D3D11Importer {
    pub fn new(device: ID3D11Device, context: ID3D11DeviceContext) -> Self {
        Self { device, context }
    }

    pub fn import_parts(
        &mut self,
        key: Cow<'static, str>,
        parts: &BuilderParts,
    ) -> Result<(String, ID3D11ShaderResourceView, ID3D11Texture2D), ExtError> {
        if let Some(cpu) = parts.cpu_owned() {
            let (srv, tex) = self.import_cpu_owned(cpu)?;
            return Ok((key.into_owned(), srv, tex));
        }
        let Some(desc) = parts.try_into_handle_desc(key.clone()) else {
            if matches!(&parts.source, TextureSourceInput::Auto) {
                return Err(ExtError::UnsupportedSource);
            }
            return Err(ExtError::UnsupportedSource);
        };
        let imported = self.import(desc).map_err(ExtError::from)?;
        let res = imported
            .handle
            .downcast::<ExtResource>()
            .map_err(|_| ExtError::Backend("unexpected imported handle type".into()))?;
        Ok((key.into_owned(), res.srv, res.texture))
    }

    fn import_cpu_owned(
        &mut self,
        cpu: &CpuTextureDescOwned,
    ) -> Result<(ID3D11ShaderResourceView, ID3D11Texture2D), ExtError> {
        let tight = (cpu.width as usize) * (cpu.height as usize) * 4;
        let _row_padded =
            (cpu.row_pitch as usize).max((cpu.width as usize) * 4) * (cpu.height as usize);
        if cpu.pixels.len() < tight {
            return Err(ExtError::Backend(format!(
                "cpu pixels too small: {} < {}",
                cpu.pixels.len(),
                tight
            )));
        }
        let tex = create_rgba_texture(
            &self.device,
            &self.context,
            &cpu.pixels,
            cpu.width,
            cpu.height,
        )
        .map_err(|e| ExtError::Backend(format!("create_rgba_texture: {e:?}")))?;
        let srv = create_srv(&self.device, &tex)
            .map_err(|e| ExtError::Backend(format!("create_srv: {e:?}")))?;
        Ok((srv, tex))
    }
}

impl ExternalTextureImporter for D3D11Importer {
    fn import(&mut self, desc: ExternalTextureDesc) -> Result<ImportedTexture, ImportError> {
        match desc.source {
            ExternalTextureSource::NtHandle { handle, .. } => {
                let h = HANDLE(handle.0 as *mut core::ffi::c_void);
                let mut tex: Option<ID3D11Texture2D> = None;
                // SAFETY: 调用方保证句柄有效且指向可共享的 D3D11/DXGI 资源；打开后由 COM 管理生命周期。
                unsafe { self.device.OpenSharedResource(h, &mut tex) }
                    .map_err(|e| ImportError::Backend(format!("OpenSharedResource: {e:?}")))?;
                let tex = tex.ok_or_else(|| {
                    ImportError::Backend("OpenSharedResource returned null".into())
                })?;
                let srv = create_srv(&self.device, &tex)
                    .map_err(|e| ImportError::Backend(format!("create_srv: {e:?}")))?;
                Ok(ImportedTexture::new(
                    ExtResource { srv, texture: tex },
                    FrameId(0),
                ))
            }
            #[cfg(target_os = "linux")]
            ExternalTextureSource::Dmabuf { .. } => Err(ImportError::UnsupportedSource),
            ExternalTextureSource::VkImage { .. } | _ => Err(ImportError::UnsupportedSource),
        }
    }

    fn import_cpu(&mut self, desc: CpuTextureDesc<'_>) -> Result<ImportedTexture, ImportError> {
        let tex = create_rgba_texture(
            &self.device,
            &self.context,
            desc.pixels,
            desc.width,
            desc.height,
        )
        .map_err(|e| ImportError::Backend(format!("create_rgba_texture: {e:?}")))?;
        let srv = create_srv(&self.device, &tex)
            .map_err(|e| ImportError::Backend(format!("create_srv: {e:?}")))?;
        Ok(ImportedTexture::new(
            ExtResource { srv, texture: tex },
            FrameId(0),
        ))
    }
}

/// `ExtContextImpl`：委托给回调/共享状态（由 `HostWindow` 装配）。
pub(crate) struct D3D11ExtContext {
    importer: D3D11Importer,
    pool: Arc<std::cell::RefCell<ExtPool>>,
    hooks: Arc<std::cell::RefCell<HookTable>>,
    frame: Arc<std::cell::Cell<u64>>,
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
    fn register_texture(
        &mut self,
        builder: ExternalTextureBuilder,
    ) -> Result<TextureRegistration, ExtError> {
        let (key, parts) = builder.into_parts();
        let (k, srv, tex) = self.importer.import_parts(key, &parts)?;
        Ok(self.pool.borrow_mut().insert(k, srv, tex))
    }

    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError> {
        self.pool.borrow_mut().remove(key);
        Ok(())
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
