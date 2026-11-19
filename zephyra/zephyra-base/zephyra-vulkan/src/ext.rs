//! Vulkan 扩展桥接（ash）：`ExtContextImpl` + 纹理导入最小实现。

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use zephyra_ext::prelude::*;
use zephyra_ext::ExternalTextureBuilder;
use zephyra_render_ext::capability::BackendCapabilities;

/// 已导入资源（最小元数据；可采样 VkImageView/descriptor 绑定由后续增强）。
#[derive(Clone, Debug)]
pub struct ExtResource {
    pub key: String,
    /// 来自 `ExternalTextureSource::VkImage` 的原始句柄值（不透明，供 Hook/导入层使用）。
    pub vk_image_raw: Option<u64>,
    pub vk_layout: u32,
    pub vk_memory_raw: Option<u64>,
    pub width: u32,
    pub height: u32,
    /// CPU 上传待处理数据（renderer 集成 image view/descriptor 后消费）。
    pub pending_cpu: Option<zephyra_ext::CpuTextureDescOwned>,
    /// `update_texture` 排入的帧队列（最多 3 帧）；渲染器就地更新同一张图像。
    pub pending_updates: VecDeque<zephyra_ext::CpuTextureDescOwned>,
}

#[derive(Default)]
pub struct ExtPool {
    pub textures: HashMap<String, ExtResource>,
}

impl ExtPool {
    pub fn get(&self, key: &str) -> Option<&ExtResource> {
        self.textures.get(key)
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.textures.remove(key).is_some()
    }
    pub fn insert(&mut self, key: String, res: ExtResource) -> TextureRegistration {
        if self.textures.insert(key, res).is_some() {
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

impl ExtContextImpl for VulkanExtContext {
    fn register_texture(
        &mut self,
        builder: ExternalTextureBuilder,
    ) -> Result<TextureRegistration, ExtError> {
        let (key, parts) = builder.into_parts();
        let key_s = key.into_owned();
        let (width, height) = (parts.size.0, parts.size.1);
        if let Some(cpu) = parts.cpu_owned() {
            // Vulkan CPU 上传需要 staging/command buffer + image view/descriptor；
            // 保留 pending 数据供 renderer 集成路径消费（132）。
            return Ok(self.pool.borrow_mut().insert(
                key_s.clone(),
                ExtResource {
                    key: key_s,
                    vk_image_raw: None,
                    vk_layout: 0,
                    vk_memory_raw: None,
                    width,
                    height,
                    pending_cpu: Some(cpu.clone()),
                    pending_updates: VecDeque::new(),
                },
            ));
        }
        if let Some(desc) = parts.try_into_handle_desc(std::borrow::Cow::Owned(key_s.clone())) {
            use zephyra_render_ext::external_texture::ExternalTextureSource;
            match &desc.source {
                ExternalTextureSource::VkImage {
                    image,
                    layout,
                    memory,
                } => {
                    return Ok(self.pool.borrow_mut().insert(
                        key_s.clone(),
                        ExtResource {
                            key: key_s,
                            vk_image_raw: Some(image.0 as u64),
                            vk_layout: *layout,
                            vk_memory_raw: memory.map(|m| m.0 as u64),
                            width,
                            height,
                            pending_cpu: None,
                            pending_updates: VecDeque::new(),
                        },
                    ));
                }
                #[cfg(target_os = "linux")]
                ExternalTextureSource::Dmabuf { .. } => {
                    return Err(ExtError::Import(
                        zephyra_render_ext::error::ImportError::Backend(
                            "Vulkan dmabuf/external-memory import requires VK_KHR_external_memory_fd + platform import path (not enabled in this build); use CPU fallback or provide imported VkImage+memory".into(),
                        ),
                    ));
                }
                ExternalTextureSource::NtHandle { .. } => {
                    return Err(ExtError::Import(
                        zephyra_render_ext::error::ImportError::Backend(
                            "Vulkan NT handle import requires VK_KHR_external_memory_win32 (not enabled in this build)".into(),
                        ),
                    ));
                }
                _ => {}
            }
        }
        Err(ExtError::UnsupportedSource)
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
        let needed = (res.width as usize) * (res.height as usize) * 4;
        if pixels.len() < needed {
            return Err(ExtError::Backend(format!(
                "update pixels too small: {} < {needed}",
                pixels.len()
            )));
        }
        res.pending_updates
            .push_back(zephyra_ext::CpuTextureDescOwned {
                pixels: pixels.to_vec(),
                width: res.width,
                height: res.height,
                row_pitch,
                format: zephyra_ext::prelude::ExternalTextureFormat::Rgba8UnormSrgb,
            });
        while res.pending_updates.len() > 3 {
            res.pending_updates.pop_front();
        }
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

pub fn new_ext_handle(
    pool: Rc<std::cell::RefCell<ExtPool>>,
    hooks: Rc<std::cell::RefCell<HookTable>>,
    frame: Rc<std::cell::Cell<u64>>,
) -> ExtContextHandle {
    ExtContextHandle::new(Box::new(VulkanExtContext::new(pool, hooks, frame)))
}
