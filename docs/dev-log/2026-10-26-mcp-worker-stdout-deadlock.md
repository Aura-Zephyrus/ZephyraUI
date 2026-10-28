# 2026-10-26-mcp-worker-stdout-deadlock

> 现象：`zephyra mcp` 的单元测试 50/50 全绿，真机 E2E 里 `initialize` /
> `tools/list` / JSON-RPC `ping`（主线程就地处理）全部秒回，但**每一个**
> `tools/call`（投给工具工作线程）都永远不应答——没有 stdout 回执、没有
> stderr 报错、进程存活。重连 / 读超时 / 取消那一轮修复全部到位，`tools/call`
> 依旧一条都不通。

## 排查

1. **先怀疑桥读路径**。mock 桥隔离（`TcpListener` 在 PowerShell `Start-Job` 里，
   端口 7993）：CLI 连上、发出 `viewport`，mock 60ms 内回 `1400 800\nOK\n`，
   CLI 仍然 26 秒无输出——连 10 秒读超时应在 ~21 秒（超时 10s + 重连重试一次）
   冒出的错误都没有。
2. **插桩定位**。给 `call` / `read_response` / 工作线程循环 / `write_reply` 加了
   临时 `eprintln!`（stderr）。关键观察：主线程的 `[DBG]` 行（`write_reply` 的
   ping 回执、路由日志）都正常出现，但**工作线程的 `[DBG] worker thread started`
   一条都没有**——不是收不到消息，是它连第一条语句都没执行。
3. **对照排除「进程提前退出 / spawn 失败」**：进程启动后 2 秒仍能应答 `ping`
   （主线程 stdin 循环还在跑），所以 `run()` 没有因 `spawn` 出错提前返回，线程
   确实被创建了。
4. **读 `entry.rs`**：`main_entry` 在 `dispatch()` 之前 `stdout.lock()` 和
   `stderr.lock()` 各拿一个守卫，并**在整个 dispatch（= `mcp::run`，长驻）期间
   持有**。工作线程的第一次 `eprintln!`（写 stderr）就要拿 stderr 的锁——被主线程
   持着，永久阻塞，`for line in job_rx` 循环根本没开始。主线程自己的
   `eprintln!`/`write_reply` 之所以正常，是因为同线程重入这把锁（Rust stdout/
   stderr 用可重入互斥）。

## 排除的假设

- **「`read_response` 有 `\r\n` 行尾 bug」**——桥与 mock 都发 `\n`；而且工作线程
  从未启动，压根没走到读。被「worker 第一条语句都没跑」的现象直接排除。
- **「二进制是旧的（stale binary）」**——用字节搜索确认新二进制里含
  `[DBG] worker thread started` 等新串。
- **「工作线程静默 panic」**——全仓 grep `set_hook`/`take_hook` 无一命中（没有
  压制 panic 输出的钩子），panic 会打到 stderr（已被捕获），而 stderr 干净。
- **「`Builder::spawn` 失败、`run()` 提前返回 Err」**——进程启动 2 秒后仍能应答
  `ping`，说明 `run()` 还在 stdin 循环里，spawn 成功了。
- **「channel 断了 / `job_tx.send` 失败」**——主线程打印了 `job_tx.send ok`
  （通道有缓冲，send 立即成功）；问题在接收端从未启动循环。
- **「10 秒读超时没生效」**——超时确实设了（`connect` 里 `set_read_timeout(10s)`），
  但工作线程死锁在读之前，超时无从触发。

## 结论

`entry.rs` 在 `dispatch()` 全程握着 stdout + stderr 两把全局锁，与任何「长驻 +
子线程写 stdio」的命令不兼容；MCP 的工具工作线程是第一个踩到的。修法：`main_entry`
改拿**未加锁**的 `Stdout`/`Stderr` 句柄——它们的 `Write` 实现在每次 `write` 内部
短暂加锁、写完即放。单线程命令行为不变（没有别的线程可交错）；MCP 工作线程的
`write_reply`（`stdout.lock()` 行级原子）与 `eprintln!` 都能在两次调用之间拿到锁。
`write_reply` 的「两个线程都可能调、靠 `Stdout` 内部锁保证整行原子」的设计前提，
由此才真正成立。

## 门禁

- `cargo test -p zephyra-cli`：50 条（28 lib + 22 cli），0 失败。
- mock 桥 E2E（端口 7993）：`tools/call get_viewport` 回
  `{"id":3,...,"text":"1400 800"}`（此前 26 秒无输出）。
- 真机 E2E（gallery D3D12，`ZEPHYRA_DEBUG_PORT=7878`）：`initialize` 握手 +
  `tools/list`（14 工具含 annotations）+ `ping`/`get_viewport`/`get_stats`/
  `get_tree` 全通；杀掉 gallery、重启后**同一 MCP 会话** `ping` 自动重连回
  `pong`（`with_bridge` 丢死连接→重连→重试，进程被杀走 RST 快速失败而非 10 秒
  超时）。
- 四条门禁：fmt exit 0、clippy 0 警告、`cargo test --workspace` 970 通过 /
  0 失败 / 51 ignored（82 suites）、`cargo doc --no-deps --workspace` exit 0
  （42 条警告全部落在既有文件，`host.rs:160` 是 `SwapState`→`sync_swapchain`
  的历史 intra-doc 链接，与本轮改动无关）。
