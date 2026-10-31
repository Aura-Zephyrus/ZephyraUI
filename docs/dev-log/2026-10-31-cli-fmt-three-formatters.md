# 2026-10-31 `zephyra fmt`：三个排版器聚合（Rust / `.zstyle` / `.zypui`）

> 现象：写法统一全靠人手 + 评审。上一轮把 `.zypui` 归一化了，用的是一次性 PowerShell
> 脚本（不在库里、没进版本树的正式路径、别人跑不到）；`.zstyle` 从来没排过版 ——
> 实测 19 份文件里 2 份是 CRLF、44 处单行规则、选择器与 `{` 之间有对齐空格。
> 「统一写法」没有一个命令能一键做完，也没有判据能长期钉住。

## 排查

用户拍板的分工（原话）：**「RS 代码交给 cargo fmt，.zstyle 走 zstylec fmt、
.zypui 走 zypuic fmt；cli 做聚合，允许走 CLI FMT 同时调用三个」**。

于是三处各加一个排版器，CLI 只做聚合。关键的设计决定是**排版判据住哪儿**：

- `zstylec::fmt` —— `.zstyle` 的排版器 + 五条规则的判据（`violations`）
- `zypuic::fmt` —— `.zypui` 的排版器 + 五条规则的判据（同一个 `violations`）
- `ZephyraCLI/src/commands/fmt.rs` —— 找文件、调编译器、写不写回、报什么，**不判排版**

CLI 自己再判一遍排版，就有了「CLI 说排好了、编译器门禁还红」的立足点 —— 那种
分裂极难归因（用户会怀疑其中一边装错了版本）。所以 CLI 只调库。

`.zstyle` 排版器最关键的取舍：**「只动空白」而不是从 IR 回写**。
`zstylec::parse` 把值归一化了（`10px` → `Float(10.0)`、`#rrggbb` → `Color`、
`@令牌` → `Resource`、`26 18 0 1.5` 的多空格形状全丢），从 IR 反写 CSS 不可能还原
源码拼写。所以排版器直接吃原文：选择器 / 属性名 / 值 / 注释**逐字照搬**，只重排
它们之间的缩进、换行与空行。值里一个字节都不碰 ——「不改语义」是构造上成立的。

## 排除的假设

1. **「`.zstyle` 可以从 IR 回写」** —— 试过就会发现值拼写丢了：IR 把 `10px` 存成
   `Float(10.0)`、`#rrggbb` 存成 `Color`，`@control_fill` 存成 `Resource("control_fill")`。
   回写出来的 `.zstyle` 排版好看但**值变了**（`0.00` 与 `0` 无从区分、数值列表的
   多空格形状没了）。所以判据改成「排版前后 `parse` 的 IR 一致」，那是「只动了
   空白」的硬证明，逐份存量文件钉住。

2. **「词法扫一下 `{ } ; :` 就够了」** —— `.zypui` 的值里允许裸 `>`
   （`cond="a > b"` 不用写 `&gt;`，见 `xml.rs` 的「对 XML 的刻意偏离」），拿
   `find('>')` 定位标签尾会把值里的 `>` 当成标签结尾。必须**跟引号状态**扫
   （`scan_tag`），并且这条写成了单元测试（`a_gt_inside_a_value_is_not_the_tag_end`）。

3. **「解析属性的循环，推入之后游标自己会前进」** —— 第一版 `split_attrs` 解析完
   一个属性推进 `v` 之后**忘了把 `i` 推到属性末尾**，`while i < b.len()` 原地重启，
   同一个属性被推一次一次又一次，直到申请 1.6 GB 失败。三条仓内 `.zypui` 全跑
   不过（`fmtdbg` 逐个试，三份都 OOM）。**「循环变量必须前进」在这里不是风格问题**，
   修法是把「每次 push 之前显式推进游标」写进注释，让下一个人不敢再删。

4. **「多重集比对够强」** —— 上一轮用「属性多重集 / 标签多重集 / 注释条数」证明
   归一化没改语义，这一轮做成了常驻判据并且升级到**编译产物**：
   `repo_files_survive_a_mangle_and_reformat_roundtrip` 把每份存量文件只动位置地
   弄乱一遍（去缩进 + `class` 挪最前），再交给 `fmt`，要求整理后与原件逐字节相同、
   连同其余 `.zypui` 编译出的产物逐字节相同。产物里没有位置与空白的痕迹，所以这是
   比多重集更强的判据。写这个用例时踩到三件事：
   - **组件表是跨文件的** —— `shell.zypui` 用 `TextLabel`，定义在 `controls.zypui`，
     单文件编不过（实测诊断「组件 `TextLabel` 不存在」）。所以判据要连同其余文件
     一起编，与 `gallery/build.rs` 的做法一致。
   - **顶层只认 `<component>` / `<page>`** —— 裸 `<container>` 编不过，所以单元测试
     里的片段多重集判据是 `same_tree`，编译产物判据只对真文件做。
   - **弄乱脚本自己也坏了两次** —— `class` 是唯一属性时把插入点算到 `>` 之后，
     标签被撕成 `<container>class="…"`；注释续行的缩进被 `trim()` 吃掉（而排版器
     正是逐字照抄注释的，于是判据误判）。两次都是**判据本身**写坏，不是排版器错。

