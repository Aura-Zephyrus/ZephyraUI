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
<box key="swatch" class="swatch {fill}" filter="{filter}" blend="{blend}"/>

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
cargo test -p zypuic          # 排版门禁 + `fmt` 的用例
```

| 测试 | 干什么 |
|---|---|
| `style_gate_tests::repo_zypui_files_obey_the_layout_rules` | 扫**工作区全部** `*.zypui`（递归，跳过 `target` / `.git`），逐行跑五条规则；新文件自动纳入，不需要登记 |
| `style_gate_tests::layout_violations_are_actually_detected` | 判据自检：拿故意写坏的样例（含**带缩进**的）确认门禁真的会红，防止门禁自己坏了还全绿 |
| `style_gate_tests::fmt_fixes_exactly_what_the_gate_checks` | **门禁与排版器对齐**：每条规则各给一个坏样例，`fmt` 修出来的形态必须同时过门禁（防「跑完 fmt 门禁还红」） |
| `style_gate_tests::fmt_recognizes_every_repo_file` | 存量文件 `is_formatted` 都为真，即「跑一次就修好」那条路是通的 |

违规输出是 `路径:行号: 规则N 说明 + 原文`，照着改即可 —— 五条都是机械改动，
没有一处需要判断力。行号按**源文件**算（注释是打码而不是删除，打码副本与原文
字节数、换行数逐一相等，所以注释里的 `<` 不会算成标签，行号也不会漂）。

**判据住在 `zypuic::fmt::violations`**，门禁只是把它逐文件跑一遍，排版器
（`zypuic fmt` / `zephyra fmt`）用的是同一份代码。上一轮判据与排版是两份实现，
于是门禁自己少报了一半 class 违规（靠拿旧内容回跑才抓到）。**一份实现**才不会有
这个失败模式。

## 让排版器改

```text
zephyra fmt            # 整个工程：.zypui + .zstyle + cargo fmt（Rust）
zephyra fmt --check    # 只报不写，给 CI 用（有要排版的退 1）
zephyra fmt shell.zypui
zypuic fmt shell.zypui # 编译器自己的入口，同一条判据
```

排版器只做三件事，全部**不改引号内的任何字节**：跨行的开始标签合成一行、一行
挤了多个元素时摊开、属性重排成 `key` 在前 `class` 收尾。缩进按元素深度重算
（2 的倍数），CRLF 统一成 LF，文末单个换行。

看不懂的源码（XML 语法不干净）**原样返回**，绝不猜一份排版覆盖上去，`--check`
把它报为脏。

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

### 排序往返的金标准

上一个脚本是临时的（PowerShell、一次性），本轮做成了编译器的一部分，并且判据
升级成**编译产物**：`fmt_tests::repo_files_survive_a_mangle_and_reformat_roundtrip`
把每份存量文件**只动位置**地弄乱一遍（去掉缩进 + 把 `class` 挪到最前），再交给
`fmt`，要求：

1. 整理后的源码与原件**逐字节相同**（幂等 + 不会引入新 diff）；
2. 连同其余 `.zypui`（组件表是跨文件的）一起编译，产物与原件**逐字节相同**。

产物相同是比多重集更强的判据 —— 位置与空白**不在**产物里留任何痕迹
（属性按名写表、树形与位置无关），所以任何一处差异都意味着排版器动到了内容。
