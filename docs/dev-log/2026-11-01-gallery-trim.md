# 2026-11-01 gallery 裁剪：只演示 Button 与 CheckBox

> 现象（需求）：gallery 里演示了一大批「只有外观、名字却暗示了更多能力」的控件
> （ComboBox / Menu / Dialog / Table / Slider / TextField / SwipeView …），
> 画面上看着像能用，点下去才知道是空壳。需求：**把不成熟组件踢掉，只保留
> Button 和复选框**。

## 口径（先定，再动刀）

| 问题 | 结论 |
|---|---|
| 删的是控件还是演示 | **只删 gallery 演示**。控件库 `ZephyraUI` 的 54 个组件**全部保留** —— 能力缺口是另一件事，记在 [`docs/todo/control-capability-audit.md`](../../todo/control-capability-audit.md)，不在这里动手 |
| 「没有内容的」参考页 | 删。参考路由从 9 个缩成 2 个 |
| 「不成熟」的判据 | **字面：只留 Button 与 CheckBox**。被演示的控件只留 Button 族（Button / PillButton / RoundButton / ToolButton / Hyperlink）+ CheckBox 族（CheckBox / CheckBoxGroup） |
| 外壳与导航怎么办 | **保留**。导航栏 NavItem、页头 InfoBadge / ToolButton、参考页的 Segmented 族切换器都是**结构**，不是被演示的控件 —— 砍掉它们 gallery 就切不了页、也切不了族 |

> ⚠ 口径里最容易搞混的一条：**「只留 Button 与 CheckBox」说的是「被演示的控件」，
> 不是「gallery 里只准出现这两种控件」**。外壳照样有按钮、徽标、族切换器 ——
> 它们是导航，不是演示。

## 落地

| 层 | 改动 |
|---|---|
| `gallery/frontend/shell.zypui` | 删 8 个组件（`PageLayoutFamily` / `PageCollections` / `PageMenus` / `PageDialogs` / `PagePaging` / `PageNavigation` / `PageStatus` / `PageSettings`）；`PageBasics` → `PageButtons`（key `buttons` / cls `page-buttons`）、`PageToggles` → `PageCheckbox`（key `checkbox` / cls `page-checkbox`）；`PageReference` 的 9 个 `<if>` → 2 个；导航栏删 `settings` NavItem 与 `.nav-spacer` 容器 |
| `gallery/frontend/app/mod.rs` | `REF_FAMILIES` 九族 → 两族；删 `SETTINGS_ROUTE`（连同三处引用）；删 24 个状态字段与对应 click 分派；删约 50 条 `view()` 绑定；删死常量 `DEMO_SEGS` / `SUBDEMO_IDS` / `SELDEMO_IDS` |
| `gallery/frontend/app/data.rs` | 删 25 张不再使用的表 / 函数（`COMBO_OPTIONS`、`SEGS`、`RADIO_ITEMS`、`TABLE_ROWS`、`CALENDAR`、`SWIPE`、`TOASTS` …） |
| `gallery/frontend/app/pages.rs` / `shell.rs` | `PAGES` 删 `settings`；`NAV_ITEMS` 与 `ROUTES` 重新变成同一份清单 |
| `gallery/frontend/*.zstyle` | 删死规则（`Container.nav-spacer`、`Container.stack`、`page-collections` / `page-menus` / `page-navigation` / `page-status` / `page-dialogs` / `page-paging` / `page-settings`、`card-grid`、`nav-list`）；`.page-basics` → `.page-buttons`、`.page-toggles` → `.page-checkbox` |
| `gallery/frontend/app/mod.rs`（i18n） | `CATALOG` 136 键 → **59 键**（231 条 → 177 条 × 3 语言） |
| `gallery/tests/gates.rs` | 见下「门禁的四处适配」 |

裁剪后的参考页只剩两族：

```
reference ─ segmented ─┬─ PageButtons   （Button / PillButton / RoundButton / ToolButton / Hyperlink）
                       └─ PageCheckbox  （CheckBox / CheckBoxGroup）
```

## 门禁的四处适配（每处都有原因）

| 门禁 | 原来 | 现在 | 为什么必须改 |
|---|---|---|---|
| `reference_page_has_all_families` | 断言 **9** 个族 | 断言 **2** 个族 | 族数是**写死**的 —— 这是刻意的：加族忘了改这里会红一次，逼你承认「参考页多了一个族」。不改成「≥ 1」之类，否则门禁就恒真了 |
| `only_fill_nodes_carry_local_width` | 断言 `fill:*` **≥ 4** | 断言 `fill:*` **== 0** | 原判据靠 Slider / ProgressBar 提供的 `fill:` 类撑住 ≥4。控件退场后它变成**恒真**（0 个也满足）⇒ 门禁悄悄失去意义。改成反向的更强判据：源码里**一个局部 WIDTH 都不该有** |
| `route_table_and_page_table_agree` | 补上 `SETTINGS_ROUTE` 再比对 | 直接比 `ROUTES + SETTINGS_ROUTE` 与 `PAGES` | 设置页整页撤下，两边都少一项 ⇒ 补丁本身成了「永远补一项错的东西」 |
| `shell_layout_geometry` | 断言「Settings 落在 794」 | 断言「导航栏最后一项**不得溢出**导航容器」 | 原来那一条是**按名字**钉的（"Settings" 在 x=794），删掉设置页就没法钉了。换成**按几何**钉（最后一项的右边界 ≤ 容器右边界）—— 换掉的是「名字」这个脆弱前提，不是断言强度 |

