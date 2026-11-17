# AVS3 解码：把 uavs3d 的 C 源码原样搬进 extra

> 2026-11-17 · `zephyra-base/extra/zephyra-video`（新 `uavs3d` feature）与 `native/uavs3d/`
> 排查手段：对照 `E:\uavs3d` 的 `source/`、`test/utest.c`、`CMakeLists.txt`；MSVC `cl.exe` 手工复现编译错误；171 条单测（默认 168 + feature 171）

## 现象

用户要求：「`E:\uavs3d` 看这个，对这个视频编码格式支持，直接原样抄」。

`uavs3d` 是北京大学深圳研究生院等的 **AVS3-P2 baseline** 解码器（BSD-3-Clause，C99），
输出 8/10bit 4:2:0。它是**软件**解码器，与上一轮做的 DXVA2 硬件解码是两件事。
「原样抄」落成：把 `E:\uavs3d` 的 112 个文件逐字节搬进
`zephyra-video/native/uavs3d/`，再用一层薄 FFI 包成同一个 `VideoDecoder` 接口。

## 排查

### 1. 先读 API 再决定边界

`uavs3d.h` 只暴露 6 个函数：`uavs3d_create` / `delete` / `reset` / `flush` / `decode` /
`img_cpy_cvt`。`uavs3d_decode(h, frm)` **一次只吃一个 NAL**：它直接读 `frm->bs[3]` 当
NAL 类型字节（`0xB0` 序列头、`0xB1` 序列尾、`0xB3`/`0xB6` 图像头、`0x00..=0x8E` 切片），
所以每个 NAL 必须以**三字节**起始码 `00 00 01` 开头 —— 四字节起始码前面那个 0 要丢掉。
对照 `test/utest.c::find_next_start_code()`：它在缓冲区开头自己写 `00 00 01`，正是这个意思。

⇒ Rust 侧只做两件事：`split_annex_b_nals()` 按合法 NAL 类型切流、按类型字节过滤误判；
`#[repr(C)]` 镜像 `uavs3d_io_frm_t` 的字段（完整列出 C 里的顺序，`seqhdr` 用不透明指针）。

### 2. 回调的生命周期：`frm_threads = 1`

C 的 `uavs3d_output_frame()` 是在 `uavs3d_decode()` / `uavs3d_flush()` 的**调用线程**
里同步调用回调的。把 `frm_threads` 设成 1（不走线程池）后，Rust 侧用线程局部
`RefCell<Vec<VideoFrame>>` 接帧即可，不需要全局锁，也不会串到别的解码器上。
C 的图像缓冲归它自己管、回调返回后可能被复用 ⇒ 必须逐行拷进 Rust 的 `Vec<u8>`
（用 C 给的 `stride[]` 跳过行填充，拷成紧排的 NV12）。

### 3. 两个编译期的坑

**坑 A：`cc-rs` 没给 `cl.exe` 传 MSVC / Windows SDK 的头文件目录。**
症状：`fatal error C1083: cannot open include file: 'time.h'`。手工用 `cl.exe` 复现：
只有加上 `<MSVC>\include` 与 `Windows Kits\10\Include\<ver>\{ucrt,um,shared,...}` 才能编过。
⇒ `build.rs` 里从 `cc::Build::get_compiler().path()` 往上四级定位 `<MSVC>`，
从 `ProgramFiles(x86)\Windows Kits\10\Include` 取最新版本号，把这些目录以 `-I` 加进
`base` 与 `avx2` 两个 build（只影响 C 编译，不改全局环境变量）。

**坑 B：C 头文件里的中文注释被 MSVC 吞掉 `#ifndef`。**
`version.h`（原仓库由 `version.sh` 生成、被 `.gitignore` 排除）第一版写了中文注释；
MSVC 按本地代码页读它，注释行末尾吞掉了下一行的 `#ifndef` ⇒ `unexpected #endif`。
⇒ 这个生成文件改成**纯 ASCII 注释**（它就是 `version.sh` 的 `BUILD=0` 输出，
只被 `uavs3d.c` 启动时的 `printf` 用到，不参与解码）。注意这只影响生成物 ——
vendored 的 112 个源文件**一个字节都没改**。

