# 触控适配与多点触控：把 `WindowEvent::Touch` 接上，以及「跟手」到底卡在哪

> 现象：触控屏上**点不动任何东西** —— 而应用明明「支持触控」（样式表里有
> `touch` 相关规则，代码里有 `PointerSource::Touch`）。

这一轮按官方**触控交互开发人员指南**（Windows apps）重做触控这一层，并顺手
查了「画面跟不跟手」。三个结论：

1. **触控根本没有输入路径**（`WindowEvent::Touch` 无人处理）；
2. **「跟手」没有被测过** —— 测完发现瓶颈不在 CPU，而在**没人测**；
3. **取消 ≠ 抬起**：把取消当抬起会**误触发点击**（Windows 11 的三指/四指手势
   正是走这条路）。

依据（2026-11 查，`zh-cn`）：
<https://learn.microsoft.com/zh-cn/windows/apps/develop/input/touch-developer-guide>

---

## 一、触控为什么完全不响应

### 现象

gallery 在触控屏上：点导航项没反应、点开关没反应、拖滑块不动。

### 排查

先确认「触控事件有没有到应用」。

`winit` 0.30.13 在 Windows 上会调 `RegisterTouchWindow(window, TWF_WANTPALM)`
（`platform_impl/windows/window.rs:1129`）。而 `RegisterTouchWindow` 的语义是
**关掉「触摸提升为鼠标」** —— 触摸之后不再产生鼠标消息。

于是：触摸事件**既不是触摸也不是鼠标**，而本仓三个宿主
（`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-vulkan`）都**没有**
`WindowEvent::Touch` 的分支 ⇒ 事件被整个丢掉。

核心里其实早有触控的词汇：`PointerSource::Touch`、`PointerTable`（8 槽定点表）、
`Recognizer` 的两指捏合、`momentum`（惯性）—— **全是为触控准备的，只是没有
一条路把它们接上**。

### 结论

补一条输入路径，并把触控该有的语义补齐（下一节）。

---

## 二、指南要求的那几条语义，本仓原来一条都没有

| 指南原文 | 本仓原来 | 现在 |
|---|---|---|
| 触控使用**双状态模式**……**没有可触发其他视觉反馈的悬停状态** | 触控会写 `hovered`（悬停与按下同时亮） | 触控**不产生悬停**（`PointerSource::can_hover`） |
| **指针取消**：在**异常失去**触控交互时发生 | 没有「取消」这个相位 | 新增 `PointerPhase::Cancel` |
| 操作事件提供的信息**不会标识执行的交互形式**，而是包括位置、平移增量和速度等数据 | 只有两指捏合（窄口径） | 新增 `ManipulationDelta`（平移 / 缩放 / 转角 / 速度） |
| 静态手势：Tap / DoubleTap / Holding | 没有 | 新增 `StaticGesture`（轻点 / 双击 / 长按） |
| 不应按所用手指数来区分操作 | —— | 静态手势只跟踪**一根**主指针；多指走操作数据 |
| 尽量不要使用计时交互（适当用例：**长按**） | —— | 只有长按是计时的；轻点/双击的时长只是**上限** |

### 「取消 ≠ 抬起」是最要紧的一条

Windows 11 默认把三指 / 四指手势交给系统（窗口切换、虚拟桌面）。那时应用
收到的是**取消**，不是抬起。

若把取消当抬起：用户手指**还在屏幕上**，应用却以为用户松手了 ⇒
**误提交一次点击**。症状是「我明明没点它，它自己动了」。

实测（gallery，release）：第一次接上触控后，「按下 → 取消」**确实切了页**。
修法分两层：

- **core**：`PointerPhase::Cancel` 清按下态、**不**产出 `Click`；
- **应用**：`InputState.cancelled` 告诉应用「这一帧有取消」——
  因为应用的点击判据通常是「上一帧按着、这一帧松了」，而**取消也把 `pressed`
  清成 `None`**，应用光看 `pressed` 分不出「松手」与「被抢走」。

⚠ 只改 core 不够：应用那条「上帧按着 + 这帧没按着 = 点击」的重建逻辑照样
会触发。**取消这件事必须一路传到应用**。

### 静态手势与「点击」并存

轻点比点击多两条判据：**位移容差**（10 px，指尖抖动）与**时长上限**
（500 ms）。所以「在按钮上按住 800 ms 再松开」是 `Click` 但**不是** `Tapped`。

阈值取 WinUI 的默认手势参数：轻点位移 10 px / 时长 500 ms / 长按 500 ms /
双击间隔 300 ms / 双击位移 24 px。

