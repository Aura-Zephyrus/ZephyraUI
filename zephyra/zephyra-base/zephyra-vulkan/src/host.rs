//! 窗口 + GPU + 保留树的组合体。对应 Windows 那边 `host.rs` 的角色。
//!
//! 本文件以 `zephyra-winit-vk/src/host.rs`（wgpu 版）为模板移植，只改 GPU 相关调用。
//! wgpu 版经过实测验证，接口形状直接复用。
//!
//! # 字段顺序是**硬约束**
//!
//! 窗口**最后**声明，因而**最先**被借用、最晚被析构。winit 的窗口句柄被 surface
//! 与 HostWindow 同时持有，而 surface 的生命周期不能比它所依附的窗口长。

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use ash::vk;
use winit::application::ApplicationHandler;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use zephyra_window::{
    Backend, HostEventOutcome, HostTick, WindowHandle, WindowHost, WindowKey, WindowSpec,
};

use zephyra_core::context::Context;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::input::{
    InputRouter, PointerEvent, PointerPhase, PointerSource, RoutedEvent, RoutedEvents,
};
use zephyra_core::node::Node;
use zephyra_core::theme::{Theme, ThemeVariant};

use zephyra_fx::{Frame, InputState, TextEngine, View};

use crate::renderer::{RenderCtx, RenderHook, Renderer};
use crate::surface::{Device, Surface};

/// 进程级共享设备。
///
/// # 为什么是 `OnceLock<Arc<Mutex<Device>>>`
///
/// 与 D3D12 的 `SHARED` 同形：instance / 逻辑设备 / 队列是**进程级**资源，
/// 第二个窗口再建一个设备会得到另一套纹理、管线与描述符空间，跨窗口共享就没了。
/// `Device` 只 `Send` 不 `Sync`（`ash` 的句柄里是裸指针），所以外面套 `Mutex`；
/// 事件循环是单线程的，锁的争用只可能来自应用自己另起的 `WindowSet` 线程。
static SHARED_DEVICE: OnceLock<Arc<Mutex<Device>>> = OnceLock::new();

fn shared_device() -> Result<Arc<Mutex<Device>>, crate::surface::SurfaceError> {
    if let Some(existing) = SHARED_DEVICE.get() {
        return Ok(Arc::clone(existing));
    }
    let device = Device::new()?;
    let _ = SHARED_DEVICE.set(Arc::new(Mutex::new(device)));
    // 竞争时 `set` 失败，`get` 拿到的仍是先到者建的那一份 —— 与 D3D12
    // `shared_stack` 同一写法：设备创建本身比竞态窗口贵得多，重复建出来
    // 直接丢弃即可。
    Ok(Arc::clone(
        SHARED_DEVICE.get().expect("刚刚 set 过或别人 set 了"),
    ))
}

fn lock_device(shared: &Arc<Mutex<Device>>) -> MutexGuard<'_, Device> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 窗口 + GPU + 保留树的组合体。
///
/// # 字段顺序是硬约束
///
/// `surface` 必须在 `window` **之前**声明（先析构），`window` 最后声明。
/// 交换链依附于窗口，窗口先死就是拿悬垂句柄去 `vkDestroySwapchainKHR`。
pub struct HostWindow<V: View> {
    window_key: WindowKey,
    /// 应用侧句柄。单窗口兼容路径没有它。
    handle: Option<WindowHandle>,
    /// 进程级共享设备。每窗口只锁不持有。
    device: Arc<Mutex<Device>>,
    /// 本窗口的 surface + swapchain。
    surface: Surface,
    renderer: Renderer,
    view: V,
    theme: &'static Theme,
    /// 帧缓存 —— 保留树 + `DisplayList` + 上一帧视口，全在核心里。
    cache: FrameCache,
    router: InputRouter,
    pointer: Option<u32>,
    pointer_pos: (f32, f32),
    pending_pointer_events: Vec<zephyra_fx::PointerInput>,
    pending_text_events: Vec<zephyra_core::input::TextEvent>,
    /// winit 按键 → core 文本事件的翻译器（修饰键状态也住在这里）。
    ///
    /// 本后端曾经把 `key_events` / `scroll` 写死成空值 —— 键盘与滚轮在这条
    /// 链路上从未接通过。现在与两个 Windows 后端共用同一份翻译。
    translator: zephyra_winit_input::Translator,
    /// 焦点状态机：Tab 移动、点击设焦点、失焦清空。跨帧持有。
    focus: zephyra_core::input::FocusState,
    /// 键盘激活状态机：焦点落在 ACTIVATABLE 节点上时 Space/Enter 闩住 pressed。
    activation: zephyra_core::input::ActivationState,
    pending_scroll: Option<zephyra_fx::ScrollInput>,
    dirty: bool,
    engine: std::cell::RefCell<TextEngine>,
    atlas_revision_uploaded: u64,
    system_languages: Vec<String>,
    frames: usize,
    last_frame_at: Option<std::time::Instant>,
    /// 每帧的命令缓冲 + 围栏（复用）。
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    /// 交换链图的帧缓冲（每个图一个）。resize 后重建。
    framebuffers: Vec<vk::Framebuffer>,
    /// 帧缓冲用的图视图（销毁责任在这里，resize 时先毁再用）。
    fb_views: Vec<vk::ImageView>,
    /// 帧缓冲当前尺寸 —— resize 只变尺寸不变图数，必须单独记。
    fb_extent: (u32, u32),
    /// `ZEPHYRA_FRAME_DUMP` 是否已经写过一副图。
    dumped: bool,
    /// 窗口。**最后析构**。
    window: Arc<Window>,
}

