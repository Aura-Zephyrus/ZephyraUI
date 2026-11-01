# 后端与替换点

「凡是要问操作系统才能回答的事」才在 base 层。core 不认识操作系统，base 不认识控件。

## 三条契约

写在 [`zephyra/zephyra-base/README.md`](../../zephyra/zephyra-base/README.md)，是这一层的全部约束：

1. **消费 `DisplayList`** —— 画面内容全部来自它，宿主不自己决定画什么。
2. **不决定画什么** —— 不持有 UI 状态，也不靠读 `dirty()` 来决定跳不重绘。
3. **不引回控件层** —— base 依赖 core，不依赖控件。

第 2 条被破坏过，后果见
[`../dev-log/2026-10-gpu-device-hung.md`](../dev-log/2026-10-gpu-device-hung.md)。

---

## 现有的三套宿主与呈现组件

| crate | 图形 API | 呈现 |
|---|---|---|
| `zephyra-winit` | D3D11 | 窗口交换链 |
| `zephyra-winit-d3d12` | D3D12 | DComp（`CreateSwapChainForComposition`） |
| `zephyra-vulkan` | Vulkan | Vulkan surface / swapchain（Linux / Android） |
| `zephyra-dcomp` | — | DirectComposition 呈现组件（D3D11 可选；D3D12 宿主使用） |

D3D11 与 D3D12 是**两个独立 crate**，互不依赖。换后端不用改核心。
三者的输入、调试桥与输出能力并未完全对齐，当前缺口见
[后端能力 TODO](../todo/backend-capability-audit.md)。

### 无标题栏窗口

在**消费 ZephyraUI 的应用项目根目录**创建 `ZephyraUI.json`，并设置：

```json
{
  "window": {
    "frameless": true
  }
}
```

三个宿主（D3D11、D3D12、Vulkan）从应用进程的**当前工作目录**读取该文件，不会去框架仓库或 crate 源码目录查找。省略文件或省略 `window.frameless` 时保留系统默认装饰；无效 JSON 会向 stderr 报告并回退默认窗口配置。该选项只改变 winit 建窗属性，不改变视口、渲染或呈现路径。无系统装饰窗口不会自动获得本仓自绘标题栏、窗口拖动或缩放控件；需要这些交互时由应用自行实现。

---

## 换后端要改哪里

完整清单见 [`migration-seam.md`](migration-seam.md)。最短路径：

| 要做的事 | 改哪 |
|---|---|
| 换图形 API | 实现消费 `DisplayList` 的那一层即可，core 一行不用动 |
| 换窗口系统 | 同上——窗口和渲染是两件事 |
| 加一个能力 | 先问：能不能在 core 里做完？只有真的要问操作系统才放 base |

---

## 关键设计：链只保证「≥ 客户区」，不跟窗口变

这是本仓最容易看错的一条，也是闪屏问题的根。

交换链**不随窗口尺寸重建**。窗口变化时只改**视口**，本帧照常用新尺寸排版绘制 ⇒
逐帧、清晰、无缩放。DComp 按内容自然尺寸 1:1 画在原点，窗口只显示左上角那块。

只有**装不下**时才重建，且一次给 25% 余量（`CAPACITY_HEADROOM`）。

为什么不用 DComp 缩放铺满：缩放会让画面被拉伸，而**宁可显示不全也不能显示错**。

去抖、熔断、`resize_settled` 这些机制**全部不存在**——容量制一旦成立，它们要解决的
问题就不存在了。

---

## Chromium 对照

本仓的 DComp 路径对照 Chromium 的
`components/viz/service/display_embedder/software_output_device_win_swapchain.cc`。

差异值得记一笔：Chromium 的 `ResizeDelegated` 每次 resize 都调 `ResizeBuffers`，而
**本机实测 `ResizeBuffers` 在后缓冲被用过后就再也调不成功**，稳定回 `0x887A0001`。

四种成因逐一排除过（`wait_gpu` / `release_command_list_refs` / 双槽 allocator.Reset /
`recreate_command_slots` 整体销毁重建），各自验证后保留的修复仍在，但结论不变：这条路
在本机走不通，所以选了容量制。

完整实测记录见 [`../dev-log/2026-10-gpu-device-hung.md`](../dev-log/2026-10-gpu-device-hung.md)。

---

## 平台概念

- [窗口属性（DWM）](win/01-window-attributes.md)
- [窗口透明模式](win/07-window-transparent.md)
- [系统主题通知](win/04-sys-theme.md)
- [flip model](win/05-flip-model.md)
- [交换效果](win/06-swap-effects.md)