# 2026-11 布局引擎全量重构 + `<box>`/定位（R0–R4 进展）

> 主计划：`C:\Users\Levy\.opencode\plan\zephyra-box-layout-positioning-plan.md`（D1–D19 冻结）。  
> 本文件记录 R0 基线/盘点，以及 2026-11-08 在本工作区完成的 R1/R2/R3/R4 进展（含门禁与阻断）。**保留工作区中与本次任务无关的未提交改动。**

## 范围（R0）

- 对齐计划 R0：记录实际可运行的 `--dump-cpu` 基线；盘点旧契约与现存缺陷；为将要改变的语义提交 `#[ignore = "R2: ..."]` 的**目标行为**测试（不提交“断言当前错误行为”的绿色测试）。
- 旧行为/缺陷写在本节与下方“排除的假设/现存缺陷”，作为 R2 对拍与回归判定依据。

## 基线（`--dump-cpu`，CPU 参考光栅，无 GPU）

测量条件（2026-11-08，本机 Windows，`zephyra-gallery` dev profile）：

| 命令 | 路由/族 | 结果（stdout 尾部摘录） |
|---|---|---|
| `cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route guide` | `guide` | 带滤镜命令 0；四边形 213、顶点 852（字形 199）；矢量图标 `Path` 15、三角形 454；字形图集 64 个 2.1%；内容像素 269187/1120000（24.0%），不同颜色 1146；输出 `D:\code\ZephyraUI\gallery-cpu.bmp` |
| `cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route reference --family 0` | `reference/buttons` | 带滤镜命令 4；四边形 444、顶点 1776（字形 404）；矢量图标 `Path` 21、三角形 527；字形图集 91 个 2.0%；内容像素 318965/1120000（28.5%），不同颜色 1639；输出覆盖同一路径 |
| `cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route materials` | `materials` | 带滤镜命令 7；四边形 328、顶点 1312（字形 281）；矢量图标 `Path` 15、三角形 454；字形图集 82 个 2.0%；内容像素 318810/1120000（28.5%），不同颜色 1735；缺失纹理 1；输出覆盖同一路径 |
| `cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route slider` | `slider` | 带滤镜命令 1；四边形 199、顶点 796（字形 181）；矢量图标 `Path` 15、三角形 454；字形图集 64 个 2.0%；输出覆盖同一路径 |

说明：`--dump-cpu` 每次覆盖 `gallery-cpu.bmp`；R2/R3 若需保留对拍图，应在运行后复制到带路由/提交标识的文件名。

## 旧契约盘点（R2/R3 必改）

- **`HitTestRequest.ignore_above`**：定义 `zephyra/zephyra-core/src/hit.rs:47`；构造/引用含 `zephyra/zephyra-core/src/hit.rs`、`zephyra/zephyra-core/src/input/router.rs:216`、`zephyra/zephyra-core/src/input_tests.rs`（多处）、`zephyra/zephyra-core/tests/paint_transform_tests.rs`（2 处）、`gallery/tests/gates.rs:990`。D13：R1 删除，v1c 再按明确语义加回。
- **`Node::find_center` / “left/top 指中心”文档**：`zephyra/zephyra-core/src/node.rs:280,296`；D7 清理，`center()` 保留为布局几何中心语义需与新定位语义对齐。
- **`Node::is_absolute` / `Position::Static|Absolute`**：`zephyra/zephyra-core/src/style.rs:33`、`zephyra/zephyra-core/src/layout.rs`（`Position`/分支）、`layout_tests.rs` 绝对定位用例、`props`/`zstylec` 映射。D1 改为 `flow|parent|root`，`static/absolute/fixed` 报错。
- **translate/scale 不影响命中**：`paint_transform_tests.rs`（`offset_does_not_affect_hit_testing`、`scale_does_not_affect_hit_testing`）、`props.rs`/`zypui` IR/`paint.rs`/`layout_classification_tests.rs` 中相关注释与断言（R0 盘点项）。D2 改为影响 paint+hit+damage（scale self-only）。
- **gallery hit gate 用布局 rect 中心**：`gallery/tests/gates.rs::hit_test_agrees_with_the_painted_rects`（`n.rect()`/中心点）。D8/D2 后需改用视觉几何契约。
- **UIA 屏幕矩形用 `arranged_rect`**：`zephyra-uia/rules.rs::screen_rect`（R0 盘点项）。D8/D11 后改用视觉矩形。
- **`zypuic` 小写/`<container>`**：当前白名单外裸名已报错（`zypuic/src/build.rs:234`），`_` 分支对未知标签是死分支；`<container>` 仍在白名单且测试/fmt/style_gate 夹具含 `<container>`。D5/D12/R3 需移除 `<container>`、给迁移提示、未知裸标签显式报错。

