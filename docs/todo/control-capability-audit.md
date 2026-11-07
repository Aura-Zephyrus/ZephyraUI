# ZephyraUI 控件能力缺口与重写清单

> 盘点范围：`ZephyraUI` 控件库当前公开组件、声明式模板与 Gallery 的消费方式。
> （本文件初版还盘点了同名 Rust 构造器，那一批已于 2026-10-28 删除，见下文
> 「结构整理风险」。）
> 目的：为后续重写标出**组件名称暗示的行为尚未实现或明确降级**之处；本文件不表示这些控件已删除，也不把所有声明式控件都判为无效。

## 结论先读

ZephyraUI 是声明式控件描述层，不是带事件循环、焦点管理、输入法、动画时钟或弹层管理的原生控件工具箱。大部分组件只根据调用方提供的状态生成 `Element` 树；应用仍需在 `View::on_input` 中处理输入、更新状态，并在 `view` 中重新提供状态。

以下组件能显示一个外观或静态状态，但公开名称/视觉形态容易让调用者期待其尚未提供的行为，应作为后续重写候选：

- **输入与编辑**：`TextField`、`TextArea`、`ScrollableTextArea`、`AutoSuggestBox`、`TextInputMenu`、`SpinBox`
- **浮层与模态**：`Dialog`、`Menu`、`MenuBar`、`DropDownButton`、`ComboBox`、`ToolTip`
- **滚动与手势**：`ListView`、`SwipeView`、`DatePicker`、`TimePicker`、`TableView`
- **视觉语义降级**：`AcrylicCard`、`RoundedCornerOverlay`、`ProgressRing`、`BusyIndicator`、`Toast`
- **部分能力的控件**：`ColorPicker`

这些条目不是同一种问题：有些只能画静态外观，有些可通过点击与应用状态配合工作，但缺少名称通常暗示的滚动、浮层、系统输入或动画能力。请以各项的“当前行为”列为准。

> **2026-11-01 起，gallery 不再演示本节列出的这些控件。**
> 需求是「把不成熟组件踢掉，只保留 Button 和复选框」——
> **被砍的是 gallery 的演示页，不是控件库**：`ZephyraUI` 的 54 个组件
> **一个没删**。本文件仍是这些控件能力缺口的**唯一台账**，
> 用来决定「什么时候能把它们放回 gallery 演示」。
>
> 动机很直接：gallery 里演示一个只有外观的控件，画面上看着像能用，
> 点下去才知道是空壳 —— 这比「没有这个控件」更容易误导人。
>
> 被砍的演示页：`collections` / `menus` / `dialogs` / `paging` /
> `navigation` / `status` / `layout-family` / `settings`，
> 以及 basics / toggles 两页里非 Button、非 CheckBox 的段落。
> 参考页现在只剩两族：`buttons` 与 `checkbox`。
>
> 详见 [2026-11-01-gallery-trim](../dev-log/2026-11-01-gallery-trim.md)。

## 能力缺口

