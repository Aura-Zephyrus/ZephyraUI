# AGENTS.md

> 给 AI 代理与新来的人看的**仓库规矩**。人手写的东西放 `docs/`，别放这里。

本文件回答三件事：**去哪读、怎么跑与验、哪些边界不能碰**。

---

## 一、先读这三处

| 文件 | 什么时候读 |
|---|---|
| [`docs/developer/README.md`](docs/developer/README.md) | 对外规范：数据流 / 样式表 / 帧流水线 / 后端 |
| [`docs/dev-log/README.md`](docs/dev-log/README.md) | 本仓**排除过的假设**。动 GPU / 呈现 / resize / 动画之前必读；末尾「长期存在的问题」写着哪些事**别当成已完成** |
| [`zephyra/zephyra-base/README.md`](zephyra/zephyra-base/README.md) | 三层结构、base 的三条禁令、这一层还缺什么 |

⚠ 文档里有一批路径还是上次搬家前的写法：`docs/win/…` 现为 `docs/developer/win/…`，
`docs/portability.md` 现为 `docs/developer/portability.md`。旧路径已不存在，别照着改错位置。

**语法权威**：[`docs/developer/syntax.md`](docs/developer/syntax.md) 是唯一语法权威（四所有权 /
决策过程 / 各语言语法 / 动画能力清单）。下面第三节只放摘要，细节以该文件为准。
**未经登记的新语法一律拒绝**，旧语法不保留兼容层。

---

## 二、跑与验

| 门禁 | 命令 |
|---|---|
| 格式 | `cargo fmt --all -- --check` |
| 代码规范 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 测试 | `cargo test --workspace` —— 1175 通过 / 0 失败 / 72 ignored（vector 294 条、camera 10 条已独立成仓）（本机 2026-11-21；视频解码、CLI、gallery 均已独立成仓，不再计入；各仓门禁见对应 README） |
| 隔离门禁 | vector 已独立成仓（`E:\zephyra-vector`）：其 `cargo tree -p zephyra-vector --depth 1` 只出现 `ab_glyph` / `bytemuck`；主仓通过 git 依赖它。camera / video 也已独立成仓（`E:\zephyra-camera` / `E:\zephyra-video`），主仓与它们**零双向依赖**（它们不依赖主仓）。唯一例外：`zephyra-base/extra/zephyra-media` 单向 path 依赖 zephyra-video；它是可选播放桥接、不进 default-members，默认构建连依赖解析都不做。主仓核心仍零依赖 |
| 文档 | `cargo doc --no-deps --workspace` —— 退出 0。**警告数取决于重编范围**（warm 全量实测 43 条，全部来自 `zephyra-ui` / `zephyra-fx` / `zephyra-style` / `zephyra-winit-d3d12` 的既有链接；判据是「本轮有没有**新增**」而不是总数）。bin 目标不受门禁保护 |

单点验证：

```text
cargo test -p zephyra-core <filter>          # 单 crate / 单条用例
cargo test --manifest-path ../zephyra-vector/Cargo.toml --workspace  # vector 独立仓：几何 / 图标 / 文本 / 光栅（294 条）
cargo test -p zephyra-dcomp --lib            # 纯逻辑断言，不要窗口
cargo test -p zephyra-dcomp --test smoke     # 真窗口 + 真 GPU
# gallery 已迁至独立仓 E:\zephyra-gallery（https://github.com/Aura-Zephyrus/ZephyraUI-gallery.git）
# 它单向依赖本仓；控件层的整树门禁（tests/gates.rs）也随之迁出
cargo run --manifest-path ../zephyra-gallery/Cargo.toml -p zephyra-gallery --bin gallery -- --list-routes --dump-cpu --route <id>
cargo run --manifest-path ../zephyra-gallery/Cargo.toml -p zephyra-gallery --bin gallery -- --dump-cpu --bench --alloc-probe
# CLI 与编译器前端已迁至独立仓 E:\ZephyraCLI（https://github.com/Aura-Zephyrus/ZephyraCLI）
# 主仓保留 zstyle-compile / zypui-compile 两个编译库（gallery build.rs 用 zypui_compile::compile）
cargo run --manifest-path ../ZephyraCLI/Cargo.toml -p zephyra-cli -- <子命令>
cargo run --manifest-path ../ZephyraCLI/Cargo.toml -p zstylec -- fmt [--check] <file.zstyle>
cargo run --manifest-path ../ZephyraCLI/Cargo.toml -p zypuic -- fmt [--check] <file.zypui>
# CSS @keyframes → 运行时 SMIL（构建期导入；输出是普通 SVG，与手写 SMIL 同路）
cargo run --manifest-path ../ZephyraCLI/Cargo.toml -p zanim -- import <file.css> [-o <out.svg>]
```

