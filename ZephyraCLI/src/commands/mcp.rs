//! `zephyra mcp`：**Model Context Protocol** server —— AI 客户端经 stdio 上的
//! JSON-RPC 读布局、操控运行中的 ZephyraUI 应用。
//!
//! # 链路与职责
//!
//! ```text
//! AI 客户端 ──stdio·JSON-RPC── 本进程 ──TCP·行协议── 运行中的应用（zephyra-fx::bridge）
//! ```
//!
//! 本进程只做**翻译**：MCP 工具调用 ↔ 桥的行协议命令。它不理解布局树的内容
//! （树 dump 是桥那边生成的文本，原样转发），也不持有任何 UI 状态 —— 应用一关，
//! 工具调用返回连接错误，仅此而已。
//!
//! # 为什么应用侧是行协议而这里是 JSON
//!
//! 库层保持零依赖（见 `zephyra-fx::bridge` 的模块文档）；JSON 的解析责任收敛在
//! 本 crate——它反正要为 MCP 持有 `serde_json`。三层里只有这一层需要懂 JSON。
//!
//! # stdio 纪律
//!
//! MCP 的 stdio 传输是**换行分隔的 JSON**：stdout 上只能出现协议消息，
//! 任何日志/诊断必须走 stderr。违反这一条的代价是 AI 客户端直接解析失败——
//! 所以本文件的 `eprintln!` 一个都不能改到 stdout 上。

use serde_json::{json, Value};

use super::{CliError, Write};
use crate::{args::Args, ExitCode};

use std::io::{BufRead, Write as _};

/// 桥客户端：一条到运行中应用的 TCP 连接 + 协议响应切分。
struct BridgeClient {
    stream: std::net::TcpStream,
    /// 半行/跨读缓冲——与桥那边对称：响应按「读到终止符」切分。
    buf: Vec<u8>,
}

/// 一条桥响应（与 `zephyra_fx::bridge::Response` 同构，但 CLI 侧只看文本形状）。
enum BridgeResponse {
    /// 文本正文（不含 `OK` 终止行）。
    Text(Vec<String>),
    /// 二进制载荷（BMP）。头行的 note（`bmp 24bpp`）在切分时已被消费——
    /// 它只描述格式，CLI 侧没有消费者；留解析是为了校验头行形状。
    Binary {
        #[allow(dead_code, reason = "切分时消费，无下游读者")]
        note: String,
        data: Vec<u8>,
    },
    /// 桥明确回的错误。
    Err(String),
}

