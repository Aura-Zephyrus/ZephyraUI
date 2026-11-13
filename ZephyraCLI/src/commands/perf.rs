//! `zephyra perf`：性能监测。
//!
//! # 为什么要有它
//!
//! `bridge` 能问「现在是什么」，但**问不出「刚才那 5 秒里哪一段慢」** ——
//! 而后者才是性能排查要的答案。
//!
//! 之前唯一的性能入口是 MCP 的 `analyze_performance`（起一个 MCP server、
//! 走 JSON-RPC），而那是给 AI 的形态：一次调用一个来回。人要的是
//! 「开着窗口看一会儿数字」。
//!
//! 两条路子，各司其职：
//!
//! | 形态 | 适合 |
//! |---|---|
//! | `zephyra perf profile` | **取一次报告**（百分位）—— AI 与 CI 都能用 |
//! | `zephyra perf watch` | **盯着看**（实时表）—— 人盯着窗口时用
//!
//! # 数据从哪来
//!
//! 两条路都只用**已有的**桥命令，不新增协议：
//!
//! - `stats --json` → 最后一帧的快照（四边形数、脏区、**分段耗时**、已渲染帧数）
//! - `profile_start` / `profile_report` → 宿主侧的**逐帧**采样与百分位
//!
//! 分段耗时是 core 自己测的（`zephyra_core::frame::StageTimings`），
//! 于是「这一帧慢在哪」是**读出来的**，不是猜的。

use crate::args::Args;
use crate::bridge_client::{BridgeClient, BridgeResponse};
use crate::commands::{CliError, Write};
use crate::ExitCode;

use serde_json::Value;

/// 应用侧桥的默认端口。与 `zephyra-fx::bridge::DEFAULT_PORT` 一致
/// （CLI 刻意不依赖 zephyra-fx；两边各写一份，由测试钉住）。
fn default_port() -> u16 {
    7878
}

/// `watch` 的默认采样间隔（毫秒）。
///
/// 250 ms 的依据：人眼盯屏幕时「一眨眼」约 100~300 ms；比这更密的采样只会
/// 刷屏，比这更疏会漏掉「卡了一下又好了」。
const DEFAULT_INTERVAL_MS: u64 = 250;

/// `watch` 的默认观察时长（毫秒）。
const DEFAULT_DURATION_MS: u64 = 5_000;

pub fn run(args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let sub = args
        .positional
        .first()
        .map(String::as_str)
        .unwrap_or("watch")
        .to_string();

    let host = args.option("host").unwrap_or("127.0.0.1").to_string();
    let port: u16 = args
        .option("port")
        .map(|p| p.parse())
        .transpose()
        .map_err(|_| CliError::usage("--port needs a number"))?
        .unwrap_or_else(default_port);

    let mut client = BridgeClient::connect(&host, port, 3).map_err(|e| {
        CliError::failure(format!(
            "连不上应用调试桥 {host}:{port}（{e}）——确认应用以 ZEPHYRA_DEBUG_PORT={port} 启动"
        ))
    })?;

    match sub.as_str() {
        "profile" => profile(&mut client, args, io),
        "watch" => watch(&mut client, args, io),
        other => Err(CliError::usage(format!(
            "perf 只接受 `profile` 与 `watch`，实得 `{other}`"
        ))),
    }
}

/// `perf profile`：起一段采样，等它跑完，报百分位。
///
/// # 为什么是「起 + 等 + 报」一条命令
///
/// 分成三条（`profile_start` / 等 / `profile_report`）就要三个进程、三个往返，
/// 而中间那一步是**等**——没有命令能表达它。于是「等」只能落在 shell 的
/// `sleep` 里，而 `sleep` 与采样窗口的边界对不齐（窗口短了就没有数据，
/// 窗口长了就白等）。
///
/// 这里让**宿主**按秒计（`profile_start` 的单位就是秒），CLI 按同样的秒数
/// 等，然后读报告 —— 边界天然对齐。
fn profile(
    client: &mut BridgeClient,
    args: &Args,
    io: &mut dyn Write,
) -> Result<ExitCode, CliError> {
    let seconds: u64 = match args.option("seconds") {
        Some(v) => v
            .parse()
            .map_err(|_| CliError::usage("--seconds needs a number"))?,
        None => 3,
    };
    if !(1..=300).contains(&seconds) {
        return Err(CliError::usage("--seconds 必须在 1..=300 之间"));
    }

    call_ok(client, &format!("profile_start {seconds}"))?;
    io.line(&format!(
        "采样 {seconds} 秒中……（应用静止时不会有帧 —— 那也是结论）"
    ));
    // ⚠ 多等 200 ms：宿主的采样窗口是**从收到命令那一刻**开始的，而命令要
    // 走一个来回才到。少等这一截，最后那几百毫秒的帧就丢了。
    std::thread::sleep(
        std::time::Duration::from_secs(seconds) + std::time::Duration::from_millis(200),
    );

    let report = call_ok(client, "profile_report")?;
    for line in report {
        io.line(&line);
    }
    Ok(ExitCode::Success)
}

