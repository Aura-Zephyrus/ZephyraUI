# 视频解码支持：Chromium 形状的 H.264 + DXVA2 硬件解码

> 2026-11-17 · `zephyra-base/extra/zephyra-video`（新 crate，Windows 平台 + `dxva2` feature 时才编设备代码）
> 排查手段：MSVC 探针程序实测 DXVA 结构体布局与位域位序；对照 Chromium `media/gpu/h264_decoder.cc` 与 `d3d_h264_accelerator.cc`；161 条单测（`cargo test -p zephyra-video --offline`）

## 现象

用户要求：「在 extra 做对视频解码的支持，尽量用和 chrome 用一样的方案」，源码位于 `E:\chromium`，全程自主完成。

新 crate `zephyra-video` 放在 `zephyra-base/extra/`（base 侧 extra：可碰平台、允许 unsafe），`default = []`，不进 workspace `default-members`，默认构建是**零依赖的纯逻辑**——解码配置、缓冲、帧布局、解码器选择、H.264 位流解析、DXVA2 的 DPB 与图片参数拼装，全部能在没有 GPU 的机器上单测。开 `dxva2` feature 才编进碰 D3D11 / COM 的那一半。

## 排查

### 1. 为什么选 DXVA2 而不是 Media Foundation

Chromium 在 Windows 上的**硬件**解码走的是 **DXVA2**（`ID3D11VideoDevice::CreateVideoDecoder` + `ID3D11VideoContext::DecoderBeginFrame / SubmitDecoderBuffers / DecoderEndFrame`），输出是 D3D11 纹理，**不落系统内存**。Media Foundation 的 MFT 是 Chromium 里的**软件**路径，本机无硬件 H.264 MFT，直连 `IMFTransform` 解码恒返回 `MF_E_TRANSFORM_NEED_MORE_INPUT`。

⇒ 本 crate 的 Windows 后端也走 DXVA2，形状与 Chromium 一致，输出同样是 GPU 纹理。

### 2. `windows` 0.62 未绑 `GetVideoDevice` / `GetVideoContext`

`ID3D11Device` / `ID3D11DeviceContext` 的接口继承里**没有** `ID3D11VideoDevice` / `ID3D11VideoContext`，不能当成普通方法调用。`windows` crate 0.62 没绑这两个方法。

⇒ 改用 `Interface::cast::<T>()` 运行时 QueryInterface（出参传 `Some(&mut opt)`，不是 `&mut opt`）。`CreateVideoDecoderOutputView` 接受 `&ID3D11Texture2D`（`ID3D11Texture2D: CanInto<ID3D11Resource>`，QUERY=true，可直接传）。

### 3. DXVA2 结构体布局与位域位序（MSVC 探针实测）

`DXVA_PicParams_H264` 实测 `sizeof == 1040`（不是此前推测的 1056），`dxva.h` 用 `#pragma pack(1)`，Rust 用 `#[repr(C)]` 自然对齐，**字段偏移与总长完全一致**。唯独 `DXVA_Slice_H264_Short` 自然对齐会撑到 12 字节，DXVA2 要求 10 字节 ⇒ Rust 侧用 `[u8; 10]` 表示。

位域 `wBitFields` 位序实测（MSVC 小端，第一个声明的位域占最低位）：`field_pic_flag`=bit0 … `IntraPicFlag`=bit15；`chroma_format_idc`=bits4-5，`weighted_bipred_idc`=bits9-10。

`DXVA_Qmatrix_H264` = 224 字节；8×8 缩放列表 DXVA2 只带 2 张，Chromium 拷前两张 ⇒ Rust 只拷 `[0]` / `[1]`。

### 4. 位流解析里两个静默错误的根因

**根因 A：`parse_nalus` 两个 match 分支元组顺序不一致。** 编译通过，行为静默错误——下一个 NAL 的头字节被当成 `0x00` 前导零，NAL 类型被误判。已修成 `(payload, next)` 两处一致。

**根因 B：`find_start_code` 旧「找 0x01 再回看两字节」写法。** 返回偏移与起始码首字节不一致（回看位置与返回值不是同一个偏移），导致负载切错。已改为直接扫 `\0\0\01`，`offset` 始终指向起始码第一个字节。

### 5. POC 计算：MSB 的双重调整根因

Chromium `H264Decoder::CalculatePicOrderCounts()` 在栈上算一次 MSB 调整，然后在 `FinishPicture()` 里把结果写进持久状态（`prev_ref_pic_order_cnt_msb_` 等）。最初实现把「调整」写进 `compute_picture_order_cnt` 的持久状态，**再拿调整后的状态去算下一张图的 MSB** ⇒ 同一轮调整做了两遍，POC 整整差一个 `MaxPicOrderCntLsb`。

⇒ 改为：`compute_picture_order_cnt` **不更新**状态，只返回 `PictureOrderCnt` 五元组（top / bottom / msb / 原始 lsb / frame_num_offset）；状态更新移到 `commit_picture`（Chromium `FinishPicture()`）。`prev_ref_*` 只在**参考**图像更新，`prev_frame_num` / `prev_frame_num_offset` / `prev_has_mmco5` 每张都更新。`prev_pic_order_cnt_lsb` 存**原始 lsb**，不是算出的 POC。type 1 的 `frameNumInPicOrderCntCycle` 从 `(absFrameNum - 1)` 起算；type 2 的 IDR ⇒ POC 0。

