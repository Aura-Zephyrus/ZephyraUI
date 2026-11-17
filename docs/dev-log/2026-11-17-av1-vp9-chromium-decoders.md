# AV1 / VP9：照 Chromium 的 DEPS pin 搬 dav1d 与 libvpx

> 2026-11-17 · `zephyra-base/extra/zephyra-video`（新 `av1` / `vp9` feature）
> 排查手段：对照 `E:\chromium\third_party\{dav1d,libvpx}` 的 BUILD.gn / README.chromium / DEPS；MSVC `cl.exe` 复现 C 编译错误；逐 feature 跑 clippy 与 174 条单测

## 现象

用户要求「支持 AV1 和 VP9」。延续上一条的「和 Chrome 一样」：Chromium 的 AV1 用
**dav1d**（BSD-2），VP9 用 **libvpx**（BSD-3）。`E:\chromium\third_party\` 里只有
构建文件与配置（`libdav1d/` 是空目录、DEPS 未同步），所以从上游按 Chromium `DEPS`
里 pin 的 commit 下载源码，连配置与源码清单一起原样搬进 `native/`。

## 排查

### 1. 源码从哪来

`E:\chromium\DEPS` 给出精确 commit：
dav1d `99f3354ed0b9b4bf3c7c9347318dbcc522c94af5`、
libvpx `0a6f769e44989222d99ded46daa1e9763637cc19`。Chromium 的 git mirror 超时，
改用上游主机（code.videolan.org / codeload.github.com）按同一 commit 下载。

vendored 布局照 Chromium：`native/dav1d/{libdav1d,config,version,dav1d_generated.gni}`、
`native/libvpx/<源码目录>,config,libvpx_srcs.gni,LICENSE,PATENTS`。
`build.rs` 解析 Chromium 生成的 `.gni` 源文件清单来编译 —— 不再自己维护一份会漂移的表。

### 2. MSVC 的两个编译开关

- 缺头文件：`cc-rs` 没传 MSVC / SDK 目录（与上一条同款），仍用 `msvc_system_includes()` 以 `-I` 补。
- MSVC 14.5x 把 C11 atomics 放进 `<stdatomic.h>`，必须同时给 `/std:c11` 与
  `/experimental:c11atomics`，否则 `#error: C atomic support is not enabled`。

### 3. dav1d：模板源码要编两遍

Chromium 把 `template_sources` 分别以 `BITDEPTH=8` / `16` 编成两个静态库，函数名
带 `_8bpc` / `_16bpc` 后缀。最初用两次 `cc::Build::compile()`，在本机报
`Could not copy or create a hard-link to the generated lib file`。
⇒ 改成在 OUT_DIR 生成 wrapper `.c`（`#define BITDEPTH n` + `#include "绝对路径"`），
**同一个** build 一次编完。两个坑：
① `gni_sources` 行首没锚定时 `template_sources` 会命中 `x86_template_sources`（空列表）；
② `canonicalize()` 在 Windows 返回 `\\?\` 前缀，`cl.exe` 的 `#include` 认不了，改用 cwd 拼接。

另外 `dav1d_data_wrap()` 会校验 free callback 非空 ⇒ shim 传一个 no-op 回调。
`build.rs` 只 `rerun-if-changed=native/dav1d`，改 shim 不会触发重编 ⇒ 补上 shim 的路径。

### 4. libvpx：不能用 Chromium 的 win/x64 rtcd 头

`config/win/x64/vp9_rtcd.h` **无条件**把函数指针映射到 `*.asm` 里的 `*_sse2` 符号，
即使 `HAVE_X86_ASM=0`。本机没有 nasm/yasm ⇒ 改用 `config/linux/generic/*_rtcd.h`
（纯 C 映射）放进 `config/noasm_rtcd/`，并在 include 顺序里排最前；
`vpx_config.h` 仍取平台版（Windows 兼容）。vendored 配置里所有 `HAVE_{MMX,SSE,...}`
统一改成 0，这样 `system_state.h` 把 `vpx_clear_system_state()` 定义成空宏，
不再需要 `emms_mmx.c`（它依赖 x64 不支持的 `_mm_empty`）。

`gni_sources` 还遇到 `libvpx_srcs_x86_64_mmx =\n    [ ... ]` 这种换行写法 ⇒
改成先找行首 `name =`、再找随后的 `[`。

### 5. FFI 边界：C shim 摊平大结构

`Dav1dPicture` 与 `vpx_codec_ctx_t` / `vpx_image_t` 都很大且随版本变化，
Rust 侧不去镜像它们：`native/{dav1d,libvpx}_shim.c` 把输出帧摊平成标量参数
（宽高 / 位深 / 平面指针 / stride），Rust 回调逐行拷进紧排的 I420 `VideoFrame`。
vendored 源码一个字节没改（除配置里的汇编开关与许可证要求的副本）。

## 排除的假设

❌「Chromium third_party 里就有完整源码」—— 只有 BUILD.gn / config / 清单，
`libdav1d/` 是空的（DEPS 未同步）⇒ 必须按 DEPS 的 commit 从上游取。

❌「用 win/x64 的 rtcd 头 + HAVE_X86_ASM=0 就行」—— rtcd 头无条件引用 asm 符号，
没有 nasm/yasm 必然链接失败 ⇒ 换 generic 的纯 C rtcd 头。

❌「多 compile() 只是写法问题」—— 本机 cc-rs 在同 OUT_DIR 多次 compile 会 hard-link 失败 ⇒
wrapper 方案一次编完。

## 结论

- 新 feature `av1 = ["dep:cc"]` / `vp9 = ["dep:cc"]`，`cc` 仍可选（build script 收得到 feature cfg）；
  两个后端与 `uavs3d` / `dxva2` 可同时开。
- `VideoCodec::AV1` / `VP9` 接 `Dav1dDecoder` / `Vpx9Decoder`，输出 8bit I420。
- vendored 目录保留各自许可证：dav1d BSD-2（`COPYING`）、libvpx BSD-3（`LICENSE` + `PATENTS`）。
- 编译关闭全部 SIMD 汇编（无 nasm/yasm），走纯 C 回退 —— 能解码，但不是最快路径。
- 本机没有 .obv / .ivf 样片，FFI 烟测只覆盖创建 / 拒绝错误 codec / 垃圾码流报错。

## 门禁

- `cargo test -p zephyra-video --offline`：默认 168、av1 171、vp9 171、av1+vp9 174、dxva2+av1+vp9 174，全 0 失败。
- `cargo clippy -p zephyra-video --all-targets`：以上五种配置均 0 警告。
- `cargo fmt -p zephyra-video` 干净；`cargo doc --no-deps -p zephyra-video --features av1,vp9` 退出 0。
