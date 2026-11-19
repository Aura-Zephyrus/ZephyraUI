//! D3D12 渲染器：根签名、PSO、描述符堆、顶点/索引缓冲与绘制命令的记录。
//!
//! # 与 D3D11 版（`zephyra-winit::renderer`）的对应关系
//!
//! | D3D11 | D3D12 |
//! |---|---|
//! | `CreateVertexShader` / `CreatePixelShader` | 无独立对象，字节码直接进 PSO |
//! | `CreateInputLayout` | 无独立对象，作为 `InputLayout` 字段随 PSO 创建 |
//! | 立即上下文 `VSSetShader` 等 | 命令列表上的 `SetPipelineState` / `SetGraphicsRootSignature` |
//! | `OMSetRenderTargets(视图对象)` | `OMSetRenderTargets(堆里的第 n 个描述符)` |
//! | 驱动隐式做资源状态转换 | **必须手工发 `ResourceBarrier`** |
//! | `Map` / `Unmap` 逐帧写顶点 | UPLOAD 堆缓冲常驻映射，写完就完 |
//! | 无根签名概念 | 根签名决定 `t0`/`s0` 怎么找描述符 |
//!
//! # 记录 vs 提交
//!
//! D3D11 的 `render()` 里所有调用**立即生效**；D3D12 的同名调用只是**往命令列表里
//! 记一条命令**，要 `Gpu::submit` 之后才交给 GPU。因此本文件的 [`Renderer::record`]
//! **只记录**，不提交 —— 提交与等待交给宿主，这样「记录 / 提交 / 等待」三步在测试
//! 里都看得见，而不是被一个函数悄悄合在一起。

#![cfg(windows)]

use std::cell::Cell;
use std::rc::Rc;
use windows::core::Interface;
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, RECT};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16_UINT;

use crate::quad::{self, RenderReport, Vertex};
use crate::shader::{self, ShaderError, Shaders};
use crate::surface::{create_resource, heap_props, texture_desc, DescriptorHeap, SurfaceError};
use zephyra_core::filter::{BlendMode, Coeff};
use zephyra_render_ext::{registration::HookContext, types::Rect as ExtRect};

/// 一张已上传的图像纹理。
///
/// # 暂存缓冲为什么跟着活这么久
///
/// `CopyTextureRegion` 是**异步**的：命令记录完就返回，GPU 可能还没读那块内存。
/// 暂存缓冲一释放就把内存还给驱动，于是 GPU 读到的是别人的数据 —— 而这类故障在
/// 画面上表现为「偶发的花屏 / 少一块」，几乎不可能归因。
///
/// 稳妥的做法是等 GPU 读完再释放（宿主那边 `submit_one_off` 已经在等）。但那要求
/// 渲染器知道「什么时候可以等」。所以这里干脆**一直留着**：图像纹理是启动期一次性
/// 建、之后只读的资源，暂存缓冲的常驻代价（`w*h*4` 字节，图标是 4KB）与正确性不成
/// 比例。真要省，用 `Vec<Option<Staging>>` 在下一次上传时替换即可。
struct UploadedTexture {
    /// 纹理本体。**只为了持有所有权** —— 绘制时着色器读的是描述符堆里那个 SRV，
    /// 而 SRV 只是堆里的一个槽位，不延长纹理的寿命。
    _resource: ID3D12Resource,
    _staging: UploadBuffer,
    /// 它在 `srv_heap` 里的槽位。槽位用尽时据此说清「一共要几张」。
    _slot: u32,
}

/// 渲染器：持有 D3D12 管线状态与常驻资源。
///
/// # 设备不归本结构所有
///
/// `ID3D12Device` 由 [`crate::surface::Gpu`] 持有并按引用传入 —— 它是整个进程唯一的
/// 一份。若渲染器自己存一份，就多出一条与 `Gpu` 并行的引用路径，「谁在用设备」会变得
/// 无法回答（同 `zephyra-winit::renderer` 的取舍）。
/// SRV 堆槽位总数：0 号图集 + 7 张图像纹理 + 1 张 `g_dst` 副本。
pub const ATLAS_SLOTS: u32 = 12;

/// `g_dst` 副本的 SRV 槽位（堆的倒数第 3 号）。
pub const DST_SRV_SLOT: u32 = ATLAS_SLOTS - 3;

/// 离屏乒乓 RT_A 的 SRV 槽位。
pub const OFFSCREEN_SRV_A: u32 = ATLAS_SLOTS - 2;

/// 离屏乒乓 RT_B 的 SRV 槽位。
pub const OFFSCREEN_SRV_B: u32 = ATLAS_SLOTS - 1;

/// `g_dst` 的懒建副本。
///
/// D3D12 的 SRV 是堆里的一个**槽位**而不是对象，所以这里只持有资源本体；
/// 视图在 `refresh_dst_copy` 里建一次，之后直接用槽位。
///
/// `state` 记的是资源**当前**处于什么状态。屏障的 `StateBefore` 必须与实际一致，
/// 否则调试层报「Before state does not match」——而那不只是报错：把真实的
/// `PIXEL_SHADER_RESOURCE` 谎报成 `COPY_DEST` 会让下一次拷贝把「上一帧还挂在
/// SRV 表里的资源」当成可写副本，静悄悄给出错画面。
struct DstCopy {
    resource: ID3D12Resource,
    size: (u32, u32),
    state: D3D12_RESOURCE_STATES,
    /// 这份副本里**有效**的区域（像素，`(x, y, w, h)`）；`None` = 整幅都有效。
    ///
    /// # 为什么需要记它
    ///
    /// MSAA 开着时解析按**区**做（`ResolveSubresourceRegion`），一次只解析
    /// 一个滤镜元素的包围盒 —— 整幅解析 1.12M 像素要 ~1.2 ms，而磨砂样板只有
    /// 160×120。但副本是**跨帧复用**的：若上一帧只解析了 A 区、这一帧来要 B 区，
    /// A∪B 之外的内容是上一次的。复用判据因此必须带上区域，见 `refresh_dst_copy`。
    resolved: Option<(u32, u32, u32, u32)>,
}

/// 解析目标（`ResolveSubresource` 的 destination）的合法状态。
///
/// # 为什么不是 `RENDER_TARGET`
///
/// `RENDER_TARGET` 当解析目标**只对交换链后缓冲成立** —— `swapchain.rs` 的
/// [`resolve_msaa`](crate::swapchain::Swapchain::resolve_msaa) 走的正是那条路，所以它
/// 一直没踩到坑。换成普通提交式纹理后，`ResolveSubresource` **照样能被记录进命令
/// 列表**，错要到 `Close()` 才现形：
///
/// ```text
/// [zephyra-d3d12] 提交失败: Close 命令列表失败: 参数错误 (0x80070057)
/// [zephyra-d3d12] 开帧失败: Reset 命令列表失败: 参数错误 (0x80070057)   ← 此后每帧
/// ```
///
/// 于是整帧提交失败、`Reset` 也失败，画面**永久冻结**在最后一帧。外部症状极具欺骗性：
/// 「点导航页面不换」「点控件没反应」—— 看着像输入/路由坏了，实际是渲染循环死了。
/// 实测（gallery，导航 state → glass）第一次点导航就触发。
const RESOLVE_DST_STATE: D3D12_RESOURCE_STATES = D3D12_RESOURCE_STATE_RESOLVE_DEST;

/// `g_dst` 副本的纹理描述。
///
/// 要求三条：格式与源一致（否则解析/采样整体错位）、单采样（解析目标必须单采样）、
/// 带 `ALLOW_RENDER_TARGET`。最后一条不是可选的：`RESOLVE_DEST` 与 `RENDER_TARGET`
/// 一样受同一条 flag 门禁，缺了它 `CreateCommittedResource` 只回「参数错误」，
/// 不提是哪个字段（`D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET` 的文档明写它
/// "enables the resource to transition into the state of ... RENDER_TARGET"）。
fn dst_copy_desc(size: (u32, u32)) -> D3D12_RESOURCE_DESC {
    texture_desc(size, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET)
}

pub struct Renderer {
    root_signature: ID3D12RootSignature,
    /// PSO 缓存，键 = (PS 变体下标, 混合模式下标, MSAA 采样数)。
    ///
    /// D3D12 的混合状态烤在 PSO 里，所以换混合模式必然换 PSO。懒建：
    /// 绝大多数帧只用默认那一个，全量预建是纯粹的启动开销。
    psos: std::cell::RefCell<std::collections::HashMap<(usize, usize, u32), ID3D12PipelineState>>,
    /// shader 可见的 CBV/SRV 堆。着色器的 `t0` 绑到其中第 0 号。
    srv_heap: DescriptorHeap,
    /// 图集的 SRV 在 `srv_heap` 里的位置（恒为 0）。
    atlas_srv_slot: u32,
    /// 图集边长（像素）。
    atlas_size: u32,
    /// 图集纹理（所有权）。
    ///
    /// D3D12 **没有**单独的采样视图对象（那是 D3D11 的概念）：SRV 是描述符堆里的
    /// 一个槽位，所以这里只有纹理本体，视图由 `srv_heap` 的槽位隐含表示。
    atlas: ID3D12Resource,
    /// 图集上传的暂存缓冲（上传堆）。
    ///
    /// 必须活得比 `record_upload_atlas` 记录的那次 `CopyTextureRegion` 久 —— 释放了
    /// 就等于让 GPU 去读一段已经归还给驱动的内存。所以挂在渲染器上，由下一次上传替换。
    atlas_staging: Option<UploadBuffer>,
    /// 输入布局（**持有** `SemanticName` 的内存）。
    ///
    /// ⚠ 只存 `descs()` 返回的**描述符数组**是不行的：那些 `SemanticName` 是指向
    /// `InputElement` 内部缓冲的**裸指针**，而 `Shaders` 在 `new` 结束时就销毁了。
    /// 悬空指针不会立刻炸 —— `CreateGraphicsPipelineState` 读到已释放内存后
    /// **不确定地**给 `E_INVALIDARG`，于是症状是「有时建得出有时建不出」，而且
    /// **每次挂的是不同的模式**。见 `InputLayout::owners` 的注释。
    elements: shader::InputLayout,
    /// MSAA 采样数。PSO 的 `SampleDesc` 必须与渲染目标一致，见 `create_pso`。
    samples: u32,
    /// 设备。
    device: ID3D12Device,
    /// 已上传的图像纹理（所有权）。**必须活得比画它的那些命令久**。
    textures: Vec<UploadedTexture>,
    /// 下一个可用的 SRV 槽位。0 号恒给图集（见 [`Renderer::new`]）。
    next_texture_slot: u32,
    /// 可复用的 SRV 槽位（纹理/ ext 卸载后回收）。
    free_texture_slots: Vec<u32>,
    /// ext 资源键 → SRV 槽位（用于卸载回收与重复分配检测）。
    ext_srv_keys: std::collections::HashMap<String, u32>,
    /// 单窗口 / 测试用的默认目标。
    ///
    /// 多窗口宿主**不碰**它 —— 每个窗口把自己的 [`WindowTargets`] 传给
    /// [`Renderer::record_with`]。它在 [`Renderer::record`] 借用期间是 `None`
    /// （take / restore），除此之外恒为 `Some`。
    default_targets: Option<WindowTargets>,
}

/// 一个窗口私有的渲染目标与几何缓冲。
///
/// # 为什么这部分不能共享
///
/// [`Renderer`] 里共享的是「怎么画」：PSO、图集、SRV 堆、纹理表。而这里装的是
/// 「画到哪儿」以及「本窗口上一帧画的是什么」：
///
/// - 离屏 RT 与目标副本的尺寸跟着窗口走（窗口 A 1200×800、工具窗口 B 420×600，
///   两者的离屏 RT 不能是同一张）；
/// - 几何缓冲的上传游标与「上次上传的字节」逐窗口独立 —— 两个窗口每帧的顶点流
///   不同，共用一份影子缓存会让其中一个窗口误判「没变」而跳过上传，症状是
///   「工具窗口的内容画进了主窗口」；
/// - 外部渲染源钩子按窗口登记。
///
/// # 线程
///
/// `WindowTargets` 跟着 [`crate::host::HostWindow`] 走，**不是**进程级共享的。
/// 共享的是 [`Renderer`]（见 `crate::host::Shared`）。
pub struct WindowTargets {
    /// 上传堆 + 默认堆的逐帧几何缓冲。
    vertex: StreamBuffer,
    /// 索引缓冲（**逐帧重写**，内容由 [`quad::expand_indices`] 生成）。
    index: StreamBuffer,
    /// 顶点缓冲已分配字节数，用于验证稳态不增长。
    vertex_capacity: u64,
    /// 跨帧复用的展开缓冲（顶点流 + 索引 + 分批）。
    scratch: quad::Scratch,
    /// 上一次真正上传的顶点字节。逐字节比较后决定要不要重传（见 upload_geometry）。
    last_vertex_bytes: Vec<u8>,
    /// 上一次真正上传的索引字节。
    last_index_bytes: Vec<u8>,
    /// 离屏 pass 的两张乒乓 RT（E4.4 / E4.6）。见 [`OffscreenPair`]。
    offscreen: std::cell::RefCell<Option<OffscreenPair>>,
    /// 离屏 RT 专用的 RTV 描述符堆（2 槽）。
    offscreen_rtv_heap: DescriptorHeap,
    /// `g_dst`（目标副本）的懒建纹理，见 [`Renderer::refresh_dst_copy`]。
    dst_copy: std::cell::RefCell<Option<DstCopy>>,
    /// 目标里**内容**的版本号。
    ///
    /// # 它解决什么
    ///
    /// `g_dst` 是「元素背后已经画在屏幕上的画面」的快照。折射 / 透镜 / 磨砂
    /// 都要读它，于是**每个带滤镜的元素都要拷一次目标**。
    ///
    /// 旧实现里那次拷贝是**整张**：MSAA 开着时它是一次
    /// `ResolveSubresource`（整幅解析，4× 采样读 + 全屏写）。而
    /// D3D12 **没有**「按区解析」——只有整幅。
    ///
    /// 实测（gallery，1400×800，Intel UHD 630）：材质页一帧 7 个带滤镜的
    /// 元素 ⇒ 7 次全屏解析 ⇒ **5.2 ms/帧**，占那一帧 GPU 开销的 26%。
    ///
    /// 于是改成「**一帧只拷一次**」：第一趟滤镜取一次副本，同帧后续的
    /// 滤镜趟直接复用。
    ///
    /// # 语义变化（一处，且是刻意的）
    ///
    /// 复用意味着**同帧内后面的滤镜看到的是「本帧第一趟滤镜之前」的画面**
    /// —— 也就是「上层玻璃的背景里不含下层玻璃」。
    ///
    /// 物理上玻璃叠玻璃应当互相透出，但 UI 里的「背景」指的是**页面内容**
    /// 而不是另一块玻璃 —— 而把它做成「不含」反而让结果**可预期**
    /// （不会出现「A 的玻璃里浮着 B 的玻璃」这种说不清的层次）。
    ///
    /// 实测（gallery 材质页 / 参考页，1400×800）：改前改后**逐像素相同**
    /// （0 个像素有差异）—— 那些页面上玻璃卡之间本来就没有重叠。
    ///
    /// # 谁推进它
    ///
    /// - 帧首（[`Renderer::record`]）：+1 —— 新的一帧，上一帧的副本作废。
    /// - 主路径画进目标（[`Renderer::replay`]，非离屏段）：+1。
    /// - **玻璃 / 挖洞投影的合成趟：不动它**。
    ///
    /// 最后一条是刻意的：玻璃的合成趟画进目标，但**不是**「内容」——
    /// 它是「这一块玻璃自己」。若把它算进版本号，同帧里第二块玻璃就
    /// 会因为「第一块玻璃画进去了」而重拷副本（而那块玻璃多半不在
    /// 第二块玻璃背后，白拷一次全屏解析）。
    ///
    /// 可分离混合那条路（`replay` 里的 PS 变体）**不走缓存**
    /// （`allow_reuse = false`）：它要的是**此刻**的目标。
    content_revision: std::cell::Cell<u64>,
    /// 当前这份 `g_dst` 副本是在哪个内容版本上取的。
    dst_copy_revision: std::cell::Cell<u64>,
    /// 外部渲染源钩子（按 `RenderSource::kind` 线性查表；钩子个位数，不值得哈希）。
    hooks: Vec<(std::borrow::Cow<'static, str>, Box<dyn RenderHook>)>,
    /// ext 共享状态（与 ExtContextHandle 同步）。
    pub(crate) ext_pool: std::rc::Rc<std::cell::RefCell<crate::ext::ExtPool>>,
    pub(crate) ext_hooks: std::rc::Rc<std::cell::RefCell<crate::ext::HookTable>>,
    pub(crate) ext_frame: std::rc::Rc<std::cell::Cell<u64>>,
    /// 本窗口 id（P1.1：窗口作用域的 ext hook 必须按 id 精确匹配，不能只按 kind）。
    pub(crate) window_id: u64,
}

impl WindowTargets {
    /// 为一个窗口建一套渲染目标与几何缓冲。
    ///
    /// 起始 64KB 顶点 / 4KB 索引，稳态下按需增长到 2 的幂，之后不再动
    /// （容量判据见 `Renderer::upload_geometry` 与 `Renderer::draw_pass_quad`）。
    pub fn new(device: &ID3D12Device) -> Result<Self, SurfaceError> {
        let vertex = StreamBuffer::new(
            device,
            64 * 1024,
            D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER,
        )?;
        let index = StreamBuffer::new(device, 4 * 1024, D3D12_RESOURCE_STATE_INDEX_BUFFER)?;
        let vertex_capacity = vertex.size;
        let offscreen_rtv_heap = DescriptorHeap::new_rtv(device, 2)?;
        Ok(Self {
            vertex,
            index,
            vertex_capacity,
            scratch: quad::Scratch::new(),
            last_vertex_bytes: Vec::new(),
            last_index_bytes: Vec::new(),
            offscreen: std::cell::RefCell::new(None),
            offscreen_rtv_heap,
            dst_copy: std::cell::RefCell::new(None),
            content_revision: std::cell::Cell::new(0),
            dst_copy_revision: std::cell::Cell::new(u64::MAX),
            hooks: Vec::new(),
            ext_pool: std::rc::Rc::new(std::cell::RefCell::new(crate::ext::ExtPool::default())),
            ext_hooks: std::rc::Rc::new(std::cell::RefCell::new(crate::ext::HookTable::default())),
            ext_frame: std::rc::Rc::new(std::cell::Cell::new(0)),
            window_id: 0,
        })
    }

    /// 顶点缓冲容量（字节）。稳态下跨帧不变。
    pub fn vertex_capacity(&self) -> u64 {
        self.vertex_capacity
    }