impl BridgeClient {
    /// 连接应用调试桥。`tries` 次、每次间隔 500ms——应用刚启动时端口可能
    /// 还没监听（AI 客户端往往先配好 MCP 再起应用）。
    fn connect(host: &str, port: u16, tries: usize) -> std::io::Result<Self> {
        let addr = (host, port);
        let mut last = None;
        for _ in 0..tries {
            match std::net::TcpStream::connect(addr) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    return Ok(Self {
                        stream: s,
                        buf: Vec::new(),
                    });
                }
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("connect failed")))
    }

    /// 发一条命令并读一条响应。
    fn call(&mut self, cmd: &str) -> Result<BridgeResponse, String> {
        if self.stream.write_all(cmd.as_bytes()).is_err() || self.stream.write_all(b"\n").is_err() {
            return Err("桥连接已断开".into());
        }
        self.stream.flush().ok();
        self.read_response()
    }

    /// 从流里读出**一条**完整响应（读到 `OK` 行 / `ERR` 行 / `BIN` 载荷完）。
    ///
    /// ⚠ 这份切分逻辑与 `zephyra-fx::bridge` 的单测是**同一协议的两个读者**：
    /// 桥怎么写，这里就怎么读。二进制载荷按长度分块读取，不能逐字节转 UTF-8。
    fn read_response(&mut self) -> Result<BridgeResponse, String> {
        use std::io::Read;
        let mut chunk = [0u8; 8192];
        loop {
            if self.buf.starts_with(b"BIN ") {
                if let Some(header_end) = self.buf.iter().position(|&b| b == b'\n') {
                    let header = std::str::from_utf8(&self.buf[..header_end])
                        .map_err(|e| format!("桥二进制头不是 UTF-8: {e}"))?;
                    let (length, note) = header[4..]
                        .split_once(' ')
                        .ok_or_else(|| format!("桥二进制头格式错误: {header}"))?;
                    let length = length
                        .parse::<usize>()
                        .map_err(|e| format!("桥二进制长度无效: {e}"))?;
                    let note = note.to_string();
                    let payload_start = header_end + 1;
                    let response_end = payload_start
                        .checked_add(length)
                        .and_then(|n| n.checked_add(b"\nOK\n".len()))
                        .ok_or_else(|| "桥二进制长度溢出".to_string())?;
                    while self.buf.len() < response_end {
                        let n = self
                            .stream
                            .read(&mut chunk)
                            .map_err(|e| format!("读取桥二进制响应失败: {e}"))?;
                        if n == 0 {
                            return Err("桥连接在二进制响应中断开".into());
                        }
                        self.buf.extend_from_slice(&chunk[..n]);
                    }
                    let payload_end = payload_start + length;
                    if &self.buf[payload_end..response_end] != b"\nOK\n" {
                        return Err("桥二进制响应缺少 OK 终止符".into());
                    }
                    let data = self.buf[payload_start..payload_end].to_vec();
                    self.buf.drain(..response_end);
                    return Ok(BridgeResponse::Binary { note, data });
                }
            }
            if self.buf.starts_with(b"ERR ") {
                if let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
                    let line = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                    self.buf.drain(..=end);
                    return Ok(BridgeResponse::Err(line));
                }
            }
            let terminator = self
                .buf
                .windows(b"\nOK\n".len())
                .position(|window| window == b"\nOK\n")
                .map(|index| (index, index + b"\nOK\n".len()))
                .or_else(|| self.buf.starts_with(b"OK\n").then_some((0, 3)));
            if let Some((body_end, response_end)) = terminator {
                let body = String::from_utf8_lossy(&self.buf[..body_end]).into_owned();
                let lines = if body.is_empty() {
                    Vec::new()
                } else {
                    body.lines().map(str::to_string).collect()
                };
                self.buf.drain(..response_end);
                return Ok(BridgeResponse::Text(lines));
            }

            let n = self
                .stream
                .read(&mut chunk)
                .map_err(|e| format!("读取桥响应失败: {e}"))?;
            if n == 0 {
                return Err("桥连接已断开".into());
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

/// 一个 MCP 工具的定义（`tools/list` 的数据源 + `tools/call` 的执行）。
struct Tool {
    name: &'static str,
    description: &'static str,
    /// JSON Schema（`inputSchema`）。存**静态文本**：`json!` 宏展开成
    /// 非常量调用，进不了 `static TOOLS`；运行时 parse 一次，成本可忽略。
    schema: &'static str,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "get_layout",
        description: "Return the current UI layout as plain text for model context: \
physical-pixel viewport, last-frame stats, and each node's child-index path, tag, key, \
absolute bounds, and style classes. Coordinates use the same space as hover/click. \
Use get_props for a node's full properties; use screenshot only when visual pixels are needed.",
        schema: r#"{"type": "object", "properties": {}}"#,
    },
    Tool {
        name: "get_tree",
        description: "Dump the layout tree of the running ZephyraUI app. Each line: \
`path tag key x,y w*h [styles]` — path (child-index chain like `0.1.2`) is the key for \
`get_props`; coordinates are ABSOLUTE pixels and are the same coordinate system that \
`hover`/`click` use, so you can copy a coordinate from the tree and click it directly.",
        schema: r#"{"type": "object", "properties": {}}"#,
    },
    Tool {
        name: "get_props",
        description: "Dump the properties of one node. `path` is the child-index chain \
from `get_tree` (e.g. `0.1.2`). Returns tag / key / rect (relative to parent) / styles \
/ every property (`name = value (styled)` lines are overridden by later local values).",
        schema: r#"{"type": "object", "properties": {"path": {"type": "string", "description": "child-index chain, e.g. 0.1.2"}}, "required": ["path"]}"#,
    },
    Tool {
        name: "get_stats",
        description: "Stats of the last rendered frame: quad count, dirty flags, hovered node.",
        schema: r#"{"type": "object", "properties": {}}"#,
    },
    Tool {
        name: "get_resources",
        description: "Read process resource telemetry: CPU utilization as a percentage of total \
machine capacity, working/private memory, and Windows GPU engine utilization plus dedicated/shared GPU memory when the driver exposes them. \
The first sample may report warming_up while interval counters collect a baseline.",
        schema: r#"{"type": "object", "properties": {}}"#,
    },
    Tool {
        name: "get_viewport",
        description: "Viewport size of the app window in physical pixels (`width height`).",
        schema: r#"{"type": "object", "properties": {}}"#,
    },
    Tool {
        name: "hover",
        description: "Move the pointer to (x, y) — same as a physical mouse move; hover \
states and hit-testing see the synthetic event exactly like a real one. Takes effect \
this frame; read the tree afterwards to see what changed.",
        schema: r#"{"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}}, "required": ["x", "y"]}"#,
    },
    Tool {
        name: "click",
        description: "Synthesize a full click (move → down → up) at (x, y). Pick the \
coordinate from `get_tree` output. Takes effect this frame.",
        schema: r#"{"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}}, "required": ["x", "y"]}"#,
    },
    Tool {
        name: "press_key",
        description: "Synthesize one logical key press and release (for example Enter, \
ArrowDown, Escape, or a character). The running View receives both key-edge events.",
        schema: r#"{"type": "object", "properties": {"key": {"type": "string", "description": "One logical key name; spaces and control characters are not allowed"}}, "required": ["key"]}"#,
    },
    Tool {
        name: "scroll",
        description: "Synthesize a wheel event at (x,y) in physical pixels. delta_y > 0 \
means scroll up; delta_y < 0 means scroll down. The View receives the position and \
wheel deltas (native events preserve the platform's pixel/line units).",
        schema: r#"{"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "delta_x": {"type": "number"}, "delta_y": {"type": "number"}}, "required": ["x", "y", "delta_x", "delta_y"]}"#,
    },
    Tool {
        name: "pointer_drag",
        description: "Press and hold the primary pointer button while moving, then release. \
Modes: segmented follows a polyline in `points`; linear moves from `start` to `end`; easing \
uses `start`/`end` and `easing` (linear, ease_in, ease_out, smoothstep). Sampling occurs \
across `duration_ms`; optional hold_before_ms/hold_after_ms keep the button down. \
Coordinates are physical pixels. Sampling and total duration are bounded.",
        schema: r#"{"type":"object","properties":{"mode":{"type":"string","enum":["segmented","linear","easing"]},"points":{"type":"array","minItems":2,"maxItems":128,"items":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"]}},"start":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"]},"end":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"]},"easing":{"type":"string","enum":["linear","ease_in","ease_out","smoothstep"]},"duration_ms":{"type":"integer","minimum":1,"maximum":60000},"sample_interval_ms":{"type":"integer","minimum":1,"maximum":1000,"default":16},"hold_before_ms":{"type":"integer","minimum":0,"maximum":10000,"default":0},"hold_after_ms":{"type":"integer","minimum":0,"maximum":10000,"default":0}},"required":["mode","duration_ms"]}"#,
    },
    Tool {
        name: "analyze_performance",
        description: "Collect a bounded CPU frame-performance profile and return it when the \
requested capture duration ends. Records per-frame update/layout, render-submit and total CPU \
generation time, with min/average/median/p95/max plus frame-by-frame rows. Sampling is opt-in; \
duration_seconds is required and limited to 1..=300 seconds, with automatic stop at 3000 frames.",
        schema: r#"{"type":"object","properties":{"duration_seconds":{"type":"integer","minimum":1,"maximum":300,"description":"Required bounded capture duration; automatically stops at expiry or 3000 frames."}},"required":["duration_seconds"]}"#,
    },
    Tool {
        name: "screenshot",
        description: "Render a frame and read back the pixels as a 24bpp BMP. D3D11 and \
D3D12 apps support this. Returns the file path that was written; use get_layout for \
textual layout context without pixel data.",
        schema: r#"{"type": "object", "properties": {"path": {"type": "string", "description": "where to write the .bmp; defaults to a temp file"}}}"#,
    },
];

pub fn run(args: &Args, io: &mut dyn Write) -> Result<ExitCode, CliError> {
    let port: u16 = args
        .option("port")
        .map(|p| p.parse())
        .transpose()
        .map_err(|_| CliError::usage("--port needs a number"))?
        .unwrap_or(zephyra_bridge_port());
    let host = args.option("host").unwrap_or("127.0.0.1").to_string();

    // 懒连接：MCP 握手（initialize / tools/list）**不需要**应用在线——
    // AI 客户端往往先拉起 MCP 再起应用，握手被卡住会让整条配置看起来是坏的。
    let mut client: Option<BridgeClient> = None;

    io.err_line(&format!(
        "[zephyra-mcp] ready (bridge {}:{port}); stdout is protocol-only",
        host
    ));

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break }; // EOF → 正常退出
        if line.trim().is_empty() {
            continue;
        }
        let reply = handle_message(&line, &host, port, &mut client);
        if let Some(out) = reply {
            // stdout 只准出现协议消息（见模块文档的 stdio 纪律）。
            println!("{out}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    }
    Ok(ExitCode::Success)
}

/// 应用侧桥的默认端口。CLI 不依赖 zephyra-fx，常量这里**独立**写——
/// 但它必须与 `zephyra_fx::bridge::DEFAULT_PORT` 一致（测试钉住这一点）。
fn zephyra_bridge_port() -> u16 {
    7878
}

/// 处理一条 JSON-RPC 消息。返回要写回 stdout 的 JSON 行；通知类返回 `None`。
///
/// 拆成纯函数（stdin/网络都走参数）是为了能测：MCP 消息的形状与分派逻辑
/// 不必起真进程就能钉住。
fn handle_message(
    line: &str,
    host: &str,
    port: u16,
    client: &mut Option<BridgeClient>,
) -> Option<String> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(error_reply(
                Value::Null,
                -32700,
                &format!("parse error: {e}"),
            ))
        }
    };
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let is_notification = msg.get("id").is_none();

    match method {
        // 通知不回（回了会污染 AI 客户端的消息流）。
        "notifications/initialized" | "notifications/cancelled" => None,
        "initialize" => Some(json_reply(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "zephyra", "version": crate::VERSION}
            }),
        )),
        "tools/list" => {
            let tools: Vec<Value> = TOOLS
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": serde_json::from_str::<Value>(t.schema)
                            .expect("内置工具的 schema 必须是合法 JSON"),
                    })
                })
                .collect();
            Some(json_reply(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tool_args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            let text = match run_tool(name, &tool_args, host, port, client) {
                Ok(t) => t,
                Err(e) => {
                    return Some(json_reply(
                        id,
                        json!({"content": [{"type": "text", "text": e}], "isError": true}),
                    ))
                }
            };
            Some(json_reply(
                id,
                json!({"content": [{"type": "text", "text": text}]}),
            ))
        }
        other => {
            if is_notification {
                None
            } else {
                Some(error_reply(
                    id,
                    -32601,
                    &format!("unknown method `{other}`"),
                ))
            }
        }
    }
}

