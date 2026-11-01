# ZephyraUI MCP 工具详解

> 主文档：`skills/zephyra-mcp/SKILL.md`。本文只讲**每个工具的参数与语义**；
> 返回文本的逐字格式见 `output-formats.md`。

所有坐标一律**物理像素、绝对坐标**。所有工具都经同一个桥会话串行执行。

---

## 只读工具

### `get_layout`

无参数。一次性返回面向模型的布局上下文：

1. `viewport <宽>x<高> physical-pixels`
2. `stats <最近一帧统计>`
3. `layout-tree path tag key absolute-x,absolute-y widthxheight [classes]` 表头
4. 布局树本体（与 `get_tree` 同格式）

**这是首选入口**：一次调用同时给出坐标、身份与统计，省掉「先 tree 再 layout」两趟。

### `get_tree`

无参数。只返回布局树，每行：

```text
<path> <tag> <key> <x>,<y> <w>x<h> [样式分类名...]
```

- `path`：根标记 `0` + 逐层子下标，如 `0.1.2`
- `tag`：节点标签（`Container` / `Text` / `Image` / `Control`）
- `key`：控件身份键；无则 `-`
- `x,y`：绝对矩形左上角
- `w,h`：绝对矩形尺寸
- 末尾空格分隔的样式分类名（无则省略）

样式名只是**分类名**（如 `button` / `primary`），不是状态值。要判断 hover / press / focus
是否成立，读 `get_props` 的属性与 `get_stats` 的 `hovered` 字段。

### `get_props`

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `path` | string | 是 | 来自 `get_tree` / `get_layout` 的路径，含根标记 `0` |

返回单节点的全部属性与样式值：

- `tag <标签>`
- `key <键>`（有才出现）
- `rect <x>,<y> <w>x<h> (absolute)` —— **绝对坐标**，以行尾 `(absolute)` 为准
- 每行一个 `style <分类名>`
- 样式层属性行 `<名字> = <值> (styled)`
- 本地层属性行 `<名字> = <值>`

样式行在前、本地行在后，与优先级相反（本地赢）—— 读的时候「后面盖前面」。

路径不存在时返回错误：`路径 0.9 不存在（用 tree 查看当前形状；路径是子下标链，树里每行第一列）`。

### `get_stats`

无参数。最近一帧统计：

```text
quads=97 skipped=0 dirty=2 needs_layout=0 redrawn=true hovered=-
```

- `quads` 渲染出的四边形数，`skipped` 跳过的数量。
- `dirty` 是**本次 diff 新标脏的节点数**（不是布尔），`needs_layout` 是**走到 Layout 或更高脏级的节点数**（不是布尔）。两者为 `0` 表示这一帧没有节点需要重排。
- `redrawn` 是布尔：本帧是否真的重画了。
- `hovered` 是命中节点的 key，点在空白处是 `-`。

首帧尚未跑过时返回 `(还没有帧)`。

### `get_viewport`

无参数。返回视口物理像素尺寸，形如 `1400 800`（宽 高，空格分隔）。

### `get_resources`

无参数。进程遥测：

```text
process_id=<pid>
cpu_process_percent=<数值 | warming_up>
cpu_basis=whole_machine_capacity
memory_working_set_bytes=<字节>
memory_private_bytes=<字节>
gpu_engine_max_percent=<数值 | unavailable (原因) | warming_up_or_idle>
gpu_dedicated_bytes=<字节 | unavailable (原因)>
gpu_shared_bytes=<字节 | unavailable (原因)>
```

- **CPU 是整机总容量口径**，不是单核百分比。`cpu_basis=whole_machine_capacity` 就是这条口径的声明。
- 首次读数可能是 `warming_up`：区间计数器需要基线，隔一拍再读一次。
- GPU 计数器不可用时**只有 GPU 字段降级**，CPU 与内存照常返回。
- `unavailable` 与 `0` 是两回事：驱动没给计数器 ≠ 利用率为零。空闲时的 `0.00` 也不能证明负载下的表现。

### `ping`

无参数。走调试桥探**应用**活性，成功返回 `pong`（外层包一句说明它与 MCP 协议层 ping 的区别）。

与 MCP 协议层的 `ping` 互补：

| 探什么 | 用什么 | 失败意味着 |
|---|---|---|
| MCP server 进程是否活着 | MCP 协议层 `ping` | server 挂了，重配 MCP |
| 应用是否在线 | 本工具 | 应用没起 / 没开桥 / 端口不对 |

---

## 输入类工具

### `hover`

| 参数 | 类型 | 必填 |
|---|---|---|
| `x` | number | 是 |
| `y` | number | 是 |

移动指针。与物理鼠标走**同一条**命中测试路径，hover 状态真的变。本帧生效。

### `click`

| 参数 | 类型 | 必填 |
|---|---|---|
| `x` | number | 是 |
| `y` | number | 是 |

合成完整点击：`move → down → up`。三个边沿分三次同步给应用，因此基于按下态变化的点击处理能看到 down 与 up 两个边沿。

本帧生效。**回 `clicked` 只说明事件注入了**，要确认控件状态变化需再读 `get_props` / `get_layout`。

