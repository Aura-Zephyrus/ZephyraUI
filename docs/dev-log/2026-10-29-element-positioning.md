# 元素定位：坐标进样式表

## 现象

控件重写进行到「浮层 / 弹层」之前，缺一块地基：**没有任何办法把一个元素钉在
指定坐标上**。布局只有 Stack / Flex 两条正常流路径，元素位置永远由父容器的
游标决定。弹出的下拉、tooltip、悬浮按钮、右键菜单都无处安放。

同时，「某个元素现在在窗口里的哪儿」虽然能从 `Node::rect()` 拿到，但没有一个
按事件身份（key）查矩形的入口，应用得自己遍历树。

## 排查

CSS 的定位模型分两半：

1. **查**：`getBoundingClientRect()` 给元素在视口里的矩形。
2. **设**：`position` + `inset`（`left` / `top` / `right` / `bottom`）把元素钉住。

本仓的坐标系恰好只有**一个**：`Node::rect()` 已经是窗口绝对坐标
（`zephyra-core/src/frame.rs` 里明说 «`Node::rect()` 是绝对坐标（`layout::arrange`
从根的 (0,0,w,h) 往下传）»）。于是「绝对定位」在这里的自然语义就是
**直接给窗口坐标**——等价于 CSS 的 `position: fixed`，而不是 `absolute`
（后者相对最近的定位祖先）。

这正好对上真实用法：先查锚点的窗口矩形，再把弹层放到那个坐标上。如果按 CSS
`absolute` 做「相对父容器内边距」的语义，反而不合用——弹层通常挂在根附近，
父容器是谁取决于树的形状，应用不该被迫去算「我离父容器左边缘多少」。

## 排除的假设

- **「照搬 CSS `absolute`（相对最近的定位祖先）」** —— 需要 `position: relative`
  才能让父容器成为包含块，还要沿父链找最近定位祖先。本仓没有 `relative`，
  引入它等于同时引入「relative 保留空间但视觉偏移」和「包含块沿父链上溯」两套
  语义，而用户要的是窗口相对坐标。**排除**，改用 `fixed` 语义。
- **「未设 `left` / `top` 时回落到 CSS 的静态位置」** —— 静态位置要求维护一份
  「如果它在正常流里会在哪」的镜像游标；而绝对子项已被排除出正常流，算它要
  在 measure/arrange 两趟里各加一支。对一个每帧重建的布局趟，不值得。**排除**，
  回落到 `0`（窗口左上角）。真实用法里两个偏移总是成对出现。
- **「`position: relative` 也一并实现」** —— `TRANSLATE_X` / `TRANSLATE_Y` 已经
  覆盖了「视觉偏移但不重排」的需求（且它们是**只重绘**属性，动画友好）。
  `relative` 唯一的额外价值是「建立包含块」，而包含块这套在上一条已被排除。
  **排除**，`Position` 只留 `Static` / `Absolute`。
- **「绝对子项不进测量」** —— 第一版在 `measure_flex` 里直接 `continue` 掉了
  绝对子项，结果 `arrange` 读 `cached_size_or_zero` 拿到的是零尺寸，元素消失。
  绝对子项**必须测**（`arrange` 要读它的缓存尺寸），只是**不参与父内容尺寸**。

## 结论

`position: static | absolute` + `left` / `top`（DIP，窗口坐标）三件套：

- **core**：`layout::Position` 枚举 + `props::POSITION` / `LEFT` / `TOP`
  （三者都声明为**影响布局**）；`style::Node::position` / `is_absolute`。
- **布局**：`measure_stack` / `measure_flex` 仍**测**绝对子项，但不把它计入父的
  内容尺寸；`arrange` / `arrange_flex` 的正常流第二趟跳过绝对子项，之后由
  `arrange_absolute_children` 按 `Rect::new(left, top, w, h)` 摆位。
- **查询**：`Node::find(key)` 深搜（文档序，与 `collect_focusable` 一致）+
  `Node::find_rect(key)` 直接给窗口矩形。这就是「分别可定位元素，窗口相对位置」。
- **编译器**：`zstylec` 的 `props_map` 加 `position` / `left` / `top` 三条，
  `value.rs` 的 `enum_value` 加 `Position` 分支（只认 `static` / `absolute`，
  其它关键字报错）。

### 门禁

- `zephyra-core/src/layout_tests.rs`：
  - `absolute_子项的矩形直接采用窗口坐标`
  - `absolute_子项不把兄弟挤下去`（绝对子项不占正常流）
  - `absolute_未设偏移时落在窗口左上角`
  - `absolute_子项仍然绘制在其窗口坐标`（几何 → 绘制命令）
  - `默认定位是_static`
  - `find_rect_按_key_查元素的窗口矩形`
- `zephyra-core/src/layout_classification_tests.rs`：`Position` / `Left` / `Top`
  三条进了 `EXPECTED` + `DECLARED` + `NAMES`，且 `name_affects_layout` 同步。
- `zstylec/src/tests.rs`：`position_parses_static_and_absolute_and_rejects_others`
  （`absolute` 能解析、`sticky` 报错）。

### 边界

- 绝对子项的画序仍按树序（与兄弟同层）。要让浮层盖在正常流之上，得靠
  `LAYER` / 将来的 `Z_INDEX`，本轮没做。
- `left` / `top` 之外没有 `right` / `bottom` / 尺寸拉伸（`left + right` 定宽）。
- 未设 `left` / `top` 时落在窗口左上角，不是 CSS 的静态位置——见「排除的假设」。
