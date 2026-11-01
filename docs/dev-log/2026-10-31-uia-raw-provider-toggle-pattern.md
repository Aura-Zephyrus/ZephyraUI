# 2026-10-31 UI Automation 原始提供者：树通了，图案取不到

> 现象：屏幕阅读器 / 自动化脚本能读到我们的窗口元素树 —— 名字、角色、AutomationId、
> 禁用态全对 —— 但**唯独 Toggle 图案取不到**。客户端 `GetCurrentPatternAs(Toggle)`
> 报「这个元素没有这个图案」。元素在、名字在、唯独状态读不出来。

## 排查

分两层拆：**连接层**（客户端能不能找到我们）与**图案层**（找到之后能不能拿到能力）。

### 连接层

1. `WM_GETOBJECT` 必须走 `UiaReturnRawElementProvider`，**不能**直接返回裸指针 ——
   客户端在别的进程里，直接给地址它跨不过去，会**静默退回系统默认的 HWND 代理**
   （窗口还在，但它下面的控件一个都不存在，`FindAll` 返回 0 个元素）。
2. `ProviderOptions` 必须报 `ServerSideProvider`。报 0（None）时 UIA 按「客户端侧
   提供者」处理，连接建立失败后同样**静默退回 HWND 代理**。
3. 必须实现 `IRawElementProviderFragmentRoot`。客户端要先解析出片段根才知道树的
   根是谁；`FragmentRoot()` 报「不支持」的话它会**当场放弃整棵树**。

这三条做对之后，元素树就通了（7 个元素、角色 / AutomationId / 禁用态全对）。

### 图案层（本条的主角）

链路是：客户端 `GetCurrentPatternAs(Toggle)` → UIA 服务端 → 我们的
`GetPatternProvider(UIA_TogglePatternId)` → 交出一个实现了图案接口的对象。

服务端 trace 显示 **`GetPatternProvider` 被调用了，而且我们交出了对象**，
但客户端仍然报「没有图案」，而且服务端**从未收到**对图案的任何后续调用
（`ToggleState` 一次都没被调过）—— UIA 拿到指针就丢弃了。

对照实验：用原生 Win32 复选框（`BS_CHECKBOX`）走**同一套**客户端 API，
立刻拿到 Toggle 图案 ⇒ 客户端 API 姿势没问题，问题在我们的提供者。

## 排除的假设

这一段最值钱 —— 下面每一条都花过真时间。

| 假设 | 结论 | 判据 |
|---|---|---|
| 图案接口要**另造一个对象**才能封送回客户端 | **错** | 微软文档 `GetPatternProvider` 的例子就是 `*pRetVal = static_cast(this); AddRef();` —— 图案接口可以**就实现在提供者对象自己身上**；而且「UI Automation calls **QueryInterface** on the returned object as a separate step after this」—— UIA 拿到返回值之后**自己会 QI** |
| 我们的对象**不可封送**（没 `IMarshal` / 不在 ROT） | **错** | windows-implement 的 `agile` **默认就是 true**（`ImplementAttributes::parse` 里 `agile: true`）⇒ QI(IMarshal) 会给出自由线程封送器 |
| `IUIAutomationTogglePattern` 就是 raw provider 要实现的图案接口 | **错** | SDK 的 `UIAutomationCore.h` 里**根本没有** `IUIAutomationTogglePattern` —— 那是**客户端侧**接口（UIA 自己实现）。raw provider 这一侧是 **`IToggleProvider`**（`Toggle()` + `get_ToggleState()`）与 **`IInvokeProvider`**（`Invoke()`）。两者 **IID 不同**，UIA 的服务端 QI 的是前者 ⇒ 实现错 ⇒ **静默**判定「不支持」 |
| 客户端 API 姿势不对 | **排除** | 原生 Win32 复选框对照组通过 |
| 元素属性链不完整（`UIA_NativeWindowHandlePropertyId` / `ProcessId` / `ClassName` 我们报「未实现」） | **排除**（但确实该补） | 补齐后现象**不变**。UIA 确实会问 30020，但补它不解决图案问题 |
| windows-rs 报 `Err` 就是 COM 层失败 | **错** | 那是**客户端包装层**的翻译：`Type::from_abi(空指针)` → `Err(Error::empty())`，而 `Error::empty()` 的码恰好是 **`S_OK`(0)**。COM 层其实是「成功 + NULL」——**要断言 COM 契约必须打 vtable** |
| `GetRuntimeId` 顶层元素该返回自己造的稳定 id | **错** | 文档：顶层元素（挂在窗口里的）返回 **NULL**，UIA 改用宿主提供者的 runtime id |
| 片段根应该能导航到父（返回自己） | **错** | 文档：「Fragment roots do **not** enable navigation to a parent or siblings」—— 往上走是系统默认窗口提供者的活儿 |
| 片段里的子元素 `HostRawElementProvider` 该返回窗口提供者 | **错** | 文档：「In other cases, the property should be **NULL**」—— 片段里只有片段根返回 `UiaHostProviderFromHwnd` |
| `WM_GETOBJECT` 的返回值是指针 | **错** | 文档：「The **key** for the client process to connect to the server process」—— 拿它去读 vtable ⇒ **访问违规**（我们踩过，进程直接崩） |