/// 拿到一条可用的桥连接：没有就连（重试 3 次），半死就重连。
///
/// 唯一的连接管理者——`run` 的主循环只调 [`handle_message`]，连接的生灭
/// 全部收敛在这里，不会出现「两条路径各养一个连接」。
fn ensure_client<'a>(
    host: &str,
    port: u16,
    client: &'a mut Option<BridgeClient>,
) -> Result<&'a mut BridgeClient, String> {
    if client.is_none() {
        *client = Some(BridgeClient::connect(host, port, 3).map_err(|e| {
            format!(
                "连不上应用调试桥 {host}:{port}（{e}）——确认应用以 ZEPHYRA_DEBUG_PORT={port} 启动"
            )
        })?);
    }
    Ok(client.as_mut().unwrap())
}

/// 执行一个工具调用。返回给 AI 的文本。
fn run_tool(
    name: &str,
    a: &Value,
    host: &str,
    port: u16,
    client: &mut Option<BridgeClient>,
) -> Result<String, String> {
    // 连接失败重连一次再试——桥断开最常见于应用重启。
    macro_rules! bridge {
        () => {
            ensure_client(host, port, client)?
        };
    }
    let text_of = |r: BridgeResponse| -> Result<String, String> {
        match r {
            BridgeResponse::Text(lines) => Ok(if lines.is_empty() {
                "(empty)".into()
            } else {
                lines.join("\n")
            }),
            BridgeResponse::Err(e) => Err(e),
            BridgeResponse::Binary { .. } => Err("意外的二进制响应".into()),
        }
    };
    match name {
        "get_tree" => text_of(bridge!().call("tree")?),
        "get_layout" => text_of(bridge!().call("layout")?),
        "get_stats" => text_of(bridge!().call("stats")?),
        "get_resources" => text_of(bridge!().call("resources")?),
        "get_viewport" => text_of(bridge!().call("viewport")?),
        "get_props" => {
            let path = a
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("get_props 需要 path 参数（来自 get_tree 的路径列）")?;
            text_of(bridge!().call(&format!("props {path}"))?)
        }
        "hover" => {
            let (x, y) = xy(a)?;
            text_of(bridge!().call(&format!("hover {x} {y}"))?)
        }
        "click" => {
            let (x, y) = xy(a)?;
            text_of(bridge!().call(&format!("click {x} {y}"))?)
        }
        "press_key" => {
            let key = a
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("press_key 需要 key 参数")?;
            if key.is_empty()
                || key.chars().any(char::is_whitespace)
                || key.chars().any(char::is_control)
            {
                return Err("key 必须是单个逻辑键名，不能包含空白或控制字符".into());
            }
            text_of(bridge!().call(&format!("press_key {key}"))?)
        }
        "scroll" => {
            let (x, y) = xy(a)?;
            let (delta_x, delta_y) = deltas(a)?;
            text_of(bridge!().call(&format!("scroll {x} {y} {delta_x} {delta_y}"))?)
        }
        "pointer_drag" => {
            let plan = drag_samples(a)?;
            perform_pointer_drag(
                host,
                port,
                client,
                plan.start,
                &plan.samples,
                plan.hold_before_ms,
                plan.hold_after_ms,
            )?;
            Ok(format!(
                "pointer drag completed ({} movement samples)",
                plan.samples.len()
            ))
        }
        "analyze_performance" => {
            let duration_seconds = profile_duration_seconds(a)?;
            text_of(bridge!().call(&format!("profile_start {duration_seconds}"))?)?;
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(duration_seconds + 10);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let report = text_of(bridge!().call("profile_report")?)?;
                if report.starts_with("PROFILE_STATE complete") {
                    return Ok(report);
                }
                if std::time::Instant::now() >= deadline {
                    return Err("性能采样未在声明时限内完成；可能宿主帧循环已停止".into());
                }
            }
        }
        "screenshot" => {
            let r = bridge!().call("shot")?;
            match r {
                BridgeResponse::Binary { data, .. } => {
                    if data.is_empty() {
                        return Err("后端读回为空".into());
                    }
                    let path = a
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            std::env::temp_dir()
                                .join(format!("zephyra-shot-{}.bmp", std::process::id()))
                                .display()
                                .to_string()
                        });
                    std::fs::write(&path, &data).map_err(|e| format!("写 {path} 失败: {e}"))?;
                    Ok(format!(
                        "screenshot written to {path} ({} bytes, 24bpp BMP)",
                        data.len()
                    ))
                }
                BridgeResponse::Err(e) => Err(e),
                BridgeResponse::Text(_) => Err("桥回了文本而不是图像".into()),
            }
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

