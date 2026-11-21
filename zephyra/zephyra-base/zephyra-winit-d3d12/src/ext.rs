//! D3D12 扩展桥接：`ExtContextImpl` + 外部纹理导入（NT handle / CPU，最小实现）。
#![cfg(windows)]

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12Resource};

use zephyra_ext::prelude::*;
use zephyra_ext::{BuilderParts, ExternalTextureBuilder};
use zephyra_render_ext::capability::BackendCapabilities;
use zephyra_render_ext::external_texture::CpuVideoTextureDesc;
use zephyra_render_ext::types::ExternalTextureFormat;

/// 已导入的外部资源。
///
/// - 句柄导入：`resource` 已有效；`srv_slot` 由渲染器在 `record` 路径中按需分配并写回。
/// - CPU 上传：`pending_cpu` 由 `ExtContext` 写入，渲染器在 `record` 路径中创建纹理/上传/SRV 后填充 `resource`。
pub struct ExtResource {
    pub resource: Option<ID3D12Resource>,
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    pub srv_slot: Option<u32>,
    pub pending_cpu: Option<zephyra_ext::CpuTextureDescOwned>,
    /// `update_texture` 排入的帧队列：渲染器在下一帧就地拷进**同一张**纹理，
    /// 不重新分配。容量有限（3 帧），相机帧生产更快时丢中间帧 —— 对视频流正确。
    pub pending_updates: VecDeque<zephyra_ext::CpuTextureDescOwned>,
    /// 保持上传堆存活至 GPU 完成拷贝。替换/卸载时移入 `ExtPool::retired`，由渲染器在 GPU 空闲点释放。
    pub(crate) staging: Option<crate::renderer::UploadBuffer>,
}

#[derive(Default)]
pub struct ExtPool {
    pub textures: HashMap<String, ExtResource>,
    /// 已卸载/替换的资源（含 staging/资源/SRV 账本信息由调用方处理）。仅在 GPU 已确认空闲时释放。
    retired: Vec<ExtResource>,
}

impl ExtPool {
    pub fn get(&self, key: &str) -> Option<&ExtResource> {
        self.textures.get(key)
    }
    pub fn srv_slot(&self, key: &str) -> Option<u32> {
        self.textures.get(key).and_then(|r| r.srv_slot)
    }
    /// 快照：key → SRV 槽位（用于渲染期间避免与 `&mut WindowTargets` 冲突的借用）。
    pub fn srv_slots_snapshot(&self) -> std::collections::HashMap<String, u32> {
        self.textures
            .iter()
            .filter_map(|(k, v)| v.srv_slot.map(|s| (k.clone(), s)))
            .collect()
    }
    pub fn remove(&mut self, key: &str) -> bool {
        if let Some(old) = self.textures.remove(key) {
            self.retired.push(old);
            true
        } else {
            false
        }
    }
    pub fn insert(&mut self, key: String, res: ExtResource) -> TextureRegistration {
        if let Some(old) = self.textures.insert(key, res) {
            self.retired.push(old);
            TextureRegistration::Replaced
        } else {
            TextureRegistration::Inserted
        }
    }

    /// 在 GPU 已空闲（如 `begin_frame` 的 `wait_gpu()` 之后）的安全点释放已退役资源。
    pub fn release_retired(&mut self) {
        self.retired.clear();
    }
}

#[derive(Default)]
pub struct HookTable {
    pub global: HashMap<String, Box<DrawHook>>,
    pub window: HashMap<(u64, String), Box<DrawHook>>,
}

pub struct D3D12Importer {
    device: ID3D12Device,
}

impl D3D12Importer {
    pub fn new(device: ID3D12Device) -> Self {
        Self { device }
    }

    pub fn import_parts(
        &mut self,
        key: Cow<'static, str>,
        parts: &BuilderParts,
    ) -> Result<(String, ExtResource), ExtError> {
        if let Some(cpu) = parts.cpu_owned() {
            let cpu = cpu.clone();
            return Ok((
                key.into_owned(),
                ExtResource {
                    resource: None,
                    width: cpu.width,
                    height: cpu.height,
                    format: cpu.format,
                    srv_slot: None,
                    pending_cpu: Some(cpu),
                    pending_updates: VecDeque::new(),
                    staging: None,
                },
            ));
        }
        let Some(desc) = parts.try_into_handle_desc(key.clone()) else {
            return Err(ExtError::UnsupportedSource);
        };
        let imported = self.import(desc).map_err(ExtError::from)?;
        let res = imported
            .handle
            .downcast::<ExtResource>()
            .map_err(|_| ExtError::Backend("unexpected imported handle type".into()))?;
        Ok((key.into_owned(), *res))
    }
}

