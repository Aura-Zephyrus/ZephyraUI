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
//!
//! # 并发模型：协议消息即时答，工具调用排队做
//!
//! stdin 线程只处理**不碰桥**的消息（`initialize` / `tools/list` / `ping` /
//! 通知）；`tools/call` 连同原始行投给唯一的工具工作线程，后者持有桥连接、
//! 按到达顺序执行。这样做的理由：`analyze_performance` 最长 300 秒、
//! `pointer_drag` 最长 60 秒——单线程版在这些调用期间对任何消息都不应答，
//! 而 MCP host 的默认工具超时往往只有 60 秒，超时后整个 server 看起来死了。
//!
//! 取消（`notifications/cancelled`）由跨线程共享的登记表传递：排队中的调用
//! 在开跑前查表跳过；在途的 `analyze_performance` 在每轮轮询间隙查表中止。
//! 按协议，被取消的请求**不回**。

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
                    // 读超时是「MCP server 不被冻结的应用拖死」的关键：没有它，
                    // 应用帧循环停摆（模态框 / 最小化 / 卡死）时 read 会永远阻塞，
                    // 本进程从此不应答任何消息，客户端只能杀进程重配。
                    // 10 秒远大于实测的最慢单次往返（get_resources 首次采样
                    // 约 1.2 秒、强制截图约 0.3 秒，gallery 1400×800 D3D12 真机）。
                    s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
                        .ok();
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
    /// `annotations`：只读/破坏性提示。AI 客户端据此判断哪些调用会改变
    /// 界面状态——没有它，模型只能靠猜工具名里的动词。
    annotations: &'static str,
}

/// 读工具：不改变应用状态。
const READ_ONLY: &str = r#"{"readOnlyHint": true, "destructiveHint": false}"#;
/// 输入/采集类工具：会改变界面状态或进程内采样状态，但不销毁数据。
const MUTATES: &str = r#"{"readOnlyHint": false, "destructiveHint": false}"#;

