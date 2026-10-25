# 2026-10-24-command-validation-gates

> 现象：用户贴来两行运行时错误 —— `[zephyra-d3d12] 提交失败: Close 命令列表失败:
> 参数错误。(0x80070057)` 与 `[zephyra-d3d12] 开帧失败: Reset 命令列表失败`。
> 症状是画面永久冻结（外部看着像「点导航没反应」，像输入坏了）。

## 排查

根因族：**非法命令录进命令列表时静默通过，`Close()` 才回 `E_INVALIDARG`** ——
整帧提交失败，此后每帧 `Reset` 失败，画面冻结。而调试层的消息是**排队**的：
「没看到错误」完全推不出「没有错误」。本仓 D3D11 侧早有一条同型教训（三项
自检全绿、画面全错，驱动那句话一直躺在队列里没人取）。

所以本轮不修个案，而是把两个 Windows 后端的「驱动是否接受了这些调用」变成
**有证据的结论**：调试层开着回放每一帧，成功路径也抽干队列，零 ERROR 级消息
才算过。D3D12 侧夹具 `record_one`（begin → record → submit → wait → drain），
D3D11 侧立即执行、`render` 返回即抽干。

harness 当场抓出的真 bug（全部是当前 HEAD 存量）：

| # | 后端 | 问题 | 修法 |
|---|---|---|---|
| 1 | D3D12 | resolve 源停在 `RENDER_TARGET`（调试层 538：规格要 `RESOLVE_SOURCE`；代码注释「源不需要转换」与调试层矛盾） | `refresh_dst_copy` 与 `swapchain::resolve_msaa` 两处补源屏障对 |
| 2 | D3D12 | `gpu_barrier` 无空转守卫（before == after 是 ERROR 525；`refresh_dst_copy` 第一帧「副本→RESOLVE_DEST」即空转） | `before == after` 早退 |
| 3 | D3D12 | `replay` 无脑用 `self.samples` 建 PSO（614：MSAA 下离屏内容段画进单采样 RT_A，每个带滤镜元素都中） | `replay` 加 `samples` 参数，内容段传 1 |
| 4 | D3D12 | `DebugInfo::drain` 严重级比较写反（`>= ERROR` 收进 WARNING/INFO、恰好漏 CORRUPTION；820 清屏色提示挤进 severe） | 改 `<= ERROR` |
| 5 | 两端 | `unimplemented_blends` 计数器说谎：fx 按**命令**计、后端从不减（两个后端都实现着 14 个 PS 变体模式却都不减，计数常年虚高） | `DrawRun.commands` 记段覆盖命令数，两端按段执行时精确减 |
| 6 | D3D12 | resolve 尺寸守卫用 `min()`：只挡源大于内容，**挡不住源小于内容**（会录尺寸不匹配解析 → 冻死） | 改逐位相等 + 补两条守卫测试钉住 |
| 7 | D3D11 | `unimplemented_filters` 同样虚高：`render` 的 `redraw` 分支 absorb 之后少减一次（投影/折射每帧恒报 1） | 补 `saturating_sub(1)`（D3D12 侧同一处 10-24 已修，两边同构） |
| 8 | D3D11 | `drain_severe_messages` 一条消息都读不出：`GetMessage` 只给 32 字节，而**描述串紧随结构体之后一起写出** | 两段式（先空指针问尺寸再取），与 D3D12 `drain` 同款 |
| 9 | D3D11 | 离屏 RT 还挂在 PS 采样槽上就去设它当渲染目标：`OMSETRENDERTARGETS_HAZARD` WARNING 每帧刷（驱动替我们置空，但那是隐式行为） | `bind_offscreen` / `draw_pass_quad` 设 RT **之前**清 t0/t1；`draw_offscreen_content` 显式补绑 t1（此前吃残留） |

顺带修掉一条**存量红**：`shaders/zephyra.hlsl` 10-24 改过（折射那轮）但只重编了
`PSRefraction.ps.bin`，`PSMain.ps.bin` 还是 10-18 的 —— mtime 守卫
（`shader_assets_are_in_sync`）报「改了着色器但忘了重新编译」。用 fxc
10.0.26100.0 重编两侧全部 20 个入口，**输出与已提交的 bin 逐字节相同**
（git 无 diff）—— 证明陈旧 bin 内容本来就对（那轮 diff 只动 PSRefraction 本体），
缺的只是时间戳。

