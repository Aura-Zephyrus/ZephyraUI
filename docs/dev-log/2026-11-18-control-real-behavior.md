# 控件的功能补全：从「状态演示」到「真能用」

> 2026-11-18 · `ZephyraUI` / `gallery` / `zephyra-fx` / 三宿主
> 起因：用户指出控件「所对应的功能也要实现」—— 不少控件只有状态翻转，控件本身没反应。

## 现象

审计 catalog / catalog2 两页 19 个控件，空壳/半空壳的有：ProgressRing（静态）、Avatar（静态，
本就是展示型）、ToolTip（只有旁边按钮变色）、SettingItem（同上）、Hyperlink（只标 visited，
不打开 URL）、ColorPicker（无颜色预览）、PageIndicator（点本身无效）。

## 实现

按 RinUI 语义补真功能：

- **ColorPicker**：预览块复用 `.zstyle` 的 `sw-0..15` 类（颜色留在样式表，源码不写外观），
  选中色号 → 预览块换类 → 真颜色。MCP 读回：点 `swatch:0` 后预览变为 `sw-0` / `#F44A67FF`。
- **PageIndicator**：pip 改成 `interactive` 控件（`pip:N`），点第 N 个切到第 N 页；
  点当前页再点一次前进一页（避免空操作，也让门禁的树指纹看到变化）。
- **ToolTip**：模板加 `visible` 参数驱动 `collapsed`；绑定 `hover('c2:tip-target') || pinned`，
  悬停即显、点目标按钮固定显 —— 真 hover 行为，而非旁边按钮变色。
- **SettingItem**：模板加 `clickable` 参数，`interactive` 与 `.clickable` 类都跟着走；
  点击真的切换可点态并改描边。
- **Hyperlink**：`View` 新增 `take_pending_url()` 默认空实现；`Gallery::click` 只记 URL 意图
  （**不碰系统 API**，所以门禁/离线路径不会误开浏览器）；三宿主每帧 drain 后用
  `rundll32.exe url.dll,FileProtocolHandler` 打开系统默认浏览器。失败静默。
- **ProgressRing**：模板根可点，点击轮换 running/paused/error 三态（描边色真的变）+ 推进进度。

## 排除的假设

❌「在 `Gallery::click` 里直接 `Command::new` 开浏览器」—— 门禁会反复 `click` 同一 key，
   那样会弹出一堆窗口；必须走「View 发意图、宿主 drain」的通道。
❌「用 `Val::Raw(PropertyValue::color(...))` 给预览块上色」—— `the_whole_tree_writes_no_appearance_in_source`
   拦 Local 层 BACKGROUND；改用样式表已有的 `sw-N` 类，外观零源码。

## 踩坑

- zypui 模板改动必须跑 `cargo run -p zypuic -- fmt <file.zypui>`，否则 `zypuic` 的格式门禁
  （`fmt_tests` / `style_gate_tests`）失败。
- 门禁 `every_interactive_key_is_wired_or_whitelisted` 只认 tag/key/class/text 指纹：
  纯颜色变化不算，所以 pip/SettingItem 的点击都配了类变化。

## 门禁

- MCP 读回：ColorPicker 预览随选中变色；ToolTip 悬停显隐；PageIndicator pip 可点切换。
- `cargo test --workspace`：**1736 passed / 0 failed**；fmt 干净、clippy 0 警告、doc 退出 0。
