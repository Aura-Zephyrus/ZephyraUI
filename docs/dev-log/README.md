# 开发日志

给**内部**的。每条记一个问题：现象 → 排查 → **排除过的假设** → 结论 → 钉住它的门禁。

用户文档在 [`../developer/`](../developer/README.md)。这里的「排除过的假设」在那边
写不出来，而那恰恰是最值钱的部分。

## 新的一条怎么写

```markdown
# YYYY-MM-DD-<slug>

> 现象：<一句可复现的描述>

## 排查

## 排除的假设          ← 不能省。下一个来的人靠这段省下几轮

## 结论

## 门禁                ← 哪条测试钉住了它。没有门禁的结论会烂掉
```

## 索引

| 日期 | 条目 | 状态 |
|---|---|---|
| 2026-10-25 | [外部渲染源拓展接口：Command::Custom](2026-10-25-render-source-extension.md) | 已接；DisplayList 开出唯一放行位 `Command::Custom{bounds,source,filter,blend}`——核心只搬运、后端按 `kind` 查钩子注册表、宿主解释 `Arc<dyn Any>` 载荷（指针相等 = diff 静止）；`Segment::Custom` 保序（段数不变量 `2f+2c+1`，压力测试同步扩展）；降级不崩（未注册跳过+计数，`custom_passes`/`unhandled_custom` 双面计数）；D3D11 端到端验收：钩子 `ClearView` 涂红、读回像素恰好 = bounds 面积。v1 边界：离屏不支持自绘、CPU 路径只计数、Vulkan 待真机轨道 |
| 2026-10-25 | [MCP 调试桥：AI 直连运行中的界面](2026-10-25-mcp-debug-bridge.md) | 已接；三层链路 `AI ── zephyra mcp（JSON-RPC/stdio）── TCP 行协议 ── fx::bridge`——库层纯 std 零依赖、JSON 只落在 CLI（显式破例有注释）；**操控 = 输入注入**（与物理鼠标同一条命中/路由路径，属性直改会被 diff 覆盖）；树 dump 出绝对坐标（抄坐标即可点击）；截图钉 present 前（flip model 读回实测表）；D3D12 控制流联动（has_client→Poll/Wait）；工具 7 个（tree/props/stats/viewport/hover/click/screenshot），D3D12 截图 v1 诚实降级 |
| 2026-10-25 | [core 结构审计与 API 收口](2026-10-25-core-structure-audit.md) | 已接；真实依赖图落地 `lib.rs`（node 枢纽三处双向 = 缓存写回形状，非违规）；模块表补 responsive/i18n/styler；`control_fill_role` 归位 `theme::roles`；收口删 8 项零引用 API（含文档谎称「布局趟在用」的 `content_size(_of)`——measure_stack 内联后遗留）；方法论坑：前缀掩码 + props 键必须 pub（字符串解析）。扩展缝盘点基线已记，DisplayList 命令闭集与布局策略闭集是下一步「拓展接口」的明显缺口 |
| 2026-10-25 | [.zypui 结构迁移：控件层 + gallery + demo 全量落地](2026-10-zypui-structure-migration.md) | 已接；三处 Rust 结构树各归一份 `.zypui`（树全等 parity 验收后删旧码）；语言层补 `Val::Raw` 结构值通道 + `filter`/`blend` bare attr；十条真红过的缝（实参必须 `{}` / 类序即匹配序 / for 内 hover 显式 key / 属性式 `<for>` / `&&` 非 `and` / 属性顺序非语义 / build-deps 段位…）；语言规范入 `docs/developer/07-declarative-ui.md` |
| 2026-10-25 | [严格测试与压力门禁：6 个真问题](2026-10-25-strict-tests-and-pressure-gates.md) | 已接；fx 确定性压力测试（段数 2f+1 文档错 / `plan_offscreens` 去重洞）+ 分段回放守恒律；D3D11 合成趟丢混合（144 命令压力场景抓出，Multiply+SoftLight 恰 60）+ 像素门禁；D3D12 内容趟对全 0 RT 混合（纯黑 + 计数器说谎，`into_fresh_rt` 修）；**`ID3D12InfoQueue` 是进程级单例** ⇒ 并行 drain 偷消息（假红之外更危险的是**假绿**）⇒ 两侧 harness 加进程级串行锁 + singleton 常驻测试；**SoftLight 公式死区**（0.25<b<0.5 ⇒ B=0 纯黑，产品级 bug）与两端 UNORM8 量化差 1 LSB（D3D11 128 vs D3D12 127）叠加造成 parity 差 87 ⇒ 换 Pegtop 标准式 + fxc 重编 20 entry ⇒ parity 转绿。全量 855 通过 / 0 失败。**Vulkan 同型审计未过真 GPU**（无 WSL） |
| 2026-10-24 | [传参校验门禁：Close() 回 0x80070057 那一族](2026-10-24-command-validation-gates.md) | 已接；调试层开着回放每帧、成功路径也抽干队列。D3D12 harness 抓出 4 个存量 bug（resolve 源状态 538 / 空转屏障 525 / PSO 采样数 614 / drain 严重级写反）+ 计数器说谎 + `min()` 守卫漏洞；D3D11 harness 修好「一条消息都读不出」（两段式）+ redraw 少减账 + SRV 卫生（WARNING 每帧刷）。反向对照证明**同一错误不同驱动一个炸一个不炸**。用户贴的冻结链已修，真机 gallery 验证待用户。**Vulkan 门禁未写**：本机无 WSL（管道已就绪，`ZEPHYRA_VK_DEBUG=1`） |
| 2026-10-24 | [Vulkan 后端补上折射（液态玻璃在 Linux 侧真正生效）](2026-10-24-vulkan-refraction.md) | 已接（**未过真 GPU**）；WGSL 补 `fs_refraction`（与 HLSL `PSRefraction` 逐项对应）+ 渲染器取全屏目标副本（`snapshot_dst`）+ 退化路径；5 条字符串门禁钉住四个已实测的失效模式（1px 遮罩 / bezel 不按半径夹 / 有界楔形偏折 / 色散反向）。**待办**：WSL2 真机跑 gallery glass 路由（本机 wsl 被黑名单挡了） |
| 2026-10-24 | [两处「画面不更新」：陈旧文字成形缓存 + MSAA 折射读错源](2026-10-gallery-stale-text-and-msaa-backdrop.md) | 已修；`TEXT` 不参与布局 ⇒ 改字既不重排也不清 `measure_cache`，而 `paint` 拿**旧** `ShapedText` 算字形、把**新**文字写进 `glyph_quads`（缓存中毒，等 resize 才自愈）→ 未命中时无条件按当前文字重新成形；折射副本取的是**后缓冲**（MSAA 时那是上一帧的解析结果）→ 改取本帧绘制目标，按 `SampleDesc.Count` 分岔；**解析目标的状态必须是 `RESOLVE_DEST`**，用 `RENDER_TARGET` 会录进非法命令 ⇒ `Close()` 回 `0x80070057`、之后每帧 `Reset` 都失败、画面永久冻结（看着像「点导航没反应」） |
| 2026-10-22 | [液态玻璃：位移发散、遮罩压掉效果、几何错位](2026-10-liquid-glass-refraction-fixes.md) | 已修（两轮）；薄棱镜近似在大角度下发散（位移 491px vs 厚度 28px）→ 换有界的楔形偏折；遮罩 30px 把最强的折射乘成 0 → 收窄成 1px；倒角带被圆角半径夹住 → 上 3/4 档是空档；第三块玻璃下面**没有内容**可折射 → 加贯穿整宽的竖条带；标题 Thin/Normal/Thick 与三块相同的玻璃**说谎** → 厚度按档位偏移；**白 @30% 的玻璃压白竖条 ⇒ 玻璃块整个消失** → 竖条取中间调；**CPU 参考光栅器把 alpha 用了两次**（参考图整体偏暗）→ 修混合式 |
| 2026-10-22 | [切语言没反应（LCID 串 ⇒ 每帧重跑协商）](2026-10-language-switch-no-response.md) | 已修；平台层 `MUI_LANGUAGE_NAME` 写成了 `MUI_LANGUAGE_ID`（`0x4`）⇒ 拿到 LCID 串 ⇒ 协商得空表 ⇒ 判据「空表 = 还没协商」失效 ⇒ **每帧**把用户选的语言改回英文；两条缺一不可 |
| 2026-10-20 | [flex 折行：两趟量出两个宽度](2026-10-flex-wrap-two-pass-width.md) | 已修；显式 `width` 的容器折行用父上限而不是自己声明的宽度 ⇒ 行数错 ⇒ 内容叠在一起；端到端门禁直接量 `.zstyle` 驱动的几何 |
| 2026-10-20 | [画廊点了没反应（`DEVICE_RESET` 死设备）](2026-10-gpu-device-reset-dead-clicks.md) | 已通；死设备 ⇒ 每帧空帧 ⇒ 输入照常跑但画面冻结；恢复 = 重建整条 GPU 栈，限流每秒一次；6 条门禁全部不需要真 GPU |
| 2026-10-18 | [空壳控件（只有样式、点了没反应）](2026-10-hollow-controls.md) | 已通；审计出 30 个 key（4 类）；门禁按「扰动世界 + 树指纹」判定；顺带发现无字体布局的门禁看不见裁切 |
| 2026-10-18 | [离屏 pass + 14 档 PS 变体混合](2026-10-linux-vulkan-offscreen.md) | 已通；`未实现混合` 归零；四个 usage/loadOp/initial_layout 坑 + 「RT 装的是命令自己」；对照 Windows 95.79% 逐字节一致 |
| 2026-10-18 | [1:1 像素级达成（sRGB 取舍实测）](2026-10-linux-vulkan-parity.md) | 前 6 大颜色与 Windows 逐字节一致；UNORM 直通是正解 |
| 2026-10 | [Linux 裸 Vulkan 后端（wgpu → ash）](2026-10-linux-vulkan-raw.md) | demo/gallery 跑通；Entry 生命周期是 segfault 根因；14 档 PS 变体未接线（已记账） |
| 2026-10 | [Linux 适配（WSL2 实测）](2026-10-linux-port.md) | 已通；14 档 PS 变体混合模式未接线（已记账） |
| 2026-10 | [GPU 设备挂死（DEVICE_HUNG）](2026-10-gpu-device-hung.md) | 已定案（`ResizeBuffers` 本机不可用） |
| 2026-10 | [Chromium 合成器对照](2026-10-chromium-compositor-reference.md) | 参考 |

