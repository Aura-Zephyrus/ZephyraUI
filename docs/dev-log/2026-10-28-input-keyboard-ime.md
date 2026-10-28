# 2026-10-28-input-keyboard-ime

> 现象：文本类控件（`TextField` / `TextArea` / `SpinBox` / `AutoSuggestBox`）
> 无论怎么点都只能显示调用方给的字符串 —— 因为它们拿不到「用户按了什么」。
> 根因不在控件层，而在输入链路的最后一公里：核心只有 `KeyInput { key: String,
> pressed: bool }`（键名 + 边沿），没有可打印字符，没有 IME 的三阶段，也没有
> 修饰键与自动重复。

## 排查

**核心侧**：`zephyra-fx` 的 `InputState.key_events: Vec<KeyInput>` 里
`KeyInput` 只有 `key: String` 与 `pressed: bool`。缺的四样：

| 缺什么 | 后果 |
|---|---|
| 可打印字符 | `TextField` 只能靠「键名是单字符」猜用户按了什么，Shift 与键盘布局全丢 |
| IME 三阶段 | 中文 / 日文 / 韩文根本进不来 —— 宿主的 `window_event` 里**没有** `WEvent::Ime` 分支 |
| 修饰键 | Ctrl+A / Shift+方向键无法区分，全选与按词移动这类命令写不出来 |
| 自动重复 | 长按退格不能连续删除；反过来快捷键又会被连发 |

**宿主侧**：`logical_key_name` 有两份拷贝（`zephyra-winit/src/lib.rs:951`、
`zephyra-winit-d3d12/src/host.rs:1688`），而 `zephyra-vulkan/src/host.rs` 里
根本没有这个函数 —— 它的 `sync_input` 直接写：

```rust
key_events: Vec::new(),
scroll: None,
```

于是 Linux 那条链路上键盘与滚轮从未接通过，且**没有任何东西会报错**。
这是「三份要永远同改」的典型结局：漏的那份没人发现。

**消费侧**：全仓 grep `key_events` 在 `gallery/` 与 `examples/` 下**零命中**。
键盘事件被塞进 `InputState` 传了一趟，然后丢掉。

## 排除的假设

- **「把 winit 事件原样透传进 `InputState` 最省事」**——省的是这一轮，代价是
  平台词汇漏进控件层：换窗口系统（或加一个非 winit 宿主）时每个控件都要重写。
  而且 `winit::KeyEvent` 的 `platform_specific` 是 `pub(crate)`，**外部 crate
  造不出来** —— 透传即意味着这段判据不可测。
- **「翻译可以放进 `zephyra-core`」**——core 零依赖，`cargo tree -p zephyra-core
  --depth 1` 只有它自己一行，装不下 winit。
- **「翻译可以放进 `zephyra-fx`」**——fx 的 `cpu_only::CpuHost` 不开窗口，装上
  winit 会把 CPU 光栅路径一起拖进窗口依赖，「无显示器也能验整条链」就没了。
- **「翻译可以放进 `zephyra-platform`」**——那是零依赖的能力定义 crate，同样
  装不下 winit，而且它一行平台代码都没有，加进去就破了这个性质。
- **「让三个宿主各自写一份，反正只有几十行」**——实测这个形态**已经**漂移过：
  Vulkan 那份从来没写。三个后端互不依赖（AGENTS 第 4 条边界）说的是「不许在
  宿主里 `#[cfg]` 挑后端」，不是「同一段翻译要写三遍」。
- **「IME 文本用 `&'static str` 省一次分配」**——每帧取走即丢的瞬态，用它就得
  leak 或 interner 每次按键，那才是真的堆放大器。先例：`InputState.languages`
  本身就是 `Vec<String>`。
- **「`Ime::Update` 的光标用字符下标更自然」**——winit 给的是**字节**下标对，
  而控件最终要切片 `String`，字节边界是 `&str` 直接可用的；换成字符下标反而
  每次都要走一遍 `char_indices` 换算。

## 结论

**三层分工，一层一份：**

| 层 | 位置 | 内容 |
|---|---|---|
| 词汇 | `zephyra-core/src/input/text.rs` | `Modifiers` / `Key` / `KeyInput` / `Ime` / `TextEvent`，零依赖、禁 unsafe |
| 翻译 | `zephyra/zephyra-base/zephyra-winit-input/` | winit → 上表，三宿主共用 |
| 消费 | 控件层（下一步） | 只认 `TextEvent`，不知道 winit 存在 |

### 契约里几个不显然的决定

1. **`Key` 是收窄的 13 项 + `Other`**，不是 winit 键位表的镜像。控件要的是语义
   （左箭头、退格），不是物理布局（`KeyCode::KeyA` 在 AZERTY 上是别的东西）。
   穷举全部键位等于把平台词汇搬进 core。