## 现存缺陷/行为特征（写入 dev-log，不提交断言旧错行为的绿测）

- Stack 与 Flex 脱流缺轴回落路径不一致（D6 将统一为参照原点回落）。
- `left/top` 当前是“中心窗口坐标”语义 + `find_center`/文档混用风险（D7/D9）。
- `translate/scale` 命中与 paint 不一致；hit 读布局 rect，paint 读变换（D2/D8/D13/D14）。
- hit 顺序与 paint `(layer,z,order)` 不一致；`descend` 反向子节点早返回；`ignore_above` 未读取（D13/D14）。
- root/视口测量约束缺失；measure 缓存键不含 viewport（D15）。
- 文字不随 scale 缩放，但脏区/视觉边界处理需与 D17 对齐；文字 bounds 在 hit/paint 前可能取不到（`ctx.engine`/`glyph_quads` 时序，R1 阻断）。
- `layer: popup` `.zstyle` 解析缺失；`LAYER` 当前 `PropertyKey::new`（不可继承）且 paint/hit 各自按节点读取，弹层子树有效层有风险（D18，R1 阻断）。
- `NodeTag::Container` 与 `<container>`/大写类型选择器迁移面大（D5/D12，R3 单独提交）。

## 排除的假设

- **不假设** R0 能在不改 API 的前提下覆盖 D13/D14/D18/D17 的全部目标测试：`HitNode.inside`、`effective_layer`、节点级 `preorder`、统一文字 bounds fallback 等需要 R1/R2 API；R0 只提交当前 API 可表达且 `#[ignore = "R2: ..."]` 的目标测试，其余写明阻断原因。
- **不假设** `--dump-cpu` 的 BMP 在 R2/R3 语义变更后应字节一致：D2/D6/D8/D11/D13/D14 会改变几何/命中/层序相关行为，需区分“预期变化”与回归（以计划与 spec 为准）。
- **不假设** 可为“让计数好看”删 `#[ignore]` 或把目标测试改成断言旧错行为的绿测；R4 必须清零 `R2:` ignore。

## R0 目标测试（已由 R2 落地/清理）

- R0 曾添加 `paint_transform_tests` 的 `translate_y_affects_hit_testing_target_d2`、`scale_affects_hit_testing_target_d2`（`#[ignore = "R2: ..."]`）。R2 实现视觉命中后，这两条现为**通过的常规测试**（`cargo test -p zephyra-core --test paint_transform_tests`，2026-11-08，Windows，28 passed / 0 failed / 0 ignored）。
- `layout_tests.rs` 中 R0 记录的 `absolute_without_offsets_falls_back_to_reference_origin_target_d6` 在当前工作区未检出（`git grep absolute_without_offsets_falls_back_to_reference_origin_target_d6` 无输出）；D6 语义已在布局/`zstylec` 侧按 `flow|parent|root` 迁移，门禁以现有 `zephyra-core`/`zstylec` 测试与位置解析行为为准。
- **R4 检查**：`git grep -n 'ignore.*R2\|#\[ignore = "R2'` 全仓无输出（2026-11-08）。

## R2 实现（核心几何/命中/脏区/绘制）

### 变更

- 新增 `zephyra/zephyra-core/src/visual.rs`：共享 `VisualState`、`enter`（D11 `position: root` 清除祖先累计 translate，**自身** translate 仍生效）、`effective_layer`（D18：显式 `LAYER` 优先，否则沿祖先上下文传播）、`visual_rect`（布局矩形 + 祖先 translate + 本节点 scale；并集 `text_visual_bounds`）、`damage_bounds`（视觉边界 + filter/decoration outsets）、`text_visual_bounds`（D17：优先 glyph quad cache，必要时经 `ctx.engine` shape；无引擎时不额外外扩文本）。
- `hit.rs`：改用 `visual::enter`/`visual::visual_rect`；候选选择使用 `effective_layer` + 节点 preorder（全局计数）+ 全局 best；移除旧的本地 transform 复制；`HitNode.rect` 为视觉矩形。
- `frame.rs`：脏区扫描改用 `VisualState` 与 `visual::damage_bounds`；D11 边界随 `enter` 生效；force-full 条件仍在非恒等 translate/scale 时保守触发（无旧矩形快照时避免残影）。
- `paint.rs`：`PaintState` 增加有效 `layer`；`descend` 在 `position: root` 时清除祖先累计 translate（D11），并通过 `visual::effective_layer` 传播 D18；渲染层使用 `state.layer`。
- `layout.rs`/`node.rs`：R2 测量缓存键包含 `(Constraints, viewport)`（`LayoutEnv`/`cached_measure` 路径）；修复 stale rustdoc 链接 `cached_for` → `Node::cached_measure`/`clear_dirty` 说明。

