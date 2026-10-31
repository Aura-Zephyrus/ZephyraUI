# ZephyraCLI

ZephyraUI 的命令行界面。**ZephyraUI workspace 的成员**（`E:\ZephyraUI\ZephyraCLI`），
不是独立工程 —— 因为它链的就是 workspace 里那一份 `zstylec`，而不是「碰巧指向
同一个目录的路径依赖」。这样「CLI 看到的属性表」与「编译器看到的属性表」在结构上
就不可能是两份。

```text
cargo build --release
target\release\zephyra.exe --help

# 或者当成 cargo 的子命令（需要 cargo-zephyra.exe 在 PATH 上）
cargo zephyra dev
```

`cargo-zephyra` 这个名字**不能改**：cargo 靠「`cargo-` + 子命令名」去 PATH 上找子
命令可执行文件，并把子命令名一起传进来（`cargo zephyra dev` ⇒ argv[1] 是
`zephyra`）。入口里会剥掉它，所以两个入口等价。

## 命令

| 命令 | 干什么 |
|---|---|
| `new <name>` | 生成一个能运行的交互示例工程，附带关键开发说明（`--path <dir>`、`--force`） |
| `check <file.zstyle>...` | 查样式表（`--format human\|json`） |
| `fmt [<路径>...]` | 排版：`.zstyle` / `.zypui` / Rust（`cargo fmt`）三者一起；`--check` 只报不写 |
| `dev` | 开发模式：打开 trace，样式表或窗口配置变更后重启 |
| `run` / `build` | 转交 cargo，并将项目窗口配置嵌入应用 |
| `info` | 版本与工具链状态（含路径） |
| `props [过滤词]` | 列受支持的属性 |
| `mcp` | MCP server：AI 直连运行中的界面（见下） |

### `new` — 新建项目

```text
zephyra new myapp
zephyra new myapp --path E:\work
```

会生成可交互的窗口示例（hover / press / click 状态）、项目 `README.md`、
`.zstyle` 样式表、`zephyra.toml` 和 `ZephyraUI.json` 窗口配置。`zephyra run`、
`dev` 与 `build` 会把该配置嵌入应用构建；`dev` 也会监视它的变化并重启窗口。
默认选择 D3D12；Windows 下可用
`cargo run --no-default-features --features d3d11` 选择 D3D11。
脚手架 Cargo manifest 中的 ZephyraUI 依赖指向当前源码 checkout，迁移工程时需修改依赖路径。

### `mcp` — AI 集成

```
zephyra mcp [--port 7878] [--host 127.0.0.1]
```

把 ZephyraUI 应用交给 AI 操控的桥。链路三层：

```text
AI 客户端 ──stdio·JSON-RPC── zephyra mcp ──TCP·行协议── 运行中的应用
```

**用法**：

1. 应用以 `ZEPHYRA_DEBUG_PORT=7878`（默认端口）启动——库层调试桥随环境变量
   开启，不设则**零监听**（默认关闭，不存在隐式端口）；
2. AI 客户端（Claude / WorkBuddy / 任何 MCP host）配一条 stdio MCP server：

```json
{
  "mcpServers": {
    "zephyra": { "command": "zephyra", "args": ["mcp"] }
  }
}
```

**工具表**：