| 组件 | 当前行为 | 未实现 / 降级能力 | 建议重写目标 |
|---|---|---|---|
| `TextField` | 显示调用方提供的值或 placeholder；`focused` 用框架焦点系统（Tab/点击设焦点）；清除按钮只提供可命中的 key | 无可编辑缓冲、键盘焦点、光标、选区、键入与 IME；宿主必须自行处理输入并回传新值 | 建立可接入宿主输入事件的编辑状态、焦点与 IME 契约；明确受控/非受控模型 |
| `TextArea` | 将调用方提供的字符串切成静态文本行 | 无编辑、光标、选区、换行输入或键盘焦点 | 与 `TextField` 共用编辑模型，再提供多行布局 |
| `ScrollableTextArea` | 显示文本区和按 `ratio` 计算高度的滚动 thumb | thumb 仅是视觉件；内容不会滚动，滚轮不会更新偏移 | 接入滚动偏移、视口裁剪、滚轮/触控滚动和真实 thumb 映射 |
| `AutoSuggestBox` | 输入框下方以内联子节点显示建议列表；`open`、选中项和文字由调用方提供 | 不是浮层；没有锚定定位、键盘导航或内建输入查询 | 接入可编辑输入状态、键盘候选选择和窗口/视口约束下的弹层定位 |
| `TextInputMenu` | 生成一张内联菜单卡 | 不是右键上下文浮层；不读取文本选区或光标位置 | 接入文本选区、上下文请求及宿主浮层定位 |
| `SpinBox` | 显示受控数值与加减按钮 | 数值键入、选区、IME 不支持；数值变化由宿主按按钮 key 更新 | 建立受控数值编辑、范围/步长/无效输入策略和键盘可访问行为 |
| `ComboBox` | 显示受控当前值与箭头 | 不展开选项浮层；选择逻辑由宿主实现（现有 Rust 注释描述为点击循环切换） | 提供可定位的选项弹层、导航/选择/关闭语义，或改名为明确的循环选择控件 |
| `DropDownButton` | 按 `open` 显示按钮外观 | 不创建或定位下拉内容；打开后的内容由宿主另行摆放 | 明确其只是 trigger，或统一接入 Popup/Overlay 管理 |
| `Menu` / `MenuBar` | 生成菜单项或菜单栏结构 | 菜单卡是内联树；MenuBar 不负责创建、定位或关闭弹出菜单；互斥、子菜单状态由宿主维护 | 接入菜单弹层、焦点/键盘导航、关闭规则与宿主命令回调 |
| `Dialog` | 生成标题、正文和按钮行 | 没有模态遮罩、焦点圈闭、焦点恢复或系统级弹层生命周期；按钮行为由宿主处理 | 接入 Overlay/Modal 服务及焦点管理；否则改用非模态 `DialogCard` 一类名称 |
| `ToolTip` | 生成提示卡片和文本 | 不与目标控件锚定；不监听 hover/focus，不负责延迟、避边或隐藏 | 接入 anchor、hover/focus 生命周期与弹层定位，或明确改名为静态提示卡 |
| `AcrylicCard` | 透明/半透明背景卡片 | 没有桌面背景模糊与噪点，不是系统 Acrylic 材质 | 改名表达普通半透明卡，或接入窗口透明模式下与合成器配合的模糊能力 |
| `RoundedCornerOverlay` | 生成带圆角分类的容器/边框 | 不执行子树裁剪；圆角外溢内容不会因此自动被 mask | 接入真正的 clip/mask 能力，或改为不承诺裁剪的圆角容器 |
| `ListView` | 一次生成传入的全部行，并以 key 暴露行选择 | 无滚动容器、可视区域、虚拟化或按需创建 | 接入滚动视口与大列表虚拟化；小型静态列表可另设不同名称 |
| `SwipeView` | 显示当前页和可点击的页指示点；页面索引由宿主提供 | 无触控/鼠标滑动手势和动画；点击指示点后的翻页仍由宿主更新状态 | 接入手势识别、边界行为和切页动画，或改名为静态分页面板 |
| `DatePicker` | 按调用方给定的星期表头与日期格显示一个月；日期格可由宿主按 key 处理 | 不计算年月日历、不切换月份、不滚动或键盘选择；可见月份/日期数据全由调用方给 | 接入日期模型、月份导航、日历计算、约束和键盘交互 |
| `TimePicker` | 显示调用方提供的小时/分钟可见窗口并高亮指定项 | 可见窗口由宿主滚动后重新提供；控件本身不滚动、不操作时间值 | 接入滚动/循环选择、键盘行为和完整时间值模型 |
| `TableView` | 按传入数据绘制固定结构的表头和单元格，单元格可由宿主按 key 命中 | 无列宽拖动、排序、表头行为、滚动或虚拟化 | 明确静态表格与完整数据表两种能力；完整表格接入列模型、滚动与排序 |
| `ColorPicker` | 显示固定的 16 格样式分类色板及调用方给出的选中索引 | 没有 HSV/RGB 编辑、取色器、滑块或可配置颜色值模型 | 接入可配置 palette 与颜色值读写、键盘选择及自定义色编辑 |
| `ProgressRing` | 显示圆环和可选中央文字；`state` 改变环色 | 没有弧线或真实进度角度；数值文字不等于圆弧进度 | 增加弧形图元/渲染能力后实现 determinate ring；否则改名为状态环 |
| `BusyIndicator` | 根据调用方传入的 `frame` 在三档静态大小间切换 | 不自行计时或动画；不随时间变化，宿主必须驱动帧状态 | 接入统一动画时钟/动画系统，或明确作为静态动画帧部件 |
| `Toast` | 生成一张带级别颜色的提示卡 | 不自动出现、消失、计时、队列管理或动画；显示周期由宿主维护 | 接入通知队列/生命周期/计时与动画；否则改名为静态通知卡 |