### 多点触控的数据

`ManipulationDelta` 给的是**原始数据**：平移增量、缩放比、转角、质心、速度 ——
**不**说「这是捏合」。指南原话：「操作事件提供的信息不会标识执行的交互形式」。

两处实现细节是踩出来的：

- **参考指对要固定**：若每次取「当前的前两根指针」，一根手指抬起、槽位被
  另一根补上时，参考量会**跳变** ⇒ 一个稳定的手势突然冒出一大段缩放。
- **朝向要用「两指连线」而不是点集的二阶矩**：二阶矩给出的是**轴**（周期 π），
  于是「转 180°」与「没转」分不出来 —— 而方向是有 2π 周期的。

### ⚠ 多指同时按两个控件：按下态曾经**互相覆盖**

按下态原来只有**一个槽**（`pressed: Option<ElementKey>`）。两根手指分别按在
两个控件上时，第二根**覆盖**第一根；而抬起时 `pressed.take()` 只取走**一个**
⇒ 第一根手指**收不到 `Release`**。

实测（两指分别按 a / b，先抬 a）产出的是：

```text
Release { pressed: "b", released: "a" }     ← 按下的是 b、松开的是 a，配对反了
```

两个症状叠在一起：

1. **第一个控件卡在按下态** —— 它的 `Release` 从此消失，控件永远回不到正常外观；
2. **配对反了** —— 那个 Release 说「b 在 a 上松开了」，于是**错误的控件**
   收到了提交。

触屏上「两根手指各按一处」很常见（多指缩放页面时手指常常各压一处），所以
这不是边角情形。

修法：按下态**按指针**记（`pressed_by_pointer: [(指针 id, 节点); MAX_POINTERS]`），
抬起时取回**这根手指**按下的那个。

⚠ 应用面还要能画**两个**按下态，所以 `InputState` 加了 `pressed_keys`
（全部按下中的节点）；`pressed` 保留为「主按下态」（最近一次按下的那个）。
单指针应用不受影响（集合长度 1）。

⚠ 而 `pressed_keys` **必须**把键盘激活（Space/Enter）也包进去 ——
`pressed` 那一栏是「指针的按下态，否则是键盘激活」，两栏必须**同款**口径，
否则「按 key 找自己」那条路在键盘激活时找不到自己（症状：Space 激活的按钮
画不出按下态）。这条是实测踩到的：改完立刻有两条 gallery 门禁变红。

---

## 三、「跟手」到底卡在哪 —— 测过才知道

### 先把延迟变成可测量的东西

新增 `FrameStats::input_latency_us`：**输入事件到达 → 承载它的那一帧就绪**的
耗时（不含 DWM 合成 —— 那一段是系统的）。宿主在处理指针事件时记下时刻，
帧末算出差值；调试桥的 `stats` 暴露它，CLI 的 `perf watch` 显示它。

顺带记一笔：`zephyra perf watch` 现在多一列 `lat_us`。

### 实测（gallery，D3D12，release，1400×800）

```text
$ zephyra bridge "touch down 0 64 122"
$ zephyra bridge "touch up 0 64 122"
$ zephyra bridge "stats --json"
{"quads":115,...,"stages":{"diff":31,"style":19,"scan":8,"layout":0,"paint":44,"total":103},
 "frames":8,"latency_us":null,...}
```

拆开看（release，逐段计时）：

| 段 | 耗时 |
|---|---|
| core（对账 + 样式 + 脏区 + 布局 + 绘制） | **103 µs** |
| 宿主记命令（`render_pass`） | **170~370 µs** |
| `present`（等 vsync） | 90~180 µs |
| GPU 等待（`wait_gpu`） | **1 µs** |

⇒ **应用这一侧整帧 ~0.5~1 ms**，是 60 Hz 帧预算（16.7 ms）的 **3~6%**。
**CPU 不是「不跟手」的原因**；余下的是 vsync 与 DWM 合成（系统的）。

### 三个被测量推翻的假设

- ❌「每帧 `wait_gpu()` 全等一次，所以卡。」—— **实测 1 µs**。GPU 跟得上时
  那个等待几乎为零。`docs/developer/05-frame-pipeline.md` §「一个必须记住的
  GPU 约束」把「顶点/索引只有一份 resident 缓冲」列成「唯一一处未解决的 GPU
  性能问题」—— **那句话在稳态下不成立**（实测 1 µs）；它成立的是「应用提交
  的帧比显示快时」—— 而那时等待是**正确的背压**，不是缺陷。
