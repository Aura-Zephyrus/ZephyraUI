# 删掉 `examples/`：demo 已被 gallery 取代，两处能力都重复

> 现象：`examples/` 里那个 681 行的 `demo.rs` 还在 workspace 里编着，而它
> 提供的每一样东西 gallery 都有一份更好的。

这一轮把**整个 `examples/` crate** 删掉了（15 个文件），并修掉所有指向它的
文档与命令。

---

## 一、为什么它「没用」——逐项对照

`examples/` 是 workspace 成员 `zephyra-foundation`，含两个 bin 与三个测试目标。
逐项看它提供的东西在别处有没有：

| `examples/` 提供的 | 等价物 | 状态 |
|---|---|---|
| `--dump-cpu`（CPU 参考光栅器出图） | `gallery --dump-cpu --route <id>` | ✅ gallery **更强**（9 条路由 vs demo 的 1 屏） |
| `--bench`（分段计时） | `gallery --bench` | ✅ gallery 还多 `--bench-resize` |
| `--alloc-probe`（每帧分配数/字节） | `gallery --bench --alloc-probe` | ✅ 同款 |
| Linux 后端冒烟（`--features linux-vk`） | `gallery --features linux-vk` | ✅ gallery 有同一个 feature |
| 交互回归（10 条） | `gallery/tests/interaction_tests.rs` | ✅ **同名同义，一条不多一条不少** |
| 「源码里不许有样式」 | `gallery/tests/gates.rs::the_whole_tree_writes_no_appearance_in_source` | ✅ |
| 样式表「每条规则钉一个属性」 | 门禁**只查应用层**（控件层是给宿主用的样式库，本 gallery 没有那些状态） | 见下 |

**没有任何东西依赖它**：`cargo tree` 全仓无一处引用 `zephyra-foundation`，
Rust 代码里也没有 `zephyra_foundation::` 的使用点。

### 最硬的一条证据：gallery 的测试**自己写着**它取代了 demo

`gallery/tests/interaction_tests.rs` 的文件头：

> 交互回归 —— **保留** `examples/demo_tests.rs` 那 10 条测试。
> # 为什么复制而不是直接复用 `examples` 那份
> `demo_tests.rs` 驱动的是 `demo::view`（demo 自己的按钮行）。本文件驱动的是
> **gallery 的真实控件**（按钮、复选框…）。两者测的是**同一批框架语义** […]
> 但**跑在**迁移后的代码**上，这才是「保留测试功能」的意义：测试跟着被测对象走，
> 而不是守着一个已被替换的 demo。

⇒ 那 10 条**逐条同名**（`no_input_means_diff_is_a_noop` /
`hovering_a_button_changes_the_display_list` / … / `cropping_an_image_changes_only_the_command_not_the_geometry`）。
删掉 demo 不丢任何框架语义的覆盖。

### `foundation.rs` 也不再有独立价值

它是骨架阶段的「验收标准」—— 一份**可执行的契约陈述**（6 条断言）。现在
core 的单元测试逐条覆盖了它们：

| `foundation.rs` 的断言 | core 里的等价物 |
|---|---|
| DisplayList 按 `(layer, z, order)` **稳定**排序 | `tests.rs::display_list_sorts_by_layer_then_z_then_order` / `display_list_sort_is_stable_for_equal_keys` |
| 稳态 diff 是 no-op | `tests.rs::steady_state_diff_is_a_noop` |
| 属性相等可判定 | `property_store_tests.rs` |
| 深浅主题都有语义角色、surface 不同 | `tests.rs::theme_toggle_round_trips` + surface 断言 |

⇒ 它是一份**重复的**契约陈述，而且**不在门禁里**（`AGENTS.md` 的门禁表从来没列
`cargo run -p zephyra-foundation`）。留着它只是让「契约在哪里」有两个答案。

---

## 二、顺带修掉的：一条**本来就坏**的门禁

`AGENTS.md` 的 Linux 验收写的是：

```bash
cargo clippy -p zephyra-foundation -p zephyra-ui --all-targets \
    --no-default-features --features zephyra-foundation/linux-vk,zephyra-ui/linux-vk
```