const TOOLS: &[Tool] = &[
    Tool {
        name: "get_layout",
        description: "Return the current UI layout as plain text for model context: \
physical-pixel viewport, last-frame stats, and each node's child-index path, tag, key, \
absolute bounds, and style classes. Coordinates use the same space as hover/click. \
Use get_props for a node's full properties; use screenshot only when visual pixels are needed.",
        schema: r#"{"type": "object", "properties": {}}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "get_tree",
        description: "Dump the layout tree of the running ZephyraUI app. Each line: \
`path tag key x,y w*h [styles]` — path (child-index chain like `0.1.2`) is the key for \
`get_props`; coordinates are ABSOLUTE pixels and are the same coordinate system that \
`hover`/`click` use, so you can copy a coordinate from the tree and click it directly.",
        schema: r#"{"type": "object", "properties": {}}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "get_props",
        description: "Dump the properties of one node. `path` is the child-index chain \
from `get_tree` (e.g. `0.1.2`, including the leading root marker `0`). Returns tag / key / \
rect (ABSOLUTE pixels, same space as hover/click) / styles / every property \
(`name = value (styled)` lines are overridden by later local values).",
        schema: r#"{"type": "object", "properties": {"path": {"type": "string", "description": "child-index chain, e.g. 0.1.2"}}, "required": ["path"]}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "get_stats",
        description: "Stats of the last rendered frame: quad count, dirty flags, hovered node.",
        schema: r#"{"type": "object", "properties": {}}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "get_resources",
        description: "Read process resource telemetry: CPU utilization as a percentage of total \
machine capacity, working/private memory, and Windows GPU engine utilization plus dedicated/shared GPU memory when the driver exposes them. \
The first sample may report warming_up while interval counters collect a baseline.",
        schema: r#"{"type": "object", "properties": {}}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "get_viewport",
        description: "Viewport size of the app window in physical pixels (`width height`).",
        schema: r#"{"type": "object", "properties": {}}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "ping",
        description: "Check that the target ZephyraUI app is reachable over the debug bridge. \
Returns pong on success. Use this to tell 'MCP server is ready' apart from 'app is online' when \
other tools report connection failures.",
        schema: r#"{"type": "object", "properties": {}}"#,
        annotations: READ_ONLY,
    },
    Tool {
        name: "hover",
        description: "Move the pointer to (x, y) — same as a physical mouse move; hover \
states and hit-testing see the synthetic event exactly like a real one. Takes effect \
this frame; read the tree afterwards to see what changed.",
        schema: r#"{"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}}, "required": ["x", "y"]}"#,
        annotations: MUTATES,
    },
    Tool {
        name: "click",
        description: "Synthesize a full click (move → down → up) at (x, y). Pick the \
coordinate from `get_tree` output. Takes effect this frame.",
        schema: r#"{"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}}, "required": ["x", "y"]}"#,
        annotations: MUTATES,
    },
    Tool {
        name: "press_key",
        description: "Synthesize one logical key press and release (for example Enter, \
ArrowDown, Escape, or a character). The running View receives both key-edge events.",
        schema: r#"{"type": "object", "properties": {"key": {"type": "string", "description": "One logical key name; spaces and control characters are not allowed"}}, "required": ["key"]}"#,
        annotations: MUTATES,
    },
    Tool {
        name: "scroll",
        description: "Synthesize a wheel event at (x,y) in physical pixels. delta_y > 0 \
means scroll up; delta_y < 0 means scroll down. The View receives the position and \
wheel deltas (native events preserve the platform's pixel/line units).",
        schema: r#"{"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "delta_x": {"type": "number"}, "delta_y": {"type": "number"}}, "required": ["x", "y", "delta_x", "delta_y"]}"#,
        annotations: MUTATES,
    },
    Tool {
        name: "pointer_drag",
        description: "Press and hold the primary pointer button while moving, then release. \
Modes: segmented follows a polyline in `points`; linear moves from `start` to `end`; easing \
uses `start`/`end` and `easing` (linear, ease_in, ease_out, smoothstep). Sampling occurs \
across `duration_ms`; optional hold_before_ms/hold_after_ms keep the button down. \
Coordinates are physical pixels. Sampling and total duration are bounded.",
        schema: r#"{"type":"object","properties":{"mode":{"type":"string","enum":["segmented","linear","easing"]},"points":{"type":"array","minItems":2,"maxItems":128,"items":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"]}},"start":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"]},"end":{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"}},"required":["x","y"]},"easing":{"type":"string","enum":["linear","ease_in","ease_out","smoothstep"]},"duration_ms":{"type":"integer","minimum":1,"maximum":60000},"sample_interval_ms":{"type":"integer","minimum":1,"maximum":1000,"default":16},"hold_before_ms":{"type":"integer","minimum":0,"maximum":10000,"default":0},"hold_after_ms":{"type":"integer","minimum":0,"maximum":10000,"default":0}},"required":["mode","duration_ms"]}"#,
        annotations: MUTATES,
    },
    Tool {
        name: "analyze_performance",
        description: "Collect a bounded CPU frame-performance profile and return it when the \
requested capture duration ends. Records per-frame update/layout, render-submit and total CPU \
generation time, with min/average/median/p95/max plus frame-by-frame rows. Sampling is opt-in; \
duration_seconds is required and limited to 1..=300 seconds, with automatic stop at 3000 frames. \
By default only the summary is returned; pass include_rows=true to also receive the per-frame rows.",
        schema: r#"{"type":"object","properties":{"duration_seconds":{"type":"integer","minimum":1,"maximum":300,"description":"Required bounded capture duration; automatically stops at expiry or 3000 frames."},"include_rows":{"type":"boolean","default":false,"description":"Also return per-frame rows (can be up to 3000 lines). Default false keeps the response summary-only."}},"required":["duration_seconds"]}"#,
        annotations: MUTATES,
    },
    Tool {
        name: "screenshot",
        description: "Render a frame and read back the pixels as a 24bpp BMP. D3D11 and \
D3D12 apps support this. Returns the absolute file path that was written; use get_layout for \
textual layout context without pixel data.",
        schema: r#"{"type": "object", "properties": {"path": {"type": "string", "description": "where to write the .bmp; defaults to a temp file"}}}"#,
        annotations: MUTATES,
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

    // 取消登记表：stdin 线程写，工具工作线程读（见模块文档的并发模型）。
    let cancelled = Cancelled::default();
    let (job_tx, job_rx) = std::sync::mpsc::channel::<String>();
    let worker_cancelled = cancelled.clone();
    let worker_host = host.clone();
    std::thread::Builder::new()
        .name("zephyra-mcp-tools".into())
        .spawn(move || {
            // 工具线程持有唯一的桥会话——工具调用因此天然串行，AI 的
            // 「读-改-读」次序不会被并发打乱。
            let mut session = Session::new(&worker_host, port, worker_cancelled);
            for line in job_rx {
                if let Some(reply) = handle_message(&line, &mut session) {
                    write_reply(&reply);
                }
            }
            // 通道关闭即退出。**不**等在途调用：客户端关 stdin 意味着它已经
            // 不在了，等一个没人收的应答只会让 host 觉得 server 卡死。
        })
        .map_err(|e| CliError::failure(format!("cannot start tool worker thread: {e}")))?;

    // 主线程只处理不碰桥的消息（initialize / tools/list / ping / 通知）。
    // 握手不需要应用在线——AI 客户端往往先拉起 MCP 再起应用。
    let mut session = Session::new(&host, port, cancelled);

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
        if is_tools_call(&line) {
            // 投给工作线程；应答由它直接写 stdout（行级原子，见 write_reply）。
            if job_tx.send(line).is_err() {
                io.err_line("[zephyra-mcp] tool worker thread is gone; dropping tool call");
            }
        } else if let Some(reply) = handle_message(&line, &mut session) {
            write_reply(&reply);
        }
    }
    Ok(ExitCode::Success)
}

/// 一条消息是否是工具调用（决定它去工作线程还是主线程就地处理）。
fn is_tools_call(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|msg| {
            msg.get("method")
                .and_then(Value::as_str)
                .map(|m| m == "tools/call")
        })
        .unwrap_or(false)
}

/// 写一行协议消息到 stdout。两个线程都会调：`Stdout` 的内部锁保证整行
/// 原子（LineWriter 在锁内冲到末尾换行），不会出现两条应答交错成乱 JSON。
fn write_reply(reply: &str) {
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{reply}");
    let _ = lock.flush();
}

/// 一个会话的桥侧状态：连接缓存 + 取消登记表。
///
/// 主线程与工具线程各有一个实例（主线程的 `client` 永远是 `None`——它不碰桥），
/// 但共享同一张取消表。
struct Session {
    host: String,
    port: u16,
    client: Option<BridgeClient>,
    cancelled: Cancelled,
}