- ❌「`perf profile` 报 5 ms/帧，说明帧很贵。」—— **那是测量方法造成的**。
  采样器原先让宿主「每 tick 都渲一帧」且事件循环切到 `ControlFlow::Poll`
  （全速空转）⇒ 帧提交得比 GPU 消费得快 ⇒ 队列积压 ⇒ 帧变慢。**同一批操作
  在正常节奏下实测 `frame_total` 中位数 615 µs。** 采样器现在**只记录真的发生
  的帧**（静止 ⇒ `frames=0`，那本身就是结论）。
- ❌「图集每帧重传 4 MB，所以卡。」—— 图集**不是**每帧重传（`atlas_revision`
  变了才传），而重传时那 4 MB 是**每次新建一块上传堆缓冲**（`UploadBuffer::new`
  在函数局部，出了函数就析构）⇒ 一次 4 MB 的堆分配 + `CreateCommittedResource`。
  已改成**复用**（装得下就复用），实测图集上传那次从「每次新建」变成
  「一次 memcpy」。

### 「跟手」这件事本仓已经做对的部分（复核过）

- 输入事件在 `handle_event` 里处理 ⇒ `dirty = true` ⇒ `about_to_wait` 里
  `tick()` 跑帧 ⇒ **输入的效果出现在同一帧**（不是下一帧）。
- 调试桥的 `poll` 放在**帧判定之前** ⇒ 注入的输入也在同一帧生效。
- 命中测试用的是**上一帧**的布局 —— 那正是用户点击时**看到的**画面，
  不是缺陷。

---

## 四、排除的假设

- ❌「触控事件没送到 winit。」—— 送到了。`RegisterTouchWindow` 是 winit 自己
  调的；问题是**本仓没处理** `WindowEvent::Touch`。
- ❌「触控应该由系统提升成鼠标，应用收到 `MouseInput` 就够了。」—— 不够。
  提升成鼠标会**丢掉多指**（鼠标只有一个指针）与「取消」这个相位。而且
  winit 已经 `RegisterTouchWindow`，提升**根本没发生**。
- ❌「在 core 里加一个 `PointerSource::Touch` 就算支持触控了。」—— 词汇早就有了，
  缺的是**输入路径**与**语义**（双状态 / 取消 / 手势）。
- ❌「帧慢是 CPU 慢。」—— 实测 CPU 侧 0.5~1 ms；慢的是「提交比显示快」
  时的背压，以及 vsync 本身。
- ❌「取消当抬起处理，用户会立刻发现。」—— 发现不了。用户只会觉得
  「这个界面有时候会自己动一下」，而这正是最难查的一类投诉。

---

## 五、结论

- **触控输入路径此前不存在**：`WindowEvent::Touch` 无人处理，而 winit 的
  `RegisterTouchWindow` 已经关掉了「触摸提升为鼠标」⇒ 触摸被整个丢掉。
  现在三个宿主都接上了。
- **触控是双状态的**：不产生悬停（官方指南原文）。
- **取消不是抬起**：core 不产出点击，且 `InputState.cancelled` 一路传到应用
  —— 应用光看 `pressed` 分不出「松手」与「被系统抢走」。
- **静态手势**（轻点 / 双击 / 长按）与「点击」并存；长按是**时钟驱动**的
  （手指不动时没有事件，所以宿主每帧查一次）。
- **多点触控给原始数据**（平移 / 缩放 / 转角 / 速度），不替应用判断交互形式。
- **延迟是可测的**（`input_latency_us`），实测应用侧整帧 0.5~1 ms；
  **CPU 不是「不跟手」的原因**。
- **采样器不该改变被测对象**：原先它让宿主全速空转，于是报出来的帧成本
  比真实运行高一个数量级。

---

## 六、门禁

⚠ 测试文件已搬到 crate 根的 `tests/unit/`（见 §七），所以下面的路径是**新的**。