    pub fn ext_handle(&self, device: &ID3D12Device) -> zephyra_ext::ExtContextHandle {
        crate::ext::new_ext_handle(
            device.clone(),
            self.ext_pool.clone(),
            self.ext_hooks.clone(),
            self.ext_frame.clone(),
        )
    }
}

/// 上传堆上一块常驻映射的暂存缓冲。
///
/// # 上传堆的资源**只能**处于 `GENERIC_READ`，不能做状态转换
///
/// 这是 D3D12 的一条硬规则（上传堆的可写性以牺牲状态灵活性为代价）。所以上传堆缓冲
/// **不能**直接当顶点缓冲用 —— 那需要 `VERTEX_AND_CONSTANT_BUFFER` 状态，而这条转换
/// 在上传堆资源上是非法的。
///
/// 因此走 [`StreamBuffer`] 的「上传 → 拷贝 → 默认堆」三步。
pub(crate) struct UploadBuffer {
    pub(crate) resource: ID3D12Resource,
    /// 已映射的 CPU 地址（`resource` 拥有底层内存，这里只是借一个指针）。
    pub(crate) mapped: *mut u8,
    /// 这块缓冲的**容量**（字节）。
    ///
    /// 有了它才能判断「上次那块还装得下吗」—— 而「装得下就复用」是这里
    /// 最重要的一条性能规则（见 [`Renderer::record_upload_atlas`]）。
    pub(crate) capacity: u64,
}

// 上传缓冲的所有权在 `resource` 上；`mapped` 是它的别名，Rust 不知道这层关系。
// 这里显式声明：该缓冲的 CPU 指针只在单线程记录命令时使用。
unsafe impl Send for UploadBuffer {}
unsafe impl Sync for UploadBuffer {}

/// 逐帧重写的几何缓冲：上传堆暂存 + 默认堆常驻。
///
/// # 每帧固定四步一循环
///
/// ```text
/// CPU 写 staging（常驻映射，无需屏障）
/// staging: GENERIC_READ → COPY_SRC ─┐
/// resident: VERTEX/INDEX → COPY_DEST ├→ CopyBufferRegion
/// resident: COPY_DEST → VERTEX/INDEX ┘
/// staging: COPY_SRC → GENERIC_READ（为下一帧的 CPU 写让路）
/// ```
///
/// # 为什么不直接把上传堆缓冲绑给 draw
///
/// 上传堆资源**不允许**状态转换，而索引缓冲被 draw 读取时要求处于 `INDEX_BUFFER`。
/// 看似"上传堆内存本来就 CPU 可读、直接绑就行"，但零售驱动上会拿到未定义行为 ——
/// 而这正是本项目最讨厌的那类故障（不报错、画面乱）。所以走 `CopyBufferRegion`：
/// 多一次拷贝，换来"每一步状态转换都是文档规定的"，调试层能查、零售驱动不会崩。
///
/// 每帧多出的那次拷贝对 UI 来说可以忽略（几百 KB 量级），而正确性不能。
struct StreamBuffer {
    /// 默认堆上、绘制时处于 `VERTEX_AND_CONSTANT_BUFFER` / `INDEX_BUFFER` 的缓冲。
    resident: ID3D12Resource,
    /// 上传堆上的暂存，常驻映射。
    staging: UploadBuffer,
    /// 常驻缓冲的字节数，也是暂存的字节数。
    size: u64,
    /// 绘制时它必须处于哪个状态（决定屏障的两端）。
    draw_state: D3D12_RESOURCE_STATES,
    /// 暂存区里的**写游标**（字节）。每次上传都从这里划走一段对齐后的独占空间。
    ///
    /// # 为什么暂存区不能只用同一个偏移
    ///
    /// `staging.mapped` 是**常驻映射**的 CPU ptr_of，`CopyBufferRegion` 只是**记录**一条
    /// 拷贝指令 —— 真正的读取发生在 GPU 跑到那条指令的时候，而那时本次 `record` 里的
    /// **所有** CPU 写入都早已发生。
    ///
    /// 每次都写 0 号偏移的话，帧内 N 次上传会互相覆盖，GPU 最终读到的是**最后一次**
    /// 上传的内容：整帧的所有 draw 都用同一份顶点/索引。微软《Uploading different
    /// types of resources》里的 D3D12 例子正是为此维护 `m_pDataCur` 每次推进，
    /// 《Fence-based resource management》把它正式称为「上传区的 ring buffer」。
    ///
    /// # 症状为什么这么难认
    ///
    /// 整帧退化成「一份顶点」，画面不是花屏而是**一个死值**：模糊结果整张同一个灰度。
    /// 而只要同一条命令的「内容 / 合成 / 重画」三份顶点碰巧相同（例如投影那种三者
    /// 画的是同一个矩形），画面就**完全正确** —— 于是「拆开写」和「写错」同时存在，
    /// 测试却是绿的。本仓库已经吃过一次同型亏（D3D11 的 `ATLAS_SLOTS` 被局部字面量
    /// 遮蔽），所以这里把每段上传的偏移都单独记一笔。
    ///
    /// # 游标为什么可以在帧首归零
    ///
    /// [`crate::surface::Gpu::begin_frame`] **第一件事就是 `wait_gpu`**
    /// （见该函数的注释「上传发生在 GPU 已空闲之后」）。所以进入 [`Renderer::record`]
    /// 时，GPU 已经读完了上一帧的全部拷贝，归零不会踩到未读的数据 —— 这是
    /// `begin_frame` 已经替我们保证的前提，不是这里额外做的假设。
    head: std::cell::Cell<u64>,
    /// 还没做过任何一次上传（`resident` 仍停在 `COMMON`）。
    fresh: std::cell::Cell<bool>,
    /// 帧内被替换掉的旧 `StreamBuffer`，留到帧尾（`record` 帧首清空）。
    ///
    /// 帧内扩容发生在**已经上传过之后**：此刻本次 `record` 里记下了往旧块的
    /// `CopyBufferRegion`。GPU 要等执行到那条命令才读完，所以旧块不能当场丢 ——
    /// 见 [`Self::reserve`]。
    retire: Vec<StreamBuffer>,
}

/// 一次 `record` 的输入。
///
/// 打包成结构体而不是六个位置参数：位置参数多到第 5 个之后就没法靠记忆区分了，而
/// 传错 `size` 与 `clear` 的后果（画面拉伸 / 底色错）都不带任何编译期信号。
pub struct RenderCtx<'a> {
    /// 设备（建资源时用；顶点缓冲扩容走这条路）。
    pub device: &'a ID3D12Device,
    /// 记录用的命令列表（必须处于**记录中**状态，即 `Gpu::begin_frame` 之后）。
    pub list: &'a ID3D12GraphicsCommandList,
    /// 目标渲染目标的 CPU 描述符句柄（离屏目标的 `rtv` 或交换链后缓冲的句柄）。
    pub rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    /// 要回放的命令列表。
    pub display_list: &'a zephyra_core::paint::DisplayList,
    /// 目标像素尺寸。
    pub size: (u32, u32),
    /// 清屏颜色（线性 RGBA）。
    pub clear: zephyra_core::core::Color,
    /// 图像纹理表（名字 → SRV 描述符位置）。
    pub textures: &'a TextureTable,
    /// 本帧脏区，用于**帧内** scissor。
    ///
    /// # ⚠ 它不能用来「跳过脏区外的回放」
    ///
    /// 本后端是 flip model（`FLIP_DISCARD`，DComp 路径 `FLIP_SEQUENTIAL`），
    /// `Present` **之后**后缓冲内容是**未定义**的。所以任何要 present 的帧都必须
    /// 重画整个目标 —— 顶点流永远整份提交，只有**光栅化范围**能被裁掉。
    ///
    /// ⚠ 而且 D3D12 的 scissor 与视口**必须同时**设（见下面第 6 步）：只设视口忘了
    /// 裁剪，或反过来，都得到「什么都不画且零错误」。
    /// 目标渲染目标的**资源**（不是 RTV 句柄 —— D3D12 的 RTV 拿不到资源）。
    ///
    /// 只有当本帧里出现可分离 / 非可分离混合模式（它们的 PS 要读 `dst`）时才用
    /// 得上：宿主先把目标拷进一张单采样纹理，再当 SRV 绑到 `t1`。
    /// `None` = 宿主没给 ⇒ 那 14 个模式退回 `SrcOver`（而不是画出一个颜色错误
    /// 的结果）。
    ///
    /// # 为什么不是必填
    ///
    /// 绝大多数帧只用默认模式，让宿主为此多拿一个资源是纯粹的成本。
    pub rt: Option<&'a ID3D12Resource>,
    pub damage: zephyra_core::frame::Damage,
}

/// 图像纹理表：名字 → `srv_heap` 里的描述符位置。
///
/// # 与 D3D11 版的差别
///
/// D3D11 版的表直接存 `ID3D11ShaderResourceView` 对象；D3D12 里「SRV」不是对象，而是
/// **描述符堆里的一个槽位**。所以这里存的是 `u32` 位置，`record` 时按位置取 GPU 句柄。
pub struct TextureTable {
    entries: std::collections::HashMap<String, u32>,
}

impl TextureTable {
    /// 新建一张表。
    ///
    /// # 为什么槽位只增不减
    ///
    /// `srv_heap` 的槽位是**描述符堆里的地址**，不是引用计数。忘掉一个名字并不
    /// 意味着那个地址可以被安全地交给别人 —— 驱动的描述符缓存可能还持有指向它的
    /// GPU 可见状态。要回收就得显式走「销毁纹理 + 把槽位标为自由」的流程，
    /// 那需要跟踪哪些槽位是活的（[`Renderer`] 里的 `next_texture_slot` 只增）。
    ///
    /// 对 UI 来说这没关系：图像纹理的种类数是有界的（小图标集），而槽位数在
    /// [`Renderer::new`] 里按 `ATLAS_SLOTS` 固定。**纹理种类超过槽位时报错**，
    /// 而不是静默覆盖别人的槽位。
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
        }
    }

    /// 登记一个纹理名 → 描述符位置。
    pub fn insert(&mut self, name: String, slot: u32) {
        self.entries.insert(name, slot);
    }

    /// 这个名字有没有登记。
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// 取描述符位置。
    pub fn slot(&self, name: &str) -> Option<u32> {
        self.entries.get(name).copied()
    }

    /// 忘掉一个名字。返回它原来占的槽位，好让调用方决定是否回收。
    ///
    /// **不**改描述符堆 —— 见 [`Self::insert`] 的说明：槽位只能单调增长。
    pub fn remove(&mut self, name: &str) -> Option<u32> {
        self.entries.remove(name)
    }

    /// 登记了几张纹理。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 一张都没登记。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for TextureTable {
    fn default() -> Self {
        Self::new()
    }
}

/// `Coeff` 到 D3D12 的**颜色**通道混合因子。
///
/// 与 D3D11 那边的 `d3d11_factor` **逐项一致**：两边各写一份是刻意的（各自的后端
/// 枚举类型不同），但内容必须一样，否则同一个模式在两个后端上画出来不一样。
///
/// **穷尽**匹配：漏一个分支是编译错误，而不是运行时才暴露的静默错误。
fn d3d12_factor(c: Coeff) -> D3D12_BLEND {
    match c {
        Coeff::Zero => D3D12_BLEND_ZERO,
        Coeff::One => D3D12_BLEND_ONE,
        Coeff::SrcColor => D3D12_BLEND_SRC_COLOR,
        Coeff::OneMinusSrcColor => D3D12_BLEND_INV_SRC_COLOR,
        Coeff::DstColor => D3D12_BLEND_DEST_COLOR,
        Coeff::OneMinusDstColor => D3D12_BLEND_INV_DEST_COLOR,
        Coeff::SrcAlpha => D3D12_BLEND_SRC_ALPHA,
        Coeff::OneMinusSrcAlpha => D3D12_BLEND_INV_SRC_ALPHA,
        Coeff::DstAlpha => D3D12_BLEND_DEST_ALPHA,
        Coeff::OneMinusDstAlpha => D3D12_BLEND_INV_DEST_ALPHA,
    }
}

/// `Coeff` 到 D3D12 的 **alpha 通道**混合因子。
///
/// # 为什么不能直接复用 [`d3d12_factor`]
///
/// D3D11 与 D3D12 在这一点上**同样限制**：alpha 通道只接受
/// `ZERO` / `ONE` / `SRC_ALPHA` / `INV_SRC_ALPHA` / `DEST_ALPHA` / `INV_DEST_ALPHA`，
/// 颜色因子（`SRC_COLOR` 等）在 alpha 通道上是非法的 —— D3D12 会在
/// `CreateGraphicsPipelineState` 处给 `E_INVALIDARG`，症状同样是「建不出 PSO」
/// 却不指出是哪个字段。
///
/// 正确映射同样是"颜色系数 → 它的 alpha 对偶"（`Cs → as`，`1−Cs → 1−as`，
/// `Cd → ad`，`1−Cd → 1−ad`）—— 这不是近似，是 Porter-Duff 算子在预乘空间下的
/// 正确展开。见 D3D11 那边 `d3d11_alpha_factor` 的完整推导表。
fn d3d12_alpha_factor(c: Coeff) -> D3D12_BLEND {
    match c {
        Coeff::Zero => D3D12_BLEND_ZERO,
        Coeff::One => D3D12_BLEND_ONE,
        // 颜色系数 → alpha 对偶，理由见上。
        Coeff::SrcColor => D3D12_BLEND_SRC_ALPHA,
        Coeff::OneMinusSrcColor => D3D12_BLEND_INV_SRC_ALPHA,
        Coeff::DstColor => D3D12_BLEND_DEST_ALPHA,
        Coeff::OneMinusDstColor => D3D12_BLEND_INV_DEST_ALPHA,
        Coeff::SrcAlpha => D3D12_BLEND_SRC_ALPHA,
        Coeff::OneMinusSrcAlpha => D3D12_BLEND_INV_SRC_ALPHA,
        Coeff::DstAlpha => D3D12_BLEND_DEST_ALPHA,
        Coeff::OneMinusDstAlpha => D3D12_BLEND_INV_DEST_ALPHA,
    }
}

/// PS 合成段（可分离 / 非可分离模式、模糊、合成）用的混合状态：**`Src`**（ONE/ZERO）。
///
/// 那些像素着色器已经把 `dst` 算进去、写出了**最终**颜色，固定功能方程只能"原样
/// 接受"。配成 `SrcOver` 会再叠一次 ⇒ 整体过曝。
///
/// D3D12 与 D3D11 的差别在这里很实在：D3D11 可以逐段换 `ID3D11BlendState`，
/// D3D12 的混合状态**烤在 PSO 里**，所以"换混合状态"在这里等于"换 PSO"。
const fn src_blend_desc() -> D3D12_BLEND_DESC {
    let rt = D3D12_RENDER_TARGET_BLEND_DESC {
        BlendEnable: windows::core::BOOL(1),
        LogicOpEnable: windows::core::BOOL(0),
        SrcBlend: D3D12_BLEND_ONE,
        DestBlend: D3D12_BLEND_ZERO,
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: D3D12_BLEND_ONE,
        DestBlendAlpha: D3D12_BLEND_ZERO,
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        // windows-rs 0.62 把写掩码声明成了**裸 `u8`**（不是 `D3D12_COLOR_WRITE_ENABLE`），
        // 与 D3D11 那边的怪癖同源。
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };
    D3D12_BLEND_DESC {
        AlphaToCoverageEnable: windows::core::BOOL(0),
        IndependentBlendEnable: windows::core::BOOL(0),
        RenderTarget: [rt; 8],
    }
}

/// 第 `mode` 号混合模式的 D3D12 混合描述。
///
/// 系数可表达的模式从 [`BlendMode::as_coeff`] **推导**（绝不手抄一张表 —— 手抄
/// 是本仓最出过事的一类错误：改了 `as_coeff` 忘了同步拷贝的表，两边静默漂移）；
/// 其余的用 [`src_blend_desc`]。
fn blend_desc_for(mode: usize) -> Option<D3D12_BLEND_DESC> {
    let bm = mode_by_index(mode)?;
    let (ks, kd) = bm.as_coeff()?;
    let rt = D3D12_RENDER_TARGET_BLEND_DESC {
        BlendEnable: windows::core::BOOL(1),
        LogicOpEnable: windows::core::BOOL(0),
        SrcBlend: d3d12_factor(ks),
        DestBlend: d3d12_factor(kd),
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: d3d12_alpha_factor(ks),
        DestBlendAlpha: d3d12_alpha_factor(kd),
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };
    Some(D3D12_BLEND_DESC {
        AlphaToCoverageEnable: windows::core::BOOL(0),
        IndependentBlendEnable: windows::core::BOOL(0),
        RenderTarget: [rt; 8],
    })
}

/// 按**枚举声明顺序**取第 `i` 个混合模式（与 D3D11 那边 `mode_by_index` 一致）。
fn mode_by_index(i: usize) -> Option<BlendMode> {
    let all = [
        BlendMode::Clear,
        BlendMode::Src,
        BlendMode::Dst,
        BlendMode::SrcOver,
        BlendMode::DstOver,
        BlendMode::SrcIn,
        BlendMode::DstIn,
        BlendMode::SrcOut,
        BlendMode::DstOut,
        BlendMode::SrcATop,
        BlendMode::DstATop,
        BlendMode::Xor,
        BlendMode::Plus,
        BlendMode::Modulate,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Multiply,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];
    all.get(i).copied()
}

// ═══════════════════════════════════════════════════════════════════════════
//  E4.4 / E4.5 / E4.6：离屏 pass（D3D12 版）
// ═══════════════════════════════════════════════════════════════════════════
//
//  执行顺序、参数、乒乓规则全部来自 `zephyra_fx::offscreen` 的计划 —— 这边只剩
//  「照着计划设状态、画一个铺满四边形」。两边各写一遍判断逻辑的话，早晚有一边
//  顺序反了，而症状只是「某个元素的阴影糊的方向不对」。
//
//  ── D3D12 特有的三件事 ──
//
//  1. **RTV 描述符堆**：`Renderer` 自带一个 2 槽的 RTV 堆给离屏对用。交换链的
//     RTV 堆不能借用 —— 它在 `ResizeBuffers` 时会被重建，而离屏 RT 与窗口无关。
//  2. **资源状态推进**：每趟模糊都要把源切到 `PIXEL_SHADER_RESOURCE`、把目标切回
//     `RENDER_TARGET`，而且当前状态是**记录期**的一份推理（见 `render_offscreen`）。
//  3. **屏障必须在资源解除绑定之后**：见 [`rt_barrier`]。

/// 离屏 pass 用的一对乒乓 RT（D3D12 版）。
///
/// 描述符位置固定：RTV 堆 0/1 号、SRV 堆 [`OFFSCREEN_SRV_A`] / [`OFFSCREEN_SRV_B`]。
/// 不用动态分配是因为它们只有两张，位置固定省掉「哪个槽是谁」这类状态 —— 而这类
/// 状态一旦与 `dst_copy` 的槽位算错，症状是模糊采样到了上一帧的副本。
#[derive(Default)]
struct OffscreenPair {
    a: Option<OffscreenRt>,
    b: Option<OffscreenRt>,
}

type RtState = Rc<Cell<D3D12_RESOURCE_STATES>>;

/// 一张离屏 RT（资源 + RTV 槽位 + SRV 槽位）。
struct OffscreenRt {
    resource: ID3D12Resource,
    /// RTV 在**自有** RTV 堆里的槽位。
    rtv_slot: u32,
    /// SRV 在 `srv_heap` 里的槽位（shader-visible）。
    srv_slot: u32,
    size: (u32, u32),
    /// 当前资源状态。刚建出来时是 `RENDER_TARGET`（`ALLOW_RENDER_TARGET` 的硬性要求）。
    ///
    /// # 为什么状态不能放在 `render_offscreen` 的局部变量里
    ///
    /// 这对 RT 是**跨帧缓存**的（只有尺寸变了才重建），而 D3D12 的屏障必须「从**当前**
    /// 状态到目标状态」—— 当前状态是**记录期**的一份推理，不是资源里的字段。
    ///
    /// 每次进来重置成 `RENDER_TARGET`，**第一帧是对的**（建出来就是 `RENDER_TARGET`），
    /// 第二帧起全错：上一帧收尾时末趟结果停在 `PIXEL_SHADER_RESOURCE`，于是第一道屏障
    /// 变成 `PSR → PSR` 被 [`rt_barrier`] 当空转换吃掉，而真正需要的 `RENDER_TARGET`
    /// 那一道永远发不出去。症状是「单帧测试全绿、跑第二帧开始整张图变成一个死值」。
    ///
    /// 用 `Rc<Cell<..>>` 而不是 `Cell`：记录期要把它从缓存里**克隆**出来用（`&mut self`
    /// 与 `&targets.offscreen` 的借用链没法共存，见 [`offscreen_pair`]），而克隆出来的必须
    /// 是**同一个**格子。
    state: RtState,
}

impl Clone for OffscreenRt {
    fn clone(&self) -> Self {
        Self {
            resource: self.resource.clone(),
            rtv_slot: self.rtv_slot,
            srv_slot: self.srv_slot,
            size: self.size,
            // ⚠ 状态**共享**而不是复制：克隆出来的是缓存里那一张的句柄别名，它写的状态
            // 必须落回同一个格子，否则跨帧跟踪就断了。
            state: self.state.clone(),
        }
    }
}

impl OffscreenRt {
    /// 建一张离屏 RT 并把它的 RTV / SRV 描述符写好。
    ///
    /// # 为什么 RT 是单采样的
    ///
    /// 模糊核要在源与目标之间乒乓，两张都必须是 `PIXEL_SHADER_RESOURCE`，而 MS
    /// 纹理只能被 resolve、不能被采样。所以离屏 RT 恒为单采样 —— 这同时决定了模糊
    /// 趟的 PSO 必须用 `samples = 1`（见文件头的第 3 条）。
    ///
    /// # 初始状态为什么是 `RENDER_TARGET`
    ///
    /// 我们建它出来就是为了往里画内容。`ALLOW_RENDER_TARGET` 标志要求初始状态包含
    /// `RENDER_TARGET`，否则 `CreateCommittedResource` 直接 `E_INVALIDARG` ——
    /// 而错误信息完全不提「初始状态」这几个字。
    fn new(
        device: &ID3D12Device,
        rtv_heap: &DescriptorHeap,
        srv_heap: &DescriptorHeap,
        rtv_slot: u32,
        srv_slot: u32,
        size: (u32, u32),
    ) -> Option<Self> {
        let resource = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_DEFAULT),
            &texture_desc(size, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        )
        .ok()?;
        unsafe {
            device.CreateRenderTargetView(&resource, None, rtv_heap.cpu(rtv_slot).ok()?);
            device.CreateShaderResourceView(&resource, None, srv_heap.cpu(srv_slot).ok()?);
        }
        Some(Self {
            resource,
            rtv_slot,
            srv_slot,
            size,
            state: Rc::new(Cell::new(D3D12_RESOURCE_STATE_RENDER_TARGET)),
        })
    }

    fn rtv(&self, heap: &DescriptorHeap) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE, SurfaceError> {
        heap.cpu(self.rtv_slot)
    }

    fn srv(&self, heap: &DescriptorHeap) -> Result<D3D12_GPU_DESCRIPTOR_HANDLE, SurfaceError> {
        heap.gpu(self.srv_slot)
    }
}

/// 取（必要时建）一对尺寸为 `size` 的离屏 RT。尺寸变了就**整对**重建。
///
/// 只换一张会让另一张仍按旧尺寸建造，拿它当写入目标时报尺寸不匹配，而错误信息指向
/// 「格式」，极具误导性。
fn offscreen_pair(
    slot: &std::cell::RefCell<Option<OffscreenPair>>,
    device: &ID3D12Device,
    rtv_heap: &DescriptorHeap,
    srv_heap: &DescriptorHeap,
    size: (u32, u32),
) -> Option<(OffscreenRt, OffscreenRt)> {
    if size.0 == 0 || size.1 == 0 {
        return None;
    }
    let mut slot = slot.borrow_mut();
    let need_new = match (
        slot.as_ref().and_then(|p| p.a.as_ref()),
        slot.as_ref().and_then(|p| p.b.as_ref()),
    ) {
        (Some(a), Some(b)) => a.size != size || b.size != size,
        _ => true,
    };
    if need_new {
        *slot = Some(OffscreenPair {
            a: OffscreenRt::new(device, rtv_heap, srv_heap, 0, OFFSCREEN_SRV_A, size),
            b: OffscreenRt::new(device, rtv_heap, srv_heap, 1, OFFSCREEN_SRV_B, size),
        });
    }
    // 返回**拥有所有权**的两份，把 `targets.offscreen` 的借用在这里结束掉 —— 否则整个
    // 离屏 pass 期间 `self` 都不可变借用，后面的 `&mut self` 调用（上传顶点、换绑
    // 缓冲）全编译不过。
    Some((slot.as_ref()?.a.clone()?, slot.as_ref()?.b.clone()?))
}

/// ⚠ **资源必须先从 `OMSetRenderTargets` 上解绑，才能发它的状态屏障。**
///
/// D3D12 明确禁止「把当前仍绑在 RTV 上的资源转出 `RENDER_TARGET`」。调试层报的是
/// 「resource is currently set as a render target」，而症状是**整张图变成一个死值**
/// —— 与「PS 绑错了」「SRV 描述符指错了」完全同形。
///
/// 实测踩过：模糊趟里先发「读源 → PSR」的屏障、再在 `draw_pass_quad` 里绑目标，
/// 每趟都撞一次，于是第一趟就把内容冲掉了。正确顺序是**先绑目标、再发屏障**。
///
/// # ⚠ `before == after` 的屏障是 **`E_INVALIDARG`**
///
/// 乒乓的前两趟恰好会撞上：第一趟里「要写的 B」本来就还是 `RENDER_TARGET`（刚建
/// 出来、还没被写过），于是 `RENDER_TARGET → RENDER_TARGET` 就是一次空转换。症状是
/// `Gpu::submit` 里 `Close 命令列表` 返回 `E_INVALIDARG` —— 报错位置离真正的原因隔了
/// 整个离屏流程。
fn rt_barrier(
    list: &ID3D12GraphicsCommandList,
    res: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    if before == after {
        return;
    }
    crate::surface::Gpu::barrier(list, res, before, after);
}