实测：`zephyra-ui` **没有** `linux-vk` feature（控件层零后端依赖）⇒

```text
error: the package 'zephyra-ui' does not contain this feature: zephyra-ui/linux-vk
```

⇒ **这条门禁从来没有跑通过**。它一直是一行看起来很严谨、实际跑不起来的命令。

删掉 demo 正好把它改成真的：

```bash
cargo clippy -p zephyra-gallery -p zephyra-ui --all-targets \
    --no-default-features --features zephyra-gallery/linux-vk -- -D warnings
cargo run -p zephyra-gallery --bin gallery --no-default-features --features linux-vk
```

（`zephyra-ui` 那一项留着是对的 —— 它确实要一起编；错的只是那个不存在的
feature 名。）

---

## 三、连带改动

| 位置 | 改动 |
|---|---|
| `Cargo.toml` | `members` 与 `default-members` 去掉 `"examples"` |
| `gallery/src/bin/main.rs` | 字体的 `include_bytes!` 路径改到 `gallery/fonts/` |
| `gallery/fonts/MiSans-Regular.ttf` | **从 `examples/fonts/` 移过来**（gallery 是它唯一的消费者 —— 之前是 demo 借给 gallery 用的） |
| `zephyra-fx/src/text.rs` | 文档里提到字体路径的那句跟着改 |
| `AGENTS.md` | 门禁表的 demo 命令换成 gallery；`default-members` 那句去掉 examples；Linux 验收改成能跑的那条；`examples/` 的特例说明改成 `gallery/` |
| `docs/developer/01-overview.md` | crate 树里去掉 `examples/` |
| `docs/developer/architecture.md` | 「当前状态」那节改写：不再说「控件库与模板系统**未实现**」（两者都已实现），也不再说 `examples/foundation` 是验收标准 —— 改成指向 core 的单元测试 |
| `docs/developer/portability.md` | Linux 验收命令换 gallery；删掉 demo 那条已经跑不起来的 `--features zephyra-ui/linux-vk` |
| `docs/developer/05-frame-pipeline.md` | 「实测（demo，45 节点）」→「实测（gallery，45 节点）」；`demo --bench` → `gallery --bench` |
| `docs/developer/07-declarative-ui.md` | 「仓里有两个完整的真实用户」改成「一个」（第二个就是 demo）；`build.rs` 那段示例路径换成真实的两个文件 |
| `TODO.md` | Linux 验收命令换 gallery |
| `.gitignore` | `demo --dump-cpu` 那句注释改 gallery；顺带删掉「examples/ 那份是早先的残留」（那份已经没了） |
| `gallery/src/bin/main.rs` | 三处注释里的 `examples/demo.rs` 改成实际在说什么（「照 demo 的成熟形态」→「三条路」／bench 的注释不再提「两处同步」／首帧补画那条去掉「同 demo 的注释」） |
| `gallery/tests/interaction_tests.rs` | 文件头改写：说明这 10 条的**来历**（原是 `examples/demo_tests.rs`，那个 crate 已删）而不是「复制而不是复用」——原措辞在源文件被删后成了悬空的对照 |
| `gallery/tests/gates.rs` | 文件头那句「与 `examples/demo.rs` 的 6 条同一理由」改成不指名的同义说法 |
| `zephyra-fx/src/quad.rs` | `vertex_bytes` 那段注释里的 `examples/demo.rs --dump-cpu` → `gallery --dump-cpu` |
| `zephyra-winit-d3d12/src/renderer.rs` | 「demo 实测过这个场景：demo.zstyle 全量迁移后…」→「实测过这个场景：样式表全量迁移后…」—— 路径没了，结论留下 |

⚠ **dev-log 里的旧记录不改** —— 那些是**有日期的历史**（当时 demo 确实存在）。
只有本轮自己写的那份（`2026-11-perf-monitor-and-debugger.md`）里提到
`examples/demo.rs` 的地方改了。

⚠ **代码注释里指路被删的文件，要连结论一起处理** —— 「定位信息必须与结论成对出现」
这条规矩在**删除**的场景下有另一半：路径没了，结论还在，于是要么把路径换成
「已删除」+ dev-log 指向（`architecture.md` / `interaction_tests.rs` 那样），
要么把路径整段去掉、只留结论（`renderer.rs` 那样）。**留着一个指向不存在文件的
路径 = 下一个来的人按图索骥扑空**，比一开始就没写更糟。

