# TODO — ZephyraUI

> 按轨道分组。✅ 表示已完成（保留最近几条做上下文），其余按优先级排序。
> 每项做完后在 dev-log（`docs/dev-log/`）记账，这里的条目只留结论与指针。

## 进行中：外部渲染源拓展接口（收尾）

### 1. Vulkan：渲染源钩子接线（真机轨道）

现状：`zephyra-vulkan` 对 `Command::Custom` 只诚实计数
（prepare 阶段 `Segment::Custom` 臂 `unhandled_custom += 1`，见
`zephyra-vulkan/src/renderer.rs`）。目标是与 D3D11/D3D12 对齐：
`register_hook` + 钩子真实执行 + 端到端像素验收。

**架构背景（决定了做法）**：Vulkan 后端是**两段式**——

- prepare 阶段（CPU）：`plan_segments` → 逐段展开几何 → 存进
  `SegmentPlan { Plain | Offscreen }`（此刻**没有**命令缓冲）；
- record 阶段：`record_frame(cmd, …)` 拿到**活的** `vk::CommandBuffer`，
  逐段回放计划记录进 cmd。

钩子需要的是**提交段的活命令缓冲**，所以落点在 record 阶段，
prepare 阶段只负责把 Custom 段保留下来。

**步骤**：

1. `SegmentPlan` 加变体 `Custom { index: usize }`——prepare 阶段的
   `Segment::Custom` 臂从「直接计数」改成 push 这个计划段（下标即命令
   下标，与 Offscreen 段同款约定；钩子执行时再从 display_list 取回
   `bounds`/`source`，不需要在计划里克隆）。
2. 定义 Vulkan 形状的 `RenderHook` trait + `HookCtx`（与 D3D11/D3D12
   同构但**不抽公共 crate**，两个 D3D 后端已论证过：上下文毫无共性）：

   ```rust
   pub struct HookCtx<'a> {
       /// record_frame 的活命令缓冲——钩子的命令记进同一帧。
       pub cmd: vk::CommandBuffer,
       /// 当前 render pass 的目标尺寸（宽，高）。
       pub size: (f32, f32),
   }

   pub trait RenderHook: Send + Sync {
       /// ⚠ 状态纪律：调用点在主 render pass **内部**；
       /// 管线自理（钩子自己 vkCmdBindPipeline）；
       /// 下一段回放自会绑自己的管线，无需恢复。
       fn draw(&mut self, ctx: HookCtx<'_>,
               bounds: Rect, source: &RenderSource);
   }
   ```

   Vulkan 没有 RTV 句柄概念，render target 由 render pass 的 attachment
   管——v1 钩子**只允许在当前 pass 内画**（`ctx.cmd` 就是 pass 内的记录点）；
   钩子想开自己的 pass / 换 attachment，属 v2 再议。
3. `Renderer` 加 `hooks: Vec<(Cow<'static, str>, Box<dyn RenderHook>)>`
   字段 + `register_hook(kind, hook)`（同 kind 覆盖，照抄 D3D12 版）。
4. `record_frame` 的 `SegmentPlan::Custom` 臂：查注册表 → 命中则
   `hook.draw(...)` + `report.custom_passes += 1`；未命中保持
   `unhandled_custom += 1`（现行为就是降级路径，不用改）。
5. **验收测试（真机）**：参照 `d3d12_draw.rs` 的
   `render_and_readback` 夹具（Vulkan 读回走 `vkCmdCopyImageToBuffer` +
   map），两条对齐 D3D 侧：
   - 已注册：钩子把 bounds 涂成纯红，读回**恰好** bounds 面积个红像素；
   - 未注册：全黑 + `unhandled_custom == 1`。
   - 勾子原语候选 `vkCmdClearAttachments` 带 rect 数组（与
     `ClearRenderTargetView` 的 pRects 同款逐像素确定）。⚠ 有一个必须
     实测确认的点：它是否受 scissor 影响——若受影响，改用最小 fill
     管线 + 全屏动态 scissor 兜底；判据本身不变。
   - 格式字节序先查清（Vulkan 侧选的 swap/离屏格式决定红是
     `[255,0,0,255]` 还是 `[0,0,255,255]`）。
6. **顺序约束**：本机无 WSL，Vulkan 改动只能 `cargo check
   -p zephyra-vulkan --all-targets` 编译级验证；像素级验收必须到用户
   Linux 机跑。可以代码先行（含 Windows 上编译通过 + 逻辑自查），
   验收与合并挂真机轨道；真机上先跑 `ZEPHYRA_VK_DEBUG=1`（管道已就绪、
   门禁未写）确认调试层无消息，再跑验收测试。

### 2. Vulkan：命令校验门禁（真机轨道，原 #7）

`ZEPHYRA_VK_DEBUG=1` 管道就绪但门禁未写。照 D3D11/D3D12 的
command_validation harness 移植：validation layer 消息抽干 + 断言零
ERROR。注意 D3D 侧踩过的 InfoQueue 进程级单例问题在 Vulkan 侧对应的是
messenger/callback 形态，串行锁是否需要按实测定。

## 后续（已排期未动工）

- **离屏自绘支持（v2）**：带非恒等滤镜的 Custom 进离屏段时，几何展开层
  画不出宿主内容（现在诚实计数 `unhandled_custom`）。v2 让离屏 pass
  也能调钩子画进 RT。依赖：三个后端钩子先全部落地。
- **自定义布局策略拓展接口（第二缺口）**：Measure/Arrange trait 化，
  允许宿主注入非流式布局算法。与渲染源同一「核心只搬运、宿主解释」
  哲学，设计前先盘点 layout 趟的缓存写回形状（node 枢纽）。
- **`.zypui` 渲染源语法**：`props::RENDER_SOURCE` 目前只能宿主 Rust
  侧 `.with(...)` 注入（样式表写不了 Custom 分配值）；若出现真实需求，
  给 `.zypui` 设计只读 kind 引用语法，payload 仍归宿主。

## 已完成（近期，详见 dev-log）

- ✅ 2026-10-25 外部渲染源拓展接口：`Command::Custom` + `RenderSource`
  （ptr_eq 判等保 diff 静止）+ fx `Segment::Custom` 保序（段数不变量
  2f+2c+1）+ D3D11/D3D12 `RenderHook` 注册表 + 两侧像素级端到端验收
  （D3D11 `ClearView` / D3D12 `ClearRenderTargetView` pRects）。
- ✅ 2026-10-25 core 结构审计与 API 收口；`.zypui` 声明式结构全量落地。
- ✅ 2026-10-25 SoftLight Pegtop 修复 + 压力门禁；D3D 调试层命令校验
  门禁 26 条（8 个存量真 bug）。