/// 每段（普通段 / 离屏段）都要用到的环境。
///
/// # 为什么要打包而不是八个位置参数
///
/// 位置参数多到第 5 个之后就没法靠记忆区分，而传错 `rtv` 与 `size` 的后果（画进
/// 离屏 RT / 视口尺寸不对）**都不带任何编译期信号**。离屏 pass 里尤其危险：`rtv`
/// 会被换成离屏 RT、尺寸会换成 RT 尺寸，而两者都是「看起来合法」的另一个值。
struct CmdEnv<'a> {
    /// 目标渲染目标的 CPU 描述符句柄（离屏段会换成离屏 RT 的）。
    rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    /// 当前目标像素尺寸（离屏段会换成 RT 尺寸）。
    size: (u32, u32),
    textures: &'a TextureTable,
    /// 扩展纹理 `resource_key` → SRV 槽位快照（避免与 `&mut WindowTargets` 冲突）。
    ext_slots: Option<&'a std::collections::HashMap<String, u32>>,
    /// 目标的**资源**（`refresh_dst_copy` 拿它当拷贝**源**）。
    ///
    /// # 它**不**用于发目标的状态屏障
    ///
    /// 目标的状态由**宿主**负责：`Offscreen` 建出来就是 `RENDER_TARGET`；交换链
    /// 路径由宿主在帧首 `barrier_to_render_target`、帧尾 `barrier_to_present`。而离屏
    /// pass 只在**离屏 RT 之间**切状态，从不碰目标。
    ///
    /// 无条件发一道 `PIXEL_SHADER_RESOURCE → RENDER_TARGET` 是**错的**：帧首时目标
    /// 处在 `PRESENT`，从 `PIXEL_SHADER_RESOURCE` 转换会被调试层判为非法。
    rt: Option<&'a ID3D12Resource>,
    damage: zephyra_core::frame::Damage,
    /// 离屏那条命令的 `SortKey`（造单条列表时要用）。
    key: zephyra_core::paint::SortKey,
}

/// 命令的混合模式（按 [`zephyra_core::filter::BlendMode`] 下标）；剪裁标记没有。
///
/// # 为什么渲染器自己匹配
///
/// 合成趟是**唯一**需要「命令的 blend」的地方（普通段走 `DrawRun.blend`，
/// 已按段切好）。与 D3D11 侧同名的助手并列存在而不是抽去 core：两个后端的
/// 合成路径本来就是两份实现，共用一个助手只会让「改一处忘了另一处」变成
/// 「改一处悄悄影响两边」。
fn command_blend_mode(cmd: &zephyra_core::paint::Command) -> Option<usize> {
    use zephyra_core::paint::Command;
    match cmd {
        Command::Quad { blend, .. }
        | Command::Path { blend, .. }
        | Command::Image { blend, .. }
        | Command::Text { blend, .. }
        | Command::Custom { blend, .. } => Some(*blend as usize),
        Command::PushClip(_) | Command::PopClip => None,
    }
}

/// 外部渲染源在 **D3D12 后端**的接入点。
pub trait RenderHook: Send + Sync {
    /// 在 `bounds`（屏幕像素坐标）处绘制自绘内容。
    fn draw(
        &mut self,
        ctx: HookCtx,
        bounds: zephyra_core::core::Rect,
        source: &zephyra_core::paint::RenderSource,
    );
    fn draw_with_key(
        &mut self,
        ctx: HookCtx,
        bounds: zephyra_core::core::Rect,
        source: &zephyra_core::paint::RenderSource,
        _resource_key: Option<&str>,
    ) {
        self.draw(ctx, bounds, source);
    }
}

/// 传给 [`RenderHook`] 的 D3D12 上下文。
///
/// # 为什么字段是裸指针而不是引用
///
/// `HookContext::as_any_mut` 必须返回 `&mut dyn Any`，而 `dyn Any: 'static` ——
/// 带生命周期的 `HookCtx<'a>` 无法向下转型。用裸指针（`'static`）存设备与命令
/// 列表，再通过 [`HookCtx::device`] / [`HookCtx::list`] 在 `&self` / `&mut self`
/// 的生命周期内安全取回引用：钩子 `downcast_mut::<HookCtx>()` 成功，且返回值的
/// 生命周期被借用检查器钉在这次调用内（不能跨帧持有）。
pub struct HookCtx {
    /// 设备（见 [`HookCtx::device`]）。
    pub device: *const ID3D12Device,
    /// 记录中的命令列表（见 [`HookCtx::list`]）。
    pub list: *mut ID3D12GraphicsCommandList,
    /// 当前目标的 RTV 描述符句柄。
    pub rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    /// 目标像素尺寸（宽，高）。
    pub size: (f32, f32),
    pub frame_index: u64,
    /// 外部纹理池（仅在渲染回放期间有效，见 [`HookCtx::ext_pool`]）。
    pub ext_pool: Option<*const crate::ext::ExtPool>,
}

impl HookCtx {
    /// 设备引用（生命周期绑定在本次借用）。
    pub fn device(&self) -> &ID3D12Device {
        // SAFETY: 指针由 renderer 在构造时从有效引用取得，且本方法只在钩子的
        // `&self` 借用期内调用。
        unsafe { &*self.device }
    }

    /// 可录制命令的列表（生命周期绑定在本次 `&mut` 借用）。
    pub fn list(&mut self) -> &mut ID3D12GraphicsCommandList {
        // SAFETY: 同上；`&mut self` 保证不会与其它借用别名。
        unsafe { &mut *self.list }
    }

    /// 按 `resource_key` 查找外部纹理 SRV（ext 回放）。
    pub fn ext_pool(&self) -> Option<&crate::ext::ExtPool> {
        // SAFETY: 同上。
        unsafe { self.ext_pool.map(|p| &*p) }
    }
}

impl HookContext for HookCtx {
    fn frame_index(&self) -> u64 {
        self.frame_index
    }
    fn size(&self) -> (f32, f32) {
        self.size
    }
    fn damage(&self) -> Option<ExtRect> {
        None
    }
    fn as_any(&self) -> &dyn std::any::Any {
        // 让 `HookContextExt::downcast_mut::<HookCtx>()` 成功 —— 钩子据此拿到
        // `list` / `device` / `rtv` 录制自己的绘制命令（P0.1，笔迹层硬阻塞）。
        // 只以 `&mut` 借用存在于渲染线程，不跨线程，无需 Send。
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn core_rect_to_ext(r: zephyra_core::core::Rect) -> ExtRect {
    ExtRect::new(r.x, r.y, r.width, r.height)
}

/// 由脏区算出帧内 scissor。
///
/// # 清屏与 scissor 是**两件事**（曾经在这里写错过）
///
/// 原文断言「D3D12 的 `ClearRenderTargetView` 受 scissor 裁掉，D3D11 不受」——
/// **这是错的，而且错得很贵**。两个后端的 clear 区域都由 `ClearRenderTargetView`
/// 的 `pRects` 参数决定：`pRects == NULL` 清**整张**目标，与 scissor 无关。
///
/// 错在这里的直接后果：每帧整屏清成 surface 色，再只画脏矩形 ⇒ **任何交互帧
/// （hover / 按下 / 勾选）都会把界面上没脏的部分抹成一片底色**。症状是
/// 「点一下之后界面花掉」，而且首帧完全正常（首帧 `Damage::Full`），
/// 于是「渲染有问题」这件事只会在交互之后出现，与像素、内容、布局都无关。
///
/// 现在清屏走 [`damage_clear_rects`]：脏矩形就只清那一块。
///
/// 退化矩形回退到**全屏**而不是照设：D3D12 的 scissor 是半开区间 `[left, right)`，
/// `right <= left` 时什么都不画，而「脏区为空」更可能是调用方算错。
fn damage_scissor(size: (u32, u32), damage: zephyra_core::frame::Damage) -> RECT {
    match damage {
        zephyra_core::frame::Damage::Full => RECT {
            left: 0,
            top: 0,
            right: size.0 as i32,
            bottom: size.1 as i32,
        },
        zephyra_core::frame::Damage::Rect(r) => {
            // DIP → 像素的比例**不能假设为 1**：当前宿主恰好把像素尺寸当 DIP 传进来，
            // 但高 DPI 下同一个 `DisplayList` 会被放大。
            let sx = size.0 as f32 / (size.0 as f32).max(1e-6);
            let sy = size.1 as f32 / (size.1 as f32).max(1e-6);
            let l = (r.x * sx).floor().max(0.0) as i32;
            let t = (r.y * sy).floor().max(0.0) as i32;
            let rt = ((r.x + r.width) * sx).ceil() as i32;
            let b = ((r.y + r.height) * sy).ceil() as i32;
            if rt > l && b > t {
                RECT {
                    left: l,
                    top: t,
                    right: rt,
                    bottom: b,
                }
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: size.0 as i32,
                    bottom: size.1 as i32,
                }
            }
        }
    }
}

/// 清屏区域：`Full` → `NULL`（整张），`Rect` → 那一块。
///
/// 返回的 `RECT` 只在这一帧有效（栈上），所以借用它的 `Option` 也一样短命 ——
/// 调用点必须在自己的作用域里用完，不能存进 `CmdEnv`。
fn damage_clear_rects(
    size: (u32, u32),
    damage: zephyra_core::frame::Damage,
) -> (Option<RECT>, Option<[RECT; 1]>) {
    match damage {
        zephyra_core::frame::Damage::Full => (None, None),
        other => {
            let r = damage_scissor(size, other);
            (Some(r), Some([r]))
        }
    }
}

/// 是否**整条离屏路径都不跑**（`ZEPHYRA_NO_FILTERS=1`）。
///
/// 给 GPU 挂掉时做二分用，见 [`Renderer::record`] 里 `no_filters` 那段说明。
pub fn filters_disabled() -> bool {
    matches!(
        std::env::var("ZEPHYRA_NO_FILTERS")
            .as_deref()
            .map(str::trim),
        Ok("1") | Ok("true") | Ok("on")
    )
}

impl Renderer {
    /// 建根签名、PSO、描述符堆、顶点/索引缓冲与图集纹理。
    ///
    /// 着色器编译失败**必须**冒泡：它唯一的症状是「窗口一片空白 / 全透明」，而那与
    /// 「没有绘制任何东西」看起来一模一样。
    /// `samples` 是 MSAA 采样数（1 = 关）。**必须**与绑定的渲染目标一致 ——
    /// 传 [`crate::swapchain::Swapchain::samples`] 的返回值，别自己写常量。
    pub fn new(device: &ID3D12Device, atlas_size: u32, samples: u32) -> Result<Self, ShaderError> {
        let shaders = Shaders::new()?;

        // ---- 描述符堆 ----
        //
        // 1 个给图集（`t0` 恒绑它），剩下留给图像纹理。图集固定占 0 号，这样即使一个
        // 图像都没注册，`t0` 也绑着有效对象 —— 着色器在矩形分支不采样它，但「绑着
        // 一个无效 SRV」在某些驱动上会让后续采样返回未定义值。
        // 用模块级的 `ATLAS_SLOTS`（含 `g_dst` 副本与两张离屏 RT 的保留槽）。
        //
        // 这里曾经写着字面量 9 而模块级是 12 —— 于是堆只有 9 个槽，`refresh_dst_copy`
        // 取第 9 号时得到「描述符索引越界」，被 `.ok()` 吞掉，症状是「可分离混合
        // 模式全部退回 SrcOver」且**零错误**。
        // 第 0 号图集，1..=7 号图像纹理，最后一号留给 `g_dst` 的副本
        // （可分离 / 非可分离混合模式要读它，见 `refresh_dst_copy`）。
        let srv_heap = DescriptorHeap::new_srv(device, ATLAS_SLOTS, true)
            .map_err(|e| ShaderError::Create("建 SRV 描述符堆", e))?;

        // ---- 根签名 ----
        //
        // 一个描述符表（`t0`，一个 SRV）+ 一个静态采样器（`s0`）。静态采样器直接写在
        // 根签名里，就不需要单独的采样器堆与 `CreateSampler` —— 对我们这种「采样器永不
        // 改变」的场景是纯粹的简化。
        let root_signature =
            Self::create_root_signature(device).map_err(|e| ShaderError::Create("建根签名", e))?;

        // ---- PSO ----
        // PSO 全部懒建 —— 见 `pso_for`。这里只把**默认**那个先建出来，`Renderer::new`
        // 一旦失败就能立刻报错，而不是等到第一帧才炸。
        let pso = Self::create_pso(
            device,
            &root_signature,
            shaders.elements.descs(),
            0,
            zephyra_core::filter::BlendMode::SrcOver as usize,
            samples,
        )
        .map_err(|e| ShaderError::Create("建 PSO", e))?;
        let psos = std::collections::HashMap::from([(
            (
                0usize,
                zephyra_core::filter::BlendMode::SrcOver as usize,
                samples,
            ),
            pso.clone(),
        )]);

        // ---- 顶点 / 索引缓冲（每窗口一份，这里建默认的那一套）----
        let default_targets =
            WindowTargets::new(device).map_err(|e| ShaderError::Create("建默认渲染目标", e))?;

        // ---- 图集纹理 ----
        //
        // R8 单通道（字形位图只需覆盖率），与 D3D11 版一致。
        let atlas =
            create_atlas(device, atlas_size).map_err(|e| ShaderError::Create("建图集纹理", e))?;
        unsafe {
            device.CreateShaderResourceView(
                &atlas,
                None,
                srv_heap
                    .cpu(0)
                    .map_err(|e| ShaderError::Create("取图集 SRV 槽", e))?,
            );
        }

        Ok(Self {
            root_signature,
            psos: std::cell::RefCell::new(psos),
            srv_heap,
            atlas_srv_slot: 0,
            atlas_size,
            atlas,
            atlas_staging: None,
            device: device.clone(),
            textures: Vec::new(),
            next_texture_slot: 1,
            free_texture_slots: Vec::new(),
            ext_srv_keys: std::collections::HashMap::new(),
            elements: shaders.elements,
            samples,
            default_targets: Some(default_targets),
        })
    }

    /// 登记一个外部渲染源钩子（`kind` 相同者覆盖）。
    ///
    /// 见 [`RenderHook`] 与 [`zephyra_core::paint::Command::Custom`]。
    ///
    /// 写的是**默认目标**的钩子表（单窗口 / 测试路径）。多窗口宿主用
    /// [`Self::register_hook_on`] 往自己窗口的 [`WindowTargets`] 里登记。
    pub fn register_hook(
        &mut self,
        kind: impl Into<std::borrow::Cow<'static, str>>,
        hook: Box<dyn RenderHook>,
    ) {
        let targets = self
            .default_targets
            .as_mut()
            .expect("默认目标只在 record 借用期间为 None");
        Self::register_hook_on(targets, kind, hook);
    }

    /// 往指定窗口的目标里登记外部渲染源钩子。
    pub fn register_hook_on(
        targets: &mut WindowTargets,
        kind: impl Into<std::borrow::Cow<'static, str>>,
        hook: Box<dyn RenderHook>,
    ) {
        let kind = kind.into();
        if let Some(slot) = targets.hooks.iter_mut().find(|(k, _)| *k == kind) {
            slot.1 = hook;
        } else {
            targets.hooks.push((kind, hook));
        }
    }

