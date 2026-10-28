# 2026-10-28-zephyra-ui-single-source

> 现象：`zephyra-ui` 每个控件要维护**两份结构**（`.zypui` 模板 + Rust 构造器），
> 改一处必须同改另一处；crate 里还有两个零消费者的依赖和一条静默逃过门禁的令牌。

## 排查

**谁在消费什么**，全仓 grep 的结果：

| 产物 | 消费者 |
|---|---|
| `ZephyraUI/src/controls/*.rs`（59 个 `pub fn`，1900 行） | **只有** `tests/zypui_parity.rs:25` |
| `ZephyraUI/ui/controls.zypui`（53 个组件） | `gallery/build.rs:15` 直接读文件编译 |
| `ZephyraUI/controls-*.zstyle`（15 份） | `gallery/frontend/styles.rs:52` 的 `STYLESHEETS` |
| `ZephyraUI/src/tokens.rs` 的 `ALL` | `gallery/tests/gates.rs:19` |
| `ZephyraUI/src/ui.rs` 的 `controls()` | **零消费者** |
| 依赖 `zypui` / `zephyra-style` | `.rs` 里一次都没出现 |

gallery 的 `shell.zypui` 里 `<Button>` / `<CheckBox>` / `<Slider>` / `<Segmented>` 等
全部从 `.zypui` 展开（`gallery/build.rs` 把控件库与页面模板编进**同一个**组件表），
不经由 crate 的 Rust API。上一轮迁移 `8c47183` 写的是「树全等 parity 验收后删旧码」
—— 旧码没删。

**Rust 那一层的写法本身就是重写障碍**：`spin_box` 8 个位置布尔参数
（`hovered`/`pressed`/`up_hovered`/`up_pressed`/`down_hovered`/`down_pressed`）配
`#[allow(clippy::too_many_arguments)]`；`gen_swatch_key` 16 项、`cell_key` 16 项、
`table_row_key`/`head_key`/`swipe_pip_key` 各 4 项的手写 key 常量表（`.zypui` 用
`for` + interner 一行解决）；`state()` 每次调用分配一个 `Vec`。

**令牌漏网**：`SURFACE_ALT_TRANSLUCENT` 在 `tokens.rs` 里定义了，却不在 `ALL` 里。
而门禁遍历的是 `ALL` ⇒ 这个令牌永远不会被检查。失败模式正是本文件开头说的那类：
样式表里写 `@surface_alt_translucent` 时颜色静默保持原值。

**Cargo.toml 里的谎话**：`zephyra-style` 的注释声称「控件的排布要断点与触控目标、
外观要缓动档位，两者都在框架层」—— 实测本 crate 的 `.rs` 里 `zephyra_style`
一次都没出现。那是脚手架期的计划，写进了依赖表却从未落地。

## 排除的假设

- **「Rust 层是给外部使用者的 API，删了等于破坏兼容」**——全仓 `zephyra_ui::`
  的外部引用只有 3 处（`STYLESHEETS` ×2、`tokens::ALL`、以及 parity 测试自己）。
  `publish = false`，没有外部使用者。
- **「`.zypui` 里的 `url=\"{url}\"` 依赖 Rust 的 `HYPERLINK_URL` 属性键」**——
  `zypuic/build.rs:10` 明写「未知属性当成默认属性」，属性键按名字自动产生，
  不查 `props_map`。删 Rust 层不碰 Hyperlink。
- **「`src/ui.rs` 的运行时 `Ui` 入口要留着，将来好让 crate 自己暴露组件库」**——
  gallery 是在**自己的** build.rs 里读文件编译的，不经过这个入口。留着等于为一个
  从未存在的使用者维护一条 `include_bytes!` 链路（build.rs → OUT_DIR → decode）。
- **「删掉 parity 测试后逐控件结构就没人管了」**——判据换成 gallery 的**整树**门禁
  （`every_element_is_reachable_by_the_stylesheet` /
  `hit_test_agrees_with_the_painted_rects` /
  `every_interactive_key_is_wired_or_whitelisted` /
  `the_whole_tree_writes_no_appearance_in_source`，遍历真实应用 × 全部页面 × 全部
  交互态）+ zypuic 编译期报错（未知元素/属性/标识符）。判据从「与脚手架全等」
  变成「gallery 真用的那棵树是对的」。
- **「令牌名单漏项补一行就行」**——补行修的是这一处，修不了这一类：门禁遍历
  `ALL`，于是「定义了但忘了列」永远静默。真门禁必须判**反方向**（core 有 →
  名单必须有），而 Rust 没有常量反射，只能手列 core 的角色常量在测试里。

## 结论

`zephyra-ui` 从 3301 行降到 300 行左右，内容变成它本来就该是的东西：**控件样式表 +
令牌名单 + 结构声明文件**。结构真相全仓一份（`ui/controls.zypui`，由应用的 build.rs
编译）。两处静默失败模式被钉住：令牌名单漏项（新测试 `core_的每个角色都出现在名单里`）
与 `#[allow(dead_code)]` 掩盖的未用常量（allow 已删）。

**下一步的地基账**（本轮不动，只记录）：键盘/IME 全空（`KeyInput` 只有键名+边沿，
宿主无 `WEvent::Ime` 分支，且全仓零消费者）；焦点系统没有（`props.rs` 41 个属性键里
无 `Focused`/`TabIndex`）；`Layer::Popup` 零生产者、`Z_INDEX` 键没人读（`paint.rs`
里 `z` 恒 0）；`PushClip/PopClip` 零生产者；Vulkan 后端 `scroll`/`key_events` 硬编码
空值。这些是「用 Rust 写原生控件」的地基，见 `docs/todo/control-capability-audit.md`。

## 门禁

- `cargo test -p zephyra-ui`：4 条（令牌：两套主题可解析 / 无重复 / core 角色全在
  名单 / 镜像常量仍指向 core 常量）。
- `cargo test --workspace`：见提交末尾的验收行（含 gallery 的整树门禁）。
- 删前核对的边界：`hovered_key` / `pressed_key` / `state` / `glyph` / `separator`
  在 gallery 与 examples 下零引用；`ui/parity.zypui` 没被任何 build.rs 编译。
