//! 调试桥：AI 与外部工具经 **localhost TCP** 读取布局、操控界面。
//!
//! # 定位：库层只管「管道」，不管「谁在听」
//!
//! MCP（Model Context Protocol）是 stdio 上的 JSON-RPC，而 AI 客户端拉起的是
//! `zephyra mcp` 这个 CLI 进程——它够不着运行中的应用。所以链路是三层：
//!
//! ```text
//! AI 客户端 ──stdio/JSON── zephyra mcp（CLI）──TCP 行协议── 应用（本模块）
//! ```
//!
//! 本模块住在 `zephyra-fx` 而不是某个渲染后端：它一个 GPU 类型都不碰
//! （判据照旧——「能不能在无 GPU 环境写单测」），而它的两个消费者
//! （`zephyra-winit` / `zephyra-winit-d3d12` 的 `HostWindow`）都依赖 fx。
//!
//! # 为什么是行协议而不是 JSON
//!
//! 应用端保持**零依赖**（workspace 至今没有 serde）。JSON 的解析责任放在 CLI
//! 一侧（它反正要为 MCP 持有 `serde_json`）；桥内协议只需表达「读树 / 读属性 /
//! 注入点击 / 截图」这几件事，一行一命令的文本协议足够，而转义、浮点格式这些
//! JSON 坑就永远不会出现在热路径上。
//!
//! # 状态纪律（谁负责什么）
//!
//! - **桥**：连接管理（非阻塞 accept / 半行缓冲 / 写响应），命令的**解析**；
//! - **宿主**（[`BridgeHost`] 的实现者）：命令的**执行**——树 dump、属性读取、
//!   输入注入、截图。桥对 `Node` / `InputState` / GPU 一无所知。
//!
//! 宿主每帧调一次 [`DebugBridge::poll`]（帧循环开头、`frame()` 之前）：
//! 注入的 hover/click 在**同一帧**就参与命中测试与布局，AI 读回的树永远是
//! 操控后的状态。截图是唯一例外——它必须发生在「渲染后、present 前」
//! （flip model 下 present 之后后缓冲内容未定义），由宿主实现
//! [`BridgeHost::screenshot`] 时自行择机（D3D11 侧的做法见
//! `zephyra-winit` 的 `frame_captured`）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// 默认端口。`zephyra mcp` 与 `ZEPHYRA_DEBUG_PORT` 缺省都用它。
pub const DEFAULT_PORT: u16 = 7878;

/// 开启桥的环境变量名（值 = 端口）。
pub const PORT_ENV: &str = "ZEPHYRA_DEBUG_PORT";

/// 一条从 TCP 上解析出来的命令。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 探活。回 `pong`。
    Ping,
    /// 布局树 dump（缩进文本，路径列可直接复制给 `props`）。
    Tree,
    /// 指定节点的属性表。`path` 是树里的子下标链（`0.1.2`）。
    Props { path: Vec<usize> },
    /// 最近一帧的统计（quads / dirty / hovered）。
    Stats,
    /// 视口尺寸（物理像素）。
    Viewport,
    /// 注入指针移动（同物理鼠标移到该坐标）。
    Hover { x: f32, y: f32 },
    /// 注入一次完整点击：move → down → up（顺序在同一帧内完成）。
    Click { x: f32, y: f32 },
    /// 渲一帧并读回像素，回 BMP 二进制（见 [`Response::Binary`]）。
    Shot,
}

/// 一条命令的响应（写回 TCP 的形式）。
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// 多行文本（每行一条，不带换行；写出时统一补 `\n`）。
    Text(Vec<String>),
    /// 二进制载荷（目前只有 BMP）+ 一行说明。
    Binary { note: String, data: Vec<u8> },
    /// 失败。错误文案要能**直接指认原因**（路径不对 / 后端不支持），
    /// 与本仓「报错质量」的一贯要求一致。
    Err(String),
}

/// 宿主实现的执行面。桥只认识这个 trait，不认识窗口与 GPU。
pub trait BridgeHost {
    /// 视口尺寸（物理像素，宽，高）。
    fn viewport(&self) -> (f32, f32);

