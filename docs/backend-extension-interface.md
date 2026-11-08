# Zephyra 后端拓展接口标准化设计文档

> 版本：v1.18  
> 日期：2026-11-07  
> 状态：设计评审通过，§3/§4 已按 §17 对齐

---

## 1. 最终架构：平台原生后端 + 统一扩展契约

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  应用层                                                                        │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │ 统一扩展接口：ExternalTexture / RenderHook / RenderExtension              │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                    │                                         │
│              ┌─────────────────────┼─────────────────────┐                  │
│              ▼                     ▼                     ▼                  │
│     ┌───────────────┐      ┌───────────────┐      ┌───────────────┐        │
│     │ Windows       │      │ Linux         │      │ macOS (未来)   │        │
│     │ D3D11 / D3D12 │      │ Vulkan        │      │ Metal         │        │
│     │               │      │               │      │               │        │
│     │ - UI 渲染      │      │ - UI 渲染      │      │ - UI 渲染      │        │
│     │ - ExternalTex │      │ - ExternalTex │      │ - ExternalTex │        │
│     │   NT Handle   │      │   VkImage     │      │   IOSurface   │        │
│     │ - RenderHook  │      │ - RenderHook  │      │ - RenderHook  │        │
│     │   D3D11 Ctx   │      │   VkCmdBuf    │      │   MTLCommand  │        │
│     └───────────────┘      └───────────────┘      └───────────────┘        │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 核心原则

| 平台 | 主后端 | 扩展共享方式 | RenderHook 上下文 | 同步原语 |
|------|--------|-------------|------------------|----------|
| **Windows** | **D3D11** (默认) / **D3D12** (可选) | **NT Handle** (`ID3D11Texture2D` 共享) | `&ID3D11DeviceContext` | `ID3D12Fence` / `WaitForSingleObject` |
| **Linux** | **Vulkan** | **VkImage** 零拷贝 (`VkImageView`) | `&VkCommandBuffer` | `VkSemaphore` / `VkFence` |
| **macOS** (未来) | **Metal** | **IOSurface** / `MTLTexture` | `&MTLCommandBuffer` | `MTLEvent` / `dispatch_semaphore` |

**应用代码完全一致**，只在注册时传入后端特定的句柄类型。

---

## 2. 后端职责分配

| Crate | 平台 | 职责 |
|-------|------|------|
| **zephyra-winit** | Windows | D3D11 主后端：UI、窗口、呈现、`register_external_texture` (NT Handle)、`RenderHook` (即时上下文) |
| **zephyra-winit-d3d12** | Windows | D3D12 可选后端：同 D3D11 但用 D3D12 API |
| **zephyra-vulkan** | Linux | Vulkan 主后端：UI、窗口、呈现、`register_external_texture` (VkImage)、`RenderHook` (命令缓冲) |
| **zephyra-render-ext** | 跨平台 | 统一契约：`ExternalTexture`、`RenderExtension`、`ExternalTextureImporter`、`Capability`、`Lifecycle` |
| **zephyra-ext** (新增) | 跨平台 | **统一门面 API**：`ExtContextHandle`、`ExternalTextureBuilder`、`HookRegistry`、`TextureHandle` —— 应用零分支、类型安全、RAII 生命周期 |

---

## 3. 统一门面 API 设计 (`zephyra-ext`)

### 3.1 核心原则

| 原则 | 实现方式 |
|------|----------|
| **应用零平台分支** | 所有公共 API 无 `#[cfg]`，平台差异在框架内部路由 |
| **类型安全** | 平台特定 Hook 通过泛型/特化提供编译期保证 |
| **单一入口** | `ExtContextHandle` 统一管理注册、生命周期、能力查询 |
| **零成本抽象** | 编译期单态化，运行时零开销 |

---

### 3.2 统一入口：`ExtContextHandle`

```rust
// zephyra-ext/src/ext_context.rs
use std::cell::RefCell;
use std::rc::Rc;

/// 扩展系统统一上下文：应用只持有这一个句柄。
/// **线程模型（Phase 1）**：仅 UI/渲染线程使用，`!Send + !Sync`。
pub struct ExtContextHandle {
    inner: Rc<RefCell<ExtContext>>,
}

impl ExtContextHandle {
    /// 创建扩展上下文（由 HostWindow 在初始化时创建并传给应用）
    pub(crate) fn new(inner: Box<dyn ExtContextImpl>) -> Self {
        Self { inner: Rc::new(RefCell::new(ExtContext::new(inner))) }
    }

    /// 获取可变上下文（执行注册/查询操作）
    pub fn ctx(&self) -> std::cell::RefMut<'_, ExtContext> {
        self.inner.borrow_mut()
    }

    /// 克隆句柄（同线程共享）
    pub fn clone_handle(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/// 扩展上下文操作接口
pub struct ExtContext {
    inner: Box<dyn ExtContextImpl>,
}

impl ExtContext {
    /// 注册/更新外部纹理：统一入口，无平台分支。
    /// 同 key 已存在时执行**替换**（保持纹理逻辑句柄有效，触发重绘标记）。
    pub fn register_texture(&mut self, builder: ExternalTextureBuilder) 
        -> Result<ExternalTextureHandle, ExtError> 
    {
        self.inner.register_texture(builder)
    }

    /// 获取 Hook 注册表
    pub fn hooks(&mut self) -> HookRegistry<'_> {
        HookRegistry { ctx: self }
    }

    /// 查询后端能力
    pub fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    /// 获取当前帧 ID（用于同步）
    pub fn frame_id(&self) -> FrameId {
        self.inner.frame_id()
    }
}

/// 内部 trait：平台特定实现
trait ExtContextImpl {
    fn register_texture(&mut self, builder: ExternalTextureBuilder) 
        -> Result<ExternalTextureHandle, ExtError>;
    fn register_hook(&mut self, kind: Cow<'static, str>, hook: Box<DrawHook>) 
        -> Result<HookHandle, ExtError>;
    fn unregister_texture(&mut self, key: &str) -> Result<(), ExtError>;
    fn unregister_hook(&mut self, kind: &str) -> bool;
    fn capabilities(&self) -> BackendCapabilities;
    fn frame_id(&self) -> FrameId;
}
```

---

### 3.3 统一纹理构建器：`ExternalTextureBuilder`

```rust
// zephyra-ext/src/texture_builder.rs

/// 外部纹理构建器：链式 API，隐藏平台差异
pub struct ExternalTextureBuilder {
    key: Cow<'static, str>,
    size: (u32, u32),
    format: ExternalTextureFormat,
    source: TextureSourceInput,
    sync: Option<SyncInput>,
    color_space: Option<ColorSpace>,
    hdr_metadata: Option<HdrMetadata>,
}

impl ExternalTextureBuilder {
    pub fn new(key: impl Into<Cow<'static, str>>, size: (u32, u32)) -> Self {
        Self {
            key: key.into(),
            size,
            format: ExternalTextureFormat::Rgba8UnormSrgb,
            source: TextureSourceInput::Auto,
            sync: None,
            color_space: None,
            hdr_metadata: None,
        }
    }

    pub fn format(mut self, fmt: ExternalTextureFormat) -> Self { self.format = fmt; self }

    /// 设置来源：自动推断（推荐）
    pub fn source_auto(mut self, source: impl Into<TextureSourceInput>) -> Self {
        self.source = source.into(); self
    }

    /// 显式指定 NT Handle (Windows 专用，不透明句柄)
    #[cfg(windows)]
    pub fn source_nt_handle(mut self, handle: RawHandle) -> Self {
        self.source = TextureSourceInput::NtHandle(handle); self
    }

    /// 显式指定 VkImage (Linux/Vulkan 专用，不透明句柄)
    #[cfg(target_os = "linux")]
    pub fn source_vk_image(mut self, image: RawHandle, layout: u32) -> Self {
        self.source = TextureSourceInput::VkImage { image, layout }; self
    }

    /// 同步原语：自动推断最优
    pub fn sync_auto(mut self, sync: impl Into<SyncInput>) -> Self {
        self.sync = Some(sync.into()); self
    }

    pub fn color_space(mut self, cs: ColorSpace) -> Self { self.color_space = Some(cs); self }
    pub fn hdr_metadata(mut self, meta: HdrMetadata) -> Self { self.hdr_metadata = Some(meta); self }

    /// 构建并注册（消费 self）。同 key 执行替换更新。
    pub fn register(self, ctx: &mut ExtContext) -> Result<ExternalTextureHandle, ExtError> {
        ctx.register_texture(self)
    }
}

/// 来源输入：构建器层（可含 `Auto`），注册时解析为契约层 `ExternalTextureSource`。
#[non_exhaustive]
pub enum TextureSourceInput {
    Auto,
    NtHandle(RawHandle),
    VkImage { image: RawHandle, layout: u32 },
    Dmabuf { fd: i32, modifier: u64 },
    Cpu(CpuTextureDescOwned),
}

/// 同步输入：统一枚举（围栏使用 `Arc` 以便复用）
#[non_exhaustive]
pub enum SyncInput {
    Auto,
    Fence(std::sync::Arc<dyn GpuFence>),
    None,
}
```

**应用使用示例：**
```rust
use zephyra_ext::prelude::*;

// 完全无平台分支！
let texture = ExternalTextureBuilder::new("video-frame", (1920, 1080))
    .format(ExternalTextureFormat::Rgba8UnormSrgb)
    .source_auto(video_frame)  // video_frame 实现 Into<TextureSourceInput>
    .sync_auto(frame_fence)
    .color_space(ColorSpace::BT709)
    .register(&mut ext_ctx)?;
```

---

### 3.4 类型安全 Hook 注册：`HookRegistry`

```rust
// zephyra-ext/src/hook_registry.rs

/// Hook 注册表：提供类型安全的平台特定注册
pub struct HookRegistry<'a> {
    ctx: &'a mut ExtContext,
}

impl<'a> HookRegistry<'a> {
    /// 注册 D3D11 Hook (仅 Windows 编译)
    #[cfg(windows)]
    pub fn d3d11<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: FnMut(&D3D11HookContext, Rect, &RenderSource) + Send + 'static,
    {
        self.ctx.register_hook(kind, D3D11HookAdapter(hook))
    }

    /// 注册 D3D12 Hook (仅 Windows 编译)
    #[cfg(windows)]
    pub fn d3d12<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: FnMut(&D3D12HookContext, Rect, &RenderSource) + Send + 'static,
    {
        self.ctx.register_hook(kind, D3D12HookAdapter(hook))
    }

    /// 注册 Vulkan Hook (仅 Linux 编译)
    #[cfg(target_os = "linux")]
    pub fn vulkan<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: FnMut(&VulkanHookContext, Rect, &RenderSource) + Send + 'static,
    {
        self.ctx.register_hook(kind, VulkanHookAdapter(hook))
    }

    /// 通用 Hook (跨平台，仅用公共 HookContext 方法)
    pub fn generic<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where
        F: FnMut(&dyn HookContext, Rect, &RenderSource) + Send + 'static,
    {
        self.ctx.register_hook(kind, hook)
    }
}

/// ExtContext 扩展方法
impl ExtContext {
    pub fn hooks(&mut self) -> HookRegistry<'_> {
        HookRegistry { ctx: self }
    }
}
```

---

### 3.5 RAII 生命周期包装器

```rust
// zephyra-ext/src/handles.rs
use std::borrow::Cow;
use std::rc::Weak;
use std::cell::RefCell;

/// 外部纹理 RAII 句柄：Drop 时自动 unregister。仅 UI/渲染线程使用。
pub struct ExternalTextureHandle {
    key: Cow<'static, str>,
    ctx: Weak<RefCell<ExtContext>>,
}

impl ExternalTextureHandle {
    pub fn key(&self) -> &str { &self.key }

    /// 同 key 替换纹理内容/源（保持句柄有效，触发重绘标记）。
    pub fn update(&self, builder: ExternalTextureBuilder) -> Result<(), ExtError> {
        let Some(ctx) = self.ctx.upgrade() else { return Err(ExtError::ContextDropped); };
        ctx.borrow_mut().register_texture(builder).map(|_| ())
    }

    /// 显式移除
    pub fn unregister(self) -> Result<(), ExtError> {
        self.try_unregister()
    }

    pub fn try_unregister(&self) -> Result<(), ExtError> {
        let Some(ctx) = self.ctx.upgrade() else { return Err(ExtError::ContextDropped); };
        ctx.borrow_mut().unregister_texture(&self.key)
    }
}

impl Drop for ExternalTextureHandle {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.upgrade() {
            if let Err(e) = ctx.borrow_mut().unregister_texture(&self.key) {
                crate::debug::log_texture(&format!("drop unregister failed key={}: {e}", self.key));
            }
        }
    }
}

/// Hook 句柄：Drop 时自动移除
pub struct HookHandle { /* 类似实现，Drop 时 unregister_hook */ }
```

**应用使用：**
```rust
let ext = ExtContextHandle::new(platform_create_ext_context()); // HostWindow 创建

// 纹理：作用域结束自动清理
{
    let tex = ExternalTextureBuilder::new("temp", (100,100))
        .source_auto(data)
        .register(&mut ext.ctx())?;
    // 使用 tex...
} // 离开作用域自动 unregister

// Hook：同上
let _hook = ext.ctx().hooks().d3d11("debug", |ctx, rect, src| { ... })?;
```

---

### 3.6 统一导出：`prelude.rs`

