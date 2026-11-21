//! RAII 句柄（§3.5，P0-2/S7）。

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Weak;

use zephyra_render_ext::prelude::{FrameDesc, RegisteredTexture};

use crate::debug;
use crate::ext_context::{ExtContext, ExtError};

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
    /// 同 key 再提交一帧（S7 原地替换）。
    ///
    /// 与扩展上下文的 submit_frame 走同一个动作，不存在独立的
    /// 「更新」路径。`valid_until` 必须大于当前帧号。
    pub fn update(
        &self,
        desc: FrameDesc,
        valid_until: zephyra_render_ext::prelude::FrameId,
    ) -> Result<RegisteredTexture, ExtError> {
        let ctx = self.ctx.upgrade().ok_or(ExtError::ContextDropped)?;
        {
            let c = ctx.borrow();
            if valid_until <= c.frame_id() {
                return Err(ExtError::ExpiredFrame {
                    valid_until,
                    current: c.frame_id(),
                });
            }
        }
        let result = ctx
            .borrow_mut()
            .submit_frame(self.key.clone(), desc, valid_until);
        result
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