    /// 根签名：两个描述符表（`t0` 主纹理 + `t1` 目标副本）+ 一个静态采样器。
    ///
    /// # 为什么要有 `t1`
    ///
    /// 可分离 / 非可分离混合模式必须读 `dst`，而 D3D12 **不能采样当前绑定的 RTV**
    /// （D3D11 同理）—— 所以宿主先把渲染目标拷进一张单采样纹理，再当 SRV 绑到
    /// `t1`。HLSL 里 `Texture2D<float4> g_dst : register(t1)` 说的就是它。
    ///
    /// # 为什么是**两个**表而不是一个表放两个描述符
    ///
    /// 一个表里的描述符在堆里必须**连续**，那意味着每帧都要把"当前纹理"的描述符
    /// `CopyDescriptorsSimple` 到目标副本旁边 —— 纯粹为了相邻而做一次拷贝。
    /// 两个表各自指向堆里任意的槽位，一次 `SetGraphicsRootDescriptorTable` 就够。
    fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature, SurfaceError> {
        let ranges = [
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 1,
                BaseShaderRegister: 0, // 对 `Texture2D g_tex : register(t0)`
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            },
            D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 1,
                BaseShaderRegister: 1, // 对 `Texture2D g_dst : register(t1)`
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            },
        ];
        // 两个参数各取 `ranges` 的一个元素。`params` 里只存指针，所以 `ranges`
        // 必须活到 `D3D12SerializeRootSignature` 之后 —— 它在本函数作用域里，满足。
        let params = [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[0],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &ranges[1],
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
        ];
        // 静态采样器：与 `SamplerState g_smp : register(s0)` 对应。
        //
        // 过滤取线性、寻址取夹取（clamp）。夹取而非环绕：字形图集的 UV 在 [0,1] 内，
        // 但边缘像素的采样器权重会溢出到邻格 —— 环绕会从图集另一侧取到字，开取会把
        // 图集外的黑色渲进来。两者都会在图集边界上看到「鬼影」。
        let sampler = D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
            BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
            MinLOD: 0.0,
            MaxLOD: f32::MAX,  // D3D12_LOD_CLAMP_NONE
            ShaderRegister: 0, // s0
            RegisterSpace: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        };
        let desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: 2,
            pParameters: params.as_ptr(),
            NumStaticSamplers: 1,
            pStaticSamplers: &sampler,
            // `ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT`：根签名只声明了片元可见的
            // `t0`，而 PSO 里**有**顶点输入布局。没有这个标志时，D3D12 允许「有输入
            // 布局但根签名没为它留位」，但版本差异多；加上它最省心，且与 D3D11 的行为
            // 一致（D3D11 没有这一层校验）。
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };

        let mut blob = None::<windows::Win32::Graphics::Direct3D::ID3DBlob>;
        let mut err_blob = None::<windows::Win32::Graphics::Direct3D::ID3DBlob>;
        unsafe {
            D3D12SerializeRootSignature(
                &desc,
                D3D_ROOT_SIGNATURE_VERSION_1_0,
                &mut blob,
                Some(&mut err_blob),
            )
            .map_err(|e| {
                // 把驱动的错误 blob 转成可读消息：根签名序列化失败时，HRESULT
                // 常是 `E_INVALIDARG`，真正的原因在 blob 里。
                SurfaceError::new(
                    "D3D12SerializeRootSignature",
                    windows::core::Error::new(
                        e.code(),
                        blob_message(&err_blob, "根签名描述本身就不合法"),
                    ),
                )
            })?;
        }
        let blob = blob.ok_or_else(|| {
            SurfaceError::new(
                "根签名序列化成功却没给 blob",
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "no blob"),
            )
        })?;
        // `CreateRootSignature` 收的是**字节**，而 `D3D12SerializeRootSignature` 给的是
        // 一个 `ID3DBlob`。所以要把 blob 的内容拷出来 —— 注意必须拷，因为 blob 在函数
        // 返回时就被释放了。
        let ptr = unsafe { blob.GetBufferPointer() };
        let len = unsafe { blob.GetBufferSize() };
        if ptr.is_null() || len == 0 {
            return Err(SurfaceError::new(
                "根签名 blob 是空的",
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "empty blob"),
            ));
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }.to_vec();
        unsafe {
            device
                .CreateRootSignature(0, &bytes)
                .map_err(|e| SurfaceError::new("CreateRootSignature", e))
        }
    }

    /// 建图形 PSO：**指定**的像素着色器变体 + **指定**的混合模式。
    ///
    /// # 为什么 PSO 要按 (PS 变体, 混合模式, 采样数) 三元组建
    ///
    /// D3D12 的混合状态**烤在 PSO 里**，不像 D3D11 那样能逐段换
    /// `ID3D11BlendState`。于是"换混合模式"在这里必然等于"换 PSO"。再叠加
    /// 每个模式用哪个 PS 变体（系数可表达的那 15 个都用 `PSMain`，其余 14 个各用
    /// 自己的合成变体）与 MSAA 采样数（`MultisampleEnable` 与 `SampleDesc` 都进
    /// PSO），就是一个三元组。
    ///
    /// 数量上限 15 + 14 = 29（× 采样数），全部懒建 —— 绝大多数帧只用默认那一个。
    fn create_pso(
        device: &ID3D12Device,
        root_signature: &ID3D12RootSignature,
        elements: &[D3D12_INPUT_ELEMENT_DESC],
        ps_index: usize,
        mode: usize,
        samples: u32,
    ) -> Result<ID3D12PipelineState, SurfaceError> {
        // 混合状态与 D3D11 版**逐项一致**（两边各写一份，所以这里必须同步改）。
        //
        // **预乘 alpha**：顶点色在 `quad::Vertex::quad` 里已乘过自己的 alpha，所以
        // `SrcOver` 的颜色因子是 `ONE` —— 再乘一次 `SRC_ALPHA` 就是双重预乘，
        // 表现为半透明区域整体发暗。
        let blend = blend_desc_for(mode).unwrap_or_else(src_blend_desc);
        let (name, ps_bytes) = shader::PS_VARIANTS.get(ps_index).unwrap_or_else(|| {
            panic!(
                "ps_index {ps_index} 越界（表长 {}）",
                shader::PS_VARIANTS.len()
            )
        });

        // 光栅化：关背面剔除 —— 圆角 SDF 在边缘产生的三角形可能退化，剔除会让圆角
        // 出现缺口（表现为四边形的角被啃掉一块）。
        let raster = D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: windows::core::BOOL(0),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: windows::core::BOOL(1),
            // ⚠ MSAA。**必须**与实际绑定的渲染目标采样数一致 —— D3D12 在
            // `OMSetRenderTargets` / `DrawIndexedInstanced` 处逐项比对，不一致时
            // 报「render target format does not match」而画面全错。
            MultisampleEnable: windows::core::BOOL((samples > 1) as i32),
            AntialiasedLineEnable: windows::core::BOOL(0),
            ForcedSampleCount: 0,
            ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        };

        let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            // `ManuallyDrop`：PSO 描述里的 COM ptr_of**不**拥有所指对象 —— 根签名的所有权在
            // `Renderer::root_signature` 上。所以这里必须手动包一层，不能用 `From`。
            pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature.clone())),
            VS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: shader::VS_BYTECODE.as_ptr() as *const _,
                BytecodeLength: shader::VS_BYTECODE.len(),
            },
            PS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: ps_bytes.as_ptr() as *const _,
                BytecodeLength: ps_bytes.len(),
            },
            DS: D3D12_SHADER_BYTECODE::default(),
            HS: D3D12_SHADER_BYTECODE::default(),
            GS: D3D12_SHADER_BYTECODE::default(),
            StreamOutput: D3D12_STREAM_OUTPUT_DESC::default(),
            BlendState: blend,
            SampleMask: u32::MAX, // 全采样掩码（D3D12 必填，传 0 等于「什么都不画」）
            RasterizerState: raster,
            // 不用深度缓冲：UI 全部靠 painter's algorithm 排序。
            DepthStencilState: D3D12_DEPTH_STENCIL_DESC::default(),
            InputLayout: D3D12_INPUT_LAYOUT_DESC {
                pInputElementDescs: elements.as_ptr(),
                NumElements: elements.len() as u32,
            },
            IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            // ⚠ 只有第 0 槽是真实格式，第 1..8 槽**必须**是 `UNKNOWN`。
            //
            // 调试层的原话：
            //     "ID3D12Device::CreateGraphicsPipelineState: RTVFormats[] must be
            //      DXGI_FORMAT_UNKNOWN for all indices >= NumRenderTargets."
            RTVFormats: {
                let mut f = [windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN; 8];
                f[0] = windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
                f
            },
            DSVFormat: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
            // ⚠ 必须与 `MultisampleEnable` 和实际 RTV 的 `SampleDesc` **三者一致**。
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: samples,
                Quality: 0,
            },
            // 单 GPU 就是 0（驱动会转成 1）。理由见 [`PSO_NODE_MASK`]。
            NodeMask: PSO_NODE_MASK,
            CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
            Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
        };
        unsafe {
            device.CreateGraphicsPipelineState(&desc).map_err(|e| {
                SurfaceError::new(
                    "CreateGraphicsPipelineState",
                    windows::core::Error::new(
                        e.code(),
                        format!("ps={name} mode={mode} samples={samples}: {e}"),
                    ),
                )
            })
        }
    }

    /// 目标像素的**单采样**副本 —— 可分离 / 非可分离混合模式读它当 `dst`。
    ///
    /// # 为什么必须拷贝
    ///
    /// D3D12（与 D3D11 一样）**不能采样当前绑定的 RTV**。直接绑一个指向渲染
    /// 目标的 SRV 得到的是未定义值，症状是「效果完全没发生」而零错误。
    ///
    /// # 为什么返回堆槽位而不是视图
    ///
    /// D3D12 的 SRV 不是对象而是堆里的一个**槽位** —— 与 D3D11 返回
    /// `ID3D11ShaderResourceView` 不同。槽位在 `record` 里经
    /// `SetGraphicsRootDescriptorTable(1, …)` 绑给 `t1`。
    ///
    /// `None` = 目标资源没给（`RenderCtx::rt` 为 `None`）或建不出来 ⇒ 调用方
    /// 退回 `SrcOver`。宁可少个效果，也不要画出一个颜色错误的结果。
    // 参数是拷一次所需的全部输入，彼此没有可成组打包的语义 —— 凑一个 struct
    // 只是把 8 个参数搬进另一个名字，读的人反而要多跳一层（同 `replay`）。
    // `region` 是「只要这一块」，`allow_reuse` 是「内容没变就别再拷」，
    // 两者正交，合并只会让调用点更难读。
    #[allow(clippy::too_many_arguments)]
    fn refresh_dst_copy(
        &self,
        targets: &WindowTargets,
        device: &ID3D12Device,
        list: &ID3D12GraphicsCommandList,
        rt: Option<&ID3D12Resource>,
        size: (u32, u32),
        // 复用已有副本的许可。
        //
        // `true` = 允许复用（玻璃 / 折射 / 磨砂那条路：只要目标内容没变，
        // 副本就是现成的）；`false` = 强制重拷（可分离混合那条路：它要的
        // 是**此刻**的目标，而此刻可能已经有玻璃合成进去了）。
        allow_reuse: bool,
        // 只需要这块区域（像素，`(x, y, w, h)`）；`None` = 整幅。
        //
        // MSAA 开着时整幅解析是 ~1.2 ms（1400×800 × 4 样本），而一个磨砂样板
        // 只有 160×120 —— 按区解析能把它压到几十微秒。见 [`DstCopy::resolved`]。
        region: Option<(u32, u32, u32, u32)>,
    ) -> Option<u32> {
        use windows::Win32::Graphics::Direct3D12::{
            D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_RESOLVE_SOURCE,
        };

        // ⚠ 这里**必须**是模块级的 [`DST_SRV_SLOT`]，不能自己再算一次。
        //
        // `ATLAS_SLOTS - 1` 是 `OFFSCREEN_SRV_B` —— 也就是离屏乒乓 RT_B 的 SRV 槽位。
        // 写错时，同一帧里「既有可分离混合又有离屏 pass」就会让 `refresh_dst_copy` 覆写掉
        // B 的描述符，而 `t1` 绑到 B 上的模糊 / 合成趟于是读到目标副本，症状是
        // 「有混合模式时模糊结果整个错掉」—— 两条路径各自单测都是绿的。
        const DST_SLOT: u32 = DST_SRV_SLOT;
        let rt = rt?;

        // 采样数与尺寸从**源资源本体**读，不是从调用方约定：同一个函数要同时伺候三种源 ——
        // 多重采样纹理（MSAA 开）、后缓冲（MSAA 关）、离屏 RT（可分离混合读自己）。
        // 采样数不同 ⇒ 拷贝类调用一律拒绝，必须走 resolve。
        let src_desc = unsafe { rt.GetDesc() };
        let src_samples = src_desc.SampleDesc.Count.max(1);

        let mut slot = targets.dst_copy.borrow_mut();

        // 把请求区域夹进内容区；空区域 ⇒ 当作整幅（调用方本来也不该给空的）。
        let region = region.and_then(|(x, y, w, h)| {
            let x = x.min(size.0);
            let y = y.min(size.1);
            let w = w.min(size.0.saturating_sub(x));
            let h = h.min(size.1.saturating_sub(y));
            (w > 0 && h > 0).then_some((x, y, w, h))
        });

        // 副本里**已有**的区域是否覆盖本次请求。
        let covers = |have: Option<(u32, u32, u32, u32)>| -> bool {
            let want = match region {
                None => return true, // 要整幅 ⇒ 只有整幅副本算覆盖
                Some(w) => w,
            };
            match have {
                None => true, // 整幅副本覆盖一切
                Some(h) => {
                    h.0 <= want.0
                        && h.1 <= want.1
                        && h.0 + h.2 >= want.0 + want.2
                        && h.1 + h.3 >= want.1 + want.3
                }
            }
        };

        // ★ 内容没变 ⇒ 副本仍然有效，**不重拷**。
        //
        // 这是本函数最值钱的一步：MSAA 开着时重拷是一次**整幅解析**
        // （D3D12 没有「按区解析」；本函数改用 `ResolveSubresourceRegion`
        // 按区做，但区域之外的复用判据仍然要成立），而一页里有 7 个带滤镜
        // 的元素时就是 7 次全屏解析（实测 5.2 ms/帧）。
        //
        // ⚠ 判据是**内容版本**而不是「帧号」—— 后者会让同一帧里第二块
        // 玻璃的背景里**少了**夹在两块玻璃之间画的内容（实测材质页
        // 10.3% 的像素变了，最大通道差 157，肉眼可见）。内容版本只在
        // **真有内容画进目标**时推进，所以「背景没变」的判断是精确的。
        if allow_reuse
            && targets.dst_copy_revision.get() == targets.content_revision.get()
            && slot.as_ref().is_some_and(|d| d.size == size)
            && size.0 != 0
            && size.1 != 0
            && slot.as_ref().is_some_and(|d| covers(d.resolved))
        {
            return Some(DST_SLOT);
        }
        if slot
            .as_ref()
            .is_none_or(|d| d.size != size || size.0 == 0 || size.1 == 0)
        {
            // 格式与源相同（本仓所有渲染目标都是 R8G8B8A8_UNORM），否则解析与采样
            // 都会错位。标志与初始状态的理由见 [`dst_copy_desc`] / [`RESOLVE_DST_STATE`]。
            let dst = create_resource(
                device,
                &heap_props(D3D12_HEAP_TYPE_DEFAULT),
                &dst_copy_desc(size),
                RESOLVE_DST_STATE,
            )
            .ok()?;
            unsafe {
                device.CreateShaderResourceView(&dst, None, self.srv_heap.cpu(DST_SLOT).ok()?);
            }
            *slot = Some(DstCopy {
                resource: dst,
                size,
                state: RESOLVE_DST_STATE,
                resolved: None,
            });
        }
        // 取出来当本地变量：`state` 收尾要写回去，而借用不能同时读写。
        let (dst_res, before) = {
            let d = slot.as_ref()?;
            (d.resource.clone(), d.state)
        };

        // 只取**内容区**：源纹理按「交换链容量」建（`Swapchain::resize` 里按 25% 余量
        // 增长，≥ 内容），而 `g_dst` 的采样 uv 走 `DstUv`（NDC → 视口），所以副本必须是
        // 内容尺寸、且内容落在左上角。源不可能小于内容，这里 `min` 只是给 API 兜底。
        let cw = size.0.min(src_desc.Width as u32);
        let ch = size.1.min(src_desc.Height);

        if src_samples > 1 {
            // 多重采样源 ⇒ `ResolveSubresource`（逐样本求平均，与帧末
            // `Swapchain::resolve_msaa` 同一份调用）。
            //
            // 为什么不能 `CopyResource` / `CopyTextureRegion`：两者都要求源与目标
            // **采样数相同**，4× → 1× 是非法的。D3D11 后端的同名函数栽过这点
            // （`zephyra-winit/src/renderer.rs` 的 `refresh_dst_copy`），它的注释写着
            // 「某些驱动只警告就继续，于是画面停在上一帧的内容上」—— 与本仓 MSAA 开启时
            // 折射玻璃里浮现上一页控件残影是同一类症状，只是触发条件不同。
            //
            // ⚠ `ResolveSubresource`（整幅那条岔路）解析的是**整个子资源**，所以
            // 源与目标必须逐位同尺寸。交换链带 25% 余量时（`Swapchain::resize`）
            // 源会大于内容 —— 那一档走**按区解析**（见下），它不要求尺寸相同；
            // 只有在按区那条路走不通时才会退回这里，此时**放弃本次折射**
            // （返回 `None` ⇒ 调用方退回 `SrcOver`）：宁可少个效果，也不要录进
            // 一条非法命令。非法命令的后果不是画错，是 `Close()` 报
            // `E_INVALIDARG`、整帧提交失败、此后每帧 `Reset` 都失败 ——
            // 画面**永久冻结**在最后一帧（实测踩过，表现为「点了导航页面不换」）。
            //
            // 源的状态不转：MSAA 纹理从建起来就一直是 `RENDER_TARGET`
            // （见 `swapchain.rs` `resolve_msaa` 的文档），且它整帧都是那个渲染目标。
            //
            // ★ 整幅那条路必须要求**逐位相等**，不能拿 `min` 兜底。副本就是按
            // `size` 建的，所以「源比内容小」同样是不匹配：拿 `min(size, src)` 当判据时，
            // 源更小那一档会**通过检查**然后把一张小图解析进大副本 —— 尺寸不匹配的
            // 解析是规格级非法调用，录进去就冻死。`min` 只对按区那条路有意义。
            let size_matches = (src_desc.Width as u32, src_desc.Height) == (size.0, size.1);
            // 目标状态必须走 [`RESOLVE_DST_STATE`]（`RESOLVE_DEST`），不能用
            // `RENDER_TARGET` —— 理由与后果都写在那个常量的文档里。
            //
            // ★ 源同样必须转状态：`RESOLVE_SOURCE`。原注释写的是「源不转，它从
            // 建起来就是 RENDER_TARGET」—— 驱动容忍，规格不容忍：调试层 538
            // 「Expected State Bits (all): RESOLVE_SOURCE」，而严格的驱动在
            // `Close()` 就拒（0x80070057 + 永久冻结）。两条各错过一遍，症状
            // 一模一样，所以两边都转。
            //
            // 屏障成对：解析完立刻转回 `RENDER_TARGET` —— 源还是本帧的渲染
            // 目标（折射合成趟紧接着要绑它画；PS 变体那条路它正绑着）。
            gpu_barrier(
                list,
                rt,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_RESOLVE_SOURCE,
            );
            gpu_barrier(list, &dst_res, before, RESOLVE_DST_STATE);
            // ★ 按区解析（`ResolveSubresourceRegion`）而不是整幅。
            //
            // `ResolveSubresource` 只解析**整个子资源**，1400×800×4 样本一次
            // ~1.2 ms —— 而触发它的往往是一个 160×120 的磨砂样板。按区版本
            // 只付那块面积的代价。
            //
            // ⚠ 约束（写错就是 `Close()` 报 `E_INVALIDARG`、整帧提交失败、
            // 此后每帧 `Reset` 也失败 —— 画面**永久冻结**）：
            // - 源与目标**格式相同**（两边都是 R8G8B8A8_UNORM）；
            // - `pSrcRect` 必须落在源资源内，且目标偏移 + 区域不越界；
            // - 区域的左上角对非 YUV / 非 BC 格式没有对齐要求（本仓全是
            //   RGBA8），但**越界**一律非法。
            // 上面已经把 `region` 夹进内容区，这里再兜一次。
            let clipped = region.and_then(|(x, y, w, h)| {
                let x = x.min(cw);
                let y = y.min(ch);
                let w = w.min(cw.saturating_sub(x));
                let h = h.min(ch.saturating_sub(y));
                (w > 0 && h > 0).then_some((x, y, w, h))
            });
            // 只在小区域上走按区解析：区域接近整幅时省不下什么，反而多一条
            // 少有人走的 API 路径。阈值取 40%（磨砂样板约 1.6%）。
            let clipped = clipped
                .filter(|(_, _, w, h)| (*w as u64 * *h as u64) * 10 < (cw as u64 * ch as u64) * 4);
            // `ID3D12GraphicsCommandList1` 才有 `ResolveSubresourceRegion`；
            // 拿不到（极老的运行环境）就退回整幅解析。
            let list1 = clipped.and_then(|_| {
                list.cast::<windows::Win32::Graphics::Direct3D12::ID3D12GraphicsCommandList1>()
                    .ok()
            });
            match (clipped, list1) {
                (Some((x, y, w, h)), Some(list1)) => {
                    // ⚠ MSAA 解析的区域按**偶数**对齐：某些格式/驱动要求解析
                    // 矩形落在 2×2 的样本栅格上，越界或错位一律是
                    // `E_INVALIDARG`，而那个错误要到 `Close()` 才现形 ——
                    // 后果是整帧提交失败、此后每帧 `Reset` 也失败（画面永久冻结）。
                    // 向外扩到偶数像素只多拷一圈，不会错位。
                    let x0 = x & !1;
                    let y0 = y & !1;
                    let x1 = ((x + w + 1) & !1).min(cw & !1);
                    let y1 = ((y + h + 1) & !1).min(ch & !1);
                    if x1 <= x0 || y1 <= y0 {
                        // 扩完没有有效区域 ⇒ 整幅（尺寸不匹配时只能放弃）。
                        if !size_matches {
                            slot.as_mut()?.state = RESOLVE_DST_STATE;
                            return None;
                        }
                        unsafe {
                            list.ResolveSubresource(&dst_res, 0, rt, 0, src_desc.Format);
                        }
                        slot.as_mut()?.resolved = None;
                    } else {
                        let src_rect = windows::Win32::Foundation::RECT {
                            left: x0 as i32,
                            top: y0 as i32,
                            right: x1 as i32,
                            bottom: y1 as i32,
                        };
                        unsafe {
                            list1.ResolveSubresourceRegion(
                                &dst_res,
                                0,
                                x0,
                                y0,
                                rt,
                                0,
                                Some(&src_rect),
                                src_desc.Format,
                                windows::Win32::Graphics::Direct3D12::D3D12_RESOLVE_MODE_AVERAGE,
                            );
                        }
                        slot.as_mut()?.resolved = Some((x0, y0, x1 - x0, y1 - y0));
                    }
                }
                _ => {
                    // 整幅解析要求源与目标逐位同尺寸（见上面的 `size_matches`）；
                    // 不然只能放弃这次效果。
                    if !size_matches {
                        slot.as_mut()?.state = RESOLVE_DST_STATE;
                        return None;
                    }
                    unsafe {
                        list.ResolveSubresource(&dst_res, 0, rt, 0, src_desc.Format);
                    }
                    slot.as_mut()?.resolved = None;
                }
            }
            gpu_barrier(
                list,
                rt,
                D3D12_RESOURCE_STATE_RESOLVE_SOURCE,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
            slot.as_mut()?.state = RESOLVE_DST_STATE;
        } else {
            // 单采样源 ⇒ `CopyTextureRegion` + 源框。不用 `CopyResource`：容量可能大于
            // 内容，而它要求源与目标尺寸逐位相同（resize 之后就不再成立）。
            //
            // 源前后各一道屏障：拷完必须回 `RENDER_TARGET`，否则后续对目标的绘制非法。
            gpu_barrier(
                list,
                rt,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            gpu_barrier(list, &dst_res, before, D3D12_RESOURCE_STATE_COPY_DEST);
            unsafe {
                list.CopyTextureRegion(
                    &D3D12_TEXTURE_COPY_LOCATION {
                        pResource: std::mem::ManuallyDrop::new(Some(dst_res.clone())),
                        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                            SubresourceIndex: 0,
                        },
                    },
                    0,
                    0,
                    0,
                    &D3D12_TEXTURE_COPY_LOCATION {
                        pResource: std::mem::ManuallyDrop::new(Some(rt.clone())),
                        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                            SubresourceIndex: 0,
                        },
                    },
                    Some(&D3D12_BOX {
                        left: 0,
                        top: 0,
                        right: cw,
                        bottom: ch,
                        front: 0,
                        back: 1,
                    }),
                );
            }
            gpu_barrier(
                list,
                rt,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
            slot.as_mut()?.state = D3D12_RESOURCE_STATE_COPY_DEST;
            slot.as_mut()?.resolved = None;
        }

        // 副本最后一步才允许被采样 —— 在此之前读它拿到的是上一次的内容。
        let after = slot.as_ref()?.state;
        gpu_barrier(
            list,
            &dst_res,
            after,
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        );
        slot.as_mut()?.state = D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
        // ★ 记下这份副本对应的是哪个内容版本 —— 下一趟滤镜若发现版本没变，
        // 就能直接复用（见本函数开头的「复用」分支）。
        targets
            .dst_copy_revision
            .set(targets.content_revision.get());
        Some(DST_SLOT)
    }

    /// 取（必要时建）`(PS 变体, 混合模式, 采样数)` 对应的 PSO。
    ///
    /// 懒建 + 缓存：绝大多数帧只用默认那一个，把 29 × 采样数 个 PSO 全在
    /// `Renderer::new` 里建出来是纯粹的启动开销。
    fn pso_for(
        &self,
        device: &ID3D12Device,
        elements: &[D3D12_INPUT_ELEMENT_DESC],
        ps_index: usize,
        mode: usize,
        samples: u32,
    ) -> Result<ID3D12PipelineState, SurfaceError> {
        let key = (ps_index, mode, samples);
        if let Some(p) = self.psos.borrow().get(&key) {
            return Ok(p.clone());
        }
        let pso = Self::create_pso(
            device,
            &self.root_signature,
            elements,
            ps_index,
            mode,
            samples,
        )?;
        self.psos.borrow_mut().insert(key, pso.clone());
        Ok(pso)
    }

    /// 图集边长（像素）。
    pub fn atlas_size(&self) -> u32 {
        self.atlas_size
    }

    /// 顶点缓冲容量（字节）。稳态下跨帧不变。
    ///
    /// 读的是**默认目标**的容量（单窗口 / 测试路径）。
    pub fn vertex_capacity(&self) -> u64 {
        self.default_targets
            .as_ref()
            .map(|t| t.vertex_capacity)
            .unwrap_or(0)
    }

    /// 把字形图集位图上传到 GPU 纹理（R8，`atlas_size × atlas_size`）。
    ///
    /// # 为什么纹理上传要走「上传缓冲 + placed footprint」而不是直接映射纹理
    ///
    /// 只有 `UPLOAD` 堆的资源能被 CPU 写，而纹理**不能**建在上传堆里 —— 上传堆只支持
    /// 缓冲。所以纹理上传的固定形态是：
    ///
    /// ```text
    /// CPU → (upload 缓冲, 按 footprint 摆放)  → CopyTextureRegion  → 图集纹理
    /// ```
    ///
    /// # 为什么必须用驱动给的行距
    ///
    /// `GetCopyableFootprints` 返回的 `RowPitch` 是驱动挑的（1024 宽的 R8 纹理常常是
    /// 1024 而恰好等于紧密行距，但换个分辨率就不等了）。按紧密行距连续写进去，内容会
    /// **斜着**排在缓冲里 —— 症状是字形整体倾斜、且倾斜量随行号累积，极难定位。
    ///
    /// # 调用时机
    ///
    /// **记录**命令（不提交）。调用方负责 `Gpu::submit` + `Gpu::wait_gpu`，这样
    /// 「记录 / 提交 / 等待」三步在测试里都看得见。
    ///
    /// # 为什么整张上传而不是只传新增区域
    ///
    /// 图集会被**整体重排**（LRU 淘汰之后按高度重排），那时几乎每个字形的 uv 都变了，
    /// 增量上传无从下手。代价是每次变化多一次 4MB 上传；稳态下图集不再变化，这个上传
    /// 也就不再发生 —— 宿主用 [`zephyra_core::text::TextStack::atlas_revision`] 判断，
    /// **不能**用字形数（淘汰可能让数量不变而内容全变）。
    pub fn record_upload_atlas(
        &mut self,
        list: &ID3D12GraphicsCommandList,
        pixels: &[u8],
    ) -> Result<(), SurfaceError> {
        let side = self.atlas_size;
        // ⚠ 行宽是 `side * 4`，不是 `side`。
        //
        // 图集是 RGBA8（覆盖率写在四个通道里），所以一行 `side` 个像素占 `side*4`
        // 字节。这里原先按 `side` 算 —— 于是 `CopyTextureRegion` 源缓冲区只有实际需要的
        // 1/4，症状是「上传越界」或「图集只填了左上角一条」。
        let bytes_per_px = 4usize;
        let tight = side as usize * bytes_per_px;
        let height = side as usize;

        // 图集纹理只当 SRV 采样，不需要任何 bind flag（D3D12 里 shader 可见的资源
        // 默认就是 `FLAG_NONE`；`ALLOW_UNORDERED_ACCESS` 才会互斥）。
        let tex_desc = crate::surface::texture_desc((side, side), D3D12_RESOURCE_FLAG_NONE);

        // 行距与总量都由驱动挑，且**必须**按它摆放 / 按它定暂存大小。
        let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut num_rows = 0u32;
        let mut row_size = 0u64;
        let mut total = 0u64;
        unsafe {
            self.device.GetCopyableFootprints(
                &tex_desc,
                0,
                1,
                0,
                Some(&mut footprint),
                Some(&mut num_rows),
                Some(&mut row_size),
                Some(&mut total),
            );
        }
        let row_pitch = footprint.Footprint.RowPitch as usize;
        if row_pitch < tight {
            return Err(SurfaceError::new(
                "图集上传：驱动给的行距装不下一行像素",
                windows::core::Error::new(E_FAIL, format!("行距 {row_pitch} < 一行 {tight} 字节")),
            ));
        }

        // ⚠ 暂存是**上传堆**缓冲，**直接**当拷贝源 —— 中间不经过 DEFAULT 堆的副本。
        //
        // 原先这里建的是 `StreamBuffer`（上传堆 + DEFAULT 堆两份），像素写进了上传堆的
        // 映射内存，`CopyTextureRegion` 却从 `resident`（DEFAULT 堆那份、**从未被写过**）
        // 拷。于是每次上传都「成功」，而图集永远是 `CreateCommittedResource` 时的内容
        // （零）—— 采样覆盖率恒为 0，文字整批消失且**没有任何报错**。
        //
        // 上传堆资源恒为 `GENERIC_READ`，而 `GENERIC_READ` 正是拷贝源要的状态，所以
        // 一条屏障都不需要。原先那条 `GENERIC_READ → COPY_SOURCE` 反而是**非法**的
        // （上传堆资源不允许状态转换）。
        let needed = total.max((tight * height) as u64);
        // ⚠⚠ **复用上一块暂存**（装得下就复用）。
        //
        // 原先这里**每次上传都新建**一块 4 MB 的上传堆缓冲 —— 而图集是 1024×1024
        // 的 RGBA（4 MB）。于是一次上传要付「4 MB 堆分配 + `CreateCommittedResource`
        // + `Map` + 释放」的钱，而实际要拷的像素只有**变化的那部分**。
        //
        // 实测（gallery，release，1400×800，切换页面时图集修订号变化）：
        // 切页那一帧的 `render_submit` 中位数 **4818 µs**，而 `wait_gpu` 只有 1 µs、
        // `present` 只有 88 µs —— 那 4.8 ms **全在这块缓冲的创建上**。
        //
        // 复用是安全的：图集上传发生在帧内（`begin_frame` 之后），而 `begin_frame`
        // 的第一件事就是等 GPU 排空 ⇒ 上一帧的那次拷贝**已经完成**，映射内存可以
        // 随便覆写。
        let staging = match self.atlas_staging.take() {
            Some(existing) if existing.capacity >= needed => existing,
            _ => UploadBuffer::new(&self.device, needed)?,
        };

        // 按行距逐行写进映射内存。
        //
        // `n` 是**字节**数，不是像素数 —— 所以还要除以 `bytes_per_px` 才得到
        // "源数据里有几行"。这里早先写成 `side * height`（像素数），于是每个像素
        // 只写了 1/4 个字节，图集变成一条条斜纹。
        let have_bytes = (tight * height).min(pixels.len());
        let rows_usable = have_bytes / tight;
        unsafe {
            let dst = staging.mapped;
            for row in 0..rows_usable.min(height) {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add(row * tight),
                    dst.add(row * row_pitch),
                    tight,
                );
            }

            gpu_barrier(
                list,
                &self.atlas,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            list.CopyTextureRegion(
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(self.atlas.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: 0,
                    },
                },
                0,
                0,
                0,
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(staging.resource.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: footprint,
                    },
                },
                None,
            );
            gpu_barrier(
                list,
                &self.atlas,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
        }
        // 暂存缓冲必须活得比这次拷贝久。把它挂到渲染器上，由下一帧开头替换。
        self.atlas_staging = Some(staging);
        Ok(())
    }

    /// 上传一张图像纹理，返回它在 `srv_heap` 里的槽位。
    ///
    /// **只记录，不提交** —— 与 [`Self::record`] / [`Self::record_upload_atlas`] 同一个
    /// 约定。调用方负责提交并等 GPU 读完（见 [`crate::surface::Gpu::submit_one_off`]）。
    ///
    /// # 三步，与图集上传同构
    ///
    /// 1. 纹理建在**默认堆**，初始状态 `COPY_DEST`；
    /// 2. 上传堆暂存按驱动给的 `RowPitch` 逐行填，屏障转 `COPY_SOURCE` 后
    ///    `CopyTextureRegion`；
    /// 3. 转回 `PIXEL_SHADER_RESOURCE`，并 `CreateShaderResourceView` 写进堆槽。
    ///
    /// # 为什么第 2 步不能用 `WriteToSubresource`
    ///
    /// 那个 API 也能一步写完，但它是 **D3D11 风格的同步拷贝**：内部隐式等待
    /// 「GPU 不再引用该资源」，也就是一个隐式的 fence。对启动期一次性上传这无所谓，
    /// 可它把「一次隐式全局等待」藏在一个看起来像普通 memcpy 的调用里 ——
    /// 与本项目「每一步都能被调试层查到」的取舍相反。
    ///
    /// # 尺寸不做对齐
    ///
    /// 图标是 32×32、任意小图也是。`GetCopyableFootprints` 的行距必须按它填，
    /// 但纹理本身不必补齐到 2 的幂 —— 非幂尺寸在 `CreateShaderResourceView` 里
    /// 只需注意 `MipLevels = 1`。
    pub fn upload_texture(
        &mut self,
        list: &ID3D12GraphicsCommandList,
        pixels: &[u8],
        w: u32,
        h: u32,
    ) -> Result<u32, SurfaceError> {
        if w == 0 || h == 0 {
            return Err(SurfaceError::new(
                "上传纹理",
                windows::core::Error::new(E_INVALIDARG, "宽高不能为 0"),
            ));
        }
        let tight = w as usize * 4;
        let need = tight * h as usize;
        if pixels.len() < need {
            return Err(SurfaceError::new(
                "上传纹理",
                windows::core::Error::new(
                    E_INVALIDARG,
                    format!("像素 {need} 字节，实收 {}", pixels.len()),
                ),
            ));
        }
        let slot = self.alloc_srv_slot()?;

        let tex_desc = crate::surface::texture_desc((w, h), D3D12_RESOURCE_FLAG_NONE);
        let texture = create_resource(
            &self.device,
            &heap_props(D3D12_HEAP_TYPE_DEFAULT),
            &tex_desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;

        // ⚠ 行距与总量都必须用驱动给的，不能用 `tight * h`。
        //
        // 拷贝源是 `PLACED_FOOTPRINT` 时，D3D12 按 `GetCopyableFootprints` 返回的
        // `RowPitch` 读 —— 那是按 256 字节对齐过的。32×32 的图每行 128 字节，
        // 而驱动要 256，于是：
        //
        // - 按 `tight` 摆数据 → 每隔一行横向偏 128 字节（图像内容斜着错位）；
        // - 暂存缓冲按 `tight * h`（4096）建 → 驱动要 8064，`CopyTextureRegion`
        //   越界。
        //
        // 这两条**不报 HRESULT**，只有调试层会说 —— 而 `E_INVALIDARG` 那条只在
        // 内存访问真正越出去时才触发。所以先算 footprint、**再**按 `total` 定暂存
        // 大小，而不是反过来。
        let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut num_rows = 0u32;
        let mut row_size = 0u64;
        let mut total = 0u64;
        unsafe {
            self.device.GetCopyableFootprints(
                &tex_desc,
                0,
                1,
                0,
                Some(&mut footprint),
                Some(&mut num_rows),
                Some(&mut row_size),
                Some(&mut total),
            );
        }
        let row_pitch = footprint.Footprint.RowPitch as usize;
        if row_pitch < tight {
            return Err(SurfaceError::new(
                "上传纹理：驱动给的行距装不下一行像素",
                windows::core::Error::new(E_FAIL, format!("行距 {row_pitch} < 一行 {tight} 字节")),
            ));
        }
        let staging = UploadBuffer::new(&self.device, total.max(need as u64))?;

        unsafe {
            for row in 0..h as usize {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add(row * tight),
                    staging.mapped.add(row * row_pitch),
                    tight,
                );
            }
            // ⚠ 这里**不发** `GENERIC_READ → COPY_SOURCE` 屏障：上传堆资源只能处于
            // `GENERIC_READ`，状态转换本身是非法的。`GENERIC_READ` 正是拷贝源要的
            // 状态，直接拷即可。
            list.CopyTextureRegion(
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: 0,
                    },
                },
                0,
                0,
                0,
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(staging.resource.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: footprint,
                    },
                },
                None,
            );
            // 拷贝**之后**才能转回 PS 资源状态。转早了 GPU 会在拷贝中途重新排布它 ——
            // 而那次重排与拷贝的先后关系没有任何东西记录，症状是随机花屏。
            gpu_barrier(
                list,
                &texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
        }

        // 描述符**必须在记录之外**建：它是 CPU 侧立刻生效的状态，不是命令。
        //
        // 顺序上也无所谓先后（描述符内容对已记录的命令立刻可见），但要放在
        // `next_texture_slot` 递增之后 —— 万一上面某步已经失败了，就不该占掉槽位。
        //
        // 视图描述传 `None`：D3D12 会从**资源**推出格式与尺寸，覆盖全部 mip
        // （`texture_desc` 只建了 1 层）。手写一遍 `D3D12_SHADER_RESOURCE_VIEW_DESC`
        // 只会多一处「和资源格式对不上就花屏」的失同步点 —— 而格式一致性正是
        // 前面几个 bug 的同一类成因。
        unsafe {
            self.device
                .CreateShaderResourceView(&texture, None, self.srv_heap.cpu(slot)?);
        }

        self.textures.push(UploadedTexture {
            _resource: texture,
            _staging: staging,
            _slot: slot,
        });
        Ok(slot)
    }

    /// 回放一帧：普通段与离屏段按 `DisplayList` 顺序**交替**。
    ///
    /// # 为什么必须交替（E4.4）
    ///
    /// 带非恒等滤镜的命令必须先被画进离屏 RT、模糊过、再合成回来，顺序必须等于绘制
    /// 顺序。先跑完所有离屏的话，离屏结果会落到**所有**普通图元之下（同一张卡片的背景
    /// 会盖住它自己的阴影）；反过来则模糊文字会被卡片背景盖住。两种顺序都只对一半。
    ///
    /// # 分段为什么**不**额外分配
    ///
    /// 整帧无滤镜时 `plan_segments` 返回单个覆盖全部的 `Plain(0..len)`，于是循环体与
    /// 从前**逐字节**同路径。
    pub fn record(&mut self, ctx: RenderCtx<'_>) -> Result<RenderReport, SurfaceError> {
        let mut targets = self
            .default_targets
            .take()
            .expect("默认目标只在 record 借用期间为 None");
        let result = self.record_with(&mut targets, ctx);
        self.default_targets = Some(targets);
        result
    }

    /// 用**指定窗口**的渲染目标记录一帧。
    ///
    /// 与 [`Self::record`] 逐字节同路径，只是几何缓冲 / 离屏 RT / 目标副本 / 钩子
    /// 都取自 `targets` 而不是渲染器的默认那一套。多窗口宿主走这条。
    /// 为 ext pool 中尚未分配 SRV 槽位的外部资源创建描述符（D3D12 通用采样路径）。
    fn alloc_srv_slot(&mut self) -> Result<u32, SurfaceError> {
        // 本后端 `begin_frame()` 对队列做全量 `wait_gpu()`，`sync_ext_texture_srvs` 在其后录制，
        // 因此此前帧引用的 SRV 描述符已无在飞 GPU 读取；回收槽位可在此安全复用。
        // 若未来改为 ring-buffer/多帧在飞，必须改为 fence 值延迟回收。
        if let Some(slot) = self.free_texture_slots.pop() {
            return Ok(slot);
        }
        // 跳过 DST/离屏保留槽（9/10/11）与 atlas(0)；若已进入保留区则推进。
        while self.next_texture_slot >= DST_SRV_SLOT && self.next_texture_slot < 12 {
            self.next_texture_slot += 1;
        }
        let slot = self.next_texture_slot;
        if slot >= self.srv_heap.count() {
            return Err(SurfaceError::new(
                "SRV 槽位分配",
                windows::core::Error::new(
                    E_FAIL,
                    format!(
                        "SRV 堆只有 {} 个槽位（0 号给图集），已用尽",
                        self.srv_heap.count()
                    ),
                ),
            ));
        }
        self.next_texture_slot = slot + 1;
        Ok(slot)
    }

    /// 把一份 CPU 像素拷进 `res.resource`（`resource` 为空时先创建纹理）。
    ///
    /// P0.2：`register_texture` 的首次上传与 `update_texture` 的逐帧更新共用这条
    /// 路径 —— 复用同一张纹理与 SRV，不重新分配。原地更新时先把纹理从
    /// `PIXEL_SHADER_RESOURCE` 转回 `COPY_DEST`（复制目标必须是这个状态）。
    /// staging 在 GPU 空闲点（`begin_frame` 的 `wait_gpu`）才由下一帧释放，且每帧
    /// 只处理队列里**最新**一份更新 ⇒ 同帧内不会丢弃仍被 GPU 引用的 staging。
    fn stage_ext_cpu_upload(
        &self,
        list: &ID3D12GraphicsCommandList,
        res: &mut crate::ext::ExtResource,
        cpu: &zephyra_ext::CpuTextureDescOwned,
    ) -> Result<(), SurfaceError> {
        let w = cpu.width.max(1);
        let h = cpu.height.max(1);
        let tight = (w as usize) * 4;
        let first = res.resource.is_none();
        if first {
            let tex_desc = texture_desc((w, h), D3D12_RESOURCE_FLAG_NONE);
            res.resource = Some(create_resource(
                &self.device,
                &heap_props(D3D12_HEAP_TYPE_DEFAULT),
                &tex_desc,
                D3D12_RESOURCE_STATE_COPY_DEST,
            )?);
        }
        let texture = res.resource.as_ref().expect("just ensured");
        let tex_desc = texture_desc((w, h), D3D12_RESOURCE_FLAG_NONE);
        let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut total = 0u64;
        unsafe {
            self.device.GetCopyableFootprints(
                &tex_desc,
                0,
                1,
                0,
                Some(&mut footprint),
                None,
                None,
                Some(&mut total),
            );
        }
        let row_pitch = footprint.Footprint.RowPitch as usize;
        if row_pitch < tight {
            return Err(SurfaceError::new(
                "ext CPU texture: row pitch too small",
                windows::core::Error::from(E_FAIL),
            ));
        }
        let staging = UploadBuffer::new(&self.device, total.max((row_pitch * h as usize) as u64))?;
        unsafe {
            for row in 0..h as usize {
                let dst_off = row * row_pitch;
                let src_off = row * (cpu.row_pitch.max(tight as u32) as usize);
                let copy = tight.min(cpu.pixels.len().saturating_sub(src_off));
                if copy > 0 {
                    std::ptr::copy_nonoverlapping(
                        cpu.pixels.as_ptr().add(src_off),
                        staging.mapped.add(dst_off),
                        copy,
                    );
                }
            }
            if !first {
                crate::surface::Gpu::barrier(
                    list,
                    texture,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    D3D12_RESOURCE_STATE_COPY_DEST,
                );
            }
            list.CopyTextureRegion(
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: 0,
                    },
                },
                0,
                0,
                0,
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(staging.resource.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: footprint,
                    },
                },
                None,
            );
            crate::surface::Gpu::barrier(
                list,
                texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
        }
        res.staging = Some(staging);
        Ok(())
    }

    fn sync_ext_texture_srvs(
        &mut self,
        list: &ID3D12GraphicsCommandList,
        ext_pool: &mut crate::ext::ExtPool,
    ) -> Result<(), SurfaceError> {
        // `begin_frame()` 已做全量 `wait_gpu()`，此处 GPU 空闲，可安全释放上一帧/卸载退役资源。
        ext_pool.release_retired();
        // 回收已卸载/替换的 ext SRV 槽位（131：与 ext 生命周期同步）。
        let live: std::collections::HashSet<String> = ext_pool.textures.keys().cloned().collect();
        let stale: Vec<String> = self
            .ext_srv_keys
            .keys()
            .filter(|k| !live.contains(*k))
            .cloned()
            .collect();
        for k in stale {
            if let Some(slot) = self.ext_srv_keys.remove(&k) {
                self.free_texture_slots.push(slot);
            }
        }
        for (k, slot) in self.ext_srv_keys.clone() {
            if let Some(res) = ext_pool.textures.get(&k) {
                if res.srv_slot != Some(slot) {
                    self.ext_srv_keys.remove(&k);
                    self.free_texture_slots.push(slot);
                }
            }
        }

        let keys: Vec<String> = ext_pool.textures.keys().cloned().collect();
        for key in keys {
            let Some(res) = ext_pool.textures.get_mut(&key) else {
                continue;
            };
            // CPU 首次上传 / 逐帧原地更新共用同一条 staging 路径。每帧只取队列里
            // 最新的一份更新 —— 相机帧生产快于渲染帧时丢中间帧是正确语义。
            let pending = res
                .pending_cpu
                .take()
                .or_else(|| res.pending_updates.drain(..).next_back());
            if let Some(cpu) = pending {
                self.stage_ext_cpu_upload(list, res, &cpu)?;
            }
            if let Some(slot) = res.srv_slot {
                self.ext_srv_keys.insert(key.clone(), slot);
                continue;
            }
            let Some(resource) = &res.resource else {
                continue;
            };
            let slot = match self.alloc_srv_slot() {
                Ok(s) => s,
                Err(_) => {
                    zephyra_ext::debug::log_fallback(
                        "d3d12 ext SRV heap full; skip auto-binding (use hook + ext_pool)",
                    );
                    break;
                }
            };
            unsafe {
                self.device
                    .CreateShaderResourceView(resource, None, self.srv_heap.cpu(slot)?);
            }
            res.srv_slot = Some(slot);
            self.ext_srv_keys.insert(key, slot);
        }
        Ok(())
    }

    pub fn record_with(
        &mut self,
        targets: &mut WindowTargets,
        ctx: RenderCtx<'_>,
    ) -> Result<RenderReport, SurfaceError> {
        targets
            .ext_frame
            .set(targets.ext_frame.get().wrapping_add(1));
        // ★ 新的一帧 ⇒ 上一帧留下的 `g_dst` 副本作废。
        //
        // 不推进它的话，「内容没变就不重拷」那条捷径会在**跨帧**时误判：
        // 目标每帧都被清屏重画，内容早就变了，而版本号还停在上一帧。
        targets
            .content_revision
            .set(targets.content_revision.get().wrapping_add(1));
        {
            let mut pool = targets.ext_pool.borrow_mut();
            self.sync_ext_texture_srvs(ctx.list, &mut pool)?;
        }
        let RenderCtx {
            device,
            list,
            rtv,
            display_list,
            size,
            clear,
            textures,
            rt,
            damage,
        } = ctx;

        let ext_slots = targets.ext_pool.borrow().srv_slots_snapshot();
        let env = CmdEnv {
            rtv,
            size,
            textures,
            ext_slots: Some(&ext_slots),
            rt,
            damage,
            // 离屏那条命令的 `SortKey` 由 `plan_segments` 按下标给出并覆盖它；这个默认值
            // 只在普通段被读到，而普通段不碰它。
            key: zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: 0,
            },
        };

        // ---- 1. 根签名 / 描述符堆 / 目标状态 ----
        //
        // 每段都会重设，这里设的是「清屏」所需的那一次。
        //
        // ⚠ 先把上传暂存的写游标归零。
        //
        // 归零之所以安全，是因为 [`crate::surface::Gpu::begin_frame`] **第一件事就是
        // `wait_gpu`** —— 进入 `record` 时 GPU 已经读完了上一帧的全部 `CopyBufferRegion`，
        // 暂存区可以整段复用。这不是这里自己做的假设，而是宿主已经强制的前提。
        //
        // 漏掉这一步的后果是「跑几千帧之后画面慢慢变乱」，而不是立刻报错：游标一路涨到
        // 暂存区末尾，撞上 `record_upload` 的越界检查才失败，于是错在离原因很远的地方。
        targets.vertex.head.set(0);
        targets.index.head.set(0);
        // 上一帧退役的旧缓冲到此作废：`begin_frame` 已经 `wait_gpu`，GPU 空闲，
        // 它们引用的所有 `CopyBufferRegion` 都读完了。
        targets.vertex.clear_retired();
        targets.index.clear_retired();
        //
        // ⚠ `t0` **也**必须先绑一次（绑图集兜住）：D3D12 的根描述符表在命令列表打开时
        // 是**未设置**的，而未设置在调试层不一定报错 —— 症状是采样读到随机内容，于是某个
        // 混合模式看起来生效了、另一个完全不对。
        unsafe {
            list.SetGraphicsRootSignature(&self.root_signature);
            list.SetDescriptorHeaps(&[Some(self.srv_heap.heap.clone())]);
            list.SetGraphicsRootDescriptorTable(0, self.srv_heap.gpu(self.atlas_srv_slot)?);
            list.SetGraphicsRootDescriptorTable(1, self.srv_heap.gpu(self.atlas_srv_slot)?);
        }
        self.bind_state(list, &env, size, damage)?;
        // ⚠ 清屏色必须**预乘**：目标纹理从此与顶点色在同一个预乘空间里。漏掉这一步，一个
        // 半透明的 clear 色会让整帧偏亮，且只有非 1.0 的 clear 色才暴露 —— 而默认 clear
        // 是全 1.0 或全 0.0，于是永远看不出来。
        //
        // `Color` 存的是**线性** 0..1，而 `ClearRenderTargetView` 要的也是线性。
        //
        // ⚠⚠ `pRects` 决定清屏区域，**与 scissor 无关**（见 [`damage_scissor`] 的说明）。
        // 这里必须按脏区清：整屏清 + 只画脏区 = 脏区之外全被抹成 clear 色，而
        // 「点一下之后界面花掉」正是这么来的。
        let ca = clear.a;
        let (_sc, clear_rects) = damage_clear_rects(size, damage);
        let rect_slice: Option<&[RECT]> = clear_rects.as_ref().map(|r| r.as_slice());
        unsafe {
            list.ClearRenderTargetView(
                rtv,
                &[clear.r * ca, clear.g * ca, clear.b * ca, ca],
                rect_slice,
            );
        }

        // ---- 2. 分段 ----
        let mut report = RenderReport::default();
        // ⚠ `ZEPHYRA_NO_FILTERS=1` 把所有离屏（滤镜）段降级成普通段 —— 于是**整条离屏
        // 路径一次都不跑**：不建 RT、不写描述符、不发那几道跨状态屏障。
        //
        // # 这是给 GPU 挂掉时做二分用的开关，不是产品选项
        //
        // 本机反复出现 `Present` 报 `DEVICE_REMOVED` 而 `GetDeviceRemovedReason` 说
        // `DEVICE_HUNG`（真 TDR），而**调试层在本机两条路都不可用**（事后开启会让设备
        // 刚建好就挂；改成建设备带 `D3D12_CREATE_DEVICE_DEBUG` 后直接访问违例退出）。
        // 拿不到「哪个 API」的证据，只能靠**关掉子系统**二分。
        //
        // 离屏路径是首要嫌疑：本仓自己记过两处 `DEVICE_HUNG` 都在这里 —— 末趟结果还停在
        // `RENDER_TARGET` 却被当采样源，以及往 `srv_heap` 写新描述符后没重新
        // `SetDescriptorHeaps`。两处都已修，但离屏 RT 在 resize 时会**按新尺寸重建**，
        // 而它的状态追踪是跨帧的 —— 重建与状态推理的交界处仍是最可疑的一段。
        let segments = zephyra_fx::offscreen::plan_segments(display_list);
        let no_filters = filters_disabled();
        for seg in segments {
            match seg {
                zephyra_fx::offscreen::Segment::Plain(range) => {
                    if range.is_empty() {
                        continue;
                    }
                    let pr = self.draw_range(targets, list, device, &env, display_list, &range)?;
                    if std::env::var_os("ZEPHYRA_FILTER_TRACE").is_some()
                        && pr.unimplemented_filters > 0
                    {
                        eprintln!(
                            "[ftrace] PLAIN {:?} unimpl={} offscreen={} refract={}",
                            range,
                            pr.unimplemented_filters,
                            pr.offscreen_passes,
                            pr.refraction_passes
                        );
                    }
                    report.absorb(pr);
                }
                zephyra_fx::offscreen::Segment::Custom { index } => {
                    let item = display_list.iter().nth(index).expect("下标来自同一份列表");
                    let zephyra_core::paint::Command::Custom {
                        bounds,
                        source,
                        resource_key,
                        ..
                    } = &item.command
                    else {
                        unreachable!("Custom 段只由 Command::Custom 产出");
                    };
                    let key_str = resource_key.as_deref();
                    let hook_kind = source.kind.as_ref();
                    let ext_pool = targets.ext_pool.borrow();
                    let imported_payload = key_str
                        .and_then(|k| ext_pool.get(k))
                        .map(|r| r as &dyn std::any::Any);
                    let payload = imported_payload
                        .unwrap_or_else(|| source.payload_any() as &dyn std::any::Any);
                    let mut ctx = HookCtx {
                        device: device as *const _,
                        list: (list as *const _) as *mut _,
                        rtv: env.rtv,
                        size: (env.size.0 as f32, env.size.1 as f32),
                        frame_index: targets.ext_frame.get(),
                        ext_pool: Some(&*ext_pool as *const _),
                    };
                    let mut handled = false;
                    {
                        let mut ext_hooks = targets.ext_hooks.borrow_mut();
                        if let Some(h) = ext_hooks.global.get_mut(hook_kind) {
                            let ext_src = zephyra_render_ext::registration::RenderSource::new(
                                key_str,
                                Some(payload),
                            );
                            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                h(&mut ctx, core_rect_to_ext(*bounds), &ext_src);
                            }));
                            if r.is_ok() {
                                handled = true;
                            } else {
                                zephyra_ext::debug::log_hook(&format!(
                                    "ext hook panicked: kind={hook_kind} key={:?}",
                                    key_str
                                ));
                            }
                        } else if let Some(h) = ext_hooks
                            .window
                            .get_mut(&(targets.window_id, hook_kind.to_string()))
                        {
                            // P1.1：按 `(窗口 id, kind)` **精确**匹配 —— 早先只按
                            // kind `.find`，A 窗口注册的钩子会被 B 窗口命中（多窗口
                            // 下同 kind 不同行为的场景直接串台）。
                            {
                                let ext_src = zephyra_render_ext::registration::RenderSource::new(
                                    key_str,
                                    Some(payload),
                                );
                                let r =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        h(&mut ctx, core_rect_to_ext(*bounds), &ext_src);
                                    }));
                                if r.is_ok() {
                                    handled = true;
                                } else {
                                    zephyra_ext::debug::log_hook(&format!(
                                        "ext hook panicked: kind={hook_kind} key={:?}",
                                        key_str
                                    ));
                                }
                            }
                        }
                    }
                    if !handled {
                        if let Some((_, hook)) = targets
                            .hooks
                            .iter_mut()
                            .find(|(k, _)| k.as_ref() == hook_kind)
                        {
                            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                hook.draw_with_key(ctx, *bounds, source, key_str);
                            }));
                            if r.is_ok() {
                                handled = true;
                            } else {
                                zephyra_ext::debug::log_hook(&format!(
                                    "internal hook panicked: kind={hook_kind} key={:?}",
                                    key_str
                                ));
                            }
                        }
                    }
                    drop(ext_pool);
                    if handled {
                        report.custom_passes += 1;
                    } else {
                        report.unhandled_custom += 1;
                    }
                }
                zephyra_fx::offscreen::Segment::Offscreen {
                    index,
                    pass,
                    redraw,
                    key,
                } => {
                    if no_filters {
                        // 降级成「什么都不做」而不是「画原命令」：滤镜一旦不生效，
                        // 画原始内容（比如一张被模糊的图）会给出**看起来正常**的画面，
                        // 让人以为二分没生效。只有什么都不画才能确定「这条路径没跑」。
                        report.unimplemented_filters += 1;
                        continue;
                    }
                    let item = display_list.iter().nth(index).expect("下标来自同一份列表");
                    let inner = CmdEnv { key, ..env };
                    let r =
                        self.render_offscreen(targets, list, device, &inner, &item.command, &pass)?;
                    if std::env::var_os("ZEPHYRA_FILTER_TRACE").is_some() {
                        eprintln!(
                            "[ftrace] inner #{} unimpl={} offscreen={} refract={}",
                            index, r.unimplemented_filters, r.offscreen_passes, r.refraction_passes
                        );
                    }
                    report.absorb(r);
                    // 滤镜真的被执行了 —— 从 `unimplemented_filters` 里减掉这一条。
                    report.unimplemented_filters = report.unimplemented_filters.saturating_sub(1);
                    // 投影：影子是**额外**画上去的一层，原图仍要画在它上面。
                    //
                    // ⚠ 这一步漏掉时症状特别有欺骗性：影子画出来了（那正是这条命令唯一
                    // 的可见效果），而元素本体不见了 —— 于是看起来像「投影把元素盖住了」，
                    // 不像「少画了一步」。
                    if redraw {
                        let r = self.draw_range(
                            targets,
                            list,
                            device,
                            &env,
                            display_list,
                            &(index..index + 1),
                        )?;
                        // ⚠⚠ 重画原图这一趟**已经把上面的 `saturating_sub(1)`
                        // 又抵消掉了**，所以这里必须再减一次。
                        //
                        // 账是这样走的（`unimplemented_filters` 的定义是「有滤镜
                        // 但**没被执行**的命令数」）：
                        //
                        // | 步 | `unimplemented_filters` |
                        // |---|---|
                        // | `expand` 展开这条命令 | +1 |
                        // | 离屏 pass 内部 `saturating_sub(1)` | −1（归零）|
                        // | `absorb(inner)` | +0 |
                        // | 上面的 `saturating_sub(1)` | −1（0 处饱和，保持 0）|
                        // | **本行** `draw_range` 又展开同一条 | **+1 ← 元凶** |
                        //
                        // 症状：`unimplemented_filters` **永远比 0 大 1**，而它按
                        // 定义是「谁没接上」的唯一诊断信号 —— 于是每次用折射都
                        // 报「有一条滤镜没实现」，而画面其实是对的。
                        //
                        // 那种信号比没有更糟：它恒为非零，于是「它是不是红的」
                        // 不再携带任何信息（本条就是在 `refraction_bends_the_backdrop_on_d3d12`
                        // 里被抓住的 —— 那条测试的第一句断言就是这个数必须是 0）。
                        //
                        // 重画阶段要减一，是因为滤镜**已经在离屏 pass 里做过**，
                        // 这一趟画的只是元素本体。
                        //
                        // ⚠ 减法必须在 `absorb` **之后**：反过来写就是「先减再原样
                        // 加回来」，净效果为零 —— 而症状与没写一样（计数恒为 1），
                        // 所以这个顺序错误不会自己暴露。
                        report.absorb(r);
                        report.unimplemented_filters =
                            report.unimplemented_filters.saturating_sub(1);
                    }
                }
            }
        }
        Ok(report)
    }

    /// 绑渲染目标 + 视口 + scissor。
    ///
    /// # 为什么每段都要调
    ///
    /// 离屏 pass 会把目标换成离屏 RT 并在它们之间切换资源状态；沿用上一段的状态是**错的**
    /// 而不是快的。D3D12 对状态错误的报错又指向别处（「resource is in an incompatible
    /// state」），排查成本很高。
    fn bind_state(
        &self,
        list: &ID3D12GraphicsCommandList,
        env: &CmdEnv<'_>,
        size: (u32, u32),
        damage: zephyra_core::frame::Damage,
    ) -> Result<(), SurfaceError> {
        unsafe {
            list.RSSetViewports(&[D3D12_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: size.0 as f32,
                Height: size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]);
            // D3D12 **没有** `ScissorEnable` 开关：裁剪永远生效。所以只设视口、忘了裁剪，
            // 就得到「默认裁剪矩形是空的 → 什么都不画且零错误」。
            list.RSSetScissorRects(&[damage_scissor(size, damage)]);
            // ⚠ `OMSetRenderTargets` 必须在这里设。
            //
            // 漏掉时**不报错**：命令列表的 RTV 绑定默认是空描述符，于是
            // `ClearRenderTargetView` 照常工作（它把 RTV 当**参数**收），而
            // `DrawIndexedInstanced` 一个像素都不画 —— 症状是「只剩清屏色，而
            // `RenderReport` 里 quads 一切正常」。
            list.OMSetRenderTargets(1, Some(&env.rtv), false, None);
        }
        Ok(())
    }

    /// 把 `range` 这段普通图元展开、上传、回放。
    ///
    /// 顶点 / 索引缓冲是**共用**的，而离屏 pass 会用四边形覆写它们。所以每段都是
    /// 「展开 → 上传 → 立刻画完」，不能攒着一起画。
    fn draw_range(
        &mut self,
        targets: &mut WindowTargets,
        list: &ID3D12GraphicsCommandList,
        device: &ID3D12Device,
        env: &CmdEnv<'_>,
        display_list: &zephyra_core::paint::DisplayList,
        range: &std::ops::Range<usize>,
    ) -> Result<RenderReport, SurfaceError> {
        // ⚠ `Scratch` 是**取出来**用的：`expand_*` 的 `is_registered` 闭包要借
        // `env.textures`，同时又要 `&mut targets.scratch` —— `mem::take` 把借用链切断。
        let mut scratch = std::mem::take(&mut targets.scratch);
        let mut report = quad::expand_range_translated_into(
            display_list,
            env.size,
            &|n: &str| env.textures.contains(n) || env.ext_slots.is_some_and(|m| m.contains_key(n)),
            zephyra_core::core::Point::new(0.0, 0.0),
            Some(range.clone()),
            &mut scratch,
        );
        report.vertex_bytes = (scratch.vertices.len() * std::mem::size_of::<Vertex>()) as u64;
        report.quads = scratch.solid.len() / 4 + scratch.images.len() / 4;
        report.path_triangles = scratch.paths.len() / 3;
        self.upload_geometry(targets, device, list, &scratch, &mut report)?;
        self.bind_state(list, env, env.size, env.damage)?;
        if !scratch.vertices.is_empty() {
            self.replay(
                targets,
                list,
                device,
                env,
                scratch.runs(),
                scratch.path_first,
                scratch.path_index_start,
                BlendMode::SrcOver,
                // 普通段画的是主目标（MSAA 开时多重采样）。
                false,
                self.samples,
                &mut report,
            )?;
        }
        targets.scratch = scratch;
        Ok(report)
    }

    /// 顶点 / 索引缓冲按需增长，然后上传。
    fn upload_geometry(
        &mut self,
        targets: &mut WindowTargets,
        device: &ID3D12Device,
        list: &ID3D12GraphicsCommandList,
        scratch: &quad::Scratch,
        report: &mut RenderReport,
    ) -> Result<(), SurfaceError> {
        let needed = report.vertex_bytes.max(64);
        // 容量判据必须含已经花掉的 `head` —— 主路径与离屏 pass 共用这条缓冲，
        // 见 `StreamBuffer::reserve`。
        if targets.vertex.reserve(device, needed)? {
            targets.vertex_capacity = targets.vertex.size;
            report.reallocated = true;
        }
        // 索引要逐四边形加顶点基址，而四边形数逐帧可变，所以索引每帧都不同。
        //
        // ⚠ 四边形数必须**只数 solid + images** —— 用 `vertices.len() / 4` 会把路径顶点
        // 也除进去，而路径顶点数是 3 的倍数，整数截断会让四边形段的基址全部错位。
        // ★ 几何**逐字节**没变就不重传。
        //
        // 为什么值得：UI 静止时宿主虽然会跳过整帧，但「只改颜色/只动 hover」的帧仍然
        // 要画 —— 而那一帧的顶点流与上一帧**逐字节相同**。demo 实测每帧顶点 55 KB
        // （141 四边形），也就是每次这样的交互都白传 55 KB 外加一对屏障。
        //
        // 为什么用**逐字节比较**而不是哈希：哈希相等不等于内容相等，一旦碰撞就会跳过
        // 一次真上传，画面直接错一帧且此后不再自愈 —— 那是「看起来时好时坏」的一类
        // bug，极难往哈希上联想。55 KB 的 memcmp 在 CPU 上是微秒级。
        if !scratch.indices.is_empty() {
            let need = (scratch.indices.len() * 2) as u64;
            targets.index.reserve(device, need)?;
            let bytes = bytemuck::cast_slice::<u16, u8>(&scratch.indices);
            if targets.last_index_bytes != bytes {
                targets.index.record_upload(list, bytes)?;
                targets.last_index_bytes.clear();
                targets.last_index_bytes.extend_from_slice(bytes);
                report.index_uploads = report.index_uploads.saturating_add(1);
            } else {
                report.index_upload_skipped = true;
            }
        }
        if !scratch.vertices.is_empty() {
            let bytes = bytemuck::cast_slice::<Vertex, u8>(&scratch.vertices);
            if targets.last_vertex_bytes != bytes {
                targets.vertex.record_upload(list, bytes)?;
                targets.last_vertex_bytes.clear();
                targets.last_vertex_bytes.extend_from_slice(bytes);
                report.index_uploads = report.index_uploads.saturating_add(1);
            } else {
                report.index_upload_skipped = true;
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    // 参数是画一次所需的全部输入，彼此没有可成组打包的语义 —— 凑一个 struct 只是把
    // 8 个参数搬进另一个名字，读的人反而要多跳一层。见 AGENTS.md「代码自解释优先」。
    /// 逐 `DrawRun` 回放：一段一个 draw，PSO 与混合模式**成对**换。
    ///
    /// # 为什么按 [`quad::DrawRun`] 走而不是按「矩形/图像/多边形」分三段
    ///
    /// 混合模式是**逐命令**的属性：同一个 `DisplayList` 里完全可能前一条是 `SrcOver`、
    /// 后一条是 `Multiply`。按图元类型分三段的话，中间那段会被拆到别处，绘制顺序就不再
    /// 等于 `DisplayList` 的顺序 —— 半透明一多结果就错。
    ///
    /// # 为什么每段都要 `SetPipelineState`
    ///
    /// D3D12 的混合状态**烤在 PSO 里**，不能像 D3D11 那样只换 `OMSetBlendState`。
    /// PSO 变体与混合模式必须**成对**决定：
    ///
    /// | 模式            | PS 变体       | 混合状态           |
    /// |-----------------|---------------|--------------------|
    /// | 系数可表达 15 个 | `PSMain`       | 该模式的系数对      |
    /// | 可分离/非可分离  | 各自的合成变体 | **`Src`**（ONE/ZERO）|
    ///
    /// 第二行为什么用 `Src`：那些 PS 已经把 `dst` 算进去、写出了**最终**颜色，固定功能
    /// 方程只能原样接受。配成 `SrcOver` 会再叠一次 ⇒ 整体过曝。
    ///
    /// PSO 按 `(PS 变体, 模式, 采样数)` 缓存（[`Renderer::pso_for`]），懒建。
    fn replay(
        &mut self,
        targets: &mut WindowTargets,
        list: &ID3D12GraphicsCommandList,
        device: &ID3D12Device,
        env: &CmdEnv<'_>,
        runs: &[zephyra_fx::quad::DrawRun],
        path_first: u32,
        path_index_start: u32,
        default_blend: BlendMode,
        // 本趟画的是「刚清成全 0 的离屏 RT_A」还是真实目标。
        //
        // `true` = 内容趟：混合状态**一律 `Src`**，命令的混合模式留给合成趟
        // （对真实目标生效）。`false` = 普通段 / 重画段：按 `run.blend`
        // 逐段决定 PS 变体与混合状态。
        into_fresh_rt: bool,
        // 本趟目标的采样数。**不是** `self.samples`：离屏内容趟画的是单采样
        // RT_A，用 `self.samples`（MSAA 开时是 4）建 PSO 就是「4× PSO 画进
        // 1× RTV」—— 调试层 614，严格的驱动在 `Close()` 就拒。
        samples: u32,
        // 帧统计。PS 变体真的画上时要减 `unimplemented_blends` —— 那个计数
        // 按**命令**计（fx 侧），而这里按**段**走，所以减 `run.commands`。
        report: &mut RenderReport,
    ) -> Result<(), SurfaceError> {
        unsafe {
            // 索引缓冲**必须**在 draw 之前绑定 —— 漏掉它不报错，只是整帧什么都不画。
            list.IASetVertexBuffers(
                0,
                Some(&[D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: targets.vertex.gpu_addr(),
                    SizeInBytes: targets.vertex_capacity as u32,
                    StrideInBytes: std::mem::size_of::<Vertex>() as u32,
                }]),
            );
            list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: targets.index.gpu_addr(),
                SizeInBytes: targets.index.size as u32,
                Format: DXGI_FORMAT_R16_UINT,
            }));
            list.IASetPrimitiveTopology(
                windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            );
        }
        for run in runs {
            if run.vertex_count == 0 {
                continue;
            }
            let mode = run.blend as usize;
            let (ps_idx, pso_mode) = if into_fresh_rt {
                // 内容趟：元素画进**刚清成全 0** 的 RT_A，混合状态必须 `Src`。
                //
                // 按 `run.blend` 走的后果分两种，都错且都不报错：
                // - 系数模式读 dst（`Modulate` / `DstIn` / `Clear`…）：
                //   RT_A 全是 0 ⇒ 内容变成纯黑，合成上去是一团黑；
                // - PS 变体：`refresh_dst_copy` 拷的是同一张全 0 的 RT_A
                //   ⇒ 对着零混合（Multiply 出纯黑）。
                //
                // 命令的混合模式属于**合成趟** —— 那里才是「与真实目标」混合。
                // D3D11 侧的内容趟一直是默认 `SrcOver`（对全 0 等价于 Src），
                // 两边由此对齐（2026-10-25 由 D3D11 侧压力场景揭出后同修）。
                //
                // 顺带省掉每条离屏命令一次 dst 拷贝 + 一对屏障。
                (0usize, BlendMode::Src as usize)
            } else {
                match shader::ps_for_blend_mode(mode) {
                    // 系数可表达：PS 只算源色，混合交给固定功能方程。
                    None => (0usize, mode),
                    // PS 合成模式：要读 `dst`，先取副本；取不到就退回默认。
                    Some(k) => match self
                        .refresh_dst_copy(targets, device, list, env.rt, env.size, false, None)
                    {
                        Some(dst_slot) => {
                            // 副本绑到 `t1`（根签名第 1 个参数）。
                            unsafe {
                                list.SetGraphicsRootDescriptorTable(
                                    1,
                                    self.srv_heap.gpu(dst_slot)?,
                                );
                            }
                            // PS 已写出最终颜色 ⇒ 混合状态用 `Src`。
                            //
                            // ★ 减账：这条（些）命令的混合**被执行**了。fx 侧按命令
                            // 计 `unimplemented_blends`，而相邻同模式命令在这里合并成
                            // 一段 —— 按段减会少减、计数器继续说谎（2026-10-24 揭出）。
                            report.unimplemented_blends = report
                                .unimplemented_blends
                                .saturating_sub(run.commands as usize);
                            (k, BlendMode::Src as usize)
                        }
                        None => (0usize, default_blend as usize),
                    },
                }
            };
            let pso = self.pso_for(device, self.elements.descs(), ps_idx, pso_mode, samples)?;
            // 纹理绑定。只有 `Command::Image` 逐段换；矩形 / 字形 / 多边形记 `None`，
            // 绑图集兜住（`g_tex` 在 PS 里被声明，`t0` 悬空会让某些驱动解除引用并崩溃）。
            let slot = match &run.texture {
                Some(name) => {
                    if let Some(s) = env.textures.slot(name) {
                        s
                    } else if let Some(s) =
                        env.ext_slots.and_then(|m| m.get(name.as_ref()).copied())
                    {
                        s
                    } else {
                        // 顶点收集阶段已过滤掉未注册纹理，这里本不该发生。
                        continue;
                    }
                }
                None => self.atlas_srv_slot,
            };
            // 段落定位：`runs` 记**顶点**下标，`DrawIndexedInstanced` 要**索引**下标。
            // 文本 4 顶点 → 6 索引（`/4*6`），路径是 1:1（每个顶点恰好被引用一次）。
            let (index_start, index_count) = if run.first_vertex >= path_first {
                (
                    path_index_start + (run.first_vertex - path_first),
                    run.vertex_count,
                )
            } else {
                ((run.first_vertex / 4) * 6, (run.vertex_count / 4) * 6)
            };
            unsafe {
                list.SetPipelineState(&pso);
                list.SetGraphicsRootDescriptorTable(0, self.srv_heap.gpu(slot)?);
                list.DrawIndexedInstanced(index_count, 1, index_start, 0, 0);
            }
            // ★ 目标的内容变了 ⇒ `g_dst` 副本作废。
            //
            // 只在**主路径**（`into_fresh_rt == false`，即画进真实目标）推进；
            // 离屏段画的是离屏 RT，与目标无关。
            if !into_fresh_rt {
                targets
                    .content_revision
                    .set(targets.content_revision.get() + 1);
            }
        }
        Ok(())
    }

    /// 离屏三步（E4.4 / E4.5 / E4.6）：内容 → 模糊（乒乓）→ 合成。
    ///
    /// # 资源状态为什么要**手工跟踪**
    ///
    /// D3D12 的屏障必须是「从**当前**状态到目标状态」，而当前状态是**记录期**的一份
    /// 推理，不是资源里的字段。乒乓意味着同一张 RT 在一趟里当写入目标、在下一趟里
    /// 当读取源：
    ///
    /// ```text
    ///   内容  → A: RENDER_TARGET（刚建出来）
    ///   趟1 H → 读 A（转 PSR）、写 B（已是 RT）
    ///   趟2 V → 读 B（转 PSR）、写 A（要转回 RT）
    ///   合成  → 读末趟结果（已是 PSR）、写目标（宿主保证是 RT）
    /// ```
    ///
    /// 无条件地每趟发两道屏障是**错的**：从资源并不处于的状态转换会被调试层判为非法。
    ///
    /// # 顺序：**先绑目标，再发屏障**
    ///
    /// 见 [`rt_barrier`] —— 资源必须先从 RTV 绑定上解绑才能转状态。
    fn render_offscreen(
        &mut self,
        targets: &mut WindowTargets,
        list: &ID3D12GraphicsCommandList,
        device: &ID3D12Device,
        cmd_env: &CmdEnv<'_>,
        cmd: &zephyra_core::paint::Command,
        pass: &zephyra_fx::offscreen::OffscreenPass,
    ) -> Result<RenderReport, SurfaceError> {
        use windows::Win32::Graphics::Direct3D12::{
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET,
        };
        use zephyra_fx::offscreen;

        let mut report = RenderReport::default();
        let rt_size = pass.rt_size;
        let Some((a, b)) = offscreen_pair(
            &targets.offscreen,
            device,
            &targets.offscreen_rtv_heap,
            &self.srv_heap,
            rt_size,
        ) else {
            report.unimplemented_filters += 1;
            return Ok(report);
        };
        // ⚠⚠ **刚往 `srv_heap` 里写了新描述符（两张离屏 RT 的 SRV），必须重新设一次堆。**
        //
        // `record` 的开头已经 `SetDescriptorHeaps` 过了，而 D3D12 明确规定：**堆已经设在
        // 命令列表上时，往里面写描述符是非法的**；要么之后再 `SetDescriptorHeaps` 通知
        // 运行时，要么调 `IncrementDirtyDescriptorCount`。见
        // <https://learn.microsoft.com/windows/win32/api/d3d12/nf-d3d12-id3d12device-setdescriptorheaps>
        //
        // # 症状（本项目实测）
        //
        // 画面**完全正确**、像素断言全绿、`submit` 与 `wait_gpu` 都返回成功，然后
        // `GetDeviceRemovedReason()` 给出 `DXGI_ERROR_DEVICE_HUNG`。调试层**不报**任何
        // 错误，`ZEPHYRA_D3D12_DEBUG=1` 也只有一条无害的清屏色警告。
        //
        // 所以单看一帧的像素永远发现不了它 —— 只有**下一帧创建第二个设备**时报
        // `D3D12CreateDevice` 失败才暴露，而那时错已经隔了整整一条命令列表。
        //
        // 代价：一次 `SetDescriptorHeaps`（每个离屏 pass 一次）。忽略无滤镜的帧就省掉了。
        unsafe {
            list.SetDescriptorHeaps(&[Some(self.srv_heap.heap.clone())]);
        }

        // ---- 1. 把内容画进 RT_A（按 translate 平移）----
        //
        // ⚠⚠ **RT_A 必须在画内容之前回到 `RENDER_TARGET`。**
        //
        // 上一帧的合成会把末趟结果转成 `PIXEL_SHADER_RESOURCE`，而末趟可能就是 A
        // （一奇数趟的链就是这样）。RT 对是跨帧复用的，所以进入第二帧时 A 停在 PSR
        // —— 直接 `OMSetRenderTargets(A)` + 清屏 + 画，就是**往一张 PSR 纹理里画**。
        //
        // 第一帧完全正常（新建的 RT 本来就是 `RENDER_TARGET`），第二帧起才错 ——
        // 这正是它能一路躲过所有单帧测试的原因。症状在 NVIDIA 上也只是「第二帧的
        // 模糊偏亮一点」，调试层才说得清：
        //
        // ```text
        // ID3D12_MESSAGE_ID(527) ResourceBarrier: Before state (PIXEL_SHADER_RESOURCE)
        // ... does not match with the current resource state (RENDER_TARGET)
        // ```
        //
        // ⚠ 顺序仍然必须**先绑后发**（资源要先从 RTV 绑定上解绑才能转状态）。
        unsafe {
            list.OMSetRenderTargets(1, Some(&a.rtv(&targets.offscreen_rtv_heap)?), false, None);
        }
        rt_barrier(
            list,
            &a.resource,
            a.state.get(),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        a.state.set(D3D12_RESOURCE_STATE_RENDER_TARGET);
        //
        // ⚠ 这里必须只画**这一条**命令（`single_command_list` 已把列表限成一条，
        // 所以展开区间传 `None`）。若传整份 `DisplayList`，所有图元都会被画进这张
        // 离屏 RT —— 症状是每个带滤镜的元素都带着一整个窗口的模糊副本。
        let mut scratch = std::mem::take(&mut targets.scratch);
        let sub = offscreen::single_command_list(cmd, cmd_env.key);
        let mut inner_report = quad::expand_range_translated_into(
            &sub,
            rt_size,
            &|n: &str| {
                cmd_env.textures.contains(n) || cmd_env.ext_slots.is_some_and(|m| m.contains_key(n))
            },
            zephyra_core::core::Point::new(pass.translate.0, pass.translate.1),
            None,
            &mut scratch,
        );
        inner_report.vertex_bytes = (scratch.vertices.len() * std::mem::size_of::<Vertex>()) as u64;
        inner_report.quads = scratch.solid.len() / 4 + scratch.images.len() / 4;
        self.upload_geometry(targets, device, list, &scratch, &mut inner_report)?;
        report.absorb(inner_report);

        let a_rtv = a.rtv(&targets.offscreen_rtv_heap)?;
        let inner_env = CmdEnv {
            rtv: a_rtv,
            size: rt_size,
            textures: cmd_env.textures,
            ext_slots: cmd_env.ext_slots,
            rt: Some(&a.resource),
            damage: zephyra_core::frame::Damage::Full,
            key: cmd_env.key,
        };
        self.bind_state(list, &inner_env, rt_size, zephyra_core::frame::Damage::Full)?;
        // RT_A 先清成**全 0**（不是清屏色）：离屏 RT 里存的是「这个元素自己」，它
        // 周围必须透明，否则模糊会把清屏色一起卷进去 —— 症状是元素周围一圈与背景
        // 同色的毛边。
        //
        // ⚠ 顺序：清屏在 `OMSetRenderTargets` **之后**（`bind_state` 已设）。反过来
        // 会把内容清掉，而两者顺序反了时画面是「一个全透明的离屏 + 目标上什么
        // 都没有」，同样零错误。
        unsafe {
            list.ClearRenderTargetView(a_rtv, &[0.0, 0.0, 0.0, 0.0], None);
        }
        self.replay(
            targets,
            list,
            device,
            &inner_env,
            scratch.runs(),
            scratch.path_first,
            scratch.path_index_start,
            BlendMode::SrcOver,
            // 内容趟画的是刚清成全 0 的 RT_A —— 混合状态一律 `Src`，
            // 命令的混合模式在合成趟生效（见 `replay` 里 `into_fresh_rt` 那段）。
            true,
            // ⚠ 离屏 RT 是**单采样**的 —— 传 `self.samples` 会是「4× PSO 画进
            // 1× RTV」（调试层 614）。模糊趟那边一直是硬编码 1，内容趟这条
            // 路由由 command_validation harness 抓到（2026-10-24）。
            1,
            &mut report,
        )?;
        targets.scratch = scratch;
        // 离屏期间不该有「未实现的滤镜」—— 内容里那条命令的滤镜正是在做这件事。
        report.unimplemented_filters = report.unimplemented_filters.saturating_sub(1);
        report.offscreen_passes += 1;
        // 注：`refraction_passes` 在下面那一段才加，而不在这里 ——
        // 容易被当成作用的是「运行了离屏」，而它真正的含义是「退化成
        // 普通合成时**不算**」。放错位置会让「拿不到副本」也报成生效。

        // ---- 2. 模糊：逐趟乒乓 ----
        let mut src_is_a = true;
        for step in &pass.blurs {
            let ps = if step.horizontal {
                shader::PS_BLUR_H
            } else {
                shader::PS_BLUR_V
            };
            let (from, to) = if src_is_a { (&a, &b) } else { (&b, &a) };
            let to_rtv = to.rtv(&targets.offscreen_rtv_heap)?;
            // ⚠⚠ **先绑目标，再发屏障**。反过来（先发「读源 → PSR」的屏障）时，
            // 读源此刻仍绑在 RTV 上 —— D3D12 禁止这种转换，整张图会变成一个死值。
            unsafe {
                list.OMSetRenderTargets(1, Some(&to_rtv), false, None);
            }
            rt_barrier(
                list,
                &from.resource,
                from.state.get(),
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
            from.state.set(D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE);
            rt_barrier(
                list,
                &to.resource,
                to.state.get(),
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
            to.state.set(D3D12_RESOURCE_STATE_RENDER_TARGET);

            self.draw_pass_quad(
                targets,
                device,
                list,
                &offscreen::blur_quad(step, pass.rt_size_as_f32(), pass.rt_size_as_f32()),
                ps,
                &from.srv(&self.srv_heap)?,
                None,
                to_rtv,
                rt_size,
                BlendMode::Src,
                RECT {
                    left: 0,
                    top: 0,
                    right: rt_size.0 as i32,
                    bottom: rt_size.1 as i32,
                },
                // ⚠ 离屏 RT 是单采样的。
                1,
            )?;
            src_is_a = !src_is_a;
        }

        // ---- 3. 合成回目标 ----
        //
        // ⚠⚠ 折射 / 背景模糊必须在**这一步之前**把目标拷一份。
        //
        // 两者都读"元素背后已经画在屏幕上的画面"，所以拷贝的时机必须是
        // **元素自己的内容被合上去之前** —— 那正是此刻。晚一步（元素已经盖住了
        // 背景）拷到的就是元素自己，症状是"玻璃边缘把元素自己的轮廓糊开"；
        // 早一步（元素内容还没进 RT 时）则少拷了 RT 里的内容，同样错。
        let needs_dst =
            pass.refraction.is_some() || pass.lens.is_some() || pass.backdrop_blur.is_some();
        let dst_copy = match needs_dst {
            true => {
                // 建不出副本 ⇒ 这一趟画不了。计进"未实现"并**降级成普通合成**，
                // 而不是让整帧失败：症状会变成"折射/磨砂没反应"，可与其它未接线的
                // 能力区分开，而不会把整个应用打死。
                // 只需要**这一趟的包围盒**（含外扩）那一块：整幅解析 1.12M 像素
                // 要 ~1.2 ms，而磨砂样板只有 160×120。区域夹取与越界保护在
                // `refresh_dst_copy` 里做。
                //
                // ⚠ 四周再放 4px：合成趟的双线性采样会在边界外读半像素，
                // 只按包围盒解析时那半像素是**上一次**的内容（实测差 ≤7/255，
                // 落在边界一圈上）。
                let pad = 4.0;
                let region = Some((
                    (pass.bounds.x - pad).max(0.0).floor() as u32,
                    (pass.bounds.y - pad).max(0.0).floor() as u32,
                    (pass.bounds.width + pad * 2.0).max(0.0).ceil() as u32,
                    (pass.bounds.height + pad * 2.0).max(0.0).ceil() as u32,
                ));
                match self.refresh_dst_copy(
                    targets,
                    device,
                    list,
                    cmd_env.rt,
                    cmd_env.size,
                    true,
                    region,
                ) {
                    Some(slot) => self.srv_heap.gpu(slot).ok(),
                    None => {
                        report.unimplemented_filters += 1;
                        None
                    }
                }
            }
            false => None,
        };
        let final_rt = if src_is_a { &a } else { &b };
        // 合成输出的是**预乘**颜色（`Vertex::quad` 已把顶点色预乘，而
        // `PSComposite` 的透传分支直接返回采样值）⇒ 混合状态是预乘空间的
        // `SrcOver`，不是非预乘的 `SRC_ALPHA / INV_SRC_ALPHA`。
        //
        // scissor 用**目标**的脏区：局部脏区下把滤镜结果合成到脏区之外，症状是
        // 「只改了脏区，脏区外却变了」。
        self.bind_state(list, cmd_env, cmd_env.size, cmd_env.damage)?;
        // ⚠⚠ **必须先把末趟结果转成 `PIXEL_SHADER_RESOURCE`**。
        //
        // 模糊循环的收尾是「把写目标转回 `RENDER_TARGET`」，所以合成的采样源此刻正停在
        // `RENDER_TARGET`。直接采样一份仍是渲染目标的纹理是未定义行为，而且**调试层不
        // 一定报得出来** —— 实测到的后果是画面变成一个死值、随后
        // `GetDeviceRemovedReason` 返回 `DXGI_ERROR_DEVICE_HUNG`，错在**帧尾**而不在这里。
        //
        // 顺序上必须**先** `bind_state`（把目标绑成 RTV）**再**发这道屏障：资源要先从
        // `OMSetRenderTargets` 上解绑，才能转出 `RENDER_TARGET`（见 [`rt_barrier`]）。
        rt_barrier(
            list,
            &final_rt.resource,
            final_rt.state.get(),
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        );
        final_rt
            .state
            .set(D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE);
        // 折射 / 背景模糊与普通合成**互斥**：一次只画一趟。三者都在同一块屏幕
        // 区域上"贴回"，同时画会互相覆盖 —— 症状是折射区域变成了模糊结果。
        let screen = (cmd_env.size.0 as f32, cmd_env.size.1 as f32);
        // 元素在**屏幕**上的内容矩形。
        //
        // ⚠ 传的是它、而不是 `pass.bounds`：合成四边形必须与它的
        // `local` / `half_size` 覆盖**同一块区域**，否则四边形越出元素
        // 的那一圈里 `local` 仍落在 `[0, ew]` 内、SDF 判成「内部」，
        // 于是整幅画面都被当成玻璃 / 磨砂。那是个**看起来完全正常**的错
        // （折射本来就该铺满元素），所以只有这里能挡。
        let content = zephyra_core::paint::command_content_rect(cmd).unwrap_or(pass.bounds);
        let corner = zephyra_core::paint::command_corner_radius(cmd);
        // 命令自己的混合模式在**合成趟**生效 —— 滤镜链的结果与已画内容按它合成。
        //
        // ⚠ 只传**系数可表达**的 15 个（`Clear..Screen`）：PS 变体那 14 个的
        // 混合状态是 `Src`（PS 自己算最终色），而合成趟的 PS 是 `PSComposite`
        // —— 把变体模式传给 `pso_for` 会拿到「PSComposite + Src」的组合，
        // 等于把目标整个换成 RT 内容。那条路要新的着色器入口，见
        // `unimplemented_blends` 的已知缺口记账（D3D11 侧同一处修复，
        // 2026-10-25 由 `stress_many_filtered_commands_stay_clean` 揭出）。
        let composite_blend = match command_blend_mode(cmd) {
            Some(m) if shader::ps_for_blend_mode(m).is_none() => {
                mode_by_index(m).unwrap_or(BlendMode::SrcOver)
            }
            _ => BlendMode::SrcOver,
        };
        let scissor = damage_scissor(cmd_env.size, cmd_env.damage);

        // ---- 3a. 装饰的外投影（`hollow_shadow`）先画 ----
        //
        // ⚠⚠ 它和折射 / 透镜 / 磨砂**不是互斥关系**，而是**两层**：外投影在
        // 玻璃**背后**（内侧靠 `PSHollowShadow` 挖掉），玻璃再盖上去。曾经这里把
        // 它写成了 `if hollow_shadow { ... } else { 折射/透镜/模糊 }` 的二选一，
        // 后果是所有**带装饰**的玻璃（开关把手、材质页玻璃卡片）把滤镜整趟丢掉：
        // 画面上只剩高光与内阴影，折射一点都没有 —— 而"没有折射的玻璃"看起来
        // 完全正常，所以只能靠材质页「开/关装饰」的对照才看得出来。
        if pass.hollow_shadow {
            let hquad = offscreen::hollow_shadow_composite_quad(
                pass.bounds,
                pass.element_rect,
                screen,
                pass.corner_radius,
                pass.corner_shape_k,
                pass.tint,
            );
            self.draw_pass_quad(
                targets,
                device,
                list,
                &hquad,
                shader::PS_HOLLOW_SHADOW,
                &final_rt.srv(&self.srv_heap)?,
                None,
                cmd_env.rtv,
                cmd_env.size,
                composite_blend,
                scissor,
                self.samples,
            )?;
        }

        // ---- 3b. 玻璃本体：折射 / 透镜 / 磨砂 / 普通合成 ----
        //
        // 三者互斥（都"替换"元素背后那块区域，一次只能选一个）；与上面的
        // 外投影则**叠加**。没有滤镜效果时退回普通合成（把 RT 内容贴回来）。
        let effect = match (pass.refraction, pass.lens, pass.backdrop_blur, dst_copy) {
            (Some(r), _, _, Some(srv)) => Some((
                offscreen::refraction_composite_quad(
                    content,
                    screen,
                    corner,
                    pass.corner_shape_k,
                    r,
                ),
                shader::PS_REFRACTION,
                srv,
                1u8,
            )),
            (_, Some(l), _, Some(srv)) => Some((
                offscreen::lens_composite_quad(content, screen, corner, pass.corner_shape_k, l),
                shader::PS_LENS,
                srv,
                2,
            )),
            (_, _, Some(bb), Some(srv)) => Some((
                offscreen::backdrop_blur_composite_quad(
                    content,
                    screen,
                    corner,
                    pass.corner_shape_k,
                    bb.sigma,
                ),
                shader::PS_BACKDROP_BLUR,
                srv,
                3,
            )),
            _ => None,
        };
        let (quad, ps, dst_srv, dst_effect) = match effect {
            Some((quad, ps, srv, kind)) => (quad, ps, Some(srv), kind),
            None => (
                offscreen::composite_quad(pass.bounds, screen, pass.rt_size_as_f32(), pass.tint),
                shader::PS_COMPOSITE,
                None,
                0,
            ),
        };
        let drew_dst_effect = dst_srv.is_some();
        // 外投影那一趟已经把 RT 内容贴回来了（`PSHollowShadow` 的 `t * keep`
        // 透传分支），没有滤镜时**不能**再叠一次普通合成 —— 那会把同一份内容
        // 半透明地画两遍（带 alpha 的元素会明显变深）。
        if pass.hollow_shadow && !drew_dst_effect {
            // 已经画完，跳过下面的合成趟与计数。
        } else {
            self.draw_pass_quad(
                targets,
                device,
                list,
                &quad,
                ps,
                &final_rt.srv(&self.srv_heap)?,
                dst_srv.as_ref(),
                cmd_env.rtv,
                cmd_env.size,
                composite_blend,
                scissor,
                self.samples,
            )?;
        }
        // 只在实际走了折射 / 背景模糊那一支时计。
        //
        // `dst_srv.is_some()` 等价于“拿到了目标副本且 PS 是 PSRefraction /
        // PSBackdropBlur”：两者只在同一个 match 分支里同时为真。而
        // `refresh_dst_copy` 失败时这里是 `None` —— 退化成普通合成，
        // 那时屏幕上有玻璃但没有弯曲/模糊，**不能算已经生效**。
        if drew_dst_effect {
            match dst_effect {
                1 => report.refraction_passes += 1,
                2 => report.lens_passes += 1,
                _ => report.backdrop_blur_passes += 1,
            }
        }

        // ---- 装饰 overlay：画在内容**之上** ----
        //
        // 层序契约（`Decorations` 文档）：外投影（上面的合成趟已带）→ 内容 →
        // **高光** → **内阴影**。两趟都是纯 SDF 计算、零纹理采样，几何直接铺
        // 屏幕上 —— 顺序反了的话内阴影会把高光压在下面（画面只是"高光没了"）。
        //
        // 高光按 kind 切混合：Ambient 原版是 SrcOver（半黑半白，Plus 会把黑半
        // 加成无操作 ⇒ "内侧压暗"失效）；Default / Plain 是 Plus（加色）。
        // 源 SRV 仍传 `final_rt`（状态已在 SHADER_RESOURCE）—— 着色器不读它，
        // 只是 `draw_pass_quad` 的签名要求一个源。
        if let Some(hl) = &pass.highlight {
            let blend = if hl.kind > 1.5 {
                BlendMode::SrcOver
            } else {
                BlendMode::Plus
            };
            self.draw_pass_quad(
                targets,
                device,
                list,
                &offscreen::highlight_overlay_quad(pass.element_rect, screen, *hl),
                shader::PS_HIGHLIGHT,
                &final_rt.srv(&self.srv_heap)?,
                None,
                cmd_env.rtv,
                cmd_env.size,
                blend,
                damage_scissor(cmd_env.size, cmd_env.damage),
                self.samples,
            )?;
        }
        if let Some(sh) = &pass.inner_shadow {
            self.draw_pass_quad(
                targets,
                device,
                list,
                &offscreen::inner_shadow_overlay_quad(pass.element_rect, screen, *sh),
                shader::PS_INNER_SHADOW,
                &final_rt.srv(&self.srv_heap)?,
                None,
                cmd_env.rtv,
                cmd_env.size,
                BlendMode::SrcOver,
                damage_scissor(cmd_env.size, cmd_env.damage),
                self.samples,
            )?;
        }
        Ok(report)
    }

    /// 画一个 pass 四边形（模糊趟 / 合成）。
    ///
    /// # 为什么不复用普通段那套回放
    ///
    /// 模糊 / 合成四边形是**手工构造**的 4 个顶点（`offscreen::pass_quad`），走
    /// 普通段就要先把它们塞进一份临时 `DisplayList` 再展开一遍 —— 而 `expand` 只认
    /// 四种命令变体，没有「裸四边形」这一种。
    ///
    /// # `samples` 为什么是参数而不是读 `self.samples`
    ///
    /// 模糊趟写的是**单采样**的离屏 RT，而合成趟写的是可能是 MSAA 的目标。两者必须
    /// 用不同的 PSO（`SampleDesc` 是 PSO 的一部分），所以由调用方明确传进来 ——
    /// 从 `self.samples` 推断就得靠「这个 RT 是不是离屏的」这类判断，而那个判断一旦
    /// 错，症状是「render target format does not match」而画面全错。
    #[allow(clippy::too_many_arguments)]
    fn draw_pass_quad(
        &mut self,
        targets: &mut WindowTargets,
        device: &ID3D12Device,
        list: &ID3D12GraphicsCommandList,
        quad: &[Vertex; 4],
        ps: usize,
        src_srv: &D3D12_GPU_DESCRIPTOR_HANDLE,
        // 绑到 `t1`（`g_dst`）的**目标副本**。`None` = 绑 `src_srv`（现状）。
        //
        // 只有折射需要真的给一张不同的：`PSRefraction` 读 `g_dst` 取元素**背后**
        // 的画面，而 `src_srv` 是离屏 RT（元素自己）。混淆这两者的症状是
        // "玻璃边缘把元素自己的轮廓糊开了" —— 那看起来像接线错误而不是参数问题。
        dst_srv: Option<&D3D12_GPU_DESCRIPTOR_HANDLE>,
        dst_rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
        dst_size: (u32, u32),
        blend: BlendMode,
        scissor: RECT,
        samples: u32,
    ) -> Result<(), SurfaceError> {
        let bytes = (4 * std::mem::size_of::<Vertex>()) as u64;
        // 与 `upload_geometry` 同一条缓冲：主路径可能刚把暂存区用到某个位置，
        // 所以这里也要按「head + 本段」判容量（见 `StreamBuffer::reserve`）。
        if targets.vertex.reserve(device, bytes)? {
            targets.vertex_capacity = targets.vertex.size;
        }
        let idx = quad::expand_indices_with_paths(1, 0).0;
        let need = (idx.len() * 2) as u64;
        targets.index.reserve(device, need)?;
        targets
            .vertex
            .record_upload(list, bytemuck::cast_slice(quad))?;
        // ⚠⚠ **离屏 pass 与主路径共用同一条 StreamBuffer**（见 StreamBuffer::reserve），
        // 所以这里一上传，主路径那份「上次上传内容」的影子就**过期**了。
        //
        // 不清的后果很具体：`d3d12_draw.rs` 的
        // `the_second_frame_is_identical_to_the_first_when_a_filter_is_used` 会红 ——
        // 第二帧主路径看到几何与影子一致就跳过上传，而 GPU 缓冲里已经是离屏的四边形，
        // 于是主绘制读到别人的几何。实测差 30588 个字节。
        //
        // 这类「省掉一次上传」的优化不能只看上传点自己：谁碰了这条缓冲，谁就得让
        // 「内容没变」这个判断失效。
        targets.last_vertex_bytes.clear();
        targets.last_index_bytes.clear();
        targets
            .index
            .record_upload(list, bytemuck::cast_slice(&idx))?;

        let pso = self.pso_for(device, self.elements.descs(), ps, blend as usize, samples)?;
        unsafe {
            list.SetPipelineState(&pso);
            // `t0` 是 pass 四边形要采样的源。`t1` 默认也绑同一个：模糊 / 合成 PS 在
            // `kind == 4` 时不读它，但它仍被声明着，悬空 SRV 在某些驱动上会让后续
            // 采样返回未定义值。
            //
            // 折射是唯一的例外 —— 它要的就是目标副本，所以调用方给一张真的。
            list.SetGraphicsRootDescriptorTable(0, *src_srv);
            list.SetGraphicsRootDescriptorTable(1, *dst_srv.unwrap_or(src_srv));
            let viewport = [D3D12_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: dst_size.0 as f32,
                Height: dst_size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }];
            list.RSSetViewports(&viewport);
            list.RSSetScissorRects(&[scissor]);
            list.OMSetRenderTargets(1, Some(&dst_rtv), false, None);
            list.IASetVertexBuffers(
                0,
                Some(&[D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: targets.vertex.gpu_addr(),
                    SizeInBytes: targets.vertex_capacity as u32,
                    StrideInBytes: std::mem::size_of::<Vertex>() as u32,
                }]),
            );
            list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: targets.index.gpu_addr(),
                SizeInBytes: targets.index.size as u32,
                Format: DXGI_FORMAT_R16_UINT,
            }));
            list.IASetPrimitiveTopology(
                windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            );
            list.DrawIndexedInstanced(6, 1, 0, 0, 0);
        }
        Ok(())
    }
}