```rust
// zephyra-ext/src/prelude.rs

pub use crate::ext_context::{ExtContextHandle, ExtContext, ExtError};
pub use crate::texture_builder::{ExternalTextureBuilder, TextureSourceInput, SyncInput, CpuTextureDescOwned};
pub use crate::handles::{ExternalTextureHandle, HookHandle};
pub use crate::hook_registry::HookRegistry;
pub use crate::types::{ExternalTextureFormat, ColorSpace, HdrMetadata, FrameId, RawHandle, AdapterIdentity};
pub use crate::capability::{BackendCapabilities, Capability};
pub use crate::lifecycle::{LifecycleContext, RenderLifecycle};
pub use crate::registration::{HookContext, DrawHook, RenderSource, Rect};
pub use crate::sync::{GpuFence, FenceHandle};

#[cfg(windows)]
pub use crate::platform::d3d11::D3D11HookContext;
#[cfg(windows)]
pub use crate::platform::d3d12::D3D12HookContext;
#[cfg(target_os = "linux")]
pub use crate::platform::vulkan::VulkanHookContext;

// use zephyra_ext::prelude::*;
```

---

### 3.7 平台内部实现路由（应用不可见）

```rust
// zephyra-ext/src/platform/mod.rs

// 内部 trait：平台特定实现
trait ExtContextImpl: Send + Sync {
    fn register_texture(&mut self, builder: ExternalTextureBuilder) 
        -> Result<TextureHandle, ExtError>;
    // ...
}

// Windows: D3D11 实现
#[cfg(windows)]
mod d3d11_impl {
    use super::*;
    
    pub struct D3D11ExtContext {
        renderer: Arc<Mutex<D3D11Renderer>>,
        vulkan_interop: Option<VulkanInterop>, // 按需创建
    }
    
    impl ExtContextImpl for D3D11ExtContext {
        fn register_texture(&mut self, builder: ExternalTextureBuilder) 
            -> Result<TextureHandle, ExtError> 
        {
            // 1. 根据 builder.source 自动路由：
            //    - NtHandle -> 直接导入 D3D11
            //    - VkImage -> 通过 vulkan_interop 转 NT Handle 再导入
            //    - CpuBuffer -> 普通上传
            // 2. 复用句柄池
            // 3. 返回 TextureHandle
        }
    }
}

// Linux: Vulkan 实现
#[cfg(target_os = "linux")]
mod vulkan_impl { /* 类似 */ }
```

---

### 3.8 应用最终代码：完全跨平台

```rust
use zephyra_ext::prelude::*;

fn setup_extensions(ext: &mut ExtContextHandle, video: &VideoSource) {
    // 1. 注册视频纹理：零平台分支
    let video_tex = ExternalTextureBuilder::new("video", video.size())
        .format(ExternalTextureFormat::Rgba8UnormSrgb)
        .source_auto(video)  // video 实现 Into<TextureSourceInput>
        .sync_auto(video.fence())
        .color_space(ColorSpace::BT709)
        .register(&mut ext.ctx())?;

    // 2. 注册自定义渲染 Hook：类型安全
    #[cfg(windows)]
    ext.ctx().hooks().d3d11("custom-shader", |ctx, rect, src| {
        let payload: &ShaderPayload = src.payload().unwrap();
        ctx.device_context().DrawIndexedInstanced(...);
    })?;

    #[cfg(target_os = "linux")]
    ext.ctx().hooks().vulkan("custom-shader", |ctx, rect, src| {
        let payload: &ShaderPayload = src.payload().unwrap();
        unsafe { vkCmdDrawIndexed(ctx.command_buffer(), ...); }
    })?;

    // 3. 跨平台通用 Hook
    ext.ctx().hooks().generic("debug-overlay", |ctx, rect, src| {
        println!("Frame: {:?}, Size: {:?}", ext.ctx().frame_id(), ctx.size());
    })?;

    // 4. 查询能力
    if ext.ctx().capabilities().has(Capability::Readback) {
        // 启用截图功能
    }

    Ok(())
}
```

---

### 3.8.1 ExternalTexture 与 UI 布局集成：像控件一样显示在指定位置

**核心机制（v1.17 对齐）**：**Key 关联模式**。`ExternalTextureHandle`（RAII）只负责注册/生命周期，**不进入 DisplayList**；`Segment::Custom` 携带 `resource_key`，后端 replay 按 key 从纹理池解析并注入 `RenderSource`。

#### 完整链路

```
应用代码                          UI 层                          渲染后端
────────────────────────────────────────────────────────────────────────────
1. 注册纹理
   let _tex: ExternalTextureHandle =
     ExternalTextureBuilder::new("video-frame", size)
       .source_auto(frame).sync_auto(fence.into())
       .register(&mut ext.ctx())?;

2. 注册 Hook（kind = "video-player"）
   ext.ctx().hooks().d3d11("video-player", |ctx, bounds, src| {
       let imported: &D3D11ImportedTexture = src.payload().expect("imported");
       ctx.device_context().DrawIndexed(...);
   })?;

3. UI 树（像普通控件）
   View::custom("video-player")
     .resource_key("video-frame")   // 仅 key，不传 GPU 句柄
     .size(640, 360)

4. DisplayList / Segment
   Segment::Custom {
     kind: "video-player",
     bounds: Rect { x,y,w,h },
     resource_key: Some("video-frame"),
   }

5. 后端 replay
   hook = registry[kind]
   imported = texture_pool.get(resource_key)
   ctx = make_hook_context(bounds, imported)
   source = RenderSource::new(resource_key, imported)
   hook(&mut ctx, bounds, &source)

6. 呈现上屏
```

#### 关键点：`kind` + `resource_key`

| 环节 | 字段 | 作用 |
|------|------|------|
| **注册纹理** | `ExternalTextureBuilder::new("video-frame", ...)` | `resource_key` 全局唯一（ExtContext 级） |
| **注册 Hook** | `hooks().d3d11("video-player", ...)` | `kind` 绑定绘制逻辑 |
| **UI 树** | `View::custom("video-player").resource_key("video-frame")` | `kind` 选 Hook，`resource_key` 选纹理 |
| **回放** | `src.resource_key()` / `src.payload::<T>()` | key 用于诊断；payload 为后端注入的已导入纹理 |

#### 应用示例（关键片段）

```rust
let _tex = ExternalTextureBuilder::new("video-frame", video.size())
    .format(ExternalTextureFormat::Rgba8UnormSrgb)
    .source_auto(video)
    .sync_auto(std::sync::Arc::new(video.fence()))
    .register(&mut ext.ctx())?; // RAII：保持存活；update() 可逐帧替换

ext.ctx().hooks().d3d11("video-player", |ctx, bounds, src| {
    let Some(imported) = src.payload::<D3D11ImportedTexture>() else { return; };
    imported.draw_to(ctx.device_context(), bounds);
})?;

let ui = View::column()
    .child(View::custom("video-player").resource_key("video-frame").size(640, 360))
    .build();
```

#### 后端 replay 要点

```rust
fn replay_custom(&mut self, seg: &Segment) {
    let Segment::Custom { kind, bounds, resource_key } = seg else { return };
    let Some(hook) = self.hook_registry.get_mut(kind) else { return };
    let imported = resource_key.as_deref().and_then(|k| self.texture_pool.get(k));
    let mut ctx = self.make_hook_context(*bounds);
    let source = RenderSource::new(resource_key.as_deref(), imported.map(|t| t as &dyn Any));
    hook(&mut ctx, *bounds, &source);
}
```

---

### 3.9 这套 API 解决的问题总结

| 问题 | 解决方式 |
|------|----------|
| **平台分支** | `ExternalTextureBuilder::source_auto()` + `hooks().d3d11/vulkan/generic()` 编译期分发 |
| **类型安全** | 平台特定 Hook 上下文 (`D3D11HookContext` 等) 通过泛型约束保证 |
| **生命周期** | `ExternalTextureHandle`/`HookHandle` 实现 `Drop` 自动清理（Debug 记录失败） |
| **统一入口** | `ExtContextHandle` 单一持有句柄（Phase 1 单线程），所有操作通过它 |
| **零成本** | 编译期单态化，运行时仅虚表调用；Release 调试代码编译期剔除 |
| **可扩展** | 新平台只需实现 `ExtContextImpl` + 平台特定 HookContext，应用代码完全不用动 |

---

### 3.10 统一门面模式：所有跨平台差异的唯一处理方式

> **核心模式**：应用零分支、框架内部路由、后端各自实现、契约层做类型统一

| 领域 | 应用看到的统一 API | 框架内部路由实现 |
|------|-------------------|-----------------|
| **外部纹理** | `ExternalTextureBuilder::source_auto()` | `ExtContextImpl::register_texture()` 按平台分发 NT Handle / VkImage / dmabuf / CPU |
| **RenderHook** | `hooks().d3d11()` / `hooks().vulkan()` / `hooks().generic()` | `HookRegistry` 泛型包装 → 后端存 `Box<DrawHook>`（`for<'a> FnMut(&mut dyn HookContext, Rect, &RenderSource<'a>)`） |
| **HookContext** | `&dyn HookContext`（14 类别全默认实现） | D3D11 实现 `downcast_ref::<ID3D11DeviceContext>()`，Vulkan 实现 `downcast_ref::<VkCommandBuffer>()` |
| **能力查询** | `ext.ctx().capabilities().has(Capability::Readback)` | 后端返回各自的 `BackendCapabilities` 位图 |
| **生命周期** | `LifecycleContext::on_prepare/on_resize/on_device_lost/on_device_restored` | D3D11 在 `Present` 失败触发，D3D12 在 `ExecuteCommandLists` 失败触发，Vulkan 在 `vkQueueSubmit` 失败触发 |
| **同步原语** | `SyncInput::Auto` / `Arc<dyn GpuFence>` | Windows: `ID3D12Fence` NT Handle；Linux: `VkSemaphore` Timeline。框架只负责同步点，不创建 |
| **错误处理** | `ImportError::is_retryable()` | 统一变体，后端把 `DXGI_ERROR_DEVICE_REMOVED`/`VK_ERROR_DEVICE_LOST` 映射为 `DeviceLost` |
| **纹理格式** | `ExternalTextureFormat` 统一枚举 | 后端内部转 `DXGI_FORMAT` / `VkFormat` |
| **色彩空间/HDR** | `ColorSpace` / `HdrMetadata` | 后端按能力决定是否生效，不支持则忽略 |

**代码层面的统一门面三要素**：

```rust
// 1. 公共 trait（zephyra-render-ext / zephyra-ext 导出）
pub trait UnifiedTrait {
    fn unified_method(&mut self, input: UnifiedInput) -> Result<UnifiedOutput, UnifiedError>;
}

// 2. 内部实现 trait（zephyra-ext 内部，平台特定）
trait PlatformImpl: Send + Sync {
    fn platform_specific_method(&mut self, ...) -> Result<..., ...>;
}

// 3. 平台实现（zephyra-winit / zephyra-winit-d3d12 / zephyra-vulkan 各自 crate）
#[cfg(windows)]
struct D3D11Impl { ... }
#[cfg(windows)]
impl PlatformImpl for D3D11Impl { ... }

#[cfg(target_os = "linux")]
struct VulkanImpl { ... }
#[cfg(target_os = "linux")]
impl PlatformImpl for VulkanImpl { ... }

// 4. 统一入口（zephyra-ext，应用唯一可见）
pub struct UnifiedFacade {
    inner: Box<dyn PlatformImpl>, // 运行时由 HostWindow 创建时注入
}
impl UnifiedTrait for UnifiedFacade {
    fn unified_method(&mut self, input: UnifiedInput) -> Result<UnifiedOutput, UnifiedError> {
        self.inner.platform_specific_method(input.into())
    }
}
```

---

## 4. 统一契约接口 (zephyra-render-ext)

> **v1.18 对齐**：契约层**零平台依赖**（无 `windows`/`ash` 类型），使用不透明句柄；导入统一单入口。

### 4.1 ExternalTexture

```rust
// zephyra-render-ext/src/external_texture.rs
use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;
use crate::types::{ExternalTextureFormat, FrameId, RawHandle};
use crate::sync::GpuFence;
use crate::error::ImportError;

/// CPU 纹理上传描述（借用，后端按需拷贝）
#[derive(Clone, Copy, Debug)]
pub struct CpuTextureDesc<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: ExternalTextureFormat,
}

/// 跨平台统一描述符
pub struct ExternalTextureDesc {
    pub key: Cow<'static, str>,
    pub size: (u32, u32),
    pub format: ExternalTextureFormat,
    pub source: ExternalTextureSource,
    pub sync: Option<Arc<dyn GpuFence>>,
}

/// 源类型：平台句柄使用不透明 `RawHandle`
#[derive(Clone, Copy, Debug)]
pub enum ExternalTextureSource {
    NtHandle { handle: RawHandle },
    VkImage { image: RawHandle, layout: u32 },
    #[cfg(target_os = "linux")]
    Dmabuf { fd: i32, modifier: u64, planes: u32, offsets: [u32; 4], strides: [u32; 4] },
}

/// 导入结果（帧级租借）
pub struct ImportedTexture {
    pub handle: Box<dyn Any + Send + Sync>,
    pub valid_until: FrameId,
}

/// 统一入口：后端按 source 分发；不支持返回 `ImportError::UnsupportedSource`
pub trait ExternalTextureImporter {
    fn import(&mut self, desc: ExternalTextureDesc) -> Result<ImportedTexture, ImportError>;
    fn import_cpu(&mut self, desc: CpuTextureDesc<'_>) -> Result<ImportedTexture, ImportError>;
}
```

### 4.2 HookContext —— 零空档、全覆盖版

