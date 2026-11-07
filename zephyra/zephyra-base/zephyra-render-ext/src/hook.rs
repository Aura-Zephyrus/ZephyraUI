//! Hook context traits for render extensions.

use std::any::Any;
use zephyra_core::core::Rect;

pub trait HookContext: Send + Sync {
    fn size(&self) -> (f32, f32);
    fn damage(&self) -> Option<Rect>;
    fn is_offscreen(&self) -> bool;
    fn offscreen_pass_index(&self) -> Option<usize>;
}

pub trait HookContextDowncast: Any {
    fn downcast_ref<T: 'static>(&self) -> Option<&T>;
    fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T>;
}

pub trait D3D11HookContext: HookContext + HookContextDowncast {}
pub trait D3D12HookContext: HookContext + HookContextDowncast {}
pub trait VulkanHookContext: HookContext + HookContextDowncast {}

pub trait GpuResourceHandle: Any + Send + Sync {}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestCtx;
    impl HookContext for TestCtx {
        fn size(&self) -> (f32, f32) {
            (100.0, 100.0)
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
    impl HookContextDowncast for TestCtx {
        fn downcast_ref<T: 'static>(&self) -> Option<&T> {
            None
        }
        fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
            None
        }
    }
    #[test]
    fn hook_context_basic() {
        let ctx = TestCtx;
        assert_eq!(ctx.size(), (100.0, 100.0));
        assert!(!ctx.is_offscreen());
    }
    #[test]
    fn hook_context_downcast() {
        let ctx = TestCtx;
        let result: Option<&i32> = ctx.downcast_ref();
        assert!(result.is_none());
    }
}
