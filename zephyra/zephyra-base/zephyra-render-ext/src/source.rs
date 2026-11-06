//! RenderSource convenience constructors.

use std::any::Any;

pub use zephyra_core::paint::RenderSource;

pub fn none() -> RenderSource { RenderSource::none() }

pub fn custom(kind: impl Into<std::borrow::Cow<'static, str>>, payload: impl Any + Send + Sync) -> RenderSource {
    RenderSource::new(kind, payload)
}

pub fn custom_shared<T: Send + Sync + 'static>(kind: impl Into<std::borrow::Cow<'static, str>>, payload: std::sync::Arc<T>) -> RenderSource {
    RenderSource::new(kind, payload)
}

pub mod kind {
    pub const VIDEO: &str = "video";
    pub const CHART_GPU: &str = "chart:gpu";
    pub const EMBED: &str = "embed";
    pub const CUSTOM: &str = "custom";
    pub const DEBUG: &str = "debug";
}

#[cfg(test)]
mod tests {
    use super::*; use std::sync::Arc;
    #[derive(Clone, Debug, PartialEq)] struct TestPayload { value: u32 }
    #[test] fn source_none() { let src = none(); assert!(src.is_none()); assert_eq!(src.kind, ""); }
    #[test] fn source_custom() { let src = custom("test:kind", TestPayload { value: 42 }); assert!(!src.is_none()); assert_eq!(src.kind, "test:kind"); assert_eq!(src.payload::<TestPayload>().unwrap().value, 42); }
    #[test] fn source_custom_shared() { 
        let arc = Arc::new(TestPayload { value: 100 }); 
        let src1 = custom_shared("shared", arc.clone()); 
        let src2 = custom_shared("shared", arc); 
        assert_eq!(src1.kind, src2.kind); 
        let v1 = src1.payload::<std::sync::Arc<TestPayload>>().map(|a| a.value).unwrap_or_else(|| src1.payload::<TestPayload>().map(|a| a.value).unwrap_or(0));
        let v2 = src2.payload::<std::sync::Arc<TestPayload>>().map(|a| a.value).unwrap_or_else(|| src2.payload::<TestPayload>().map(|a| a.value).unwrap_or(0));
        assert_eq!(v1, v2); 
    }
    #[test] fn source_kind_constants() { assert_eq!(kind::VIDEO, "video"); assert_eq!(kind::CUSTOM, "custom"); }
}
