---
name: zephyra-mcp
description: 'Operate and inspect a running ZephyraUI application through its MCP server. Use for ZephyraUI MCP setup, UI layout inspection, clicks, hover, keyboard, scrolling, held pointer drags, screenshots, frame performance profiling, CPU/GPU/memory telemetry, and MCP troubleshooting.'
argument-hint: 'Describe the ZephyraUI app and the interaction or diagnostics you need'
---

# ZephyraUI MCP 操作技能

通过 ZephyraUI MCP server 检查并操作**用户授权的、本机运行中的 ZephyraUI 窗口**。优先读取文本布局，以可验证的小步操作完成任务；需要像素证据或确实无法从文本判断时才截图。性能采集必须按需、限时开启。

## 适用范围

- 配置或诊断 ZephyraUI MCP server 与运行中应用之间的连接。
- 读取布局、节点属性、窗口视口、帧统计和资源使用。
- 通过坐标执行 hover、click、按住拖动、逻辑键输入和滚轮输入。
- 获取 D3D11 / D3D12 应用的 BMP 截图。
- 采集有界的逐帧 CPU 生成性能报告并解释其口径。

## 关键原则

1. **先读后操作**：先调用 `get_layout`，由其文本识别目标节点和绝对坐标；必要时用 `get_props` 读目标完整属性。不要猜坐标、根据不明缩放的截图估坐标，或把节点局部坐标当成窗口坐标。
2. **使用物理像素坐标**：`get_layout` / `get_tree`、`hover`、`click`、`scroll`、`pointer_drag` 共用窗口物理像素坐标系。节点布局矩形的坐标是绝对坐标。
3. **操作后核验**：输入完成后重新读取 `get_layout`、`get_props` 或 `get_stats`，确认界面状态改变符合预期。截图可用来核验像素，但不是文本布局的替代品。
4. **性能分析默认关闭**：只有用户明确要求性能数据或排查帧性能时才调用 `analyze_performance`，且选择完成任务所需的最短时长。
5. **如实报告数据口径与能力缺失**：CPU 帧计时不代表 GPU 执行耗时；GPU 遥测可能不可用；单次资源值或空闲利用率不能证明负载性能。
6. **操作范围受限**：只连接用户指定/授权的本机 ZephyraUI 应用。合成输入会实际改变窗口状态；不要将本地调试桥暴露到不可信网络或用它操控无关应用。

## 配置与启动

MCP 链路如下：

```text
MCP 客户端 --stdio / JSON-RPC--> zephyra mcp --TCP / 行协议--> 运行中的 ZephyraUI 应用
```

### 1. 开启应用调试桥

在启动应用的环境中设置 `ZEPHYRA_DEBUG_PORT`。默认值是 `7878`；未设置该变量时桥**不监听**。

PowerShell 示例：

```powershell
$env:ZEPHYRA_DEBUG_PORT = "7878"
cargo run -p zephyra-ui --bin gallery
```

按目标工程的实际启动命令替换示例命令。需要选择渲染后端时，遵循工程现有的 Cargo feature 和启动方式；不要同时启用互斥后端。

### 2. 配置 MCP 客户端

让 MCP host 以 stdio 启动 CLI：

```json
{
  "mcpServers": {
    "zephyra": {
      "command": "zephyra",
      "args": ["mcp"]
    }
  }
}
```

如果 CLI 不在 `PATH`，将 `command` 替换为其可执行文件路径。可用参数：

```text
zephyra mcp [--port 7878] [--host 127.0.0.1]
```

`--port` 必须与应用 `ZEPHYRA_DEBUG_PORT` 一致；`--host` 默认 `127.0.0.1`。若应用使用其他端口，两侧都显式配置相同端口。

MCP `initialize` 与 `tools/list` 不要求应用已经启动；首次执行工具时才连接应用。若 MCP server 已就绪但调用报告连接失败，先启动目标应用并确认端口一致。桥为单客户端连接：另一个连接通常需等现有客户端断开。

## 标准操作流程

### A. 建立界面上下文

1. 调用 `get_layout` 获取一份模型可读的当前布局文本。结果包含物理像素视口、最近帧统计，以及节点的路径、tag、key、绝对边界和样式分类。
2. 通过节点的绝对矩形选择操作点。通常使用矩形内部、远离边缘的点；若目标不可见、边界无效或遮挡关系不清楚，不要盲目点击。
3. 需要完整节点属性时，从布局中复制 `path`，调用 `get_props({ "path": "0.1.2" })`。路径必须来自 `get_tree` / `get_layout` 当前返回值，不要自行推测。
4. 若需要单独查看树结构，可调用 `get_tree`。每行格式大致为 `path tag key x,y w*h [styles]`；路径首段 `0` 是根标记，其余部分是子节点下标链。
5. 只有需要单独确认尺寸时调用 `get_viewport`；它返回窗口视口的物理像素宽高。

### B. 交互并验证

