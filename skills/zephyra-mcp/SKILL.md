---
name: zephyra-mcp
description: 'Operate and inspect a running ZephyraUI application through the ZephyraUI MCP server. This skill should be used when reading UI layout or node properties of a running ZephyraUI app, driving it with hover / click / key / scroll / drag input, capturing screenshots, collecting frame performance profiles or resource telemetry, or troubleshooting the ZephyraUI MCP connection.'
argument-hint: 'Describe the running ZephyraUI app and the interaction or diagnostics you need'
---

# ZephyraUI MCP 操作

通过 ZephyraUI MCP server 检查并操作**用户授权的、本机运行中的 ZephyraUI 窗口**。

默认用文本布局理解界面，只有在需要像素证据或文本无法判断时才截图。性能采集按需、限时开启。所有结论必须有读回状态佐证。

## 何时用 / 何时不用

| 用 | 不用 |
|---|---|
| 读布局树、节点属性、视口、最近一帧统计 | 读源码语义 —— 那是 `docs/developer/` 与 `docs/dev-log/` 的事 |
| hover / click / 按键 / 滚轮 / 按住拖动 | 在应用**没启动**时操作 —— 没有进程就没有桥 |
| 截图取像素证据 | 把截图当布局的替代品（截图无结构、无路径、无属性） |
| 帧 CPU 生成耗时、进程 CPU/内存/GPU 遥测 | 断言 GPU 执行耗时（本工具不是 GPU timestamp profiler） |

## 链路

```text
MCP 客户端 ──stdio·JSON-RPC── zephyra mcp（CLI）──TCP·行协议── 运行中的 ZephyraUI 应用
```

三层各司其职：应用侧桥生成文本与执行注入；CLI 只做翻译并持有唯一桥连接；客户端只见 MCP 工具。

### 1. 应用侧：开启调试桥

库层调试桥**默认关闭**，只有设置了 `ZEPHYRA_DEBUG_PORT` 才监听；未设置时零监听，不存在隐式端口。

```bash
# Windows PowerShell
$env:ZEPHYRA_DEBUG_PORT = "7878"
cargo run -p zephyra-gallery --bin gallery

# bash / Linux
ZEPHYRA_DEBUG_PORT=7878 cargo run -p zephyra-gallery --bin gallery
```

- 取值是端口号；**空字符串取默认端口 7878**（`zephyra-fx::bridge` 的 `PORT_ENV` / `DEFAULT_PORT`）。
- 端口被占时桥绑定失败，**应用照常运行**（stderr 打印绑定失败，无桥继续运行）—— 端口冲突不会让应用起不来。
- 启动成功的标志是 stderr 出现 `[zephyra] 调试桥已开启 127.0.0.1:7878`。
- 后端切换走工程现有的 Cargo feature，不要同时启用互斥后端：`zephyra-gallery` 默认 `d3d12`，`--no-default-features --features d3d11` 切 D3D11，`--features linux-vk` 切 Vulkan。
- `zephyra dev` / `zephyra run` 会 `Command::new(cargo)` 起子进程并**继承环境变量**，所以先 `export ZEPHYRA_DEBUG_PORT` 再 `zephyra dev` 同样开桥。

### 2. 客户端侧：配置 MCP server

MCP host 以 stdio 启动 CLI：

```json
{
  "mcpServers": {
    "zephyra": { "command": "zephyra", "args": ["mcp"] }
  }
}
```

- CLI 二进制名是 `zephyra`（另有 `cargo-zephyra`）。不在 `PATH` 时把 `command` 换成可执行文件完整路径，例如仓库内的 `target/release/zephyra.exe`。
- `zephyra mcp [--port 7878] [--host 127.0.0.1]`。`--port` **默认恒为 7878**，不读 `ZEPHYRA_DEBUG_PORT`—— 应用改了端口，CLI 侧必须显式 `--port` 跟上。`--host` 默认 `127.0.0.1`。
- CLI 的 stdout **只**输出 JSON-RPC 协议消息，诊断一律走 stderr；`mcp` 启动后 stderr 首行是 `[zephyra-mcp] ready (bridge 127.0.0.1:7878); stdout is protocol-only`。
- 握手（`initialize` / `tools/list`）**不需要**应用在线，连接在第一次 `tools/call` 时才建立（重试 3 次 × 500 ms）。应用后启动是常态。
- 协议版本 `2024-11-05`。

