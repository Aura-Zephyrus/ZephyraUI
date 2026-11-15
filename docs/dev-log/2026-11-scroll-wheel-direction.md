# 滚轮方向与平台相反（物理滚轮向上，看到的却是更靠后的内容）

> 2026-11-15 · Gallery（D3D12 后端，1400×800 物理像素，scale 1.25）
> 排查手段：`zephyra bridge`（调试桥）+ **真 Win32 滚轮消息**（`MOUSEEVENTF_WHEEL`）+ 临时探针
> 起因：上一条（`2026-11-scroll-text-clip.md`）排查滚动容器时顺手撞到的「顺带发现」

## 现象

滚动容器上滚滚轮，**方向与系统其它地方相反**：滚轮**向前**（远离用户，通常是「向上
滚」）时，内容却往**下**走 —— 看到的是更靠后的条目。

这件事不容易被当成 bug：容器「能滚」、步长也对、拖动也跟手，只有**方向**不对。
而方向不对的代价是实打实的：真实应用里往回翻内容只能靠拖。

## 排查

### 1. 先把三处「方向约定」摊开对账

| 层 | 位置 | 约定 |
|---|---|---|
| 平台 | Win32 `WM_MOUSEWHEEL` | 高位正 = 滚轮向前（远离用户）= **向上滚** |
| 绑定 | winit 0.30.13 `event_loop.rs:1730` | `LineDelta(0, HIWORD/120)`，**垂直方向不取反**（取反的是水平那条，`:1751`，注释指向 PR #2105） |
| 契约 | `zephyra-fx/src/lib.rs` `ScrollInput::delta_y` | 「正数向上」 |
| 契约 | `zephyra-fx/src/bridge.rs:777` | 「增量方向与 winit 一致（正 y 向上）」 |
| 工具 | `ZephyraCLI` MCP `scroll` 工具描述 | 「delta_y > 0 means scroll up」 |
| 工具 | `skills/zephyra-mcp/references/tools.md:167` | 「`delta_y > 0` 向上」 |
| **控件** | **`ZephyraUI/src/scroll_view.rs::scroll_wheel`** | **「正数表示向下滚」** ❌ |

⇒ 上游六处一致，**只有控件层反了**。

### 2. 为什么「反了」还看不出

`scroll_offset.y > 0` 的含义是「内容向上移、看到下半段」= **向下滚**；而
`delta_y > 0` 的含义是「向上滚」。两个约定差一个符号 ⇒ 控件层**必须显式取反**，
漏了就整体反向。

### 3. 真机验证（关键：不能只靠读代码）

读代码只能证明「控件层与契约不一致」，不能证明「物理滚轮真的反了」。所以要打真实的
Win32 滚轮消息。

**踩到的坑 1：直接给窗口过程发 `WM_MOUSEWHEEL` 没反应。**
`SendMessage(hwnd, WM_MOUSEWHEEL, ...)` 不报错、返回值正常，偏移却纹丝不动。对照
实验：同一条路发 `WM_MOUSEMOVE`，hover 立刻变了 ⇒ 消息**能**进到 winit ⇒ 排除的是
「注入方式不对」，不是「消息被吞」。

**踩到的坑 2：`mouse_event(MOUSEEVENTF_WHEEL)` 也没反应。**
这次连宿主都没收到事件（见下面的探针）。原因：真实滚轮按「**光标下的窗口**」路由，
而 gallery 窗口当时被别的窗口**盖住了** ⇒ 滚轮被上面那个窗口吃掉。
⇒ 必须先把 gallery **置顶**（`SetWindowPos(HWND_TOPMOST)`）+ 抢前台（ALT 诀窍：先按
一下 ALT，系统才允许本进程 `SetForegroundWindow`），再发滚轮。
⇒ 顺带一条判据：`WindowFromPoint` 与 `GetForegroundWindow` 要**一起**看 —— 只看
「前台是不是我」不够，光标下面是谁才是路由的判据。

**探针**（临时加在宿主与 gallery 上，验证完就撤）：

```text
[probe] host got MouseWheel dx=0 dy=1
[probe] gallery on_input scroll=Some((0.0, 1.0)) hover=Some("scroll:demo")
```

⇒ 物理滚轮向前 ⇒ winit `LineDelta(0, +1)` ⇒ 一路原样传到控件层。

**修复前**：`Scroll.OffsetY` 232 → **280**（+48）⇒ 看到更靠后的内容 ⇒ 方向反。
**修复后**：192 → **144**（−48）⇒ 往回看 ⇒ 方向对。

### 4. 桥命令那一路也是反的（同一个根因）

`zephyra bridge "scroll 596 314 0 2"` 在修复前把偏移**增大**（向下），而桥自己的
契约写的是「正 y 向上」⇒ 调试工具一直在骗人：MCP 的 `scroll` 工具描述
（"delta_y > 0 means scroll up"）与 skills 文档（"`delta_y > 0` 向上"）都写着向上，
实际却是向下。修复后两者与实际一致 ✓。

