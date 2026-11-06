//! Lifecycle notifications for render extensions.

use zephyra_core::core::Rect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LifecycleContext {
    pub size: (u32, u32),
    pub damage: Option<Rect>,
    pub is_first: bool,
}

/// 生命周期钩子 trait（不要求 Send + Sync，因为渲染器通常不跨线程）。
pub trait RenderLifecycle {
    fn on_prepare(&mut self, _ctx: &LifecycleContext) {}
    fn on_resize(&mut self, _ctx: &LifecycleContext) {}
    fn on_device_lost(&mut self) {}
    fn on_device_restored(&mut self, _ctx: &LifecycleContext) {}
}

pub trait LifecycleHook: RenderLifecycle {}
impl<T: RenderLifecycle> LifecycleHook for T {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_context_debug() {
        let ctx = LifecycleContext {
            size: (1920, 1080),
            damage: Some(Rect::new(100.0, 100.0, 200.0, 200.0)),
            is_first: true,
        };
        let s = format!("{:?}", ctx);
        assert!(s.contains("1920"));
        assert!(s.contains("1080"));
    }
}