| 工具 | 干什么 |
|---|---|
| `get_layout` | 给模型的纯文本布局上下文：物理像素视口、最近帧统计、节点路径/tag/key/绝对边界/classes；要读完整节点属性用 `get_props` |
| `get_tree` | 布局树 dump。每行 `path tag key x,y w*h [styles]`；路径列（子下标链）是 `get_props` 的钥匙；坐标是**绝对**像素，与 `hover`/`click` 同一坐标系——从树里抄个坐标就能点 |
| `get_props {path}` | 单节点属性表（tag/key/矩形/样式分类/全部属性） |
| `get_stats` | 最近一帧统计（quads / dirty / hovered） |
| `get_resources` | 进程 CPU%、工作集/私有内存；Windows 驱动提供时还含 GPU 引擎利用率与专用/共享显存 |
| `get_viewport` | 窗口视口尺寸 |
| `ping` | 走调试桥探一次应用活性，回 `pong`。与 MCP 协议层的 `ping`（探 server 进程）互补：桥 ping 回不来 = 应用没起/没开桥，server ping 回不来 = MCP 进程死了 |
| `hover {x,y}` | 注入指针移动——与物理鼠标走**同一条**命中测试路径 |
| `click {x,y}` | 注入完整点击（move → down → up），本帧生效 |
| `pointer_drag` | 按住主指针按钮拖动后抬起；`mode=segmented` 沿 `points` 折线，`linear` 走 start/end 直线，`easing` 按 linear/ease_in/ease_out/smoothstep 插值；可配置总时长、采样间隔及按下/抬起前停留 |
| `analyze_performance {duration_seconds,include_rows?}` | 默认关闭，按次选择性采集逐帧 CPU 生成性能；必须声明 1–300 秒，最多 3000 帧。**默认只回汇总**（min/avg/p50/p95/max）——逐帧行最多 3000 行 ≈ 120KB，不该灌进模型上下文；要逐帧明细传 `include_rows=true` |
| `press_key {key}` | 注入逻辑键按下与抬起；`View::on_input` 收到两个键边沿 |
| `scroll {x,y,delta_x,delta_y}` | 指定物理像素位置注入滚轮；正 `delta_y` 向上，事件交给 `View::on_input` |
| `screenshot {path?}` | 渲一帧读回 BMP 存盘（D3D11 / D3D12 支持；文本布局请用 `get_layout`）。回执里的路径是**绝对**路径——客户端不知道本进程的 cwd，相对路径等于没说 |

每个工具带 MCP `annotations`（`readOnlyHint` / `destructiveHint`）：读类工具
（`get_*` / `ping`）为只读，输入与采集类（`hover` / `click` / `press_key` /
`scroll` / `pointer_drag` / `analyze_performance` / `screenshot`）标记为会改状态。

典型闭环：`get_layout`（文本理解布局）→ `click`（点某个坐标）→ `get_layout` /
`screenshot`（确认变化）。所有工具都是「读-改-读」里的一步，无隐式状态。

**协议注意**：

- `mcp` 进程的 stdout **只**输出 JSON-RPC 消息，诊断全走 stderr——MCP stdio
  传输的硬约束，违反即客户端解析失败；
- MCP 握手（`initialize` / `tools/list`）**不需要**应用在线；连接在第一次
  `tools/call` 时才建立（重试 3 次 × 500ms），应用后启动是常态；
- **应用重启不废会话**：传输失败（应用重启留下死连接是 dev 工作流常态）会
  丢连接、重连、重试一次，下一次 `tools/call` 自动恢复；
- 每次桥 I/O 有 **10 秒读超时**：冻结的应用最多拖慢一次调用，不会挂死整个
  MCP 进程；
- **并发模型**：`tools/call` 投给唯一工作线程串行执行（保 AI「读-改-读」
  次序），其余消息（握手 / `ping` / 通知）在 stdin 线程就地处理；
- **取消**：客户端发 `notifications/cancelled` 后，排队中的调用开跑前跳过、
  在途的 `analyze_performance` 在下一轮轮询间隙中止；被取消的请求按协议不回；
- 桥单客户端：第二个连接要等第一个断开（AI 会话即独占使用）。MCP server 在
  按下与抬起之间消失时，应用侧会在察觉断连时补一次指针抬起，不会卡在拖动态。

### `fmt` — 排版

```text
zephyra fmt                  # 整个工程：.zstyle + .zypui + Rust（cargo fmt）
zephyra fmt --check          # 只报不写，给 CI 用（有要排版的退 1）
zephyra fmt frontend/        # 只排某个目录里的
zephyra fmt app.zstyle       # 只排指定文件（Rust 那一档跳过）
```

三个排版器，分工固定：

| 文件 | 谁排 | 判据住在哪 |
|---|---|---|
| `.rs` | `cargo fmt`（spawn，不重造） | rustfmt |
| `.zstyle` | `zstylec::format`（只动空白，值逐字照搬） | `zstylec/src/fmt.rs` |
| `.zypui` | `zypuic::format`（只动位置，引号内逐字节不变） | `zypuic/src/fmt.rs` |