```rust
// zephyra-render-ext/src/registration.rs

use crate::types::{ExternalTextureFormat, ColorSpace, Rect, GpuFence};
use crate::sync::FenceHandle;
use std::any::Any;

/// 渲染 Hook 执行时的完整上下文参数
/// 
/// 覆盖：几何、帧信息、渲染目标、管线状态、资源、同步、调试、自定义、输入、XR
/// 所有方法都有默认实现，后端仅需实现能提供的字段。
/// 不支持的字段返回 None/默认值，应用需做兼容处理。
pub trait HookContext: Send + Sync {
    // ═══════════════════════════════════════════════════════════════
    // 1. 基础标识
    // ═══════════════════════════════════════════════════════════════
    
    /// 当前渲染 pass 名称（"main"/"shadow"/"ui"/"post"/"custom" 等）
    fn render_pass_name(&self) -> Option<&str> { None }
    
    /// 当前渲染 pass 类型
    fn render_pass_type(&self) -> RenderPassType { RenderPassType::Unknown }
    
    /// 当前子 pass 索引（同一 pass 内多阶段）
    fn subpass_index(&self) -> u32 { 0 }
    
    // ═══════════════════════════════════════════════════════════════
    // 2. 帧信息
    // ═══════════════════════════════════════════════════════════════
    
    /// 当前帧号（单调递增，从 0 开始）
    fn frame_index(&self) -> u64;
    
    /// 上一帧号（用于插值、双缓冲切换）
    fn prev_frame_index(&self) -> Option<u64> { None }
    
    /// 帧时间（秒，相对应用启动）
    fn frame_time(&self) -> f32 { 0.0 }
    
    /// 帧间隔时间（秒，上一帧到这一帧）
    fn delta_time(&self) -> f32 { 0.0 }
    
    /// 当前帧的时间戳（高精度，纳秒）
    fn frame_timestamp_ns(&self) -> u64 { 0 }
    
    /// 是否为首帧
    fn is_first_frame(&self) -> bool { false }
    
    /// 全局资源版本号（资源更新时递增）
    fn resource_version(&self) -> u64 { 0 }
    
    // ═══════════════════════════════════════════════════════════════
    // 3. 基础几何
    // ═══════════════════════════════════════════════════════════════
    
    /// 目标渲染区域尺寸（像素，物理分辨率）
    fn size(&self) -> (f32, f32);
    
    /// 当前视口矩形（通常等于 size，但可能不同）
    fn viewport(&self) -> Rect {
        let (w, h) = self.size();
        Rect::new(0.0, 0.0, w, h)
    }
    
    /// 当前裁剪矩形（用于部分重绘/脏区优化）
    fn scissor_rect(&self) -> Option<Rect> {
        self.damage()
    }
    
    /// 当前脏区域（脏矩形合集的包围盒）
    fn damage(&self) -> Option<Rect> { None }
    
    /// 多视图/立体渲染的视图数量
    fn view_count(&self) -> u32 { 1 }
    
    /// 当前视图索引（多视图渲染时）
    fn view_index(&self) -> u32 { 0 }
    
    // ══════════════════════════════════════════════════════════════
    // 4. 渲染目标
    // ═══════════════════════════════════════════════════════════════
    
    /// 是否在离屏 pass 中渲染
    fn is_offscreen(&self) -> bool { false }
    
    /// 离屏 pass 索引（多级离屏时区分层级）
    fn offscreen_pass_index(&self) -> Option<usize> { None }
    
    /// 当前渲染目标信息
    /// 返回 (width, height, format, sample_count, is_srgb)
    fn render_target_info(&self) -> Option<RenderTargetInfo> { None }
    
    /// 当前深度/模板目标信息
    fn depth_stencil_info(&self) -> Option<DepthStencilInfo> { None }
    
    /// 当前渲染目标的 DPI 缩放因子
    fn dpi_scale(&self) -> f32 { 1.0 }
    
    /// 当前渲染目标的 DPI 缩放因子（分离 X/Y）
    fn dpi_scale_xy(&self) -> (f32, f32) {
        let s = self.dpi_scale();
        (s, s)
    }
    
    /// 输出色彩空间
    fn color_space(&self) -> ColorSpace { ColorSpace::SRGB }
    
    /// 是否启用 sRGB 写入
    fn is_srgb(&self) -> bool { true }
    
    /// MSAA 采样数
    fn sample_count(&self) -> u32 { 1 }
    
    /// MSAA 质量级别
    fn sample_quality(&self) -> u32 { 0 }
    
    // ══════════════════════════════════════════════════════════════
    // 5. 摄像机/视图
    // ══════════════════════════════════════════════════════════════
    
    /// 视图矩阵（世界→视图，列主序）
    fn view_matrix(&self) -> Option<[[f32; 4]; 4]> { None }
    
    /// 投影矩阵（视图→裁剪，列主序）
    fn projection_matrix(&self) -> Option<[[f32; 4]; 4]> { None }
    
    /// 视图投影合并矩阵
    fn view_projection_matrix(&self) -> Option<[[f32; 4]; 4]> { None }
    
    /// 逆视图矩阵
    fn inverse_view_matrix(&self) -> Option<[[f32; 4]; 4]> { None }
    
    /// 逆投影矩阵
    fn inverse_projection_matrix(&self) -> Option<[[f32; 4]; 4]> { None }
    
    /// 逆视图投影矩阵
    fn inverse_view_projection_matrix(&self) -> Option<[[f32; 4]; 4]> { None }
    
    /// 摄像机世界位置
    fn camera_position(&self) -> Option<[f32; 3]> { None }
    
    /// 摄像机世界朝向（前向向量）
    fn camera_forward(&self) -> Option<[f32; 3]> { None }
    
    /// 摄像机世界上向量
    fn camera_up(&self) -> Option<[f32; 3]> { None }
    
    /// 摄像机最近裁剪面
    fn camera_near(&self) -> f32 { 0.1 }
    
    /// 摄像机最远裁剪面
    fn camera_far(&self) -> f32 { 1000.0 }
    
    /// 摄像机视野角（弧度，垂直）
    fn camera_fov_y(&self) -> f32 { std::f32::consts::PI / 3.0 }
    
    /// 摄像机宽高比
    fn camera_aspect(&self) -> f32 {
        let (w, h) = self.size();
        w / h.max(1.0)
    }
    
    // ══════════════════════════════════════════════════════════════
    // 6. 管线状态（只读查询，不保证所有后端都支持）
    // ══════════════════════════════════════════════════════════════
    
    /// 当前绑定的图形管线 ID
    fn graphics_pipeline_id(&self) -> Option<PipelineId> { None }
    
    /// 当前绑定的计算管线 ID
    fn compute_pipeline_id(&self) -> Option<PipelineId> { None }
    
    /// 当前绑定的光线追踪管线 ID
    fn ray_tracing_pipeline_id(&self) -> Option<PipelineId> { None }
    
    /// 当前绑定的图形管线名称（调试用）
    fn graphics_pipeline_name(&self) -> Option<&str> { None }
    
    /// 当前混合状态
    fn blend_state(&self) -> Option<BlendState> { None }
    
    /// 当前深度/模板状态
    fn depth_stencil_state(&self) -> Option<DepthStencilState> { None }
    
    /// 当前光栅化状态
    fn rasterizer_state(&self) -> Option<RasterizerState> { None }
    
    /// 当前多重采样状态
    fn multisample_state(&self) -> Option<MultisampleState> { None }
    
    // ══════════════════════════════════════════════════════════════
    // 7. 资源绑定（只读查询）
    // ══════════════════════════════════════════════════════════════
    
    /// 当前绑定的顶点缓冲
    fn bound_vertex_buffers(&self) -> &[BufferBinding] { &[] }
    
    /// 当前绑定的索引缓冲
    fn bound_index_buffer(&self) -> Option<BufferBinding> { None }
    
    /// 当前绑定的描述符集/资源堆
    fn bound_descriptor_sets(&self) -> &[DescriptorSetBinding] { &[] }
    
    /// 当前推送常量数据
    fn push_constants(&self) -> &[u8] { &[] }
    
    /// 当前绑定的采样器
    fn bound_samplers(&self) -> &[SamplerBinding] { &[] }
    
    /// 当前绑定的纹理/SRV
    fn bound_textures(&self) -> &[TextureBinding] { &[] }
    
    /// 当前绑定的 UAV/存储图像
    fn bound_storage_images(&self) -> &[StorageImageBinding] { &[] }
    
    /// 当前绑定的存储缓冲
    fn bound_storage_buffers(&self) -> &[StorageBufferBinding] { &[] }
    
    /// 当前绑定的常量缓冲
    fn bound_constant_buffers(&self) -> &[ConstantBufferBinding] { &[] }
    
    /// 当前绑定的光线追踪加速结构
    fn bound_acceleration_structures(&self) -> &[AccelerationStructureBinding] { &[] }
    
    // ══════════════════════════════════════════════════════════════
    // 7.1 光线追踪专用
    // ══════════════════════════════════════════════════════════════
    
    /// 当前光线追踪顶层加速结构
    fn top_level_acceleration_structure(&self) -> Option<AccelerationStructureHandle> { None }
    
    /// 当前光线追踪管线
    fn ray_tracing_pipeline(&self) -> Option<RayTracingPipelineHandle> { None }
    
    /// 当前着色器绑定表
    fn shader_binding_table(&self) -> Option<ShaderBindingTableHandle> { None }
    
    // ══════════════════════════════════════════════════════════════
    // 7.2 Mesh Shader / Task Shader 专用
    // ══════════════════════════════════════════════════════════════
    
    /// 当前 Mesh Shader 管线
    fn mesh_pipeline_id(&self) -> Option<PipelineId> { None }
    
    /// 当前 Task Shader 管线
    fn task_pipeline_id(&self) -> Option<PipelineId> { None }
    
    // ══════════════════════════════════════════════════════════════
    // 7.3 可变速率着色
    // ══════════════════════════════════════════════════════════════
    
    /// 可变速率着色状态
    fn vrs_state(&self) -> Option<VrsState> { None }
    
    /// 片元着色率附件
    fn shading_rate_image(&self) -> Option<TextureHandle> { None }
    
    // ══════════════════════════════════════════════════════════════
    // 8. 同步/围栏
    // ══════════════════════════════════════════════════════════════
    
    /// 当前帧的同步围栏（用于跨 API 等待）
    fn sync_fence(&self) -> Option<&dyn GpuFence> { None }
    
    /// 当前帧的等待围栏列表
    fn wait_fences(&self) -> &[&dyn GpuFence] { &[] }
    
    /// 当前帧的信号围栏列表
    fn signal_fences(&self) -> &[&dyn GpuFence] { &[] }
    
    /// 当前命令缓冲区的等待信号量
    fn wait_semaphores(&self) -> &[SemaphoreWait] { &[] }
    
    /// 当前命令缓冲区的信号信号量
    fn signal_semaphores(&self) -> &[SemaphoreSignal] { &[] }
    
    // ══════════════════════════════════════════════════════════════
    // 9. 调试/分析/工具链
    // ══════════════════════════════════════════════════════════════
    
    /// 当前命令缓冲区标签（用于 RenderDoc/PIX/NSight 调试）
    fn debug_label(&self) -> Option<&str> { None }
    
    /// 当前 pass 调试标签
    fn pass_debug_label(&self) -> Option<&str> { None }
    
    /// 当前命令缓冲区标签
    fn command_buffer_debug_label(&self) -> Option<&str> { None }
    
    /// GPU 时间戳（纳秒）
    fn gpu_timestamp_ns(&self) -> Option<u64> { None }
    
    /// GPU 计时查询句柄
    fn gpu_timer_query(&self) -> Option<TimerQueryHandle> { None }
    
    /// 当前 RenderDoc/PIX/NSight 捕获标记
    fn capture_marker(&self) -> Option<CaptureMarker> { None }
    
    /// 着色器调试信息
    fn shader_debug_info(&self) -> Option<ShaderDebugInfo> { None }
    
    // ══════════════════════════════════════════════════════════════
    // 10. 内存/资源管理
    // ══════════════════════════════════════════════════════════════
    
    /// 显存预算信息
    fn memory_budget(&self) -> Option<MemoryBudget> { None }
    
    /// 当前资源驻留状态
    fn resource_residency(&self) -> &[ResourceResidency] { &[] }
    
    /// 当前资源堆
    fn resource_heap(&self) -> Option<HeapHandle> { None }
    
    /// 瞬态堆（D3D12/Vulkan 专用）
    fn transient_heap(&self) -> Option<TransientHeapHandle> { None }
    
    // ══════════════════════════════════════════════════════════════
    // 11. 帧/资源版本控制
    // ══════════════════════════════════════════════════════════════
    
    /// 全局资源版本号（资源更新时递增）
    fn resource_version(&self) -> u64 { 0 }
    
    /// 当前渲染 pass 版本
    fn render_pass_version(&self) -> u64 { 0 }
    
    /// 管线缓存版本
    fn pipeline_cache_version(&self) -> u64 { 0 }
    
    /// 是否为首帧
    fn is_first_frame(&self) -> bool { false }
    
    /// 上一帧号
    fn prev_frame_index(&self) -> Option<u64> { None }
    
    // ═════════════════════════════════════════════════════════════
    // 12. 输入/交互（可选，用于 ImGui 等即时模式 GUI）
    // ═════════════════════════════════════════════════════════════
    
    /// 当前鼠标位置（像素坐标，相对渲染目标）
    fn mouse_position(&self) -> Option<[f32; 2]> { None }
    
    /// 当前鼠标按键状态
    fn mouse_buttons(&self) -> MouseButtonState { MouseButtonState::empty() }
    
    /// 当前键盘按键状态
    fn keyboard_state(&self) -> Option<KeyboardState> { None }
    
    /// 触摸点列表
    fn touches(&self) -> &[TouchPoint] { &[] }
    
    // ═════════════════════════════════════════════════════════════
    // 12.1 多视图/XR 专用
    // ══════════════════════════════════════════════════════════════
    
    /// XR 会话是否激活
    fn xr_session_active(&self) -> bool { false }
    
    /// XR 视图配置类型
    fn xr_view_configuration(&self) -> XrViewConfiguration { XrViewConfiguration::PrimaryMono }
    
    /// XR 当前视图的投影矩阵
    fn xr_view_projection_matrix(&self, view_index: u32) -> Option<[[f32; 4]; 4]> { None }
    
    /// XR 当前视图的视图矩阵
    fn xr_view_matrix(&self, view_index: u32) -> Option<[[f32; 4]; 4]> { None }
    
    /// XR 当前视图的注视点
    fn xr_gaze_point(&self) -> Option<[f32; 3]> { None }
    
    // ═════════════════════════════════════════════════════════════
    // 13. 自定义/扩展（pass 标识见 Category 1，帧版本见 Category 2/11，v1.18 实现去重）
    // ══════════════════════════════════════════════════════════════
    
    /// 应用自定义数据槽位（只读）
    fn user_data(&self) -> &dyn Any { &() }
    
    /// 应用自定义数据槽位（可写）
    fn user_data_mut(&mut self) -> &mut dyn Any { &mut () }
    
    /// 应用自定义数据槽位（类型安全获取）
    fn get_user_data<T: 'static>(&self) -> Option<&T> {
        self.user_data().downcast_ref::<T>()
    }
    
    /// 应用自定义数据槽位（类型安全获取可变）
    fn get_user_data_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.user_data_mut().downcast_mut::<T>()
    }
    
    /// 应用自定义数据槽位（插入/替换）
    fn set_user_data<T: Send + Sync + 'static>(&mut self, data: T) {
        *self.user_data_mut() = Box::new(data);
    }
    
    // ══════════════════════════════════════════════════════════════
    // 14. 类型擦除下转（核心）
    // ══════════════════════════════════════════════════════════════
    
    /// 下转到后端特定类型（如 `ID3D11DeviceContext`、`VkCommandBuffer`）。
    /// 
    /// **契约**：
    /// - 若当前后端为 D3D11，`downcast_ref::<D3D11HookContext>()` **必须返回 `Some`**
    /// - 若当前后端非 D3D11，**必须返回 `None`**
    /// - 后端实现者**必须**为自家类型正确实现此方法
    /// 
    /// **应用建议**：
    /// ```rust
    /// let d3d11 = ctx.downcast_ref::<D3D11HookContext>()
    ///     .expect("must be D3D11 on Windows backend");
    /// ```
    /// 
    /// **Debug 辅助**：`debug_assert_downcast::<T>(ctx)` 在开发期断言成功，Release 下零开销。
    fn downcast_ref<T: 'static>(&self) -> Option<&T>;
    
    fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T>;
}

