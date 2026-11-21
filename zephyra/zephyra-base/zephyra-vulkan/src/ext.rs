//! Vulkan 扩展桥接（ash）：`ExtContextImpl` + 纹理导入最小实现。

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use zephyra_ext::prelude::*;
use zephyra_ext::ExternalTextureBuilder;
use zephyra_render_ext::capability::BackendCapabilities;
use zephyra_render_ext::error::ImportError;
use zephyra_render_ext::external_texture::{
    CpuTextureDesc, CpuVideoTextureDesc, ExternalTextureDesc, ExternalTextureImporter,
    ImportedTexture,
};
use zephyra_render_ext::prelude::{ExternalTextureFormat, ExternalTextureSource, FrameId};

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

/// Vulkan 外部纹理导入器。
///
/// 与 D3D11/D3D12 一样实现 [`ExternalTextureImporter`]，让 `zephyra-media`
/// 能把解码帧导入 Vulkan 后端。`VkImage` 句柄直接登记；CPU / 视频帧走
/// `pending_cpu` 延迟上传路径（renderer 集成 image view/descriptor 后消费）。
pub struct VulkanImporter {
    pool: Rc<std::cell::RefCell<ExtPool>>,
}

impl VulkanImporter {
    pub fn new(pool: Rc<std::cell::RefCell<ExtPool>>) -> Self {
        Self { pool }
    }

    fn insert_cpu(
        &mut self,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        row_pitch: u32,
        format: ExternalTextureFormat,
    ) -> ImportedTexture {
        let key = format!("vulkan-cpu-{}", FrameId(0).0);
        let res = ExtResource {
            key: key.clone(),
            vk_image_raw: None,
            vk_layout: 0,
            vk_memory_raw: None,
            width,
            height,
            pending_cpu: Some(zephyra_ext::CpuTextureDescOwned {
                pixels,
                width,
                height,
                row_pitch,
                format,
            }),
            pending_updates: VecDeque::new(),
        };
        self.pool.borrow_mut().insert(key.clone(), res);
        // handle 只作为标识；renderer 按 key 查 pool 取真实资源。
        ImportedTexture::new(key, FrameId(0))
    }
}

impl ExternalTextureImporter for VulkanImporter {
    fn import(&mut self, desc: ExternalTextureDesc) -> Result<ImportedTexture, ImportError> {
        match desc.source {
            ExternalTextureSource::VkImage {
                image,
                layout,
                memory,
            } => {
                let key = format!("vulkan-image-{}", image.0);
                let res = ExtResource {
                    key: key.clone(),
                    vk_image_raw: Some(image.0 as u64),
                    vk_layout: layout,
                    vk_memory_raw: memory.map(|m| m.0 as u64),
                    width: desc.size.0,
                    height: desc.size.1,
                    pending_cpu: None,
                    pending_updates: VecDeque::new(),
                };
                self.pool.borrow_mut().insert(key.clone(), res);
                Ok(ImportedTexture::new(key, FrameId(0)))
            }
            // VK_KHR_external_memory 未启用；不假装支持。
            _ => Err(ImportError::UnsupportedSource),
        }
    }

    fn import_cpu(&mut self, desc: CpuTextureDesc<'_>) -> Result<ImportedTexture, ImportError> {
        let tight = desc.width as usize * 4;
        let mut pixels = Vec::with_capacity(desc.row_pitch as usize * desc.height as usize);
        for row in 0..desc.height as usize {
            let start = row * desc.row_pitch as usize;
            let end = start + tight;
            let src = desc
                .pixels
                .get(start..end)
                .ok_or(ImportError::SizeMismatch {
                    expected: (desc.width, desc.height),
                    actual: (0, 0),
                })?;
            pixels.extend_from_slice(src);
        }
        Ok(self.insert_cpu(pixels, desc.width, desc.height, desc.width * 4, desc.format))
    }

    fn import_video_cpu(
        &mut self,
        desc: CpuVideoTextureDesc<'_>,
    ) -> Result<ImportedTexture, ImportError> {
        let (pixels, width, height, row_pitch) =
            desc.flatten_nv12().ok_or(ImportError::UnsupportedSource)?;
        Ok(self.insert_cpu(
            pixels,
            width,
            height,
            row_pitch,
            ExternalTextureFormat::Nv12,
        ))
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