### 4. 让默认构建继续零依赖

想让 `cc` 只在 `uavs3d` feature 下才是依赖。实验（临时 crate，`build.rs` 里打
`#[cfg(feature = "foo")]` 的 warning）证明 **Cargo 会把 feature 作为 `--cfg` 传给
build script** ⇒ 可以写两个 `#[cfg(...)] fn main()`：无 feature 时 `main()` 是空的，
`cc` 保持 `optional = true` 且根本不会被编译。默认构建仍是零依赖纯逻辑。

### 5. `AVX2` 与 SSE 必须分开编

照抄 `source/CMakeLists.txt`：x86 上 SSE4 源文件与 AVX2 源文件是两组编译 flags。
`build.rs` 把 `decore/avx2/*.c` 单独编成第二个静态库并加 `/arch:AVX2`（MSVC）或
`-mavx2`（gcc），其余 C 文件合成第一个库 —— 避免整个库都要求 AVX2。

### 6. 测试里的假阴性

「喂垃圾 NAL 应返回错误」第一版用了类型字节 `0xFF`，但 `split_annex_b_nals` 按合法
NAL 类型过滤后它根本到不了 C（0 个 NAL ⇒ 0 次调用 ⇒ Ok）。⇒ 改用合法切片类型
`0x01` + 垃圾负载：C 侧 `init_flag` 还没被序列头置位，返回 `ERR_SEQ_HEADER_UNREADY`。

## 排除的假设

❌「AVS3 走 DXVA2 / Media Foundation」—— uavs3d 是纯软件 C 解码器，输出 NV12
系统内存帧，与 D3D11 / D3D12 / Vulkan 三个渲染后端都无关。

❌「`cc` 得是非可选 build-dependency」—— 实测 build script 收得到 feature cfg，
两个 `#[cfg] main()` 能让 `cc` 保持可选，默认构建零依赖不被破坏。

❌「要完整镜像 `com_seqh_t` 等所有 C 结构」—— 只需要 `uavs3d_io_frm_t`（输出帧）
和 `uavs3d_cfg_t`（创建参数）；序列头用不透明指针，Rust 侧不读。

## 结论

- `native/uavs3d/` 是 `E:\uavs3d` 112 个文件的逐字节副本（`git diff --no-index` 无差异）；
  `version.h` 是 `version.sh` 的生成物（ deterministic `BUILD=0` 输出），不是源文件改动。
- 新 feature `uavs3d = ["dep:cc"]`，`cc` 可选；build script 两个 `#[cfg] main()`。
- `src/uavs3d/`：`split_annex_b_nals` / 错误码映射是纯逻辑（默认构建可测）；
  `imp.rs` 是 FFI + `Uavs3dDecoder`，`#[allow(unsafe_code)]` 只加在这一处。
- `VideoCodec::Av3 = 11`、`VideoDecoderType::Uavs3d`；配置 codec 必须是 `Av3`。
- 输出 8bit NV12（`COMPILE_10BIT=0`）；10bit 需要另加 P010LE 帧布局，本轮没做。
- 许可证：crate 自己的 Rust 代码 `MIT OR Apache-2.0`；vendored C 是 BSD-3-Clause，
  `native/uavs3d/COPYING` 与每个源文件的版权头都保留，分发时必须一并带上。
- 本机没有 `.avs3` 样片，所以没有「真码流解出帧」的测试 —— 现有 FFI 烟测只覆盖
  创建/析构/错误路径（足以钉住 struct 布局：若 `bs` 偏移错了，C 会读野指针崩掉）。

## 门禁

- `cargo test -p zephyra-video --offline`：默认 **168 passed**，`--features uavs3d` **171 passed**，全 0 失败。
- `cargo clippy -p zephyra-video --all-targets`：默认与 `--features uavs3d` 均 0 警告。
- `cargo fmt -p zephyra-video`：干净。
- `cargo doc --no-deps -p zephyra-video`：退出 0，无新增警告。
- vendored 源文件与 `E:\uavs3d\source` 逐字节相同（`git diff --no-index` 无差异）。
