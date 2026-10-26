# 2026-10-26-frameless-window

> 现象：应用需要在保留现有窗口行为的前提下选择无标题栏窗口模式。

## 排查

- 三个宿主都在 winit `Window::default_attributes()` 上配置系统窗口装饰：D3D11、D3D12、Vulkan。
- 标题栏属于窗口创建属性，不属于 DisplayList、样式系统或 GPU 呈现路径。
- 配置应随消费 ZephyraUI 的应用项目存放，而不是放在框架仓库或要求每次启动附带参数。

## 排除的假设

- 不需要新增 DWM API、窗口句柄操作或后端渲染分支；winit 已提供跨平台 decorations 属性。
- 隐藏系统装饰不等同于实现自绘标题栏；不应隐式承诺拖动、最大化或缩放控件。
- 该开关不需要调整客户区尺寸、布局坐标或交换链策略。

## 结论

三个宿主从进程当前工作目录读取 `ZephyraUI.json` 中的 `window.frameless`。文件缺失或未设置该字段时沿用系统默认装饰；配置无效时报告错误并使用默认窗口配置。配置文件属于消费 ZephyraUI 的应用项目根目录，不从框架仓库或 crate 源码目录读取。应用的自绘标题栏与窗口操作交互不在此项范围内。

## 门禁

- `cargo fmt --all -- --check`
- `cargo test -p zephyra-app-config`
- 针对 D3D11、D3D12、Vulkan 宿主执行平台可用的 `cargo check`。
- Windows Gallery 项目目录的 `ZephyraUI.json` 启用 frameless；从 `gallery/` 工作目录启动 D3D12 Gallery，启动日志读取到 frameless，且 Win32 客户区顶边与窗口顶边差 `0 px`。