### `--workspace` 不是可选项

`default-members` 只有 core / fx / zephyra-winit，所以裸 `cargo test`
只跑 **493 通过 / 18 ignored**（本机 2026-11-20 实测）。另外这些 crate 在 `members` 里但**不在**
`default-members`，
无差别的 `cargo build` / `cargo test` 连编都不编（这是有意的：默认构建不该编译
unsafe 与设备代码）。23 个成员里有 **20 个**不在 `default-members`：
`zephyra-svg-anim`、`zephyra-style`、`zstyle-compile`、`zypui`、`zypui-compile`、
`zephyra-winit-d3d12`、`zephyra-winit-input`、`zephyra-uia`、
`zephyra-dcomp`、`zephyra-platform`、`zephyra-platform-win`、`zephyra-app-config`、
`zephyra-window`、`zephyra-platform-linux`、`zephyra-vulkan`、`zephyra-bezier`、
`ZephyraUI`、`zephyra-render-ext`、`zephyra-ext`、`zephyra-ext-wgpu`。
要跑要编就 `-p` 点名。

### `extra/` —— 「不是所有项目都需要」的包

有**两个** extra 目录，按**判据**分流，而不是按"随便放哪"：

| 位置 | 判据 | 现有成员 |
|---|---|---|
| `zephyra/zephyra-core/extra/` | 零依赖 + 无 GPU 可单测的纯逻辑 | `zephyra-bezier` |
| `zephyra/zephyra-base/extra/` | 要碰平台 / 允许 unsafe，或可选播放栈 | `zephyra-uia`、`zephyra-media`（可选播放桥接；单向 path 依赖 zephyra-video，见上方门禁例外） |

**「选择性编译」是 Cargo 层面的事实**：这些包在 `members` 里但不在
`default-members` 里，所以无差别构建**连依赖解析都不做**。不要在 extra 里
加 `#[cfg(feature)]` 当作"选择性编译"—— 那只省编译，不省依赖解析。

它们是**独立 crate**，不是 core / base 的子模块。于是 core 的「无外部依赖」门禁
比"子模块"更硬：extra 里带依赖，core 完全看不见（`cargo tree -p zephyra-core --depth 1`
只有 core 与 `zephyra-vector` 两行）。

`--workspace` 当前共 **72 条 `ignored`**，散落在三个后端与诊断 / 桩测试里
（`command_validation.rs` / `swapchain_draw.rs` / `ext_contract_stubs.rs` /
`d3d12_draw.rs` / `sweep.rs` / `pso_bisect.rs` 等），不是集中在某一个 crate。
它们开真窗口或碰真设备，历史上真把设备拖进过 TDR，所以默认 `#[ignore]`。
**别为了让计数好看去删 `--ignore`。**

注意 `--workspace` 会把需要**真窗口 + 真 GPU** 的套件也跑起来（`zephyra-dcomp` 的
`smoke` 3 条不在 ignore 之列）。要在无显示器的机器上只验逻辑，退回 `cargo test`
（default-members）或显式 `--lib`。

### Linux（WSL2 实测，2026-10）

同一套门禁在 Linux 上可用，数字不同：

```bash
cargo fmt --all -- --check                                  # 干净
cargo clippy --workspace --all-targets -- -D warnings      # 0 警告
cargo test --workspace                                      # 672 / 0 / 3 ignored
```

⚠ 两个非显然的地方：

1. **Linux 后端的验证在 gallery 新仓做。** 主仓 `--workspace` 已不含 gallery —— 它迁到了
   `E:\zephyra-gallery`，所以这里只需保证主仓自身在 Linux 上编过。gallery 的
   `default = ["d3d12"]`，而 d3d12 那个 crate 在非 Windows 上是空的
   （`#![cfg(windows)]`）。所以**判据必须是「平台 且 feature」**，只看 feature 会
   在 `use zephyra_winit_d3d12::HostWindow` 上炸 —— 症状像「Linux 适配把应用弄坏了」，
   真相只是「默认 feature 指向一个本平台为空的 crate」。真要把 Linux 后端编一遍，到新仓跑：

   ```bash
   # 工作目录 E:\zephyra-gallery
   cargo clippy --all-targets --no-default-features --features linux-vk -- -D warnings
   cargo run --bin gallery --no-default-features --features linux-vk
   ```

   ⚠ 不要写 `--features zephyra-ui/linux-vk`：控件层零后端依赖，`zephyra-ui` 没有
   `linux-vk` feature，后端 feature 只属于 gallery 这个应用 crate。

   只给 `--features linux-vk` 而**不**给 `--no-default-features` 会把 d3d12 也编进来：
   能跑（`#[cfg]` 选中的那个出画面），但多编一遍 D3D12 的依赖链。