## 标准工作流

**读 → 操作 → 读**。所有工具都是这三步里的一步，工具之间没有隐式状态。

### 读

1. `get_layout` 一次拿到三样东西：视口（物理像素）、最近一帧统计、布局树。
2. 从布局树里挑目标：路径（`get_props` 的钥匙）、绝对矩形、tag、key、样式分类名。
3. 目标矩形无效（零宽高）、不可见、被遮挡时不要盲点——先换目标或换页面。
4. 需要完整属性再 `get_props`，路径必须来自最新一次 `get_tree` / `get_layout` 的返回值。

### 操作

对单个目标做最小必要动作，**一次一个**，等工具返回再决定下一步。

- `hover` / `click`：坐标取自最新布局的绝对矩形内部（远离边缘）。
- `press_key`：单个逻辑键名，见下方「能力边界」。
- `scroll`：在目标位置注入滚轮增量。
- `pointer_drag`：一次**按下—按住—移动—抬起**的连续手势，不是多个 click。

### 验证

操作后重新读状态：`get_layout`（看树变化）、`get_props`（看该节点属性变化）、`get_stats`（看 hovered / dirty）。

- 状态可能异步更新时，隔一拍再读一次；不要靠快速重复点击碰运气。
- 桥响应成功 **≠** 应用行为正确。`click` 回 `clicked` 只说明事件注入了，不说明控件状态变了。

## 工具速查

| 工具 | 参数 | 用途 |
|---|---|---|
| `get_layout` | 无 | 文本布局全量：视口 + 帧统计 + 树 |
| `get_tree` | 无 | 只要布局树 |
| `get_props` | `path` | 单节点全部属性与样式值 |
| `get_stats` | 无 | 最近一帧统计 |
| `get_viewport` | 无 | 视口物理像素尺寸 |
| `get_resources` | 无 | 进程 CPU / 内存 / GPU 遥测 |
| `ping` | 无 | 探**应用**是否在线（区别于 MCP 协议层 ping 探 server 进程） |
| `hover` | `x`, `y` | 移动指针 |
| `click` | `x`, `y` | 完整点击（move → down → up） |
| `press_key` | `key` | 单个逻辑键按下/抬起 |
| `scroll` | `x`, `y`, `delta_x`, `delta_y` | 定位滚轮注入 |
| `pointer_drag` | `mode` + 参数 | 按住拖动 |
| `analyze_performance` | `duration_seconds`，可选 `include_rows` | 有界逐帧 CPU 报告 |
| `screenshot` | 可选 `path` | 24bpp BMP（D3D11 / D3D12） |

完整参数、返回样例见 `references/tools.md`；各工具返回文本的确切格式见 `references/output-formats.md`。

## 坐标系与路径

- `get_layout` / `get_tree` / `hover` / `click` / `scroll` / `pointer_drag` 共用**窗口物理像素坐标系**，节点矩形是**绝对坐标**。从树里抄一个坐标就能直接点。
- **不要**累加父级偏移，**不要**把截图上的缩放坐标直接传进去。
- `get_props` 的 `rect` 行同样以 `(absolute)` 标注，是绝对坐标——MCP 工具自身描述里写的 "relative to parent" 与实现不符，**以返回文本里的 `(absolute)` 为准**（`zephyra-fx::bridge::props_dump_of` 打印的就是绝对矩形）。
- 树路径首段 `0` 是**根标记**，其余段是逐层子下标（`0.1.2` = 根的第 2 个子节点的第 3 个子节点）。`get_props` 按根标记解析路径，路径错了会返回「路径 … 不存在」。
- 样式分类名只是名字，**不是**状态值：`hovered` / `pressed` / `focused` 这类状态要看 `get_props` 里的属性与 `get_stats` 的 `hovered` 字段。

## 能力边界（不要越界推断）

这几条是实现里的硬事实，越界会得到看似成功实则错误的结果。