/// 渲染 pass 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RenderPassType {
    Unknown,
    Main,
    Shadow,
    DepthPrepass,
    GBuffer,
    Lighting,
    PostProcess,
    UI,
    ShadowMap,
    Reflection,
    Custom,
}

/// 渲染目标信息
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderTargetInfo {
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub is_srgb: bool,
}

/// 深度/模板目标信息
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthStencilInfo {
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    pub sample_count: u32,
    pub has_depth: bool,
    pub has_stencil: bool,
}

/// 管线 ID（不透明句柄）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineId(pub u64);

/// 混合状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendState {
    pub enabled: bool,
    pub src_color: BlendFactor,
    pub dst_color: BlendFactor,
    pub src_alpha: BlendFactor,
    pub dst_alpha: BlendFactor,
    pub color_op: BlendOp,
    pub alpha_op: BlendOp,
    pub color_write_mask: ColorWriteMask,
}

/// 深度/模板状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthStencilState {
    pub depth_test_enabled: bool,
    pub depth_write_enabled: bool,
    pub depth_compare: CompareOp,
    pub stencil_test_enabled: bool,
    pub stencil_read_mask: u32,
    pub stencil_write_mask: u32,
    pub front_face: StencilFaceState,
    pub back_face: StencilFaceState,
}

/// 光栅化状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterizerState {
    pub cull_mode: CullMode,
    pub front_face: FrontFace,
    pub polygon_mode: PolygonMode,
    pub depth_bias: i32,
    pub depth_bias_clamp: f32,
    pub slope_scaled_depth_bias: f32,
    pub depth_clip_enabled: bool,
    pub scissor_enabled: bool,
    pub multisample_enabled: bool,
    pub antialiased_line_enabled: bool,
    pub conservative_rasterization: ConservativeRasterMode,
}

/// 多重采样状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MultisampleState {
    pub sample_count: u32,
    pub sample_mask: u32,
    pub alpha_to_coverage_enabled: bool,
    pub alpha_to_one_enabled: bool,
}

/// 缓冲绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferBinding {
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
    pub stride: u32,
}

/// 描述符集绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DescriptorSetBinding {
    pub set_index: u32,
    pub descriptor_set: DescriptorSetHandle,
    pub dynamic_offsets: &'static [u32],
}

/// 采样器绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerBinding {
    pub binding: u32,
    pub sampler: SamplerHandle,
}

/// 纹理/SRV 绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureBinding {
    pub binding: u32,
    pub texture_view: TextureViewHandle,
    pub layout: Option<TextureLayout>,
}

/// 存储图像/UAV 绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StorageImageBinding {
    pub binding: u32,
    pub image_view: ImageViewHandle,
    pub layout: Option<TextureLayout>,
}

/// 存储缓冲绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StorageBufferBinding {
    pub binding: u32,
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
}

/// 常量缓冲绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConstantBufferBinding {
    pub binding: u32,
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
}

/// 加速结构绑定
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccelerationStructureBinding {
    pub binding: u32,
    pub acceleration_structure: AccelerationStructureHandle,
}

/// 可变速率着色状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VrsState {
    pub enabled: bool,
    pub shading_rate: ShadingRate,
    pub combiners: [VrsCombiner; 2],
}

/// 着色率
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShadingRate {
    Rate1x1,
    Rate1x2,
    Rate2x1,
    Rate2x2,
    Rate2x4,
    Rate4x2,
    Rate4x4,
}

/// VRS 组合器
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VrsCombiner {
    Passthrough,
    Override,
    Min,
    Max,
    Sum,
}

/// 信号量等待
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemaphoreWait {
    pub semaphore: SemaphoreHandle,
    pub stage_mask: PipelineStageFlags,
    pub value: u64, // Timeline semaphore 用
}

/// 信号量信号
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemaphoreSignal {
    pub semaphore: SemaphoreHandle,
    pub value: u64, // Timeline semaphore 用
}

/// 管线阶段标志
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineStageFlags(pub u32);

impl PipelineStageFlags {
    pub const TOP_OF_PIPE: Self = Self(1 << 0);
    pub const DRAW_INDIRECT: Self = Self(1 << 1);
    pub const VERTEX_INPUT: Self = Self(1 << 2);
    pub const VERTEX_SHADER: Self = Self(1 << 3);
    pub const TESSELLATION_CONTROL_SHADER: Self = Self(1 << 4);
    pub const TESSELLATION_EVALUATION_SHADER: Self = Self(1 << 5);
    pub const GEOMETRY_SHADER: Self = Self(1 << 6);
    pub const FRAGMENT_SHADER: Self = Self(1 << 7);
    pub const EARLY_FRAGMENT_TESTS: Self = Self(1 << 8);
    pub const LATE_FRAGMENT_TESTS: Self = Self(1 << 9);
    pub const COLOR_ATTACHMENT_OUTPUT: Self = Self(1 << 10);
    pub const COMPUTE_SHADER: Self = Self(1 << 11);
    pub const TRANSFER: Self = Self(1 << 12);
    pub const BOTTOM_OF_PIPE: Self = Self(1 << 13);
    pub const HOST: Self = Self(1 << 14);
    pub const ALL_GRAPHICS: Self = Self(1 << 15);
    pub const ALL_COMMANDS: Self = Self(1 << 16);
}

/// 捕获标记
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureMarker {
    pub capture_id: u64,
    pub frame_number: u64,
    pub timestamp_ns: u64,
}

/// 着色器调试信息
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderDebugInfo {
    pub shader_stage: ShaderStage,
    pub entry_point: String,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
}

/// 着色器阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShaderStage {
    Vertex,
    TessellationControl,
    TessellationEvaluation,
    Geometry,
    Fragment,
    Compute,
    RayGeneration,
    AnyHit,
    ClosestHit,
    Miss,
    Intersection,
    Callable,
    Task,
    Mesh,
}

/// 内存预算
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryBudget {
    pub total_budget_bytes: u64,
    pub current_usage_bytes: u64,
    pub available_bytes: u64,
}

/// 资源驻留状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceResidency {
    pub resource: ResourceHandle,
    pub resident: bool,
    pub priority: f32,
}

/// 鼠标按键状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseButtonState(u32);

impl MouseButtonState {
    pub const LEFT: Self = Self(1 << 0);
    pub const RIGHT: Self = Self(1 << 1);
    pub const MIDDLE: Self = Self(1 << 2);
    pub const X1: Self = Self(1 << 3);
    pub const X2: Self = Self(1 << 4);
    pub fn empty() -> Self { Self(0) }
    pub fn contains(&self, other: Self) -> bool { (self.0 & other.0) == other.0 }
}

/// 键盘状态
#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardState {
    pub pressed_keys: Vec<KeyCode>,
    pub modifiers: KeyModifiers,
}

/// 按键码
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    Escape, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Backtick, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0, Minus, Equal, Backspace,
    Tab, Q, W, E, R, T, Y, U, I, O, P, LeftBracket, RightBracket, Backslash,
    CapsLock, A, S, D, F, G, H, J, K, L, Semicolon, Quote, Enter,
    LeftShift, Z, X, C, V, B, N, M, Comma, Period, Slash, RightShift,
    LeftCtrl, LeftSuper, LeftAlt, Space, RightAlt, RightSuper, Menu, RightCtrl,
    Up, Down, Left, Right,
    Insert, Delete, Home, End, PageUp, PageDown,
    NumLock, NumpadDivide, NumpadMultiply, NumpadSubtract,
    Numpad7, Numpad8, Numpad9, NumpadAdd,
    Numpad4, Numpad5, Numpad6,
    Numpad1, Numpad2, Numpad3, NumpadEnter,
    Numpad0, NumpadDecimal,
    PrintScreen, ScrollLock, Pause,
}

/// 按键修饰符
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyModifiers(u32);

impl KeyModifiers {
    pub const SHIFT: Self = Self(1 << 0);
    pub const CTRL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const SUPER: Self = Self(1 << 3);
    pub fn contains(&self, other: Self) -> bool { (self.0 & other.0) == other.0 }
}

/// 触摸点
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPoint {
    pub id: u64,
    pub position: [f32; 2],
    pub pressure: f32,
    pub radius: [f32; 2],
}

/// XR 视图配置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum XrViewConfiguration {
    PrimaryMono,
    PrimaryStereo,
    SecondaryMono,
}

/// 不透明句柄类型（占位，实际在后端定义）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorSetHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureViewHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageViewHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccelerationStructureHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RayTracingPipelineHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderBindingTableHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemaphoreHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerQueryHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CaptureMarker(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeapHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransientHeapHandle(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureLayout(pub u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderStageHandle(pub u32);

/// 混合因子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlendFactor {
    Zero, One, SrcColor, OneMinusSrcColor, DstColor, OneMinusDstColor,
    SrcAlpha, OneMinusSrcAlpha, DstAlpha, OneMinusDstAlpha,
    ConstantColor, OneMinusConstantColor, ConstantAlpha, OneMinusConstantAlpha,
    SrcAlphaSaturate, Src1Color, OneMinusSrc1Color, Src1Alpha, OneMinusSrc1Alpha,
}

/// 混合操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlendOp {
    Add, Subtract, ReverseSubtract, Min, Max,
}

/// 颜色写入掩码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorWriteMask(u8);

impl ColorWriteMask {
    pub const RED: Self = Self(1 << 0);
    pub const GREEN: Self = Self(1 << 1);
    pub const BLUE: Self = Self(1 << 2);
    pub const ALPHA: Self = Self(1 << 3);
    pub const ALL: Self = Self(0xF);
    pub const NONE: Self = Self(0);
}

/// 比较操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompareOp {
    Never, Less, Equal, LessOrEqual, Greater, NotEqual, GreaterOrEqual, Always,
}

/// 剔除模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CullMode {
    None, Front, Back, FrontAndBack,
}

/// 正面朝向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrontFace {
    CounterClockwise, Clockwise,
}

/// 多边形模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolygonMode {
    Fill, Line, Point,
}

/// 保守光栅化模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConservativeRasterMode {
    Off, On, PostDepth,
}

/// 模板面状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StencilFaceState {
    pub fail_op: StencilOp,
    pub pass_op: StencilOp,
    pub depth_fail_op: StencilOp,
    pub compare_op: CompareOp,
    pub compare_mask: u32,
    pub write_mask: u32,
    pub reference: u32,
}

/// 模板操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StencilOp {
    Keep, Zero, Replace, IncrementAndClamp, DecrementAndClamp,
    Invert, IncrementAndWrap, DecrementAndWrap,
}

/// 纹理布局
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextureLayout {
    Undefined,
    General,
    ColorAttachmentOptimal,
    DepthStencilAttachmentOptimal,
    DepthStencilReadOnlyOptimal,
    ShaderReadOnlyOptimal,
    TransferSrcOptimal,
    TransferDstOptimal,
    Preinitialized,
    PresentSrc,
    SharedPresent,
    VideoDecodeDst,
    VideoEncodeSrc,
    AttachmentFeedbackLoopOptimal,
}

/// 混合常量
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendConstants(pub [f32; 4]);
```

### 4.2 RenderExtension / Hook 注册

```rust
// zephyra-render-ext/src/registration.rs (续)

use std::any::Any;
use std::borrow::Cow;

/// Hook 可获得的渲染源信息（每次调用借用，hooks 长期存储）
pub struct RenderSource<'a> {
    pub resource_key: Option<&'a str>,
    payload: Option<&'a dyn Any>,
}

