//! D3D12 扩展桥接：`ExtContextImpl` + 统一帧提交（NT handle / CPU / NV12）。
#![cfg(windows)]

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12Resource};

use zephyra_ext::prelude::*;
use zephyra_render_ext::capability::BackendCapabilities;
use zephyra_render_ext::prelude::{
    CpuPlanes, ExternalTextureFormat, ExternalTextureSource, FrameDesc, FrameSource,
};

/// 已导入的外部资源。
///
/// - 句柄导入：`resource` 已有效；`srv_slot` 由渲染器在 `record` 路径中按需分配并写回。
/// - CPU 导入：`pending_cpu`（RGBA）或 `cpu_planes`（NV12）由 `submit_frame` 写入，
///   渲染器在下一帧建纹理 / 上传 / SRV 后填充 `resource`。NV12 平面**不**flatten。
pub struct ExtResource {
    pub resource: Option<ID3D12Resource>,
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    pub srv_slot: Option<u32>,
    pub pending_cpu: Option<zephyra_ext::CpuTextureDescOwned>,
    /// NV12/I420 多平面（拥有所有权，`Arc` 跨线程零拷贝）。渲染器按 `format` 解释。
    pub cpu_planes: Option<Arc<CpuPlanes>>,
    /// 兼容保留（新 API 不再写入）：旧相机帧更新队列。
    pub pending_updates: VecDeque<zephyra_ext::CpuTextureDescOwned>,
    /// 保持上传堆存活至 GPU 完成拷贝。
    pub(crate) staging: Option<crate::renderer::UploadBuffer>,
}

#[derive(Default)]
pub struct ExtPool {
    pub textures: HashMap<String, ExtResource>,
    retired: Vec<ExtResource>,
    valid_until: HashMap<String, FrameId>,
    current_frame: Cell<FrameId>,
    diagnostics: Arc<FrameDiagnostics>,
}

impl ExtPool {
    pub fn diagnostics(&self) -> &Arc<FrameDiagnostics> {
        &self.diagnostics
    }

    /// 帧边界：剔除过期纹理。
    pub fn begin_frame(&mut self, frame: FrameId) {
        self.current_frame.set(frame);
        let mut expired = 0u32;
        self.valid_until.retain(|key, until| {
            if *until > frame {
                true
            } else {
                self.textures.remove(key);
                expired += 1;
                false
            }
        });
        if expired > 0 {
            self.diagnostics.note_expired(expired);
        }
    }

    fn live(&self, key: &str) -> bool {
        self.valid_until
            .get(key)
            .is_some_and(|until| *until > self.current_frame.get())
    }

    pub fn get(&self, key: &str) -> Option<&ExtResource> {
        if self.live(key) {
            self.textures.get(key)
        } else {
            None
        }
    }
    pub fn srv_slot(&self, key: &str) -> Option<u32> {
        if !self.live(key) {
            return None;
        }
        self.textures.get(key).and_then(|r| r.srv_slot)
    }
    pub fn srv_slots_snapshot(&self) -> HashMap<String, u32> {
        self.textures
            .iter()
            .filter(|(k, _)| self.live(k))
            .filter_map(|(k, v)| v.srv_slot.map(|s| (k.clone(), s)))
            .collect()
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.valid_until.remove(key);
        if let Some(old) = self.textures.remove(key) {
            self.retired.push(old);
            true
        } else {
            false
        }
    }
    pub fn insert(
        &mut self,
        key: String,
        res: ExtResource,
        valid_until: FrameId,
    ) -> TextureRegistration {
        let replaced = self.textures.insert(key.clone(), res).is_some();
        self.valid_until.insert(key, valid_until);
        if replaced {
            TextureRegistration::Replaced
        } else {
            TextureRegistration::Inserted
        }
    }
    pub fn release_retired(&mut self) {
        self.retired.clear();
    }
}

#[derive(Default)]
pub struct HookTable {
    pub global: HashMap<String, Box<DrawHook>>,
    pub window: HashMap<(u64, String), Box<DrawHook>>,
}

/// D3D12 导入器：按来源直接产出强类型 [`ExtResource`]（无 `Box<dyn Any>`）。
pub struct D3D12Importer {
    device: ID3D12Device,
}

impl D3D12Importer {
    pub fn new(device: ID3D12Device) -> Self {
        Self { device }
    }