/// 上传缓冲的实现。
impl UploadBuffer {
    /// 在上传堆上建一块 `size` 字节的缓冲并**立即映射**。
    ///
    /// 上传堆保证 CPU 可写，所以映射传 `None`（整块）即可，不需要写类型参数 ——
    /// D3D12 的读写方向由**堆类型**决定，不像 D3D11 的 `Map` 还要再传一个
    /// `D3D11_MAP_WRITE_DISCARD`。
    fn new(device: &ID3D12Device, size: u64) -> Result<Self, SurfaceError> {
        let resource = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_UPLOAD),
            &buffer_desc(size),
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        // 上传堆没有 barrier；`Map` 直接写。注意 `Map` 的出参形式与 D3D11 完全不同：
        // 它返回 `Result<()>`，指针走出参。
        let mut ptr = std::ptr::null_mut();
        unsafe { resource.Map(0, None, Some(&mut ptr)) }
            .map_err(|e| SurfaceError::new("Map 上传暂存缓冲", e))?;
        Ok(Self {
            resource,
            mapped: ptr.cast::<u8>(),
            capacity: size,
        })
    }
}

/// 流上传的**源偏移对齐**（`CopyBufferRegion` 的 `SrcOffset` 要求）。
///
/// 缓冲本身只要 4 字节，但用 256 与纹理上传的规则一致，省掉"这里为什么是 4"
/// 的解释成本。
const UPLOAD_ALIGN: u64 = 256;