impl<'a> RenderSource<'a> {
    pub fn new(resource_key: Option<&'a str>, payload: Option<&'a dyn Any>) -> Self {
        Self { resource_key, payload }
    }
    pub fn resource_key(&self) -> Option<&str> { self.resource_key }
    pub fn payload<T: 'static>(&self) -> Option<&T> { self.payload.and_then(|p| p.downcast_ref()) }
}

/// 扩展绘制回调（v1.18/S2）：HRTB，允许 `RenderSource` 借用单次调用。
pub type DrawHook = dyn for<'a> FnMut(&mut dyn HookContext, Rect, &RenderSource<'a>) + Send + 'static;

/// 扩展注册接口（后端实现）。
/// **v1.18/P1-2**：移除 `HookCtx` 关联类型；类型安全由 `HookRegistry::d3d11/d3d12/vulkan` 泛型方法 + 内部适配器保证。
pub trait RenderExtension {
    fn register_hook(&mut self, kind: Cow<'static, str>, hook: Box<DrawHook>);
    fn unregister_hook(&mut self, kind: &str) -> bool;
    fn has_hook(&self, kind: &str) -> bool;
    fn registered_kinds(&self) -> Vec<Cow<'static, str>>;
}

/// 扩展工厂：创建扩展实例并自动注册钩子。
pub trait ExtensionFactory: Send + Sync {
    fn create_and_register(
        &self,
        register: &mut dyn FnMut(Cow<'static, str>, Box<DrawHook>),
    ) -> Box<dyn Any + Send + Sync>;
}
```

---

## 5. Capability 能力声明矩阵

| 能力项 | D3D11 | D3D12 | Vulkan | 备注 |
|--------|-------|-------|--------|------|
| `CustomBasic` | ✅ | ✅ | ✅ | 基础自绘 |
| `CustomOffscreen` | ❌ | ❌ | ❌ | Phase 2 实现 |
| `AttachmentSwitch` | ❌ | ❌ | ❌ | 不支持切 pass |
| `Readback` | ✅ | ✅ | ✅ | CPU 读回 |
| `DeviceRebuild` | ❌ | ✅ | ✅ | D3D11 无显式重建路径 |
| `Screenshot` | ✅ | ✅ | ✅ | BMP 导出 |

**应用查询**：`HostWindow::capabilities()` 返回 `BackendCapabilities`

> **硬性约束**：**所有后端必须使用同一物理适配器（LUID 完全一致）**。**框架不校验、不枚举、不选**；应用创建 wgpu Adapter 后将其 LUID 传给框架（`HostWindow::builder().force_adapter_luid(luid)`），框架在该 LUID 上创建所有后端设备。若应用传错 LUID 导致跨适配器，属于应用配置错误，框架不负责兜底。

---

## 6. Lifecycle 生命周期集成点

| 回调 | D3D11 (zephyra-winit) | D3D12 (zephyra-winit-d3d12) | Vulkan (zephyra-vulkan) |
|------|----------------------|----------------------------|------------------------|
| `on_prepare` | `Renderer::render()` 入口 | `Renderer::record_with()` 入口 | `Renderer::prepare()` |
| `on_resize` | `Surface::resize()` 后 | `WindowTargets::resize()` 后 | `Surface::recreate_swapchain()` 后 |
| `on_device_lost` | `Present` 失败 `DXGI_ERROR_DEVICE_REMOVED` | `ExecuteCommandLists` 失败 / `GetDeviceRemovedReason` | `vkQueueSubmit` 返回 `VK_ERROR_DEVICE_LOST` |
| `on_device_restored` | 重建 swapchain/device 后 | `Shared::recreate_device()` 后 | `Device::wait_idle` 重建后 |

---

### 6.1 多窗口 Hook 注册模式

**设计决策**：同时支持「统一表 + 窗口键」与「每窗口独立表（分表）」，由后端按需选择，应用统一 API 无感。

#### 统一 API（zephyra-ext）

```rust
// zephyra-ext/src/hook_registry.rs

pub struct HookRegistry<'a> {
    ctx: &'a mut ExtContext,
    default_window: Option<WindowId>,
}

impl<'a> HookRegistry<'a> {
    // ===== 模式 1：统一表 + 窗口键（默认、简单场景） =====
    
    /// 注册到默认/当前窗口（单窗口应用零成本）
    pub fn d3d11<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError>
    where F: FnMut(&D3D11HookContext, Rect, &RenderSource) + Send + 'static { ... }
    
    /// 显式指定窗口（多窗口共享 hook 表）
    pub fn d3d11_for_window<F>(&mut self, window: WindowId, kind: &str, hook: F) 
        -> Result<HookHandle, ExtError> { ... }

    // ===== 模式 2：每窗口独立表（分表、复杂场景）=====
    
    /// 获取指定窗口的独立 HookRegistry（D3D12/Vulkan 复杂场景）
    pub fn for_window(&mut self, window: WindowId) -> WindowHookRegistry<'_> {
        WindowHookRegistry { ctx: self.ctx, window }
    }
}

/// 窗口级独立注册表（持有窗口键，所有方法自动带窗口上下文）
pub struct WindowHookRegistry<'a> {
    ctx: &'a mut ExtContext,
    window: WindowId,
}

impl<'a> WindowHookRegistry<'a> {
    pub fn d3d11<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError> { ... }
    pub fn d3d12<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError> { ... }
    pub fn vulkan<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError> { ... }
    pub fn generic<F>(&mut self, kind: &str, hook: F) -> Result<HookHandle, ExtError> { ... }
}
```

#### 后端实现策略

| 后端 | 内部存储 | `HookRegistry::d3d11()` | `WindowHookRegistry::d3d12()` |
|------|---------|------------------------|------------------------------|
| **D3D11** | `HashMap<WindowId, HashMap<Kind, Hook>>` | 写入 `default_window` 或指定窗口 | 委托到同一存储，按窗口隔离 |
| **D3D12** | 每窗口独立 `HashMap<Kind, Hook>` | **报错/panic**（D3D12 必须分表） | 写入该窗口独立表 |
| **Vulkan** | 可配置：统一表或分表 | 写入统一表 | 写入分表 |

#### 应用使用示例

```rust
// 场景 1：单窗口 / 简单多窗口（共享 hook 表）
let ext = host.ext_context();
ext.ctx().hooks().d3d11("debug-overlay", |ctx, rect, src| { ... })?;           
ext.ctx().hooks().d3d11_for_window(preview_win, "debug-overlay", |ctx, ...| { ... })?;

// 场景 2：复杂多窗口（每窗口完全不同渲染逻辑）
let main_win = host.create_window("Main", ...)?;
let preview_win = host.create_window("Preview", ...)?;

main_win.ext_ctx().hooks().for_window(main_win.id()).d3d12("main-3d", |ctx, rect, src| {
    ctx.command_queue().execute(&[cmd_list]);
})?;

preview_win.ext_ctx().hooks().for_window(preview_win.id()).d3d12("preview-2d", |ctx, rect, src| {
    ctx.command_queue().execute(&[ui_cmd_list]);
})?;
```

**关键点**：
1. API 统一：应用只用 `hooks()` 和 `for_window()`，不关心后端怎么存
2. D3D12 强制分表：`hooks().d3d12()` 直接报错，逼迫用 `for_window()`
3. D3D11/Vulkan 灵活：两种模式都支持，按需选
4. 零成本：单窗口应用完全感觉不到 `WindowId` 存在

---

## 7. 错误处理策略

```rust
pub enum ImportError {
    InvalidHandle(String),
    FormatMismatch { expected, actual },
    SizeMismatch { expected, actual },
    UnsupportedSource,        // 后端不支持该 source
    DeviceLost,               // 可重试：设备丢失
    Backend(String),          // 其他
}

impl ImportError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, ImportError::DeviceLost)
    }
}
```

**失败策略**：跳过该纹理 + warn + 透明占位，不崩主进程。应用可通过 `is_retryable()` 判断是否重试。

> **注意**：框架**不再产生 `AdapterMismatch` 错误**。适配器由应用（wgpu）决定，框架跟随。若应用传错 LUID 导致跨适配器共享失败，属于应用配置错误，表现为 `InvalidHandle` 或 `Backend` 错误。

---

### 7.1 NT Handle 错误映射（Windows 后端专用）

**作用**：把 `ID3D11Device::OpenSharedResource` / `ID3D12Device::OpenSharedHandle` 返回的 `HRESULT` 翻译成跨平台统一的 `ImportError`，应用零 Windows 依赖、可判断重试。

#### HRESULT 映射表

| HRESULT | 十六进制 | 含义 | 映射到 `ImportError` | `is_retryable()` |
|---------|---------|------|---------------------|------------------|
| `E_INVALIDARG` | `0x80070057` | Handle 无效/格式不对/尺寸格式不匹配 | `InvalidHandle("...")` | ❌ |
| `E_ACCESSDENIED` | `0x80070005` | 无权限（跨进程未设置共享、安全描述符拦截） | `InvalidHandle("access denied")` | ❌ |
| `DXGI_ERROR_INVALID_CALL` | `0x887A0001` | 参数错、设备状态不对 | `InvalidHandle` / `Backend` | ❌ |
| `DXGI_ERROR_DEVICE_REMOVED` | `0x887A0005` | **GPU 挂了/驱动崩了/TDR** | `DeviceLost` | ✅ |
| `DXGI_ERROR_DEVICE_RESET` | `0x887A0007` | 设备重置（模式切换、驱动更新） | `DeviceLost` | ✅ |
| `E_OUTOFMEMORY` | `0x8007000E` | 显存/内存不足 | `Backend("OOM")` | ❌ |
| `DXGI_ERROR_UNSUPPORTED` | `0x887A0003` | 格式/功能不支持（如旧 GPU 不支持共享） | `UnsupportedSource` | ❌ |
| `STATUS_INVALID_HANDLE` | `0xC0000008` | Handle 已关闭/非共享 Handle | `InvalidHandle` | ❌ |

#### 映射实现（zephyra-winit / zephyra-winit-d3d12 共用）

```rust
// zephyra-render-ext/src/error.rs (共享映射函数)

/// 把 Windows HRESULT 映射为跨平台 ImportError
pub fn map_hresult_to_import_error(hr: HRESULT) -> ImportError {
    match hr.0 as u32 {
        0x80070057 => ImportError::InvalidHandle("E_INVALIDARG".into()),       // E_INVALIDARG
        0x80070005 => ImportError::InvalidHandle("E_ACCESSDENIED".into()),     // E_ACCESSDENIED
        0x887A0005 => ImportError::DeviceLost,                                  // DEVICE_REMOVED
        0x887A0007 => ImportError::DeviceLost,                                  // DEVICE_RESET
        0x887A0003 => ImportError::UnsupportedSource,                           // UNSUPPORTED
        0x8007000E => ImportError::Backend("OOM".into()),                       // OUTOFMEMORY
        0xC0000008 => ImportError::InvalidHandle("STATUS_INVALID_HANDLE".into()),
        _ => ImportError::Backend(format!("HRESULT 0x{:08X}", hr.0)),
    }
}
```

#### 后端调用示例

```rust
// zephyra-winit/src/external_texture.rs (D3D11)
impl ExternalTextureImporter for D3D11Renderer {
    fn import_nt_handle(&mut self, desc: NtHandleDesc) -> Result<ImportedTexture, ImportError> {
        let result = unsafe { self.device.open_shared_resource::<ID3D11Texture2D>(desc.handle) };
        match result {
            Ok(texture) => Ok(ImportedTexture::new(texture)),
            Err(e) => Err(map_hresult_to_import_error(e.code())), // 统一映射
        }
    }
}

// zephyra-winit-d3d12/src/external_texture.rs (D3D12) - 同理
impl ExternalTextureImporter for D3D12Renderer {
    fn import_nt_handle(&mut self, desc: NtHandleDesc) -> Result<ImportedTexture, ImportError> {
        let result = unsafe { self.device.open_shared_handle(desc.handle) };
        match result {
            Ok(resource) => Ok(...),
            Err(e) => Err(map_hresult_to_import_error(e.code())), // 同一个映射函数
        }
    }
}
```

**为什么要映射**：
- 应用零 `windows` crate 依赖，只依赖 `zephyra-ext`
- 跨平台统一错误处理（Linux Vulkan 也有对应 `VkResult` 映射）
- `is_retryable()` 明确告诉应用：`DeviceLost` 可重试，`InvalidHandle` 不可
- 统一日志、遥测、上报格式

---

## 8. 老旧设备回退兼容策略（参考 Chromium 设计）

> **核心参考**：Chromium `gpu/config/` 目录下的 `GpuControlList`、`GpuBlocklist`、`GpuDriverBugList`、两阶段信息收集、有序回退链、`fallback_gr_context_types` 机制。

### 8.1 设计原则（源自 Chromium 实践）

| 原则 | Chromium 做法 | Zephyra 对应 |
|------|--------------|-------------|
| **两阶段探测** | `CollectBasicGraphicsInfo()`(无上下文，快、安全) → `CollectContextGraphicsInfo()`(建上下文，全、可能崩) | Phase 1：读 wgpu 传入的 LUID + 驱动版本；Phase 2：实测建设备/提交一帧 |
| **数据驱动 Blocklist** | JSON 列表：`vendor_id`/`device_id`/`driver_version`/`gl_vendor`/`gl_renderer`/OS/异常允许名单 | `device_blocklist.json` + `driver_workaround.json`，构建时嵌入二进制 |
| **特性级启用/禁用** | 每个特性独立状态：`Enabled/Blocklisted/Disabled/Software` | `BackendCapabilities` 位图 + `FeatureStatus` 枚举 |
| **有序回退链** | `fallback_gr_context_types`：GraphiteDawn → GraphiteVulkan → Vulkan → GL → SwiftShader | D3D12 → D3D11 → Vulkan → CPU，每级仅降不升 |
| **驱动 Bug Workaround 独立** | 单独列表 `gpu_driver_bug_list.json`，启用 `disable_d3d11` 等 workaround | `DriverWorkaround` bitmask，应用于后端初始化 |
| **SwiftShader 兜底** | 硬件全挂时自动切 SwiftShader (软件光栅) | 最后一级 `Backend::Cpu` (memcpy + staging buffer) |
| **可观测/可调试** | `chrome://gpu` 显示 applied entries、feature status、workarounds | `ExtContext::dump_diagnostics()` + 环境变量强制回退 |
| **适配器选择** | 浏览器进程枚举、打分、选优 | **框架不选适配器，跟随 wgpu（应用传入 LUID）** |