/// 未指定尺寸时的默认客户区尺寸。
pub const DEFAULT_CLIENT_SIZE: (u32, u32) = (1100, 700);

impl<V: View> HostWindow<V> {
    /// 建一个宿主（默认尺寸）。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        Self::new_lenient_sized(event_loop, view, DEFAULT_CLIENT_SIZE)
    }

    /// 建一个指定客户区尺寸的宿主。
    pub fn new_lenient_sized(event_loop: &ActiveEventLoop, view: V, size: (u32, u32)) -> Self {
        Self::new_lenient_opts(
            event_loop,
            view,
            size,
            zephyra_app_config::WindowOpts::default(),
        )
    }

    /// [`Self::new_lenient_sized`] + **按窗口的创建时选项**。
    ///
    /// 透明是 per-window 参数（[`zephyra_app_config::WindowOpts`]，显式给定
    /// 优先于 `window.transparent`），且**transparent ⇒ frameless**。
    /// 本后端的交换链 clear 恒为 alpha=0、composite alpha 探测恒首选
    /// `PRE_MULTIPLIED`（见 `surface.rs`），所以透明在这里**只**差 winit 的
    /// `with_transparent`（X11 侧换 ARGB visual）。
    pub fn new_lenient_opts(
        event_loop: &ActiveEventLoop,
        view: V,
        size: (u32, u32),
        opts: zephyra_app_config::WindowOpts,
    ) -> Self {
        let config = zephyra_app_config::load_from_current_dir();
        let transparent = opts.transparent.unwrap_or(config.window.transparent);
        let frameless = config.window.frameless || transparent;
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("ZephyraUI")
                .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1))
                .with_decorations(!frameless)
                .with_transparent(transparent),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("[zephyra] 建窗失败：{e}");
                panic!("建窗失败：winit 拿不到窗口");
            }
        };
        eprintln!(
            "[zephyra] 窗口装饰: {}{}",
            if frameless { "frameless" } else { "system" },
            if transparent {
                "（透明窗口）"
            } else {
                ""
            }
        );
        // winit **默认不允许** IME（`Window::set_ime_allowed` 的文档原话），
        // 不开就永远收不到 `WindowEvent::Ime` —— 那条分支会变成死代码。
        //
        // ⚠ 真实应用应当「文本框获得焦点时才开」：组字期间 winit 不送
        // `KeyboardInput`，于是组字中的快捷键本来也不生效。焦点系统还没建，
        // 所以先全局打开；有了焦点系统后改成随焦点开关。
        window.set_ime_allowed(true);

        let device = match shared_device() {
            Ok(d) => d,
            Err(e) => {
                eprintln!(
                    "[zephyra] 建立 GPU 失败：{e}\n\
                     [zephyra] 这台机器上可能没有可用的 Vulkan 适配器。"
                );
                panic!("建立 GPU 失败");
            }
        };
        let surface = {
            let dev = lock_device(&device);
            match Surface::new(&dev, &window) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[zephyra] 建立表面失败：{e}");
                    panic!("建立表面失败");
                }
            }
        };

        let (engine, note) = zephyra_fx::text::from_system_font();
        eprintln!("[zephyra] {note}");

        // MSAA 与两个 Windows 后端同档（4×；ZEPHYRA_MSAA=0 可关）。渲染通道
        // 带 resolve 附件 —— 由硬件在通道结束时自动完成，等价 wgpu 版的
        //「画进 MSAA、resolve 进 surface」。
        let (samples, cmd, fence) = {
            let dev = lock_device(&device);
            let samples = dev.msaa_samples.as_raw();
            let cmd = unsafe {
                dev.device.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::builder()
                        .command_pool(dev.command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1)
                        .build(),
                )
            }
            .expect("command buffer")[0];
            let fence = unsafe {
                dev.device
                    .create_fence(&vk::FenceCreateInfo::default(), None)
            }
            .expect("fence");
            (samples, cmd, fence)
        };
        let renderer = {
            let dev = lock_device(&device);
            match Renderer::new(&dev, surface.format, 1024, samples) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[zephyra] 建渲染管线失败：{e}");
                    panic!("建渲染管线失败");
                }
            }
        };

        let platform = platform_current();
        let system_languages = platform.system_languages();

        Self {
            window_key: WindowKey::unspecified(),
            handle: None,
            device,
            surface,
            renderer,
            view,
            theme: Box::leak(Box::new(Theme::dark())),
            cache: FrameCache::new(),
            router: InputRouter::new(),
            pointer: None,
            pointer_pos: (0.0, 0.0),
            pending_pointer_events: Vec::new(),
            pending_text_events: Vec::new(),
            translator: zephyra_winit_input::Translator::new(),
            focus: zephyra_core::input::FocusState::new(),
            activation: zephyra_core::input::ActivationState::new(),
            pending_scroll: None,
            dirty: true,
            engine: std::cell::RefCell::new(engine),
            atlas_revision_uploaded: u64::MAX,
            system_languages,
            frames: 0,
            last_frame_at: None,
            cmd,
            fence,
            framebuffers: Vec::new(),
            fb_views: Vec::new(),
            fb_extent: (0, 0),
            dumped: false,
            window,
        }
    }

    /// 登记外部渲染源钩子（相同 `kind` 会覆盖）。
    pub fn register_hook(
        &mut self,
        kind: impl Into<std::borrow::Cow<'static, str>>,
        hook: Box<dyn RenderHook>,
    ) {
        self.renderer.register_hook(kind, hook);
    }

    /// 装一个样式注入点。
    pub fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        self.cache.set_styler(styler);
    }

    /// 切主题（Light / Dark）。
    pub fn set_theme(&mut self, variant: ThemeVariant) {
        self.theme = Box::leak(Box::new(Theme::builtin(variant)));
        self.cache.invalidate();
    }

    /// 当前主题。
    pub fn theme_variant(&self) -> ThemeVariant {
        self.theme.variant
    }

    /// 扩展上下文句柄：外部纹理/Hook 注册。
    pub fn ext_context(&self) -> zephyra_ext::ExtContextHandle {
        self.renderer.ext_handle()
    }

    /// 图像纹理表。
    pub fn textures(&self) -> &crate::renderer::TextureTable {
        self.renderer.textures()
    }

    /// 已注册纹理数量（与 D3D12 宿主 API 对齐，便于共享 demo 代码）。
    pub fn texture_count(&self) -> usize {
        self.renderer.textures().len()
    }

    /// 注册 / 重传一张图像纹理。
    pub fn register_texture(
        &mut self,
        name: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), crate::surface::SurfaceError> {
        let dev = lock_device(&self.device);
        self.renderer
            .upload_texture(&dev, name, pixels, (width, height))
    }

    /// 窗口。
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// 客户区尺寸（**物理**像素）。
    pub fn client_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width.max(1), s.height.max(1))
    }

    /// 布局用的尺寸（有效像素）。
    pub fn effective_size(&self) -> (f32, f32) {
        let scale = self.window.scale_factor() as f32;
        let (w, h) = self.client_size();
        (w as f32 / scale, h as f32 / scale)
    }

    /// 当前 hover 的节点。
    pub fn hovered(&self) -> Option<&zephyra_core::id::ElementKey> {
        self.router.hovered()
    }

    /// 按应用请求设置窗口外观。
    pub fn apply_window_chrome(
        &self,
        chrome: zephyra_platform::WindowChrome,
    ) -> zephyra_platform::ApplyOutcome {
        platform_current().apply_window_chrome(window_handle_of(&self.window), chrome)
    }

    /// 一帧什么都没做时返回的东西。
    fn empty_frame(&self) -> Frame {
        Frame {
            diff: zephyra_core::diff::DiffStats::default(),
            render: zephyra_fx::quad::RenderReport::default(),
            hovered: None,
            damage: Damage::Full,
            redrawn: false,
        }
    }

    /// 跑一帧：布局 → `DisplayList` → 顶点展开 → 提交。
    pub fn frame(&mut self) -> Frame {
        let size = self.client_size();
        let revision_now = self.engine.borrow().atlas_revision();
        if revision_now != self.atlas_revision_uploaded {
            self.cache.invalidate();
        }
        self.atlas_revision_uploaded = revision_now;

        let theme = self.theme;
        let (diff, damage, redrawn) = {
            let ctx =
                Context::with_engine(theme, &self.engine).with_icons(zephyra_core::icon::builtin());
            let element = self.view.view(&ctx);
            let outcome = self
                .cache
                .frame(&element, ctx, (size.0 as f32, size.1 as f32));
            match outcome {
                FrameOutcome::Unchanged => (
                    zephyra_core::diff::DiffStats::default(),
                    Damage::Full,
                    false,
                ),
                FrameOutcome::Updated { stats, damage, .. } => (stats, damage, true),
            }
        };
        if !redrawn {
            return Frame {
                diff,
                render: zephyra_fx::quad::RenderReport::default(),
                hovered: self.router.hovered().map(|k| k.to_string()),
                damage,
                redrawn: false,
            };
        }

        let (atlas_pixels, atlas_size) = {
            let engine = self.engine.borrow();
            let atlas = engine.atlas();
            (atlas.pixels().to_vec(), atlas.size())
        };
        let ctx = RenderCtx {
            display_list: self.cache.list(),
            size,
            damage,
            atlas_pixels: &atlas_pixels,
            atlas_size,
            atlas_revision: revision_now,
            filters_enabled: true,
            default_blend: zephyra_core::filter::BlendMode::SrcOver,
        };
        let render = {
            let dev = lock_device(&self.device);
            match self.renderer.prepare(&dev, &ctx) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[zephyra] 渲染失败：{e}");
                    return self.empty_frame();
                }
            }
        };
        self.frames += 1;
        if trace_enabled() {
            eprintln!(
                "[trace] frame {} 客户区 {:?} 顶点 {} 字节 四边形 {} 未实现混合 {}",
                self.frames,
                self.client_size(),
                render.vertex_bytes,
                render.quads,
                render.unimplemented_blends,
            );
        }
        Frame {
            diff,
            render,
            hovered: self.router.hovered().map(|k| k.to_string()),
            damage,
            redrawn: true,
        }
    }

    /// 上屏：开渲染通道 → 画 → 提交 → present。
    pub fn present(&mut self) {
        // ⚠ 锁住共享设备做整帧提交：`Device` 不 `Sync`，多个窗口的提交必须
        // 串行。锁只覆盖本帧，空闲时不占用。
        let shared = Arc::clone(&self.device);
        let dev = lock_device(&shared);
        // 帧缓冲可能还没建（首次或 resize 后）。尺寸也要盯：resize 只变尺寸
        // 不变图数，光比个数会漏掉整段错位的帧缓冲。
        if self.framebuffers.len() != self.surface.images.len()
            || self.fb_extent != self.surface.extent
        {
            self.rebuild_framebuffers(&dev);
        }
        let (idx, wait_sem) = match self.surface.acquire(&dev) {
            Ok(v) => v,
            Err(_) => return,
        };
        let signal_sem = unsafe {
            dev.device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        }
        .expect("semaphore");
        let fb = match self.framebuffers.get(idx as usize) {
            Some(fb) => *fb,
            None => return,
        };

        // 开渲染通道（用最近一次 frame() 录好的命令）。
        unsafe {
            dev.device
                .begin_command_buffer(
                    self.cmd,
                    &vk::CommandBufferBeginInfo::builder()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                        .build(),
                )
                .ok();
            // ⚠ 绘制命令必须**在渲染通道内**录制 —— 这是本次重构的核心：
            // frame() 只做顶点展开与上传（prepare），真正的绘制在这里录。
            let (unimplemented, refractions, lens_passes, backdrop_blurs) =
                self.renderer.record_frame(
                    &dev,
                    self.cmd,
                    fb,
                    self.cache.list(),
                    zephyra_core::filter::BlendMode::SrcOver,
                );
            if trace_enabled() && unimplemented > 0 {
                eprintln!("[trace] 退回默认混合 {unimplemented} 段");
            }
            if trace_enabled() && refractions > 0 {
                eprintln!("[trace] 折射趟 {refractions}");
            }
            if trace_enabled() && lens_passes > 0 {
                eprintln!("[trace] 透镜趟 {lens_passes}");
            }
            if trace_enabled() && backdrop_blurs > 0 {
                eprintln!("[trace] 背景模糊趟 {backdrop_blurs}");
            }
            dev.device.end_command_buffer(self.cmd).ok();
        }
        let _ = self
            .surface
            .submit_and_present(&dev, self.cmd, wait_sem, signal_sem, idx, self.fence);
        // 等 GPU 完成（简化）。
        unsafe {
            dev.device.device_wait_idle().ok();
        }
        // 诊断：ZEPHYRA_FRAME_DUMP —— 读回刚画完的那张交换链图（此刻还在
        // PRESENT_SRC；read_back 自己做布局往返并转回）。1:1 对比的测量工具。
        if !self.dumped {
            if let Ok(path) = std::env::var("ZEPHYRA_FRAME_DUMP") {
                self.dumped = true;
                let (w, h) = self.surface.extent;
                match self
                    .surface
                    .read_back(&dev, self.surface.images[idx as usize], w, h)
                {
                    Ok(data) => {
                        let px = zephyra_fx::pixels::PixelRead {
                            data,
                            order: zephyra_fx::pixels::ChannelOrder::Bgra,
                            row_pitch: w * 4,
                            width: w,
                            height: h,
                            declared_width: w,
                            declared_height: h,
                        };
                        let colors = px.distinct_colors(2);
                        if path == "-" {
                            eprintln!("[dump] {w}x{h} 颜色数 {colors}");
                        } else {
                            // `encode_srgb = false`：swapchain 用 UNORM（见
                            // `Surface::new` 的格式取舍），读回字节 = 着色器线性值直通，与 Windows
                            // D3D12 完全同构 —— 直接落盘就是「Windows 显示的那个值」。
                            match zephyra_fx::write_bmp(&path, &px, false) {
                                Ok(()) => {
                                    eprintln!("[dump] 已写 {path}（{w}x{h}，颜色数 {colors}）")
                                }
                                Err(e) => eprintln!("[dump] 写 {path} 失败: {e}"),
                            }
                        }
                    }
                    Err(e) => eprintln!("[dump] 读回失败: {e}"),
                }
            }
        }
    }

    /// 重建帧缓冲（resize 后或首次 present 时）。
    fn rebuild_framebuffers(&mut self, dev: &Device) {
        let device = &dev.device;
        // 销毁旧的（fb 引用的交换链图在 resize 后已失效 —— 不销毁就是悬垂句柄）。
        for fb in self.framebuffers.drain(..) {
            unsafe {
                device.destroy_framebuffer(fb, None);
            }
        }
        for v in self.fb_views.drain(..) {
            unsafe {
                device.destroy_image_view(v, None);
            }
        }
        let (w, h) = self.surface.extent;
        // MSAA 时帧缓冲 = [MSAA 视图, 交换链视图]（两附件，与渲染通道一致）。
        let msaa = self.renderer.ensure_msaa(dev, (w, h));
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        for &img in &self.surface.images {
            let view = unsafe {
                device.create_image_view(
                    &vk::ImageViewCreateInfo::builder()
                        .image(img)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(self.surface.format)
                        .subresource_range(range)
                        .build(),
                    None,
                )
            }
            .expect("image view");
            let attachments = match msaa {
                Some(m) => vec![m, view],
                None => vec![view],
            };
            let fb = unsafe {
                device.create_framebuffer(
                    &vk::FramebufferCreateInfo::builder()
                        .render_pass(self.renderer.msaa_render_pass)
                        .attachments(&attachments)
                        .width(w)
                        .height(h)
                        .layers(1)
                        .build(),
                    None,
                )
            }
            .expect("framebuffer");
            self.fb_views.push(view);
            self.framebuffers.push(fb);
        }
        self.fb_extent = (w, h);
    }

    /// 把当前输入状态推给视图。
    pub fn sync_input(&mut self) {
        let size = self.client_size();
        let scale = self.window.scale_factor();
        let now = std::time::Instant::now();
        let dt_ms = self
            .last_frame_at
            .map(|t| now.duration_since(t).as_secs_f32() * 1000.0)
            .unwrap_or(0.0);
        self.last_frame_at = Some(now);
        let state = InputState {
            hovered: self.router.hovered().cloned(),
            // 键盘激活复用指针的 pressed 通道（指针优先）—— Space 按住时模板出
            // 按下态、抬起时归空，与应用既有的「上帧按着→这帧松了 = 点击」重建逻辑
            // 同构，应用零改动。
            pressed: self
                .router
                .pressed()
                .or_else(|| self.activation.held())
                .cloned(),
            // 视觉焦点 = 焦点环可见时才报告（鼠标点击设焦点但不画环，见
            // `FocusState::is_visible`）。激活 / 无障碍另走 `self.focus.focused()`。
            focused: if self.focus.is_visible() {
                self.focus.focused().cloned()
            } else {
                None
            },
            pointers: self.router.pointer_count(),
            text_events: std::mem::take(&mut self.pending_text_events),
            scroll: self.pending_scroll.take(),
            pointer_events: std::mem::take(&mut self.pending_pointer_events),
            viewport: (size.0 as f32, size.1 as f32),
            scale: scale as f32,
            dt_ms,
            languages: self.system_languages.clone(),
        };
        self.view.on_input(&state);
    }

    /// 处理指针移动。返回是否产生了路由事件。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        self.pointer_pos = (x, y);
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Move,
        });
        let theme = self.theme;
        let pos = PointerEvent {
            id: 0,
            x,
            y,
            phase: PointerPhase::Move,
            source: PointerSource::Mouse,
        };
        self.route(|router, node, out| {
            router.on_pointer(node, pos, Context::new(theme), out);
        })
        .0
    }

    /// 处理一次按下。
    pub fn on_pointer_down(&mut self) -> bool {
        self.pointer = Some(0);
        let (x, y) = self.pointer_pos;
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Down,
        });
        let theme = self.theme;
        self.route(|router, node, out| {
            router.on_pointer(
                node,
                PointerEvent {
                    id: 0,
                    x,
                    y,
                    phase: PointerPhase::Down,
                    source: PointerSource::Mouse,
                },
                Context::new(theme),
                out,
            );
        })
        .0
    }

    /// 处理一次抬起。
    pub fn on_pointer_up(&mut self) -> bool {
        self.pointer = None;
        let (x, y) = self.pointer_pos;
        self.pending_pointer_events.push(zephyra_fx::PointerInput {
            x,
            y,
            phase: zephyra_fx::PointerInputPhase::Up,
        });
        let theme = self.theme;
        let (changed, out) = self.route(|router, node, out| {
            router.on_pointer(
                node,
                PointerEvent {
                    id: 0,
                    x,
                    y,
                    phase: PointerPhase::Up,
                    source: PointerSource::Mouse,
                },
                Context::new(theme),
                out,
            );
        });
        // 点击设焦点：`RoutedEvent::Click` 是「按下 + 抬起在同一节点」。
        // 焦点设给被点到的那个 key —— 文本框点进去获得键盘焦点。
        if let Some(target) = out.iter().find_map(|e| match e {
            RoutedEvent::Click(key) => Some(key.clone()),
            _ => None,
        }) {
            if let Some(node) = self.cache.node_mut() {
                if self.focus.focus(node, Some(&target), Context::new(theme)) {
                    self.dirty = true;
                }
            }
        }
        changed
    }

    /// 统一的「跑一次路由器 + 置脏」包装。
    fn route<F>(&mut self, f: F) -> (bool, RoutedEvents)
    where
        F: FnOnce(&mut InputRouter, &Node, &mut RoutedEvents),
    {
        let Some(node) = self.cache.node() else {
            return (false, RoutedEvents::new());
        };
        let before = self.router.revision();
        let mut out = RoutedEvents::new();
        f(&mut self.router, node, &mut out);
        let changed = self.router.revision() != before;
        if changed {
            self.dirty = true;
        }
        (changed, out)
    }
}

