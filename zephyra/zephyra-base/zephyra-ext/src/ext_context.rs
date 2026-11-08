//! 扩展上下文（§3.2，P1-1 单线程）。

use std::borrow::Cow;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use zephyra_render_ext::prelude::{BackendCapabilities, DrawHook, FrameId, ImportError};

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

/// 平台/后端内部实现（由 HostWindow 注入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureRegistration {
    Inserted,
    Replaced,
}

pub trait ExtContextImpl {
    fn register_texture(
        &mut self,
        builder: ExternalTextureBuilder,
    ) -> Result<TextureRegistration, ExtError>;
    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError>;
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
    pub fn register_texture_parts(
        &mut self,
        builder: ExternalTextureBuilder,
    ) -> Result<TextureRegistration, ExtError> {
        self.inner.register_texture(builder)
    }
    pub fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError> {
        self.inner.unregister_texture(key)
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

    /// 注册/替换纹理（S7：同 key 替换）。
    pub fn register_texture(
        &self,
        builder: ExternalTextureBuilder,
    ) -> Result<ExternalTextureHandle, ExtError> {
        let key = builder.key.clone();
        let reg = self.inner.borrow_mut().register_texture_parts(builder)?;
        let owned = reg == TextureRegistration::Inserted;
        Ok(ExternalTextureHandle::new(
            key,
            Rc::downgrade(&self.inner),
            owned,
        ))
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
