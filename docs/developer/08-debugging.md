# 调试与性能监测

> 面向**外部使用者**：怎么用 CLI 直连一个跑着的应用，看它里面发生了什么、
> 慢在哪。

## 一、先打开调试桥

调试桥默认**关闭** —— 应用必须显式打开才会监听：

```bash
ZEPHYRA_DEBUG_PORT=7878 cargo run
```

⚠ **必须用 debug 构建**：桥、profiler、帧 dump、`ZEPHYRA_TRACE` 等诊断设施在
release 里由 `#[cfg(debug_assertions)]` 编译期剔除 —— release 二进制里连这些
环境变量名都不存在，不是「运行时默认关闭」而是「代码根本不在里面」。

| 变量 | 作用 |
|---|---|
| `ZEPHYRA_DEBUG_PORT=7878` | 在 `127.0.0.1:7878` 开调试桥（**只监听本机**） |
| `ZEPHYRA_FRAME_DUMP=<path>` | 把某帧的后缓冲写成 BMP（给「肉眼才看得见」的故障留证据） |

不设 `ZEPHYRA_DEBUG_PORT` ⇒ 零监听，端口一个都不占。

⚠ 桥是**调试设施**，不是产品功能：它没有任何鉴权，只监听回环地址。正因如此它不进 release 二进制；要在 release 性能下排查只能用 debug 构建。

## 二、四条入口，一份协议

```text
zephyra bridge "click_key switch:solid" "wait_frames 2" "stats"   # 一条命令做一件事
zephyra bridge --stdin < commands.txt                             # 脚本
zephyra perf watch                                                # 性能监测
zephyra debug watch "props 0.1.0.1"                              # 调试器
zephyra mcp                                                       # 给 AI 的 MCP server
```

**协议只有一份**（`zephyra-fx::bridge` 的行文本协议），四个入口共用它 ——
`mcp` 只是在外面套了 JSON-RPC。跨入口共享的那一半只能有一份实现，否则
「改了一处忘了另一处」就会让两个入口对同一件事给出不同答案。

| 入口 | 适合 |
|---|---|
| `bridge` | **人 / 脚本**：一次调用做几件事（点击 → 等两帧 → 截图） |
| `perf` | **性能**：这一帧慢在哪？刚才 5 秒的帧率是多少？ |
| `debug` | **调试**：盯着某个属性随时间怎么变 |
| `mcp` | **AI 客户端**：工具调用协议（stdio + JSON-RPC） |

## 三、`zephyra bridge`：直接问应用

```bash
zephyra bridge --list                        # 列出可达的实例（扫 7878..7890）
zephyra bridge "tree"                        # 布局树（绝对坐标）
zephyra bridge "tree --json"                 # 同一份数据的 JSON 形态
zephyra bridge "props 0.1.0.1"               # 某个节点的属性
zephyra bridge "find 纯色"                   # 按**可见文字**找元素
zephyra bridge "hit 488 770"                 # 这个像素上是什么
zephyra bridge "click_path 0.1.0.7"          # 按路径点击
zephyra bridge "click_key switch:solid"      # 按 key 点击
zephyra bridge "hover 120 48"
zephyra bridge "press_key Enter"
zephyra bridge "type_text \"hello world\""    # 一次输入一段文字
zephyra bridge "scroll 120 48 0 3"              # 向上滚 3 格（正 delta_y = 向上）
zephyra bridge "wait_frames 2"               # 等应用真的画够 2 帧
zephyra bridge "resize 800 600"              # 改窗口客户区尺寸（物理像素）
zephyra bridge "touch down 0 120 48"         # 注入一次**触摸**（多点触控）
zephyra bridge "touch move 0 160 48"
zephyra bridge "touch up 0 160 48"
zephyra bridge "touch cancel 0 160 48"       # 系统抢走手势（Windows 11 三指/四指）
zephyra bridge "stats --json"                # 最后一帧的统计
zephyra bridge "shot --out a.png"            # 截图
zephyra bridge "shot --out a.png --rect 430,750,230,40 --scale 3"   # 裁一块 + 放大
```

四条约定，避免来回换算：

- **一切坐标都是物理像素**（`viewport` 明说 physical-pixels）。树的坐标、
  点击坐标、`resize` 的宽高，同一套。
- **`--out` / `--rect` / `--scale` 是客户端侧的** —— 应用只回字节，落盘与
  裁剪都是 CLI 的事。
- **`find` / `hit` 给的是路径，不是 key** —— key **允许重复**（每个导航项
  的标签都叫 `label`），按 key 点会点错那一个。
