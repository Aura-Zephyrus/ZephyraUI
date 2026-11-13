//! 调试桥客户端：一条到运行中应用的 TCP 连接 + 协议响应切分。
//!
//! # 为什么独立成模块
//!
//! `mcp`（给 AI）与 `bridge`（给人 / 脚本）两个子命令共用**同一份协议读者**
//! —— 协议只有一份实现，改一处不会漏另一处。这与库里「D3D11 / D3D12 两个
//! 后端只共同依赖 core + fx」的论证是同一条：跨后端/跨入口共享的那一半，
//! 只能有一份。
//!
//! 协议细节见 `zephyra-fx::bridge`（库侧）。这里只做「客户端视角」的事：
//! 连上、发一行、切一条响应回来。

use std::io::{Read, Write};

/// 桥客户端：一条到运行中应用的 TCP 连接 + 协议响应切分。
pub(crate) struct BridgeClient {
    stream: std::net::TcpStream,
    /// 半行/跨读缓冲——与桥那边对称：响应按「读到终止符」切分。
    buf: Vec<u8>,
}

/// 一条桥响应（与 `zephyra_fx::bridge::Response` 同构，但 CLI 侧只看文本形状）。
pub(crate) enum BridgeResponse {
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
    /// 用一条**已建立**的连接构造。
    ///
    /// 存在的理由：测试要拿**替身服务器**给的流（而不是真的去连一个端口），
    /// 而字段是私有的。没有这个入口，测试就只能靠「真的起一个监听端口」——
    /// 那是把协议测试写成网络测试，端口与时序都会变成 flake 的来源。
    ///
    /// ⚠ 只在测试里用：生产路径一律走 [`Self::connect`]（它带重试与读超时）。
    #[cfg(test)]
    pub(crate) fn from_stream(stream: std::net::TcpStream) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    /// 连接应用调试桥。`tries` 次、每次间隔 500ms——应用刚启动时端口可能
    /// 还没监听（AI 客户端往往先配好 MCP 再起应用）。
    pub(crate) fn connect(host: &str, port: u16, tries: usize) -> std::io::Result<Self> {
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
    ///
    /// ⚠ `sleep <ms>` 是**客户端侧**的伪命令：应用不需要知道它（按墙钟等
    /// 是客户端的事，而按帧等有专门的 `wait_frames`）。这里就地睡掉，省得
    /// 为了等一会儿去起第二个进程。
    pub(crate) fn call(&mut self, cmd: &str) -> Result<BridgeResponse, String> {
        if let Some(rest) = cmd.strip_prefix("sleep ") {
            let ms: u64 = rest
                .trim()
                .parse()
                .map_err(|_| format!("sleep 的毫秒数 `{rest}` 不是整数"))?;
            if ms > 60_000 {
                return Err("sleep 最多 60000 毫秒".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(ms));
            return Ok(BridgeResponse::Text(vec![format!("slept {ms} ms")]));
        }
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
    pub(crate) fn read_response(&mut self) -> Result<BridgeResponse, String> {
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
