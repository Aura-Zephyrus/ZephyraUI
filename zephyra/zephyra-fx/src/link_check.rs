//! 动态库缺库自检。
//!
//! `zephyra-video` 的 build.rs 在生成共享库时会把产物文件名写进 exe 同目录的
//! `zephyra_dynamic_libs.txt`。本模块启动时读它，逐个检查文件是否存在；
//! 缺失时给出明确错误（调用方映射为退出码 3），而不是让系统加载器抛一句
//! 「找不到指定的模块」。

use std::path::PathBuf;

/// 返回缺失的库文件名（空 = 全部就位）。没有清单文件时视为不需要检查。
pub fn missing_dynamic_libs() -> Vec<String> {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    else {
        return Vec::new();
    };
    let manifest = dir.join("zephyra_dynamic_libs.txt");
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !dir.join(name).exists())
        .map(str::to_string)
        .collect()
}

/// 缺库时返回可直接展示的错误文案。
pub fn ensure_dynamic_libs() -> Result<(), String> {
    let missing = missing_dynamic_libs();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "缺少运行库：{}\n把上述文件放到可执行文件同目录，或用 `zephyra build --sf` 构建单文件版本。",
        missing.join("、")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_manifest_means_no_check() {
        // 测试进程目录下通常没有该清单；有也不该 panic。
        let _ = missing_dynamic_libs();
    }
}
