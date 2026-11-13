# 调试桥的「AI  ergonomics」：树里没有文字、`wait_frames` 干等 2 秒、引号被吃

> 现象：AI 拿着调试桥问「那个写着『纯色』的开关在哪」，答不上来；点完按钮等
> 「画面稳定」，每次要等 2 秒；而往输入框里打 `a  b`（两个空格），打进去只剩一个。

这一轮没有新功能的需求，是**拿 CLI 当 AI 用一遍**，把过程中卡住的地方找出来补上。
三个卡点分别落在三个层：库层协议、宿主侧计时、CLI 侧传参。

- 现象（一）：树里每行是 `path tag key x,y w*h [类名]`。gallery 的**每一个**导航项
  标签的 key 都叫 `label` —— 所以「哪个是『快速开始』」这个问题，**看树看不出来**。
  文字确实在节点里（`props 0.1.0.0.2` → `Text = 快速开始`），但那要**逐个**
  `props` 才看得见：一次 O(n) 次往返的排查。
- 现象（二）：`wait_frames 5` 在**静止**的应用上要等满 2 秒才回话（回复是
  「waited only 0 of the requested frames」）。AI 的典型动作是
  「点一下 → 等画面稳定 → 截图」，于是**每次**都白等 2 秒。
- 现象（三）：`zephyra bridge 'type_text "a  b"'` 打进输入框的是 `a b`（一个空格）。
  而 `type_text` 在协议层是「行文本」，带引号的一段必须是**一个**参数。

## 排查

### （一）树里没有文字

先确认不是「树 dump 漏了」：直接问应用
`zephyra bridge "props 0.1.0.0.2"` → `Text = 快速开始`。文字在节点的
**本地属性** `props::TEXT` 里。

所以缺的不是数据，是**树 dump 的那一行没带它**。

为什么当初没带：树的列是给「抄坐标」用的（path / tag / key / 坐标 / 类名），
每一列都对应「点击时需要什么」。而**文字**对应的是「人看得见的那个东西」——
在键不唯一的界面里（`label` 到处都是），那才是定位的依据。当时没想到这一层。

修法（`zephyra-fx/src/bridge.rs`）：

- `node_text(node)`：从节点的属性里取 `props::TEXT`（本地优先于样式层）。
  比**地址**不比名字 —— `Text` 与 `Text.Size` 是不同的键但名字前缀相同。
- 树文本形态在**最后**加一列 `"文字"`（带引号）；JSON 形态加 `"text"` 字段。
  加在最后 ⇒ 前面几列形状不变，老的读法还能用。
- 文字里的 `\n` **必须**转义：协议是行文本，一个裸换行会把一条命令劈成两条 ——
  症状是「应用收到了莫名其妙的命令」，而没人知道为什么。

顺带补了一条**按文字找**的命令 `find <文字>`：它给的不只是文字节点自己的路径，
还有 `owner` —— **最近的可点祖先**（带 key 且不是纯文本节点的那个）。

`owner` 这一栏是第二轮才发现要加的：第一次实现取「最近的带 key 的祖先」，
结果每个文字节点自己就带 key（`label`），于是 `owner` 永远是文字节点自己 ——
**点它多半没反应**（真正可点的是它的父控件）。这条只有**真的用**才能发现：
看代码时「带 key 的祖先」读起来完全合理。

### （二）`wait_frames` 干等 2 秒

先量：`wait_frames 5` 在静止的 gallery 上，实测 **320 ms**（改后）；改前是
**2000 ms**（= `WAIT_FRAMES_TIMEOUT` 整个耗尽）。

为什么这么设计：协议的 `wait_frames` 数的是**渲染过**的帧（不是「跑过」的帧）——
静止的应用**永远出不了**那么多帧，所以必然超时。超时回复是诚实的，但没有用：
调用方要的是「画面稳定了」，不是「我等了 2 秒」。

