#![cfg(feature = "dynamic")]

//! 动态加载失败上报：缺库不能 panic，必须能通过报告暴露给主程序。

use zephyra_video::{current_report, library_status, load_all, LoadFailure};

#[test]
fn querying_unknown_library_is_safe() {
    assert!(library_status("__no_such_codec__").is_none());
}

#[test]
fn load_all_returns_a_report_even_with_failures() {
    // 测试环境里 DLL 通常存在；无论成功失败，都不应 panic。
    let report = load_all();
    let _ = report.is_ok();
    let _ = report.libraries().len();
    let _ = report.failure_summary();
    let _ = current_report();
}

#[test]
fn load_failure_is_actionable() {
    // 构造一个必定不存在的库名；load_all 只加载已启用的 codec，
    // 这里只验证失败类型的 Display 对主程序是可读的。
    let e = LoadFailure::NotFound {
        library: "z_missing",
        path: std::path::PathBuf::from("z_missing.dll"),
    };
    let msg = e.to_string();
    assert!(msg.contains("z_missing"));
    assert!(msg.contains("not found"));
}