/// 流上传段落落在哪的**唯一定义**：起点与结束。
///
/// # 为什么必须是**这一个**函数
///
/// [`StreamBuffer::reserve`]（决定要不要扩容）与 [`StreamBuffer::record_upload`]
/// （真正记录拷贝）必须对"这一段落在哪"给出**同一个**答案。它们曾各算一遍，
/// 于是 `reserve` 少算了对齐那一项：
///
/// - `reserve` 判的是 `head + extra <= size`；
/// - `record_upload` 实际用的起点是 `align_up(head)`。
///
/// `head = 8000`、缓冲 8192、`extra = 12` 时，前者算 `8012 <= 8192` 放行，
/// 后者把起点推到 **8192** 再放 12 字节 → 越界。报错形如
/// 「本段要 12 字节，从 8192 起放不下（暂存区 8192 字节）」——
///
/// - 12 字节正是 `draw_pass_quad` 传的一枚四边形（6 个 `u16` 索引，无路径）；
/// - 8192 是索引流缓冲 `next_power_of_two` 之后的容量；
/// - 起点正好顶在末尾，是"只差那点对齐"的形状，不是"缓冲真的太小"。
///
/// 症状是**整帧被丢**（`record` 一路上抛到宿主），表现为"界面某一帧起全不更新"
/// —— 与样式没加载、与 GPU 坏掉都长得不一样，极难归因。
///
/// 所以这里把调用点收敛到**一个**函数上，并把不变式写成测试（见 `renderer::tests`）。
/// 将来再加第三种上传路径，没有"再单独算一遍"的机会。
fn upload_span(head: u64, bytes: u64) -> (u64, u64) {
    // ⚠️ **两处**都要饱和。
    //
    // `head + UPLOAD_ALIGN - 1` 自己就会溢出（早先的写法只给最后一步加了
    // `saturating_add`，保护错了操作数）。回绕的后果比 panic 更糟：`off`
    // 变成一个很小的数 → `reserve` 判 `want <= size` **放行** →
    // `record_upload` 从头写，覆盖同帧已录制的其它段。那是静默的内存损坏，
    // 而不是一条看得见的报错。
    let off = head.saturating_add(UPLOAD_ALIGN - 1) & !(UPLOAD_ALIGN - 1);
    (off, off.saturating_add(bytes))
}

