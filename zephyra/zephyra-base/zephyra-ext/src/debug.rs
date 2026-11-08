//! 调试开关（§9.5）：debug 默认可开，release 编译期剔除。

#[cfg(debug_assertions)]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SYNC: AtomicBool = AtomicBool::new(false);
    static TEXTURE: AtomicBool = AtomicBool::new(false);
    static HOOK: AtomicBool = AtomicBool::new(false);
    static FALLBACK: AtomicBool = AtomicBool::new(false);

    pub fn init_from_env() {
        SYNC.store(
            std::env::var_os("ZEPHYRA_DEBUG_SYNC").is_some(),
            Ordering::Relaxed,
        );
        TEXTURE.store(
            std::env::var_os("ZEPHYRA_DEBUG_TEXTURE").is_some(),
            Ordering::Relaxed,
        );
        HOOK.store(
            std::env::var_os("ZEPHYRA_DEBUG_HOOK").is_some(),
            Ordering::Relaxed,
        );
        FALLBACK.store(
            std::env::var_os("ZEPHYRA_DEBUG_FALLBACK").is_some(),
            Ordering::Relaxed,
        );
    }
    #[inline]
    pub fn log_sync(msg: &str) {
        if SYNC.load(Ordering::Relaxed) {
            eprintln!("[ZEPHYRA SYNC] {msg}");
        }
    }
    #[inline]
    pub fn log_texture(msg: &str) {
        if TEXTURE.load(Ordering::Relaxed) {
            eprintln!("[ZEPHYRA TEX] {msg}");
        }
    }
    #[inline]
    pub fn log_hook(msg: &str) {
        if HOOK.load(Ordering::Relaxed) {
            eprintln!("[ZEPHYRA HOOK] {msg}");
        }
    }
    #[inline]
    pub fn log_fallback(msg: &str) {
        if FALLBACK.load(Ordering::Relaxed) {
            eprintln!("[ZEPHYRA FALLBACK] {msg}");
        }
    }
}

#[cfg(not(debug_assertions))]
mod imp {
    #[inline]
    pub fn init_from_env() {
        if std::env::var_os("ZEPHYRA_VALIDATION").is_some() {
            eprintln!("ZEPHYRA_VALIDATION ignored in release build");
        }
    }
    #[inline]
    pub fn log_sync(_: &str) {}
    #[inline]
    pub fn log_texture(_: &str) {}
    #[inline]
    pub fn log_hook(_: &str) {}
    #[inline]
    pub fn log_fallback(_: &str) {}
}

pub use imp::*;