- **滚轮增量正数 = 向上**（与 winit / `zephyra_fx::ScrollInput` 同一口径）：
  `scroll x y dx dy` 的 `delta_y > 0` 是**向上滚**（往回看），`< 0` 向下。
  控件层必须照这个符号做 —— 反了的话，物理滚轮的方向会与平台相反，
  见 [`docs/dev-log/2026-11-scroll-wheel-direction.md`](../dev-log/2026-11-scroll-wheel-direction.md)。

### `touch`：在没有触摸屏的机器上验触控

```bash
zephyra bridge "touch down 0 100 200" "wait_frames 1" "touch up 0 100 200"
zephyra bridge "touch down 1 100 200" "touch cancel 1 100 200"   # 系统抢走手势
```

相位与 `WindowEvent::Touch` 一一对应：`down` / `move` / `up` / `cancel`。
`id` 是**手指身份** —— 不同 id 是不同的手指，于是「两指捏合」也能复现。

⚠ **`cancel` 不是 `up`**：取消时**不**提交点击（用户的手指还在屏幕上）。
MCP 侧有对应的 `touch` 工具。

### `wait_frames` 的三种结局

```text
waited 5 rendered frames                    达标
the app settled (rendered 0 of the requested 5 frames, then went idle)
                                           应用静止（画面已经稳定，没等到更多帧是因为没有动画在跑）
waited only 0 of the requested frames (...)  超时（应用既没画够、也不算静止）
```

静止的应用**永远出不了**那么多帧 —— 所以「静止」是一个**结论**，不是错误。

## 四、`zephyra perf`：性能监测

```bash
zephyra perf profile --seconds 5           # 取一次报告（fps + 分段百分位）
zephyra perf watch --interval-ms 250        # 实时表
```

`watch` 的输出：

```text
      t     fps   core_us  diff style scan layout paint  quads dirty damage
  +18ms    0.0      165   87    55   21      0     0     98     2 full
 +835ms    0.0       66   37    23    6      0     0     98     2 full
```

| 列 | 含义 |
|---|---|
| `fps` | **渲染帧率**（两次采样的帧数差 / 时间差）。应用静止 ⇒ 0（那是事实） |
| `core_us` | core 处理这一帧的总耗时 |
| `lat_us` | **输入 → 承载它的那一帧就绪**的耗时（不含 DWM 合成 —— 那一段是系统的） |
| `diff` / `style` / `scan` / `layout` / `paint` | **分段耗时**（core 自己测的） |
| `quads` | 画面上有多少个四边形（**上一次画出来的那帧**） |
| `dirty` | 这一帧有几个节点被标脏 |
| `damage` | 这一帧重画了多大范围（`full` / `rect`） |

⚠ 稳态帧（树没变、core 跳过整帧）里 `layout` 与 `paint` 是 **0** ——
意思是「那两段**没跑**」，不是「不花钱」。

### 「GPU 占用」怎么问 —— `tools/gpu_probe.ps1`

上面的表全是 **CPU 侧**的。GPU 侧的占用要单独量，工具在仓里：

```bash
powershell -ExecutionPolicy Bypass -File tools\gpu_probe.ps1 -TargetPid 12345
powershell -ExecutionPolicy Bypass -File tools\gpu_probe.ps1 -Port 7878 -TargetPid 12345
```

它做两件事：

1. 读 Windows 的 **"GPU Engine"** 性能计数器 —— 这就是任务管理器那一列的
   来源，**按进程归因**；并且**按引擎拆开**（3D / Copy / Video）：
   一份 4 MB 纹理上传落在 **Copy** 上，光栅化落在 **3D** 上，
   两者的修法完全不同；
2. 同时读应用自己的 `frames`（走调试桥）⇒ 换算出
   **「每呈现一帧多少 GPU 毫秒」**。

⚠ **只报百分比会优化错口径。** GPU 占用 = 呈现帧数 × 每帧 GPU 时间，
而「GPU 5%」在 16 fps 时是 **3 ms/帧**、在 120 fps 时是 **0.4 ms/帧**。
把每帧成本砍掉一半，帧率会自己涨回来（保留式 UI 是「有事就画」），
占用原地弹回原处 —— 看起来「优化了但占用没降」。

```text
GPU  avg=6.33%  max=12.47%

per engine (avg / max):
  3D         avg=6.33  max=12.47

frames 512 -> 627  (+115 in 18.2s => 6.3 fps)
GPU per presented frame: 10.012 ms
```

⇒ 「6.33%」与「10 ms/帧」说的是**两件事**，而优化要动的是后者。

### 「GPU 占用」的上界在哪