---

## 四、排除的假设

- ❌「demo 还在当 Linux 后端的冒烟入口。」—— gallery 有同一个
  `linux-vk` feature，而且 demo 的冒烟**从来跑不通**（Linux 上 d3d12 是空
  crate，而 `required-features` 表达不了「d3d12 或 linux-vk」⇒ 那个 bin 在
  Linux 上**永远存不住**）。
- ❌「删掉 demo 会丢掉交互回归的覆盖。」——不会：gallery 那 10 条与 demo 那 10 条
  **同名**，而 gallery 的文件头写着它就是为了取代 demo。
- ❌「`foundation.rs` 是唯一的契约陈述，删了就没地方说『core 满足哪些条件』了。」
  ——它**不在门禁里**，而 core 的单元测试逐条覆盖了它的 6 条断言。留着它会让
  「契约在哪里」有两个答案，且其中一个（那份可执行陈述）**跑得比单元测试慢**
  （它是独立 bin，要编整个应用）。
- ❌「字体得留在 `examples/fonts/`，因为 gallery 借它。」——反了：字体是
  **gallery** 的资产（它要含 CJK 才能不出豆腐块），demo 只是当时先有它。
  放 `gallery/fonts/` 才是它该在的地方。

---

## 五、结论

- **`examples/` 整体是重复的**：它提供的每一样能力 gallery 都有更好的版本，
  而 gallery 的交互测试**逐条同名**地覆盖了它。
- **删掉它还修掉一条本来就坏的门禁**：`zephyra-ui/linux-vk` 这个 feature
  不存在 ⇒ AGENTS.md 的 Linux 验收命令从来跑不起来。
- **删掉的 25 条测试全部有归属**：10 条交互回归 → gallery 同名同义；
  6 条「源码里不许有样式」→ gallery 的
  `the_whole_tree_writes_no_appearance_in_source`；9 条是 demo **自己样式表**的
  门禁（那份表随 demo 一起没了）。
- **字体归位**：`examples/fonts/` → `gallery/fonts/`（唯一的消费者是 gallery）。

---

## 六、门禁

- `cargo fmt --all -- --check` 干净
- `cargo clippy --workspace --all-targets -- -D warnings` **0 警告**
- `cargo test --workspace`：**1460 passed / 0 failed / 72 ignored**
  （删 demo 前 1485 ⇒ 少的 25 条就是 demo 那三条测试文件，见 §五）
- `cargo doc --no-deps --workspace` 退出 0，**39 条既有链接警告**（`zephyra-core` 22 /
  `zephyra-winit-d3d12` 5 / `zephyra-fx` 3 / `zephyra-ext-wgpu` 2 / `zephyra-uia` 1 +
  2 条 `zephyra_render_ext` 未解析链接）—— 本轮**没有新增**（判据是「有没有新增」，
  不是总数；删掉一个 crate 只会让总数变少）
- 换行符逐文件比对（`git cat-file blob` 取字节，见
  `2026-11-bridge-ai-ergonomics.md` ⑤）：**已跟踪文件全部与 HEAD 的风格一致**；
  新增的 72 个文件一律 LF，而 HEAD 里 `.rs` 是 LF 263 / CRLF 39 / MIXED 4
  ⇒ LF 是本仓的主流风格
  - ⚠ 期间发现 `ZephyraUI/src/tokens.rs` 的**内容只改了 1 行**（测试文件搬家），
    但 diff 是 **101/101** —— 它的换行符在 HEAD 里本来就是 MIXED（1 CRLF +
    101 LF），一次整文件重写把它归一成了 CRLF。修法：**从 HEAD 的字节重建**，
    只替换那一行的字节（不碰行尾）⇒ diff 回到 1/1。
- 真机：`gallery --dump-cpu --route slider` 出图正常（199 四边形 / 181 字形 /
  64 字形缓存 ⇒ 字体路径改对了，CJK 没变豆腐块）