2. **Linux 上测试数比 Windows 少一截**，因为 Windows-only 的套件
   （`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-dcomp` /
   `zephyra-platform-win` 的 `tests/` 和它们的 `ignored` 探针）在那边根本不存在。
   **不是**少跑了什么；具体数字见各自 README 的实测记录。

3. **`cargo doc --workspace` 在 Linux 上必然失败**，失败的是三个 Windows-only crate
   （`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-dcomp`）：它们在 Linux 上是
   空 crate，而 rustdoc 照样解析 crate 级文档，于是指向自家模块的链接解析不到，
   配上 `#![deny(rustdoc::broken_intra_doc_links)]` 就**整个失败**。这不是本轮引入的
   （那三个 crate 的 `src/` 本轮一行没动）。Linux 侧改用逐 crate：

   ```bash
   cargo doc --no-deps -p zephyra-platform-linux -p zephyra-vulkan \
       -p zephyra-core -p zephyra-fx -p zephyra-platform      # 退出 0
   ```

   **新写 crate 级文档时**：指向自家模块的引用一律写**代码块**，不要用 intra-doc
   链接 —— 否则那个 crate 一旦被 `#![cfg(...)]` 整体关掉，就会在**别的平台**上
   把 doc 门禁弄红。模块**内部**随便用链接。

### WSL 上构建本仓的环境前置（**不是**仓库配置，别写进 manifest）

| 事项 | 做法 | 为什么 |
|---|---|---|
| `CARGO_TARGET_DIR` 指到 ext4 | `export CARGO_TARGET_DIR=~/zephyra-target` | 仓库在 `/mnt/e`（NTFS）上，Linux 构建绝不能写进 `E:\ZephyraUI\target`（那是 Windows 产物） |
| crates.io 走镜像 | `~/.cargo/config.toml` 里 `replace-with = 'rsproxy-sparse'` | 实测直连 `static.crates.io` 约 170 KB/s，`static.rust-lang.org` 约 37 KB/s，rsproxy 约 7 MB/s |
| 编译并发 | `-j 6` | WSL 常见 4 GB 内存；图形栈依赖树并行编译容易 OOM |
| **不需要** `apt install` | 什么都不装也能构建 | ash 直接 dlopen `libvulkan`；实测在**没有** `pkg-config`、没有 vulkan 头文件的机器上也能编过（naga 是纯 Rust，不依赖系统 glslang） |

---

## 三、四条不能碰的边界

1. **「这一帧画什么」只有一个地方决定**：`zephyra_core::frame::FrameCache`。
   宿主只回答「要不要推上屏」，不读 `Node::dirty()` 决策内容。这条被破过
   （`HostWindow` 曾长期存 `node` / `list` / `needs_layout` 三块状态），破了的
   症状就是本仓 GPU 问题反复出现的根源，见 `docs/dev-log/2026-10-gpu-device-hung.md`。
2. **绘制库已独立成仓**：`zephyra-vector` 在 `E:\zephyra-vector`（https://github.com/Aura-Zephyrus/zephyra-vector.git），主仓通过 Cargo.toml 的 git 依赖引用（不写 path）。依赖严格单向（主仓 → vector），vector 不依赖任何主仓 crate。`cargo tree -p zephyra-core --depth 1` 里 vector 以 git 源出现；vector 仓自己的 `cargo tree -p zephyra-vector --depth 1` 只出现 `ab_glyph` / `bytemuck`。vector 拥有全部后端无关的矢量绘制（几何 / 图标 / 文本 / 图集 / 顶点展开 / 光栅）。
   core 与 fx `forbid(unsafe_code)`；unsafe 只许出现在 `zephyra-base/`。
3. **外观不进源码**：控件只发 `key`（事件身份）与分类名（选择器），颜色 / 尺寸 /
   圆角全在 `.zstyle`，颜色走 `@令牌` → `Theme::role()`。整树门禁现居
   `E:\zephyra-gallery\tests\gates.rs`（遍历全部页面 × 全部交互态），比你想的更容易红。