对单个目标采取最小必要动作，等待工具返回后再读取新状态。优先根据文本状态确认结果；状态可能异步更新时，再读取一次，不要通过快速重复点击“碰运气”。

- `hover({ "x": 120, "y": 80 })`：移动指针并触发与物理鼠标相同的命中/hover 路径。
- `click({ "x": 120, "y": 80 })`：在位置合成完整 move → down → up 点击。坐标取自最新布局。
- `press_key({ "key": "Enter" })`：注入单个逻辑键的按下与抬起。可用键名示例包括 `Enter`、`ArrowDown`、`Escape` 或一个字符。键名不可为空、不能包含空白或控制字符；这不是文本输入或粘贴接口。
- `scroll({ "x": 120, "y": 80, "delta_x": 0, "delta_y": -2 })`：在指定指针位置注入滚轮增量。`delta_y > 0` 向上，`delta_y < 0` 向下；横向方向按 `delta_x`。单位遵循后端传递的平台滚轮增量，不假定固定“行数”。
- `get_stats()`：读取最近一帧 quad 数、dirty 状态和 hovered 节点等统计。

### C. 按住指针移动（拖动）

`pointer_drag` 是一次**按下、保持按住、移动、最后抬起**的手势，不是多个独立 click。用它测试拖拽、滑动或需要连续按住移动的交互。CLI 会在 MCP 客户端侧按采样间隔编排输入，不会让窗口事件线程睡眠。

参数：

| 字段 | 说明 |
|---|---|
| `mode` | 必填：`segmented`、`linear` 或 `easing` |
| `points` | `segmented` 必填，2–128 个 `{ "x": number, "y": number }`，按顺序组成折线路径 |
| `start` / `end` | `linear`、`easing` 必填，分别为起点和终点坐标 |
| `easing` | `easing` 模式可选：`linear`（默认）、`ease_in`、`ease_out`、`smoothstep` |
| `duration_ms` | 必填，移动时长 1–60000 毫秒 |
| `sample_interval_ms` | 可选，采样间隔 1–1000 毫秒，默认 16 毫秒 |
| `hold_before_ms` / `hold_after_ms` | 可选，按下后开始移动前 / 到达终点后抬起前的停留时间，各 0–10000 毫秒，默认 0 |

所有坐标使用物理像素。总时长（移动和停留之和）最多 60000 毫秒；最多 512 个移动采样。若采样点超限，增大间隔、缩短时长或简化折线。拖动失败时 CLI 会尽力发送 `pointer_up`；若工具同时报告释放失败，明确告知用户按钮状态可能未清理，禁止把该结果说成成功。

调用示例：

```json
{
  "mode": "easing",
  "start": { "x": 180, "y": 240 },
  "end": { "x": 500, "y": 240 },
  "easing": "smoothstep",
  "duration_ms": 800,
  "sample_interval_ms": 16,
  "hold_before_ms": 100,
  "hold_after_ms": 100
}
```

### D. 截图

仅在需要检查像素外观、视觉层叠或输出视觉证据时调用 `screenshot`。D3D11 和 D3D12 支持 BMP 读回；其他后端可能不支持。工具可选参数 `path` 指定 `.bmp` 文件输出位置；省略时写入系统临时目录，并返回实际路径与文件大小。

截图会强制渲染并读回一帧，可能产生比文本查询更大的数据和额外开销。完成后只向用户报告必要的路径、格式和观察结果；不要把整张图编码或转储进对话。若截图调用返回后端不支持、读回为空或写文件失败，应保留错误并改用 `get_layout` 等文本工具，不要虚构截图。

### E. 逐帧性能分析

只在明确需要诊断 UI 帧 CPU 生成耗时时开启。`analyze_performance` 必须提供 `duration_seconds`，范围为 1–300 秒；宿主到时自动停止，且最多记录 3000 帧。该次调用会等待采集结束，再返回逐帧行与 `min / avg / p50 / p95 / max` 汇总。

报告字段：

- `frame_total`：每帧总 CPU 生成时间。
- `update_layout`：更新与布局阶段耗时。
- `render_submit`：渲染提交阶段耗时。
- 逐帧行包含帧序号、采集经过时间及上述阶段耗时，单位为微秒。

解释报告时：

1. 同时给出时长、帧数和统计量，不只挑最大值或单帧。
2. `render_submit` 是 CPU 提交阶段，不代表 GPU 完成时间；本工具不是 GPU timestamp profiler。
3. 帧数达到上限会早于声明时长结束；若没有宿主帧完成，报告可能显示 `frames=0`。
4. 该采样会改变运行时工作负载，只对用户要求的窗口/场景采集。采集前后避免并行进行会污染结果的其他测试或自动交互。
5. 若调用超时且宿主帧循环可能停止，报告为采样未完成，不要将部分等待描述成成功报告。

### F. CPU / GPU / 内存资源

调用 `get_resources` 读取运行进程遥测：

