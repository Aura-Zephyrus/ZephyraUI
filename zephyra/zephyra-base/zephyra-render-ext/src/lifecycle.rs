//! Lifecycle notifications for render extensions.

use crate::types::Rect;

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

    #[test]
    fn render_lifecycle_defaults_are_noop() {
        struct L(bool);
        impl RenderLifecycle for L {
            fn on_device_lost(&mut self) {
                self.0 = true;
            }
        }
        let mut l = L(false);
        let ctx = LifecycleContext {
            size: (1, 1),
            damage: None,
            is_first: false,
        };
        l.on_prepare(&ctx);
        l.on_resize(&ctx);
        l.on_device_restored(&ctx);
        assert!(!l.0);
        l.on_device_lost();
        assert!(l.0);
    }
}
