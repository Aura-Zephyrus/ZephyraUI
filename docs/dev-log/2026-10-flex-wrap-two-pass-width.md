# flex 折行：两趟量出两个宽度 ⇒ 行数不同 ⇒ 容器高度与真实行数不符

> 现象：给 flex 容器写死 `width: 340px`（`.zstyle`），折行**看起来**折了，
> 但第二行的内容被压在第一行上面 —— 不裁切、不报错，就是叠着。

---

## ★ 结论（先看这段）

**flex 的折行判定必须在两趟里用同一个宽度。** 排列趟拿的是父摆进来的那个
矩形（`inner.width`），测量趟当时拿的是**父约束的上限** —— 于是容器声明
`width: 340px` 而父容器允许 1139px 时：

- 测量趟按 1139 折行 ⇒ 判定**单行** ⇒ 内容交叉尺寸 = 一行高 = 56；
- 排列趟按 340 折行 ⇒ 真实是**两行** ⇒ 第二行画在 y=66。

而容器只有 56px 高（测量趟算出来的），第二行整个落在盒子外面。

修法：`measure_flex` 里，容器**自己声明的主轴长优先**于父上限，并且要像
`pick_axis` 那样**先夹进父给的区间再扣内边距** —— 父上限比 `width` 小时摆出来
就是父上限，按一个永远不会出现的宽度折行同样是错的。

门禁：`zephyra-core` 的
`layout_tests::flex_wrap_breaks_lines_against_the_container_own_width_not_the_parent_max`。

---

## 排查

症状是「文字叠在一起」，排查方向有十几种（字号、行高、裁切、缩放、
图集…）。决定性的一问是：**容器自己报的高度是多少？**

`rect().height == 10`（一行）而子节点的 `y == 20`（第二行）—— 这两件事
不可能同时为真，除非**两趟用了不同的宽度**。顺着这个看 `measure_flex`
就能看到 `flex.available_main(deflated)` 拿的是父约束上限。

## 排除的假设

| 曾经的怀疑 | 排除方式 |
|---|---|
| 「是 zstylec 没解析 `width: 340px`」 | 解析是好的：同一个值让容器**外**宽确实变成 340（`rect().width == 340` 断言过）。错的是内部折行用的数 |
| 「是 `align-content: stretch` 把行拉出去了」 | stretch 分配的是**容器已经给出的**交叉空间，不改变行数。把 `align-content` 显式写成 `start` 症状不变 |
| 「是 `.card` 的 padding 把子项顶出去了」 | 容器上没有 padding；把子项换成裸 `NodeTag::Control` 症状一样 |
| 「是测量缓存没失效」 | 第一帧就错，不是「改了之后不刷新」。缓存失效走 `Dirty::Layout`，与本条无关 |

---

## 为什么这条容易再犯

两趟设计本身是对的（grow/shrink 只能在排列趟做，否则容器尺寸随 grow 变 ⇒
约束变 ⇒ 缓存永不命中 ⇒ 静止界面每帧全树重排，见 `measure_flex` 的函数文档）。
但「两趟」就意味着**任何跨趟共享的量都必须同源**。这一条漏掉的地方不是算法，
是**取值顺序**：`pick_axis` 有明确的「显式 > 内容，再夹进区间」顺序，而新加的
折行判定没有跟上，于是同一个 `width` 在两趟里被解释成了两个数。

---

## 门禁

| 测试 | 钉住什么 |
|---|---|
| `zephyra-core` `layout_tests::flex_wrap_breaks_lines_against_the_container_own_width_not_the_parent_max` | 显式 `width: 100` 在 300 宽的父里 ⇒ 必须得两行、容器高 30、第三项落在 y=20。三条一起断言：宽度被夹、行数对、行位对 |
| `zephyra-gallery` `gates::the_flex_demos_on_the_layout_page_really_allocate` | 端到端：`.zstyle` 的 `flex-direction` / `flex-wrap` / `flex-grow` 真的走到几何。grow 格比固定格宽 100px 以上；第四格换行且 x 归零 |

第二条值得单独说：样式表那一侧的门禁只查「规则有没有选中元素」，不查它有没有
**效果**。所以「样式表写着 `flex-grow: 1`，画面上纹丝不动」这种断链，只有直接
量最终几何才抓得住。
