//! 扩展上下文（§3.2，P1-1 单线程；2026-11-21 统一为单一 submit_frame 动作）。

use std::borrow::Cow;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use zephyra_render_ext::prelude::{
    BackendCapabilities, DrawHook, FrameDesc, FrameId, ImportError, RegisteredTexture,
};

use crate::handles::{ExternalTextureHandle, HookHandle};
use crate::texture_builder::ExternalTextureBuilder;

#[derive(Debug)]
#[non_exhaustive]
pub enum ExtError {
    ContextDropped,
    UnsupportedSource,
    HookConflict(String),
    HooksOnD3D12RequireForWindow,
    Import(ImportError),
    /// `valid_until` 必须严格大于当前帧号，否则这一帧注册出来就已经过期。
    ExpiredFrame {
        valid_until: FrameId,
        current: FrameId,
    },
    Backend(String),
}

impl core::fmt::Display for ExtError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ExtError::ContextDropped => write!(f, "extension context dropped"),
            ExtError::UnsupportedSource => write!(f, "unsupported texture source"),
            ExtError::HookConflict(k) => write!(f, "hook conflict: {k}"),
            ExtError::HooksOnD3D12RequireForWindow => {
                write!(f, "D3D12 hooks require window-scoped registration")
            }
            ExtError::Import(e) => write!(f, "import: {e}"),
            ExtError::ExpiredFrame {
                valid_until,
                current,
            } => write!(
                f,
                "frame already expired: valid_until={:?} <= current={:?}",
                valid_until, current
            ),
            ExtError::Backend(s) => write!(f, "backend: {s}"),
        }
    }
}

impl std::error::Error for ExtError {}

impl From<ImportError> for ExtError {
    fn from(e: ImportError) -> Self {
        if matches!(e, ImportError::UnsupportedSource) {
            ExtError::UnsupportedSource
        } else {
            ExtError::Import(e)
        }
    }
}

/// 纹理池插入结果（同 key 替换 vs 新增）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextureRegistration {
    Inserted,
    Replaced,
}

/// 平台/后端内部实现（由 HostWindow 注入）。
///
/// 2026-11-21 起只有**一个**资源动作：`submit_frame`。旧的 `register_texture` +
/// `update_texture` 两条路径合并 —— 同 key 再提交即原地替换，不再有独立的
/// 「单像素更新」后端入口（那是两表不通与死代码的根源）。
pub trait ExtContextImpl {
    /// 提交一帧到 `key`。同 key 再提交即原地替换（S7）。
    ///
    /// 实现必须：写 ExtPool（Hook 可见）、按 `desc.source` 分发上传、`valid_until`
    /// 随资源一起存进池，供渲染器每帧拒绝过期资源。
    fn submit_frame(
        &mut self,
        key: Cow<'static, str>,
        desc: FrameDesc,
        valid_until: FrameId,
    ) -> Result<RegisteredTexture, ExtError>;

    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError>;
    /// 判断 key 是否已注册（决定返回句柄是否拥有 RAII 卸载权）。
    fn has_texture(&self, _key: &str) -> bool {
        false
    }
    fn register_hook(
        &mut self,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) -> Result<(), ExtError>;
    /// D3D12/多窗口后端可覆盖；默认等同全局注册。
    fn register_hook_for_window(
        &mut self,
        _window: u64,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        self.register_hook(kind, hook)
    }
    /// `window: None` 全局，`Some(id)` 窗口作用域。
    fn unregister_hook(&mut self, kind: &str, window: Option<u64>) -> bool;
    fn capabilities(&self) -> BackendCapabilities;
    fn frame_id(&self) -> FrameId;
}

/// 扩展上下文操作接口（持有内部实现）。
pub struct ExtContext {
    inner: Box<dyn ExtContextImpl>,
}