4. **三个渲染后端互不依赖**：`zephyra-winit`（D3D11）/ `zephyra-winit-d3d12` /
   `zephyra-vulkan`（raw ash，**不用 wgpu**）只共同依赖 `zephyra-core` + `zephyra-fx`。
   **不要**把 Linux 后端并进 Windows 那两个，也不要在宿主里写 `#[cfg(windows)]` 分支挑后端 ——
   D3D11 / D3D12 / Vulkan 是三套词汇，揉进一处后「画面一空时是哪个后端的问题」
   就没法回答了。后端由**特性开关**选，同一个二进制只会有一个。
   跨后端共享的只有「后端无关的那一半」，它住在 `zephyra-fx`
   （`DisplayList → 顶点流` / 字形图集 / 纹理表 / CPU 光栅器）。
   着色器同源：Windows 用 `zephyra_fx::shaders` 里编译好的 DXBC，Vulkan 在
   `zephyra-vulkan/build.rs` 里用 naga 把 `zephyra_vector::shaders::wgsl_source()`
   编成 SPIR-V。仓库里**没有**独立的 `.wgsl` / `.hlsl` 文件了，别去找那份「手抄双份」。

### 语法宪法摘要（全文见 `docs/developer/syntax.md`）

四所有权：状态值与流转规则在 Rust 控件模块；节点树 / 分类名 / key / 资源引用在
`.zypui`；颜色尺寸圆角在 `.zstyle`（颜色走 `@token`）；状态存在在宿主；时间归应用侧
driver。`core::theme::roles` 是 token 唯一权威。

新增可声明之物按顺序问：① 外观→`.zstyle`；② 结构/引用→`.zypui`；③ 状态/规则→Rust
控件；④ 时间/程序化→应用 driver；⑤ 部署配置→JSON/env；⑥ 都不是→先用现有「引用」
表达，确实装不下才去 syntax.md 登记新表面。**未经登记一律拒绝，旧语法不做兼容层。**

动画语法：运行时只有 SMIL（`zephyra-svg-anim`），CSS `@keyframes` 仅作构建期 `zanim`
导入工具的输入；`.zypui` 的 `animation` 只引用 id / 路径；颜色必须 `@token`；时钟归
应用；`FrameCache` 只新增 `set_post_diff` 钩子，不改 diff 决策。能力清单
（Tier A/B/C）见 syntax.md 第 6 节，清单外明确不支持。

### 加一个属性要动两处

`zephyra/zephyra-core/src/props.rs` → `zstyle-compile/src/props_map.rs`（kebab-case
映射表，`.zstyle` 侧**唯一**入口；`zstylec` 二进制已迁至 CLI 仓，主仓只留编译库）。

- `affects_layout` 有门禁兜底：`layout_classification_tests.rs` 会因漏声明而红。
  漏声明的后果是「只改颜色」也触发全树重排。
- **映射表那一侧没有门禁**：`props_map.rs` 的模块文档写着「B2 里那条对齐测试会红」，
  但那条测试并不存在。所以漏加映射的症状是——那个属性在样式表里**永远设不了，
  且没有任何提示**。加属性时自己核对，别指望测试喊你。

`.zypui` 的裸属性另走一条：`zypui/src/ir.rs` 的 `BARE_ATTRS`（zypuic 校验白名单）
**和** `zypui/src/instantiate.rs` 的 `match` 分支，两处都要加。只加前者能通过校验、
但 `instantiate.rs` 里落进 `_ => el_el` 被静默丢弃，症状同样是「设了没反应」。

---

## 四、注释与代码

### 1. 模块文档（`//!`）与函数文档（`///`）

模块文档超过一屏就搬去 `docs/`，源码只留三行摘要加一个链接。

**每个实测数字必须带测量条件**——一个孤零零的 `162 µs` 半年后没人知道是哪个场景、
什么分辨率、多少节点测的，那时它比没有更糟。

```rust
/// 四阶段合计 162 µs（gallery，45 节点，1400×800）
// ↑ 数字 + 条件 + 规模，缺一不可
```

### 2. 本仓硬约定

1. **行内 `//`：写「不这样写就一定会错」的东西**，例如某个 API 要求 `flush` 之前
   不能改描述符、某段顺序被 spec 钉死、某处漏了会 panic。解释结构的搬进 `///`。
   （注：现状是生产源文件里约 3,300 行行内注释，`zephyra-base` 与 core/fx 占八成，
   Win32/COM/GPU 边界与不变量说明是它们的正当位置——所以判据是「说了会不会错」，
   不是「有没有」。）