### `press_key`

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `key` | string | 是 | 单个逻辑键名；不允许空白字符与控制字符 |

事件序列（`zephyra-winit-input::injected_events`）：

| 键名 | 产生的事件 |
|---|---|
| 白名单键（`Enter` / `Escape` / `Space` / `Tab` / `ArrowDown` / `Backspace` / `Delete` / `Home` / `End` / `PageUp` / `PageDown` / `NumpadEnter`） | 仅键边沿（按下 + 抬起） |
| 单个非控制字符（`a`、`中`） | 键边沿 **+** 字符事件 |
| 其余（`F5`、`ab`） | 仅键边沿，身份保留在 `KeyInput::name` |

白名单：`Backspace`、`Delete`、`Enter`/`NumpadEnter`、`Escape`、`Space`、`Tab`、
`ArrowLeft`、`ArrowRight`、`ArrowUp`、`ArrowDown`、`Home`、`End`、`PageUp`、`PageDown`。

**已知不等价点**：

- 物理空格在系统层会产生 `Char(' ')`；注入的 `"Space"` 命中白名单、**不产生字符事件**，
  文本框里不会插入空格。`press_key` 也不接受裸空格字符。
- **没有修饰键位**：注入的按键一律不带 Ctrl / Alt / Shift，组合快捷键无法模拟。
- 白名单键不走字符通道是**刻意**的：`Enter` 若再从字符通道走一遍，按一次会插两个换行。

### `scroll`

| 参数 | 类型 | 必填 |
|---|---|---|
| `x` | number | 是 |
| `y` | number | 是 |
| `delta_x` | number | 是 |
| `delta_y` | number | 是 |

在指定指针位置注入滚轮增量。`delta_y > 0` 向上，`< 0` 向下；横向用 `delta_x`。
单位跟随后端传递的平台滚轮增量，**不要假定固定「行数」**。

事件进入 `View::on_input` 的输入状态，与原生滚轮同路。

### `pointer_drag`

一次**按下、保持按住、移动、最后抬起**的连续手势 —— 不是多个独立 click。
用它测拖拽、滑动、需要连续按住移动的交互。

| 字段 | 类型 | 必填 | 约束 |
|---|---|---|---|
| `mode` | string | 是 | `segmented` / `linear` / `easing` |
| `points` | array | `segmented` 必填 | 2–128 个 `{x, y}`，按顺序组成折线 |
| `start` | `{x, y}` | `linear`/`easing` 必填 | 起点 |
| `end` | `{x, y}` | `linear`/`easing` 必填 | 终点 |
| `easing` | string | 否 | `linear`（默认）/ `ease_in` / `ease_out` / `smoothstep`；仅 `easing` 模式读取 |
| `duration_ms` | integer | 是 | 1–60000，移动时长 |
| `sample_interval_ms` | integer | 否 | 1–1000，默认 16 |
| `hold_before_ms` | integer | 否 | 0–10000，默认 0；按下后开始移动前的停留 |
| `hold_after_ms` | integer | 否 | 0–10000，默认 0；到达终点后抬起前的停留 |

约束：

- **总时长**（移动 + 两段停留）不超过 60000 毫秒。
- **采样点总数不超过 512**。超限时增大 `sample_interval_ms`、缩短 `duration_ms` 或简化折线。
- 时间编排在 **MCP 客户端侧**完成（CLI 按采样间隔发指令），不会让窗口事件线程睡眠。

失败处理：CLI 会尽力补发 `pointer_up`。若工具同时报告释放失败，**必须明确告知用户按钮状态
可能未清理**，不得把该结果说成成功。应用侧还有一道兜底：桥断连时宿主补一次抬起，
避免应用永久卡在拖动态。

示例：

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

---

## 采集类工具

### `analyze_performance`

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `duration_seconds` | integer | 是 | 1–300 的采集时长 |
| `include_rows` | boolean | 否 | 默认 `false`（只回汇总）；`true` 才附带逐帧行 |

行为：

- 宿主按时限自动停止，最多记录 3000 帧。**帧数先到则早于声明时长结束**。
- 调用会**等待采集结束**再返回。
- 默认**只返回汇总**（`min/avg/p50/p95/max`）。逐帧行最多 3000 行（约 120KB），
  整段进上下文是纯负担 —— 需要明细时显式传 `include_rows=true`。
- 客户端若支持 `notifications/cancelled`，在途采集会在下一轮轮询间隙中止。

报告字段与解释口径见 `output-formats.md`。

### `screenshot`

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `path` | string | 否 | `.bmp` 输出位置；省略则写系统临时目录 |

- 渲染一帧并读回像素，输出 **24bpp BMP**。仅 D3D11 / D3D12 支持；其他后端返回错误。
- 强制渲染，不依赖「界面正好在动」。
- 回执给**绝对路径**（客户端不知道 CLI 进程的 cwd，相对路径等于没说）。
- 完成后只报告路径、格式与观察结果；不要把整张图编码或转储进对话。
- 失败（后端不支持 / 读回为空 / 写盘失败）时保留原错误，改用 `get_layout` 等文本工具，
  **不要虚构截图**。