### 6. 图片索引重复分配根因

最初「每次从头找第一个没用的索引」：调用方在**解码前**就要拿索引（写进 `DXVA_PicParams_H264.CurrPic.Index7Bits`），那时图还没进 DPB ⇒ 连续两次分配会拿到**同一个索引** ⇒ 驱动把后一张画到前一张的纹理上，画面静默错位。

⇒ 改为「从上次位置往后找第一个没用的」（`allocate_picture_index`），索引空间 0..16（`MAX_PICTURES`），不是 128。16 张全占用返回 `u8::MAX` 让上层发现「DPB 满」，而不是悄悄给出错误画面。

### 7. DXVA2 `RefFrameList` 按 DPB 顺序填

对照 Chromium `D3DH264Accelerator::SetRefFrames`：遍历 DPB，**不是按参考列表顺序**；`Index7Bits = picture_index`，`AssociatedFlag = long_term`，`frame_num_list[i] = long_term ? long_term_pic_num : frame_num`，`used_for_reference_flags |= 3 << (2*i)`，`non_existing_frame_flags |= nonexisting << i`；非参考帧 `continue`（槽位留 0xFF）。驱动自己做参考列表构造，不塞占位。

### 8. 缓冲提交顺序

照抄 Chromium：图片参数 → 反量化矩阵 → 切片控制 → 位流。

### 9. `E_PENDING` 与 `D3DERR_WASSTILLDRAWING`

`windows` 0.62 未绑这两个码 ⇒ 本地定义常量（符号名与回码成对：`E_PENDING（0x8000000A）`、`D3DERR_WASSTILLDRAWING（0x887A0001）`）。`DecoderBeginFrame` 重试上限 4096 次（Chromium 是无限重试；本 crate 是同步 API，无取消手段，驱动卡死不该拖垮整个进程）。

### 10. 输出纹理所有权

每个图片索引一张 NV12 纹理，按索引复用。`VideoFrame` 通过新增的 `NativeResourceOwner`（`Arc<dyn Any + Send + Sync>`）**持有**纹理所有权，句柄不悬空，但内容可能被覆盖（下一帧同索引复用）。

## 排除的假设

❌「MF 直连解码可行」—— 本机无硬件 H.264 MFT，`IMFTransform` 直连解码恒 `MF_E_TRANSFORM_NEED_MORE_INPUT`；Chromium 的 Windows 硬件路径不经过 MF。

❌「`ID3D11Device::GetVideoDevice` / `GetVideoContext` 是普通方法」—— `windows` 0.62 未绑，只能 `Interface::cast::<T>()` 运行时 QI。

❌「图片索引每次从头找第一个没用的」—— 连续两次分配会拿到同一索引，驱动把后一张画到前一张纹理上（画面静默错位）。

❌「POC 状态在 `compute_picture_order_cnt` 里更新」—— 等于把 MSB 调整做了两遍，POC 整整差一个 `MaxPicOrderCntLsb`；状态只在 `commit_picture` 更新。

## 结论

- crate 名 `zephyra-video`，`zephyra-base/extra/zephyra-video`；`default = []`；不进 `default-members`；`windows` 0.62 target-gated 到 `cfg(windows)`；`unsafe_code = "deny"`，仅 `src/dxva2/decoder.rs` 局部 `#[allow(unsafe_code)]`。
- 标识符英文；中文只出现在文档注释散文。
- 硬件解码走 DXVA2（与 Chromium 一致），不走 Media Foundation。
- `VideoFrame` 两态：CPU 平面 / Native（不透明 u64 句柄 = D3D11 纹理指针），由 `NativeResourceOwner` 持有。
- H.264 位流约定：`extra_data` 非空 ⇒ AVCC，空 ⇒ Annex-B。
- DXVA2 `RefFrameList` 按 DPB 顺序填；`picture_index` 空间 16（0..16）；`DXVA_PicParams_H264` 1040 字节；`DxvaSliceH264Short` 用 `[u8; 10]`；位域位序位 0..15；`DXVA_Qmatrix_H264` 224 字节只拷前两张 8×8 缩放列表。
- 缓冲提交顺序：图片参数 → 反量化矩阵 → 切片控制 → 位流。
- POC：`compute_picture_order_cnt` 只计算不更新状态；`commit_picture` 更新（Chromium `FinishPicture()`）；MSB 调整每张图只做一次；`prev_pic_order_cnt_lsb` 存原始 lsb。
- 图片索引：从上次位置往后找第一个没用的；满了返回 `u8::MAX`。
- `DecoderBeginFrame` 重试上限 4096；`E_PENDING` / `D3DERR_WASSTILLDRAWING` 本地定义。

## 门禁

- `cargo test -p zephyra-video --offline`：**161 passed / 0 failed / 0 ignored**（`tests/unit/dxva2_dpb_tests.rs` 与 `tests/unit/dxva2_pic_params_tests.rs` 新写，覆盖 POC type 0/1/2、MMCO、DPB 维护、参考帧列表、图片索引分配、DXVA 布局/位序/填充规则）。
- `cargo clippy -p zephyra-video --all-targets`：默认与 `--features dxva2` 两种都 **0 警告**。
- `cargo fmt -p zephyra-video`：干净。
- `cargo doc --no-deps -p zephyra-video`：退出 0，无新增警告。
- 全仓四条门禁（`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`cargo doc --no-deps --workspace`）：待本条落地后一次性跑完（下一条）。