    pub fn import_frame(
        &mut self,
        source: &ExternalTextureSource,
        size: (u32, u32),
        format: ExternalTextureFormat,
    ) -> Result<ExtResource, ExtError> {
        match source {
            ExternalTextureSource::NtHandle { handle, .. } => {
                let h = HANDLE(handle.0 as *mut core::ffi::c_void);
                let mut resource: Option<ID3D12Resource> = None;
                unsafe { self.device.OpenSharedHandle(h, &mut resource) }
                    .map_err(|e| ExtError::Backend(format!("OpenSharedHandle: {e:?}")))?;
                let resource = resource
                    .ok_or_else(|| ExtError::Backend("OpenSharedHandle returned null".into()))?;
                Ok(ExtResource {
                    resource: Some(resource),
                    width: size.0,
                    height: size.1,
                    format,
                    srv_slot: None,
                    pending_cpu: None,
                    cpu_planes: None,
                    pending_updates: VecDeque::new(),
                    staging: None,
                })
            }
            _ => Err(ExtError::UnsupportedSource),
        }
    }
}

pub struct D3D12ExtContext {
    importer: D3D12Importer,
    pool: Rc<std::cell::RefCell<ExtPool>>,
    hooks: Rc<std::cell::RefCell<HookTable>>,
    frame: Rc<std::cell::Cell<u64>>,
}

impl D3D12ExtContext {
    pub fn new(
        device: ID3D12Device,
        pool: Rc<std::cell::RefCell<ExtPool>>,
        hooks: Rc<std::cell::RefCell<HookTable>>,
        frame: Rc<std::cell::Cell<u64>>,
    ) -> Self {
        Self {
            importer: D3D12Importer::new(device),
            pool,
            hooks,
            frame,
        }
    }
}

impl ExtContextImpl for D3D12ExtContext {
    fn submit_frame(
        &mut self,
        key: Cow<'static, str>,
        desc: FrameDesc,
        valid_until: FrameId,
    ) -> Result<RegisteredTexture, ExtError> {
        let resource = match &desc.source {
            FrameSource::Native(source) => {
                self.importer.import_frame(source, desc.size, desc.format)?
            }
            FrameSource::Cpu(planes) => {
                let is_rgba = matches!(
                    planes.format,
                    ExternalTextureFormat::Rgba8Unorm
                        | ExternalTextureFormat::Rgba8UnormSrgb
                        | ExternalTextureFormat::Bgra8Unorm
                        | ExternalTextureFormat::Bgra8UnormSrgb
                );
                let pending_cpu = if is_rgba {
                    let px = planes.plane(0).ok_or(ExtError::UnsupportedSource)?;
                    Some(zephyra_ext::CpuTextureDescOwned {
                        pixels: px.to_vec(),
                        width: desc.size.0,
                        height: desc.size.1,
                        row_pitch: planes.row_pitch[0],
                        format: planes.format,
                    })
                } else {
                    None
                };
                // NV12 / I420：保留完整平面，不 flatten 成 RGBA。
                if !is_rgba
                    && !matches!(
                        planes.format,
                        ExternalTextureFormat::Nv12 | ExternalTextureFormat::I420
                    )
                {
                    self.pool.borrow().diagnostics().note_unsupported_format();
                    return Err(ExtError::UnsupportedSource);
                }
                ExtResource {
                    resource: None,
                    width: desc.size.0,
                    height: desc.size.1,
                    format: planes.format,
                    srv_slot: None,
                    pending_cpu,
                    cpu_planes: Some(planes.clone()),
                    pending_updates: VecDeque::new(),
                    staging: None,
                }
            }
            _ => return Err(ExtError::UnsupportedSource),
        };
        let k = key.into_owned();
        self.pool
            .borrow_mut()
            .insert(k.clone(), resource, valid_until);
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
        _kind: Cow<'static, str>,
        _hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        Err(ExtError::HooksOnD3D12RequireForWindow)
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
        BackendCapabilities::none()
            .with(
                zephyra_render_ext::capability::Capability::CustomBasic,
                true,
            )
            .with(
                zephyra_render_ext::capability::Capability::CustomOffscreen,
                true,
            )
    }

    fn frame_id(&self) -> FrameId {
        FrameId(self.frame.get())
    }
}

pub fn new_ext_handle(
    device: ID3D12Device,
    pool: Rc<std::cell::RefCell<ExtPool>>,
    hooks: Rc<std::cell::RefCell<HookTable>>,
    frame: Rc<std::cell::Cell<u64>>,
) -> ExtContextHandle {
    ExtContextHandle::new(Box::new(D3D12ExtContext::new(device, pool, hooks, frame)))
}