    /// 布局树 dump。**绝对**坐标（点击注入用的就是同一坐标系）。
    fn tree_dump(&self) -> String;

    /// 指定路径节点的属性表；路径无效返回 `None`。
    fn props_dump(&self, path: &[usize]) -> Option<String>;

    /// 最近一帧的统计。
    fn stats(&self) -> String;

    /// 注入指针移动。
    fn inject_hover(&mut self, x: f32, y: f32);

    /// 注入一次完整点击（move → down → up）。
    fn inject_click(&mut self, x: f32, y: f32);

    /// **立即**渲一帧并读回像素，返回 BMP 字节。后端不支持读回时返回 `None`。
    ///
    /// 「立即」的含义：不能等下一帧——AI 发 `shot` 时界面往往静止
    /// （`dirty == false`），等帧循环自己跑就永远等不到。
    fn screenshot(&mut self) -> Option<Vec<u8>>;
}

/// 调试桥。`from_env` 开启；每帧 [`Self::poll`]。
pub struct DebugBridge {
    listener: TcpListener,
    /// v1 单客户端：AI 客户端一次一个。第二个连接等第一个断开
    /// （AI 会话通常就是独占的；多客户端队列属 v2）。
    conn: Option<Connection>,
}

struct Connection {
    stream: TcpStream,
    /// 半行缓冲：非阻塞读到的字节先攒在这，凑齐一行才解析。
    buf: Vec<u8>,
}

impl DebugBridge {
    /// 绑定 `port` 并置非阻塞。绑定失败（端口被占）由调用方决定怎么报——
    /// 桥是调试设施，**不该**让应用起不来。
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            conn: None,
        })
    }

    /// 按环境变量开启桥：`ZEPHYRA_DEBUG_PORT=<port>`（或空串取默认端口）。
    /// 未设置返回 `None`——这是「调试桥默认关闭」的唯一入口，不存在隐式监听。
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var(PORT_ENV).ok()?;
        let port = if raw.is_empty() {
            DEFAULT_PORT
        } else {
            raw.parse().ok()?
        };
        match Self::bind(port) {
            Ok(b) => {
                eprintln!("[zephyra] 调试桥已开启 127.0.0.1:{port}（{PORT_ENV}={raw}）");
                Some(b)
            }
            Err(e) => {
                eprintln!("[zephyra] 调试桥绑定 {port} 失败（{e}）——无桥继续运行");
                None
            }
        }
    }

    /// 每帧一次：接受连接、读可用字节、把凑齐的行解析成命令并执行。
    ///
    /// 全程**非阻塞**：没有连接、没有数据、对端半行，都直接返回，
    /// 帧循环一微秒都不多等。
    pub fn poll(&mut self, host: &mut dyn BridgeHost) {
        // 只有一个连接槽：已有客户端在位时不 accept（新连接立即被关——
        // 但 v1 干脆不 accept，让它悬在 backlog 里等旧连接断开更省事）。
        if self.conn.is_none() {
            if let Ok((stream, _addr)) = self.listener.accept() {
                let _ = stream.set_nonblocking(true);
                let _ = stream.set_nodelay(true);
                self.conn = Some(Connection {
                    stream,
                    buf: Vec::new(),
                });
            }
        }

        let Some(conn) = &mut self.conn else {
            return;
        };

        // 读可用字节（非阻塞：WouldBlock = 本帧没有新命令）。
        let mut chunk = [0u8; 4096];
        loop {
            match conn.stream.read(&mut chunk) {
                Ok(0) => {
                    // 对端关了。丢弃缓冲（半行永远等不齐了）。
                    self.conn = None;
                    return;
                }
                Ok(n) => {
                    conn.buf.extend_from_slice(&chunk[..n]);
                    if conn.buf.len() > 64 * 1024 {
                        // 没有换行的洪泛：断开。调试桥不该被喂死。
                        self.conn = None;
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.conn = None;
                    return;
                }
            }
        }

        // 逐条处理凑齐的行。半行留在缓冲里等下一帧。
        loop {
            let Some(pos) = conn.buf.iter().position(|&b| b == b'\n') else {
                break;
            };
            let line: Vec<u8> = conn.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]); // 去 \n
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let resp = match parse_command(line) {
                Ok(cmd) => execute(&cmd, host),
                Err(e) => Response::Err(e),
            };
            if !write_response(&mut conn.stream, &resp) {
                self.conn = None;
                return;
            }
        }
    }

    /// 当前是否挂着客户端。
    ///
    /// 事件驱动的宿主（D3D12）拿它决定控制流：有客户端切 Poll（TCP 数据
    /// 唤不醒 Wait 模式），无客户端回 Wait（静止零开销不动）。
    pub fn has_client(&self) -> bool {
        self.conn.is_some()
    }
}

