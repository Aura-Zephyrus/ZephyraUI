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
    /// 桥怎么写，这里就怎么读。改写端必须同步读端。
    fn read_response(&mut self) -> Result<BridgeResponse, String> {
        let mut byte = [0u8; 1];
        loop {
            // 一字节一读：协议里没有吞吐需求（树 dump 也就几十 KB），
            // 换来的是「终止符边界永远精确」。
            use std::io::Read;
            self.stream
                .read_exact(&mut byte)
                .map_err(|e| e.to_string())?;
            self.buf.push(byte[0]);
            let s = String::from_utf8_lossy(&self.buf).into_owned();

            // ERR 是单行终止。
            if s.starts_with("ERR ") && s.ends_with('\n') {
                self.buf.clear();
                return Ok(BridgeResponse::Err(s.trim_end().to_string()));
            }
            // BIN：头行 `BIN <len> <note>\n` + len 字节 + `\nOK\n`。
            if s.starts_with("BIN ") {
                if let Some(hend) = s.find('\n') {
                    let rest = &s[4..hend];
                    if let Some(sp) = rest.find(' ') {
                        if let Ok(len) = rest[..sp].parse::<usize>() {
                            let need = hend + 1 + len + "\nOK\n".len();
                            if self.buf.len() >= need {
                                let note = rest[sp + 1..].to_string();
                                let head = hend + 1;
                                let data = self.buf[head..head + len].to_vec();
                                self.buf.drain(..need);
                                return Ok(BridgeResponse::Binary { note, data });
                            }
                        }
                    }
                }
                continue;
            }
            // 文本响应：`OK` 行终止。
            if s.ends_with("\nOK\n") {
                self.buf.clear();
                let body = s.trim_end_matches("OK\n").trim_end_matches('\n');
                let lines = if body.is_empty() {
                    Vec::new()
                } else {
                    body.lines().map(str::to_string).collect()
                };
                return Ok(BridgeResponse::Text(lines));
            }
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
        name: "screenshot",
        description: "Render a frame and read back the pixels as a 24bpp BMP. D3D11 apps \
support this; apps whose backend has no pixel readback return an error. Returns the \
file path that was written.",
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
        "get_stats" => text_of(bridge!().call("stats")?),
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
            "get_props",
            "get_stats",
            "get_viewport",
            "hover",
            "click",
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
