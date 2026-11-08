//! GPU 同步原语（契约）。框架只负责 wait，不创建不 signal。

use std::sync::Arc;

/// 同步错误。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyncError {
    DeviceLost,
    InvalidHandle(String),
    Backend(String),
}

impl core::fmt::Display for SyncError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SyncError::DeviceLost => write!(f, "device lost"),
            SyncError::InvalidHandle(s) => write!(f, "invalid handle: {s}"),
            SyncError::Backend(s) => write!(f, "backend: {s}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// 导出句柄（不透明；内部可含 NT Handle/FD，由后端解释）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceHandle {
    pub raw: usize,
    pub kind: FenceHandleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FenceHandleKind {
    Unknown,
    NtHandle,
    Fd,
}

/// GPU 围栏统一抽象（应用创建，框架只 wait）。
pub trait GpuFence: Send + Sync {
    fn signaled_value(&self) -> u64;
    /// 在提交消费者命令前确保生产者完成（S3）。优先 GPU 队列等待；回退 CPU 等待须避免阻塞 UI。
    fn wait(&self, value: u64) -> Result<(), SyncError>;
    fn export_handle(&self) -> Result<FenceHandle, SyncError>;
    /// 跨进程导出场景的句柄克隆；共享使用 `Arc<dyn GpuFence>`。
    fn clone_handle(&self) -> Box<dyn GpuFence>;
}

/// 统一同步输入（S4：围栏用 `Arc` 复用）。
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum SyncInput {
    #[default]
    Auto,
    Fence(Arc<dyn GpuFence>),
    None,
}

impl core::fmt::Debug for SyncInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SyncInput::Auto => write!(f, "Auto"),
            SyncInput::Fence(_) => write!(f, "Fence(..)"),
            SyncInput::None => write!(f, "None"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct F(Arc<AtomicU64>);
    impl GpuFence for F {
        fn signaled_value(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
        fn wait(&self, _value: u64) -> Result<(), SyncError> {
            Ok(())
        }
        fn export_handle(&self) -> Result<FenceHandle, SyncError> {
            Ok(FenceHandle {
                raw: 1,
                kind: FenceHandleKind::Unknown,
            })
        }
        fn clone_handle(&self) -> Box<dyn GpuFence> {
            Box::new(F(self.0.clone()))
        }
    }

    #[test]
    fn sync_input_fence_arc() {
        let f: Arc<dyn GpuFence> = Arc::new(F(Arc::new(AtomicU64::new(3))));
        let s = SyncInput::Fence(f.clone());
        if let SyncInput::Fence(g) = s {
            assert_eq!(g.signaled_value(), 3);
            assert!(g.wait(3).is_ok());
            assert!(g.export_handle().is_ok());
            let _ = g.clone_handle();
        } else {
            panic!("expected fence");
        }
    }

    #[test]
    fn sync_input_default_and_debug() {
        assert!(matches!(SyncInput::default(), SyncInput::Auto));
        assert!(format!("{:?}", SyncInput::Auto).contains("Auto"));
        assert!(format!("{:?}", SyncInput::None).contains("None"));
        let f: Arc<dyn GpuFence> = Arc::new(F(Arc::new(AtomicU64::new(0))));
        assert!(format!("{:?}", SyncInput::Fence(f)).contains("Fence"));
    }

    #[test]
    fn sync_error_display_and_variants() {
        assert!(SyncError::DeviceLost.to_string().contains("device lost"));
        assert!(SyncError::InvalidHandle("x".into())
            .to_string()
            .contains("invalid handle"));
        assert!(SyncError::Backend("b".into())
            .to_string()
            .contains("backend"));
        let e: &dyn std::error::Error = &SyncError::DeviceLost;
        let _ = e.to_string();
    }

    #[test]
    fn fence_handle_kinds() {
        let h = FenceHandle {
            raw: 7,
            kind: FenceHandleKind::NtHandle,
        };
        assert_eq!(h.raw, 7);
        assert_eq!(h.kind, FenceHandleKind::NtHandle);
        assert_ne!(FenceHandleKind::Fd, FenceHandleKind::Unknown);
    }
}