### 依赖应用状态，但不属于上述缺陷

以下组件也不会自行切换业务状态，但这是控件层的声明式边界，不应单独据此判为无效：

- `ToolButton`、`PillButton`、`RoundButton`、`Hyperlink`
- `RadioButton` / `ToggleSwitch`（2026-11-02 已补齐状态模型、键盘与无障碍；2026-11-06 起 ToggleSwitch 支持 `material` 属性：`solid`（默认，位移 250 ms「快入」tween + 按压瞬变，轨道色由 `.zstyle` 静态双类定义）/ `glass`（液态玻璃，维持复刻），轨道与把手都支持，见 dev-log 2026-11-03-liquid-toggle-1to1.md 续五/六）
- `Segmented`、`NavItem`、`NavSubItem`、`SelectorBar`
- `CheckBox` / `CheckBoxGroup`（2026-11-01 已补齐三态与键盘，见下）
- `Slider`、`Expander`、`SettingExpander`、`InfoBar` 的关闭项
- `MenuItem`、`MenuBarItem`、`ListView` 行、`SwipeView` 页点、`DatePicker` 日期格

这些组件提供 `key` / `INTERACTIVE` / 状态样式所需的结构；点击、拖动、键盘或关闭后要做什么，以及新状态何时回到 `view()`，属于使用它们的应用。**但 `Slider` 目前只有传值与绘制 fill 的结构，不会把拖动位置反算成值**，因此与普通受控按钮不同，仍在上方列为需要补足的交互能力。

**`RadioButton` / `ToggleSwitch` 已补齐状态模型、键盘与无障碍**（2026-11-02）：
两者的状态**值**与流转**规则**住在控件层 `ZephyraUI/src/toggle.rs`
（`ToggleSwitch` 两态、`RadioGroup` / `RadioOption` 单选互斥）。开关**没有**半选
——半选是「组里选了一部分」的派生态，开关没有组。
文案走双通道（`label_key` → 控件内部 `tr()`；`label` → 动态文本），
两个组件都补了 `focusable` / `activatable` / `disabled` 与焦点环、禁用态规则；
开关补了 `key="label"` 文本子节点（无障碍名的来源），状态类 `on` 落在
**控件自己**身上（只落在子节点 `switch-track` 上时屏读器会一直念「关」）。
UIA 侧：`switch` → `Role::Switch`（ControlType 复用 CheckBox —— UIA 没有
Switch 类型），仍走 TogglePattern；`radio-box` → `Role::RadioButton` +
`ISelectionItemProvider`（`Select` / `IsSelected`，`SelectionContainer` 报父分组），
状态变化事件改报 `SelectionItemIsSelected` 而不是 `ToggleToggleState`。
参考页新增第三族「Switches」；门禁 `the_switch_family_is_exposed_with_the_right_roles`
钉住角色、名字与「恰好一项选中」。

**`CheckBox` / `CheckBoxGroup` 已补齐三态、禁用与键盘**（2026-11-01）：
`CheckBox` 从「只画一个勾」升级为真三态控件（`Unchecked` / `Checked` /
`Indeterminate`），`tristate` 是 `.zypui` 里的**结构开关**（三态框才带半选横杠
那个节点），状态**值**与流转**规则**住在控件层 `ZephyraUI/src/checkbox.rs`
（`CheckState` / `Checkbox` / `CheckboxGroup`），状态**存在哪**仍是宿主的事。
新增 `CheckBoxGroup`：一个三态组头（= 全选 / 全部不选）+ 一层两态子项，
组头状态**由子项聚合派生**（不另存一份真相），禁用子项既不被「全选」捎带改、
也不参与聚合（否则组头会卡在半选、点了永远没反应）。
键盘：Space 与 Enter 都切换，复用 core 的 `ActivationState`（应用零改动）；
焦点环画在**指示器**上而不是整颗控件。UIA 侧给出 `IToggleProvider` 图案，
On / Off / Indeterminate 跨进程可读（见
[2026-11-01-checkbox-tristate](../dev-log/2026-11-01-checkbox-tristate.md)）。