- `zephyra-core`（468 条）：
  - `tests/unit/input/touch.rs`（11 条）：触控不产生悬停 / 鼠标仍然产生悬停 /
    取消不产出点击 / 取消产出无目标的 `Release` / 轻点同时产出 `Click` 与
    `Tapped` / 两指并存 / 两指产出操作增量 / 原地点击不产出「操作结束」/
    拖动产出惯性速度 / **两指各按一个控件时各自收到 `Release`** /
    **两个按下态都在集合里**。
  - `tests/unit/input/gesture.rs`（12 条）：轻点 / 长按不是轻点 / 位移超容差
    不是轻点 / 抖动仍是轻点 / 双击 / 间隔太长不是双击 / 距离太远不是双击 /
    长按由时钟报出 / 挪动取消长按 / **取消不报轻点** / 取消断掉双击窗口 /
    第二根手指不破坏第一根的轻点。
  - `tests/unit/input/manipulation.rs`（9 条）：单指只有平移 / 两指缩放 /
    两指旋转 / 转角归一（不跳过一整圈）/ 速度 = 位移 ÷ 时间 / 最后一根手指
    抬起结束操作 / 还有手指时操作继续 / **取消不产生惯性** / 没动过就是 idle。
  - `tests/unit/input/router.rs`（12 条）：鼠标路径的悬停 / 按下抬起 / 点击 /
    捕获语义 / 复位。
- `zephyra-fx`（167 条）：`tests/unit/bridge.rs` 里的
  `touch_command_parses_the_phase_and_the_finger_id` —— 相位与手指身份；
  错的相位 / 少的参数**报错**，不静默当成抬起。
- `zephyra-winit-input`（43 条）：`tests/unit/pointer.rs` 的 `PointerTranslator`
  三条 —— 相位映射（含 **取消 ≠ 抬起**）、单调时钟不回退、鼠标事件的来源与 id。

---

## 七、附带：测试布局重整（源码只留生产代码）

这一轮还顺手把**本轮动过的 crate** 的测试搬出了 `src/`。

### 为什么

两个理由，第二个更要命：

1. **`src/` 里混着测试** —— `zephyra-fx/src/bridge.rs` 有 **1200 行内联测试**
   （`mod tests { … }` 直接写在生产文件里），生产文件 3500 行、其中三分之一
   是测试。读生产逻辑要先跳过它。
2. **改一处要重跑全仓** —— 测试按「一个大文件」组织时，改路由也得重跑命中
   测试；而每次都 `cargo test --workspace` 又是本仓最慢的一条命令（本机
   约 2 分钟），于是大家都不愿意跑测试。

### 搬法

测试文件住 **crate 根的 `tests/unit/`**，按**生产模块的边界**分文件：

```text
zephyra-core/
  src/input/router.rs        ← 生产代码 + 两行声明
  tests/unit/input/router.rs ← 路由测试
  tests/unit/input/touch.rs  ← 触控语义测试
  tests/unit/support.rs      ← 跨模块共用的构件（树 / 方块 / 路由）
```

生产文件里只留：

```rust
#[cfg(test)]
#[path = "../../tests/unit/input/router.rs"]
mod router_tests;
```

`#[path]` 让**模块路径不变**（仍是生产模块的子模块），所以 `use super::*`
与对父模块私有项的访问照旧 —— 不必把私有项提成 `pub(crate)`。

### 效果：能只跑改动的那部分

```text
cargo test -p zephyra-core input::router          → 23 条
cargo test -p zephyra-core input::router::touch_tests → 11 条
cargo test -p zephyra-core input::gesture         → 12 条
cargo test -p zephyra-core hit                    → 11 条
cargo test -p zephyra-fx bridge                   → 36 条
```

### 两个搬出来的坑

- **`include_str!` 的相对路径**：测试文件换目录后，`include_str!("icon/assets/…")`
  指到了别处（编译报「找不到文件」）。改成
  `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/icon/assets/…"))`。
- **搬完忘了接上声明**：路由测试搬走后，生产文件里**没有**对应声明 ⇒ 测试
  一条都不跑而**编译全绿**。症状是「`cargo test -p … input::router` 跑出 11 条」
  —— 那 11 条是触控的，路由的**没跑**。这种「静默不跑」比红更坏，所以搬完
  必须**按模块数一遍条数**。

---

## 八、真机复核（gallery，D3D12，release，1400×800，`ZEPHYRA_DEBUG_PORT=7879`）

```text
touch down 0 64 122 → touch up 0 64 122     → 切到「状态」页（轻点生效）
touch down 1 64 200 → touch cancel 1 64 200 → **页面不变**（取消不提交点击）
touch down 0 64 200 → touch up 0 64 200     → 切到「布局」页（正常轻点仍生效）
两指 touch down 0/1 → touch move 1 拉开     → 缩放比 > 1（操作增量生效）
两指各按一个导航项                       → **两个都亮 pressed**
  抬起第一根                             → 只有它自己松开（改前：配对反了）
stats --json                                → "hovered":null（触控不产生悬停）
```
