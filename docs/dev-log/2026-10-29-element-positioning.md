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
- **「未设 `left` / `top` 时回落到 `0`」** —— 第一版这么做过，但用户要求「X、Y 可
  单独赋值或计算」：只给 X 时 Y 应当由布局算，而不是钉到窗口上缘。**排除**，
  改成逐轴回落到正常流本位的中心（栈路径精确、flex 用容器原点）。
- **「`position: relative` 也一并实现」** —— `TRANSLATE_X` / `TRANSLATE_Y` 已经
  覆盖了「视觉偏移但不重排」的需求（且它们是**只重绘**属性，动画友好）。
  `relative` 唯一的额外价值是「建立包含块」，而包含块这套在上一条已被排除。
  **排除**，`Position` 只留 `Static` / `Absolute`。
- **「绝对子项不进测量」** —— 第一版在 `measure_flex` 里直接 `continue` 掉了
  绝对子项，结果 `arrange` 读 `cached_size_or_zero` 拿到的是零尺寸，元素消失。
  绝对子项**必须测**（`arrange` 要读它的缓存尺寸），只是**不参与父内容尺寸**。

## 结论

`position: static | absolute` + `left` / `top`（DIP，**窗口坐标 + 元素中心**）三件套：

- **core**：`layout::Position` 枚举 + `props::POSITION` / `LEFT` / `TOP`
  （三者都声明为**影响布局**）；`style::Node::position` / `is_absolute`。
- **布局**：`measure_stack` / `measure_flex` 仍**测**绝对子项，但不把它计入父的
  内容尺寸；`arrange_flow`（栈）与 `arrange_absolute_children`（flex）按
  `Rect::new(cx - w/2, cy - h/2, w, h)` 摆位——`left` / `top` 指元素**中心**。
- **逐轴独立**：设了的轴直接赋中心坐标；没设的轴沿用正常流本位的中心（栈路径用
  正常流游标、flex 用容器内容原点）。于是 `left: 120;`（只给 X）横向钉住、
  纵向仍随兄弟排——正是 CSS `top: auto` 的行为。
- **查询**：`Node::find(key)` 深搜（文档序，与 `collect_focusable` 一致）+
  `Node::find_rect(key)`（窗口矩形）+ `Node::center()` / `find_center(key)`
  （窗口中心，与 `left` / `top` 同语义）。
- **编译器**：`zstylec` 的 `props_map` 加 `position` / `left` / `top` 三条，
  `value.rs` 的 `enum_value` 加 `Position` 分支（只认 `static` / `absolute`，
  其它关键字报错）。

### 为什么坐标是「中心」而不是左上角

`left` / `top` 的典型用法是把一个弹层/提示**居中叠在锚点上**。若按左上角，
应用每次都要自己减半个尺寸；按中心则「查锚点中心 → 写进 left/top」一步到位，
与 `Node::find_center` 对称。这是刻意的、与 CSS 的差异（CSS 的 `left` 是左边缘）。

### 门禁

- `zephyra-core/src/layout_tests.rs`：
  - `absolute_子项的矩形直接采用窗口坐标`（中心 → 左上角换算）
  - `absolute_只设_x_时_y_沿用正常流位置` / `absolute_只设_y_时_x_沿用正常流位置`
    （逐轴独立）
  - `absolute_子项不把兄弟挤下去`（绝对子项不占正常流）
  - `absolute_子项仍然绘制在其窗口坐标`（几何 → 绘制命令）
  - `absolute_在嵌套容器里仍是窗口坐标`（父偏移不累加）
  - `默认定位是_static`
  - `find_rect_按_key_查元素的窗口矩形`
- `zephyra-core/src/layout_classification_tests.rs`：`Position` / `Left` / `Top`
  三条进了 `EXPECTED` + `DECLARED` + `NAMES`，且 `name_affects_layout` 同步。
- `zstylec/src/tests.rs`：`position_parses_static_and_absolute_and_rejects_others`
  （`absolute` 能解析、`sticky` 报错）。

### 与 DComp 的关系

本仓 DComp 呈现路径只有**一个** visual、挂着整帧后缓冲（见
`zephyra-dcomp` 与 `zephyra-winit-d3d12/src/host.rs` 的 `attach_visual`），
所以「所有元素坐标转化为窗口绝对坐标」这件事在**绘制阶段**就已经成立：
`paint::render_rec` 用 `node.rect()`（窗口绝对坐标）+ 累积 `translate` 出命令。
DComp 只是把这张已经按窗口坐标画好的图合成上屏，不做逐元素偏移叠加。

因此「嵌套容器里的绝对子项仍是窗口坐标」这条门禁是本轮对 DComp 语义的直接钉住：
如果哪天改成逐元素 visual 树，坐标必须是窗口绝对坐标而不是父-子相对累加。

### 边界

- 绝对子项的画序仍按树序（与兄弟同层）。要让浮层盖在正常流之上，得靠
  `LAYER` / 将来的 `Z_INDEX`，本轮没做。
- `left` / `top` 之外没有 `right` / `bottom` / 尺寸拉伸（`left + right` 定宽）。
- `left` / `top` 指中心，与 CSS 的「左边缘 / 上边缘」不同。