### 8.2 核心数据结构

```rust
// zephyra-ext/src/capability.rs

/// 后端特性状态（参考 Chromium GpuFeatureStatus）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureStatus {
    Enabled,        // 正常启用
    Blocklisted,    // 被 blocklist 禁用
    Disabled,       // 显式禁用/不支持
    Software,       // 回退到软件实现
}

/// 设备能力评分（参考 Chromium 启发式评分）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendScore {
    pub timeline_fence: bool,
    pub shared_texture: bool,
    pub d3d11_feature_level: Option<D3D11FeatureLevel>, // 9_1 ~ 12_2
    pub vulkan_version: Option<VulkanVersion>,          // 1.0 ~ 1.3
    pub upload_bandwidth_mbps: f32,    // 实测上传带宽
    pub submit_latency_ms: f32,        // 实测提交延迟
    pub tdr_count: u32,                // 历史 TDR 次数
    pub driver_bug_workarounds: u32,   // 已启用 workaround 位图
    pub score: f32,                    // 综合分
}

/// 设备缓存（参考 Chromium GPUInfo 持久化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCapabilityCache {
    pub timestamp: u64,
    pub adapter_luid: Option<LUID>,           // Windows: LUID 唯一标识适配器
    pub vendor_id: u32,
    pub device_id: u32,
    pub revision: u32,
    pub driver_version: String,               // 驱动版本字符串
    pub driver_date: Option<String>,          // 驱动日期
    pub backend_scores: HashMap<Backend, BackendScore>,
    pub selected_backend: Backend,
    pub fallback_chain: Vec<Backend>,         // 降级顺序
    pub applied_blocklist_entries: Vec<u32>,  // 匹配的 blocklist 条目 ID
    pub applied_workarounds: Vec<u32>,        // 启用的 workaround ID
}

/// 回退策略（应用可控）
#[derive(Debug, Clone)]
pub enum FallbackPolicy {
    Automatic,                                // 框架自动（默认）
    Disable(Backend),                         // 禁用某后端
    Force(Backend),                           // 强制某后端
    Custom(Box<dyn Fn(&DeviceCaps) -> Backend + Send + Sync>), // 应用自定义
}
```

### 8.3 两阶段探测流程

```
启动
  │
  ├─► 应用创建 wgpu Instance/Adapter/Device
  │     └─ 拿到 wgpu 选定适配器的 LUID
  │
  ├─► 应用传 LUID 给框架：HostWindow::builder().force_adapter_luid(wgpu_luid)
  │
  ├─► Phase 1: 基础信息收集 (CollectBasicGraphicsInfo 等价)
  │     ├─ 框架用传入 LUID 打开对应适配器（不枚举、不打分、不选）
  │     ├─ 读取 vendor_id / device_id / revision / driver_version
  │     ├─ 查询 blocklist (vendor_id + device_id + driver_version 匹配)
  │     ├─ 查询 workaround 列表
  │     └─ 计算每个后端初步分数（仅用于能力上报，不做选择）
  │           D3D12: CheckFeatureSupport(D3D12_FEATURE_D3D12_OPTIONS) + Timeline Fence 支持
  │           D3D11: CheckFeatureSupport(D3D11_FEATURE_D3D11_OPTIONS) + 共享纹理支持
  │           Vulkan: vkGetPhysicalDeviceProperties/Features2 + Timeline Semaphore
  │           CPU:   总是可用
  │
  ├─► Phase 2: 实测验证 (CollectContextGraphicsInfo 等价)
  │     ├─ 在同一 LUID 上创建所有后端设备/队列/交换链
  │     ├─ 提交 1 帧空命令缓冲，测 submit_latency_ms
  │     ├─ 上传 4MB 纹理，测 upload_bandwidth_mbps
  │     ├─ 验证共享纹理导入/导出 (NT Handle / VkImage)
  │     ├─ 验证 Timeline Fence/Semaphore Signal+Wait
  │     └─ 通过 → 写缓存；失败 → 标记该后端 Blocklisted，进入下一级回退
  │
  └─► 运行时监控
        ├─ 连续 3 帧 submit > 16ms → 触发降级
        ├─ DXGI_ERROR_DEVICE_REMOVED / VK_ERROR_DEVICE_LOST → 触发降级
        ├─ 显存 OOM → 触发降级
        └─ 降级路径：D3D12 → D3D11 → Vulkan → CPU
              （每级只降不升，除非应用显式 request_upgrade()）
```

> **关键变更**：框架**不再枚举适配器、不做 LUID 校验、不存 AdapterMismatch**。适配器唯一由应用（wgpu）决定，框架只负责在该适配器上创建所有后端设备。

### 8.4 Blocklist 与 Workaround 文件格式（构建时嵌入）

```json
// device_blocklist.json (参考 Chromium software_rendering_list.json)
{
  "name": "zephyra device blocklist",
  "version": 1,
  "entries": [
    {
      "id": 1,
      "description": "NVIDIA GeForce 600/700 series D3D12 driver crash",
      "cr_bugs": [12345],
      "os": "win",
      "vendor_id": "0x10de",
      "device_id": ["0x0FC0", "0x0FC1", "0x0FC2", "0x0FC6", "0x0FC8", "0x0FC9", "0x0FCA", "0x0FCB", "0x0FCC", "0x0FCD", "0x0FCE", "0x0FCF", "0x1040", "0x1042", "0x1048", "0x1049", "0x104A", "0x104B", "0x104C", "0x104D", "0x104E", "0x104F", "0x1050", "0x1051", "0x1052", "0x1054", "0x1055", "0x1056", "0x1057", "0x1058", "0x1059", "0x105A", "0x105B", "0x105C", "0x105D", "0x105E", "0x105F", "0x1070", "0x1071", "0x1072", "0x1073", "0x1074", "0x1075", "0x1076", "0x1077", "0x1078", "0x1079", "0x107A", "0x107B", "0x107C", "0x107D", "0x107E", "0x107F"],
      "backend": "d3d12",
      "features": ["disable_d3d12"]
    },
    {
      "id": 2,
      "description": "Intel HD 4000/5000 Vulkan driver missing timeline semaphore",
      "cr_bugs": [12346],
      "os": "linux",
      "vendor_id": "0x8086",
      "device_id": ["0x0162", "0x0166", "0x016A", "0x0412", "0x0416", "0x041E", "0x0A16", "0x0A26", "0x0A2E", "0x0D26", "0x0D2E", "0x1612", "0x1616", "0x161E", "0x1622", "0x1626", "0x162B", "0x162E"],
      "driver_version": { "op": "<", "value": "1.3.0" },
      "backend": "vulkan",
      "features": ["disable_vulkan_timeline"]
    }
  ]
}
```

```json
// driver_workaround.json (参考 Chromium gpu_driver_bug_list.json)
{
  "name": "zephyra driver workaround list",
  "version": 1,
  "entries": [
    {
      "id": 1,
      "description": "D3D11 device reset on context loss in sandbox",
      "os": "win",
      "workaround": "exit_on_context_lost"
    },
    {
      "id": 2,
      "description": "Older NVIDIA drivers D3D11 feature level 11_1 broken",
      "os": "win",
      "vendor_id": "0x10de",
      "driver_version": { "op": "<", "value": "26.21.14.4166" },
      "workaround": "force_d3d11_feature_level_11_0"
    },
    {
      "id": 3,
      "description": "Mesa drivers partial swap broken",
      "os": "linux",
      "gl_vendor": "Mesa.*",
      "workaround": "disable_post_sub_buffers"
    }
  ]
}
```

### 8.5 环境变量速查表（运维/调试用，参考 Chromium 命令行开关）

| 环境变量 | 值 | 作用 |
|---------|----|------|
| `ZEPHYRA_FORCE_CPU` | `1` | 强制 CPU 上传，跳过所有 GPU 路径 |
| `ZEPHYRA_DISABLE_D3D12` | `1` | 禁用 D3D12 后端 |
| `ZEPHYRA_DISABLE_VULKAN` | `1` | 禁用 Vulkan 后端 |
| `ZEPHYRA_FORCE_D3D11` | `1` | 强制 D3D11（Windows） |
| `ZEPHYRA_CPU_THRESHOLD_KB` | `1024` | CPU 兜底阈值（KB），超过走 CPU |
| `ZEPHYRA_SKIP_CACHE` | `1` | 忽略设备缓存，强制重探测 |
| `ZEPHYRA_DEBUG_FALLBACK` | `1` | 打印每次降级决策日志 |
| `ZEPHYRA_IGNORE_BLOCKLIST` | `1` | 忽略 blocklist（测试用） |
| `ZEPHYRA_FORCE_BACKEND` | `d3d12\|d3d11\|vulkan\|cpu` | 强制指定后端 |

### 8.6 Phase 落地计划

| 阶段 | 做什么 |
|------|--------|
| **Phase 1** | 在 `zephyra-ext` 定义 `DeviceCapabilityCache`、`FallbackPolicy`、`BackendScore`、`FeatureStatus`、环境变量读取、blocklist/workaround 解析器；**不实现探测逻辑**，只定契约 |
| **Phase 2 (D3D11)** | 实现 D3D11 探测：`ID3D11Device::CheckFeatureSupport` + 实测上传 1 帧 + 共享纹理导入测试 |
| **Phase 3 (D3D12)** | 实现 D3D12 探测：`ID3D12Device::CheckFeatureSupport` + Timeline Fence 测试 + 共享句柄导入测试 |
| **Phase 4 (Vulkan)** | 实现 Vulkan 探测：`vkGetPhysicalDeviceProperties/Features2` + Timeline Semaphore + VkImage 导入测试 + 黑名单匹配 |
| **Phase 5** | 串联运行时监控、降级触发、缓存持久化（`%LOCALAPPDATA%/Zephyra/device_cache.json`）、测试用例 |

---

## 9. 同步原语所有权与生命周期

### 8.1 核心决策：Timeline Semaphore 共享（方案 A）

**跨 API 同步统一采用 Timeline Semaphore 共享句柄**，而非 Binary Semaphore 转换。

| 场景 | 同步原理 | 共享句柄类型 |
|------|---------|-------------|
| **wgpu → D3D11** | wgpu Fence → 导出 NT Handle → D3D11 打开共享 Fence | `ID3D12Fence` NT Handle |
| **D3D12 → D3D11** | D3D12 Fence → 导出 NT Handle → D3D11 打开共享 Fence | `ID3D12Fence` NT Handle |
| **Vulkan → Vulkan** | `VkSemaphore` (Timeline) → 导出 FD / NT Handle → 另一设备导入 | `VkSemaphore` FD (Linux) / NT Handle (Windows) |
| **Vulkan ↔ D3D12** | 双向导出/导入 Timeline Fence/Semaphore | NT Handle (Windows 同设备/跨 API) |

**优势**：
- 单一同步对象，双方直接 GPU 侧等待，无 CPU 同步点
- 单调 `u64` 天然支持多帧在飞行
- 跨进程/跨 API 透明传递

> **硬性约束**：**所有队列（wgpu/D3D12/Vulkan）必须在同一物理适配器（LUID 一致）上创建**。跨适配器同步不被支持。**框架不校验**；应用负责确保 wgpu 与框架后端使用同一 LUID。

### 8.2 所有权与生命周期表

| 场景 | 谁创建 | 谁 Signal | 谁 Wait | 生命周期 |
|------|--------|-----------|---------|----------|
| wgpu→D3D11 | 应用创建 Timeline Fence | wgpu Queue Signal | D3D11 提交前 Wait | 单帧，帧结束释放 |
| D3D12→D3D11 | 应用创建 Timeline Fence | D3D12 Queue Signal | D3D11 提交前 Wait | 单帧 |
| Vulkan→Vulkan | 应用创建 Timeline Semaphore | 应用 Queue Signal | Vulkan 提交前 Wait | 单帧 |
| **框架角色** | 只负责 `wait()`，不创建不 signal | | | |

### 8.3 统一门面 API（zephyra-ext）

```rust
// zephyra-ext/src/sync.rs

/// GPU 围栏统一抽象（应用创建，框架只 wait）
pub trait GpuFence: Send + Sync {
    /// 当前已 signal 的值（Timeline 语义）
    fn signaled_value(&self) -> u64;
    
    /// 阻塞等待直到 value（GPU 侧等待，零 CPU 自旋）
    fn wait(&self, value: u64) -> Result<(), SyncError>;
    
    /// 导出跨 API/进程句柄（NT Handle / FD）
    fn export_handle(&self) -> Result<FenceHandle, SyncError>;
    
    /// 克隆句柄（用于多消费者等待同一生产者）
    fn clone_handle(&self) -> Box<dyn GpuFence>;
}

/// 围栏句柄（不透明，内部含 NT Handle / FD / VkSemaphore）
pub struct FenceHandle { /* 平台特定 */ }

/// 统一同步输入（ExternalTextureBuilder::sync_auto() 用）
pub enum SyncInput {
    Auto,                                    // 框架自动选最优：Timeline Fence > Binary > CPU
    Fence(Box<dyn GpuFence>),                // 显式 Timeline Fence（推荐）
    BinarySemaphore(Box<dyn BinarySemaphore>), // 兼容老设备（无 Timeline 支持）
    None,                                    // 无同步（应用自行保证）
}

/// 二进制信号量（兼容层，不推荐新代码用）
pub trait BinarySemaphore: Send + Sync {
    fn wait(&self) -> Result<(), SyncError>;
    fn signal(&self) -> Result<(), SyncError>;
}
```

