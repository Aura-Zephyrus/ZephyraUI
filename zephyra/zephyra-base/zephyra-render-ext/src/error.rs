//! 跨后端统一错误类型。

use crate::types::ExternalTextureFormat;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportError {
    InvalidHandle(String),
    FormatMismatch {
        expected: ExternalTextureFormat,
        actual: ExternalTextureFormat,
    },
    SizeMismatch {
        expected: (u32, u32),
        actual: (u32, u32),
    },
    UnsupportedSource,
    DeviceLost,
    Backend(String),
}

impl ImportError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, ImportError::DeviceLost)
    }
}

/// Windows HRESULT -> `ImportError` 映射辅助（仅数值，不引入 `windows` crate）。
pub fn map_hresult_to_import_error(hr: i32) -> ImportError {
    match hr as u32 {
        0x8007_0057 => ImportError::InvalidHandle("E_INVALIDARG".into()),
        0x8007_0005 => ImportError::InvalidHandle("E_ACCESSDENIED".into()),
        0x887A_0005 | 0x887A_0007 => ImportError::DeviceLost,
        0x887A_0003 => ImportError::UnsupportedSource,
        0x8007_000E => ImportError::Backend("OOM".into()),
        0xC000_0008 => ImportError::InvalidHandle("STATUS_INVALID_HANDLE".into()),
        _ => ImportError::Backend(format!("HRESULT 0x{hr:08X}")),
    }
}

impl core::fmt::Display for ImportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ImportError::InvalidHandle(s) => write!(f, "invalid handle: {s}"),
            ImportError::FormatMismatch { expected, actual } => {
                write!(f, "format mismatch: expected {expected:?}, got {actual:?}")
            }
            ImportError::SizeMismatch { expected, actual } => {
                write!(f, "size mismatch: expected {expected:?}, got {actual:?}")
            }
            ImportError::UnsupportedSource => write!(f, "unsupported source"),
            ImportError::DeviceLost => write!(f, "device lost"),
            ImportError::Backend(s) => write!(f, "backend: {s}"),
        }
    }
}

impl std::error::Error for ImportError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_only_device_lost() {
        assert!(ImportError::DeviceLost.is_retryable());
        assert!(!ImportError::UnsupportedSource.is_retryable());
        assert!(!ImportError::InvalidHandle("x".into()).is_retryable());
        assert!(!ImportError::Backend("x".into()).is_retryable());
        assert!(!ImportError::FormatMismatch {
            expected: ExternalTextureFormat::Rgba8Unorm,
            actual: ExternalTextureFormat::Bgra8Unorm,
        }
        .is_retryable());
        assert!(!ImportError::SizeMismatch {
            expected: (1, 1),
            actual: (2, 2),
        }
        .is_retryable());
    }

    #[test]
    fn display_messages() {
        assert!(ImportError::DeviceLost.to_string().contains("device lost"));
        assert!(ImportError::UnsupportedSource
            .to_string()
            .contains("unsupported"));
        assert!(ImportError::InvalidHandle("h".into())
            .to_string()
            .contains("invalid handle"));
        assert!(ImportError::Backend("b".into())
            .to_string()
            .contains("backend"));
        assert!(ImportError::FormatMismatch {
            expected: ExternalTextureFormat::Rgba8Unorm,
            actual: ExternalTextureFormat::Bgra8Unorm,
        }
        .to_string()
        .contains("format mismatch"));
        assert!(ImportError::SizeMismatch {
            expected: (1, 1),
            actual: (2, 2),
        }
        .to_string()
        .contains("size mismatch"));
    }

    #[test]
    fn hresult_mapping() {
        assert!(matches!(
            map_hresult_to_import_error(0x887A0005u32 as i32),
            ImportError::DeviceLost
        ));
        assert!(matches!(
            map_hresult_to_import_error(0x887A0007u32 as i32),
            ImportError::DeviceLost
        ));
        assert!(matches!(
            map_hresult_to_import_error(0x887A0003u32 as i32),
            ImportError::UnsupportedSource
        ));
        assert!(matches!(
            map_hresult_to_import_error(0x80070057u32 as i32),
            ImportError::InvalidHandle(_)
        ));
        assert!(matches!(
            map_hresult_to_import_error(0x8007000Eu32 as i32),
            ImportError::Backend(_)
        ));
        assert!(matches!(
            map_hresult_to_import_error(0x12345678u32 as i32),
            ImportError::Backend(_)
        ));
    }
}
