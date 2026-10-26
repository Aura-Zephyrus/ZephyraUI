# 2026-10-26-mcp-clicks-resources

> 现象：通用交互与性能测试需要可控的按住移动输入、逐帧性能数据及进程资源读数。

## 排查

`pointer_drag` 通过桥的 `pointer_down` / `pointer_up` 与既有 `hover` 组合成真正的
按住拖动：起点按下，CLI 按采样时间逐个移动，末尾抬起。模式有 polyline 分段、
整体直线及 linear/ease-in/ease-out/smoothstep easing。等待在 MCP 客户端执行，
不阻塞窗口线程；采样最多 512 点、总时长最多 60 秒，失败时尽力释放按钮。

资源采样由 Windows 宿主按需创建监视器：`GetProcessTimes` 差分计算整机总容量口径
的进程 CPU 百分比，`GetProcessMemoryInfo` 读取工作集与私有字节；PDH GPU 实例按
PID 过滤，报告最忙引擎利用率及 dedicated/shared memory。GPU 计数器缺失或失败时
保留 CPU 与内存数据并给 GPU 字段标注 unavailable。

性能分析默认关闭，只能通过 MCP `analyze_performance` 按需采集。调用必填
`duration_seconds`（1..=300）；宿主按帧记录 update/layout、render-submit 与总 CPU
生成耗时，达到时限或 3000 帧自动停止。MCP 客户端等待采集结束后一次返回逐帧明细、
min/avg/p50/p95/max。该计时不代表 GPU 执行完成时间。

真实 D3D12 Gallery 验收时发现，布局节点的 `arranged_rect` 已经是绝对坐标，但
MCP dump 又递归累加父节点偏移，导致深层节点坐标重复增加，模型无法直接用这些坐标
驱动输入；同时，树路径包含根节点 `0`，`get_props` 却把它当成第一个子下标。dump
改为直接输出每个节点的布局矩形，并让属性查询按根标记解析路径。D3D12 窗口的
1400×800 BMP 与布局对照后，按 `guide-inc` 的绝对坐标点击，`get_props` 确认计数
从 0 变为 1。

实机 MCP 验收：

- D3D12 Gallery（Windows，1400×800）：`pointer_drag` 完成 10 个移动采样；
  1 秒性能采集返回 140 帧（实际 1037 ms，总帧 CPU 时间均值 7111.8 µs）。
- D3D11 demo（Windows，800×600）：布局、hover、click、7 个采样的拖动均经 MCP
  返回成功；1 秒性能采集返回 2525 帧（实际 1007 ms，总帧 CPU 时间均值 343.5 µs）。
- D3D11 hover 的 `get_props` 读回 `Text = pen`，确认路由状态确实传到应用 `View`，
  不只是桥侧 `hovered` 统计发生变化。
- 两个宿主都取得进程 CPU、工作集/私有内存及 GPU 指标。资源采样先返回
  `warming_up`；下一次样本 D3D11 为 CPU 8.22%、GPU 0.00%、共享 GPU 内存
  38,133,760 B，D3D12 为 CPU 8.34%、GPU 0.00%、共享 GPU 内存 51,978,240 B。
  两组都在 Intel UHD 630、空闲 Gallery、MCP 桥连接时采样，专用 GPU 内存为 0 B；
  不把空闲时的 0% 当作负载下 GPU 利用率验收。

## 排除的假设

- 将连续坐标作为多个独立点击不符合按住移动/拖动测试语义。
- 在宿主回调中等待移动间隔会阻塞窗口帧/事件线程，因此时间编排由 MCP CLI 承担。
- GPU 计数器不可用不应让本来可读的 CPU 与进程内存采样整体失败。
- 不受时长或帧数上限约束的持续 profiling 会无界占用内存并污染后续测试，因此采集
  必须由调用者设定结束时间，并有硬帧数上限。

## 结论

MCP 新增 `pointer_drag`、`analyze_performance` 和 `get_resources`。拖动保持真实
Down→Move…→Up 状态；性能报告由调用方设时限、宿主自动收尾；资源报告对 GPU 能力
降级而不丢 CPU/内存结果。

## 门禁

- `cargo test -p zephyra-cli -p zephyra-fx --lib`：定向 MCP、桥协议与 profiler 测试。
- `cargo test -p zephyra-platform-win --lib`：包含当前 Windows 主机的进程内存真实采样。
- `cargo check -p zephyra-platform-win -p zephyra-winit -p zephyra-winit-d3d12`。
- `bridge::tree_dump_tests::tree_dump_keeps_nested_rectangles_in_absolute_coordinates`：
  嵌套偏移只计一次。
- D3D11 与 D3D12 运行中的 MCP host：拖动、逐帧分析和资源采样均端到端通过；
  D3D12 布局路径读取与按坐标点击也确认状态由 0 更新为 1。
- 最终 Windows workspace：`cargo fmt --all -- --check`、
  `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`
  均通过（954 passed / 0 failed / 51 ignored）。`cargo doc --no-deps --workspace`
  退出 0；35 条 rustdoc 警告仍来自既有的 core/style/fx/D3D12/UI/gallery/demo
  文档，新增与修正的 API 面没有新增警告。
