//! Hook 注册（§3.4, §6.1）。

use std::cell::{RefCell, RefMut};
use std::rc::{Rc, Weak};

use zephyra_render_ext::prelude::{DrawHook, Rect, RenderSource};

use crate::ext_context::{ExtContext, ExtError};
use crate::handles::HookHandle;

/// Hook 注册表（持有 `ExtContext` 可变借用 + weak，用于返回 RAII 句柄）。
pub struct HookRegistry<'a> {
    ctx: RefMut<'a, ExtContext>,
    weak: Weak<RefCell<ExtContext>>,
}

impl<'a> HookRegistry<'a> {
    pub(crate) fn new(ctx: RefMut<'a, ExtContext>, weak: Weak<RefCell<ExtContext>>) -> Self {
        Self { ctx, weak }
    }

    pub fn generic<F>(&mut self, kind: &str, mut hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        let kind_s = kind.to_string();
        let hook: Box<DrawHook> = Box::new(move |ctx, rect, src| hook(ctx, rect, src));
        self.ctx.register_hook(kind_s.clone().into(), hook)?;
        Ok(HookHandle::new(kind_s.into(), None, self.weak.clone()))
    }

    pub fn for_window(&mut self, window: u64) -> WindowHookRegistry<'_> {
        WindowHookRegistry {
            ctx: &mut self.ctx,
            weak: self.weak.clone(),
            window,
        }
    }

    #[cfg(windows)]
    pub fn d3d11<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        self.generic(kind, hook)
    }

    #[cfg(windows)]
    pub fn d3d12<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        self.generic(kind, hook)
    }

    #[cfg(target_os = "linux")]
    pub fn vulkan<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        self.generic(kind, hook)
    }
}

pub struct WindowHookRegistry<'a> {
    ctx: &'a mut ExtContext,
    weak: Weak<RefCell<ExtContext>>,
    window: u64,
}

impl<'a> WindowHookRegistry<'a> {
    pub fn generic<F>(&mut self, kind: &str, mut hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        let kind_s = kind.to_string();
        let hook: Box<DrawHook> = Box::new(move |ctx, rect, src| hook(ctx, rect, src));
        self.ctx
            .register_hook_for_window(self.window, kind_s.clone().into(), hook)?;
        Ok(HookHandle::new(
            kind_s.into(),
            Some(self.window),
            self.weak.clone(),
        ))
    }

    #[cfg(windows)]
    pub fn d3d11<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        self.generic(kind, hook)
    }

    #[cfg(windows)]
    pub fn d3d12<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        self.generic(kind, hook)
    }

    #[cfg(target_os = "linux")]
    pub fn vulkan<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: for<'s> FnMut(
                &mut dyn zephyra_render_ext::prelude::HookContext,
                Rect,
                &RenderSource<'s>,
            ) + Send
            + 'static,
    {
        self.generic(kind, hook)
    }
}

impl crate::ext_context::ExtContextHandle {
    /// 获取 Hook 注册表（可变借用上下文）。返回的 `HookHandle` 为 RAII；丢弃将注销 hook。
    pub fn hooks(&self) -> HookRegistry<'_> {
        let weak = Rc::downgrade(&self.inner);
        HookRegistry::new(self.inner.borrow_mut(), weak)
    }
}