2. **行内注释禁止出现十六进制回码**（`0x887A0001` 这类）。它们属于 `docs/dev-log/`：
   回码会随 Windows 版本变化，而它恰恰**最像**有价值的注释——越像，越该放到有日期、
   会复核的地方。
3. **定位信息必须与结论成对出现。**
   - 「Chromium `dc_layer_tree.cc:626` 是在提交之后才通知」✅ —— 路径是证据，
     去掉它这条结论就没法复核。
   - 「见 `foo.cc:81`」❌ —— 只有路径没有结论。这类是纯噪音，而且会随上游改动
     悄悄指错地方。

   十六进制回码（`0x887A0001`）**保留**，但必须与符号名成对：
   `DXGI_ERROR_INVALID_CALL（0x887A0001）`。符号名会随文档变，裸码不会；
   只写符号名又会让「去哪儿搜」变麻烦。
4. **标识符一律英文，函数名（含测试函数名）禁止中文**（中文标识符已在 `a8c5a4a`
   全量清零；测试函数名的中文也已在 2026-10-29 全量改为英文）。中文只出现在
   **文档注释的散文**里。测试函数名用英文描述断言命题，例如
   `absolute_child_does_not_push_siblings`。
5. **测试体不进生产文件**：源码里只留声明，实现放独立文件。

   ```rust
   #[cfg(test)]
   #[path = "../../tests/unit/filter_tests.rs"]
   mod tests;
   ```

   用 `#[path]` 而不是直接改文件名，是为了让模块路径不变——`use super::*` 与对
   父模块私有项的访问照旧，不必把私有项提成 `pub(crate)`。

   **测试文件住 crate 根的 `tests/unit/`**，按生产模块的边界分文件
   （`tests/unit/input/router.rs` 对应 `src/input/router.rs`）。这样 `src/` 里
   **只有生产代码**，而 `cargo test -p <crate> <模块路径>` 能只跑改动的那部分
   （`cargo test -p zephyra-core input::router` ⇒ 23 条）。
   `tests/unit/` 是**子目录**，cargo 不会把它当集成测试目标 ——
   `tests/*.rs` 才是。

   gallery 仓（已迁至 `E:\zephyra-gallery`）是特例：它的 `tests/` 是 cargo 的集成测试目录
   （门禁），所以单元测试住 `gallery/tests/unit/` 而不是 `gallery/src/tests/`。
6. **命名规范就是「通过 clippy 的 `upper_case_acronyms` 且 clippy 零警告」**：
   `SRV` / `PSO` / `ID` 全大写。不要另立「缩写要不要展开」「变量该多长」这类规矩——
   机器查不了、也没有共识，写进规范只会变成口水仗。
7. **编译库与命令行前端分离**：`zstyle-compile` / `zypui-compile` 留在本 workspace；
   CLI 与两个前端迁到独立仓 `E:\ZephyraCLI`，通过路径依赖本仓的编译库。主仓不再出现
   zstylec / zypuic 这两个名字。依赖严格单向（CLI 仓 → 本仓），「CLI 看到的属性表」与
   「编译器看到的属性表」仍是同一份 lib。
8. **应用也独立成仓**：`gallery`（参考应用 + 控件层整树门禁）迁至 `E:\zephyra-gallery`，
   单向依赖本仓（crate 依赖 + `build.rs` / `tests/gates.rs` 读取 `ZephyraUI/ui/controls.zypui`）。
   主仓不再包含任何应用 crate。
9. **不加「注释必须少」这类 lint**：它会逼出废话注释凑数，与第 1 条正着来。
   注释长度只能靠评审 + 下面那些间接门禁约束。

---

## 五、环境与工具链的坑

- `cargo fmt` 会刷一屏 `Warning: can't set wrap_comments = false`——那是 nightly-only
  选项，stable 直接忽略（stable 本来就不折行注释）。**别去删配置、别为它改 nightly。**
  实测：stable 上对 `zephyra-core/src/frame.rs` 跑一遍 rustfmt 零 diff，那张 µs 对照表
  的列对齐不会被冲散。
- **PowerShell 默认编码把中文打成乱码，文件本身是真 UTF-8。** 别照着 shell 输出
  「修」注释。仓库里已有 **54 个 U+FFFD 分布在 21 个文件**（如
  `zephyra-fx/src/quad.rs`），是历史损坏——看到别扩散，也别当成待修的谜团。
