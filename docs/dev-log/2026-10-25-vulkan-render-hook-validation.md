# 2026-10-25-vulkan-render-hook-validation

> 现象：Vulkan 对 `Command::Custom` 原先只计数；钩子需要 record 阶段的活命令缓冲，且本机没有物理 Linux GPU。

## 排查

- Vulkan 分成 CPU `prepare` 和 GPU `record_frame` 两阶段，prepare 时没有活的 `vk::CommandBuffer`。因此 `SegmentPlan` 保存原始命令下标，record 阶段再从当帧 `DisplayList` 取回 bounds/source，并在主 render pass 内调用按 kind 注册的钩子。
- WSLg 提供 Wayland 与 Vulkan surface，但 `vulkaninfo --summary` 选择的是 llvmpipe（Mesa 25.2.8，LLVM 20.1.2），属于软件 Vulkan 驱动，不能代替物理 GPU 驱动覆盖。
- WSLg 门禁中，`vkCmdClearAttachments` 的 rect 即使配合 1×1 动态 scissor，仍准确写入 7×9 bounds；无需额外 fill pipeline。读回确认恰好 63 个纯红像素。
- 命令校验门禁启用 Khronos validation layer，并在每个正常帧后抽干 ERROR 队列。callback 的反向对照使用 `VK_EXT_debug_utils` 合成一条 ERROR 消息，以验证回调入队和门禁抽干路径。

## 排除的假设

- prepare 阶段不能直接执行钩子：此时没有命令缓冲；把回调塞入 CPU 计划会破坏两阶段边界。只保存 Custom 下标、延迟到 record 执行。
- `vkCmdClearAttachments` 会被动态 scissor 限制：在当前 llvmpipe WSLg 环境实测不成立；1×1 scissor 下仍得到准确 63 像素。物理 GPU 仍需复核。
- 未注册钩子可以完全跳过 Custom 段：若帧没有普通几何，跳过会漏掉 render pass 清屏，交换链图无法获得有效的 PRESENT 内容。未注册路径现仍开启并结束主 pass，再报告 `unhandled_custom`。
- 用故意非法的 Vulkan 命令作 callback 反向对照是安全的：在当前 llvmpipe 环境尝试后进程发生 SIGSEGV，因此不保留这类反向控制；改用 debug-utils 合成消息。正常帧仍由 validation layer 检查，且要求零 ERROR。

## 结论

- Vulkan 后端已支持 `Renderer::register_hook` 与 `HostWindow::register_hook`。钩子获得命令缓冲、对应 ash 设备函数表和目标尺寸，只能在当前主 render pass 内绘制；v1 不支持切换 render pass / attachment。
- 修正 debug messenger callback 的 `user_data` 解引用：传入对象是 `Arc<Mutex<Vec<DebugMessage>>>`，不能当作裸 `Mutex` 使用。
- Custom-only 帧可能没有任何几何；跳过零字节 vertex/index buffer 的 map/unmap，避免 validation 错误。
- 物理 GPU Vulkan 驱动覆盖仍待后续复核；软件门禁通过不等于硬件兼容性结论。

## 门禁

- WSLg 端到端像素与 validation 门禁：`custom_hook_pixels_and_command_validation`，测试以 `#[ignore]` 标记，执行需加 `--ignored --test-threads=1`。结果：1 passed / 0 failed；注册钩子 63 个红像素，未注册钩子 0 个红像素、`unhandled_custom == 1`，两个正常帧均无 validation ERROR。
- `cargo test -p zephyra-vulkan --lib`：5 passed / 0 failed / 1 ignored。
- `cargo fmt --all -- --check`：通过。
- `cargo clippy -p zephyra-vulkan --all-targets --no-deps -- -D warnings`：通过。带依赖的 clippy 仍被 `zephyra-fx` 既有 dead-code 警告和 `bridge.rs` 的 `while_let_loop` lint 阻断；未改无关文件。
