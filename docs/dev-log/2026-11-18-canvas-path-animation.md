# Canvas 式动态路径动画能力

> 2026-11-18 · `zephyra-core` / `zypui` / `zypuic` / `gallery`
> 起因：BusyIndicator 的旋转弧线此前用 24 帧 SVG 采样冒充动画，用户要求真正的路径动画能力。

## 现象

`BusyIndicator` 用 24 枚预生成 SVG 图标轮换 —— 离散、不平滑，而且**每帧生成顶点**
这件事本来就该由框架支持，而不是让应用层假装成图标。

## 实现

打通「宿主每帧算几何 → 模板绑定 → core 产出 `Command::Path`」这条链（三个后端早已
通用消费 `Command::Path`，无需改后端）：

1. **`zephyra_core::paint::DynamicPath`**：`Arc<[Vertex]> + local: bool`。
   `local=true`（默认）表示顶点是节点局部坐标，绘制时先加 `node.rect()` 左上 ——
   宿主只知道「我这控件 56×56、圆心 (28,28)」，不知道自己被布局到哪，局部坐标是
   自然写法。`absolute()` 构造器给已知道绝对位置的场景。
2. **`props::PATH_DATA`**：`PropertyKey::new("Path.Data", StaticStr(""))`，paint-only
   （不进布局分类），与 `RENDER_SOURCE` 同律：真值走 `PropertyValue::Custom`，
   样式表写不了、主题解析跳过。
3. **`Node::path_data(ctx)`** → `style.rs` getter；`paint::render_rec` 新增一条
   `Command::Path` 产出分支，颜色取 `icon-color`，并把它计入 `has_own_content`
   （否则带滤镜时会被补一条空 Quad，开两次离屏）。
4. **zypui**：`BARE_ATTRS` 加 `path`；`instantiate.rs` 加
   `"path" => Val::Raw(pv) => with(&props::PATH_DATA, pv)`（表达式语言没有路径字面量）。
   zypuic 无需改（直接读 `BARE_ATTRS`）。
5. **gallery**：`BusyIndicatorState` 新增连续相位方法 `phase_ms()` / `start_angle()` /
   `sweep_angle()`（照 `ProgressRing.qml:89-100` 两段动画）；`Gallery::catalog_busy_path()`
   每帧按相位生成「外弧 → 末端圆帽 → 内弧反向 → 起点圆帽」的一条闭合轮廓，
   经 `geom::triangulate` 变成顶点，塞进 `Val::Raw(custom(DynamicPath))`。

MCP 截图复核：旋转弧线现在是一条平滑的蓝弧、圆端、随相位连续伸缩旋转。

## 排除的假设 / 踩坑

❌「`geom::stroke_outline` 能直接产出弧线轮廓」—— 实测对开放弧线（无论 Butt/Round
   帽、Bevel/Round 接头）产出的轮廓都被 `triangulate` 判为自交而拒绝
   （`NoUsableContour`）。手工构造单条闭合轮廓（外弧 + 圆帽 + 内弧 + 圆帽）才稳定可三角化。

❌「顶点可以直接给 paint」—— `NodeDraw::point` 期望**绝对布局坐标**；局部坐标会被
   当成绝对坐标画到窗口左上角。这就是 `DynamicPath.local` 存在的原因。

❌「`PropertyValue::Custom` 的 Debug 能区分几何」—— 一律打印 `Custom`；不要拿路径
   绑定当 `<for>` 的 track key（`val_to_cache_key` 会字符串化，全部碰撞）。

## 门禁

- 新增单测：`dynamic_arc_annulus_triangulates_to_vertices`（弧线三角化）、
  `busy_indicator_advances_one_arc_frame_per_step`（连续相位）。
- 属性分类门禁 `layout_classification_tests` 三处补齐 `PATH_DATA` / `Path.Data`。
- `cargo test --workspace`：**1736 passed / 0 failed**；fmt 干净、clippy 0 警告、doc 退出 0。
- 旧的 24 枚 `busy-arc-N.svg` 保留作兼容（模板已不用），不影响门禁。