本后端**呈现率有封顶**：取显示器的刷新率（不是写死 60 —— 120 Hz 屏上按 60
封顶会白白丢掉一半流畅度）。没有这个上界的话，动画期间事件循环是
`ControlFlow::Poll`（能画多快画多快），GPU 占用**没有天花板**。

⚠ 封顶是「每帧 GPU 时间」与「GPU 占用」之间的那道闸：**每帧变便宜了，
帧率会涨回来把占用顶回去**。⇒ 想让占用真的降下来，得同时看 §六 那两个数。

### 「跟手吗」怎么问

`lat_us` 那一列就是答案：它是「输入事件到达 → 承载它的那一帧就绪」的耗时。

实测（gallery，D3D12，release，1400×800）：应用侧整帧 **0.5~1 ms** ——
60 Hz 帧预算（16.7 ms）的 3~6%。**CPU 不是「不跟手」的原因**；余下的是
vsync 与 DWM 合成（系统的）。

⚠ `perf profile` 采样的帧是「**真的画了**的帧」。应用静止 ⇒ `frames=0` ——
那本身就是结论。

### 为什么分段耗时由 core 测

那五段里有四段在 core 里（对账、样式注入、脏区扫描、布局、绘制），宿主只看得到
「整帧多久」。core 自己测，于是「这一帧的 300 µs 花在哪」是**读出来的**。

代价：每段一次 `Instant::now`（release 下约 20 ns，五段约 100 ns —— 一帧
30~80 µs 的 0.1% 量级）。**恒开**的好处是任何一次「这帧为什么慢」都能直接读到
分段数字，不需要改代码重编。

## 五、`zephyra debug`：调试器

```bash
zephyra debug watch "props 0.1.0.2" --interval-ms 200     # 只在答案变了时打印
zephyra debug watch "stats" --all                          # 每次都打印
zephyra debug tree 0.1.0 --depth 2                          # 一棵子树
zephyra debug snapshot before.txt                          # 存一份布局树
zephyra debug diff before.txt                              # 与存的那份比
```

### `watch`：只在答案变了的时候才说话

调试真正难的是**盯着一个量随时间怎么变**：

- 「这个按钮的 `Pressed` 属性到底有没有被写上？」
- 「我点了它，为什么画面没变？」

```text
--- +0ms ---
tag Control
key styling
...
style nav-item
--- +855ms ---            ← 答案变了才打印
tag Control
key styling
...
style nav-item
style selected
style hovered
```

一条 `props` 的输出有十几行；每 200 ms 打一次，一天下来是几十万行噪音，
而要看的恰恰是**变化的那一刻**。`--all` 可以看每次采样。

⚠ `watch` **占着一条桥连接**，而桥现在支持**多个客户端** —— 所以可以一边
`debug watch` 盯着、一边 `bridge click` 去点。

### `diff`：按路径比

```text
~ 0.1.0.2 Control styling 12,148 103x40 nav-item
  原：0.1.0.2 Control styling 12,148 103x40 nav-item selected hovered
+ 0.1.1.1.0.3.1.1.1.1.0 Control act:touch-bad 436,456 44x44 round round
- 0.1.1.1.0.2.1.1.1.0 Control act:style-subtle 436,310 50x36 button subtle
新增 17，消失 11，变了 33（共 99 个节点 → 105 个节点）
```

判据是**路径**（不是整行）：树的缩进会变（深度变了），而路径不会。

## 六、`zephyra mcp`：给 AI 客户端

```bash
zephyra mcp --port 7878
```

stdio 上的 JSON-RPC 2.0。工具表在 `initialize` 之后由 `tools/list` 给出
（当前 23 个：树 / 属性 / 统计 / 视口 / 命中测试 / 按 key 或 path 点击 /
找文字 / 输入文字 / 滚轮 / 拖动 / 改窗口大小 / 截图 / 等帧 / 性能分析…）。

人类形态的命令行与 MCP 形态的工具**共用同一份桥协议**，所以两边的行为一致。

## 七、常见问题

| 症状 | 原因 |
|---|---|
| `连不上应用调试桥` | 应用没带 `ZEPHYRA_DEBUG_PORT` 启动，或端口不是那个 |
| `应用是静止的` | 应用没在画 —— 那是结论，不是故障。用 `stats` 看它停在什么状态 |
| `quads=0` | 这一帧被跳过了（`redrawn=false`）。`quads` 报的是**上一次画出来**的那帧 |
| `the app settled` | 应用静止，没等到更多帧 |
| `wait_frames` 立刻返回 | 帧已经画够了（`waited N rendered frames`） |

## 相关

- 帧流水线（性能相关的内部机制）→ [05-frame-pipeline.md](05-frame-pipeline.md)
- 本仓自己踩过的坑 → [`../dev-log/`](../dev-log/README.md)