改成三种结局，每一种都说清楚实情（`zephyra-fx/src/bridge.rs` 的
`PendingWait`）：

1. **达标**（帧数到了）⇒ `waited N rendered frames`。
2. **应用静止**（`IDLE_WINDOW = 250 ms` 内一帧都没出）⇒
   `the app settled (rendered 0 of the requested 5 frames, then went idle)`。
3. **超时**（既没画够、也不算静止）⇒ 老话术。

250 ms 的依据：60fps 一帧 16.7 ms，250 ms ≈ 15 帧。慢于这个帧率（< 4fps）的
应用在交互排查里基本等于「卡住了」，而那正是「静止」要表达的事。

⚠ 这里的**顺序**有讲究：先判「达标」，再判「有进展」（刷新静止判据的起点），
再判「静止」，最后判超时。把「有进展」放在「静止」前面，是因为一个正在动画的
应用会**先**有几帧进展再停 —— 顺序反了会把「动画进行中」误判成「静止」。

### （三）引号被吃

分两层，各有一处：

- **库层**（`zephyra-fx::bridge::parse_command`）：原来用
  `rest.join(" ")` 把参数拼回去。`type_text "a  b"` 的 `rest` 是
  `["\"a", "b\""]`，join 之后变成 `"a b"` —— **两个空格压成一个**。
  改成用**整行余下部分**（`line[cmd.len()..].trim_start()`），不重新拼。
- **CLI 层**（`ZephyraCLI/src/commands/bridge.rs`）：客户端要摘掉自己的选项
  （`--out` / `--rect` / `--scale`），原来是「按空白切分 → 去掉 → 再 join 回去」
  —— 同样的坑。改成**按词重建**：一个词是一个整体，词**之间**用单空格，
  词**内部**（含引号里的连续空格）一字不动。

摘选项时还有一条边界：`--out` 只对 `shot` 生效。别的命令里 `--out` 可能是
**内容**（`type_text "--out"` 要把那三个字打进输入框）。无差别地摘会静默改写
用户要输入的内容。

### （四）改窗口尺寸：winit 的返回值是个**假信号**

补 `resize <宽> <高>`（验「窄窗口下这个布局还成立吗」）时，第一版按
`request_inner_size` 的返回值判成败 —— 它回 `Option`，于是第一版**每次都
报「失败」**。

而窗口**其实改好了**：实测 `resize 800 600` 之后视口确实变成了 800×600
（物理像素）。

查 winit 0.30.13 的 Windows 后端源码：

```rust
pub fn request_inner_size(&self, size: Size) -> Option<PhysicalSize<u32>> {
    ...
    None        // ← 无条件
}
```

**永远**回 `None`。那个 `Option` 在 Windows 上**不是**「成功/失败」的信号
（别的平台才是）。拿它当判据 ⇒ 每次都误报失败，而动作其实生效了。

另一处单位陷阱：winit 收的是**逻辑**像素，而桥的坐标系全是**物理**像素
（`viewport` 明说 physical-pixels、树的坐标是物理、点击坐标是物理）。
125% 的屏幕上 `resize 800 600`（当成物理）实际给出的是逻辑 640×480 =
物理 800×600 —— 正好对；但如果命令按**逻辑**发，就会差一个缩放因子。
所以 `resize` 的单位定成**物理**，宿主内部按 `scale_factor()` 换算回去。

### （五）整文件换行翻转：一行内容改动变成 8787 行 diff

`cargo fmt --all`（本轮为了过门禁跑的）之后，`gallery/tests/gates.rs` 的
`git diff --stat` 是 **8787 行**，而 `git diff -w` 只有 **3 行** —— 也就是说
**整份文件**在 diff 里被重写，实际内容只动了 3 行。

判据是换行符：HEAD 里该文件是 **CRLF**（4393 行全是 CRLF），工作区里成了
**LF**（4394 行全是 LF）。`git diff --ignore-cr-at-eol` 把这条差异消掉 ⇒
差异**只**在 CRLF/LF 上。

