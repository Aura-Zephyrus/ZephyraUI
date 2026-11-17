# 更多编码格式：VP8（复用 libvpx）与 HEVC（vendored libde265）

> 2026-11-17 · `zephyra-base/extra/zephyra-video`
> 排查手段：复用已 vendored 的 libvpx；从上游取 libde265 1.0.15（LGPL-3.0+）；全 feature 组合跑 183 条单测

## 现象

用户要求「支持更多编码格式」。在已有 H.264 / AVS3 / AV1 / VP9 的基础上补两种：
**VP8**（Chromium 同样用 libvpx）与 **HEVC / H.265**（Chromium 在多数平台用 FFmpeg 的
内置 HEVC 解码器，但那需要整棵 FFmpeg + config 生成；这里选聚焦的 libde265）。

## 排查

### 1. VP8：同一份 libvpx，换个 codec iface

libvpx 同时提供 `vpx_codec_vp8_dx()` 与 `vpx_codec_vp9_dx()` —— 源码已经 vendored，
VP8 不需要再编一遍 C。把 shim 的 open 函数抽成 `z_vpx_open_with(iface)`，
新增 `z_vpx8_open`，共用 `z_vpx_decode` / `z_vpx_close`。
`vp8` feature 与 `vp9` feature 都触发同一个 `build_vp9()`（用 `cfg(any(...))`），
不会重复编译。`Vpx8Decoder` 与 `Vpx9Decoder` 代码几乎相同，只是 codec 检查与 iface 不同。

### 2. HEVC：libde265 的构建与坑

libde265 1.0.15 是 **C++**（`.cc`）、LGPL-3.0-or-later。它的配置极简 ——
没有 config.h，只有 `de265-version.h.in`，手工填成固定版本即可。
CMakeLists 的源文件列表不含 encoder/ 与 SSE 子目录，照抄 34 个 `.cc` 交给
`cc::Build::cpp(true)`，Windows 加 `extra/win32cond.c`，关掉全部 SIMD（纯 C++ 回退）。

解码流程与 dav1d 类似：`de265_new_decoder` → 循环 `de265_push_data` /
`de265_push_end_of_NAL` / `de265_flush_data` / `de265_decode` →
`de265_get_next_picture` 取帧 → `de265_release_next_picture` 释放。
shim 用 C++ 写（库本身是 C++），对外仍暴露 C ABI 的标量回调。

⚠️ `de265_decode` 对垃圾数据比较宽容（返回 OK 但不产出帧），所以烟测只断言
「不 panic」，不强求报错 —— 这与 dav1d / libvpx 的垃圾码流测试不同。

## 排除的假设

❌「VP8 要再 vendored 一份解码器」—— libvpx 本来就含 VP8，换 codec iface 即可，零额外源码。

❌「HEVC 也照 Chromium 用 FFmpeg」—— 那需要整棵 FFmpeg + configure 生成 config.h，
构建复杂度与体积都远超一个聚焦的 HEVC 解码器；libde265 是合理的工程取舍，
但许可证从 BSD 变成 LGPL，分发义务更重（已在文档里注明）。

## 结论

- 新 feature `vp8` / `hevc`；`Vpx8Decoder` / `HevcDecoder` 接进 `VideoDecoder` 接口，
  输出 8bit I420。`VideoDecoderType::Hevc` 新增（统计用）。
- VP8 与 VP9 共用一份 libvpx 编译产物；HEVC 单独编 libde265。
- 六个后端（dxva2 / uavs3d / av1 / vp8 / vp9 / hevc）可同时开，互不冲突。

## 门禁

- `cargo test -p zephyra-video --offline`：默认 168、单后端 171、三软件 177、
  全 feature（含 dxva2）**183 passed / 0 failed**。
- `cargo clippy -p zephyra-video --all-targets`：vp8 / hevc 及其组合均 0 警告。
- fmt 干净；doc 退出 0；workspace 四条门禁全过。