以下几份是**任务清单**格式，尚未改写成日志格式：

| 文件 | 内容 |
|---|---|
| [task-resize.md](task-resize.md) | resize 优化（交换链容量制） |
| [task-live-resize.md](task-live-resize.md) | 窗口拖动时实时帧 |
| [task-responsive.md](task-responsive.md) | 响应式断点 + 窄屏 |
| [task-anim-wire.md](task-anim-wire.md) | 动画接线 |
| [task-glyph-cache.md](task-glyph-cache.md) | 字形位图缓存 |

---

## 长期存在的问题

写在这里以免被当成「已完成」：

- **`begin_frame` 每帧 `wait_gpu()` 全等一次。** 根因是顶点/索引只有一份 resident
  缓冲。正解是环形多缓冲（像命令分配器那两槽按帧轮转）。这是唯一会动 GPU 资源生命
  周期语义的改动，要连带验证 resize / recreate，所以在真做过完整验证之前不动。
  详见 [`../developer/05-frame-pipeline.md`](../developer/05-frame-pipeline.md)。

- **D3D12 调试层本机现在可用（2026-10-24 更正）。** 曾记录「两条路都不可用」
  （事后 `EnableDebugLayer` 设备刚建就 HUNG；事前 `D3D12_CREATE_DEVICE_DEBUG`
  访问违例）。本轮 `command_validation` harness 走的是**第三条路**：
  `D3D12GetDebugInterface` → `EnableDebugLayer`（建设备**之前**）+ 挂
  `ID3D12InfoQueue`，9/9 绿、反向对照能读到 ERROR 级消息 —— 旧结论不再成立，
  保留在此以防又把「本机没有调试层」当成既定事实。