5. **「`fmt --check` 可以把 `Err(CliError)` expect 掉」** —— CLI 测试第一版这么写，
   「未知扩展名」那条当场 panic（那正是 `fmt` **应该**返回 usage error 的场景）。
   改成 `Err` 也变成退出码，与 `entry::dispatch` 的形状一致。

6. **「测试里可以 `set_current_dir` 到临时目录」** —— CLI 测试第二版这么写，
   红了：`set_current_dir` 是**进程级**的，而 cargo 测试并行。两个用例同时切 cwd
   互相踩，而「跑完切回去」在别的用例已经 `remove_dir_all` 掉那个目录时直接失败
   （实测 `fmt_formats_a_zypui_file_in_place` 就这么死的）。改成**全传绝对路径**，
   一个字节的 cwd 都不动。

7. **「`cargo fmt` 在没有 manifest 的目录也能跑」** —— 不能，它会报
   `could not find Cargo.toml`。所以 Rust 那一档只在工程根有 `Cargo.toml` 时才跑，
   否则「这里只有一堆 `.zstyle`」的目录不该被一条 cargo 报错打红。

8. **「`violations` 直接从排版门禁搬过来就等价」** —— 搬漏了一条：缩进是不是 2 的
   倍数（规则1）。表现是自检用例里 `   <container key="a"/>`（3 空格）**一条违规都
   不报**。门禁绿着但判据缺一条 —— 与上一轮「少报一半 class 违规」同源的失败模式：
   判据与实现是两份，就得逐条对照。这次判据只有一份（住在 `fmt`），所以「漏搬」
   只剩一次，且被自检用例当场抓到。

9. **「库入口导出 `format` 没问题」** —— `cargo doc` 直接报
   `error: 'format' is both a function and a macro`，**整个 crate** 的文档都出不来。
   根作用域上的 `format` 与 std 的 `format!` 宏撞名。两个编译器各给一个显式别名：
   `zstylec::format_style` / `zypuic::format_ui`（模块内部仍叫 `format`，模块
   作用域不撞）。这条没有任何测试能提前发现 —— 只有 `cargo doc` 会红，而它是
   四条门禁里最容易被跳过的一条。

## 结论

- 三个排版器：

  | 命令 | 干什么 | 判据 |
  |---|---|---|
  | `zephyra fmt` | 聚合：`.zstyle` + `.zypui` + `cargo fmt --all`（工程根有 manifest 时） | 全部来自编译器 |
  | `zstylec fmt` | 只动空白重排 `.zstyle`（值逐字照搬） | `zstylec::fmt::violations` |
  | `zypuic fmt` | 只动位置重排 `.zypui`（引号内逐字节不变） | `zypuic::fmt::violations` |
  | `--check` | 只报不写，有要排版的退 1（与 `cargo fmt --check` 同形状） | |

- 「不改语义」的判据：
  - `.zstyle`：排版前后 `zstylec::parse` 的 IR 逐字段一致 + 幂等（逐份存量文件）；
  - `.zypui`：属性多重集 / 标签多重集 / 注释条数（任意片段）+ **真文件**上的
    编译产物逐字节相同（`repo_files_survive_a_mangle_and_reformat_roundtrip`）。
- 不认识的源码**原样返回**（`None`），绝不猜一份排版覆盖上去；`--check` 报脏。
- 门禁与排版器共用一份判据（`violations`），并有一条用例专门盯「fmt 修出来的
  形态必须过门禁」—— 防「跑完 fmt 门禁还红」。
- `.zstyle` 存量 19 份文件未归一化（本轮只加了判据），首跑门禁即报漂移清单。

## 门禁

- `cargo test -p zstylec` —— 排版门禁扫工作区全部 `.zstyle`：
  `style_gate_tests::repo_zstyle_files_are_formatted`（漂移清单）、
  `formatting_is_idempotent_on_every_repo_file`、
  `formatting_does_not_change_the_ir_on_any_repo_file`、
  `the_gate_actually_catches_drift`（判据自检，含单行规则）。
  另有 `fmt_tests::*`（10 条）钉「已排好不变 / 值逐字 / 注释照抄 / 空行语义 /
  CRLF / 不认识的输入返回 `None` / `//` 不是注释」。
- `cargo test -p zypuic` —— 41 条：门禁 4 条（扫全仓 + 判据自检 + **fmt 与门禁
  对齐** + 存量全被 fmt 认得）+ `fmt_tests` 9 条（含编译产物金标准）。
- `cargo test -p zephyra-cli` —— 28 条集成用例（不起子进程），新增 6 条
  `fmt` 的：就地排版 `.zstyle` / `--check` 不写盘且退 1 / 就地排版 `.zypui` /
  未知扩展名报用法错 / 传目录递归扫且跳过 `target` / 命令表登记 `--check`。
