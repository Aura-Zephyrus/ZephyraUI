# `.zypui` 写法统一（排版规范）

[07-declarative-ui](07-declarative-ui.md) 讲**语法**（这份文件里能写什么），这份讲
**排版**（该怎么排才跟仓里其余文件一样）。两者的边界很重要：

> 下面五条**没有一条是语义**。zypuic 按名字取属性（`XEl::attr`），`class` 写在行尾
> 与写在行首编出的字节码完全相同；`<if>` 挤一行与展开三行得到同一棵树。

正因编译器不管，「写法统一」没有任何天然兜底 —— 三份文件曾各写各的，改完也没有
东西保证不再漂回来。于是判据放成一条测试：[`zypuic` 的排版门禁](#门禁)。

## 五条规则

| # | 规则 | 违反时长什么样 |
|---|---|---|
| 1 | **一个开始标签独占一行** | `<TextLabel key="a"` 换行续 `text="…"` 再续 `level="…"/>` |
| 2 | **一行一个元素** | `<if cond="x"><PageBasics/></if>` 挤在一行 |
| 3 | **属性顺序：`key` → 内容 → 行为 / 结构 → `class` 最后** | `<container key="c" class="x" icon="gear"/>` |
| 4 | 缩进 2 空格（2 的倍数），无 tab、无行尾空白 | 3 空格缩进、行尾挂空格 |
| 5 | 非空行都以 `<` 开头 | 标签跨行的续行、裸文本 |

### 属性分段

| 段 | 属性 | 例 |
|---|---|---|
| 1 | `key` | `<container key="icon" …>` |
| 2 | **内容** | `text` `icon` `texture` `uv` `label` `title` |
| 3 | **行为 / 结构** | `interactive` `focusable` `activatable` `collapsed` `width` `height` `filter` `blend` `url` `level` `render-source` `cond` `var` `list` `index` |
| 4 | `class` | 恒在最后 |

分段判据是「**先说这是谁、再说它装什么、然后说它干什么，最后才是它的分类名**」。
`class` 恒在最后还有个实际理由：class 表达式是全行最长的一段（状态三元拼接，
动辄 100+ 字符），放行尾时不同行的差异集中在同一列，diff 一眼看得出；而且它是
**87.6% 的存量写法**（实测 185 个 `key`+`class` 元素里 162 个已在行尾），选改动
最小的方向。

### 写出来什么样

```xml
<!-- ✗ class 在中间，且后面还跟着属性 -->
<container key="swatch" class="swatch {fill}" filter="{filter}" blend="{blend}"/>

<!-- ✓ 内容 / 结构在前，class 收尾 -->
<container key="swatch" filter="{filter}" blend="{blend}" class="swatch {fill}"/>

<!-- ✗ 标签跨三行（行号从此与内容对不上） -->
<TextLabel key="guide-blurb"
  text="{tr('ui.counter')}"
  level="text-body"/>

<!-- ✓ 一个标签一行，哪怕它 150 字符 -->
<TextLabel key="guide-blurb" text="{tr('ui.counter')}" level="text-body"/>

<!-- ✗ 一行两个元素 -->
<if cond="@route == 'guide'"><PageGuide/></if>

<!-- ✓ 一行一个元素 -->
<if cond="@route == 'guide'">
  <PageGuide/>
</if>
```

## 与语义的两处易混点

1. **class 的「词序」是语义，class 的「位置」不是。**
   `Control.caption-btn.close.hovered` 这种多类选择器按 `.zypui` 里 class 属性的
   **词序**匹配（类序即匹配序，见 07）—— 本规范只管 class 属性排在行内第几位，
   **一个词都不许动**。移动属性时保持引号内内容逐字节不变即可。
2. **属性顺序不是语义。** 属性是按名字取用的映射，插入顺序连 parity 对照都不看
   （07 的「验收」一节写明「对照时按键名排序」）。

## 门禁

```text
cargo test -p zypuic          # 22 条，其中两条属于排版门禁
```

| 测试 | 干什么 |
|---|---|
| `style_gate_tests::repo_zypui_files_obey_the_layout_rules` | 扫**工作区全部** `*.zypui`（递归，跳过 `target` / `.git`），逐行跑五条规则；新文件自动纳入，不需要登记 |
| `style_gate_tests::layout_violations_are_actually_detected` | 判据自检：拿故意写坏的样例（含**带缩进**的）确认门禁真的会红，防止门禁自己坏了还全绿 |

违规输出是 `路径:行号: 规则N 说明 + 原文`，照着改即可 —— 五条都是机械改动，
没有一处需要判断力。行号按**源文件**算（注释是打码而不是删除，打码副本与原文
字节数、换行数逐一相等，所以注释里的 `<` 不会算成标签，行号也不会漂）。

### 归一化的实测账

2026-10-31 对三份存量文件做了一次归一化，改动与对照如下（可复现）：

| 项 | 数 |
|---|---|
| 归一化前门禁报违规 | **90**（规则2 跨行续行 20、规则3 跨行 11 + 一行两元素 18、规则5 class 不在末尾 41、规则4 key 不在首位 0） |
| 归一化后 | **0** |
| 改动性质 | 纯排版：属性多重集 / 标签多重集 / 注释条数与归一化前**逐项相等**（`controls` 642 属性 / 484 标签 / 19 注释，`shell` 839 / 651 / 14，`demo` 71 / 48 / 8） |

「与改动前语义等价」这条对照是必须的 —— 排版脚本第一版吞掉了跨行标签后面的下
一个元素，正是靠多重集比对当场抓到，详见
[2026-10-31-zypui-style-normalization](../dev-log/2026-10-31-zypui-style-normalization.md)。