- **一个二进制只编一个后端**：`d3d12` / `d3d11` 由 feature 选，不做运行时切换
  （两个都编进来时「画面一空」无法归因）。gallery 仓（`E:\zephyra-gallery`）的 bin **不带**
  `required-features`：Linux 上 d3d12 是空 crate，而 `required-features` 表达不了
  「d3d12 或 linux-vk」⇒ 照原样写这个 bin 在 Linux 上**永远存不住**。所以改成
  「bin 恒可编，后端由 `gallery/src/bin/main.rs` 里的 `#[cfg(feature)]` 分流」，没开任何
  后端 feature 时 `main()` 打印一行提示并退出。
- **dev / release 编译期分离**：所有诊断设施（调试桥、`ZEPHYRA_MCP_PORT`、profiler、
  `cpu_only`、`ZEPHYRA_FRAME_DUMP`、`ZEPHYRA_TRACE`、`alloc_probe`）都用
  `#[cfg(debug_assertions)]` 门控 —— release 编译期彻底剔除，没有运行时开关可绕过。
  验证：`cargo build --release` 后在 `gallery.exe` 里搜不到 `ZEPHYRA_DEBUG_PORT` /
  `hello ` 等字符串。需要在 release 性能下调试时只能用 debug 构建（刻意取舍）。
- 环境变量（**仅 debug 构建有效**）：`ZEPHYRA_TRACE=1`（每帧统计，`zephyra dev` 默认开）、`ZEPHYRA_DCOMP`
  （切 DirectComposition 呈现路径）、`ZEPHYRA_FRAME_DUMP=<path>`（把某帧后缓冲写成
  BMP，给「肉眼才看得见」的故障留证据）、`ZEPHYRA_MSAA`、`ZEPHYRA_ADAPTER`、
  `ZEPHYRA_FONT`（字体覆盖）、`ZEPHYRA_FRAMELESS=1|0`（强制无边框/带系统标题栏，
  优先于 `ZephyraUI.json` 的 `window.frameless`；配置按 cwd 解析，直接运行
  `target/debug/<exe>` 时 cwd 往往不含它——自绘标题栏的应用用这条与 cwd 无关）、
  `ZEPHYRA_TRANSPARENT=1|0`（同款覆盖，钉住主窗口默认值 `window.transparent`；
  窗口透明指逐像素 alpha 进合成器，不是 DWM 材质——见
  `docs/developer/win/07-window-transparent.md`）。
  `--dump-cpu` 走 `zephyra-fx::cpu_only::CpuHost`（**仅 debug 构建**）：
  不开窗口、不碰 GPU，出 BMP——**无显示器的环境也能验整条链**。
- `#![deny(rustdoc::broken_intra_doc_links)]` 只写在部分 `lib.rs` 里。**bin 目标不受
  门禁保护**（`text_probe.rs` 那 2 条警告就是这么漏过去的）。
- SVG 动画只走 SMIL（`zephyra/zephyra-svg-anim`，`<animate>` / `<animateTransform>`
  / `<animateMotion>`），**不实现 CSS 动画**；该 crate 只依赖 `zephyra-vector` 与零依赖
  的 `zephyra-bezier`，不依赖 core，产物是纯数据。

---

## 六、提交怎么写

- 正文带**实测数字 + 条件**（命令、节点数、分辨率、前后对照）。没有实测收益就明写
  「本条没有实测收益」并解释为什么（例如 bench 根本没走到那条路）——这类记账本身
  就是信息。
- 结尾一行验收（`747 passed / 0 warning` 这类）。
- 排查出一个坑 → 写 `docs/dev-log/YYYY-MM-<slug>.md`，四段式：现象 / 排查 /
  **排除的假设** / 结论 + **门禁**（哪条测试钉住它）。「排除的假设」不能省：
  下一个来的人靠它省下几轮，这是 dev-log 存在的全部理由。

---

## 七、验收一条改动

```text
cargo fmt --all -- --check                                    # 干净
cargo clippy --workspace --all-targets -- -D warnings        # 0 警告
cargo test --workspace                                        # 全绿
cargo doc --no-deps --workspace                               # 不新增警告
```

四条都过了才算完成。少一条都不算——尤其别拿「测试过了」当通行证：

> 真实发生过：往文件追加内容时首行 `#[test]` 被吞掉，**编译照过**，只是多了两条
> `never used` 警告。测试绿 ≠ 改全了。