### 测量/测试（Windows，本机，2026-11-08）

| 命令 | 条件 | 结果 |
|---|---|---|
| `cargo test -p zephyra-core --lib` | dev/unoptimized+debuginfo | 434 passed, 0 failed, 0 ignored |
| `cargo test -p zephyra-core --test paint_transform_tests` | 同上 | 28 passed, 0 failed, 0 ignored（含 D2/D11/D18 目标测试） |
| `cargo test -p zephyra-core` | 全目标（含其余集成/单测） | 通过；doc-tests 1 ignored（既有 `Element::styles`） |

新增/验证的 R2 测试包括：`visual::tests::root_position_clears_ancestor_translate_for_visual_bounds`；`paint_transform_tests::{root_position_clears_ancestor_translate_for_hit_testing_d11,popup_layer_propagates_to_subtree_for_hit_d18,translate_y_affects_hit_testing_target_d2,scale_affects_hit_testing_target_d2,translate_affects_hit_testing_visual,scale_affects_hit_testing_visual}`。

## R3 实现（词汇/工具链/仓内迁移与 D8 fallout）

### 变更

- `zstylec`/`zypuic`/`zypui`/`zephyra-style`：lowercase 裸标签与类型选择器语义、`NodeTag::Box` 词汇、`position: flow|parent|root`、`anchor`/`layer` 相关解析与迁移诊断（按计划 D1/D5/D12/D19）；BARE_ATTRS 保持 `translate/scale`/`width`/`height` 等既有运行时/例外属性，**未**放开布局定位属性为 `.zypui` 裸属性（D19 一致）。
- 仓内 `.zstyle` 迁移：`Container/Text/Control/Image` 类型选择器迁移为 `box/text/control/image`；修复 `examples/controls.zstyle`、`ZephyraUI/controls-text.zstyle` 的排版门禁（`zstylec fmt`）。
- `gallery` 测试迁移：`gates.rs`/`anim_wiring_tests.rs`/`hover_layout_tests.rs` 中 selector/断言随词汇更新；修复 `HitNode.id` 由 `ElementKey` 改为 `Option<ElementKey>` 后的格式化。
- `ZephyraCLI`：scaffold 生成的 `app.zstyle` 使用 `text {}`；`cli.rs` fmt 用例改为 `control.x`；`cargo test -p zephyra-cli`（Windows，2026-11-08）全绿（含 `scaffolded_stylesheet_passes_its_own_check`）。
- `zephyra-uia`（D8/D11 fallout）：`rules::build(root, win, ctx)` 现在接受 `Context` 并用 `visual::VisualState`/`visual::visual_rect` 计算屏幕矩形；更新 `zephyra-uia` 单测、集成测试，以及 `zephyra-winit`/`zephyra-winit-d3d12` 的 UIA 快照发布调用点（R2 API fallout 的必要同步，最小改动）。

### 测量/测试（Windows，本机，2026-11-08）

| 命令 | 结果 |
|---|---|
| `cargo test -p zstylec` | 49 passed, 0 failed, 0 ignored（含 repo `.zstyle` 排版/IR/零诊断门禁） |
| `cargo test -p zypuic -p zypui` | 通过（zypuic 42 passed；zypui 16 passed） |
| `cargo test -p zephyra-style -p zephyra-fx` | 通过 |
| `cargo test -p zephyra-ui` | 通过 |
| `cargo test -p zephyra-uia` | 2 passed, 0 failed（集成/UIA 行为测试） |
| `cargo test -p zephyra-cli` | 通过（`cargo test -p zephyra-cli` 全目标） |
| `cargo test -p zephyra-gallery --test gates` | 58 passed, 0 failed, 0 ignored |

R3 词汇扫尾（按“只清代码与门禁”口径）新增两条仓内门禁：

- `zstylec::tests::style_gate_tests::every_repo_zstyle_parses_without_diagnostics`
  —— 全仓 `.zstyle` 必须零诊断；旧类型选择器（`Container`/`Text`/`Control`/`Image`）
  与旧定位值（`position: static|absolute|fixed`）会立刻让这条红。
- `zypuic::tests::fmt_tests::repo_zypui_files_use_no_removed_container_tag`
  —— 全仓 `.zypui` 不得再出现已移除的小写 `<container>` 标签。

同时把 `zstylec` `fmt` 测试夹具里的 `Control` 选择器改为 `control`（夹具不再示范旧词汇）。
注释与历史 dev-log 里的旧词按约定**保留**（作为迁移记录）。

## 排除的假设（补充）