/// 从工具参数里取 (x, y)，接受 int / float。
fn xy(a: &Value) -> Result<(String, String), String> {
    let get = |k: &str| -> Result<String, String> {
        let v = a.get(k).ok_or_else(|| format!("需要 {k} 坐标"))?;
        if let Some(n) = v.as_f64() {
            if n.is_finite() {
                return Ok(format!("{n}"));
            }
            return Err(format!("{k} 必须是有限数"));
        }
        Err(format!("{k} 必须是数字"))
    };
    Ok((get("x")?, get("y")?))
}

fn deltas(a: &Value) -> Result<(String, String), String> {
    let get = |k: &str| -> Result<String, String> {
        let v = a.get(k).ok_or_else(|| format!("需要 {k} 滚轮增量"))?;
        let n = v.as_f64().ok_or_else(|| format!("{k} 必须是数字"))?;
        if !n.is_finite() {
            return Err(format!("{k} 必须是有限数"));
        }
        Ok(format!("{n}"))
    };
    Ok((get("delta_x")?, get("delta_y")?))
}

#[derive(Debug, Clone, Copy)]
struct DragSample {
    x: f64,
    y: f64,
    delay_ms: u64,
}

struct DragPlan {
    start: (f64, f64),
    samples: Vec<DragSample>,
    hold_before_ms: u64,
    hold_after_ms: u64,
}