// ── 树 / 属性 dump：宿主侧共享实现 ─────────────────────────────────────────
//
// 两个后端（D3D11 / D3D12）都要回答 `tree` / `props`，而它们互不依赖——逻辑
// 只能有一份，落点就是这里（fx 依赖 core，`Node` 可见；两个宿主都依赖 fx）。
// 与 `zephyra_fx::quad` 的抽取同一论证：契约一致性靠「只有一份」保证，
// 不靠「记得同步两份」。

use std::fmt::Write as _;

/// 布局树 dump（`tree` 命令的正文）。
///
/// 每行 `path tag key absx,absy w*h [styles...]`：
///
/// - **path**（子下标链，如 `0.1.2`）是 `props` 命令的钥匙；
/// - 坐标已累成**绝对**值——`hover`/`click` 用的就是这一坐标系，
///   AI 从树里抄一个坐标就能点，不需要自己做相对/绝对换算；
/// - 缩进只为人读，解析靠 path 列。
pub fn tree_dump_of(root: &zephyra_core::node::Node) -> String {
    fn rec(node: &zephyra_core::node::Node, path: String, dx: f32, dy: f32, out: &mut String) {
        let r = node.arranged_rect();
        let (x, y) = (dx + r.x, dy + r.y);
        let key = match node.key() {
            Some(k) => format!("{k}"),
            None => "-".into(),
        };
        let _ = write!(
            out,
            "{path} {:?} {key} {x:.0},{y:.0} {:.0}x{:.0}",
            node.tag(),
            r.width,
            r.height
        );
        for name in node.style_names() {
            let _ = write!(out, " {name}");
        }
        let _ = writeln!(out);
        for (i, child) in node.children().iter().enumerate() {
            let depth = path.matches('.').count() + 1;
            for _ in 0..depth {
                out.push_str("  ");
            }
            rec(child, format!("{path}.{i}"), x, y, out);
        }
    }
    let mut out = String::new();
    rec(root, "0".into(), 0.0, 0.0, &mut out);
    out
}

/// 单节点属性表 dump（`props` 命令的正文）。
///
/// 先样式后本地——与优先级顺序相反（本地赢），读的人「后面盖前面」。
pub fn props_dump_of(node: &zephyra_core::node::Node) -> String {
    let r = node.arranged_rect();
    let mut s = String::new();
    let _ = writeln!(s, "tag {:?}", node.tag());
    if let Some(k) = node.key() {
        let _ = writeln!(s, "key {k}");
    }
    let _ = writeln!(
        s,
        "rect {:.0},{:.0} {:.0}x{:.0} (相对父)",
        r.x, r.y, r.width, r.height
    );
    for name in node.style_names() {
        let _ = writeln!(s, "style {name}");
    }
    for (k, v) in node.props().iter_styled() {
        let _ = writeln!(s, "{} = {} (styled)", k.name(), value_to_text(v));
    }
    for (k, v) in node.props().iter_local() {
        let _ = writeln!(s, "{} = {}", k.name(), value_to_text(v));
    }
    s
}

