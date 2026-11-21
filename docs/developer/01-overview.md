# 总览

## 一句话

一个保留式、GPU 优先的 UI 框架。**核心负责决定画什么，宿主负责怎么画。**

## 数据流

```text
  Element 树   ──diff──▶  Node 树  ──measure/arrange──▶  DisplayList  ──▶ 像素
   (声明)         (保留)      (状态)                     (唯一契约)      (宿主)
```

五步各自回答一个问题：

| 阶段 | 回答 | 谁负责 |
|---|---|---|
| `Element` 树 | 应用**声明**了什么 | 应用代码，每帧新建，可整个丢弃 |
| `diff` | 上一帧的 `Node` 树和这一��有什么不同 | `zephyra-core`，逐节点对账 |
| `Node` 树 | 当前**状态**是什么（保留式） | `zephyra-core`，带脏标记 |
| `measure` / `arrange` | 每个节点**有多大、放在哪** | `zephyra-core`，纯几何 |
| `DisplayList` | 这一帧**要画什么命令** | `zephyra-core` 产出，宿主消费 |

`DisplayList` 是核心与后端之间**唯一**的契约。宿主拿到它，只回答另一个问题：
「我要不要把它推上屏」。

## 为什么分成这样

宿主**不允许**做三件事，详见 [`../zephyra/zephyra-base/README.md`](../../zephyra/zephyra-base/README.md)：

1. 不决定画什么 —— 画面内容全部来自 `DisplayList`
2. 不持有 UI 状态 —— 树和列表都在核心里
3. 不引回控件层 —— 免得 base 依赖 core 之外的东西

第2 条是最容易被破坏的。它一旦被破坏，「这一帧画什么」就会散落到多个地方，
而那正是本仓历史上 GPU 问题反复出现的根源（见
[`../dev-log/2026-10-gpu-device-hung.md`](../dev-log/2026-10-gpu-device-hung.md)）。

## 仓库结构

```text
zephyra/
  zephyra-core/      通用层：无外部依赖，#![forbid(unsafe_code)]
    property.rs      属性系统：定义、存储、继承、变更优先级
    props.rs         标准属性键（三方共用：布局/绘制/控件层）
    theme.rs         设计令牌 + Light/Dark
    element.rs       声明树（可丢弃）
    node.rs          保留树 + 脏标记
    diff.rs          Element→Node 对账 + DiffStats
    context.rs       只读上下文（主题 + 文本引擎）
    layout.rs        Measure / Arrange 协议
    paint.rs         paint::render 驱动（Node 树 → DisplayList）；数据类型来自 vector
    hit.rs           命中测试
    id.rs            NodeTag / ElementKey —— diff 的身份判定词汇
    frame.rs         帧缓存：跨帧存树与列表，回答「这一帧画什么」
    input/           指针事件词汇表、手势识别、视口变换、输入路由
  zephyra-vector/    矢量绘制库：core 与 fx 都依赖它；只依赖 ab_glyph + bytemuck
    core.rs/filter.rs/decoration.rs/paint.rs  绘制契约与几何基元
    geom/            路径几何：展平 / 描边 / 多孔耳切三角化
    icon/            SVG path 解析 + 图标注册表 + 内嵌字形
    text/            文本排版 + 字形图集 + ab_glyph 字体来源
    render/          顶点展开 / CPU 光栅 / 离屏规划 / 像素读写
  zephyra-fx/        效果器层：帧循环 / 调试桥 / 对 vector 渲染 API 的再导出
                     （零系统 API，与 base 并列而非其子目录）
  zephyra-base/      系统适配层：凡是要问操作系统才能回答的事
    zephyra-winit/     winit + D3D11 宿主
    zephyra-winit-d3d12/  winit + D3D12 后端（独立 crate）
    zephyra-dcomp/     DirectComposition 呈现后端（可选子包，Windows-only + unsafe）
    zephyra-platform/    平台无关的 Win32 抽象
    zephyra-platform-win/  Win32 API（unsafe + COM）
    extra/            「不是所有项目都需要」的包（可选子包，不进 default-members）
      zephyra-uia/       无障碍 / UI Automation
  zephyra-core/extra/   同上，但零依赖 + 无 GPU 可单测的纯逻辑
zephyra-camera/         已独立仓：https://github.com/Aura-Zephyrus/zephyra-camera
                        （摄像头采集与像素格式，主仓零依赖）
    zephyra-bezier/      可配置三次方贝塞尔缓动
zstyle-compile/      .zstyle 编译器（解析 + 检查）
ZephyraCLI/            已迁出为独立仓：https://github.com/Aura-Zephyrus/ZephyraCLI
                       （zstyle-compile / zypui-compile 编译库仍在本仓）
zephyra-gallery/       已迁出为独立仓：https://github.com/Aura-Zephyrus/ZephyraUI-gallery
                       （参考应用 + 控件层整树门禁，单向依赖本仓）
ZephyraUI/           控件层
```

### 三层的关系

- **core**：无外部依赖（唯一 workspace 依赖是 `zephyra-vector`）、`forbid(unsafe_code)`。不认识操作系统。
- **vector**：后端无关的绘制库。几何 / 图标 / 文本 / 顶点展开 / 光栅都在这里。
- **fx**：效果器。零系统 API；帧循环与调试桥在这里，绘制能力从 vector 再导出。
- **base**：凡是要问操作系统才能回答的事才在这里。它不认识控件。

## 下一步

- 想跑起来 → [快速开始](02-getting-started.md)
- 想理解为什么这么分层 → [核心概念](03-core-concepts.md)
- 想写样式 → [样式表](04-stylesheets.md)