impl ExternalTextureImporter for D3D12Importer {
    fn import(&mut self, desc: ExternalTextureDesc) -> Result<ImportedTexture, ImportError> {
        match desc.source {
            ExternalTextureSource::NtHandle { handle, .. } => {
                let h = HANDLE(handle.0 as *mut core::ffi::c_void);
                let mut resource: Option<ID3D12Resource> = None;
                unsafe { self.device.OpenSharedHandle(h, &mut resource) }
                    .map_err(|e| ImportError::Backend(format!("OpenSharedHandle: {e:?}")))?;
                let resource = resource
                    .ok_or_else(|| ImportError::Backend("OpenSharedHandle returned null".into()))?;
                Ok(ImportedTexture::new(
                    ExtResource {
                        resource: Some(resource),
                        width: desc.size.0,
                        height: desc.size.1,
                        format: desc.format,
                        srv_slot: None,
                        pending_cpu: None,
                        pending_updates: VecDeque::new(),
                        staging: None,
                    },
                    FrameId(0),
                ))
            }
            #[cfg(target_os = "linux")]
            ExternalTextureSource::Dmabuf { .. } => Err(ImportError::UnsupportedSource),
            ExternalTextureSource::VkImage { .. } | _ => Err(ImportError::UnsupportedSource),
        }
    }

    fn import_cpu(&mut self, _desc: CpuTextureDesc<'_>) -> Result<ImportedTexture, ImportError> {
        Err(ImportError::Backend(
            "D3D12 CPU ext texture import requires upload command path".into(),
        ))
    }

    /// 视频帧（NV12）走与 `import_parts` 相同的延迟上传路径：把两个平面合并成
    /// 一块 owned buffer，由 renderer 在下一帧建纹理/SRV。
    fn import_video_cpu(
        &mut self,
        desc: CpuVideoTextureDesc<'_>,
    ) -> Result<ImportedTexture, ImportError> {
        let (pixels, width, height, row_pitch) =
            desc.flatten_nv12().ok_or(ImportError::UnsupportedSource)?;
        Ok(ImportedTexture::new(
            ExtResource {
                resource: None,
                width,
                height,
                format: ExternalTextureFormat::Nv12,
                srv_slot: None,
                pending_cpu: Some(zephyra_ext::CpuTextureDescOwned {
                    pixels,
                    width,
                    height,
                    row_pitch,
                    format: ExternalTextureFormat::Nv12,
                }),
                pending_updates: VecDeque::new(),
                staging: None,
            },
            FrameId(0),
        ))
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
    fn register_texture(
        &mut self,
        builder: ExternalTextureBuilder,
    ) -> Result<TextureRegistration, ExtError> {
        let (key, parts) = builder.into_parts();
        let (k, res) = self.importer.import_parts(key, &parts)?;
        Ok(self.pool.borrow_mut().insert(k, res))
    }

    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError> {
        self.pool.borrow_mut().remove(key);
        Ok(())
    }

    fn update_texture(&mut self, key: &str, pixels: &[u8], row_pitch: u32) -> Result<(), ExtError> {
        let mut pool = self.pool.borrow_mut();
        let res = pool
            .textures
            .get_mut(key)
            .ok_or_else(|| ExtError::Backend(format!("texture not registered: {key}")))?;
        // 只排一帧的像素；渲染器按注册时的 width/height/format 就地上传。
        let needed = (res.width as usize) * (res.height as usize) * 4;
        if pixels.len() < needed {
            return Err(ExtError::Backend(format!(
                "update pixels too small: {} < {needed}",
                pixels.len()
            )));
        }
        let cpu = zephyra_ext::CpuTextureDescOwned {
            pixels: pixels.to_vec(),
            width: res.width,
            height: res.height,
            row_pitch,
            format: res.format,
        };
        // 相机帧生产可能快于渲染帧：只留最近 3 帧，旧的直接丢（视频流语义）。
        res.pending_updates.push_back(cpu);
        while res.pending_updates.len() > 3 {
            res.pending_updates.pop_front();
        }
        Ok(())
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
