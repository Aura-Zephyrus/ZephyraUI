//! Extension registration and hook interface.

use std::any::Any;
use std::borrow::Cow;
use zephyra_core::core::Rect;
use zephyra_core::paint::RenderSource;

pub trait HookContext: Send + Sync {
    fn size(&self) -> (f32, f32);
    fn damage(&self) -> Option<Rect>;
    fn is_offscreen(&self) -> bool;
    fn offscreen_pass_index(&self) -> Option<usize>;
}

/// 类型擦除的下转接口（不要求 Any/'static，允许非静态生命周期）。
///
/// 后端可选择实现此 trait 以支持运行时类型下转。
/// 默认实现返回 None，表示不支持下转。
pub trait HookContextDowncast {
    fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        None
    }
    fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        None
    }
}

pub type DrawHook = dyn FnMut(&mut dyn HookContext, Rect, &RenderSource) + Send + Sync;

pub trait RenderExtension {
    type HookCtx: HookContext + HookContextDowncast;
    fn register_hook(&mut self, kind: Cow<'static, str>, hook: Box<DrawHook>);
    fn unregister_hook(&mut self, kind: &str) -> bool;
    fn has_hook(&self, kind: &str) -> bool;
    fn registered_kinds(&self) -> Vec<Cow<'static, str>>;
}

pub trait ExtensionFactory: Send + Sync {
    fn create_and_register(
        &self,
        register: &mut dyn FnMut(Cow<'static, str>, Box<DrawHook>),
    ) -> Box<dyn Any + Send + Sync>;
}

#[macro_export]
macro_rules! render_extension_factory {
    (, , ) => {
        struct ;
        impl ::registration::ExtensionFactory for  {
            fn create_and_register(&self, register: &mut dyn FnMut(std::borrow::Cow<'static, str>, Box<::registration::DrawHook>)) -> Box<dyn std::any::Any + Send + Sync> {
                register(.into(), Box::new());
                Box::new(())
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    struct MockHookCtx;
    impl HookContext for MockHookCtx {
        fn size(&self) -> (f32, f32) {
            (800.0, 600.0)
        }
        fn damage(&self) -> Option<Rect> {
            None
        }
        fn is_offscreen(&self) -> bool {
            false
        }
        fn offscreen_pass_index(&self) -> Option<usize> {
            None
        }
    }
    impl HookContextDowncast for MockHookCtx {
        fn downcast_ref<T: 'static>(&self) -> Option<&T> {
            None
        }
        fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
            None
        }
    }
    #[test]
    fn hook_context_trait_object() {
        let ctx: &dyn HookContext = &MockHookCtx;
        assert_eq!(ctx.size(), (800.0, 600.0));
        assert!(!ctx.is_offscreen());
    }
    #[test]
    fn draw_hook_signature() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let called = Arc::new(AtomicBool::new(false));
        let flag = called.clone();
        let mut hook: Box<DrawHook> = Box::new(move |_ctx, _bounds, _source| {
            flag.store(true, Ordering::Relaxed);
        });
        let mut ctx = MockHookCtx;
        hook(
            &mut ctx,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            &RenderSource::none(),
        );
        assert!(called.load(Ordering::Relaxed));
    }
}