⚠ 排查时**别**用 `git show HEAD:path | grep -c $'\r'` 判 —— MSYS 的管道层会
翻译换行，永远给假答案（本仓 dev-log 里已经记过一次同样的坑）。要**字节准**
地看：

```powershell
cmd /c "git cat-file blob HEAD:path > %TEMP%\x.rs"   # cmd 重定向是字节准的
# 再数 \r\n 与裸 \n
```

**结论**：仓库里的文件**换行符不统一**（有 CRLF 也有 LF）。任何一次「整文件
重写」都会把它变成一条几千行的假 diff —— 提交时会把真正的改动淹掉。

处理：把工作区文件的换行符**改回 HEAD 的样子**（只动换行，不动内容），再
`cargo fmt --all -- --check` 验一遍 —— rustfmt 的 `newline_style = Auto`
**保留**文件原有的换行，所以 CRLF 的文件保持 CRLF，门禁照样绿。

做完这一步，本轮的总 diff 从 **8098 增 / 5128 删** 降到 **3549 增 / 579 删**。

## 排除的假设

- ❌「树里没有文字是 dump 漏了 props。」——不是。`props` 命令能看到
  `Text = 快速开始`；缺的只是**树那一行**没带它。
- ❌「`wait_frames` 慢是因为桥的 poll 频率低。」——不是。桥每帧都 poll；
  慢的是**语义**：静止的应用不出帧，于是必然走满超时。
- ❌「`wait_frames` 改成「等到静止」就会误伤慢应用。」——会，所以给的是
  「静止」这个**结论**而不是「结束」这个动作：回复里写明
  `rendered 0 of the requested 5 frames, then went idle`，读的人知道
  **没有**等到那么多帧。
- ❌「引号被吃是 PowerShell 的问题。」——第一次实测确实是（`\"` 在
  PowerShell 的双引号串里不是转义），但**改用单引号后仍然被吃** ——
  那是库里与 CLI 里**各一处**真 bug。教训：复现步骤里**先排除掉工具的引号
  规则**，否则会把工具的坑当成代码的坑（也可能反过来）。
- ❌「`owner` 取「最近的带 key 的祖先」就够。」——不够。文字节点自己带 key
  （`label`），所以那个答案永远是文字节点自己，而它多半**点不动**。
  要「带 key **且不是**文本节点」。
- ❌「`click_key label` 能点中。」——不能。key **允许重复**（`For` 的位置化
  item 就可能同名），`click_key` 取第一个。所以命中测试给的可点路径必须用
  **路径**，不能用 key。
- ❌「`request_inner_size` 回 `None` = 改窗口失败。」——错。winit 0.30.13 的
  Windows 后端**无条件**回 `None`（源码里就是 `None`，不是 `Err`）。第一版
  据此报错，而窗口**其实改好了**（实测视口 1400×800 → 800×600）。
  `Option` 在这里**不是**成败信号。
- ❌「`resize` 用逻辑像素更自然（winit 就是这么收的）。」——不。桥的坐标系
  **全是**物理像素（`viewport` / 树 / 点击），这里换单位就要 AI 自己在两套
  单位之间换算。定成物理，宿主内部按 `scale_factor()` 换算回去。
- ❌「`gallery/tests/gates.rs` 那 8787 行 diff 是内容大改。」——不是。
  `git diff -w` 只有 3 行；差异全在 CRLF/LF 上（HEAD 是 CRLF，工作区成了 LF）。

## 结论

- **树要带文字。** AI 看到的是「屏幕上写着什么」，不是「这个节点的 key 叫什么」。
  键不唯一时（`label` 到处都是），只有文字能区分。
- **按文字找元素要有 `owner`。** 文字节点通常不是可点的那个；`owner` 给的是
  最近的可点祖先。