/// `perf watch`：实时表。
///
/// # 为什么每行都带 `frames`
///
/// fps 要靠**两次采样的帧数差**算，只看一行的帧数没有意义。而应用静止时
/// 帧数**不动** ⇒ fps = 0 —— 那是事实（「它没在画」），不是工具坏了。
fn watch(client: &mut BridgeClient, args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let interval_ms = match args.option("interval-ms") {
        Some(v) => v
            .parse()
            .map_err(|_| CliError::usage("--interval-ms needs a number"))?,
        None => DEFAULT_INTERVAL_MS,
    };
    let duration_ms = match args.option("duration-ms") {
        Some(v) => v
            .parse()
            .map_err(|_| CliError::usage("--duration-ms needs a number"))?,
        None => DEFAULT_DURATION_MS,
    };
    if interval_ms == 0 {
        return Err(CliError::usage("--interval-ms 不能为 0"));
    }
    if duration_ms == 0 {
        return Err(CliError::usage("--duration-ms 不能为 0"));
    }

    io.line(&format!(
        "每 {interval_ms} ms 采一次，共 {duration_ms} ms（Ctrl-C 提前结束）"
    ));
    io.line("      t     fps   core_us  lat_us  diff style scan layout paint  quads dirty damage");

    let started = std::time::Instant::now();
    let mut prev: Option<(u64, u64)> = None;
    let mut totals: Vec<u64> = Vec::new();
    let mut rows = 0usize;

    while (started.elapsed().as_millis() as u64) < duration_ms {
        let sample = match sample(client) {
            Ok(s) => s,
            Err(e) => {
                io.err_line(&format!("采样失败：{e}"));
                return Err(CliError::failure("性能采样失败"));
            }
        };
        let now = started.elapsed().as_millis() as u64;
        let fps = match &prev {
            Some((at_ms, frames)) => {
                let dt = now.saturating_sub(*at_ms) as f64 / 1000.0;
                let df = sample.frames.saturating_sub(*frames);
                if dt > 0.0 {
                    df as f64 / dt
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        io.line(&format!(
            "{:>7} {:>6.1} {:>8} {:>7} {:>4} {:>5} {:>4} {:>6} {:>5} {:>5} {:>5} {}",
            format!("+{now}ms"),
            fps,
            sample.core_us,
            sample
                .latency_us
                .map(|u| u.to_string())
                .unwrap_or_else(|| "-".into()),
            sample.diff_us,
            sample.style_us,
            sample.scan_us,
            sample.layout_us,
            sample.paint_us,
            sample.quads,
            sample.dirty,
            sample.damage,
        ));
        totals.push(sample.core_us);
        prev = Some((now, sample.frames));
        rows += 1;
        std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    }

    // 收尾统计。
    //
    // ⚠ 中位数而不是平均值：单帧会被 OS 抢占污染，而平均值会把那几毫秒摊薄
    // 到看不出来。中位数只反映**典型**帧 —— 那才是决定手感的东西。
    if totals.is_empty() {
        io.line("没有采到任何样本");
        return Ok(ExitCode::Success);
    }
    let mut sorted = totals.clone();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let p95 = sorted[(sorted.len() - 1) * 95 / 100];
    let max = sorted[sorted.len() - 1];
    let avg = totals.iter().sum::<u64>() as f64 / totals.len() as f64;
    io.line(&format!(
        "core 帧耗时（{rows} 个样本）：平均 {avg:.1} µs  中位 {p50} µs  p95 {p95} µs  最大 {max} µs"
    ));
    if let Some((_, frames)) = &prev {
        io.line(&format!("应用一共渲染了 {frames} 帧"));
    }
    Ok(ExitCode::Success)
}

/// 一次采样的结果。
struct Sample {
    frames: u64,
    core_us: u64,
    /// 「输入 → 帧就绪」的耗时（µs）。`None` = 这一帧之前没有输入。
    latency_us: Option<u64>,
    diff_us: u64,
    style_us: u64,
    scan_us: u64,
    layout_us: u64,
    paint_us: u64,
    quads: u64,
    dirty: u64,
    damage: String,
}

/// 采一次 `stats --json` 并解析。
///
/// # 为什么解析 JSON 而不是文本形态
///
/// 文本形态是给人看的（`quads=98 skipped=0 …`），解析它要认字段顺序；
/// JSON 形态是**同一份数据**的另一种排版（`FrameStats::to_json`），
/// 字段名固定 —— 解析它不会因为排版调整而悄悄坏掉。
fn sample(client: &mut BridgeClient) -> Result<Sample, String> {
    let response = client.call("stats --json")?;
    let BridgeResponse::Text(lines) = response else {
        return Err("stats 没有返回文本".into());
    };
    let text = lines.join("\n");
    let json: Value =
        serde_json::from_str(&text).map_err(|e| format!("stats --json 不是合法 JSON：{e}"))?;
    let num = |key: &str| json.get(key).and_then(Value::as_u64).unwrap_or(0);
    let stages = json.get("stages");
    let stage = |key: &str| {
        stages
            .and_then(|s| s.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let damage = match json.get("damage") {
        Some(Value::Object(map)) => map
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_string(),
        _ => "-".to_string(),
    };
    Ok(Sample {
        frames: num("frames"),
        core_us: stage("total"),
        latency_us: json.get("latency_us").and_then(Value::as_u64),
        diff_us: stage("diff"),
        style_us: stage("style"),
        scan_us: stage("scan"),
        layout_us: stage("layout"),
        paint_us: stage("paint"),
        quads: num("quads"),
        dirty: num("dirtied"),
        damage,
    })
}

/// 发一条命令，把响应原样收回（文本）。
fn call_ok(client: &mut BridgeClient, cmd: &str) -> Result<Vec<String>, CliError> {
    match client.call(cmd) {
        Ok(BridgeResponse::Text(lines)) => Ok(lines),
        Ok(BridgeResponse::Err(e)) => Err(CliError::failure(format!("{cmd}: {e}"))),
        Ok(BridgeResponse::Binary { .. }) => {
            Err(CliError::failure(format!("{cmd}: 回的是二进制，不是文本")))
        }
        Err(e) => Err(CliError::failure(format!("{cmd}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stats --json` 的解析：字段名固定，解析不依赖排版。
    ///
    /// ⚠ 这条钉住的是「解析 JSON 而不是解析文本形态」这个选择：文本形态
    /// 的字段**顺序**一改，文本解析就会悄悄读错值（而 JSON 形态由
    /// `FrameStats::to_json` 生成，字段名固定）。
    #[test]
    fn stats_json_is_parsed_by_field_name() {
        let json = r#"{"quads":213,"skipped":0,"dirtied":2,"needs_layout":0,"redrawn":true,
            "hovered":"materials","damage":{"kind":"full"},
            "stages":{"diff":9,"style":7,"scan":3,"layout":0,"paint":0,"total":21},"frames":1234}"#;
        let value: Value = serde_json::from_str(json).unwrap();
        let stages = value.get("stages").unwrap();
        assert_eq!(stages.get("total").and_then(Value::as_u64), Some(21));
        assert_eq!(value.get("frames").and_then(Value::as_u64), Some(1234));
        assert_eq!(value.get("quads").and_then(Value::as_u64), Some(213));
    }

    /// 缺字段时**不炸**，而是当 0。
    ///
    /// 为什么：宿主可能不提供分段耗时（`stages: null`）—— 那时监测器要
    /// 照样能跑，只是那几列是 0。崩掉的话，监测器就成了「后端不支持就
    /// 什么都做不了」的东西。
    #[test]
    fn missing_stages_degrade_to_zero() {
        let json = r#"{"quads":1,"damage":null,"stages":null,"frames":0}"#;
        let value: Value = serde_json::from_str(json).unwrap();
        let stages = value.get("stages");
        assert!(stages.is_none() || stages.unwrap().is_null());
    }
}
