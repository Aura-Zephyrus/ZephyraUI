#![forbid(unsafe_code)]

/// Render extension contract crate.
/// Zero system API, forbid(unsafe_code).
pub mod capability;
pub mod hook;
pub mod lifecycle;
pub mod registration;
pub mod report;
pub mod source;

pub use zephyra_core::paint::RenderSource;
pub use zephyra_fx::offscreen::{composite_quad, pass_quad, BlurStep, OffscreenPass};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