impl Session {
    fn new(host: &str, port: u16, cancelled: Cancelled) -> Self {
        Self {
            host: host.to_string(),
            port,
            client: None,
            cancelled,
        }
    }

    /// 拿到一条可用的桥连接：没有就连（重试 3 次）。
    ///
    /// 唯一的连接管理者——连接的生灭全部收敛在这里，不会出现
    /// 「两条路径各养一个连接」。
    fn ensure_client(&mut self) -> Result<&mut BridgeClient, String> {
        if self.client.is_none() {
            let (host, port) = (self.host.clone(), self.port);
            self.client = Some(BridgeClient::connect(&host, port, 3).map_err(|e| {
                format!(
                    "连不上应用调试桥 {host}:{port}（{e}）——确认应用以 ZEPHYRA_DEBUG_PORT={port} 启动"
                )
            })?);
        }
        Ok(self.client.as_mut().unwrap())
    }

    /// 丢弃当前连接。**半读之后的流不可复用**：缓冲里可能压着半条响应，
    /// 继续用会让后续响应切分错位；唯一正确的动作是断掉重连。
    fn drop_client(&mut self) {
        self.client = None;
    }
}

/// 已取消请求 id 的登记表（跨线程共享）。id 存 JSON 文本，数字/字符串都对得上。
#[derive(Clone, Default)]
struct Cancelled(std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>);

impl Cancelled {
    fn mark(&self, id: &Value) {
        if let Ok(mut set) = self.0.lock() {
            set.insert(id.to_string());
        }
    }

    /// 取出并清除：`true` = 这个请求已被取消，调用方应回「无应答」。
    fn take(&self, id: &str) -> bool {
        self.0.lock().map(|mut set| set.remove(id)).unwrap_or(false)
    }
}

/// 应用侧桥的默认端口。CLI 不依赖 zephyra-fx，常量这里**独立**写——
/// 但它必须与 `zephyra_fx::bridge::DEFAULT_PORT` 一致（测试钉住这一点）。
fn zephyra_bridge_port() -> u16 {
    7878
}

/// 处理一条 JSON-RPC 消息。返回要写回 stdout 的 JSON 行；通知类与已取消的
/// 请求返回 `None`（协议规定：被取消的请求不回）。
///
/// 拆成纯函数（stdin/网络都走参数）是为了能测：MCP 消息的形状与分派逻辑
/// 不必起真进程就能钉住。
fn handle_message(line: &str, session: &mut Session) -> Option<String> {
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
    let id_key = id.to_string();
    let is_notification = msg.get("id").is_none();

    match method {
        // 通知不回（回了会污染 AI 客户端的消息流）。
        "notifications/initialized" => None,
        "notifications/cancelled" => {
            // 登记后两处消费：工具线程开跑前（跳过排队中的调用）、
            // analyze_performance 的轮询间隙（中止在途采集）。
            if let Some(request_id) = msg.pointer("/params/requestId") {
                session.cancelled.mark(request_id);
            }
            None
        }
        // MCP 的 keepalive。回空 result，不碰桥——客户端用它探测 server 活性，
        // 与「应用在不在线」无关。
        "ping" => Some(json_reply(id, json!({}))),
        "initialize" => {
            // 版本协商：本 server 只实现了 2024-11-05 的消息形状（tools +
            // stdio）。客户端要更新版本时回我们的版本——按协议这就是降级
            // 信号，客户端会据此决定继续还是断开。客户端要的版本打到 stderr
            // 供排查。
            if let Some(v) = msg
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
            {
                eprintln!("[zephyra-mcp] client requested protocolVersion={v}; serving 2024-11-05");
            }
            Some(json_reply(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "zephyra", "version": crate::VERSION}
                }),
            ))
        }
        "tools/list" => {
            let tools: Vec<Value> = TOOLS
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": serde_json::from_str::<Value>(t.schema)
                            .expect("内置工具的 schema 必须是合法 JSON"),
                        "annotations": serde_json::from_str::<Value>(t.annotations)
                            .expect("内置工具的 annotations 必须是合法 JSON"),
                    })
                })
                .collect();
            Some(json_reply(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            if session.cancelled.take(&id_key) {
                // 排队期间被取消：不执行、不回。
                return None;
            }
            let name = msg
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() {
                return Some(json_reply(
                    id,
                    json!({
                        "content": [{"type": "text", "text":
                            "tools/call 需要 params.name；用 tools/list 查看可用工具"}],
                        "isError": true
                    }),
                ));
            }
            let tool_args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            match run_tool(name, &tool_args, session, &id_key) {
                Ok(ToolOutcome::Reply(text)) => Some(json_reply(
                    id,
                    json!({"content": [{"type": "text", "text": text}]}),
                )),
                Ok(ToolOutcome::Cancelled) => None,
                Err(e) => Some(json_reply(
                    id,
                    json!({"content": [{"type": "text", "text": e}], "isError": true}),
                )),
            }
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

/// 在桥连接上执行一条命令；传输失败时**丢连接、重连、重试一次**。
///
/// 实测踩过：旧版把连接缓存在会话里、失败后却从不刷新——应用重启（dev
/// 工作流的常态）之后，本进程的每个工具调用都对着一条死连接报「桥连接已
/// 断开」，直到 MCP host 杀掉进程重配。旧注释写着「连接失败重连一次再试」，
/// 实现里没有那一句。
///
/// 重试的代价：读超时/读失败时命令可能已经被应用执行过（只是响应丢了），
/// 对输入类命令意味着注入两次。权衡后仍然重试——一个永久死掉的会话比
/// 偶发的重复点击糟得多；而且读超时要先冻结 10 秒，那时重试同样不会成功。
fn with_bridge<T>(
    session: &mut Session,
    f: impl Fn(&mut BridgeClient) -> Result<T, String>,
) -> Result<T, String> {
    let first = f(session.ensure_client()?);
    match first {
        Ok(v) => Ok(v),
        Err(first_err) => {
            session.drop_client();
            match session.ensure_client() {
                Ok(client) => f(client).map_err(|second_err| {
                    format!(
                        "{first_err}；重连 {}:{} 重试仍失败：{second_err}",
                        session.host, session.port
                    )
                }),
                Err(connect_err) => Err(format!("{first_err}；重连失败：{connect_err}")),
            }
        }
    }
}

/// 工具执行的结果。`Cancelled` = 请求已被取消，按协议**不回**。
enum ToolOutcome {
    Reply(String),
    Cancelled,
}

fn reply(text: String) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Reply(text))
}