/// 从 `head` 起再放 `bytes` 字节，会占到的**结束偏移**。
fn upload_end(head: u64, bytes: u64) -> u64 {
    upload_span(head, bytes).1
}

impl StreamBuffer {
    /// 建一块 `size` 字节的流缓冲。
    ///
    /// `draw_state` 是绘制时它必须处于的状态（顶点缓冲给
    /// `VERTEX_AND_CONSTANT_BUFFER`，索引缓冲给 `INDEX_BUFFER`）。
    fn new(
        device: &ID3D12Device,
        size: u64,
        draw_state: D3D12_RESOURCE_STATES,
    ) -> Result<Self, SurfaceError> {
        let resident = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_DEFAULT),
            &buffer_desc(size),
            draw_state,
        )?;
        let staging = UploadBuffer::new(device, size)?;
        Ok(Self {
            resident,
            staging,
            size,
            draw_state,
            head: std::cell::Cell::new(0),
            // `create_resource` 把**缓冲**的初始状态强制成 `COMMON`（见它自己的注释），
            // 所以刚建出来的 `resident` 并不在 `draw_state`。第一次上传必须从
            // `COMMON` 出发 —— 直接发 `draw_state → COPY_DEST` 是一次
            // 「Before 状态与实际不符」的转换，验证层报 `D3D12_MESSAGE_ID 527`。
            //
            // ⚠ 这个错误**单帧看不出来**：验证层要等第二次 `ExecuteCommandLists`
            // 才开始比对，于是「只渲一帧」的测试全绿，而真实宿主每帧都在报错。
            fresh: std::cell::Cell::new(true),
            retire: Vec::new(),
        })
    }

    /// 为「还要上传 `extra` 字节」预留空间；不足就整块重建。返回是否重建过。
    ///
    /// # 判据必须含 `head`
    ///
    /// 「本段要多少」不等于「本帧已花了多少」：主路径（[`Renderer::record`] →
    /// `upload_geometry`）与离屏 pass（`draw_pass_quad`：模糊 / 投影 / 多级链）
    /// 传的是**同一条** `StreamBuffer`。只看 `extra` 的判据会在主路径刚好把
    /// 暂存区用满时放行离屏 pass，于是 `record_upload` 在最后一个字节处越界 ——
    /// 而那个错误一路上抛，**整帧被丢**。症状是「界面某一帧起全不更新」，与
    /// 样式没加载、与 GPU 坏掉都长得不一样，极难归因。
    ///
    /// 实测过这个场景：样式表全量迁移后圆角变多，主路径顶点涨到
    /// 128KB 用满初始容量，随后三个投影样张的离屏 pass 各传 240 字节全数失败。
    ///
    /// # 为什么重建而不是回绕游标
    ///
    /// 回绕会覆盖 GPU 还没读完的段。而重建的前提是安全的：`record` 帧首
    /// `begin_frame` 已 `wait_gpu`，进入本函数时 GPU 空闲；旧块被移进
    /// [`Self::retire`] 活到帧尾，本次已记录的拷贝不受影响。
    fn reserve(&mut self, device: &ID3D12Device, extra: u64) -> Result<bool, SurfaceError> {
        // ⚠️ `extra` 之外还必须给 **256 字节对齐**留出余量。
        //
        // [`Self::record_upload`] 把 `head` 向上取整到 256 的倍数才开始放
        // （`CopyBufferRegion` 的 `SrcOffset` 要求对齐）。所以「本段要 `extra`
        // 字节」**不等于**「从对齐后的偏移起要 `extra` 字节」——
        // 最坏情况对齐会把起点再往后推 255 字节。
        //
        // 不把这份余量算进判据的后果很具体：主路径刚好把暂存区用满时
        // （`head + extra <= size` 通过了），`record_upload` 却因对齐越界，
        // 抛错**整帧被丢**。症状是"界面某一帧起全不更新"。
        // 这个错此前报的是「本段要 12 字节，从 8192 起放不下」——
        // 12 字节的东西放在 8KB 边界上，正是"只差那点对齐"的形状。
        let want = upload_end(self.head.get(), extra.max(1));
        if want <= self.size {
            return Ok(false);
        }
        let grown = want.next_power_of_two();
        let mut old = std::mem::replace(self, StreamBuffer::new(device, grown, self.draw_state)?);
        // 旧块连它自己退役的旧块一起继续留着：同一次 record 里可能多次扩容。
        let mut carried = std::mem::take(&mut old.retire);
        carried.push(old);
        self.retire = carried;
        Ok(true)
    }

    /// 清空退役列表。**只在 `record` 帧首调用**（那时 GPU 已空闲）。
    fn clear_retired(&mut self) {
        self.retire = Vec::new();
    }

    /// 记录「把新内容搬进常驻缓冲」所需的屏障、拷贝与状态还原。
    ///
    /// # 每次上传都要**独占**一段暂存区
    ///
    /// 见 [`StreamBuffer::head`]：拷贝只是被记录下来，CPU 的写入当下就完成了，所以
    /// 同一条命令列表里的多次上传必须落在不同偏移上。
    ///
    /// # 游标在帧内**不**回绕
    ///
    /// 回绕会覆盖 GPU 还没读的段，而那正是最难查的一类损坏。宁可报错：
    /// `Renderer::record` 帧首会把游标归零，而一帧的总量本来就该远小于暂存区。
    ///
    /// # `bytes` 必须不长于 `size`
    ///
    /// `CopyBufferRegion` 的尺寸由**两个缓冲的较小者**决定，所以给一个超长的切片不会
    /// 报错 —— 它只会**安静地只拷一部分**。所以这里自己夹一次并夹成整块（宁可少拷，
    /// 不要半拷出撕裂的帧）。
    fn record_upload(
        &self,
        list: &ID3D12GraphicsCommandList,
        bytes: &[u8],
    ) -> Result<(), SurfaceError> {
        if bytes.is_empty() {
            return Ok(());
        }
        // `CopyBufferRegion` 的 `SrcOffset` 要求对齐（缓冲只要 4 字节，但用 256 与
        // 纹理的规则一致，省掉「这里为什么是 4」的解释成本）。
        let n = (bytes.len() as u64).min(self.size);
        // 起点与结束都从**同一个** helper 来：早先这里自己算 `off`，而 `end` 走
        // 另一条算式，两者一旦漂移，`record_upload` 的检查就不再等于 `reserve`
        // 的判据 —— 那正是本次要修的 bug 的成因。
        let (off, end) = upload_span(self.head.get(), n);
        if end > self.size {
            return Err(SurfaceError::invalid(format!(
                "上传暂存区不够：本段要 {n} 字节，从 {off} 起放不下（暂存区 {} 字节）。\
                 帧内上传总量超过暂存区说明暂存区太小，而不是这里该偷偷回绕",
                self.size
            )));
        }
        self.head.set(end);
        // 第一次上传之前 `resident` 还在 `COMMON`（缓冲的初始状态），见 [`Self::fresh`]。
        let before = if self.fresh.replace(false) {
            D3D12_RESOURCE_STATE_COMMON
        } else {
            self.draw_state
        };
        // CPU 写入常驻映射的上传内存：不需要屏障（上传堆资源恒为 GENERIC_READ，
        // 且 CPU 写映射内存不受资源状态约束）。
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.staging.mapped.add(off as usize),
                n as usize,
            )
        };

        unsafe {
            // ⚠⚠ **上传堆资源上不发任何屏障** —— 这是每帧上传路径上最大的浪费。
            //
            // D3D12 的上传堆资源**恒为 `GENERIC_READ`**，而它本身就是 `CopyBufferRegion`
            // 要求的拷贝源状态。所以：
            //
            // - 转换 `GENERIC_READ → COPY_SOURCE → GENERIC_READ` 是**纯浪费**：状态最终
            //   与没转换时一模一样，中间那段还得让 GPU 等一次；
            // - 而且它是**非法**的 —— 上传堆资源不允许做状态转换（同文件纹理路径
            //   `upload_atlas` / `upload_texture` 早就把这��句写进注释了：原先那条
            //   `GENERIC_READ → COPY_SOURCE` 是非法的，现在纹理上传一条都不发）。
            //
            // 原先这两条屏障每帧发 2 次（顶点 + 索引），也就是每帧 4 条屏障白白排进
            // 命令流，让 GPU 多做 4 次状态校验与屏障处理。
            gpu_barrier(list, &self.resident, before, D3D12_RESOURCE_STATE_COPY_DEST);
            list.CopyBufferRegion(&self.resident, 0, &self.staging.resource, off, n);
            gpu_barrier(
                list,
                &self.resident,
                D3D12_RESOURCE_STATE_COPY_DEST,
                self.draw_state,
            );
        }
        Ok(())
    }

    /// GPU 虚拟地址（绑给 `IASet*Buffer` 时用）。
    fn gpu_addr(&self) -> u64 {
        unsafe { self.resident.GetGPUVirtualAddress() }
    }
}