- **`wait_frames` 要认「应用静止」。** 「画面稳定了」是排查动画类问题后
  第一个要问的问题，而静止的应用永远不会「等到那么多帧」。实测
  2 s → 320 ms。
- **协议里的引号参数是一个整体。** 引号内的连续空格必须逐字节保住 ——
  库层与 CLI 层各修一处。
- **命中测试给路径，不给 key。** key 可以重复，路径不会。
- **`resize` 的单位是物理像素**，且**不看 winit 的返回值**（Windows 上恒为
  `None`）。真机实测 1400×800 → 800×600，gallery 的导航随之收成
  `compact`（56 px 宽）—— 响应式断点确实触发了。
- 另修一处小谎：MCP `screenshot` 的回执报的是**线上**字节数（BMP），
  而落盘的是 PNG（大小不同）—— 报「3360054 bytes」而文件只有 743039 字节，
  会让人以为文件被截断。改成报**落盘后**的大小。

## 门禁

- `zephyra-fx`（160 条）：
  - `tree_carries_the_visible_text` —— 树必须有 `text` 字段。
  - `inline_text_escapes_control_characters` —— `\n` `\r` `\t` `"` `\\`
    的转义与反解。
  - `find_locates_elements_by_visible_text` —— `find` 命中，且 `owner` 是
    **父控件**（`0.0`）而不是文字节点自己（`0.0.0`）。
  - `hit_test_finds_the_most_specific_element` —— 命中最深那个，且
    `clickable` 指向可交互节点。
  - `type_text_takes_the_rest_of_the_line` / `type_text_reaches_the_host` ——
    带引号含空格是一个参数，且真的到了宿主。
  - `wait_frames_reports_that_the_app_settled_instead_of_hanging` ——
    静止的应用**快**回（不许拖到超时）。
  - `wait_frames_answers_only_after_the_app_rendered_enough` ——
    帧数不够时**不许**提前回话。
  - `resize_takes_physical_pixels` —— 宽高是**无符号**的，负数会绕成天文数字
    （`resize -1 -1` 必须在解析层挡住）。
- `zephyra-cli`（38 条）：
  - `out_path_is_stripped_from_the_wire_command` / `crop_options_are_stripped_and_the_rest_is_preserved`
    —— 客户端选项被摘掉，**其余逐字节保留**（含引号里的连续空格）。
  - `bridge_log_prints_the_tail_without_needing_an_app` ——
    `bridge log` 不需要连上应用。
- `zephyra-winit-input`（40 条）：`injected_text_events_are_pure_characters`
  —— `type_text` **不产生键边沿**（多发边沿会让某些控件把每次输入当成一次提交）。
- `ZephyraCLI/src/shot.rs`（4 条）：`bmp_to_png_preserves_pixels_top_down`
  —— 逐像素还原（上下颠倒、通道互换都会露馅）；
  `crop_and_scale_preserve_the_pixels` —— 裁剪 + 整数放大不改像素值；
  `crop_rejects_a_rect_outside_the_image` —— 越界**报错**，不静默裁。

## 真机复核（gallery，D3D12，1400×800，`ZEPHYRA_DEBUG_PORT=7878`）

```text
find 纯色        → 0.1.1.1.0.1.1.3.1.0.1  "纯色"  at 510,761 30x18  owner=0.1.1.1.0.1.1.3.1.0
click_path …     → clicked path 0.1.1.1.0.1.1.3.1.0 at 488,770
hit 86 122       → 0.1.0.1.2 Text key=label text="状态"  clickable=0.1.0.1 (click_path 0.1.0.1)
拖动（合成）      → 滑块值 50 → 76（hover → pointer_down → hover → pointer_up，中间 sleep）
wait_frames 5    → the app settled (rendered 0 of the requested 5 frames, then went idle)（320 ms）
shot --rect … --scale 3 → 裁剪 + 3 倍放大，人眼确认开关状态与残影
resize 800 600   → 视口 1400×800 → 800×600；导航随之收成 compact（56 px 宽）
```
