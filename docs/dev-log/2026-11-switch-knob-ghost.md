# 开关把手的残影（把手滑回起点那一帧）+ 主题切换要切页才生效

> 2026-11-11 · Gallery（D3D12 后端）· 现象来自用户截图
> `屏幕截图 2026-11-10 115238.png`（控件参考 → Switches → 「纯色」开关）

## 现象

1. **开关把手**：把手从「开」滑回「关」（或反之）时，屏幕上会**同时出现两个把手**——
   一个在当前位置，一个在上一帧的位置（两个白色胶囊重叠，白色跨度 54～60px，
   而把手本身只有 40px）。下一次交互后残影消失，所以它表现为「闪一下的脏东西」。
2. **主题切换**：点主题按钮后画面**不换色**（只有被点的那一小块变了），**切一次页面**
   才整屏换成新主题。

两个现象是**同一个类**的病：帧内重绘只覆盖了「新位置」，旧像素没人负责清。

## 排查

### 1. 先把「两个把手」量化，别靠眼睛

截图里那坨白色按行量宽度：中间行 166→219 共 **54px**（把手 40px）。
⇒ 不是「一个把手被拉宽」，而是**两个 40px 把手重叠**（偏移 ≈14px = tween 一帧的移动量）。
⇒ 是**上一帧的把手留在屏幕上**，不是元素树里有两个把手。

同一张截图里另一个（玻璃）开关只有一个 40px 的把手 ⇒ 与材质无关，与滤镜无关。

### 2. 复现要绕开 MCP 截图

MCP 的 `screenshot` 走的是**整帧**路径（`capture` ⇒ `Damage::Full`），所以它**天然
看不到残影** —— 每次截图都会把上一帧的像素全清一遍。改用 `PrintWindow`
（`PW_RENDERFULLCONTENT`）抓 DWM 合成后的真实画面，逐帧才有意义。

### 3. 复现步骤（真实输入）

`SendMessage` 注入 `WM_MOUSEMOVE → WM_LBUTTONDOWN → WM_MOUSEMOVE（一次跳到端点）→
WM_LBUTTONUP`：把手在**一帧内**从 20px 位跳到 0 位。

旧构建（`target\debug\gallery.exe`，2026-11-10 11:51）实测：

```text
old_drag_05  白块 458..497   （把手在「开」位）
old_drag_06  白块 438..497   ← 60px：两个把手并排（残影）
old_drag_07  白块 438..477   （残影被下一帧清掉）
```

修好之后（同一段注入、同一台机器）：白块恒为 `438..477`（40px，单个把手）。

### 4. 根因

`zephyra-core/src/frame.rs` 的脏区计算里，`scan` 用这条判据决定要不要退让成整帧：

```rust
if node.translate_x(ctx) != 0.0 || … { *force_full = true; }
```

——「脏节点的**当前**绘制变换非恒等」。

把手滑回起点那一帧，变换**恰好回到恒等**（位移 0、缩放 1）⇒ 判据不成立 ⇒
脏区收窄成把手**新位置**的矩形 ⇒ **旧位置的像素没人清**（没有旧矩形快照）⇒
残影。

同一处判据还漏掉「拆掉变换属性」与「样式表改了变换」两条路。

而 MSAA 目标纹理是**逐帧存活**的（帧末整体 resolve 进后缓冲），所以脏区之外的
上一帧内容确实还在 —— 残影就这样被推上屏。

主题切换是同一条路的另一头：`forced` 帧（宿主显式失效 / 换主题）原先也可能算出
局部 `Damage::Rect`，于是只有被点按钮那一小块按新主题重画。

## 排除的假设

- **「是 zypui 的 per-element 缓存命中了旧值」**：排除。布局树里始终只有一个
  `knob` 节点（`get_layout` 可见）；而且复现只跟**脏区算得对不对**有关 ——
  同一份元素树、同一段输入，只有 damage 从 `Full` 变成 `Rect` 的那一帧才出残影。
- **「元素树里真的有两个 knob」**：排除。同上（布局树只有一个），且残影只存在一帧、
  下一次交互就没了 —— 元素树层面的问题不会自愈。
- **「只有真实鼠标才复现，MCP 注入测不出来」**：排除。`SendMessage` 注入
  （`WM_*` 消息）与 MCP 注入（应用内事件）**都能复现**；之前测不出来是因为
  **MCP 截图走整帧**，把残影擦掉了。
- **「是滤镜 / 离屏 pass 留下的」**：排除。残影出现在**实色**开关上（没有滤镜），
  且另一个玻璃开关在同一张截图里是干净的。
- **「把手位移不是 translate 而是布局」**：排除。`controls.zypui` 里把手走
  `translate-x`（`.switch-track.on.animated { main-align: start }` 把静态
  `main-align: end` 压掉），实测把手位移 20px 与 `translate` 一致。

## 结论 + 门禁

**修法**（`zephyra-core`）：

1. 节点新增 `moved` 标记：「**本帧绘制变换变过**」。判据是「变了」，不是
   「变成了什么」——回到恒等那一帧同样会移走。
   写变换属性的三条路都标它：`diff` 写本地值、`Node::set_prop`、
   `Node::set_styled_prop`（样式表也能改位移），以及两条「拆键」路径
   （`retain_local_props` / `retain_styled_props`：拆掉变换 = 变回默认值）。
2. `scan` 的判据改成 `node.moved() || 当前变换非恒等` ⇒ 变换变过 ⇒ 脏区退让成
   `Damage::Full`。
3. `forced` 帧（宿主显式失效 / 换主题 / present 恢复）一律 `Damage::Full`：
   即便树上只有局部脏节点，清屏色与全局解析上下文也可能已变。

**门禁**（`zephyra-core/src/frame_tests.rs`，两条新用例）：

- `damage_falls_back_to_full_when_a_dirty_node_returns_to_identity_transform`
  —— 位移从 20 回到 0 那一帧必须整帧（**改之前是红的**）。
- `damage_falls_back_to_full_when_a_dirty_node_returns_to_unit_scale`
  —— 缩放回到 1.0（松手）那一帧必须整帧（**改之前是红的**）。

**实测**：

```text
cargo test -p zephyra-core --lib                436 passed / 0 failed
cargo clippy --workspace --all-targets -- -D warnings   0 警告
cargo fmt --all -- --check                      clean（stable 的 wrap_comments 警告除外）
```

真机 A/B（`SendMessage` 一次跳到端点，逐帧 `PrintWindow` 量白色跨度）：

```text
旧构建（2026-11-10 11:51 的 target\debug\gallery.exe）：438..497（60px，两个把手）
本次修复后的构建                                        ：438..477（40px，一个把手）
```

**为什么不用「记住上一帧的旧矩形」**：那要每帧给每个节点存一份绝对矩形 ——
本仓当初就是为了省掉它才退化成「变换非恒等就整帧」。现在改成「变换**变过**就整帧」，
判据同样便宜（一个 bool），却把漏掉的那一帧补上了。