fn drag_samples(a: &Value) -> Result<DragPlan, String> {
    const MAX_POINTS: usize = 128;
    const MAX_SAMPLES: usize = 512;
    const MAX_DURATION_MS: u64 = 60_000;
    let mode = a
        .get("mode")
        .and_then(Value::as_str)
        .ok_or("pointer_drag 需要 mode")?;
    let duration_ms = a
        .get("duration_ms")
        .and_then(Value::as_u64)
        .filter(|value| (1..=MAX_DURATION_MS).contains(value))
        .ok_or("duration_ms 必须是 1..=60000 的整数")?;
    let sample_interval_ms = optional_u64(a, "sample_interval_ms", 16, 1, 1000)?;
    let hold_before_ms = optional_u64(a, "hold_before_ms", 0, 0, 10_000)?;
    let hold_after_ms = optional_u64(a, "hold_after_ms", 0, 0, 10_000)?;
    if duration_ms + hold_before_ms + hold_after_ms > MAX_DURATION_MS {
        return Err("拖动（含按下/抬起前停留）的总时长不能超过 60000 毫秒".into());
    }

    let points = match mode {
        "segmented" => {
            let values = a
                .get("points")
                .and_then(Value::as_array)
                .ok_or("segmented 模式需要 points 数组")?;
            if !(2..=MAX_POINTS).contains(&values.len()) {
                return Err(format!(
                    "segmented 模式的 points 必须包含 2..={MAX_POINTS} 个坐标"
                ));
            }
            values
                .iter()
                .enumerate()
                .map(|(index, value)| read_point(value, &format!("points[{index}]")))
                .collect::<Result<Vec<_>, _>>()?
        }
        "linear" | "easing" => {
            let start = read_point(a.get("start").ok_or("此模式需要 start 坐标")?, "start")?;
            let end = read_point(a.get("end").ok_or("此模式需要 end 坐标")?, "end")?;
            vec![start, end]
        }
        _ => return Err("mode 必须是 segmented、linear 或 easing".into()),
    };
    let easing = if mode == "easing" {
        match a.get("easing").and_then(Value::as_str).unwrap_or("linear") {
            "linear" => Easing::Linear,
            "ease_in" => Easing::EaseIn,
            "ease_out" => Easing::EaseOut,
            "smoothstep" => Easing::Smoothstep,
            _ => return Err("easing 必须是 linear、ease_in、ease_out 或 smoothstep".into()),
        }
    } else {
        Easing::Linear
    };

    let segment_count = points.len() - 1;
    let mut samples = Vec::new();
    let mut duration_left = duration_ms;
    let mut segments_left = segment_count as u64;
    for pair in points.windows(2) {
        let segment_duration = duration_left.div_ceil(segments_left);
        duration_left -= segment_duration;
        segments_left -= 1;
        let steps = segment_duration.div_ceil(sample_interval_ms).max(1) as usize;
        if samples.len().saturating_add(steps) > MAX_SAMPLES {
            return Err(format!(
                "采样点超过 {MAX_SAMPLES}；请增大 sample_interval_ms 或缩短 duration_ms"
            ));
        }
        let mut elapsed = 0;
        for step in 1..=steps {
            let next_elapsed = segment_duration * step as u64 / steps as u64;
            let t = step as f64 / steps as f64;
            let t = easing.apply(t);
            samples.push(DragSample {
                x: pair[0].0 + (pair[1].0 - pair[0].0) * t,
                y: pair[0].1 + (pair[1].1 - pair[0].1) * t,
                delay_ms: next_elapsed - elapsed,
            });
            elapsed = next_elapsed;
        }
    }
    Ok(DragPlan {
        start: points[0],
        samples,
        hold_before_ms,
        hold_after_ms,
    })
}