/// 解析一行命令。纯函数，单测钉在这里。
///
/// 协议（一行的形状）：
///
/// ```text
/// ping
/// tree
/// props 0.1.2
/// stats
/// viewport
/// hover 120.5 48
/// click 120.5 48
/// shot
/// ```
pub fn parse_command(line: &str) -> Result<Command, String> {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().ok_or("空命令")?;
    let rest: Vec<&str> = parts.collect();
    match cmd {
        "ping" => Ok(Command::Ping),
        "tree" => Ok(Command::Tree),
        "stats" => Ok(Command::Stats),
        "viewport" => Ok(Command::Viewport),
        "shot" => Ok(Command::Shot),
        "props" => {
            let Some(p) = rest.first() else {
                return Err("props 需要一个路径参数，如 `props 0.1.2`".into());
            };
            parse_path(p).map(|path| Command::Props { path })
        }
        "hover" | "click" => {
            if rest.len() != 2 {
                return Err(format!("{cmd} 需要 x y 两个坐标参数，如 `{cmd} 120 48`"));
            }
            let x: f32 = rest[0]
                .parse()
                .map_err(|_| format!("x 坐标 `{}` 不是数字", rest[0]))?;
            let y: f32 = rest[1]
                .parse()
                .map_err(|_| format!("y 坐标 `{}` 不是数字", rest[1]))?;
            if !(x.is_finite() && y.is_finite()) {
                return Err("坐标必须是有限数".into());
            }
            if cmd == "hover" {
                Ok(Command::Hover { x, y })
            } else {
                Ok(Command::Click { x, y })
            }
        }
        other => Err(format!(
            "未知命令 `{other}`；可用：ping tree props stats viewport hover click shot"
        )),
    }
}

/// 解析树路径 `0.1.2` → `[0, 1, 2]`。
fn parse_path(s: &str) -> Result<Vec<usize>, String> {
    s.split('.')
        .map(|seg| {
            seg.parse::<usize>()
                .map_err(|_| format!("路径段 `{seg}` 不是下标（形如 `0.1.2`）"))
        })
        .collect()
}

/// 执行命令。宿主方法各管各的，这里只做搬运。
fn execute(cmd: &Command, host: &mut dyn BridgeHost) -> Response {
    match cmd {
        Command::Ping => Response::Text(vec!["pong".into()]),
        Command::Tree => Response::Text(host.tree_dump().lines().map(str::to_string).collect()),
        Command::Props { path } => match host.props_dump(path) {
            Some(s) => Response::Text(s.lines().map(str::to_string).collect()),
            None => Response::Err(format!(
                "路径 {} 不存在（用 tree 查看当前形状；路径是子下标链，树里每行第一列）",
                path.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            )),
        },
        Command::Stats => Response::Text(host.stats().lines().map(str::to_string).collect()),
        Command::Viewport => {
            let (w, h) = host.viewport();
            Response::Text(vec![format!("{w} {h}")])
        }
        Command::Hover { x, y } => {
            host.inject_hover(*x, *y);
            Response::Text(vec!["hovered".into()])
        }
        Command::Click { x, y } => {
            host.inject_click(*x, *y);
            Response::Text(vec!["clicked".into()])
        }
        Command::Shot => match host.screenshot() {
            Some(data) => Response::Binary {
                note: "bmp 24bpp".into(),
                data,
            },
            None => Response::Err("此后端不支持像素读回".into()),
        },
    }
}

/// 把响应写回 TCP。
///
/// 文本响应：每行一条 + 终止行 `OK`。二进制：`BIN <len> <note>\n` + 原始字节
/// + `\nOK`。客户端读到 `OK` / 消费完 `BIN` 才算一条响应结束。
fn write_response(stream: &mut TcpStream, resp: &Response) -> bool {
    let mut head = String::new();
    match resp {
        Response::Text(lines) => {
            for l in lines {
                head.push_str(l);
                head.push('\n');
            }
            head.push_str("OK\n");
        }
        Response::Err(e) => {
            head.push_str("ERR ");
            head.push_str(e);
            head.push('\n');
        }
        Response::Binary { note, data } => {
            head.push_str(&format!("BIN {} {note}\n", data.len()));
        }
    }
    if stream.write_all(head.as_bytes()).is_err() {
        return false;
    }
    if let Response::Binary { data, .. } = resp {
        if stream.write_all(data).is_err() || stream.write_all(b"\nOK\n").is_err() {
            return false;
        }
    }
    stream.flush().is_ok()
}

