# 2026-11-03-glass-decorations-and-scale

> 玻璃页加「装饰」开关（三块玻璃的 `decoration` 整体开关）；core 新增 `scale-x` / `scale-y`
> （绘制时缩放，绕元素中心）；开关把手改成 fraction 平移 + 按压缩放，动画从「只有
> pressProgress」补齐。参照实现：`E:\AndroidLiquidGlass`（`LiquidToggle.kt` /
> `DampedDragAnimation.kt`）。

## 现象

- gallery 玻璃页只有折射参数（厚度/倒角/色散/折射率），没有任何入口展示 `Decorations`；
- core 只有 `translate-x` / `translate-y`（绘制时偏移），没有缩放 —— 开关把手的
  「按压时放大」「状态切换滑动」在 zephyra-ui 侧无法实现（按压只有 `pressProgress`，
  没有 knob 位置进度，且缩放无通道）；
- 参考页开关族的单选组把圆点画在 `radio-indicator` 的**左上角**（`controls-toggle.zstyle`
  的 `.radio-indicator` 没有居中规则，与 `.check-indicator` 的缺口一致）。

## 排查

- **decorations 的现成通道**：`entity` 绑定走 `Val::Raw(PropertyValue::decorations(...))`，
  `empty` 恒等则不进离屏 pass —— 开/关装饰就是 Raw 结构值绑定，零新管线。
- **scale 的落点**：`paint.rs::render_rec` 只承载「本节点外观」（背景 / 描边 / 图像 / 图标 /
  装饰）；偏移沿父链相加、带子树一起动。把 scale 做成「本节点外观」比"子树变换栈"
  改动面小得多：文字的字形四边形按元素原点缓存（`Command::Text` 也没有缩放字段），
  三后端顶点展开都不动。
- **非等比缩放**时圆角 / 描边宽度 / 图像边长只有一个标量 —— 取两轴较小值（与
  「圆角不超过半边长」同类降级）。
- **把手动画**：`pressProgress` 与 `fraction` 共用 `advance_toward`（指数逼近临界阻尼
  ω=√1000≈31.6/s，0.001 阈值钉死），与 DampedDragAnimation 的时间常数一致；位移 =
  `fraction × (44 − 2×5 − 14) = 20px`，缩放 = `1 + 0.5 × press`（原版 `pressedScale = 1.5`）。
- `.switch-track.on { main-align: end }` 与「把手由 translate-x 驱动」**双重位移** ⇒
  加 `.switch-track.on.animated { main-align: start }` 覆盖（`animated` 作为轨道分类词
  由调用方传入，与 `InfoBadge` 的 `severity` 同手法）。

## 排除的假设

- **「scale 应该沿父链累乘、带子树一起变」**：文字字形缓存按元素原点存、`Command::Text`
  无缩放字段，那样要给 Text 命令加缩放字段并改三后端 + 离屏 bounds —— 独立一轮。
  本语义钉死"只缩放本节点自己的外观"，`props::SCALE_X` 的文档里写清楚；子节点要缩放
  就自己再写一遍。不能静默退化成半套语义。
- **「缩放应该影响布局与命中测试」**：`affects_layout() == false` 是这个属性的全部意义
  （与 translate 同组）—— 否则按压动画每帧全树重排。`scale_change_repaints` 与
  `scale_does_not_affect_hit_testing` 钉住这条。
- **「velocity 拉伸（原版 scaleX /= 1 − velocity·0.75 那条）也要做」**：不做。它需要
  拖动手势与速度追踪，本仓开关是点击切换；按压缩放已可见地表达"被按住了"。
- **「非法 scale 截断成 1.0 以外的值」**（如负值镜像）：负值没有镜像语义
  （圆角会变反向外框），非有限值让顶点 NaN ⇒ 「整块不画」、症状离病因很远。
- **`.on` 与 `.on.animated` 两个类同时成立时位移翻倍**：这是本轮唯一需要"互斥"的组合，
  靠 `.switch-track.on.animated { main-align: start }` 覆盖 + gates 的 knob translate-x
  实测断言兜住。
