//! Vulkan 扩展桥接（ash）：统一帧提交 + 纹理导入。

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use zephyra_ext::prelude::*;
use zephyra_render_ext::capability::BackendCapabilities;
use zephyra_render_ext::error::ImportError;
use zephyra_render_ext::prelude::{
    CpuPlanes, ExternalTextureFormat, ExternalTextureSource, FrameDesc, FrameSource,
    RegisteredTexture,
};

/// 已导入资源（可采样 VkImageView/descriptor 绑定由 renderer 维护）。
#[derive(Clone, Debug)]
pub struct ExtResource {
    pub key: String,
    pub vk_image_raw: Option<u64>,
    pub vk_layout: u32,
    pub vk_memory_raw: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    /// RGBA 单平面待上传数据（renderer 消费）。
    pub pending_cpu: Option<zephyra_ext::CpuTextureDescOwned>,
    /// NV12/I420 多平面（拥有所有权，不 flatten 成 RGBA）。
    pub cpu_planes: Option<Arc<CpuPlanes>>,
    pub pending_updates: VecDeque<zephyra_ext::CpuTextureDescOwned>,
}

#[derive(Default)]
pub struct ExtPool {
    pub textures: HashMap<String, ExtResource>,
    valid_until: HashMap<String, FrameId>,
    current_frame: Cell<FrameId>,
    diagnostics: Arc<FrameDiagnostics>,
}

impl ExtPool {
    pub fn diagnostics(&self) -> &Arc<FrameDiagnostics> {
        &self.diagnostics
    }

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

    pub fn get(&self, key: &str) -> Option<&ExtResource> {
        if self
            .valid_until
            .get(key)
            .is_some_and(|u| *u > self.current_frame.get())
        {
            self.textures.get(key)
        } else {
            None
        }
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.valid_until.remove(key);
        self.textures.remove(key).is_some()
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
}

#[derive(Default)]
pub struct HookTable {
    pub global: HashMap<String, Box<DrawHook>>,
    pub window: HashMap<(u64, String), Box<DrawHook>>,
}

pub struct VulkanExtContext {
    pool: Rc<std::cell::RefCell<ExtPool>>,
    hooks: Rc<std::cell::RefCell<HookTable>>,
    frame: Rc<std::cell::Cell<u64>>,
}

impl VulkanExtContext {
    pub fn new(
        pool: Rc<std::cell::RefCell<ExtPool>>,
        hooks: Rc<std::cell::RefCell<HookTable>>,
        frame: Rc<std::cell::Cell<u64>>,
    ) -> Self {
        Self { pool, hooks, frame }
    }
}

/// Vulkan 导入器保留为句柄转换的小工具（不再实现已删除的 trait）。
pub struct VulkanImporter;

impl VulkanImporter {
    pub fn new() -> Self {
        Self
    }

    /// 把 `ExternalTextureSource::VkImage` 转成资源元数据（不做内存导入）。
    pub fn import_frame(
        &self,
        source: &ExternalTextureSource,
        size: (u32, u32),
        format: ExternalTextureFormat,
    ) -> Result<ExtResource, ImportError> {
        match source {
            ExternalTextureSource::VkImage {
                image,
                layout,
                memory,
            } => Ok(ExtResource {
                key: format!("vulkan-image-{}", image.0),
                vk_image_raw: Some(image.0 as u64),
                vk_layout: *layout,
                vk_memory_raw: memory.map(|m| m.0 as u64),
                width: size.0,
                height: size.1,
                format,
                pending_cpu: None,
                cpu_planes: None,
                pending_updates: VecDeque::new(),
            }),
            _ => Err(ImportError::UnsupportedSource),
        }
    }
}

impl Default for VulkanImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtContextImpl for VulkanExtContext {
    fn submit_frame(
        &mut self,
        key: Cow<'static, str>,
        desc: FrameDesc,
        valid_until: FrameId,
    ) -> Result<RegisteredTexture, ExtError> {
        let k = key.into_owned();
        let resource = match &desc.source {
            FrameSource::Native(source) => match source {
                ExternalTextureSource::VkImage { .. } => VulkanImporter::new()
                    .import_frame(source, desc.size, desc.format)
                    .map_err(ExtError::from)?,
                #[cfg(target_os = "linux")]
                ExternalTextureSource::Dmabuf { .. } => {
                    return Err(ExtError::Import(ImportError::Backend(
                        "Vulkan dmabuf import requires VK_KHR_external_memory_fd (not enabled)"
                            .into(),
                    )));
                }
                _ => {
                    return Err(ExtError::Import(ImportError::Backend(
                        "Vulkan NT handle import requires VK_KHR_external_memory_win32 (not enabled)".into(),
                    )));
                }
            },
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
                    key: k.clone(),
                    vk_image_raw: None,
                    vk_layout: 0,
                    vk_memory_raw: None,
                    width: desc.size.0,
                    height: desc.size.1,
                    format: planes.format,
                    pending_cpu,
                    cpu_planes: Some(planes.clone()),
                    pending_updates: VecDeque::new(),
                }
            }
            _ => return Err(ExtError::UnsupportedSource),
        };
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

pub fn new_ext_handle(
    pool: Rc<std::cell::RefCell<ExtPool>>,
    hooks: Rc<std::cell::RefCell<HookTable>>,
    frame: Rc<std::cell::Cell<u64>>,
) -> ExtContextHandle {
    ExtContextHandle::new(Box::new(VulkanExtContext::new(pool, hooks, frame)))
}