/// 桥响应 → 文本正文（或应用侧错误）。
fn text_of(r: BridgeResponse) -> Result<String, String> {
    match r {
        BridgeResponse::Text(lines) => Ok(if lines.is_empty() {
            "(empty)".into()
        } else {
            lines.join("\n")
        }),
        BridgeResponse::Err(e) => Err(e),
        BridgeResponse::Binary { .. } => Err("意外的二进制响应".into()),
    }
}

/// 执行一个工具调用。返回给 AI 的文本（或「取消」）。
///
/// `cancel_key` 是本次请求的 id：`analyze_performance` 的每轮轮询间隙会查
/// 取消表，让 AI 能中止一次长达 300 秒的采集。
fn run_tool(
    name: &str,
    a: &Value,
    session: &mut Session,
    cancel_key: &str,
) -> Result<ToolOutcome, String> {
    match name {
        "get_tree" => reply(text_of(with_bridge(session, |c| c.call("tree"))?)?),
        "get_layout" => reply(text_of(with_bridge(session, |c| c.call("layout"))?)?),
        "get_stats" => reply(text_of(with_bridge(session, |c| c.call("stats"))?)?),
        "get_resources" => reply(text_of(with_bridge(session, |c| c.call("resources"))?)?),
        "get_viewport" => reply(text_of(with_bridge(session, |c| c.call("viewport"))?)?),
        "ping" => {
            let text = text_of(with_bridge(session, |c| c.call("ping"))?)?;
            reply(format!(
                "app reachable over the debug bridge ({text}) — 与「MCP server 已就绪」不同，这才说明应用在线"
            ))
        }
        "get_props" => {
            let path = a
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("get_props 需要 path 参数（来自 get_tree 的路径列）")?;
            reply(text_of(with_bridge(session, |c| {
                c.call(&format!("props {path}"))
            })?)?)
        }
        "hover" => {
            let (x, y) = xy(a)?;
            reply(text_of(with_bridge(session, |c| {
                c.call(&format!("hover {x} {y}"))
            })?)?)
        }
        "click" => {
            let (x, y) = xy(a)?;
            reply(text_of(with_bridge(session, |c| {
                c.call(&format!("click {x} {y}"))
            })?)?)
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
            reply(text_of(with_bridge(session, |c| {
                c.call(&format!("press_key {key}"))
            })?)?)
        }
        "scroll" => {
            let (x, y) = xy(a)?;
            let (delta_x, delta_y) = deltas(a)?;
            reply(text_of(with_bridge(session, |c| {
                c.call(&format!("scroll {x} {y} {delta_x} {delta_y}"))
            })?)?)
        }
        "pointer_drag" => {
            let plan = drag_samples(a)?;
            match perform_pointer_drag(
                session,
                plan.start,
                &plan.samples,
                plan.hold_before_ms,
                plan.hold_after_ms,
            ) {
                Ok(()) => reply(format!(
                    "pointer drag completed ({} movement samples)",
                    plan.samples.len()
                )),
                Err(e) => {
                    if matches!(e, DragError::Transport(_)) {
                        // 拖动是多命令手势，不能整段盲目重试（按下态可能已建立）。
                        // 丢掉半读的连接，下一次调用会重连。
                        session.drop_client();
                    }
                    Err(e.into_message())
                }
            }
        }
        "analyze_performance" => {
            let duration_seconds = profile_duration_seconds(a)?;
            let include_rows = a
                .get("include_rows")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            text_of(with_bridge(session, |c| {
                c.call(&format!("profile_start {duration_seconds}"))
            })?)?;
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(duration_seconds + 10);
            loop {
                if session.cancelled.take(cancel_key) {
                    return Ok(ToolOutcome::Cancelled);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                let report = text_of(with_bridge(session, |c| c.call("profile_report"))?)?;
                if report.starts_with("PROFILE_STATE complete") {
                    return reply(if include_rows {
                        report
                    } else {
                        strip_profile_rows(&report)
                    });
                }
                if std::time::Instant::now() >= deadline {
                    return Err("性能采样未在声明时限内完成；可能宿主帧循环已停止".into());
                }
            }
        }
        "screenshot" => {
            let r = with_bridge(session, |c| c.call("shot"))?;
            match r {
                BridgeResponse::Binary { data, .. } => {
                    if data.is_empty() {
                        return Err("后端读回为空".into());
                    }
                    let path = resolve_shot_path(a);
                    std::fs::write(&path, &data).map_err(|e| format!("写 {path} 失败: {e}"))?;
                    // 回**绝对**路径：AI 客户端不知道本进程的 cwd，相对路径
                    // 等于没说。
                    reply(format!(
                        "screenshot written to {path} ({} bytes, 24bpp BMP)",
                        data.len()
                    ))
                }
                BridgeResponse::Err(e) => Err(e),
                BridgeResponse::Text(_) => Err("桥回了文本而不是图像".into()),
            }
        }
        other => Err(format!(
            "unknown tool `{other}`；用 tools/list 查看可用工具"
        )),
    }
}

/// 性能报告默认只回汇总块。
///
/// 逐帧行最多 3000 行 × 约 40 字节 ≈ 120KB——整段灌进模型上下文是纯负担
/// （实测 gallery 2 秒 155 帧就有 155 行）；而 min/avg/p50/p95/max 汇总才是
/// 判断依据。要逐帧数据的调用方传 `include_rows=true`。
fn strip_profile_rows(report: &str) -> String {
    const HEADER: &str = "frame_index elapsed_ms total_us update_layout_us render_submit_us";
    let Some(header_at) = report.find(HEADER) else {
        return report.to_string();
    };
    let rows_start = report[header_at..]
        .find('\n')
        .map(|i| header_at + i + 1)
        .unwrap_or(report.len());
    let row_count = report[rows_start..]
        .lines()
        .filter(|l| !l.is_empty())
        .count();
    if row_count == 0 {
        return report.to_string();
    }
    format!(
        "{}(已省略 {row_count} 行逐帧数据；传 include_rows=true 获取全量)\n",
        &report[..rows_start]
    )
}

/// `screenshot` 的落盘路径：给 `path` 就用它（转成绝对），否则临时目录。
fn resolve_shot_path(a: &Value) -> String {
    match a.get("path").and_then(Value::as_str) {
        Some(p) if !p.is_empty() => absolute_path(p),
        _ => std::env::temp_dir()
            .join(format!("zephyra-shot-{}.bmp", std::process::id()))
            .display()
            .to_string(),
    }
}

/// 相对路径补上 cwd——给 AI 的回执里必须是客户端能直接打开的绝对路径。
fn absolute_path(p: &str) -> String {
    let path = std::path::Path::new(p);
    if path.is_absolute() {
        return p.to_string();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path).display().to_string(),
        Err(_) => p.to_string(),
    }
}