#[derive(Clone, Copy)]
enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    Smoothstep,
}

impl Easing {
    fn apply(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Self::Smoothstep => t * t * (3.0 - 2.0 * t),
        }
    }
}

fn read_point(value: &Value, name: &str) -> Result<(f64, f64), String> {
    let x = value
        .get("x")
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{name}.x 必须是数字"))?;
    let y = value
        .get("y")
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{name}.y 必须是数字"))?;
    if !(x.is_finite() && y.is_finite() && x.abs() <= f32::MAX as f64 && y.abs() <= f32::MAX as f64)
    {
        return Err(format!("{name} 坐标必须是可表示为有限 f32 的数"));
    }
    Ok((x, y))
}

fn optional_u64(
    value: &Value,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    let value = value.get(name).map_or(Ok(default), |value| {
        value.as_u64().ok_or_else(|| format!("{name} 必须是整数"))
    })?;
    if !(min..=max).contains(&value) {
        return Err(format!("{name} 必须在 {min}..={max} 范围内"));
    }
    Ok(value)
}

fn profile_duration_seconds(arguments: &Value) -> Result<u64, String> {
    arguments
        .get("duration_seconds")
        .and_then(Value::as_u64)
        .filter(|value| (1..=300).contains(value))
        .ok_or_else(|| "duration_seconds 必须是 1..=300 的整数".into())
}

fn perform_pointer_drag(
    host: &str,
    port: u16,
    client: &mut Option<BridgeClient>,
    start: (f64, f64),
    samples: &[DragSample],
    hold_before_ms: u64,
    hold_after_ms: u64,
) -> Result<(), String> {
    let send = |client: &mut Option<BridgeClient>, command: &str| -> Result<(), String> {
        let response = ensure_client(host, port, client)?.call(command)?;
        match response {
            BridgeResponse::Text(_) => Ok(()),
            BridgeResponse::Err(error) => Err(error),
            BridgeResponse::Binary { .. } => Err("拖动命令收到意外二进制响应".into()),
        }
    };
    send(client, &format!("hover {} {}", start.0, start.1))?;

    let movement = (|| {
        send(client, "pointer_down")?;
        if hold_before_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hold_before_ms));
        }
        for sample in samples {
            send(client, &format!("hover {} {}", sample.x, sample.y))?;
            if sample.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(sample.delay_ms));
            }
        }
        if hold_after_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hold_after_ms));
        }
        Ok(())
    })();
    let release = send(client, "pointer_up");
    match (movement, release) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("拖动完成但抬起注入失败：{error}")),
        (Err(error), Err(release_error)) => Err(format!(
            "{error}；清理时补发 pointer_up 也失败：{release_error}"
        )),
    }
}