- **`docs/win/03-*.md` 缺号。** 原 `win/` 目录本来就是这个状态，没有漏搬。

- **触控/字体的几何判据依赖系统字体。** `gallery/tests/gates.rs` 的 `laid_out`
  用 `zephyra_fx::from_system_font()`（与 `--dump-cpu` 同源）排版，所以「内容一屏
  装得下」这条判据跟**本机字体度量**绑定：换一个行高更大的字体，某族就可能重新
  贴边。取不到系统字体时 `from_system_font` 内部有兜底（退化成旧的无字体度量），
  于是同一份门禁在无字体机器上会**变松**。当前各族的余量：`--dump-cpu` 逐族量
  可见（basics 726/800、status 743/800 最紧）。要摆脱这一点得让 core 支持
  「按字体度量留白」或者给参考页加滚动 —— 都还没做。

- **`cargo clippy --workspace` 现在干净。** 曾记过「`zephyra-vulkan` 12 处警告」，
  本轮实测 `-D warnings` 退出 0（那 12 处已在别轮修掉），本条改为记录现状。

- **`cargo doc --no-deps --workspace` 现在干净。** 曾有两处 broken intra-doc link：
  `zephyra-style/src/anim.rs` 的 `[`curve`]` 与 `zephyra-core/src/filter.rs` 的
  `[`Self::bezel`]`（都指向方法/字段而不是可链接项）。本轮各改一处（`Self::curve` /
  `` `bezel` ``），整个 workspace 文档生成退出 0，剩余 42 条是既有的链接解析警告。