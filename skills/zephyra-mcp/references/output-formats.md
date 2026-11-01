# ZephyraUI MCP 返回文本格式

> 主文档：`skills/zephyra-mcp/SKILL.md`。本文给出各工具返回文本的**逐字格式**，
> 便于解析而不必猜。

---

## `get_layout`

```text
viewport 1400x800 physical-pixels
stats quads=97 skipped=0 dirty=2 needs_layout=0 redrawn=true hovered=-
layout-tree path tag key absolute-x,absolute-y widthxheight [classes]
0 Container window 0,0 1400x800 window
  0.0 Container titlebar 0,0 1400x44 titlebar mica
    0.0.0 Text app-title 10,13 120x18 text-body-strong
    0.0.1 Container search 524,5 340x34 search
      0.0.1.0 Container search-icon 534,14 16x16 search-icon
    0.0.2 Container caption 1258,0 132x44 caption-bar
      0.0.2.0 Control win:min 1258,0 44x44 caption-btn
```

- 第 1 行视口，第 2 行帧统计，第 3 行表头，之后是树本体。
- 树行缩进只为**人读**，解析靠第一列路径。
- 树的行格式与 `get_tree` 完全一致。
- `dirty` 与 `needs_layout` 是**计数**（新标脏 / 需重排的节点数），不是布尔。

## `get_tree`

```text
0 Container window 0,0 1400x800 window
  0.0 Container titlebar 0,0 1400x44 titlebar mica
    0.0.0 Text app-title 10,13 120x18 text-body-strong
    0.0.1 Container search 524,5 340x34 search
      0.0.1.0 Container search-icon 534,14 16x16 search-icon
    0.0.2 Container caption 1258,0 132x44 caption-bar
      0.0.2.0 Control win:min 1258,0 44x44 caption-btn
```

每行：`path tag key x,y w x h [样式名...]`（样式名空格分隔，无则整段省略）。

坐标是**绝对**值 —— 嵌套节点不重复累加父级偏移，抄一个坐标就能直接喂给 `hover` / `click`。

## `get_props`

```text
tag Control
key guide
rect 12,56 133x40 (absolute)
style nav-item
style selected
Direction = Horizontal (styled)
CrossAlign = Center (styled)
Spacing = 14 (styled)
Height = 40 (styled)
Padding = 10 (styled)
CornerRadius = 8 (styled)
Background = $control_secondary (styled)
Interactive = true
```

- `rect` 行尾的 `(absolute)` 是**坐标口径的唯一权威声明**。
- 样式行在属性行之前；`(styled)` 行来自样式表，无后缀行是本地值 —— 读的时候「后面盖前面」。
- 无 `key` 的节点不出现 `key` 行。

## `get_stats`

```text
quads=97 skipped=0 dirty=2 needs_layout=0 redrawn=true hovered=-
```

- `dirty` = 本次 diff **新标脏**的节点数（不是布尔）。
- `needs_layout` = 走到 Layout 或更高脏级的节点数（不是布尔）。
- `redrawn` 是布尔。
- `hovered` 无命中时是 `-`。

首帧前返回 `(还没有帧)`。

## `get_viewport`

```text
1400 800
```

宽 高，空格分隔，单位是物理像素。

## `get_resources`

```text
process_id=12512
cpu_process_percent=warming_up
cpu_basis=whole_machine_capacity
memory_working_set_bytes=188006400
memory_private_bytes=164380672
gpu_engine_max_percent=unavailable (PdhGetFormattedCounterArrayW(data) failed with status 0xC0000BAA)
gpu_dedicated_bytes=unavailable (PdhGetFormattedCounterArrayW(data) failed with status 0xC0000BAA)
gpu_shared_bytes=unavailable (PdhGetFormattedCounterArrayW(data) failed with status 0xC0000BAA)
```

- `cpu_process_percent` 首读可能是 `warming_up`。
- GPU 字段可能是 `unavailable (具体原因)` 或 `warming_up_or_idle`；驱动没给计数器时**只有 GPU 字段降级**，CPU 与内存照常返回。
- `cpu_basis=whole_machine_capacity` —— 整机总容量口径，**不是单核百分比**。

## `ping`

```text
app reachable over the debug bridge (pong) — 与「MCP server 已就绪」不同，这才说明应用在线
```

## `analyze_performance`

默认（`include_rows=false`）只回汇总：

```text
PROFILE_STATE complete
frames=42
elapsed_ms=2051
frame_total min=42732us avg=47440.8us p50=47072us p95=52141us max=57338us
update_layout min=42710us avg=47417.2us p50=47050us p95=52116us max=57303us
render_submit min=0us avg=0.0us p50=0us p95=0us max=1us
frame_index elapsed_ms total_us update_layout_us render_submit_us
(已省略 42 行逐帧数据；传 include_rows=true 获取全量)
```

- 第一行 `PROFILE_STATE complete` 是**完成标记**；采集进行中时返回的是
  `PROFILE_STATE running remaining_ms=<毫秒>`（CLI 内部轮询用，不直接暴露给调用方）。
- 单位是 `us`（微秒），**无空格**。`min` / `p50` / `p95` / `max` 是整数，`avg` 保留一位小数。
- 表头行 `frame_index elapsed_ms total_us update_layout_us render_submit_us` 始终存在；
  省略逐帧数据时，紧跟其后的是 `(已省略 N 行逐帧数据；传 include_rows=true 获取全量)`。

带 `include_rows=true` 时追加逐帧行：

```text
frame_index elapsed_ms total_us update_layout_us render_submit_us
1 44 44296 44273 0
2 89 44881 44863 0
3 136 47341 47324 0
```

字段口径：

| 字段 | 含义 |
|---|---|
| `frames` | 采集到的帧数 |
| `elapsed_ms` | 实际经过的毫秒数 |
| `frame_total` | 每帧总 CPU 生成时间 |
| `update_layout` | 更新与布局阶段耗时 |
| `render_submit` | 渲染提交阶段耗时 |

**解释时必须遵守**：

1. 同时给出时长、帧数与统计量，不要只挑最大值或单帧。
2. `render_submit` 是 **CPU 提交阶段**，不代表 GPU 完成时间 —— 这不是 GPU timestamp profiler。
3. 帧数达到 3000 上限会早于声明时长结束；`frames=0` 表示采集期间没有完成宿主帧。
4. 采样会改变运行时负载，只对用户要求的窗口/场景采集；采集前后避免并行做会污染结果的操作。
5. 调用超时且宿主帧循环可能已停时，报告为**采样未完成**，不要把部分等待描述成成功报告。

## `screenshot`

```text
screenshot written to C:\Users\Levy\AppData\Local\Temp\zephyra-shot-24060.bmp (3360054 bytes, 24bpp BMP)
```

路径是**绝对路径**。省略 `path` 时落在系统临时目录，文件名含进程 id。

## 输入类工具的回执

| 工具 | 回执 |
|---|---|
| `hover` | `hovered` |
| `click` | `clicked` |
| `press_key` | `pressed <键名>`（如 `pressed a`） |
| `scroll` | `scrolled` |
| `pointer_drag` | `pointer drag completed (N movement samples)` |

**这些回执只证明事件注入了**，不证明界面状态改变了。状态是否真变，靠操作后重新
`get_layout` / `get_props` / `get_stats` 读回验证。

## `get_props` 的路径错误

```text
ERR 路径 9.9 不存在（用 tree 查看当前形状；路径是子下标链，树里每行第一列）
```

错误以 `ERR ` 前缀返回，`isError` 为 `true`。