- CPU 利用率以整机总容量为百分比口径，而非单核百分比。
- 进程工作集和私有内存。
- Windows 驱动暴露计数器时的 GPU 引擎利用率与专用 / 共享 GPU 内存。

首次读数可能为 `warming_up`，因为区间计数器需要基线；可稍后再读一次。驱动或 GPU 计数器不可用时，CPU 与内存读数仍可能有效。GPU unavailable 不等同于 0%；空闲时 0% 也不能说明负载性能。说明采样对象、时间点及可用字段；不要把不同时间的单次样本当作基准对比。

## MCP 工具速查

| 工具 | 参数 | 用途 |
|---|---|---|
| `get_layout` | 无 | 获取面向模型的文本布局、视口与最近帧摘要 |
| `get_tree` | 无 | 获取带节点路径、tag、key、绝对坐标和样式分类的布局树 |
| `get_props` | `path: string` | 获取树中一个节点的全部属性与样式值 |
| `get_stats` | 无 | 获取最近一帧统计 |
| `get_viewport` | 无 | 获取视口物理像素尺寸 |
| `get_resources` | 无 | 获取 CPU、进程内存和可用的 Windows GPU 遥测 |
| `hover` | `x`, `y` | 移动指针 |
| `click` | `x`, `y` | 执行一次完整点击 |
| `press_key` | `key` | 注入单个逻辑键按下/抬起 |
| `scroll` | `x`, `y`, `delta_x`, `delta_y` | 注入定位滚轮增量 |
| `pointer_drag` | 见上文 | 保持按下状态的连续指针移动 |
| `screenshot` | 可选 `path` | 输出 24bpp BMP（D3D11 / D3D12） |
| `analyze_performance` | `duration_seconds` | 有界、按需逐帧 CPU 性能报告 |

## 故障排查

| 现象 | 检查与处理 |
|---|---|
| MCP 握手成功，但首次工具调用提示无法连接 | 确认目标 ZephyraUI 应用已启动、`ZEPHYRA_DEBUG_PORT` 与 CLI `--port` 相同、MCP CLI 指向 `127.0.0.1`。MCP server 就绪本身不表示应用在线。 |
| 端口已占用或连接像是连到另一应用 | 检查调试端口的占用者，确认连接到用户指定的 ZephyraUI 实例；必要时为应用和 CLI 一起更换端口。不要尝试连接未知服务。 |
| 第二个客户端无法访问桥 | 桥按单客户端使用；关闭旧 MCP 会话后重试。 |
| 节点没有响应点击或坐标看起来偏移 | 重新取 `get_layout`；使用它给出的绝对物理像素坐标。不要累加父级偏移，不要将截图缩放坐标直接传入。确认目标矩形可见、非零，并点击矩形内部后用 `get_props` / `get_layout` 核验。 |
| `get_props` 报路径错误或取错节点 | 复制最新 `get_tree` / `get_layout` 的路径列（如 `0.1.2`），包括开头根标记 `0`；不要手工重编号。 |
| hover 状态没有改变 | 在当前视口和最新布局上 hover 到目标矩形内部，再读 `get_layout` 或该节点 `get_props`；注意某些控件可能没有单独的 hover 样式。 |
| 键盘输入不符合预期 | `press_key` 是单键逻辑输入，不支持空格/控制字符或一整段文本；先使目标控件取得焦点，再发送一个键并检查结果。 |
| 拖动参数被拒绝 | 确认模式所需字段齐全、时长和停留时长合规、坐标为有限数、折线 2–128 点，并将采样总数限制在 512 内。减少路径点或增大采样间隔。 |
| 性能分析无帧或一直未完成 | 确认窗口宿主帧循环仍在运行、指定时长 1–300 秒。0 帧表示采集期间没有完成宿主帧；超出等待上限时应报告失败，而非生成推测数据。 |
| GPU 字段 unavailable / warming_up | 首次采样可等待基线后再读；驱动未提供计数器时如实报告不可用。不要将缺失值改写为 0。 |
| 截图不可用、为空或写盘失败 | 确认是 D3D11 / D3D12 后端并使用可写的本地 `.bmp` 路径；失败时用文本布局作为替代并报告原错误。 |

## 汇报结果

完成后简短总结目标窗口、执行的工具和动作、验证到的状态，以及数据限制。对性能或资源数据注明采样时长 / 帧数 / 单位 / 可用性；对未验证、超时或不支持的步骤明确标为未完成。不要把桥响应成功等同于应用行为正确，必须使用读回状态或截图验证最终效果。

## 仓库参考

- [ZephyraCLI MCP 用法与工具说明](../../../ZephyraCLI/README.md)
- [MCP 输入、布局与 D3D12 截图实现记录](../../../docs/dev-log/2026-10-26-mcp-input-layout-d3d12-capture.md)
- [MCP 拖动、性能分析与资源遥测记录](../../../docs/dev-log/2026-10-26-mcp-clicks-resources.md)