- **不假设** `translate/scale` 的视觉命中/脏区与旧 `--dump-cpu` 像素、旧布局 rect 中心命中完全一致：D2/D8/D11/D17/D18 属于计划内语义变更，回归判定以 `docs/developer/layout-spec.md`（D1–D19）与 R2 目标测试为准。
- **不假设** 文本视觉外扩在所有情况下都能精确到字形包围盒：D17 在无有效 glyph cache 且无 `ctx.engine` 时退化为“不额外外扩文本，仅用节点布局/视觉矩形”；有引擎或缓存时 union 文本 bounds。
- **不假设** 全仓 `--workspace` 门禁在存在与本任务无关的未提交改动时天然全绿：本工作区存在用户未提交的 `zephyra-winit`/ext/render-ext、`Command::shape`、入场缓动等改动；本轮为**跑通全仓门禁**只做了机械性 fallout 修复（见“门禁”一节的清单），未改动这些改动的设计语义。
- **不假设** `cargo doc --workspace` 的总警告数可作为“无回归”判据：AGENTS 规定“判据是本轮有没有新增”；Windows-only/既有私有链接警告与本轮 R2 变更无关的部分应单独看待。

## 结论

R2 已在 `zephyra-core` 落地共享视觉/脏区几何与 D11/D17/D18 相关行为，并清零仓内 `R2:` ignore。R3 已完成核心词汇/工具链迁移与主要 fallout 修复（含 gallery gates、CLI scaffold、UIA D8 调用签名/视觉矩形），并新增“全仓 `.zstyle` 零诊断 / `.zypui` 无 `<container>`”两条门禁。R4 的全仓四条门禁**现已全部通过**（见下）；其中为跑通 workspace 只做了机械性 fallout 修复，未改变无关未提交改动的设计语义。

## 门禁（R2–R4，本次变更，Windows，2026-11-08）

### 全仓四条（全绿）

```text
cargo fmt --all -- --check
  # 退出 0（stable 输出 nightly-only wrap_comments warning，属环境噪音，见 AGENTS）

cargo clippy --workspace --all-targets -- -D warnings
  # 退出 0

cargo test --workspace --no-fail-fast
  # 退出 0；99 个 test-result-ok 批次，0 failed（既有 ignore 仍保留）

cargo doc --no-deps --workspace
  # 退出 0；warning 43 条 —— 与 AGENTS 记录的 warm 全量基线 43 条一致，本轮无新增
```

### 点检（同轮次）

```text
cargo test -p zephyra-core -p zstylec -p zypuic -p zypui -p zephyra-style -p zephyra-fx -p zephyra-uia -p zephyra-cli
  # 通过：zephyra-core lib 434、paint_transform 30、zstylec 49、zypuic 42、zypui 16、zephyra-uia 2 等，0 failed
cargo test -p zephyra-gallery --test gates
  # 58 passed, 0 failed, 0 ignored
```

### 为跑通全仓门禁所做的机械性 fallout 修复（仅此，未改设计）

- `zephyra-uia::rules::build` 增加 `Context` 并改用视觉矩形（D8/D11）；同步 `zephyra-winit`、`zephyra-winit-d3d12` 调用点与 UIA 测试。
- `zephyra-core/src/layout.rs` 的 stale `cached_for` 链接改为 `Node::cached_measure` / `clear_dirty` 说明。
- `zephyra-winit/src/ext.rs`：`D3D11ExtContext` 及其 `new` 收为 `pub(crate)`（外部无引用；消除“类型比 API 更私有”的编译错误）。
- `zephyra-winit/src/renderer.rs`：`Arc<RefCell<..>>` 的 clippy `arc_with_non_send_sync` 处 `#[allow]`；`HookCtx.ext_pool` 收为 `pub(crate)`。
- 用户未提交的 `Command::shape` 新字段导致 `zephyra-winit` 三个测试文件 41 处 `Command::Quad/Image` 初始化缺字段：按既有测试惯例补 `shape: CornerShape::ROUND`（**测试数据**，非实现语义）。
- `gallery/tests/gates.rs` 三处 `rules::build` 补 `ctx`。
- `gallery/tests/anim_wiring_tests.rs::no_class_name_after_tier_exhausted`：用户未提交的入场缓动 `PointToPoint → AppleMenu` 使 16 ms 首帧直接落到 `enter-2`；按“1 ms 采样”修测试（保留 AppleMenu，不写死曲线），仍断言 8 档可达 + 放完回到 `None`。

### 仍然留在工作区的无关未提交改动（本轮未触碰其语义）

`zephyra-winit`/`zephyra-render-ext`/`zephyra-ext` 的 ext 重构、`paint::Command::shape`/CornerShape、`zephyra-style::anim::AppleMenu`、gallery 下拉菜单 motion 等。提交时应与本轮 R2/R3 变更分开成独立 commit。