## 结论

- 原始提供者的图案接口是 **`IToggleProvider` / `IInvokeProvider`**，不是
  `IUIAutomationTogglePattern` / `IUIAutomationInvokePattern`。这是**两套 IID**。
  实现错的后果是**静默**的：元素、名字、角色、AutomationId 全对，唯独图案读不出来。
- 图案接口可以（且应该）**由元素对象自己实现**，`GetPatternProvider` 交出它自己；
  UIA 拿到之后会自己 QI，不需要我们预先 cast。
- 「不支持这个图案」要表达成 **成功 + NULL**（`pRetVal` 收到 NULL），不是错误码。
- 「那个方向没有元素」同理：`Navigate` / `ElementProviderFromPoint` / `GetFocus`
  都要 **S_OK + NULL**，报 E_FAIL 会被 UIA 当成「出错了」，遍历当场断掉。
- 窗口销毁时要调 `UiaReturnRawElementProvider(hwnd, 0, 0, NULL)` —— 文档要求，
  否则 UIA 的「提供者已抛事件」映射表一直攥着我们的提供者，引用计数降不下来。
- 诊断侧的教训：**Rust 侧拿到「COM 成功 + 出参 NULL」时，windows-rs 会翻译成
  `Err(code = S_OK)`**。判据要落在 COM 层，就打 vtable，别信包装层。

## 门禁

- `zephyra-uia` 的单测（33 条），其中三条是**端到端**的：
  - `a_uia_client_can_read_our_tree_from_the_window_handle` —— 真 `IUIAutomation`
    客户端走 `ElementFromHandle` → 读到名字 / 角色 / AutomationId / Toggle 状态；
  - `a_native_checkbox_gives_its_toggle_pattern_to_a_uia_client` —— 原生 Win32
    复选框对照组（分水岭：出问题的是我们还是客户端 API）；
  - `wm_getobject_hands_out_the_root_provider` —— 只断言「非零 key」，**不 deref**。
- `cargo run -p zephyra-uia --example dump_tree -- "ZephyraUI (D3D12)"` —— 真机
  跨进程 dump：组头 `Indeterminate`、`check:alpha` = `On`、其余 `Off`、
  `check:locked` = `Off` 且 `enabled=false`、按钮有 Invoke 图案。

## 仍然没做

- 事件（`UiaRaiseAutomationEvent` / 属性变化事件）还没接 —— 屏读器只能**主动轮询**，
  不会在勾选变化时立刻念出来。
- `IUIAutomationLegacyIAccessiblePattern` 没实现（老式 MSAA 客户端会看不到）。
