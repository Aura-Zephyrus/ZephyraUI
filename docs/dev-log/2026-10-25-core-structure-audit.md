# 2026-10-25-core-structure-audit

> 现象：`.zypui` 迁移收官后做 core 结构梳理（为「拓展接口」设计定基线）。
> 审计三件事：模块依赖方向、pub API 面、`lib.rs` 架构文档与实际的偏差。

## 依赖审计：真实依赖图与「node 枢纽」

代码级扫描（剥注释、含内联路径）得到的依赖图，对照 `lib.rs` 宣称的单向流
（element/node → style → layout → paint），有**三处双向引用**，全部围绕
`node`：

| 双向 | 原因 | 判定 |
|---|---|---|
| `node ↔ layout` | `node` 的测量缓存引用 `layout::Constraints/MeasureResult`；写入方是 `layout` 的测量趟 | 缓存写回，非违规 |
| `node ↔ paint` | 文本成形缓存（`TextureRef`/`GlyphQuadCache`）同理，写入方是 `paint::render` | 缓存写回，非违规 |
| `props ↔ paint` | `props::LAYER` 默认值用 `paint::Layer`；`paint` 读 `props::TEXT_COLOR` | 键集中地引用了一个绘制类型，拆出去不抵动静，接受 |

结论：**`node` 是跨帧缓存的枢纽，驱动模块把结果写回它的缓存字段**——这是
「缓存写回」的形状，不是分层违规。已把这条写进 `lib.rs`（含日后拆驱动的
切口：协议类型下层、驱动上层）。

## 文档与实际对齐（lib.rs）

- 模块表漏了 `responsive` / `i18n` / `styler` 三行 → 补全。
- 「控件库与模板系统尚未实现」已过时 → 改为「控件层是 `.zypui` 组件模板
  （上层 crate `zypui`/`zypuic`）」。

## 语义归位：`control_fill_role`

「状态 → 填充色资源键」的翻译函数原本住在 `props`，产出的却是 `theme::roles`
表里的键名——`props` 因此反向依赖 `theme`。挪进 `theme::roles::control_fill_role`，
`props` 回归「只声明属性键」。全仓唯一调用点（`examples/demo_tests.rs`）同改。

## pub API 收口（零引用且无级联的 8 项删除）

扫描方法：先全 workspace 符号引用扫（113 项外部零引用），再分「core 内部在用
（保留，多数受 pub 签名 E0446 约束动不了）/ 死代码候选」，候选逐一核实定义，
**文档提及不算使用**。两处方法论坑：

1. **前缀掩码**：`content_size` 被 `content_size_of` 的出现掩盖成「有人用」；
   严格词边界复查后两个都真死——`measure_stack` 早已把流式累计**内联**，
   `content_size_of` 的文档还谎称「布局趟走这个流式版」。
2. **props 键必须 pub**：zstyle/zstylec 按字符串名解析、样式表按名字声明，
   符号扫描天然漏掉——属性词表不进收口范围。

删除清单（全部零调用、零级联）：

| 项 | 备注 |
|---|---|
| `props::{fill_props, border_props, size_props}` | 便捷构造，.zypui 迁移后控件层不再手搓属性 Vec |
| `style::{is_selected, is_disabled}` | 同族 `is_interactive` 有内部用户，这两个零 |
| `layout::Stack::{content_size, content_size_of}` | `measure_stack` 内联后遗留，文档失实 |
| `paint::content_rect_is_drawable` | 为宿主设计的跳过判据，从未接线；实际决策在 `zephyra-fx` 的 `plan_offscreens` |
| `i18n::language_only` | 回退链由 `distance` 内联实现，方法壳零调用 |

**保留**（有出处/有对称价值/是设计缝）：`context::{light,dark}_context`
（宿主便捷构造对）、`responsive::TOUCH_*`（带标准出处的判据表，跨项目价值）、
`property::custom` / `id::ElementKind`（设计好的扩展缝，见下节）。

## 剩余收口清单（不动，留待下一步）

约 100 项「仅 core 内部用」的 pub 项：绝大多数出现在 pub fn 签名里
（`Constraints`/`Size`/`MeasureResult`…），降 `pub(crate)` 会连锁 E0446；
真正的净收益只有零散几个纯内部助手，批量改动不抵风险。

## 扩展缝盘点（为「拓展接口」设计定的基线）

现状已有的缝：`ElementKind`（设计完零实现）、`PropertyKey` 公开构造
（宿主自定义键，demo 的 NOTE/Z_INDEX 在用）、`TextEngine` trait、图标注册表、
`styler::Styler` 注入点、`PropertyValue::custom`。明显缺的缝：DisplayList
命令闭集（Quad/Image/Text/Path，宿主无法注入自绘）、布局策略闭集
（Stack/Flex/Grid 三条硬编码路径）。

## 门禁

`cargo test -p zephyra-core` 337+10+6+16+7 全绿；`cargo check --workspace
--all-targets` 0 error；`clippy -p zephyra-core` 零 warning。