1. **只有 D3D11 与 D3D12 宿主实现了调试桥。** `zephyra-vulkan` 没有实现 `BridgeHost`，所以 Linux / Vulkan 应用下 **MCP 整体不可用**——不是「截图不支持」，是所有工具都连不上。
2. **`press_key` 的键名有白名单。** `Key::from_name` 认得的只有：`Backspace`、`Delete`、`Enter`（`NumpadEnter` 同）、`Escape`、`Space`、`Tab`、`ArrowLeft`/`ArrowRight`/`ArrowUp`/`ArrowDown`、`Home`、`End`、`PageUp`/`PageDown`。
3. **白名单键只产生键边沿，不产生字符事件。** 单个非控制字符（如 `a`）会产生「边沿 + 字符」；其余名字（如 `F5`）只产生边沿，身份保留在 `KeyInput::name` 里。
4. **`press_key("Space")` 不等于物理空格键。** 物理空格在系统层会同时产生 `Char(' ')`，而注入的 `"Space"` 命中键名白名单、**不产生字符事件**，文本框里不会插入空格。`press_key` 也不接受裸空格字符（参数不允许空白字符）。
5. **`press_key` 没有修饰键位。** 注入的按键一律不带 Ctrl / Alt / Shift，组合快捷键无法模拟。
6. **应用是否响应输入取决于应用。** 宿主把事件交给 `View::on_input` 的输入状态；应用不消费就没有效果。当前 gallery 的 `on_input` 不消费文本事件，因此**通过 MCP 向 gallery 的文本框输入文字不会生效**。
7. **`scroll` 的增量单位跟随后端**，不要假定是「行数」。
8. **性能报告是 CPU 生成耗时**，不是 GPU 执行时间；单帧最大值不能代表整体表现。
9. **资源读数是单次采样**，不是基准；不同时间的两次读数不能直接相减当趋势。

## 排障入口

| 现象 | 先看 |
|---|---|
| 工具调用报连不上应用 | 应用是否带 `ZEPHYRA_DEBUG_PORT` 启动；CLI `--port` 是否与应用端口一致（CLI 默认 7878 且不读环境变量） |
| MCP 握手成功但工具全失败 | 用 `ping` 工具（走桥）区分「server 就绪」与「应用在线」 |
| 应用重启后调用失败 | 传输失败会丢连接重连重试一次；等应用窗口出现后重试即可，会话不用重建 |
| 点了没反应 | 重新 `get_layout` 取最新绝对坐标；确认目标矩形可见非零；点后用 `get_props` 核验状态是否真变 |
| 截图不可用 / 空 / 写盘失败 | 只有 D3D11 / D3D12 支持像素读回；给可写的 `.bmp` 路径；失败时改用 `get_layout` 并保留原错误 |
| 性能采集 0 帧 | 采集期间没有完成宿主帧（窗口静止且未触发重绘）；如实报告未完成，不要编造数据 |

完整排查步骤与更多场景见 `references/troubleshooting.md`。

## 汇报结果

简短说明：目标窗口、执行过的工具与动作、**读回验证到的状态**、数据口径与限制。

- 性能数据注明采样时长、帧数、单位、字段可用性。
- 资源数据注明采样对象、时间点，以及哪些字段 unavailable / warming_up。
- 未验证、超时、不支持的步骤明确标为未完成。
- 不要把「桥响应成功」说成「应用行为正确」。

## 仓库内参考

以下路径均**相对仓库根**：

- `ZephyraCLI/README.md` —— CLI 命令表与 `mcp` 子命令说明
- `zephyra/zephyra-fx/src/bridge.rs` —— 桥协议、行解析、树/属性 dump、采集器
- `ZephyraCLI/src/commands/mcp.rs` —— MCP 工具表、schema、并发模型、重连与取消
- `docs/dev-log/2026-10-25-mcp-debug-bridge.md` —— 调试桥的引入与协议分层
- `docs/dev-log/2026-10-26-mcp-input-layout-d3d12-capture.md` —— 输入注入、文本布局、D3D12 读回截图
- `docs/dev-log/2026-10-26-mcp-clicks-resources.md` —— 拖动、逐帧性能、资源遥测
- `docs/dev-log/2026-10-26-mcp-reconnect-timeout.md` —— 重连、读超时、线程模型、取消