/// 属性值 → 人读文本（`props` 命令用；后端 host 实现也复用它 dump 属性表）。
///
/// 放在桥里而不是给 [`zephyra_core::property::PropertyValue`] 实现 `Display`：
/// 这是**调试口径**（如颜色出十六进制、方向出 Debug 形式），不是值的规范文本，
/// 让它渗进核心 API 会让「这个格式是不是契约」变得模糊——它不是。
pub fn value_to_text(v: &zephyra_core::property::PropertyValue) -> String {
    use zephyra_core::property::PropertyValue;
    match v {
        PropertyValue::Float(f) => format!("{f}"),
        PropertyValue::Int(i) => format!("{i}"),
        PropertyValue::Bool(b) => format!("{b}"),
        PropertyValue::Str(s) => s.to_string(),
        PropertyValue::StaticStr(s) => (*s).to_string(),
        PropertyValue::Color(c) => {
            // 十六进制 `#RRGGBBAA`——AI 与人都能直接对到设计稿。通道走
            // `srgb_bytes`（诊断口径与 BMP 截图一致），alpha 直乘。
            let [r, g, b] = crate::pixels::srgb_bytes(*c);
            let a = (c.a * 255.0).round().clamp(0.0, 255.0) as u8;
            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
        }
        PropertyValue::Resource(k) => format!("${k}"),
        // 结构性枚举走 Debug——它们的名字本身就是词汇表（`Direction::Column`）。
        PropertyValue::Direction(v) => format!("{v:?}"),
        PropertyValue::CrossAlign(v) => format!("{v:?}"),
        PropertyValue::MainAlign(v) => format!("{v:?}"),
        PropertyValue::AlignContent(v) => format!("{v:?}"),
        PropertyValue::AlignSelf(v) => format!("{v:?}"),
        PropertyValue::FlexWrap(v) => format!("{v:?}"),
        PropertyValue::FlexDirection(v) => format!("{v:?}"),
        PropertyValue::Layer(v) => format!("{v:?}"),
        PropertyValue::Filter(v) => format!("{v:?}"),
        PropertyValue::BlendMode(v) => format!("{v:?}"),
        PropertyValue::UvRect(v) => format!("{v:?}"),
        // 类型擦除载荷没有稳定文本：只标存在性。真正要读它的宿主应该
        // 注册成字符串属性，而不是指望调试器看穿 `dyn Any`。
        PropertyValue::Custom { .. } => "<custom>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 协议解析：好路径全收，坏路径**每一条都带可指认原因**。
    #[test]
    fn parses_known_commands() {
        assert_eq!(parse_command("ping"), Ok(Command::Ping));
        assert_eq!(parse_command("tree"), Ok(Command::Tree));
        assert_eq!(parse_command("stats"), Ok(Command::Stats));
        assert_eq!(parse_command("shot"), Ok(Command::Shot));
        assert_eq!(
            parse_command("props 0.1.2"),
            Ok(Command::Props {
                path: vec![0, 1, 2]
            })
        );
        assert_eq!(
            parse_command("hover 120.5 48"),
            Ok(Command::Hover { x: 120.5, y: 48.0 })
        );
        assert_eq!(
            parse_command("click 10 20"),
            Ok(Command::Click { x: 10.0, y: 20.0 })
        );
    }

    #[test]
    fn rejects_bad_commands_with_a_reason() {
        for (line, why) in [
            ("", "空命令"),
            ("nope", "未知命令"),
            ("props", "缺路径"),
            ("props x.y", "路径段不是下标"),
            ("hover 1", "缺一个坐标"),
            ("hover a b", "坐标不是数字"),
            ("hover nan 0", "非有限数"),
        ] {
            let err = parse_command(line).unwrap_err();
            assert!(!err.is_empty(), "`{line}`（{why}）的错误文案不能是空的");
        }
    }

    /// `props` 的路径下标从 0 起——`0.0` 是合法的最深起点写法。
    #[test]
    fn path_is_zero_based() {
        assert_eq!(
            parse_command("props 0.0"),
            Ok(Command::Props { path: vec![0, 0] })
        );
    }

    // ── TCP 集成：真实 socket 上走一轮 ──────────────────────────────

    /// 记录注入调用的 mock 宿主。树/属性是固定文本——这里测的是**桥**，
    /// 不是宿主的 dump 质量（后者在后端的集成测试里）。
    struct MockHost {
        clicks: Vec<(f32, f32)>,
        hovers: Vec<(f32, f32)>,
        shot_bytes: Option<Vec<u8>>,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                clicks: Vec::new(),
                hovers: Vec::new(),
                shot_bytes: None,
            }
        }
    }

    impl BridgeHost for MockHost {
        fn viewport(&self) -> (f32, f32) {
            (800.0, 600.0)
        }
        fn tree_dump(&self) -> String {
            "0 Container root 0,0 800x600\n  0.0 Text title 0,0 120x24".into()
        }
        fn props_dump(&self, path: &[usize]) -> Option<String> {
            match path {
                [0] => Some("tag Container\nkey root".into()),
                _ => None,
            }
        }
        fn stats(&self) -> String {
            "quads=2".into()
        }
        fn inject_hover(&mut self, x: f32, y: f32) {
            self.hovers.push((x, y));
        }
        fn inject_click(&mut self, x: f32, y: f32) {
            self.clicks.push((x, y));
        }
        fn screenshot(&mut self) -> Option<Vec<u8>> {
            self.shot_bytes.clone()
        }
    }

    /// 客户端读一条响应：读到 `OK` 行或 `ERR` 行为止（`BIN` 时先消费载荷）。
    ///
    /// ⚠ 终止条件**必须**包含 ERR：ERR 行后面没有 `OK`（ERR 本身就是终止行）——
    /// 漏掉这条的版本在这里**死等**，测试直接挂死（实测）。
    fn read_response(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            // 一字节一读在测试里够慢也够对；不需要行缓冲的复杂度。
            stream.read_exact(&mut byte).unwrap();
            buf.push(byte[0]);
            let s = String::from_utf8_lossy(&buf);
            if s.ends_with("\nOK\n") {
                // `BIN` 响应的头部之后还有载荷 + 尾部 `\nOK\n`——上面的判断
                // 只在「载荷读完后」才成立，正好是我们要的。
                return s.into_owned();
            }
            // ERR 是单行终止：换行即完整。
            if s.starts_with("ERR ") && s.ends_with('\n') {
                return s.into_owned();
            }
            if s.starts_with("BIN ") {
                // 头行是 `BIN <len> <note>\n`——note 变长，头行以**首个换行**结束；
                // 之后是 len 字节载荷 + `\nOK\n`。
                if let Some(hend) = s.find('\n') {
                    let rest = &s[4..hend];
                    if let Some(sp) = rest.find(' ') {
                        if let Ok(len) = rest[..sp].parse::<usize>() {
                            let need = hend + 1 + len + "\nOK\n".len();
                            if buf.len() >= need {
                                return String::from_utf8_lossy(&buf).into_owned();
                            }
                        }
                    }
                }
            }
        }
    }

    /// 绑定随机端口，返回（桥，端口）。
    fn bridge_on_ephemeral_port() -> (DebugBridge, u16) {
        // :0 由 OS 分配——测试不抢固定端口。
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        (
            DebugBridge {
                listener,
                conn: None,
            },
            port,
        )
    }

    #[test]
    fn full_round_trip_over_tcp() {
        let (mut bridge, port) = bridge_on_ephemeral_port();
        let mut host = MockHost::new();
        host.shot_bytes = Some(b"BMP!".to_vec());

        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_nonblocking(false).unwrap();
        // accept 发生在 poll 里，先 poll 一次把连接收进来。
        bridge.poll(&mut host);
        assert!(bridge.conn.is_some(), "poll 应当接受待处理的连接");

        // 文本命令。⚠ 逐条发收：桥在一次 poll 里会把凑齐的多条命令**连续**
        // 回包，客户端按「读到 OK 为止」切分时无法回头再切——协议是逐条的，
        // 测试也逐条走。
        for (cmd, expect) in [
            ("ping", "pong"),
            ("tree", "0.0 Text title"),
            ("viewport", "800 600"),
            ("stats", "quads=2"),
        ] {
            client.write_all(cmd.as_bytes()).unwrap();
            client.write_all(b"\n").unwrap();
            bridge.poll(&mut host);
            let r = read_response(&mut client);
            assert!(r.contains(expect), "`{cmd}` 的响应缺 `{expect}`：{r}");
            assert!(r.ends_with("OK\n"), "文本响应以 OK 终止：{r}");
        }

        // 注入命令到达宿主（各自一轮，响应都要读走——否则残留会污染下轮切分）。
        client.write_all(b"click 33 44\n").unwrap();
        bridge.poll(&mut host);
        assert!(read_response(&mut client).contains("clicked"));
        client.write_all(b"hover 1 2\n").unwrap();
        bridge.poll(&mut host);
        assert!(read_response(&mut client).contains("hovered"));
        assert_eq!(host.clicks, vec![(33.0, 44.0)]);
        assert_eq!(host.hovers, vec![(1.0, 2.0)]);

        // 无效路径得到 ERR，且连接不断（下一条命令还能走）。
        client.write_all(b"props 9.9\n").unwrap();
        bridge.poll(&mut host);
        let r = read_response(&mut client);
        assert!(r.starts_with("ERR "), "{r}");
        client.write_all(b"tree\n").unwrap();
        bridge.poll(&mut host);
        let r = read_response(&mut client);
        assert!(r.contains("Text title"), "{r}");

        // 二进制响应。
        client.write_all(b"shot\n").unwrap();
        bridge.poll(&mut host);
        let r = read_response(&mut client);
        assert!(r.contains("BIN 4 bmp 24bpp"), "{r}");
        assert!(r.ends_with("BMP!\nOK\n"), "{r}");
    }

    #[test]
    fn half_line_waits_for_the_rest() {
        let (mut bridge, port) = bridge_on_ephemeral_port();
        let mut host = MockHost::new();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_nonblocking(false).unwrap();
        bridge.poll(&mut host);

        // 半行：桥不该把它当命令，也不该回任何东西。
        client.write_all(b"pi").unwrap();
        bridge.poll(&mut host);
        client.set_nonblocking(true).unwrap();
        let mut byte = [0u8; 1];
        assert!(client.read(&mut byte).unwrap_or(0) == 0, "半行不该产生响应");

        // 补上后半行：凑齐了才执行。
        client.set_nonblocking(false).unwrap();
        client.write_all(b"ng\n").unwrap();
        bridge.poll(&mut host);
        assert!(read_response(&mut client).contains("pong"));
    }

    #[test]
    fn client_disconnect_is_recovered_from() {
        let (mut bridge, port) = bridge_on_ephemeral_port();
        let mut host = MockHost::new();
        {
            let c = TcpStream::connect(("127.0.0.1", port)).unwrap();
            c.set_nonblocking(false).unwrap();
            bridge.poll(&mut host);
            assert!(bridge.conn.is_some());
            drop(c);
        }
        // 第一次 poll 读到 0 字节 → 清连接；第二次 poll → 重新 accept 下一个。
        bridge.poll(&mut host);
        assert!(bridge.conn.is_none(), "对端关闭后连接槽应当清空");
        let _c2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
        bridge.poll(&mut host);
        assert!(bridge.conn.is_some(), "新客户端应当能接上");
    }
}