## 排除的假设

- **「反向对照用规格级非法调用就一定抓得到」** —— 否。第一版反向对照
  （4× 源 + `RENDER_TARGET` 目标的 resolve）在本机驱动上**容忍**：`Close()`
  成功、调试层沉默，与 dev-log 里「必炸」的记录矛盾。换成规格级非法的尺寸
  不匹配版才抓到（D3D11 侧用尺寸不匹配的 `CopyResource`，ERROR #284）。
  **同一错误不同驱动一个炸一个不炸** —— 这正是需要 harness 而不是「我记得它会炸」
  的理由。
- **「`GetMessage(0)` 回 `E_INVALIDARG` 是默认取回过滤器挡的」** —— 不够。
  压了「全部放行」的 `PushEmptyRetrievalFilter` 后照样失败。真因是缓冲区
  只给 32 字节：官方文档明写 `pMessageByteLength` 入参是「缓冲区大小，
  **含描述串**」，描述紧跟在 `D3D11_MESSAGE` 之后。旧注释里两个判断
  （过滤器归因、「只给 32 字节（对）」）都不对，已改写。
- **「`unimplemented_filters/blends` 恒为 1 是 fx 侧口径问题」** —— 否，是
  后端不减账（两侧同病）。fx 按命令计是对的，后端按段执行，少的只是
  「段覆盖几条命令」这个数（`DrawRun.commands`）。
- **「backend_parity 的 SoftLight 红是新改动引入的」** —— 否。`git stash`
  后在干净 HEAD 上复跑，同样 `差 87：D3D11=[138,95,73,255] D3D12=[51,51,51,255]`，
  既有分歧，与本轮无关。

## 结论

- 用户贴的 0x80070057 冻结链在当前 HEAD **仍可复现**（harness 的 glass page
  MSAA 场景 submit 当场失败），根因是上表 #1/#2/#3 这一族，已全部修复 ——
  真机 gallery 验证仍待用户执行。
- D3D11 侧此前**一条调试层消息都读不出来**（两段式缺失），「零 ERROR」断言
  恒真；修好后第一次真正读到消息，反向对照（非法 `CopyResource`）与
  9 条 WARNING（SRV 卫生）同时现形。
- 三端里 DX 两端从此有门禁；Vulkan 侧的验证层**管道**已就绪
  （`ZEPHYRA_VK_DEBUG=1` → `VK_LAYER_KHRONOS_validation` +
  `VK_EXT_debug_utils` 信使 + `DebugState::drain/errors`，镜像 D3D12 的
  `DebugInfo`），但**门禁测试没写**：本机无 WSL（`wsl.exe` 被安全策略挡死），
  离屏 framebuffer 那套跑不起来。设计已定（winit `run_app` 建窗 → `Gpu::new`
  → 离屏 image+framebuffer → `prepare`/`record_frame`/`queue_submit`/wait →
  断言零 ERROR 级消息，场景与 D3D11 harness 同构），待有 Linux 机时补。

## 门禁

| 门禁 | 钉子什么 | 跑法 |
|---|---|---|
| `zephyra-winit-d3d12/tests/command_validation.rs`（9 条） | 反向对照（尺寸不匹配 resolve 必须进调试层）+ plain / blur+shadow / separable blend / refraction（resolve 与 copy 两条路）/ 折射守卫（源大、源小都跳过而不是冻死）/ 同帧玻璃页 ×3 帧 | `cargo test -p zephyra-winit-d3d12 --test command_validation -- --ignored --nocapture` |
| `zephyra-winit/tests/command_validation.rs`（6 条） | D3D11 同构：反向对照（非法 `CopyResource` #284）+ 五条场景，每条都断言「读得到消息」且零 ERROR + 计数器为 0 | `cargo test -p zephyra-winit --test command_validation -- --ignored --nocapture` |
| `shader_assets_are_in_sync`（既有） | 20 个 shader 入口的 bin 必须比 `.hlsl` 新（本轮因它抓到重编译遗漏） | `cargo test -p zephyra-winit --test d3d11_draw` |

两条 harness 都断言「调试层确实挂上了」（`gpu.debug().is_some()` /
`info_queue.is_some()`），否则门禁恒真、直接红。
