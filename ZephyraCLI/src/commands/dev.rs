//! `dev` —— 开发模式：带 trace 跑起来，并在样式表或窗口配置变化时重启。
//!
//! # 和 `run` 的区别
//!
//! `run` 是「跑一次」。`dev` 多做两件事：
//!
//! 1. **打开 trace**（`ZEPHYRA_TRACE=1`）。不打开的话开发期最需要的那些行
//!    （视口变化、每帧四边形数、重建了没有）全都不打，而它们恰恰是排查
//!    「窗口变了但内容没跟上」的唯一判据。
//! 2. **盯样式表与窗口配置**。`.zstyle` 是**运行期**解析的（见 zstylec 的 lib 入口文档），
//!    所以改样式根本不需要重编译 —— 但我们不打算实现运行期的样式热重载，
//!    而是**重启进程**：那更简单也更可靠（不会残留已解析的旧样式表），
//!    而编译增量本来就已经把 Rust 那侧的等待压到很小。
//!
//! 刻意**不**盯 `.rs`：那正是 `cargo` 自己的事，两个 watcher 同时管同一份代码
//! 会互相触发，反而更难推理。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{app_config_path, cargo_path, CliError, Write};
use crate::args::Args;
use crate::ExitCode;

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    let cargo = cargo_path()?;

    // trace 默认**开**。要关就显式 `--no-trace` —— 反过来（默认关、要显式开）
    // 会让人在没 trace 的情况下排查半天「为什么日志里什么都没有」。
    if !a.has("no-trace") {
        // SAFETY：这是本进程启动时、任何其它线程都还没起来的时候。
        //     unsafe 是因为 std::env::set_var 在多线程下不安全；这里两个条件都满足。
        unsafe {
            std::env::set_var("ZEPHYRA_TRACE", "1");
        }
        w.err_line("[dev] ZEPHYRA_TRACE=1");
    }

    let style_dir = a
        .option("styles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("styles"));

    let release = a.has("release");
    let config_path = app_config_path(a)?;
    let mut cmd: Vec<String> = vec!["run".into()];
    if release {
        cmd.push("--release".into());
    }
    if let Some(p) = a.option("manifest-path") {
        cmd.push("--manifest-path".into());
        cmd.push(p.to_string());
    }

    w.err_line(&format!("[dev] cargo {}", cmd.join(" ")));
    if !a.has("no-watch") {
        w.err_line(&format!(
            "[dev] watching {}{} for changes (Ctrl-C to stop)",
            style_dir.display(),
            if config_path.is_some() {
                " and ZephyraUI.json"
            } else {
                ""
            }
        ));
    }

    let watch = !a.has("no-watch");
    let mut seen = snapshot(&style_dir, config_path.as_deref());

    let mut child = match spawn_cargo(&cargo, &cmd, config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            return Err(CliError::missing(format!(
                "cannot run {}: {e}",
                cargo.display()
            )))
        }
    };

    let mut last_check = SystemTime::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(if status.success() {
                    ExitCode::Success
                } else {
                    ExitCode::Failure
                });
            }
            Ok(None) => {}
            Err(e) => {
                return Err(CliError::failure(format!("cannot wait for child: {e}")));
            }
        }

        if !watch {
            // 没有 watch 时就是单纯等它跑完。
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // 轮询而不是用通知 API：样式表改动**很**稀疏，而轮询的代价（每 400ms 扫
        // 一次目录）低到可以忽略。用通知 API 反而要处理漏事件、去抖、重连。
        if last_check
            .elapsed()
            .map(|d| d >= Duration::from_millis(400))
            .unwrap_or(false)
        {
            last_check = SystemTime::now();
            let now = snapshot(&style_dir, config_path.as_deref());
            if now != seen {
                w.err_line("[dev] watched input changed, restarting");
                let _ = child.kill();
                let _ = child.wait();
                child = match spawn_cargo(&cargo, &cmd, config_path.as_deref()) {
                    Ok(c) => c,
                    Err(e) => {
                        return Err(CliError::missing(format!("cannot restart: {e}")));
                    }
                };
                // ⚠ 重启后**继续**盯，而不是退出循环 —— 上一版在这里 return，
                // 结果「改了样式就重启一次、然后命令就结束了」，第二次改动再也不管。
                // 那一版的症状极像「热重载只生效一次」。
                seen = now;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 目录里所有样式表和可选窗口配置的 `路径 -> 修改时刻`。**不含内容**：同一次改动
/// 多次保存不该触发多次重启，而按内容看会；按修改时刻看，编辑器连续写两遍也只算
/// 一次真正的变化（间隔会被轮询窗口吸收）。
fn spawn_cargo(
    cargo: &Path,
    args: &[String],
    config_path: Option<&Path>,
) -> std::io::Result<std::process::Child> {
    let mut command = std::process::Command::new(cargo);
    command.args(args);
    if let Some(path) = config_path {
        command.env("ZEPHYRA_APP_CONFIG_PATH", path);
    }
    command.spawn()
}

fn snapshot(dir: &Path, config_path: Option<&Path>) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("zstyle") {
                continue;
            }
            if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                out.push((p, t));
            }
        }
    }
    if let Some(path) = config_path {
        if let Ok(t) = std::fs::metadata(path).and_then(|m| m.modified()) {
            out.push((path.to_path_buf(), t));
        }
    }
    out.sort();
    out
}