impl<V: View + 'static> WindowHost for HostWindow<V> {
    fn key(&self) -> WindowKey {
        self.window_key
    }

    fn winit_id(&self) -> Option<WindowId> {
        Some(self.window.id())
    }

    fn start(&mut self) -> Result<(), String> {
        // 帧缓冲在首次 present 时建。
        self.sync_input();
        let frame = self.frame();
        if frame.redrawn {
            self.present();
        }
        Ok(())
    }

    fn tick(&mut self) -> HostTick {
        let mut tick = HostTick::default();
        // 「动画播放中」由视图回答：保留式 UI 不常驻重画，动画只在事件到来时
        // 推进一帧，然后循环睡死 —— 中间整段弹簧曲线看不到。
        //
        // ⚠ 标脏之外还必须**推时钟**：本后端的 `sync_input()`（→ `on_input`
        // → 动画 `advance`）在下面的 dirty 分支里，所以顺序是「先标脏进来，
        // 再在分支里推进」；`busy` 用**推进之后**的答案，收敛那一拍就交还
        // `Wait`（否则多 Poll 一轮，且静止时永远空转）。
        let mut animating = self.view.wants_frames();
        if animating {
            self.dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            self.sync_input();
            let frame = self.frame();
            if frame.redrawn {
                self.present();
                tick.painted = true;
            }
            animating = self.view.wants_frames();
        }
        tick.busy = animating;
        tick
    }

    fn handle_event(&mut self, event: winit::event::WindowEvent) -> HostEventOutcome {
        use winit::event::{ElementState, MouseButton, WindowEvent};
        match event {
            WindowEvent::CloseRequested => return HostEventOutcome::Close,
            WindowEvent::RedrawRequested => {
                // 帧在 `tick` 里跑（`WindowSet` 每轮都会调它）。这里只标脏。
                self.dirty = true;
            }
            WindowEvent::Resized(_) => {
                // 先销毁帧缓冲/视图再 resize：旧 fb 引用着即将被销毁的交换链图，
                // 顺序反了就是悬垂句柄 —— 下一次 present 踩到的可能是已回收的内存。
                let shared = Arc::clone(&self.device);
                let dev = lock_device(&shared);
                for fb in self.framebuffers.drain(..) {
                    unsafe {
                        dev.device.destroy_framebuffer(fb, None);
                    }
                }
                for v in self.fb_views.drain(..) {
                    unsafe {
                        dev.device.destroy_image_view(v, None);
                    }
                }
                // ⚠ resize 失败必须显式处理：静默忽略的症状是「窗口拉大后画面
                // 停在旧尺寸」，而那与「布局算错了」无法区分。
                if let Err(e) = self.surface.resize(&dev, &self.window) {
                    eprintln!("[zephyra] resize 失败：{e}");
                }
                self.dirty = true;
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.cache.invalidate();
                self.dirty = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.on_cursor_moved(position.x as f32, position.y as f32) {
                    self.dirty = true;
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    if self.on_pointer_down() {
                        self.dirty = true;
                    }
                }
                ElementState::Released if self.on_pointer_up() => {
                    self.dirty = true;
                }
                _ => {}
            },
            WindowEvent::ModifiersChanged(m) => {
                self.translator.on_modifiers(m.state());
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.translator
                    .key_events(&event, &mut self.pending_text_events);
                // 键盘激活（Space/Enter）**当帧**生效，与下面的 Tab 同理：核心状态机
                // 自己判「这个键是不是激活键、焦点节点可不可激活」，宿主只递键边沿。
                {
                    let kp = zephyra_winit_input::KeyParts::from(&event);
                    let key_input = self.translator.key(kp);
                    let focused = self.focus.focused().cloned();
                    let node = self.cache.node();
                    if let Some(true) = self.activation.on_key(
                        &key_input,
                        node,
                        focused.as_ref(),
                        Context::new(self.theme),
                    ) {
                        self.dirty = true;
                    }
                }
                // Tab 移焦点是**当帧**就该生效的：用户按 Tab 期望立刻看到焦点环
                // 跳到下一个控件。
                if event.state == winit::event::ElementState::Pressed {
                    if let winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab) =
                        event.logical_key
                    {
                        let shift = self.translator.modifiers().shift();
                        if let Some(node) = self.cache.node_mut() {
                            if self.focus.tab(node, shift, Context::new(self.theme)) {
                                self.dirty = true;
                            }
                        }
                    }
                }
                self.dirty = true;
            }
            WindowEvent::Focused(false) => {
                // 窗口失焦：焦点必须清空（否则回来后旧焦点还亮着，且按键全落到
                // 一个看不见的节点上）。`clear` 会写回 FOCUSED=false 到旧节点。
                // 激活闩也清（与 D3D11 / D3D12 宿主一致）。
                if let Some(node) = self.cache.node_mut() {
                    self.focus.clear(Some(node));
                    self.dirty = true;
                }
                self.activation.clear();
            }
            WindowEvent::Ime(ime) => {
                // 插入点：焦点系统还没建，用最近一次指针位置当代理（用户点进
                // 输入框就在那）。`Ime::Start` 的坐标本来就是「贴在哪儿」的提示。
                self.translator
                    .ime(&ime, self.pointer_pos, &mut self.pending_text_events);
                self.dirty = true;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                let (delta_x, delta_y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                self.pending_scroll = Some(zephyra_fx::ScrollInput {
                    x: self.pointer_pos.0,
                    y: self.pointer_pos.1,
                    delta_x,
                    delta_y,
                });
                self.dirty = true;
            }
            _ => {}
        }
        self.sync_input();
        HostEventOutcome::Continue
    }

    fn set_theme(&mut self, variant: ThemeVariant) {
        HostWindow::set_theme(self, variant);
    }

    fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        HostWindow::set_styler(self, styler);
    }

    fn set_title(&mut self, title: &str) {
        self.window.set_title(title);
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn request_close(&mut self) {
        // 有应用侧句柄就投命令（`WindowSet` 下一轮 drain 时关窗口）；
        // 单窗口兼容路径没有句柄，调用方自己决定怎么退出。
        if let Some(handle) = &self.handle {
            handle.close();
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Vulkan 后端的 [`Backend`] 实现。
///
/// 与两个 Windows 后端同形：共享设备由 `SHARED_DEVICE` 单例保证，`create`
/// 只解析 [`WindowSpec`] 再交给 [`HostWindow::new_lenient_opts`]。
#[derive(Default)]
pub struct VulkanBackend;

impl VulkanBackend {
    /// 建一个后端入口（设备在第一个窗口创建时才真正初始化）。
    pub fn new() -> Self {
        Self
    }
}

impl Backend for VulkanBackend {
    type Window<V: View + 'static> = HostWindow<V>;

    fn create<V: View + 'static>(
        &mut self,
        event_loop: &ActiveEventLoop,
        view: V,
        spec: &WindowSpec,
        handle: WindowHandle,
    ) -> Result<Self::Window<V>, String> {
        let config = zephyra_app_config::load_from_current_dir();
        let opts = spec.resolve(&config);
        let size = opts.size.unwrap_or(DEFAULT_CLIENT_SIZE);
        let mut host = HostWindow::new_lenient_opts(event_loop, view, size, opts);
        host.window_key = handle.key();
        host.handle = Some(handle);
        Ok(host)
    }
}

impl<V: View + 'static> ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.start();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        if self.handle_event(event) == HostEventOutcome::Close {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let tick = self.tick();
        // 单窗口兼容路径：自己决定控制流。多窗口时这个决定上提到 `WindowSet`
        // （N 个窗口各自设控制流会互相覆盖，最后一个说话的决定整个循环）。
        if tick.busy {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }
}

fn trace_enabled() -> bool {
    matches!(
        std::env::var("ZEPHYRA_TRACE").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn window_handle_of(window: &Window) -> zephyra_platform::WindowHandle {
    use raw_window_handle::HasWindowHandle;
    let raw = window.window_handle().expect("window handle").as_raw();
    let usize_handle = match raw {
        raw_window_handle::RawWindowHandle::Win32(h) => h.hwnd.get() as usize,
        raw_window_handle::RawWindowHandle::Xlib(h) => h.window as usize,
        raw_window_handle::RawWindowHandle::Xcb(h) => h.window.get() as usize,
        raw_window_handle::RawWindowHandle::Wayland(h) => h.surface.as_ptr() as usize,
        _ => 0,
    };
    zephyra_platform::WindowHandle::from_raw(usize_handle)
}

fn platform_current() -> &'static dyn zephyra_platform::Platform {
    #[cfg(windows)]
    {
        zephyra_platform_win::current()
    }
    #[cfg(target_os = "linux")]
    {
        platform_current()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        zephyra_platform::current()
    }
}
