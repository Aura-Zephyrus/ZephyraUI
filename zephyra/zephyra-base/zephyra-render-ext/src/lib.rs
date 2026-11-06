#![forbid(unsafe_code)]

/// Render extension contract crate.
/// Zero system API, forbid(unsafe_code).
pub mod capability;
pub mod lifecycle;
pub mod registration;
pub mod report;
pub mod hook;
pub mod source;

pub use zephyra_core::paint::RenderSource;
pub use zephyra_fx::offscreen::{OffscreenPass, BlurStep, pass_quad, composite_quad};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