/// 从工具参数里取一个有限、可由 f32 表示的数，返回它的文本形式。
///
/// f32 范围检查与拖动路径一致：1e300 这种数若放行，会在桥侧被解析成 inf
/// 再被拒——错误离原因更远，且白白一个来回。
fn finite_f32(v: &Value, name: &str) -> Result<String, String> {
    let n = v.as_f64().ok_or_else(|| format!("{name} 必须是数字"))?;
    if !n.is_finite() || n.abs() > f32::MAX as f64 {
        return Err(format!("{name} 必须是有限且 f32 可表示的数"));
    }
    Ok(format!("{n}"))
}

/// 从工具参数里取 (x, y)，接受 int / float。
fn xy(a: &Value) -> Result<(String, String), String> {
    let get = |k: &str| -> Result<String, String> {
        let v = a.get(k).ok_or_else(|| format!("需要 {k} 坐标"))?;
        finite_f32(v, k)
    };
    Ok((get("x")?, get("y")?))
}

fn deltas(a: &Value) -> Result<(String, String), String> {
    let get = |k: &str| -> Result<String, String> {
        let v = a.get(k).ok_or_else(|| format!("需要 {k} 滚轮增量"))?;
        finite_f32(v, k)
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

/// 拖动过程中的失败。区分「连接层面」与「应用回的 ERR」：只有前者需要
/// 丢连接重连，后者说明连接完好，会话可以继续用。
enum DragError {
    Transport(String),
    App(String),
}

impl DragError {
    fn into_message(self) -> String {
        match self {
            Self::Transport(m) | Self::App(m) => m,
        }
    }
}

fn drag_send(session: &mut Session, command: &str) -> Result<(), DragError> {
    let response = session
        .ensure_client()
        .map_err(DragError::Transport)?
        .call(command)
        .map_err(DragError::Transport)?;
    match response {
        BridgeResponse::Text(_) => Ok(()),
        BridgeResponse::Err(e) => Err(DragError::App(e)),
        BridgeResponse::Binary { .. } => Err(DragError::App("拖动命令收到意外二进制响应".into())),
    }
}

fn perform_pointer_drag(
    session: &mut Session,
    start: (f64, f64),
    samples: &[DragSample],
    hold_before_ms: u64,
    hold_after_ms: u64,
) -> Result<(), DragError> {
    drag_send(session, &format!("hover {} {}", start.0, start.1))?;

    // 移动中途失败也要补抬起：按下态是**跨命令**状态，不清理的话应用
    // 会一直认为按钮按着（后续点击全变成拖动的延续）。
    let movement = (|| {
        drag_send(session, "pointer_down")?;
        if hold_before_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hold_before_ms));
        }
        for sample in samples {
            drag_send(session, &format!("hover {} {}", sample.x, sample.y))?;
            if sample.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(sample.delay_ms));
            }
        }
        if hold_after_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hold_after_ms));
        }
        Ok(())
    })();
    let release = drag_send(session, "pointer_up");
    match (movement, release) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(DragError::App(format!(
            "拖动完成但抬起注入失败：{}",
            error.into_message()
        ))),
        (Err(error), Err(release_error)) => Err(DragError::App(format!(
            "{}；清理时补发 pointer_up 也失败：{}",
            error.into_message(),
            release_error.into_message()
        ))),
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

    /// 模拟应用重启：第一条连接收到命令后**不回就断开**；第二条连接正常应答。
    fn mock_bridge_dying_once(
        second_resp: &'static str,
    ) -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let read_line = |stream: &std::net::TcpStream| {
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                line.trim_end().to_string()
            };
            let (first, _) = listener.accept().unwrap();
            let _ = tx.send(format!("first:{}", read_line(&first)));
            drop(first); // 应用死掉：不回任何响应，直接关连接
            let (mut second, _) = listener.accept().unwrap();
            let _ = tx.send(format!("second:{}", read_line(&second)));
            writeln!(second, "{second_resp}\nOK").unwrap();
        });
        (port, rx)
    }

    /// 一直回「采集中」的假 profile：应用收到 `profile_start` 后持续 running，
    /// 直到客户端断开。给「在途取消」测试用——取消发生在轮询间隙。
    fn mock_bridge_running_profile() -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break, // 客户端断开即收工
                    Ok(_) => {}
                }
                let cmd = line.trim_end().to_string();
                let resp = if cmd.starts_with("profile_start") {
                    "performance profiling started for 1s"
                } else {
                    "PROFILE_STATE running remaining_ms=900"
                };
                if tx.send(cmd).is_err() || writeln!(stream, "{resp}\nOK").is_err() {
                    break;
                }
            }
        });
        (port, rx)
    }

    /// 回一条 4 字节的 BIN 截图（真二进制帧：BIN 头 + 载荷 + OK 终止符）。
    fn mock_bridge_binary_shot() -> (u16, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let _ = tx.send(line.trim_end().to_string());
            stream.write_all(b"BIN 4 bmp 24bpp\n").unwrap();
            stream.write_all(&[0x42, 0x4d, 0x00, 0x00]).unwrap();
            stream.write_all(b"\nOK\n").unwrap();
        });
        (port, rx)
    }

    #[test]
    fn initialize_announces_tools_capability() {
        let mut session = Session::new("127.0.0.1", 7878, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            &mut session,
        )
        .unwrap();
        assert!(r.contains("\"protocolVersion\""), "{r}");
        assert!(r.contains("serverInfo"), "{r}");
        // 握手不碰桥——AI 客户端先配 MCP 再起应用是常态。
        assert!(session.client.is_none());
    }

    /// JSON-RPC 层的 `ping`（MCP keepalive）必须立即回空 result 且不碰桥：
    /// 它探测的是「MCP server 活没活着」，与应用在不在线无关。
    #[test]
    fn jsonrpc_ping_does_not_touch_bridge() {
        // 端口 1 必然连不上：若 ping 碰桥，会得到 isError 而不是空 result。
        let mut session = Session::new("127.0.0.1", 1, Cancelled::default());
        let r =
            handle_message(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#, &mut session).unwrap();
        assert!(r.contains("\"result\":{}"), "{r}");
        assert!(!r.contains("isError"), "{r}");
        assert!(session.client.is_none());
    }

    /// 工具层的 `ping` 走桥：回执必须点明「这才说明应用在线」。
    #[test]
    fn ping_tool_reports_app_status() {
        let (port, seen) = mock_bridge("pong");
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"ping","arguments":{}}}"#,
            &mut session,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "ping");
        assert!(r.contains("app reachable over the debug bridge"), "{r}");
        assert!(r.contains("pong"), "{r}");
    }

    #[test]
    fn notifications_produce_no_output() {
        let mut session = Session::new("127.0.0.1", 7878, Cancelled::default());
        assert!(handle_message(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut session
        )
        .is_none());
    }

    #[test]
    fn tools_list_contains_every_tool() {
        let mut session = Session::new("127.0.0.1", 7878, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            &mut session,
        )
        .unwrap();
        for t in [
            "get_tree",
            "get_layout",
            "get_props",
            "get_stats",
            "get_resources",
            "get_viewport",
            "ping",
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
        assert!(session.client.is_none(), "tools/list 也不该碰桥");
    }

    /// annotations 必须把「只读」与「会改状态」分开：AI 客户端据此决定哪些
    /// 调用可以直接发、哪些要先征求用户同意。
    #[test]
    fn annotations_declare_read_only_vs_mutating() {
        let mut session = Session::new("127.0.0.1", 7878, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":16,"method":"tools/list"}"#,
            &mut session,
        )
        .unwrap();
        let list: Value = serde_json::from_str(&r).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        let find = |name: &str| {
            tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("tools/list 缺 {name}"))
        };
        assert_eq!(find("get_tree")["annotations"]["readOnlyHint"], true);
        assert_eq!(find("click")["annotations"]["readOnlyHint"], false);
        assert_eq!(find("pointer_drag")["annotations"]["readOnlyHint"], false);
        // 采集会启动进程内 profiler，属于改状态——但 destructiveHint 仍为 false。
        let profile = find("analyze_performance")["annotations"].clone();
        assert_eq!(profile["readOnlyHint"], false);
        assert_eq!(profile["destructiveHint"], false);
    }

    /// `tools/call` 漏 params.name 是客户端 bug 的信号，必须回明确错误
    /// （旧版落进 `unknown tool `` ` 的兜底，报错文本指不到真因）。
    #[test]
    fn tools_call_missing_name_is_a_clear_error() {
        let mut session = Session::new("127.0.0.1", 1, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{}}"#,
            &mut session,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(r.contains("params.name"), "报错要指出缺什么：{r}");
    }

    #[test]
    fn tool_call_forwards_to_bridge_command() {
        let (port, seen) = mock_bridge("0 Container root 0,0 800x600");
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_tree","arguments":{}}}"#,
            &mut session,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "tree");
        assert!(r.contains("0 Container root"), "{r}");
        assert!(!r.contains("\"isError\":true"), "{r}");
        assert!(session.client.is_some(), "工具调用后连接应当被缓存复用");
    }

    /// 应用重启（dev 工作流常态）会留下一条死连接。传输失败必须**丢连接、
    /// 重连、重试一次**——旧版缓存死连接从不刷新，会话从此永久报废。
    #[test]
    fn dead_connection_is_reconnected_and_retried_once() {
        let (port, seen) = mock_bridge_dying_once("reconnected root");
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"get_tree","arguments":{}}}"#,
            &mut session,
        )
        .unwrap();
        assert_eq!(
            seen.try_iter().collect::<Vec<_>>(),
            ["first:tree", "second:tree"],
            "第一次连接读到命令即断开，第二次连接应答"
        );
        assert!(!r.contains("\"isError\":true"), "重连重试后应成功：{r}");
        assert!(r.contains("reconnected root"), "{r}");
    }

    /// 排队期间被取消（`notifications/cancelled` 先于工具线程开跑到达）：
    /// 不执行、不回、不碰桥。
    #[test]
    fn queued_call_cancelled_before_start_is_skipped() {
        // 端口 1 连不上：若取消失效去执行了工具，会得到 isError 而不是 None。
        let mut session = Session::new("127.0.0.1", 1, Cancelled::default());
        session.cancelled.mark(&json!(99));
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"get_tree","arguments":{}}}"#,
            &mut session,
        );
        assert!(r.is_none(), "被取消的请求按协议不回：{r:?}");
        assert!(session.client.is_none(), "被取消的调用不该碰桥");
    }

    /// 在途的 `analyze_performance` 要能被取消：采集循环的每轮轮询间隙
    /// 查取消表，AI 由此能中止一次长达 300 秒的采样。
    #[test]
    fn in_flight_performance_capture_is_cancellable() {
        let (port, _seen) = mock_bridge_running_profile();
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let canceller = {
            let cancelled = session.cancelled.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                cancelled.mark(&json!(42));
            })
        };
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"analyze_performance","arguments":{"duration_seconds":1}}}"#,
            &mut session,
        );
        canceller.join().unwrap();
        assert!(r.is_none(), "采集中止后按协议不回：{r:?}");
    }

    #[test]
    fn resources_tool_forwards_to_bridge() {
        let (port, seen) = mock_bridge("cpu_process_percent=warming_up");
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"get_resources","arguments":{}}}"#,
            &mut session,
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
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let _ = handle_message(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"click","arguments":{"x":12.5,"y":48}}}"#,
            &mut session,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "click 12.5 48");
    }

    /// 坐标在 CLI 侧就做 f32 范围校验：1e300 若放行，会在桥侧被解析成 inf
    /// 再被拒——错误离原因更远，且白白一次连接。
    #[test]
    fn hover_rejects_out_of_f32_range_coordinates() {
        let mut session = Session::new("127.0.0.1", 1, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"hover","arguments":{"x":1e300,"y":0}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(r.contains("f32"), "报错要说明原因：{r}");
        assert!(session.client.is_none(), "参数错误不该建立连接");
    }

    #[test]
    fn layout_tool_requests_plain_text_layout_context() {
        let (port, seen) = mock_bridge("viewport 800x600 physical-pixels\nstats quads=2\nlayout-tree path tag key absolute-x,absolute-y widthxheight [classes]\n0 Container root 0,0 800x600");
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_layout","arguments":{}}}"#,
            &mut session,
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
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        handle_message(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"Enter"}}}"#,
            &mut session,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "press_key Enter");

        let (port, seen) = mock_bridge("scrolled");
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        handle_message(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"scroll","arguments":{"x":12,"y":30,"delta_x":0,"delta_y":-2.5}}}"#,
            &mut session,
        )
        .unwrap();
        assert_eq!(seen.recv().unwrap(), "scroll 12 30 0 -2.5");

        let mut session = Session::new("127.0.0.1", 7878, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"Enter\nshot"}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(session.client.is_none(), "注入内容应在连接桥之前拒绝");
    }

    #[test]
    fn pointer_drag_sends_down_moves_and_release() {
        let (port, seen) = mock_bridge_sequence(4);
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"pointer_drag","arguments":{"mode":"linear","start":{"x":1,"y":2},"end":{"x":7,"y":9},"duration_ms":1,"sample_interval_ms":1000}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(response.contains("pointer drag completed"), "{response}");
        assert_eq!(
            seen.try_iter().collect::<Vec<_>>(),
            ["hover 1 2", "pointer_down", "hover 7 9", "pointer_up"]
        );

        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"pointer_drag","arguments":{"mode":"segmented","points":[{"x":1,"y":2}],"duration_ms":10}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(response.contains("\"isError\":true"), "{response}");
        assert!(session.client.is_none(), "无效路径必须在建立桥连接前拒绝");
    }

    #[test]
    fn pointer_drag_attempts_release_after_a_move_error() {
        let (port, seen) = mock_bridge_with_responses(&[
            "hovered",
            "pointer_down",
            "ERR simulated move failure",
            "pointer_up",
        ]);
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"pointer_drag","arguments":{"mode":"linear","start":{"x":1,"y":2},"end":{"x":7,"y":9},"duration_ms":1,"sample_interval_ms":1000}}}"#,
            &mut session,
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

    /// 性能报告默认只回汇总：逐帧行是模型上下文的纯负担。逐帧数据用
    /// `include_rows=true` 取。
    #[test]
    fn performance_report_strips_rows_by_default_and_restores_them_on_request() {
        let responses: &[&str] = &[
            "performance profiling started for 1s",
            "PROFILE_STATE running remaining_ms=50",
            "PROFILE_STATE complete\nframes=2\nmin_us=100 max_us=110\nframe_index elapsed_ms total_us update_layout_us render_submit_us\n0 0 100 60 40\n1 16 110 65 45",
        ];

        let (port, seen) = mock_bridge_with_responses(responses);
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"analyze_performance","arguments":{"duration_seconds":1}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(response.contains("PROFILE_STATE complete"), "{response}");
        assert!(
            response.contains("min_us=100"),
            "汇总块必须保留：{response}"
        );
        assert!(
            !response.contains("1 16 110 65 45"),
            "逐帧行默认剥离：{response}"
        );
        assert!(
            response.contains("include_rows=true"),
            "回执要提示怎么取全量：{response}"
        );
        assert_eq!(
            seen.try_iter().collect::<Vec<_>>(),
            ["profile_start 1", "profile_report", "profile_report"]
        );

        let (port, _seen) = mock_bridge_with_responses(responses);
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let response = handle_message(
            r#"{"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"analyze_performance","arguments":{"duration_seconds":1,"include_rows":true}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(
            response.contains("1 16 110 65 45"),
            "include_rows=true 要有逐帧行：{response}"
        );
        assert!(!response.contains("已省略"), "{response}");
    }

    #[test]
    fn strip_profile_rows_only_removes_frame_rows() {
        let report = "PROFILE_STATE complete\nframes=1\nframe_index elapsed_ms total_us update_layout_us render_submit_us\n0 0 100 60 45";
        let stripped = strip_profile_rows(report);
        assert!(stripped.contains("frames=1"), "{stripped}");
        assert!(stripped.contains("已省略 1 行"), "{stripped}");
        assert!(!stripped.contains("0 0 100 60 45"), "{stripped}");
        // 没有表头（如后端旧版报告）就原样返回——剥错内容比不剥更糟。
        let legacy = "PROFILE_STATE complete\nframes=1\n0 0 100 60 45";
        assert_eq!(strip_profile_rows(legacy), legacy);
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

    /// 截图回执必须是**绝对**路径：AI 客户端不知道本进程的 cwd，相对路径
    /// 等于没说。
    #[test]
    fn screenshot_reply_carries_an_absolute_path() {
        let shot =
            std::env::temp_dir().join(format!("zephyra-cli-test-{}.bmp", std::process::id()));
        let shot_str = shot.display().to_string();
        let (port, _seen) = mock_bridge_binary_shot();
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let r = handle_message(
            &format!(
                r#"{{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{{"name":"screenshot","arguments":{{"path":"{}"}}}}}}"#,
                shot_str.replace('\\', "\\\\")
            ),
            &mut session,
        )
        .unwrap();
        assert!(!r.contains("\"isError\":true"), "{r}");
        assert!(r.contains("4 bytes"), "{r}");
        // 绝对性用 std 判，不靠字符串前缀（Windows 长路径/盘符大小写都算）。
        let reported = r
            .split("screenshot written to ")
            .nth(1)
            .and_then(|rest| rest.split(" (").next())
            .expect("回执要含路径");
        assert!(
            std::path::Path::new(reported).is_absolute(),
            "回执路径必须是绝对路径：{reported}"
        );
        let _ = std::fs::remove_file(&shot);
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
        let mut session = Session::new("127.0.0.1", port, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_props","arguments":{"path":"9.9"}}}"#,
            &mut session,
        )
        .unwrap();
        assert!(r.contains("\"isError\":true"), "{r}");
        assert!(r.contains("不存在"), "{r}");
    }

    #[test]
    fn bad_json_is_a_parse_error_not_a_crash() {
        let mut session = Session::new("127.0.0.1", 7878, Cancelled::default());
        let r = handle_message("not json", &mut session).unwrap();
        assert!(r.contains("-32700"), "{r}");
    }

    #[test]
    fn unreachable_bridge_is_a_tool_error_not_a_panic() {
        // 端口 1（几乎必然无监听）：重连重试也失败后，工具调用以 isError 结束。
        let mut session = Session::new("127.0.0.1", 1, Cancelled::default());
        let r = handle_message(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"get_tree","arguments":{}}}"#,
            &mut session,
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
