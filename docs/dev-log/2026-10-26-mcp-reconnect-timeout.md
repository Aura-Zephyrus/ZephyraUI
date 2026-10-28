# 2026-10-26-mcp-reconnect-timeout

> 现象：应用重启后 MCP 会话永久报废（每个工具调用都对着死连接报「桥连接已断开」），
> 冻结的应用会挂死整个 MCP 进程，AI 无法中止一次 300 秒的性能采集。

## 排查

对 gallery（D3D12，1400×800，`ZEPHYRA_DEBUG_PORT=7878`）做了一轮真机 E2E：13 个工具
全部走通（`get_viewport` / `get_stats` / `get_layout` / `get_tree` / `get_props` /
`click` / `hover` / `scroll` / `press_key` / `get_resources` / `screenshot` 3360054 字节
BMP / `pointer_drag` 6 采样 / `analyze_performance` 2 秒 155 帧）。通了之后逐条压
可靠性，发现四类问题：

1. **死连接不刷新**。`Session`（旧版是裸 `Option<BridgeClient>`）把连接缓存在会话里，
   读失败后却从不丢弃——旧注释写着「连接失败重连一次再试」，实现里没有那一句。
   应用重启（dev 工作流的常态）之后每次 `tools/call` 都对着同一条死流报错，直到
   MCP host 杀掉进程重配。修法：`with_bridge` 包装——传输失败即 `drop_client`、
   重连、重试一次；多命令手势 `pointer_drag` 不整段重试（按下态可能已建立），
   只丢连接让下次调用重连。
2. **没有读超时**。桥是同步阻塞读；应用一旦冻结（调试断点、设备挂起），MCP 进程
   跟着无限期挂死。修法：`set_read_timeout(10s)`——冻结应用最多拖慢一次调用。
3. **stdio 线程模型**。旧版 stdin 线程串行处理所有消息，一个 300 秒的
   `analyze_performance` 会挡住 `tools/list` / `ping`，客户端只能超时。修法：
   `tools/call` 投给唯一工作线程（串行，保 AI「读-改-读」次序），其余消息就地
   处理；stdout 行级原子写（`Stdout` 锁内 writeln+flush）防两条应答交错。
   ⚠ 这套线程模型在真机上**先不工作**：`entry.rs` 在 `dispatch()` 全程握着
   stdout+stderr 锁，工作线程第一次 `eprintln!` 即死锁——根因与修法见
   `2026-10-26-mcp-worker-stdout-deadlock.md`。配套加了 `notifications/cancelled`：
   排队中的调用开跑前跳过，在途采集在轮询间隙中止；被取消的请求按协议不回。
4. **上下文洪水**。`analyze_performance` 把逐帧行整段返回：3000 行 ≈ 120KB
   （实测 gallery 2 秒 155 帧就有 155 行）。修法：默认只回汇总块，逐帧数据要
   `include_rows=true` 显式索取；回执里提示怎么取全量。

顺带修的：`tools/call` 缺 `params.name` 时原来落进 `unknown tool` 兜底，报错指不到
真因；坐标参数（`hover`/`click`/`scroll`）没做 f32 范围校验，1e300 会在桥侧被解析成
inf 再被拒，错误离原因更远；`screenshot` 回执给相对路径（客户端不知道本进程 cwd），
现回绝对路径；工具表补 MCP `annotations`（`readOnlyHint`/`destructiveHint`）与
`ping` 工具（桥探应用活性，与协议层 `ping` 探 server 进程互补）。

应用侧：桥新增 `BridgeHost::on_bridge_disconnect` 钩子（`poll` 拆出 `poll_inner`，
外层统一判断「上一帧还有、这一帧没了」，体内多个 `return` 不会漏分支）。两个后端
各加 `bridge_pointer_pressed` 标志——D3D11 的 `on_pointer_up` 不清 `pointer` 字段，
不能拿路由层指针 id 当「按没按住」的判据。断连时补一次抬起：AI 在按下与抬起之间
杀掉 MCP server（超时/崩溃/会话取消）后，应用不再卡在拖动态。

## 排除的假设

- 「重试会注入两次输入，所以不能重试」——读超时要先冻结 10 秒，那时重试同样不会
  成功；而一个永久死掉的会话比偶发重复点击糟得多。权衡后普通工具重试一次，
  `pointer_drag` 因按下态可能已建立而不整段重试。
- 「取消表用 Vec + 线性扫描太慢」——取消是低频事件（一次采集最多一条），HashSet
  的复杂度优势不值得引入额外依赖；id 存 JSON 文本（`id.to_string()`）让数字/
  字符串两种 id 都对得上。
- 「断连钩子可以直接放在 `poll` 的每个 `self.conn = None` 处」——体内有 5 个置空点
  （flush 失败 ×2、对端关闭、洪泛、读错误），逐个加必然漏；拆 `poll_inner` 后
  外层一次判断覆盖全部路径。
- 「annotations 用运行时 `json!` 构造」——`json!` 展开非常量调用，进不了
  `static TOOLS`；改为静态 JSON 文本 + 运行时 parse（与 schema 同一模式）。

## 结论

MCP server 从「能跑通 demo」变成「能扛 dev 工作流」：应用重启自动恢复、冻结应用
有界阻塞、采集可取消、上下文占用可控。fx 与两个后端的断连收尾把「拖动中途 MCP
被杀」从永久卡死降级为一帧内的状态修正。

## 门禁

- `cargo test -p zephyra-cli`：50 条（28 lib + 22 cli），含重连重试、取消跳过、
  取消中止在途采集、`include_rows` 双态、绝对路径回执、annotations 只读/改状态
  分类、`tools/call` 缺 name、1e300 坐标拒绝。
- `cargo test -p zephyra-fx --lib bridge`：13 条，含
  `disconnect_notifies_host_once_and_then_stays_quiet`（钩子只在消失那一帧调一次，
  新一轮断连再调一次）。
- 真机 E2E：gallery D3D12 1400×800 上 13 工具全通 + 应用重启后同一 MCP 会话
  工具调用自动恢复。