- **把手绑定断了无人知晓**：模板属性是动态通道，绑定漏了 "switch 能点、状态会变、
  把手永远钉在左端" —— 全新增加门禁 `the_switch_knob_travels_with_the_fraction`
  （读树上 knob 的 `TRANSLATE_X`/`SCALE_X`，覆盖 off→on、press→1.5×）。
- **装饰开关绑定断了无人知晓**：新增门禁 `the_glass_decor_toggle_flips_the_panes_decorations`
  （读三块玻璃的 `DECORATIONS`：默认非恒等、点一下退回恒等、再点恢复）。

## 结论 + 门禁

**做了**

- core：`props::SCALE_X` / `SCALE_Y`（`PropertyKey::new`，不影响布局）、
  `Node::scale_x/scale_y`（非有限/`≤0` → 1.0）、`paint.rs::NodeDraw`（本节点变换；
  位移仍沿父链相加，缩放不累乘）；圆角/描边/图像边长非等比取 min 轴；文字不缩放
  （`text_command` 文档写明理由与代价）；`layout_classification_tests.rs` 补
  SCALE/TRANSLATE/DECORATIONS/RENDER_SOURCE 的 affects_layout 钉死。
- zypui / zstylec / zypuic：`translate-x` / `translate-y` / `scale-x` / `scale-y` 加入
  `BARE_ATTRS`、`instantiate.rs` 实例化（as_f32 失败 → 按「不设」降级）、`props_map.rs`
  `scale-x`/`scale-y` 映射（`.zstyle` 侧入口）。
- zephyra-ui（控件层）：`ToggleSwitch` 新增 `fraction` 进度（与 `press` 同一条
  `advance_toward` 指数逼近）、`fraction()` / `knob_x()` / `knob_scale()`、常量
  `KNOB_TRAVEL`（= 20px，与 `controls-toggle.zstyle` 度量对齐，测试钉住）与
  `KNOB_PRESSED_SCALE`（1.5）；模板 `knob` 接 `translate-x` / `scale-x/y`，
  `animated` 分类词落在轨道上。
- gallery：materials 页 GlassParams 下新增一行 **Decorations** 开关（默认开），
  三块玻璃的 `decoration` 由 `@glass_decorations` 驱动（`glass` ↔ `none`），
  i18n 三语言 `ui.decorations`；开关参考页把手改为绑定驱动。
- `controls-toggle.zstyle`：`.switch-track.on.animated { main-align: start }`；
  `.radio-indicator` 补 `main-align/cross-align: center`（**顺手修**：圆点此前画在左上角）。
- 文档：`docs/developer/animation.md` §4 的能力表补 SCALE 行。

**门禁**

- `paint_transform_tests.rs`：9 条新测试（缩放绕中心、布局盒不变、命中不受影响、
  不沿父链传播、非法值回退、min 轴标量、图标几何绕中心+uv 不变、改了会重画、
  文字逐字形不受影响）；`layout_classification_tests.rs` 四键钉死；
  `toggle_tests.rs` 5 条把手测试；`gates.rs` 2 条（knob 位移/缩放、玻璃装饰开关）。
- 四门禁：`cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets -D warnings`
  exit 0；`cargo test --workspace` 全绿（89 个套件）；`cargo doc --no-deps --workspace`
  exit 0，新增 0 条警告（既有 43 条均不在本轮文件里）。
- `--dump-cpu` 出图核对：materials 页装饰开关默认开（把手右移、蓝轨）；reference/switches
  页开关关着把手左端、单选圆点居中（修前左上角）。

**仍未做 / 边界**

- scale **不**作用于子树（含子节点上的文字）；文字要缩放需给 `Command::Text` 加缩放字段
  并改三后端顶点展开与离屏 bounds —— 记为独立一轮。
- `scale-x`/`scale-y` 是「本节点自己的外观」，**不是** CSS `transform`：子节点不跟随、
  命中测试按布局盒、文字不缩放。
- Velocity 拉伸（press 时按惯性增长）未实现。
