# 2026-10-25-mcp-debug-bridge

> 需求：改造 CLI，添加 MCP 功能——允许 AI 通过 MCP 读取布局、操控界面，
> 深度融合 AI 能力。

## 设计：三层链路，依赖各归其位

```text
AI 客户端 ──stdio·JSON-RPC── zephyra mcp（CLI）──TCP·行协议── 运行中的应用（fx::bridge）
```

- **库层 `zephyra-fx::bridge`（纯 std，零新依赖）**：非阻塞 `TcpListener` +
  行协议（ping/tree/props/stats/viewport/hover/click/shot）+ `BridgeHost`
  trait。桥只管「管道与解析」，命令的**执行**全在宿主——桥不认识
  `Node`/`InputState`/GPU。环境变量 `ZEPHYRA_DEBUG_PORT` 开启；不设则零监听。
- **CLI `zephyra mcp`（唯一持 serde_json 的一层）**：MCP stdio server
  （newline-delimited JSON-RPC 2.0：initialize / tools/list / tools/call），
  翻译工具调用 ↔ 桥命令。stdout 纪律：只准出协议消息，诊断全走 stderr。
- **宿主接线（D3D11 / D3D12 同构）**：`HostWindow` 实现 `BridgeHost`；
  D3D11 poll 挂 `RedrawRequested` 帧首（常驻重绘循环天然覆盖），D3D12 挂
  `about_to_wait` 且**有客户端切 Poll、无客户端回 Wait**（事件驱动后端
  TCP 数据唤不醒 Wait，但静止零开销的卖点不能丢）。

## 三个关键决定

1. **操控 = 输入注入，不是属性直改**。保留树每帧由 `View::view()` 重建，
   外部改节点属性会被下一帧 diff 覆盖——直改是错误的语义。合成事件走
   `on_cursor_moved` / `on_pointer_down` / `on_pointer_up` **同一条**物理
   输入路径（`window_event` 的 Released 分支为此提取成方法），命中测试/
   事件路由/脏标记零分叉。
2. **树 dump 出绝对坐标**。`Node::arranged_rect` 是相对父的（补了只读
   访问器），dump 时逐级累成绝对值——AI 从树里抄一个坐标就能 `click`，
   不需要懂相对/绝对换算。路径用**子下标链**（`0.1.2`）：无歧义、零发明
   （不用节点 id/key 当身份）。
3. **截图钉在 present 前**。flip model 下 present 后读回全零（本仓
   dev-log 早有实测表）。桥命令在帧首到达，截图由
   `frame_inner(capture)` 强制渲一帧并在 present 前读回——不依赖
   「界面正好在动」。D3D12 后缓冲读回未接线，v1 诚实降级返回 ERR。

## 实现

| 层 | 内容 |
|---|---|
| fx | `bridge.rs`（DebugBridge/BridgeHost/Command/Response + tree_dump_of/props_dump_of 共享 dump）+ `pixels::bmp_bytes`（从 write_bmp 提取的纯编码函数） |
| core | `Node::arranged_rect()` 只读访问器；`PropertyStore::iter_styled()`（对齐 iter_local） |
| D3D11 | `frame_inner(capture)` 拆分 + `on_pointer_up` 提取 + `BridgeHost` 实现（截图走 `Surface::read_pixels`→BMP） |
| D3D12 | 同构 BridgeHost（截图诚实降级）+ about_to_wait 联动控制流 |
| CLI | `mcp.rs`（BridgeClient TCP 切分 + MCP 消息处理 + 7 工具）+ serde_json（显式破例，注释写明理由） |

工具表：get_tree / get_props / get_stats / get_viewport / hover / click /
screenshot。

## 踩坑记录

- **TCP 测试挂死两次**：① 切分器只认 `OK`/`BIN` 终止——ERR 响应没有
  终止符，`read_exact` 永等；② BIN 的长度计算漏了变长 note 字段。
  教训同款两次：**读端必须按写端的全部终止形态切分**，ERR 也是终止行。
- `json!` 进不了 `static`（E0015）→ 工具 schema 存静态文本运行时 parse。
- `handle_message` 的闭包注入与 `&mut client` 双借用冲突 → 连接管理收敛
  进 `ensure_client`，测试改用真 TCP mock（比假闭包多测到懒连接/重连）。
- D3D11 的帧是 `RedrawRequested` 自循环（Poll 式常转），D3D12 是纯事件
  驱动——同一个桥两种宿主节奏，控制流联动（has_client → Poll/Wait）
  是 D3D12 侧独有的必要复杂度。

## 验收

- fx 131（+6：协议解析好/坏路径、TCP 全往返、半行等待、断线恢复）
- cli 9+21（+8：MCP 消息形状、通知静默、工具表齐全、命令翻译、坐标
  透传、桥错误→isError、坏 JSON→-32700、不可达→指认 ZEPHYRA_DEBUG_PORT）
- core 341 / D3D11 draw 34 / D3D12 draw 18 全绿；clippy 零 lint。
- 端到端（应用真跑 + AI 操控）挂真机轨道：`ZEPHYRA_DEBUG_PORT=7878` 起任一
  demo，MCP 配置见 `ZephyraCLI/README.md`。