2. **`KeyInput` 同时有 `key` 与 `name`**。`key` 是语义（控件 `match`），`name` 是
   身份（快捷键表比对 `"F5"`）。一个字段做不到：枚举必须 `Copy` 才能无脑 `match`，
   而键名是运行期字符串。成本是每次按键一次小分配 —— 按键是用户节奏（每秒个位数），
   不在每帧热路径上。
3. **`Char` 与 `Key` 是两条正交通道**。「按下 A」与「文档里出现 `a`」在 Ctrl+A 时
   完全不是一回事；合并成一个事件会逼每个控件自己判 `if !ctrl { insert }`。
   `Translator::translate` 保证**边沿在前、字符在后**，于是控件可以用「按键处理过
   就吞掉后面的字符」这条规则，反过来则无法撤回。
4. **`printable` 是唯一判据，四道闸**：必须按下、不带 Ctrl/Alt/Super、winit 给了
   `text`、恰好一个字符。多字符的 `text` **整串丢弃**而不是取首字符 —— 少插一个
   比插错一个容易发现。控制字符（`\n` / `\t`）不走字符通道，否则「按 Enter 插两个
   换行」。
5. **`Ime::Start` 带插入点坐标，由宿主给**。core 不知道光标画在哪；焦点系统还没建，
   所以三个宿主目前统一用**最近一次指针位置**当代理，并在代码里写明这是代理。
   有了焦点系统之后改成向控件要 caret rect。
6. **`KeyParts` 这一层是给测试用的**。`winit::KeyEvent` 的 `platform_specific` 是
   `pub(crate)`，外部造不出来；若判据直接吃 `&KeyEvent`，那「Ctrl+C 不该插 c」
   这类规则就成了不可测代码。所以判据写在 `translate(KeyParts)`，`key_events`
   只做 `&KeyEvent → KeyParts` 的薄适配。
7. **winit 默认不允许 IME**（`Window::set_ime_allowed` 的文档原话是
   "IME is **not** allowed by default"）。三个宿主在建窗口后都显式
   `set_ime_allowed(true)`，否则 `WindowEvent::Ime` 永远不来 —— 那样本轮接的
   IME 分支就是**死代码**，而且没有任何测试会红（事件循环分支本来就只能真机验）。
   真实应用应当「文本框获得焦点时才开」：组字期间 winit 不送 `KeyboardInput`，
   所以组字中的快捷键本来也不该生效。焦点系统还没建，先全局开。

### MCP `press_key` 一起收口

`inject_key(&str)` 现在走 `zephyra_winit_input::injected_events(name)`，产出
`Key(按下) → [Char] → Key(抬起)`。判据与宿主侧同源：`Key::from_name` 认得的名字
（`Enter` / `ArrowDown`）只发边沿，由控件按语义键执行；认不出来且恰好一个非控制
字符（`a` / `中`）才追加字符；其余（`F5` / `ab`）只发边沿，身份留在
`KeyInput::name` 里。工具参数不变，AI 侧无感。

### 已知限制（本轮不做，记账）

- 控件层**尚未消费** `TextEvent` —— 本轮只做契约 + 三后端接线。第一个消费者是
  可编辑的 `TextField`。
- 焦点系统仍是空白（`props.rs` 里没有 `Focused` / `TabIndex`），所以 IME 的插入点
  是代理值；没有焦点就没有「这个键该给谁」的路由。
- `Layer::Popup` 仍是零生产者，`Z_INDEX` 键没人读（`paint.rs` 里 `z` 恒 0），
  `PushClip`/`PopClip` 仍是零生产者 —— 那是滚动与浮层的地基。

## 门禁

- `cargo test -p zephyra-core --lib input::text` —— **21 条**：键名双向闭合
  （`from_name(as_str(k)) == k` 与反向表）、修饰键四位与构造参数顺序、边沿/重复/
  修饰键快照、IME 三阶段（含 `cursor` 是字节对、`None` = 不显示光标、`Clone` 是
  深拷贝）、`Char` 与 `Key` 正交、多字符与不可打印字符的处理。
- `cargo test -p zephyra-winit-input` —— **38 条**：13 个编辑键逐个映射、非编辑键
  保留身份、字符键的语义是 `Other`、死键与未识别键不 panic、core 的每个语义键都有
  winit 来源（反方向门禁）、修饰键是覆盖而不是累加、`printable` 的四道闸逐条、
  `translate` 的顺序（边沿先于字符）、IME 四个变体与一次完整组字流程、MCP 注入
  路径的七条（单字符 / 编辑键 / 宽字符 / 未知功能键 / 多字符 / 控制字符 / 无修饰）。
- 四条门禁（`fmt` / `clippy -D warnings` / `test --workspace` / `doc`）与 gallery 的
  41 条整树门禁全绿。`cargo test --workspace`：**1011 passed / 0 failed / 52 ignored**
  （上一轮基线 952 / 0 / 51；+21 core 契约测试 +38 翻译测试，+1 ignored 是新 crate
  的 doctest）。`cargo doc --no-deps --workspace` 退出 0，41 条警告全部落在既有位置，
  本轮改动的文件零新增。