## 排除的假设

- ❌「winit 在 Windows 上把垂直滚轮取反了」：排除。`winit-0.30.13/src/platform_impl/
  windows/event_loop.rs:1730` 是 `LineDelta(0.0, value)`，`value = HIWORD/WHEEL_DELTA`，
  **没有取反**；取反的是**水平**那条（`:1751`）。
- ❌「宿主把符号弄丢了」：排除。探针显示宿主拿到的就是 `dy=1`，与 winit 一致。
- ❌「gallery 没把滚轮喂给控件」：排除。探针显示 `scroll=Some((0.0, 1.0))` 到了
  `on_input`，且 hover 正是 `scroll:demo`。
- ❌「`Scroll.OffsetY` 读的是旧值 / 写回没落地」：排除。桥的 `scroll` 命令与真机
  滚轮改的是同一个字段；写回**滞后一帧**（读回发生在 `tick()` 的 poll 里，而写回
  发生在随后的帧），所以要在**两次桥调用**之间隔开读 —— 一开始用同一次调用里连着
  `scroll` + `props`，读到的是上一帧的值，差点误判成「滚轮没生效」。
- ❌「Windows 的滚轮方向本来如此」：排除。`WM_MOUSEWHEEL` 高位正 = 滚轮向前 =
  向上滚；浏览器、WinUI 同款。
- ❌「注入方式不对，所以没测出真问题」：**这条排除得最贵**。第一版注入
  （`SendMessage` / `mouse_event`）都没反应，若就此收工就会记成「无法复现」。真
  问题是**窗口被盖住**，不是消息路径。

## 结论 + 门禁

**修法**：`ZephyraUI/src/scroll_view.rs::scroll_wheel` 显式取反
（`offset -= delta_y * SCROLL_WHEEL_DELTA`）；`ScrollBar::wheel` 同款 ——
否则滚动条与滚动容器朝**相反**方向动（两者不共享状态，各滚各的）。

**为什么改控件层而不是宿主**：契约写在 `zephyra_fx::ScrollInput`（正数向上）、桥
（正 y 向上）、MCP 工具描述、skills 文档，四处都这么说；宿主与 winit 都遵守。
控件层是唯一的违规者 ⇒ 改它，一处修，真机滚轮 / 触控板 / 桥 / MCP 四条入口同时对上。

**门禁**：

- `ZephyraUI/tests/unit/scroll_view_tests.rs::a_positive_wheel_delta_scrolls_back_towards_the_top`
  —— 正增量 ⇒ 偏移减小；负增量 ⇒ 偏移增大。
- `ZephyraUI/tests/unit/scroll_view_tests.rs::the_wheel_scrolls_by_a_fixed_step_and_stops_at_the_ends`
  —— 步长与两端钳位（符号改了，断言跟着改）。
- `ZephyraUI/tests/unit/scroll_bar_tests.rs::the_bar_and_the_scroll_view_agree_on_the_wheel_direction`
  —— **跨控件**比对：两个控件不共享状态，各自的单测都绿也可能方向相反。
- `ZephyraUI/tests/unit/scroll_bar_tests.rs` 的两条滚轮用例改成从**中间偏移**出发
  —— 两端都被夹住时方向看不出来。

**实测**（D3D12，1400×800 物理像素，scale 1.25，gallery `--route scroll`）：

```text
修复前（物理滚轮向前一格）：Scroll.OffsetY 232 → 280   ← 方向反
修复后（物理滚轮向前一格）：Scroll.OffsetY 192 → 144   ← 往回看
修复后（物理滚轮向后一格）：Scroll.OffsetY 144 → 192
修复后（桥 scroll 596 314 0 2）：偏移**减小** ⇒ 与桥契约「正 y 向上」一致
```

四道门禁：`cargo fmt --all -- --check` 干净；`cargo clippy --workspace --all-targets
-- -D warnings` 0 警告；`cargo test --workspace` **1540 passed / 0 failed / 73
ignored**；`cargo doc --no-deps --workspace` 退出 0（39 条既有警告，无新增）。

**顺带发现**（本条排查里撞到的，与本 bug 无关，单独记账，见
`docs/todo/scroll-lazy-render-capability.md`）：

1. **滚轮增量单位没统一**：`ScrollInput` 的契约说「单位沿用 winit（物理像素或行
   数）」。Windows 上 winit 永远给 `LineDelta`（行数）⇒ 没问题；但 Linux/X11 的
   平滑滚动会给 `PixelDelta`（像素），而 gallery 一律 ×48 ⇒ **一滚到底**。修法应
   是宿主把 `PixelDelta` 归一化成「格」，或者契约里明确单位并让应用别假定。本机
   没有 Linux 设备，**没有实测** ⇒ 不盲改。
2. **桥的 `hit` 是布局矩形探针**（不算滚动偏移与裁剪），见
   `2026-11-scroll-text-clip.md` 的同款记录。
3. **`gallery/src/bin/main.rs` 没有 D3D11 分支**，见同款记录。