/// PSO 的适配器节点掩码。**单 GPU 就是 0。**
///
/// # 官方文档原文
///
/// `D3D12_GRAPHICS_PIPELINE_STATE_DESC::NodeMask`：
///
/// > For single GPU operation, **set this to zero**. If there are multiple GPU nodes,
/// > set bits to identify the nodes (the device's physical adapters) for which the
/// > graphics pipeline state is to apply.
///
/// `Multi-adapter systems` 页面进一步说明：
///
/// > If your application passes **0** for this bit mask, then the Direct3D 12 driver
/// > **converts that to the bit mask 1** (indicating that the object is associated with
/// > node 0).
///
/// 而 `D3D12_GRAPHICS_PIPELINE_STATE_DESC` 明确列在那一页的 **Multiple nodes** 清单里
/// —— **不**在 "Single nodes" 清单里。
///
/// # 我曾经把它改成 1，那是错的
///
/// 「单节点 API 必须恰好设一位」那条规则只适用于 `CreateCommandQueue` /
/// `CreateCommandList` / `CreateDescriptorHeap` / `CreateQueryHeap` —— **PSO 不在其列**。
///
/// 当时凭推理认定「0 意味着不属于任何节点」，改了之后
/// `CreateGraphicsPipelineState` 依然失败，**本该据此撤回这个改动却没有**。写在这里
/// 是为了留下「改完没验证好就等于没改」的记录。
///
/// # 顺带说明另两处节点掩码为什么是 1
///
/// 堆属性的 `CreationNodeMask` / `VisibleNodeMask` 不在「驱动会把 0 转成 1」那份
/// 清单里 —— 那个是 `tests/sweep.rs` 扫出来的：写 0 建不出堆。
pub const PSO_NODE_MASK: u32 = 0;

/// `size` 字节的缓冲描述。
///
/// # `Layout` 必须是 `ROW_MAJOR` —— 缓冲没有 `UNKNOWN` 这个选项
///
/// 这里曾经写着 `D3D12_TEXTURE_LAYOUT_UNKNOWN`，并配了一段注释说「提交式资源的布局
/// 必须留 UNKNOWN，写成 ROW_MAJOR 会拿到没有提示的 `E_INVALIDARG`」。
///
/// **那句话对纹理成立，对缓冲正好反了**，于是它一度成了本 crate 最大的阻塞项
/// （**已解**，诊断过程留在下面当记录）：
///
/// - `CreateCommittedResource` 对 4 块适配器**全部**返回 `E_INVALIDARG`；
/// - `CreatePlacedResource`（**另一个函数**）用同一个描述符同样失败 —— 这一条就
///   排除了"函数坏了"，指向字段；
/// - `tests/sweep.rs` 扫 81 种字段组合：凡是 `Layout = ROW_MAJOR` 的**全部通过**
///   （3 种堆类型 × 2 种节点掩码共 6 组），凡是 `UNKNOWN` 的**全部失败**。
///
/// 规则本身是：
///
/// | `Dimension` | `Layout` 必须 |
/// |---|---|
/// | `BUFFER` | `ROW_MAJOR` |
/// | 纹理，**提交式** | `UNKNOWN` |
/// | 纹理，**放置式** | `ROW_MAJOR` |
///
/// 两种写反了都是 `E_INVALIDARG`，且都不提示是哪个字段。
///
/// 官方依据（`D3D12_RESOURCE_DESC` 参考页的 Buffers 一节）：
///
/// > SampleDesc.Count must be 1 and Quality must be 0.
/// > **Layout must be D3D12_TEXTURE_LAYOUT_ROW_MAJOR**, as buffer memory layouts are
/// > understood by applications and row-major texture data is commonly marshaled
/// > through buffers.
///
/// <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_resource_desc>
pub fn buffer_desc(size: u64) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        // `Alignment` 对**提交式**资源必须是 0 —— 只有放置式才由调用方指定。
        Alignment: 0,
        Width: size,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        // 缓冲的 `Flags` 只能是 `NONE`。`D3D12_RESOURCE_FLAG_BUFFER` 是个**陷阱** ——
        // 它在 DX11 里存在，在 D3D12 里没有，传了会 `E_INVALIDARG`。
        Flags: D3D12_RESOURCE_FLAG_NONE,
    }
}

/// 建图集纹理。
///
/// # 格式必须是 `RGBA8`，不是 `R8`
///
/// [`zephyra_core::text::GlyphAtlas`] 把覆盖率写进**四个通道**（`atlas.rs` 里
/// `pixels[base..base+4]` 全赋同一个值），而 `pixels()` 返回 `ATLAS_SIZE² × 4` 字节。
///
/// 这里原先写的是 `DXGI_FORMAT_R8_UNORM` —— 那是**两处**错：
///
/// - **格式错**：着色器 `g_tex.Sample(...).a` 从 **alpha** 取覆盖率，而 R8 没有 alpha；
/// - **尺寸错**：上传缓冲按 `side * side` 分配，实际数据是 `side * side * 4` ——
///   `CopyTextureRegion` 要求拷贝区域与缓冲区匹配，多出来那 3 倍会越界。
///
/// 症状是「字完全不出现」，或者崩在上传那一步 —— 而这两者与「GPU 画不出东西」
/// 在现象上无法区分。DX11 侧用 RGBA8，**两边必须一致**。
fn create_atlas(device: &ID3D12Device, size: u32) -> Result<ID3D12Resource, SurfaceError> {
    if size == 0 {
        return Err(SurfaceError::new(
            "图集边长为零",
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "atlas size 0"),
        ));
    }
    // 格式必须是 RGBA8，不能是 R8 —— 见上面那段文档。
    create_resource(
        device,
        &heap_props(D3D12_HEAP_TYPE_DEFAULT),
        // `texture_desc` 已经是 `R8G8B8A8_UNORM`，正好对上 `GlyphAtlas` 的四个通道。
        // `FLAG_NONE`：图集只被采样，不当渲染目标。
        &crate::surface::texture_desc((size, size), D3D12_RESOURCE_FLAG_NONE),
        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
    )
}
/// 屏障的本地别名，让 `record` 里的调用短一点。
///
/// # 空转屏障必须跳过
///
/// `before == after` 的屏障在调试层是 **ERROR**（525「Before and after states
/// must be different」）。`rt_barrier` 早有这道守卫，这里原先没有 ——
/// `refresh_dst_copy` 第一帧的「副本 → `RESOLVE_DEST`」就是空转（副本建出来
/// 就在那个状态里），于是每帧第一条屏障必报 525。本仓的 command_validation
/// harness 开到调试层后当场抓到。
fn gpu_barrier(
    list: &ID3D12GraphicsCommandList,
    res: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    if before == after {
        return;
    }
    crate::surface::Gpu::barrier(list, res, before, after);
}

/// 把驱动的错误 blob 转成可读消息。
fn blob_message(
    blob: &Option<windows::Win32::Graphics::Direct3D::ID3DBlob>,
    fallback: &'static str,
) -> String {
    let Some(b) = blob else {
        return fallback.to_string();
    };
    // blob 是**不保证**以 NUL 结尾的字节串，取到第一个 0 为止。
    let ptr = unsafe { b.GetBufferPointer() }.cast::<u8>();
    let len = unsafe { b.GetBufferSize() };
    if ptr.is_null() || len == 0 {
        return fallback.to_string();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(len);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
#[path = "../tests/unit/tests/renderer_tests.rs"]
mod tests;
