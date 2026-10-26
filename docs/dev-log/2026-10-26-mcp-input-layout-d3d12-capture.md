# 2026-10-26-mcp-input-layout-d3d12-capture

> 现象：MCP 已能检查和点击运行中的界面，但无法注入键盘/滚轮、读取文本布局或在 D3D12 下取得截图；大截图也可能卡在非阻塞 TCP 写入。

## 排查

- `get_layout` 应返回面向模型的结构化文本，而不是强迫 AI 从截图推断节点 key、边界和层级。
- `press_key` 与 `scroll` 必须进入和原生事件一致的 `InputState` / `View::on_input` 路径，避免另造一套控件行为。
- D3D12 截图需在 MSAA resolve 后复制后缓冲至 readback heap，并在 GPU fence 完成后映射；即使帧缓存判断内容未变化，也要强制绘制截图帧。
- TCP 桥是非阻塞的。一次 `write_all` 不能保证大 BMP 发完；CLI 也不应逐字节将二进制转换为 UTF-8 文本。
- WSL 的 loopback 传输可能晚于一次 `poll`。测试若先单次 poll 再阻塞读响应，会把调度时序误判成协议死锁。

## 排除的假设

- 截图空缺不是 D3D12 编码器或 BMP 格式问题；根因是此前没有后缓冲 readback 接线。
- 按键/滚动不需要模拟控件私有回调；沿用输入状态让现有命中和路由路径处理即可。
- 大截图卡住不是客户端 JSON-RPC 解析问题；在桥端续写待发响应，并在 CLI 端按声明长度读取原始字节后恢复。
- WSL 单测挂起不是命令格式或桥协议不兼容；改为独立服务线程持续轮询后，同一 round-trip 测试在 WSL 快速通过。

## 结论

- MCP 增加 `get_layout`、`press_key`、`scroll`；文本布局包含视口、最近帧统计和绝对坐标布局树，节点详细属性仍由 `get_props` 获取。
- D3D11 / D3D12 宿主转发键盘与滚动状态；`.zypui` 的 `render-source` 只声明 kind，宿主提供并校验带类型的 payload。
- D3D12 在 resolve 后读回 RGBA 后缓冲并编码 BMP；桥端支持非阻塞分段续写，CLI 按长度分块接收二进制。
- 真机 MCP 闭环分别在 D3D11 与 D3D12 demo 验证：键盘、滚动、点击后布局变化与截图。D3D11 BMP 为 800×600、1,440,054 字节；D3D12 BMP 为 1100×700、2,310,054 字节。

## 门禁

- `cargo test -p zephyra-fx -p zypui -p zypuic -p zephyra-cli --no-fail-fast`：WSL 全部通过（fx 133、zypui 15、zypuic 20、CLI 12）。
- `bridge::tests::full_round_trip_over_tcp` 与 `bridge::tests::large_screenshot_is_drained_across_nonblocking_polls`：Windows targeted 测试通过；WSL 单测通过。
- CLI 2 MiB 二进制响应测试、`.zypui` 编译/解码保留属性测试、宿主 payload kind 匹配/缺失/不匹配测试通过。
- D3D11 / D3D12 真机 MCP 闭环及 D3D12 readback 截图通过。
- `cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets -- -D warnings` 通过；`cargo doc --no-deps --workspace` 退出 0，`zypui` 相关断链已修正，其余为仓库既有 rustdoc 警告。
- `cargo test --workspace`：941 通过、0 失败、51 ignored。首跑发现 D3D11 shader 产物时间戳早于源码；按门禁重编 20 个 entry 后字节码均未变化，仅刷新本机产物时间戳，完整测试通过。