**`SyncInput::Auto` 后端选择策略**：
```rust
fn choose_sync(backend: &BackendCapabilities) -> SyncInput {
    if backend.supports_timeline_fence() {
        SyncInput::Auto  // 运行时创建 Timeline Fence
    } else if backend.supports_binary_semaphore() {
        SyncInput::BinarySemaphore(...)
    } else {
        SyncInput::None  // CPU fallback，记录警告
    }
}
```

---

## 9.5 调试/校验模式

> **核心原则**：**Debug 构建开启、Release 构建强制关闭、零运行时开销**。

### 9.5.1 环境变量控制

| 环境变量 | Debug 默认 | Release 行为 | 作用 |
|---------|-----------|-------------|------|
| `ZEPHYRA_DEBUG_SYNC` | `1` | **强制忽略/报错** | 打印每帧 Fence/Semaphore Signal/Wait 详情（值、队列、耗时） |
| `ZEPHYRA_DEBUG_TEXTURE` | `1` | **强制忽略/报错** | 记录 ExternalTexture 导入/更新/释放全生命周期 |
| `ZEPHYRA_DEBUG_HOOK` | `1` | **强制忽略/报错** | 打印 Hook 注册/调用/耗时/panic 捕获 |
| `ZEPHYRA_DEBUG_FALLBACK` | `1` | **强制忽略/报错** | 打印回退链决策、Blocklist 匹配、Workaround 启用 |
| `ZEPHYRA_VALIDATION` | `1` | **强制忽略/报错** | 启用后端原生验证层（D3D11 Debug Layer / Vulkan Validation Layers / D3D12 Debug Device） |

### 9.5.2 编译期保证

```rust
// zephyra-ext/src/debug.rs

/// 调试功能入口，**Release 构建直接编译为空函数/零大小类型**
#[cfg(debug_assertions)]
pub mod debug {
    use std::sync::atomic::{AtomicBool, Ordering};
    
    // 运行时可关闭（默认读环境变量）
    static SYNC_ENABLED: AtomicBool = AtomicBool::new(true);
    static TEXTURE_ENABLED: AtomicBool = AtomicBool::new(true);
    static HOOK_ENABLED: AtomicBool = AtomicBool::new(true);
    static FALLBACK_ENABLED: AtomicBool = AtomicBool::new(true);
    
    pub fn init_from_env() {
        SYNC_ENABLED.store(std::env::var("ZEPHYRA_DEBUG_SYNC").is_ok(), Ordering::Relaxed);
        TEXTURE_ENABLED.store(std::env::var("ZEPHYRA_DEBUG_TEXTURE").is_ok(), Ordering::Relaxed);
        HOOK_ENABLED.store(std::env::var("ZEPHYRA_DEBUG_HOOK").is_ok(), Ordering::Relaxed);
        FALLBACK_ENABLED.store(std::env::var("ZEPHYRA_DEBUG_FALLBACK").is_ok(), Ordering::Relaxed);
    }
    
    #[inline] pub fn sync_enabled() -> bool { SYNC_ENABLED.load(Ordering::Relaxed) }
    #[inline] pub fn texture_enabled() -> bool { TEXTURE_ENABLED.load(Ordering::Relaxed) }
    #[inline] pub fn hook_enabled() -> bool { HOOK_ENABLED.load(Ordering::Relaxed) }
    #[inline] pub fn fallback_enabled() -> bool { FALLBACK_ENABLED.load(Ordering::Relaxed) }
    
    /// 同步调试打印（零开销：Release 下直接内联为空）
    #[inline] pub fn log_sync(msg: &str) {
        if sync_enabled() { eprintln!("[ZEPHYRA SYNC] {}", msg); }
    }
    /// 纹理调试打印
    #[inline] pub fn log_texture(msg: &str) {
        if texture_enabled() { eprintln!("[ZEPHYRA TEX] {}", msg); }
    }
    /// Hook 调试打印
    #[inline] pub fn log_hook(msg: &str) {
        if hook_enabled() { eprintln!("[ZEPHYRA HOOK] {}", msg); }
    }
    /// 回退调试打印
    #[inline] pub fn log_fallback(msg: &str) {
        if fallback_enabled() { eprintln!("[ZEPHYRA FALLBACK] {}", msg); }
    }
}

/// Release 构建：所有函数编译为空，零开销
#[cfg(not(debug_assertions))]
pub mod debug {
    #[inline] pub fn init_from_env() {}
    #[inline] pub fn log_sync(_: &str) {}
    #[inline] pub fn log_texture(_: &str) {}
    #[inline] pub fn log_hook(_: &str) {}
    #[inline] pub fn log_fallback(_: &str) {}
}
```

### 9.5.3 后端验证层（`ZEPHYRA_VALIDATION=1`）

| 后端 | Debug 开启 | Release 行为 |
|------|-----------|-------------|
| **D3D11** | `ID3D11DebugDevice::ReportLiveDeviceObjects` + Debug Layer | **强制不创建 Debug Device**，避免性能/内存开销 |
| **D3D12** | `ID3D12DebugDevice` + `ID3D12DebugCommandQueue` + GPU-Based Validation | **强制不创建** |
| **Vulkan** | `VK_LAYER_KHRONOS_validation` + `VK_EXT_debug_utils` | **强制不加载**，`vkCreateInstance` 不传 validation layer |

```rust
// zephyra-winit/src/renderer.rs (示例)
fn create_device(debug: bool) -> Device {
    #[cfg(debug_assertions)]
    if debug && std::env::var("ZEPHYRA_VALIDATION").is_ok() {
        // 创建 D3D11 Debug Device / Vulkan Validation Layer
    }
    #[cfg(not(debug_assertions))]
    if std::env::var("ZEPHYRA_VALIDATION").is_ok() {
        // Release 下设了也报错/忽略，绝不创建验证层
        eprintln!("ZEPHYRA_VALIDATION ignored in release build");
    }
    // 正常创建 Release Device
}
```

### 9.5.4 使用建议

| 场景 | 推荐配置 |
|------|---------|
| **本地开发** | `ZEPHYRA_DEBUG_SYNC=1 ZEPHYRA_DEBUG_TEXTURE=1 ZEPHYRA_VALIDATION=1 cargo run` |
| **CI 测试** | Debug 构建跑测试时自动开启基础调试；Release 构建跑性能基准时强制关闭 |
| **用户现场排查** | 让用户设 `ZEPHYRA_DEBUG_FALLBACK=1` 复现，收集日志后关闭 |
| **生产发布** | **必须用 Release 构建**，所有调试代码零开销 |

---

## 10. Offscreen v2 (CustomOffscreen)

- **Phase 1**：所有后端 `CustomOffscreen = false`，文档标注
- **Phase 2**：离屏 pass 遇到 `Segment::Custom` 也查 hook 表，传递离屏 RT
- **HookContext 新增**：`offscreen_rt: Option<BackendRt>` 字段（Phase 2）

---

## 11. 更新后的 5 天实施计划

| 阶段 | 天数 | 关键交付 |
|------|------|----------|
| **Phase 1** 契约 crate + 统一门面 | 1 | `zephyra-render-ext` + `zephyra-ext` 全绿：`ExternalTexture`/`RenderExtension`/`HookContext`（零空档全覆盖）/能力/生命周期 + 统一门面 API |
| **Phase 2** D3D11 后端 | 1 | `zephyra-winit`：`register_external_texture` (NT Handle)、`RenderHook` (即时上下文)、能力声明 |
| **Phase 3** D3D12 后端 | 1 | `zephyra-winit-d3d12`：同 D3D11 但用 D3D12 API、多窗口分表 |
| **Phase 4** Vulkan 后端 | 1 | `zephyra-vulkan`：`register_external_texture` (VkImage)、`RenderHook` (命令缓冲)、能力声明 |
| **Phase 5** 测试/文档/门禁 | 1 | 三后端测试、跨平台应用样例、全库门禁 |

**总计：5 天**

---

## 12. 验收标准

```bash
# 1. 核心零 wgpu 依赖
cargo tree -p zephyra-render-ext -i wgpu      # 空
cargo tree -p zephyra-ext -i wgpu             # 空
cargo tree -p zephyra-winit -i wgpu           # 空
cargo tree -p zephyra-winit-d3d12 -i wgpu     # 空
cargo tree -p zephyra-vulkan -i wgpu          # 空

# 2. wgpu 版本锁定验证
cargo metadata --format-version=1 | jq '.packages[] | select(.name=="wgpu") | .version'  # 必须为 "24.0.0"

# 3. Windows D3D11
cargo test -p zephyra-winit register_external_texture_nt_handle -- --nocapture
cargo test -p zephyra-winit render_hook_d3d11_context -- --nocapture
cargo test -p zephyra-winit external_texture_cpu_fallback -- --nocapture

# 4. Windows D3D12
cargo test -p zephyra-winit-d3d12 register_external_texture_nt_handle -- --nocapture
cargo test -p zephyra-winit-d3d12 multi_window_hook_registration -- --nocapture

# 5. Linux Vulkan
cargo test -p zephyra-vulkan register_external_texture_vk_image -- --nocapture
cargo test -p zephyra-vulkan render_hook_vk_command_buffer -- --nocapture
cargo test -p zephyra-vulkan vk_image_zero_copy_import -- --nocapture

# 6. 统一门面 API 测试
cargo test -p zephyra-ext external_texture_builder_auto -- --nocapture
cargo test -p zephyra-ext hook_registry_type_safety -- --nocapture
cargo test -p zephyra-ext texture_handle_raii -- --nocapture

# 7. HookContext 全覆盖测试
cargo test -p zephyra-render-ext hook_context_full_coverage -- --nocapture

# 8. 跨平台契约测试
cargo test -p zephyra-render-ext external_texture_contract -- --nocapture

# 9. 跨后端行为一致性测试（新增）
cargo test -p zephyra-winit cross_backend_texture_import -- --nocapture
cargo test -p zephyra-winit-d3d12 cross_backend_texture_import -- --nocapture
cargo test -p zephyra-vulkan cross_backend_texture_import -- --nocapture

# 10. 设备丢失恢复测试（新增）
cargo test -p zephyra-winit device_lost_recovery -- --nocapture
cargo test -p zephyra-winit-d3d12 device_lost_recovery -- --nocapture
cargo test -p zephyra-vulkan device_lost_recovery -- --nocapture

# 11. 多窗口 Hook 隔离测试（新增）
cargo test -p zephyra-winit-d3d12 multi_window_hook_isolation -- --nocapture

# 12. CPU 回退性能基线（新增）
cargo bench -p zephyra-winit cpu_fallback_throughput -- --save-baseline main

# 13. 调试模式测试（新增）
# Debug 构建：环境变量生效
ZEPHYRA_DEBUG_SYNC=1 ZEPHYRA_DEBUG_TEXTURE=1 cargo test -p zephyra-ext debug_mode_enabled -- --nocapture
ZEPHYRA_VALIDATION=1 cargo test -p zephyra-winit validation_layer_enabled -- --nocapture
# Release 构建：环境变量被忽略/报错
cargo test --release -p zephyra-ext debug_mode_disabled_in_release -- --nocapture

# 14. 全库门禁
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --no-deps --workspace
```

---

## 13. 关键决策摘要

| 决策项 | 结果 |
|--------|------|
| Windows 主后端 | **D3D11** (默认) / **D3D12** (可选 feature) |
| Linux 主后端 | **Vulkan** |
| Windows 扩展共享 | **NT Handle** (无 Vulkan 设备) |
| Linux 扩展共享 | **VkImage** 零拷贝 |
| wgpu 角色 | 仅应用层扩展，框架零依赖 |
| **wgpu 版本** | **锁定 `wgpu = "=24.0.0"`**（新框架无旧版本包袱，避免 23→24 破坏性迁移） |
| 同步原语 | Windows: `ID3D12Fence` / Linux: `VkSemaphore` |
| RenderHook 上下文 | Windows: `ID3D11DeviceContext` / Linux: `VkCommandBuffer` |
| **多 GPU 适配器策略** | **框架跟随 wgpu 适配器**：应用创建 wgpu Adapter 后将其 LUID 传给框架（`HostWindow::builder().force_adapter_luid(luid)`），框架在该 LUID 上创建所有后端设备。**框架不枚举、不选、不校验**。 |
| HookContext 完整性 | **零空档，全覆盖**（几何/帧/目标/管线/资源/光追/VRS/同步/调试/内存/版本/输入/XR/自定义） |
| macOS | Phase 2 (Metal) |
| Offscreen 自绘 | Phase 2 实现 |
| 统一门面 API | **zephyra-ext** crate，应用零分支、类型安全、RAII |
| **调试/校验模式** | **Debug 开启、Release 强制关闭**：`ZEPHYRA_DEBUG_*` 环境变量仅 Debug 生效；`ZEPHYRA_VALIDATION` 仅 Debug 下加载验证层；Release 构建编译期剔除所有调试代码，零开销 |

---

## 14. 下一步

### 14.1 Phase 1 实施（当前）

进入 **Phase 1**：
1. 修正 `zephyra-render-ext` 契约 crate（去 `Send+Sync`、`Any` 约束、加默认实现）
2. 新增 `zephyra-ext` 统一门面 crate（`ExtContextHandle`、`ExternalTextureBuilder`、`HookRegistry`、`TextureHandle`、`HookHandle`、`ExtContextImpl` 路由）
3. 导出更新、测试通过

### 14.2 后续工作（Phase 5+）

