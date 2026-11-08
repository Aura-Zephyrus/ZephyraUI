//! RAII 句柄（§3.5，P0-2/S7）。

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Weak;

use crate::debug;
use crate::ext_context::{ExtContext, ExtError};
use crate::texture_builder::ExternalTextureBuilder;

/// 外部纹理 RAII 句柄：Drop 时自动 unregister。仅 UI/渲染线程。
pub struct ExternalTextureHandle {
    key: Cow<'static, str>,
    ctx: Weak<RefCell<ExtContext>>,
    unregister_on_drop: bool,
}

impl ExternalTextureHandle {
    pub(crate) fn new(
        key: Cow<'static, str>,
        ctx: Weak<RefCell<ExtContext>>,
        unregister_on_drop: bool,
    ) -> Self {
        Self {
            key,
            ctx,
            unregister_on_drop,
        }
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    /// 同 key 替换纹理内容/源（S7）。
    pub fn update(&self, builder: ExternalTextureBuilder) -> Result<(), ExtError> {
        let Some(ctx) = self.ctx.upgrade() else {
            return Err(ExtError::ContextDropped);
        };
        // 强制同 key 替换
        let (k, parts) = builder.into_parts();
        let _ = k;
        let builder = ExternalTextureBuilder::new(self.key.clone(), parts.size)
            .format(parts.format)
            .source_auto(parts.source)
            .sync_auto(parts.sync);
        let builder = if let Some(cs) = parts.color_space {
            builder.color_space(cs)
        } else {
            builder
        };
        let builder = if let Some(h) = parts.hdr_metadata {
            builder.hdr_metadata(h)
        } else {
            builder
        };
        ctx.borrow_mut().register_texture_parts(builder)?;
        Ok(())
    }
    pub fn try_unregister(&self) -> Result<(), ExtError> {
        let Some(ctx) = self.ctx.upgrade() else {
            return Err(ExtError::ContextDropped);
        };
        let r = ctx.borrow_mut().unregister_texture(&self.key);
        r
    }
    pub fn unregister(self) -> Result<(), ExtError> {
        self.try_unregister()
    }
}

impl Drop for ExternalTextureHandle {
    fn drop(&mut self) {
        if !self.unregister_on_drop {
            return;
        }
        if let Some(ctx) = self.ctx.upgrade() {
            if let Err(e) = ctx.borrow_mut().unregister_texture(&self.key) {
                debug::log_texture(&format!("drop unregister failed key={}: {e}", self.key));
            }
        }
    }
}

/// Hook RAII 句柄。`window = None` 为全局，`Some(id)` 为窗口作用域。
pub struct HookHandle {
    kind: Cow<'static, str>,
    window: Option<u64>,
    ctx: Weak<RefCell<ExtContext>>,
}

impl HookHandle {
    pub(crate) fn new(
        kind: Cow<'static, str>,
        window: Option<u64>,
        ctx: Weak<RefCell<ExtContext>>,
    ) -> Self {
        Self { kind, window, ctx }
    }
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn window(&self) -> Option<u64> {
        self.window
    }
}

impl Drop for HookHandle {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.upgrade() {
            let _ = ctx.borrow_mut().unregister_hook(&self.kind, self.window);
        }
    }
}