fn json_reply(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn error_reply(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 起一个只服务**一条命令**的假桥（真 TCP，行协议同款应答）。
    ///
    /// 返回（端口, 已收到的命令接收端）。`handle_message` 的连接管理是真
    /// 网络路径，用纯函数假对象测不到「懒连接 / 重连」这些行为——所以这里
    /// 宁可起线程。响应固定为 `resp + OK`。
    fn mock_bridge(resp: &'static str) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                s.read_exact(&mut byte).unwrap();
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            let _ = tx.send(String::from_utf8(buf).unwrap());
            s.write_all(resp.as_bytes()).unwrap();
            s.write_all(b"\nOK\n").unwrap();
        });
        (port, rx)
    }

    fn mock_bridge_with_responses(
        responses: &'static [&'static str],
    ) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let reader = stream.try_clone().unwrap();
            let mut reader = BufReader::new(reader);
            for response in responses {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                tx.send(line.trim_end().to_string()).unwrap();
                if response.starts_with("ERR ") {
                    writeln!(stream, "{response}").unwrap();
                } else {
                    writeln!(stream, "{response}\nOK").unwrap();
                }
            }
        });
        (port, rx)
    }

    fn mock_bridge_sequence(command_count: usize) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let reader = stream.try_clone().unwrap();
            for _ in 0..command_count {
                let mut line = String::new();
                BufReader::new(&reader).read_line(&mut line).unwrap();
                tx.send(line.trim_end().to_string()).unwrap();
                stream.write_all(b"clicked\nOK\n").unwrap();
            }
        });
        (port, rx)
    }

    #[test]
    fn initialize_announces_tools_capability() {
        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "127.0.0.1",
            7878,
            &mut client,
        )
        .unwrap();
        assert!(r.contains("\"protocolVersion\""), "{r}");
        assert!(r.contains("serverInfo"), "{r}");
        // 握手不碰桥——AI 客户端先配 MCP 再起应用是常态。
        assert!(client.is_none());
    }

    #[test]
    fn notifications_produce_no_output() {
        let mut client = None;
        assert!(handle_message(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "127.0.0.1",
            7878,
            &mut client,
        )
        .is_none());
    }

    #[test]
    fn tools_list_contains_every_tool() {
        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "127.0.0.1",
            7878,
            &mut client,
        )
        .unwrap();
        for t in [
            "get_tree",
            "get_layout",
            "get_props",
            "get_stats",
            "get_resources",
            "get_viewport",
            "hover",
            "click",
            "press_key",
            "scroll",
            "pointer_drag",
            "analyze_performance",
            "screenshot",
        ] {
            assert!(r.contains(t), "tools/list 缺 {t}：{r}");
        }
        assert!(client.is_none(), "tools/list 也不该碰桥");
    }

    #[test]
    fn tool_call_forwards_to_bridge_command() {
        let (port, seen) = mock_bridge("0 Container root 0,0 800x600");
        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_tree","arguments":{}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "tree");
        assert!(r.contains("0 Container root"), "{r}");
        assert!(!r.contains("\"isError\":true"), "{r}");
        assert!(client.is_some(), "工具调用后连接应当被缓存复用");
    }

    #[test]
    fn resources_tool_forwards_to_bridge() {
        let (port, seen) = mock_bridge("cpu_process_percent=warming_up");
        let mut client = None;
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"get_resources","arguments":{}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "resources");
        assert!(
            response.contains("cpu_process_percent=warming_up"),
            "{response}"
        );
    }

    #[test]
    fn click_translates_coordinates() {
        let (port, seen) = mock_bridge("clicked");
        let mut client = None;
        let _ = handle_message(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"click","arguments":{"x":12.5,"y":48}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "click 12.5 48");
    }

    #[test]
    fn layout_tool_requests_plain_text_layout_context() {
        let (port, seen) = mock_bridge("viewport 800x600 physical-pixels\nstats quads=2\nlayout-tree path tag key absolute-x,absolute-y widthxheight [classes]\n0 Container root 0,0 800x600");
        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_layout","arguments":{}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "layout");
        assert!(
            r.contains("physical-pixels") && r.contains("layout-tree"),
            "{r}"
        );
        assert!(r.contains("\"type\":\"text\""), "{r}");
    }

    #[test]
    fn key_and_scroll_tools_translate_and_reject_protocol_injection() {
        let (port, seen) = mock_bridge("pressed Enter");
        let mut client = None;
        handle_message(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"Enter"}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "press_key Enter");

        let (port, seen) = mock_bridge("scrolled");
        let mut client = None;
        handle_message(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"scroll","arguments":{"x":12,"y":30,"delta_x":0,"delta_y":-2.5}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "scroll 12 30 0 -2.5");

        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"Enter\nshot"}}}"#,
            "127.0.0.1",
            7878,
            &mut client,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(client.is_none(), "注入内容应在连接桥之前拒绝");
    }

    #[test]
    fn pointer_drag_sends_down_moves_and_release() {
        let (port, seen) = mock_bridge_sequence(4);
        let mut client = None;
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"pointer_drag","arguments":{"mode":"linear","start":{"x":1,"y":2},"end":{"x":7,"y":9},"duration_ms":1,"sample_interval_ms":1000}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert!(response.contains("pointer drag completed"), "{response}");
        assert_eq!(
            seen.try_iter().collect::<Vec<_>>(),
            ["hover 1 2", "pointer_down", "hover 7 9", "pointer_up"]
        );

        let mut client = None;
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"pointer_drag","arguments":{"mode":"segmented","points":[{"x":1,"y":2}],"duration_ms":10}}}"#,
            "127.0.0.1",
            7878,
            &mut client,
        )
        .unwrap();
        assert!(response.contains("\"isError\":true"), "{response}");
        assert!(client.is_none(), "无效路径必须在建立桥连接前拒绝");
    }

    #[test]
    fn pointer_drag_attempts_release_after_a_move_error() {
        let (port, seen) = mock_bridge_with_responses(&[
            "hovered",
            "pointer_down",
            "ERR simulated move failure",
            "pointer_up",
        ]);
        let mut client = None;
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"pointer_drag","arguments":{"mode":"linear","start":{"x":1,"y":2},"end":{"x":7,"y":9},"duration_ms":1,"sample_interval_ms":1000}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert!(response.contains("\"isError\":true"), "{response}");
        assert_eq!(
            seen.try_iter().collect::<Vec<_>>(),
            ["hover 1 2", "pointer_down", "hover 7 9", "pointer_up"]
        );
    }

    #[test]
    fn pointer_drag_builds_segmented_and_eased_motion() {
        let segmented = drag_samples(&json!({
            "mode":"segmented",
            "points":[{"x":0,"y":0},{"x":10,"y":0},{"x":10,"y":10}],
            "duration_ms":100,
            "sample_interval_ms":100
        }))
        .unwrap()
        .samples;
        assert_eq!(segmented.len(), 2);
        assert_eq!((segmented[0].x, segmented[0].y), (10.0, 0.0));
        assert_eq!((segmented[1].x, segmented[1].y), (10.0, 10.0));

        let eased = drag_samples(&json!({
            "mode":"easing",
            "start":{"x":0,"y":0},
            "end":{"x":10,"y":0},
            "duration_ms":100,
            "sample_interval_ms":25,
            "easing":"smoothstep"
        }))
        .unwrap()
        .samples;
        assert_eq!(eased.len(), 4);
        assert!(eased[0].x < 2.5);
        assert_eq!(eased[3].x, 10.0);
    }

    #[test]
    fn performance_tool_waits_for_complete_report() {
        let (port, seen) = mock_bridge_with_responses(&[
            "performance profiling started for 1s",
            "PROFILE_STATE running remaining_ms=50",
            "PROFILE_STATE complete\nframes=2\nframe_index total_us update_layout_us render_submit_us\n1 100 60 40\n2 110 65 45",
        ]);
        let mut client = None;
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"analyze_performance","arguments":{"duration_seconds":1}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert!(response.contains("PROFILE_STATE complete"), "{response}");
        assert!(response.contains("2 110 65 45"), "{response}");
        assert_eq!(
            seen.try_iter().collect::<Vec<_>>(),
            ["profile_start 1", "profile_report", "profile_report"]
        );
    }

    #[test]
    fn performance_capture_requires_a_bounded_duration() {
        assert_eq!(
            profile_duration_seconds(&json!({"duration_seconds": 30})).unwrap(),
            30
        );
        assert!(profile_duration_seconds(&json!({})).is_err());
        assert!(profile_duration_seconds(&json!({"duration_seconds": 301})).is_err());
    }

    #[test]
    fn binary_bridge_response_reads_large_payload_by_declared_length() {
        use std::io::Write;

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let mut payload = vec![0x5a; 2 * 1024 * 1024];
        payload[100..104].copy_from_slice(b"\nOK\n");
        let sent = payload.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            writeln!(stream, "BIN {} bmp 24bpp", sent.len()).unwrap();
            stream.write_all(&sent).unwrap();
            stream.write_all(b"\nOK\n").unwrap();
        });

        let mut client = BridgeClient {
            stream: std::net::TcpStream::connect(address).unwrap(),
            buf: Vec::new(),
        };
        match client.read_response().unwrap() {
            BridgeResponse::Binary { note, data } => {
                assert_eq!(note, "bmp 24bpp");
                assert_eq!(data, payload);
            }
            _ => panic!("large screenshot response must be parsed as binary"),
        }
        server.join().unwrap();
    }

    #[test]
    fn bridge_error_becomes_is_error_result() {
        let (port, _seen) = mock_bridge("ERR 路径 9.9 不存在");
        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_props","arguments":{"path":"9.9"}}}"#,
            "127.0.0.1",
            port,
            &mut client,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(r.contains("不存在"), "{r}");
    }

    #[test]
    fn bad_json_is_a_parse_error_not_a_crash() {
        let mut client = None;
        let r = handle_message("not json", "127.0.0.1", 7878, &mut client).unwrap();
        assert!(r.contains("-32700"), "{r}");
    }

    #[test]
    fn unreachable_bridge_is_a_tool_error_not_a_panic() {
        // 端口 1（几乎必然无监听）+ 重试 3 次：工具调用以 isError 结束。
        let mut client = None;
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"get_tree","arguments":{}}}"#,
            "127.0.0.1",
            1,
            &mut client,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(r.contains("ZEPHYRA_DEBUG_PORT"), "错误要直接指出修法：{r}");
    }

    /// 协议常量对齐：CLI 的默认端口必须与库层桥一致。
    ///
    /// 为什么不直接 `use zephyra_fx::bridge::DEFAULT_PORT`：CLI 刻意不依赖
    /// `zephyra-fx`——一致性靠这条测试钉住，而不是靠一条依赖边。
    #[test]
    fn default_port_matches_bridge() {
        assert_eq!(zephyra_bridge_port(), 7878);
    }
}