**`Button` 已从「纯受控声明」升级为有原生交互行为的控件**（2026-10-30）：
声明 `focusable` / `activatable` 进 Tab 序，焦点环走 `focused` 类；Space / Enter 经
core `ActivationState` 复用 pressed 通道触发点击，应用零改动。它是第一个消费
「键盘/IME 契约 + 焦点状态机」地基的控件，也是后续重写（TextField 编辑态、
Segmented 键盘激活等）的打样（`CheckBox` 已于 2026-11-01 照同一模式补齐）。`ToolButton` / `PillButton` 等其余按钮族
尚未接入，下一步可照同一模式补。

**`Button` 支持可插拔材质**（2026-10-30）：`.zstyle` 用 `material:` 声明纯色 /
磨砂玻璃（`frosted`，背后高斯模糊）/ 液态玻璃（`glass`，背后折射），未声明默认
纯色。材质**种类**与**参数**分开：`material-params:` 独立调 σ / 厚度 / 倒角 /
色散 / 折射率（不写用默认）。材质是独立属性（与 `filter` 正交），展开成底层滤镜
链后交给渲染，三后端不感知 `material`。磨砂玻璃的「背景模糊」是本轮**新增**的
`Filter::BackdropBlur`（三后端 shader 手抄同步）。用法
`<Button material="glass"/>` + `.glass { material-params: 26 18 0 1.5; }`。详见
[2026-10-30-button-materials](../dev-log/2026-10-30-button-materials.md)。

## 结构整理风险

~~当前同一控件结构存在两种来源。~~ **已于 2026-10-28 收口**（两步提交
`b95b46e` + 本轮）：

1. ✅ `src/controls/*.rs` 的 59 个 Rust 构造器**已删**（全仓除 parity 测试外零消费者）。
2. ✅ `ui/controls.zypui` 是**唯一**结构真相，由应用的 `build.rs` 直接读文件编译
   （`gallery/build.rs:15`），crate 不再有同名 Rust API。
3. ✅ parity 测试与 `ui/parity.zypui` 一并删除；逐控件对照换成 gallery 的**整树**
   门禁 + zypuic 编译期报错。判据从「与脚手架全等」变成「gallery 真用的那棵树是对的」。

详见 [2026-10-28-zephyra-ui-single-source](../dev-log/2026-10-28-zephyra-ui-single-source.md)。

## 后续重写顺序建议

1. **先补基础能力**：输入/IME/焦点与 Overlay/Popup 管理。它们决定文本框、菜单、对话框、建议列表等控件能否有真实行为。
   - ✅ **键盘 / 字符 / IME 事件契约已完成**（2026-10-28）：core 定
     `TextEvent`（`Key` / `Char` / `Ime` 三通道），三宿主共用
     `zephyra-winit-input` 翻译。
   - ✅ **焦点状态机已完成并接入**（2026-10-28 状态机；2026-10-30 三宿主接线）：
     core 新建 `input::focus`（`FocusState`），props 加 `FOCUSABLE` / `FOCUSED`，
     Tab 按深度优先移动并圈闭；三宿主已持有并驱动它，控件层以 `focused` 内置词消费。
   - ✅ **键盘激活已完成**（2026-10-30）：core 新建 `input::activation`
     （`ActivationState`），props 加 `ACTIVATABLE`；Space / Enter 复用 pressed 通道
     触发点击。`Button` 是第一个消费它的控件（见「依赖应用状态」一节）。
   - ⬜ Overlay / Popup 管理仍是空白（`Layer::Popup` 零生产者）。
2. **再补滚动与裁剪**：Scroll、Clip/Mask 与手势。随后重写列表、文本区、日期/时间选择器和 SwipeView。
3. **最后补专用呈现**：弧线/进度环、动画时钟、系统 Acrylic 或明确的静态降级命名。
4. 每个控件确定受控状态 API、键盘/指针输入语义、禁用/边界行为、无后端逻辑测试与跨后端实机验收，再恢复到“已实现”清单。

## 对照源码

- [控件 README](../../ZephyraUI/README.md)
- [声明式控件模板（唯一结构真相）](../../ZephyraUI/ui/controls.zypui)
- [控件样式表](../../ZephyraUI/controls-base.zstyle)
- [控件如何被应用编译消费](../../gallery/build.rs)
