# ViewStage 迁移所需接口：P0.1 / P0.2 / P0.3 / P1.1 / P1.2 落地

> 2026-11-17 · 清单来源 `E:\Viewstage-Rust\docs\zephyra-required-apis.md`
> 基线本仓 `main`（比清单的 `c251b61` 新）—— P1.3 / P1.4 经查已在更新版本里具备，见文末。

## 现象

清单列了 8 个缺口，按阻塞程度排序。本轮把五个**需要代码**的补齐；P1.3 / P1.4
是文档/既有能力，确认后不改代码。

## 排查与实现

### P0.1 自绘钩子拿不到可绘制上下文

`HookContextExt::downcast_mut::<HookCtx>()` 永远失败：三个后端的 `HookCtx::as_any(_mut)`
返回占位 `()`。清单建议的「直接 `self`」**编译不过** —— `dyn Any: 'static`，而
`HookCtx<'a>` 借用命令列表/设备，不满足 `'static`。

⇒ 把三个后端的 `HookCtx<'a>` 改成无生命周期的 `HookCtx`：字段用裸指针
（`*const ID3D12Device` / `*mut ID3D12GraphicsCommandList` / `*const ash::Device` /
`*const ID3D11DeviceContext` …），配 `&self` / `&mut self` 的安全访问器取回引用。
裸指针是 `'static`，于是 `self` 能作为 `&dyn Any` 返回；返回引用的生命周期被借用
检查器钉在钩子调用内，不能跨帧持有。`cmd` 是 `Copy` 句柄，保持公开。

三个后端各加一条 `hook_ctx_is_downcastable()` 单测（空指针只验证类型转换，不解引用）。

### P0.2 外部纹理高频更新

- **门面**（`zephyra-ext`）：`ExtContextImpl::update_texture` 默认返回 `Backend` 错误，
  `ExtContext` 与 `ExtContextHandle::update_texture(key, pixels, row_pitch)` 透传。
  默认实现保证第三方 impl 与 `MockExt` 不破。
- **D3D12**：`ExtResource.pending_updates: VecDeque`（上限 3 帧），`update_texture` 只
  拷贝像素入队、立即返回；`sync_ext_texture_srvs` 每帧取最新一份，复用首次上传的
  `stage_ext_cpu_upload` 路径就地 `CopyTextureRegion` —— 同帧只处理一份，staging
  在下一帧 `begin_frame` 的全量 `wait_gpu` 后释放。原地更新前补一道
  `PIXEL_SHADER_RESOURCE → COPY_DEST` 屏障。
- **D3D11**：`ExtResource` 本就持有 `ID3D11Texture2D`，直接按 `GetDesc` 的尺寸调用
  新增的 `write_rows_with_pitch`（`SysMemPitch = row_pitch`，驱动自行读行）。无分配。
- **Vulkan**：复用 `Renderer::update_texture_in_place`（不调 `create_image_texture`、
  不销毁旧句柄），并把 `upload_image` 拆出 `upload_image_from_layout` 以传
  `SHADER_READ_ONLY_OPTIMAL` 作旧布局。⚠ `upload_image` 内部仍有 fence 等待 ——
  这是本轮**未消除**的同步点（见下）。

### P0.3 AppWaker + View::on_message

- `View` 加两个**默认空实现** `on_wake(token: u64)` / `on_message(&mut dyn Any)` ——
  四个现有实现者零改动。
- 新 `zephyra_window::AppWaker`：`Arc<Inner>`，`proxy: Mutex<Option<EventLoopProxy<()>>>`
  + `posted/wakes: Mutex<VecDeque<…>>` + `Arc<AtomicBool>` ⇒ `Send + Sync + Clone`，
  可 `move` 进 `thread::spawn`。`post` 入队后 `wake`（发 `()` 事件）；关窗后置
  alive=false，之后 `wake`/`post` 静默丢弃。
- `WindowHandle::waker()` 暴露 clone；`WindowSet::run` 用 `EventLoop::create_proxy()`
  建代理并传给每个句柄（`WindowSet::new` 测试路径无代理 → no-op）。
- `WindowHost::drain_posted()` 默认 false；三后端实现：取走 `Drained` 后按序调
  `on_wake` / `on_message`，置 dirty。`WindowSet::about_to_wait` 在 `aggregate_tick`
  **之前**排空，保证消息当帧可见。
- ⚠ 单窗口兼容路径（`HostWindow::run_with`，`handle: None`）拿不到 waker —— 跨线程
  消息只在 `WindowSet` 多窗口路径生效。

### P1.1 钩子按窗口精确匹配

`WindowHost::window_id()` 默认从 `key().as_u64()` 派生。D3D12 给 `WindowTargets` 加
`window_id`（三个创建点 + `Backend::create` 里设置）；Vulkan 给 `Renderer` 加
`window_id` 并在 `Backend::create` 里设置。两后端原先
`window.keys().find(|(_, k)| k.as_str() == kind)`（只按 kind）改为
`get_mut(&(window_id, kind.to_string()))`。D3D11 无窗口作用域 hook，不需改。

### P1.2 `rect_of`

`Node::collect_keyed_rects(&mut Vec<(ElementKey, Rect)>)` 深度优先收集布局后矩形；
三后端 `sync_input` 在构造 `InputState` 前从 `cache.node()` 采一份塞进新字段
`rects`。`InputState::rect_of(&ElementKey) -> Option<Rect>` 线性查表 —— 与
`pointer_events` 同一物理像素/绝对坐标系。`Vec` 保留 `Default`/`PartialEq`，
现有 `InputState { ..Default::default() }` 全部不破。

### P1.3 / P1.4（清单里的 remaining）

- P1.3：**已具备**。`zephyra-style::SheetStyler`（`src/apply.rs:86`）实现
  `zephyra_core::styler::Styler`，`SheetStyler::new(Arc<StyleSheet>)` 的用法见
  `gallery/src/bin/main.rs:445`。清单的基线 `c251b61` 之后合入，无需改代码。
- P1.4：DComp / 透明分层窗口的推荐组合方式属应用侧决策，框架能力
  （`WindowSpec.transparent` + `Image` quad / `zephyra-dcomp`）已在，不改代码。

## 门禁

- 新增单测：`AppWaker` ×4、`InputState::rect_of` ×2（含 View 默认钩子）、
  三后端 `hook_ctx_is_downcastable` ×1、`update_texture` 门面 ×1。
- `cargo test -p zephyra-ext` 19 passed、`-p zephyra-window` 16 passed、
  `-p zephyra-fx --lib` 172 passed；三后端 `cargo check` 通过。
- workspace 四条门禁见提交正文。