## 踩到的坑

### 1. `every_stylesheet_rule_matches_something` **只查应用层**

它比的是 `shell.zstyle` + `pages.zstyle` 两条**应用层**样式表，**不看控件层**。

⇒ 好消息：控件库 54 个组件的样式表不受本次裁剪影响（`controls-*.zstyle` 一条没动）。
⇒ 坏消息：被删页面的**类名**必须同步从应用层样式表里删干净，否则红。

### 2. 同一条门禁**依赖「每族一个脚本」**

`interaction_scripts()` 给每一族造一份交互脚本，门禁靠它反推「哪些类名真的被发出来」。
少一族 ⇒ 那族的类名发不出来 ⇒ 被误判成**死规则** ⇒ 红的不是「真死规则」而是「少写的族」。

⇒ 裁剪时族数与脚本数必须**同时**改，否则会看到一堆莫名其妙的死规则。

### 3. i18n 表是**单向**门禁，多余条目不会红

`every_tr_key_used_in_source_exists_in_every_language` 查的是「源码用到的键在表里有没有」，
**反过来不查** ⇒ 删掉演示控件后，那些键（`ui.combobox` / `ui.slider` / `ui.dialog` /
`set.theme.*` / `route.settings` …）留在表里**一声不响**。

本次手工对了一遍：把 HEAD 版导出来做对照，区分出
**本次裁剪新产生的 72 个孤儿**与**裁剪前就有的 5 个**（`app.title` / `action.language` /
`route.settings` / `set.volume.*` —— 设置页在更早一轮就撤了，键一直没人清）。
全部清掉后 `CATALOG` 与源码**完全对齐**：59 键 × 3 语言 = 177 条，零孤儿零缺失。

> 判据：`tr()` 只接受 `&'static str` 字面量，**没有动态键** ⇒ 「表里的键 = 源码里用到的键」
> 是可以成立的等式，值得当成契约守住。

### 4. ⚠ 脚本改文件时，行尾被写成了 CRLF

用 Python 读写 `.zstyle` / `.zypui` / `.rs` 时，`open(p,'w')` 在 Windows 上会把
每个 `\n` 翻译成 `\r\n`。仓库规范是 **LF**（blob 与工作区都是 LF，`core.autocrlf=false`）
⇒ `git diff` 把**整个文件**判成重写（`pages.zstyle` 变成 437 增 / 491 删），
真正的改动被淹没。

排查手法（别急着怀疑内容被改坏）：

```bash
python -c "import subprocess;print(subprocess.run(['git','cat-file','blob','HEAD:FILE'],capture_output=True).stdout.count(b'\r'))"
```

blob 的 `\r` 数是 0 而工作区不是 ⇒ 行尾问题，不是内容问题。修法：把改动文件
统一换回 LF，再看 `git diff --numstat` 是否回到「真实改动」的量级。

> ⚠ 别用 `git show HEAD:FILE | grep -c $'\r'` 判断 —— MSYS 的管道层会替 git
> 把 CRLF 翻译成 LF，这个数**永远是假的**。用 `git cat-file blob` 走二进制。

### 5. 脚本删绑定 / 删表项，两个工具各坏一次

- `drop_binding` 遇到 `=> { ... }`（末尾无逗号）时扫描越过块尾，**吞掉了下一条绑定**
  （`"cnt_text"` 被误删）。改成按**行号**定位。
- `drop_item` 用字符扫描找 `;`，切错了多行 `const` 的位置，文件被弄坏
  （`prefix 'card' is unknown`）。`git checkout` 恢复后改用**行号 + 括号配平**重做。

⇒ 这类编辑**一律先在副本上跑**，再把 diff 拿眼睛过一遍。

### 6. `cargo fmt` 会把删空的 match 分支压成一行

删绑定后留下一堆空行与陈旧小节注释（`// ---- collections ----` 之类）。
`cargo fmt` 的 `blank_lines_upper_bound=1` 会折叠连续空行，但**注释不会自动消失** ——
得自己按行清。漏一条就留下「注释说的族已经不存在了」这种自相矛盾的话。

## 验收

- **门禁**：`cargo test -p zephyra-gallery` 全绿（44 + 3 + 10 + 7）；
  `cargo test --workspace` 全绿；`cargo fmt --all -- --check` 干净；
  `cargo clippy --workspace --all-targets -- -D warnings` 干净。
- **真机（UIA 跨进程）**：`gallery.exe --route reference --family 1` ⇒ 组头
  `group:checks` 的 Toggle = `Indeterminate`、`check:alpha` = `On`、
  `check:beta` / `check:gamma` = `Off`、`check:locked` = `Off` 且 `Enabled=false`、
  `act:feedback` 有 Invoke 图案。`--family 0` ⇒ 8 个 Button 全部带 Invoke。
  （裁剪前后的 UIA 树形状一致 ⇒ 裁剪没有伤到无障碍链路。）

## 门禁

- `every_stylesheet_rule_matches_something` —— 应用层样式表不许有死规则
- `every_class_name_the_source_emits_exists_in_the_stylesheet` /
  `every_element_is_reachable_by_the_stylesheet` —— 正反两向
- `route_table_and_page_table_agree` —— 路由表与页面表必须一致
- `reference_page_has_all_families` —— 参考页族数（写死，改族数必红一次）
- `every_interactive_key_is_wired_or_whitelisted` —— 树上每个交互 key 都要有分派
- `only_fill_nodes_carry_local_width` —— 源码里一个局部 WIDTH 都不该有
