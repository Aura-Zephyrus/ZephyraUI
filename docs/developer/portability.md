# 可移植性 —— 哪些是平台能力，Linux 上各自怎么接

> 这份文档是**接口契约**，不是 TODO 清单。每一项都写清了：
> 当前状态、Linux 上对应的权威来源、以及「上层不��变」这件事如何被保证。

## 1. 抽象层的形状

```
zephyra-core / zephyra-fx      零 Win32 代码（这一层**任何平台都完全相同**）
        │
zephyra-ui / zephyra-style     零 Win32 代码；只问 zephyra-platform 的能力位
        │
zephyra-platform               trait Platform + Capabilities + stub
        ├── zephyra-platform-win     Windows 实现（DWM / 版本 / 注册表）
        └── zephyra-platform-linux   Linux 实现（gsettings / env / os-release）

zephyra-fx                      DisplayList → 顶点流 / 字形图集 / 纹理表（后端无关）
        ├── zephyra-winit          D3D11 后端（Windows）
        ├── zephyra-winit-d3d12    D3D12 后端（Windows）
        └── zephyra-winit-vk       wgpu 后端（Linux；Vulkan 优先，GL 亦可）
```

`zephyra-platform` 是 `#![forbid(unsafe_code)]` 且**没有 `cfg(windows)`**——
它本身就是「平台无关」的证据。Windows 代码只在实现 crate 里。

## 2. 能力位与 Linux 对应（`Platform::capabilities`）

| 能力位 | Windows 上 | Linux 上怎么办 | 权威来源 |
|---|---|---|---|
| `dark_mode` | `DWMWA_USE_IMMERSIVE_DARK_MODE`（22000+） | **不适用**（无标题栏 DWM 属性）→ `false` | — |
| `corner_preference` | `DWMWA_WINDOW_CORNER_PREFERENCE` | Wayland 下由合成器决定（通常圆角）；X11 下是 WM 私有约定 → `false` | wlr-protocols / XDG |
| `caption_color` / `border_color` | `DWMWA_CAPTION_COLOR` / `_BORDER_COLOR` | 同上 → `false` | — |
| `system_theme_query` | 注册表 `AppsUseLightTheme`（**未文档化**） | **有规范**，且已实现：`gsettings get org.gnome.desktop.interface color-scheme`，退化到 `GTK_THEME`（`-dark` 后缀），再退化到「读不到」 | freedesktop portal spec |
| `system_language_query` | `GetUserPreferredUILanguages` | **已实现**：`LANGUAGE`（gettext 约定，**本来就是有序表**），退化到 `LC_ALL` → `LC_MESSAGES` → `LANG` | gettext |

**关键**：Linux 的深浅色来源**有正式规范**，比 Windows 那两个注册表值可靠。
所以 Linux 适配时**不需要重新调研**，只需实现 `system_theme()`。

## 3. 上层为什么不用改代码

- 主题跟随：`Gallery` 只读 `Platform::system_theme()`；读不到（`None`）⇒
  **不跟随**（而不是跟随成浅色）。这一点已经由
  `系统主题查询结果必须与注册表一致或读不到` 这条门禁钉住。
- 窗口背后透出来：走**窗口透明模式**（`window.transparent` /
  `WindowOpts::transparent`，逐像素 alpha 进合成器），不再走系统材质。
  Windows 走 DComp premultiplied composition swapchain，Vulkan 走
  `PRE_MULTIPLIED` 探测；Linux 上 winit `with_transparent` 即可，见
  `docs/developer/win/07-window-transparent.md`。

## 4. 尚未做到的部分（**诚实边界**，2026-10 更新）

| 部分 | 现状 | Linux 上的真实情况 |
|---|---|---|
| 渲染后端 | 三个后端互不依赖：`zephyra-winit`（D3D11）/ `zephyra-winit-d3d12`（D3D12）是 `#![cfg(windows)]`；`zephyra-winit-vk`（wgpu）是 `#![cfg(any(target_os = "linux", target_os = "android"))]` | **已通**：demo 与 gallery 都能在 WSL2 上开窗出画面（见 §5） |
| 14 档 PS 变体混合模式 | D3D11 / D3D12 两边都有；`zephyra-winit-vk` 的 WGSL 里也写好了入口 | **未接线**：wgpu 下「正在渲染的附件」不能被采样（MSAA 更是根本不支持采样），接通需要「resolve 到中间纹理 → 拷贝 → 第二个通道」。现状是照常画、退回 `SrcOver`，并计入 `RenderReport::unimplemented_blends` |
| 滤镜（blur / 投影） | 三个后端都实现了「内容 → 模糊（乒乓）→ 合成」 | **已通**。实测 demo 3 条带滤镜命令全部走离屏，`unimplemented_filters = 0` |
| `zephyra-dcomp` | DirectComposition | Linux 无对应物，**保持 Windows-only**。呈现由 `wgpu::Surface` 负责（X11 走 XCB / GLX，Wayland 走 `wl_surface`） |
| 逐窗口深色 / 圆角 | DWM 属性 | `false`。逐项理由见 `zephyra-platform-linux` 的模块文档与 `stub.rs`（系统背景材质已随 DWM backdrop 移除） |
| 设备枚举（摄像头等） | 摄像头已独立成仓 `E:\zephyra-camera`（https://github.com/Aura-Zephyrus/zephyra-camera.git）：像素格式处理是零设备依赖，采集走 nokhwa | 未在 Linux 上实测（`--features capture`） |
| 跨帧局部重绘 | 三个后端都做不到 | 同样的理由：flip model / swapchain 下 present 之后后缓冲内容未定义（见 base README） |

## 5. 验收口径

### 5.1 「能编」（不需要显示器）

```bash
cargo clippy --workspace --all-targets -- -D warnings      # 默认 feature；Linux 上 d3d12 是空 crate
cargo clippy -p zephyra-gallery -p zephyra-ui --all-targets \
      --no-default-features --features zephyra-gallery/linux-vk \
      -- -D warnings                                      # 真的把 Linux 后端编一遍
cargo test --workspace
```

⚠ 第一条在 Linux 上**编的是「没有宿主」那一态**：`default = ["d3d12"]`，而 d3d12
crate 在 Linux 上是空的（`#![cfg(windows)]`）。所以判据必须是「平台 **且** feature」
而不是只看 feature —— 详见 `gallery/src/bin/main.rs` 顶部那段注释。

⚠ `zephyra-ui` **没有** `linux-vk` feature（控件层零后端依赖）——
`--features zephyra-ui/linux-vk` 会直接报「the package 'zephyra-ui' does not
contain this feature」。

### 5.2 「能跑」（需要显示环境）

```bash
cargo run -p zephyra-gallery --bin gallery --no-default-features --features linux-vk
ZEPHYRA_FRAME_DUMP=/tmp/f.bmp cargo run -p zephyra-gallery --bin gallery \
      --no-default-features --features linux-vk     # 把第一帧写成 BMP
```

`ZEPHYRA_FRAME_DUMP` 是三个后端共用的诊断开关。Linux 侧实测：gallery
1400×800、1177 个四边形、颜色数 949 —— 与 `--dump-cpu`（CPU 参考光栅器）给出的
同一张图一致。**「与 CPU 参考图一致」才是这里的判据**；「没报错」不算
（空画面同样不报错，这一点在本仓被反复验证过）。