CLI **不判排版**，只负责「找文件、调谁、写不写回、报什么」—— 判据一旦在 CLI 里
再抄一份，就有了「CLI 说排好了、编译器门禁还红」的立足点。两个编译器同属一个
workspace，所以那不可能发生，而不只是「靠一条路径依赖约定」。

Rust 那一档只在**工程根有 `Cargo.toml`** 时才跑：`cargo fmt` 需要 manifest，而
「这里只有一堆 `.zstyle`」的目录（docs/、临时目录）不该被一条 cargo 报错打红。
识别不了的扩展名**报用法错**（退 2），不静默跳过 —— 静默跳过等于「看起来生效了
其实没有」。

延伸阅读：`.zstyle` 排版 → [developer/04-stylesheets.md](../docs/developer/04-stylesheets.md)；
`.zypui` 排版 → [developer/zypui-style.md](../docs/developer/zypui-style.md)。

### `dev`

```
zephyra dev [--release] [--styles <dir>] [--no-watch] [--no-trace] [--manifest-path <p>]
```

- 默认开 `ZEPHYRA_TRACE=1`。不打开的话开发期最需要的那些行（视口变化、每帧四边形
  数、重建了没有）全都不打，而它们恰恰是排查「窗口变了但内容没跟上」的唯一判据。
- 默认盯 `styles/` 下的 `.zstyle`；`zephyra new` 生成的项目把样式放在 `frontend/`，
  因此使用 `zephyra dev --styles frontend`。样式表或 `ZephyraUI.json` 变化后会**重启进程**
  （不是运行期热重载：
  重启更简单、不会残留已解析的旧样式表，而 Rust 侧的增量编译本来就已经把等待压得很小）。
- 刻意**不**盯 `.rs` —— 那是 cargo 自己的事，两个 watcher 管同一份代码会互相触发。

## 退出码

| 码 | 含义 | 谁该改 |
|---|---|---|
| 0 | 成功（**警告不算脏**） | — |
| 1 | 动作失败（检查不过、构建失败） | 样式表 / 代码 |
| 2 | 用法错、文件读不了 | 流水线 / 路径 |
| 3 | 环境缺失（没有 cargo、目录已存在） | 环境 |

2 与 1 分开的理由：CI 里「样式表写错了」和「文件名打错了」要能分别处理。

## 为什么不用 clap

workspace 里本来就没有任何 CLI 框架。命令十几个、标志十来个，全是「位置参数 +
布尔开关」，没有嵌套子命令也没有变参类型推导 —— clap 的 derive 在这里换不来多少，
换来的是错误文案不可控（而这个项目对报错质量要求很高，见 `zstylec/src/errors.rs`）。

## 测试

```text
cargo test -p zephyra-cli
```

21 条，**每一条都是实测踩过的坑**，不是「应该这样」：

- 未知标志曾被静默忽略、`exit 0`，看起来像「开了严格模式跑过了」；
- `--manifest-path <p>` 的空格形式曾解析不出来，而命令表里明明写着它；
- `--styles` 曾只被加进「允许的选项」白名单、没加进 `VALUE_FLAGS`，于是报的是
  「unknown flag」——用户传的明明是合法标志，真因被藏住了；
- 文件读不了曾返回 `exit 1`（那是「内容有问题」）；
- 脚手架模板曾用 `font-size`，真实属性名是 `text-size`——`new` 交出一个当场就
  报错的工程；
- 模板曾写了个**不存在**的 `zephyra` 包，还把依赖路径写成猜出来的相对路径；
- 模板的入口曾缺 `View::on_input`、缺 `ApplicationHandler::window_event`、用了
  不存在的 `Element::text(..)`——这 3 个是靠**把生成的工程塞进 workspace 跑
  `cargo check`** 抓出来的，不是看出来的。

> 那次真编译的结论：模板现在 0 error 0 warning，且能链接出 `app.exe`。
> 后来只固化了「依赖路径存在」与「必需方法齐全」两条门禁 —— 真编译要链整条
> D3D12 栈，放进单元测试会让「跑测试」变成「跑构建」。