| 项 | 状态 | 备注 |
|----|------|------|
| **CI GPU Runner** | 待定 | 本地先跑通，CI 后补。矩阵设计见下 |
| **NT Handle 跨进程安全描述符** | 待定 | Phase 2 D3D11 实现时细化 |
| **Vulkan 黑名单最小集合** | 待定 | Phase 4 收集实测数据后定 |
| **CPU 回退阈值默认值** | 待定 | Phase 2/3 基准测试后定 |
| **HookContext 下转失败行为** | ✅ 已定 | `Option` + 文档契约 + Debug 断言（见 §15） |
| **多窗口 Shared Texture 池管理** | 待定 | Phase 2/3 多窗口测试时细化 |
| **RenderHook panic 捕获与隔离** | 待定 | `catch_unwind` 包装，记录日志不崩主进程 |

#### CI GPU Runner 矩阵设计（预留）

> **策略**：本地开发机先跑通所有测试，CI 后续接入自建 GPU runner。

| 层级 | 测试内容 | 运行环境 | 频率 |
|------|---------|---------|------|
| **L0: 纯逻辑** | `cargo test --lib`、契约、序列化、错误映射 | 标准 runner（无 GPU） | 每 PR |
| **L1: CPU 回退** | `ExternalTexture::cpu_fallback`、软件光栅 | 标准 runner | 每 PR |
| **L2: Vulkan 逻辑** | `vkCreateInstance`、设备枚举、能力查询 | Linux + `vulkan-tools` + llvmpipe | 每 PR |
| **L3: 真 GPU 集成** | NT Handle 导入、VkImage 零拷贝、Hook 绘制、设备丢失恢复 | 自建 GPU runner | 每日/手动 |
| **L4: 性能基线** | `cpu_fallback_throughput`、上传带宽、提交延迟 | 固定硬件 runner | 每周/Release 前 |

**最小自建配置**：
- Windows: 1 张 NVIDIA 中端卡（GTX 1660/RTX 3060）
- Linux: 1 个 NVIDIA GPU 云实例 + 1 个 llvmpipe runner

---

## 15. 剩余待讨论细节

| # | 细节 | 优先级 | 状态 |
|---|------|--------|------|
| 1 | **HookContext 下转失败行为** | Phase 1 | ✅ **已定：Option + 文档契约 + Debug 断言** |
| 2 | **NT Handle 跨进程安全描述符** | Phase 2 | 待定 |
| 3 | **Vulkan 黑名单最小集合** | Phase 4 | 待定 |
| 4 | **CPU 回退阈值默认值** | Phase 2/3 | 待定 |
| 5 | **多窗口 Shared Texture 池管理** | Phase 2/3 | 待定 |
| 6 | **RenderHook panic 捕获与隔离** | Phase 2 | 待定 |

---

## 17. Phase 1 修正草案（v1.17 审查结论）

> **目的**：消除 P0 阻塞项，定死 P1 关键约束，确保 Phase 1 可直接编码且后续不破坏 API。

### 17.1 P0 — 阻塞项（必须在 Phase 1 落地）

#### P0-1 契约层去平台类型（C1）

**问题**：`zephyra-render-ext` 不得依赖 `windows`/`ash`/平台头文件。

**修正**：
- `ExternalTextureSource` 改用不透明句柄：
  ```rust
  #[derive(Clone, Copy, Debug)]
  pub struct RawHandle(pub usize);   // HANDLE / VkImage / fd 包装

  pub enum ExternalTextureSource {
      NtHandle { handle: RawHandle, source_backend: Option<SourceBackend> },
      VkImage { image: RawHandle, layout: u32, memory: Option<RawHandle> },
      #[cfg(target_os = "linux")]
      Dmabuf { fd: i32, modifier: u64, planes: u32, offsets: [u32; 4], strides: [u32; 4] },
      Cpu { desc: CpuTextureDesc },
  }
  ```
- `ExternalTextureImporter` 收敛为单入口（与 `source_auto`/统一门面一致）：
  ```rust
  pub trait ExternalTextureImporter {
      fn import(&mut self, desc: ExternalTextureDesc) -> Result<ImportedTexture, ImportError>;
      fn import_cpu(&mut self, desc: CpuTextureDesc) -> Result<ImportedTexture, ImportError>;
  }
  ```
- 平台特定解析/校验在 `zephyra-ext::platform::*` 或各后端 crate 完成。

#### P0-2 句柄命名去冲突（C3）

| 原名 | 用途 | 新名 |
|---|---|---|
| `TextureHandle`（§3.5 RAII） | 应用持有，Drop 自动注销 | `ExternalTextureHandle` |
| `TextureHandle(pub u64)`（HookContext 不透明） | GPU 资源 ID 占位 | `GpuTextureId` |
| `HookHandle` | RAII hook 句柄 | 保持不变 |

`prelude` 只导出 `ExternalTextureHandle` / `HookHandle`，不导出 `GpuTextureId`（内部/HookContext 使用）。

#### P0-3 Payload 链路定型：Key 关联模式（C5）

**决策**：`ExternalTextureHandle`（RAII）**不进入** DisplayList；DisplayList 只携带 `resource_key`。

```rust
// UI 层（zephyra-ui / zephyra-core）
View::custom("video-player")
    .resource_key("video-frame")   // 或 .external_texture("video-frame")
    .size(640, 360)

// DisplayList / Segment
Segment::Custom {
    kind: Cow<'static, str>,
    bounds: Rect,
    resource_key: Option<Cow<'static, str>>, // 取代 payload: TextureHandle
}

// 后端 replay（示例）
fn replay_custom(&mut self, seg: &Segment) {
    let Segment::Custom { kind, bounds, resource_key } = seg else { return };
    let Some(hook) = self.hook_registry.get(kind) else { return };
    let imported = resource_key.as_deref().and_then(|k| self.texture_pool.get(k));
    let mut ctx = self.make_hook_context(*bounds, imported);
    let source = RenderSource::new(resource_key.clone(), imported.map(|t| t.as_any()));
    hook(&mut ctx, *bounds, &source);
}
```

**`RenderSource` 最小契约**（`zephyra-render-ext`）：
```rust
pub struct RenderSource<'a> {
    pub resource_key: Option<&'a str>,
    payload: Option<&'a dyn Any>, // 后端注入的已导入纹理/自定义对象
}
impl<'a> RenderSource<'a> {
    pub fn resource_key(&self) -> Option<&str> { self.resource_key }
    pub fn payload<T: 'static>(&self) -> Option<&T> { self.payload.and_then(|p| p.downcast_ref()) }
}
```

#### P0-4 适配器标识跨平台化（C4）

**问题**：Linux 无 Windows LUID；`force_adapter_luid` 语义/命名不成立。

**修正**：
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterIdentity {
    pub vendor_id: u32,
    pub device_id: u32,
    pub luid: Option<u64>,            // Windows
    pub device_uuid: Option<[u8; 16]>, // Vulkan / 通用
    pub driver_uuid: Option<[u8; 16]>,
}

impl AdapterIdentity {
    /// Phase 1 前置：验证 wgpu 24.x 可提取路径（wgpu::AdapterInfo / wgpu-hal）
    pub fn from_wgpu_adapter(_adapter: &wgpu::Adapter) -> Option<Self> { todo!("Phase 1 探测") }
}

// HostWindowBuilder
pub fn with_adapter_identity(mut self, id: AdapterIdentity) -> Self { ... }
```

**文档全局替换规则**：`LUID`（作为通用概念）→ `适配器标识`；保留 “Windows: LUID” 作为子项说明。**Phase 1 前置任务**：确认 `wgpu = "=24.0.0"` 暴露 adapter identity 的可行路径，否则回退 `wgpu-hal` 或平台枚举对齐。

### 17.2 P1 — Phase 1 定死的约束

| # | 决策 | 结论 |
|---|---|---|
| **P1-1 线程模型** | `ExtContextHandle` 使用 `Arc<Mutex<ExtContext>>` **还是** 单线程 `!Send`？ | **Phase 1 选 A：单线程**。`ExtContextHandle` 内部 `Rc<RefCell<_>>`（或 `Arc<RefCell<_>>` 但不实现 `Send/Sync`），文档明确“仅 UI/渲染线程调用”。Phase 3 若需跨线程上传再引入 `Send` 变体，避免过早加锁。 |
| **P1-2 Hook 桥接** | `RenderExtension::HookCtx` 关联类型如何对接 `DrawHook`？ | **移除 `RenderExtension::HookCtx` 关联类型**。类型安全由 `HookRegistry::d3d11/d3d12/vulkan` 泛型方法保证；注册时包装：`Box<dyn FnMut(&mut dyn HookContext, Rect, &RenderSource)>`，内部 `ctx.downcast_ref::<D3D11HookContext>().expect(...)`（Debug 断言）。 |
| **P1-3 register_hook 签名** | `ExtContextImpl::register_hook<H: RenderHook>` 中 `RenderHook` 未定义。 | 统一为：`fn register_hook(&mut self, kind: Cow<'static,str>, hook: Box<DrawHook>) -> Result<HookHandle, ExtError>;` |
| **P1-4 HookContext 去重** | Category 1/13 重复 `render_pass_*`；Category 2/11 重复 `resource_version/is_first_frame/prev_frame_index`。 | **实现时去重**，各字段仅定义一次；文档 v1.17 标记为已知冗余，Phase 1 代码以去重版为准。 |

### 17.3 P2 — Phase 1 顺手收敛

- `CpuTextureDesc { pixels: Vec<u8>|&[u8], width: u32, height: u32, row_pitch: u32, format: ExternalTextureFormat }`
- `ImportError` 增加 `kind`/`code`（可选），保留 `message`
- `ExternalTextureHandle::drop` 在 Debug 下记录注销失败；新增 `try_unregister()`
- 补齐 `WindowId`、`RenderSource`、`NtHandleDesc/VkImageDesc`（若保留）或统一 `ExternalTextureDesc` 的 crate 归属
- 同步 §14.2 与 §15 状态（已在 v1.17 修正）

### 17.4 Phase 1 实施清单（修订）

1. **前置验证**：`wgpu = "=24.0.0"` 能否获取 `AdapterIdentity`（LUID/UUID）；记录结论到 `docs/dev-log/`。
2. `zephyra-render-ext`：不透明句柄、`ExternalTextureDesc/Source/Importer`、`RenderSource`、`DrawHook`、`HookContext`（去重核心字段）、`ImportError`、`GpuFence`。
3. `zephyra-ext`：`ExternalTextureHandle`/`HookHandle` RAII、`ExtContextHandle`（单线程）、`ExternalTextureBuilder`、`HookRegistry` 桥接、`AdapterIdentity`、`debug`。
4. `zephyra-core`/DisplayList（最小改动）：`Segment::Custom` 支持 `resource_key`（与现有 payload 机制兼容迁移）。
5. 门禁：`cargo check/test/clippy/doc -p zephyra-render-ext -p zephyra-ext`（及受影响 workspace 成员）。

### 17.5 补充修正（实现前须对齐）

| # | 问题 | 修正 |
|---|---|---|
| **S1** | §4.1 仍保留平台多方法 `ExternalTextureImporter`（`import_nt_handle/import_vk_image/import_dmabuf`）与旧 `import_cpu` 签名，与 P0-1/P2 冲突 | **以 §17 P0-1 为准**：单入口 `import(ExternalTextureDesc)` + `import_cpu(CpuTextureDesc)`。§4.1 示例视为废弃，Phase 1 按此重写。 |
| **S2** | `DrawHook`/`RenderSource` 生命周期不严谨 | `pub type DrawHook = dyn for<'a> FnMut(&mut dyn HookContext, Rect, &RenderSource<'a>) + Send + 'static;`（hooks 长期存储；`RenderSource` 仅单次调用借用）。 |
| **S3** | `GpuFence::wait` 语义歧义 | 文档改为：`wait` 表示**在提交消费者命令前确保生产者完成**的同步点；实现可为 GPU 队列等待（优先）或必要时 CPU 等待（回退路径须标注）。避免在 UI 线程做长时间 CPU 阻塞。 |
| **S4** | `SyncInput::Fence(Box<dyn GpuFence>)` 独占所有权，不利复用 | 改为 `Fence(Arc<dyn GpuFence>)`（或 `SharedFence` newtype）；`GpuFence` 保留 `clone_handle` 仅用于跨进程导出场景，不作为共享手段。 |
| **S5** | `AdapterIdentity::from_wgpu_adapter(&wgpu::Adapter)` 会引入 wgpu 依赖 | `zephyra-ext`/`render-ext` **不得**依赖 wgpu。改为：`impl AdapterIdentity { pub fn from_wgpu_hal(...) }`（feature-gated）或由应用/示例 crate 提供转换函数；核心只接收 `AdapterIdentity` 值。 |
| **S6** | `CpuTextureDesc` 语法/所有权不明确 | 定为：`pub struct CpuTextureDesc<'a> { pub pixels: &'a [u8], pub width: u32, pub height: u32, pub row_pitch: u32, pub format: ExternalTextureFormat }`（导入时借用；如需上传队列持有由后端内部拷贝）。 |
| **S7** | `ExternalTextureHandle::update()` 与 `register_texture()` 语义重叠 | 明确：`update(builder)` = **同 key 替换纹理内容/源**（保持句柄有效、触发重绘标记）；`register_texture` 对已存在 key 的行为必须定义为“替换并返回同逻辑句柄”或“错误”，二选一写死（建议**替换**，便于视频帧更新）。 |
| **S8** | 多窗口 + 纹理 key 作用域未定 | Phase 1 规定：**texture key 全局唯一**（`ExtContext` 级）；多窗口共享同一纹理池。若未来需要窗口隔离，引入 `namespaced key`（`window_id/key`），不破坏现有 API。 |
| **S9** | `ImportedTexture::valid_until: FrameId` / `FrameId` 未定义 | 在 `zephyra-render-ext::types` 定义 `pub struct FrameId(pub u64);`。`valid_until` 语义：**当前帧提交前有效**；后端不得在帧结束后复用同一 imported 视图而不重新导入（除非显式缓存策略 Phase 2）。 |