impl ExtContext {
    pub fn new(inner: Box<dyn ExtContextImpl>) -> Self {
        Self { inner }
    }
    pub fn submit_frame(
        &mut self,
        key: Cow<'static, str>,
        desc: FrameDesc,
        valid_until: FrameId,
    ) -> Result<RegisteredTexture, ExtError> {
        self.inner.submit_frame(key, desc, valid_until)
    }
    pub fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError> {
        self.inner.unregister_texture(key)
    }
    pub fn has_texture(&self, key: &str) -> bool {
        self.inner.has_texture(key)
    }
    pub fn register_hook(
        &mut self,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        self.inner.register_hook(kind, hook)
    }
    pub fn register_hook_for_window(
        &mut self,
        window: u64,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) -> Result<(), ExtError> {
        self.inner.register_hook_for_window(window, kind, hook)
    }
    pub fn unregister_hook(&mut self, kind: &str, window: Option<u64>) -> bool {
        self.inner.unregister_hook(kind, window)
    }
    pub fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }
    pub fn frame_id(&self) -> FrameId {
        self.inner.frame_id()
    }
}

/// 扩展系统统一句柄（Phase 1：`!Send + !Sync`）。
#[derive(Clone)]
pub struct ExtContextHandle {
    pub(crate) inner: Rc<RefCell<ExtContext>>,
}

impl ExtContextHandle {
    pub fn new(inner: Box<dyn ExtContextImpl>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ExtContext::new(inner))),
        }
    }
    pub fn ctx(&self) -> Ref<'_, ExtContext> {
        self.inner.borrow()
    }
    pub fn ctx_mut(&self) -> RefMut<'_, ExtContext> {
        self.inner.borrow_mut()
    }
    pub fn clone_handle(&self) -> Self {
        self.clone()
    }

    /// 提交一帧（唯一资源动作，S7 同 key 替换）。
    ///
    /// 硬约束：`valid_until > frame_id()`。已经过期的帧直接拒绝 —— 否则渲染器
    /// 第一帧就会把它当过期资源剔除，提交本身没有意义。
    pub fn submit_frame(
        &self,
        key: impl Into<Cow<'static, str>>,
        desc: FrameDesc,
        valid_until: FrameId,
    ) -> Result<ExternalTextureHandle, ExtError> {
        let key = key.into();
        {
            let ctx = self.inner.borrow();
            let current = ctx.frame_id();
            if valid_until <= current {
                return Err(ExtError::ExpiredFrame {
                    valid_until,
                    current,
                });
            }
        }
        let owned = {
            let mut ctx = self.inner.borrow_mut();
            let fresh = !ctx.has_texture(&key);
            ctx.submit_frame(key.clone(), desc, valid_until)?;
            // 只有新建的 key 才在 Drop 时自动卸载；同 key 重复提交只替换内容。
            fresh
        };
        Ok(ExternalTextureHandle::new(
            key,
            Rc::downgrade(&self.inner),
            owned,
        ))
    }

    /// 兼容旧调用点：用构建器描述提交一帧，`valid_until` 设为永久有效。
    ///
    /// 新代码请直接 [`Self::submit_frame`]。此入口内部也走 submit_frame，不存在
    /// 独立的注册路径。
    pub fn register_texture(
        &self,
        builder: ExternalTextureBuilder,
    ) -> Result<ExternalTextureHandle, ExtError> {
        let (key, parts) = builder.into_parts();
        let desc = parts.clone().into_frame_desc().unwrap_or_else(|| {
            // 旧 `Auto` 源只是注册一个名字（没有像素）：合成一张透明 RGBA 占位帧，
            // 语义与旧的 TextureRef 注册一致，且仍走统一的 submit_frame 动作。
            let n = (parts.size.0 as usize) * (parts.size.1 as usize) * 4;
            FrameDesc::rgba(vec![0u8; n], parts.size.0, parts.size.1, parts.size.0 * 4)
        });
        self.submit_frame(key, desc, FrameId(u64::MAX))
    }

    pub fn register_hook(
        &self,
        kind: impl Into<Cow<'static, str>>,
        hook: Box<DrawHook>,
    ) -> Result<HookHandle, ExtError> {
        let kind = kind.into();
        self.inner.borrow_mut().register_hook(kind.clone(), hook)?;
        Ok(HookHandle::new(kind, None, Rc::downgrade(&self.inner)))
    }

    pub fn register_hook_for_window(
        &self,
        window: u64,
        kind: impl Into<Cow<'static, str>>,
        hook: Box<DrawHook>,
    ) -> Result<HookHandle, ExtError> {
        let kind = kind.into();
        self.inner
            .borrow_mut()
            .register_hook_for_window(window, kind.clone(), hook)?;
        Ok(HookHandle::new(
            kind,
            Some(window),
            Rc::downgrade(&self.inner),
        ))
    }
}
