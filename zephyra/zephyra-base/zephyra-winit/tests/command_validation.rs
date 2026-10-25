//! D3D11 的**传参校验**：调试层开着回放每一帧，零 ERROR 级消息才算过。
//!
//! # 为什么需要这个文件
//!
//! D3D11 的失败模式是**不报错但什么都不画**（见 `d3d11_draw.rs` 文件头整段），
//! 而调试层的消息是**排队**的 ——「没看到错误」完全推不出「没有错误」。本仓
//! D3D11 侧早有一条教训：三项自检全绿、画面全错，而驱动说的那句话一直躺在
//! 队列里没人取（`dump_debug_messages` 只在排查时手调）。
//!
//! 2026-10-24 D3D12 那边把同一件事做成了门禁（`zephyra-winit-d3d12/tests/
//! command_validation.rs`），当场抓出四个真 bug（resolve 源/目标状态、空转屏障、
//! PSO 采样数不匹配、以及 drain 的严重级比较写反）。这个文件是 D3D11 的对应物：
//! **成功路径也抽干队列**，让「驱动接受了这些调用」第一次成为有证据的结论。
//!
//! # 与 D3D12 版的差异
//!
//! - D3D11 没有命令列表，调用**立即执行** —— 不存在「录进非法命令、`Close()`
//!   才炸」那一族，所以没有 `submit` 断言；调试层消息就是第一现形处。
//! - 采样数不由调用方选：`Renderer::new` 内部按 `surface::msaa_samples(device)`
//!   建 PSO，所以离屏目标必须按**同一个值**建（与宿主同一条规则）。
//! - 反向对照换一种非法调用：D3D11 没有 resolve，用**尺寸不匹配的
//!   `CopyResource`**（规格级硬规则）。
//!
//! # 为什么本文件内部要串行跑
//!
//! `ID3D11InfoQueue` 与 D3D12 侧同型：**进程级单例**，对任何设备 QI 拿到
//! 的都是同一个队列，任何设备的 drain 把全部消息抽走（D3D12 侧 2026-10-25
//! 两设备交叉探针实证，D3D11 的 info queue 本就是全局对象）。并行跑会偷
//! 消息：轻则假红（反向对照录完非法 `CopyResource`、自己 drain 之前被别的
//! 测试帧尾 drain 偷走，那测试直接红），重则**假绿**（真 ERROR 被别人提前
//! 消费，本测试看到空队列，「零 ERROR」断言放过）。所以每个测试开场拿
//! 进程级锁 `GPU_GATE` 全程持握。
//!
//! # 怎么跑
//!
//! ```powershell
//! cargo test -p zephyra-winit --test command_validation -- --ignored --nocapture
//! ```
//!
//! 要真 GPU + 调试层（`ZEPHYRA_D3D_DEBUG=1` 由夹具自开），所以标 `#[ignore]`。
#![cfg(windows)]
// 宿主 crate 整个是 `#![cfg(windows)]`，而集成测试是独立目标 —— 不补这道门禁，
// `cargo test --workspace` 在 Linux 上编不过（实测 2026-10）。

use std::env;
use std::sync::Mutex;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use zephyra_core::core::{Color, Point, Rect};
use zephyra_core::filter::{BlendMode, Filter, FilterChain};
use zephyra_core::paint::{Command, DisplayList};
use zephyra_winit::renderer::{RenderCtx, Renderer};
use zephyra_winit::surface::{msaa_samples, Gpu, BACKBUFFER_FORMAT};
use zephyra_winit::texture::TextureTable;
use zephyra_winit::{drain_severe_messages, info_queue};

/// 离屏目标边长（内容尺寸）。
const SIZE: (u32, u32) = (256, 256);

/// 每个场景连回几帧。
///
/// D3D11 没有跨帧存活的资源状态（驱动在背后管），但**离屏 RT 对是跨帧复用**的、
/// `refresh_dst_copy` 的副本也是懒建复用的 —— 第二帧起走的是完全不同的代码路径
/// （`offscreen_pair` 命中缓存、副本已存在且停在 `PIXEL_SHADER_RESOURCE`）。
const ROUNDS: usize = 2;

// ─────────────────── 进程级串行锁 ───────────────────

/// 本测试二进制的进程级串行锁，每个测试开场拿、全程持握。
///
/// 存在理由与 D3D12 侧同型：`ID3D11InfoQueue` 是进程级单例，并行 drain 会
/// 互相偷消息 —— 假红（偷到反向对照的非法 `CopyResource` 消息）与**假绿**
/// （真 ERROR 被别人的 drain 提前消费，「零 ERROR」断言放过）两个方向都坏。
/// 详见 D3D12 版 `GPU_GATE` 上的完整注释。
static GPU_GATE: Mutex<()> = Mutex::new(());

fn gpu_gate() -> std::sync::MutexGuard<'static, ()> {
    // 测试 panic 时锁被 poison；`into_inner` 恢复它，让其余测试照跑。
    GPU_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ─────────────────────────────── 夹具 ───────────────────────────────

/// 回放一帧，然后**在成功路径上**抽干调试层。
///
/// D3D11 立即执行：`render` 返回时这一帧已经跑完，消息都在队列里躺着。
fn render_one(
    gpu: &Gpu,
    renderer: &mut Renderer,
    rtv: &ID3D11RenderTargetView,
    textures: &TextureTable,
    list: &DisplayList,
) -> zephyra_fx::RenderReport {
    let report = renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv,
        list,
        size: SIZE,
        clear: Color::rgb(0.0, 0.0, 0.0),
        textures,
        damage: zephyra_core::frame::Damage::Full,
    });
    let severe = drain_severe_messages(info_queue(&gpu.device).as_ref());
    assert!(
        severe.is_empty(),
        "调试层报了 {severe:?} 条 ERROR 级消息 —— 驱动拒绝了这些调用。\n\
         消息文本已打在 stderr 上（[d3d] 前缀）。"
    );
    report
}

/// 一套「开调试层的设备 + 渲染器 + 按设备采样数建的离屏目标」。
struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    rtv: ID3D11RenderTargetView,
    /// 留着不给 Drop：RTV 持有纹理引用，但测试里要的是句柄稳定。
    _tex: ID3D11Texture2D,
    textures: TextureTable,
}

impl Rig {
    fn new() -> Self {
        // 调试层靠环境变量开（`Gpu::new` 读它）。
        env::set_var("ZEPHYRA_D3D_DEBUG", "1");
        let gpu = Gpu::new().expect("建不出 D3D11 设备");
        assert!(
            info_queue(&gpu.device).is_some(),
            "调试层没挂上（拿不到 ID3D11InfoQueue）—— 本文件所有『零 ERROR』断言恒真。\n\
             多半是没装 Windows 的 \"Graphics Tools\"（D3D11 调试层是可选功能）。"
        );
        let renderer = Renderer::new(&gpu.device).expect("建不出渲染器");
        // 采样数必须与渲染器一致：`Renderer::new` 用 `msaa_samples(device)` 建
        // PSO 的 `SampleDesc`，目标另用一个数就是「PSO 与 RTV 采样数不匹配」。
        let (rtv, tex) = offscreen_target(&gpu, msaa_samples(&gpu.device));
        Self {
            gpu,
            renderer,
            rtv,
            _tex: tex,
            textures: TextureTable::new(),
        }
    }

    fn frame(&mut self, list: &DisplayList) -> zephyra_fx::RenderReport {
        render_one(
            &self.gpu,
            &mut self.renderer,
            &self.rtv,
            &self.textures,
            list,
        )
    }
}

/// 读回 Rig 目标正中心那个像素的 BGRA 四通道。
///
/// STAGING 拷贝 + Map，与 `d3d11_draw.rs` 的 `read_back` 同一套办法（那里读
/// 整张，这里只要中心一个点）。行尾填充照例剔掉。
///
/// # 为什么多了一步 Resolve
///
/// Rig 的目标按设备采样数建（可能是 4× MSAA），而 D3D11 **不允许**把多重
/// 采样纹理 `CopyResource` 到单采样 STAGING（规格级硬规则，调用失败且不报错，
/// 症状就是读到全 0）。所以先 `ResolveSubresource` 到一张同格式单采样纹理。
fn read_back_center(rig: &Rig) -> [u8; 4] {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    let samples = msaa_samples(&rig.gpu.device);
    let single = D3D11_TEXTURE2D_DESC {
        Width: SIZE.0,
        Height: SIZE.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: 0,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut resolved = None;
    unsafe {
        rig.gpu
            .device
            .CreateTexture2D(&single, None, Some(&mut resolved as *mut _))
    }
    .expect("应当能建单采样中间纹理");
    let resolved = resolved.expect("CreateTexture2D 成功时必给纹理");
    let stg_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..single
    };
    let mut stg = None;
    unsafe {
        rig.gpu
            .device
            .CreateTexture2D(&stg_desc, None, Some(&mut stg as *mut _))
    }
    .expect("应当能建 STAGING 纹理");
    let stg = stg.expect("CreateTexture2D 成功时必给纹理");
    unsafe {
        if samples > 1 {
            rig.gpu.context.ResolveSubresource(
                &resolved,
                0,
                &rig._tex,
                0,
                DXGI_FORMAT_B8G8R8A8_UNORM,
            );
            rig.gpu.context.CopyResource(&stg, &resolved);
        } else {
            rig.gpu.context.CopyResource(&stg, &rig._tex);
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        rig.gpu
            .context
            .Map(&stg, 0, D3D11_MAP_READ, 0, Some(&mut mapped as *mut _))
            .expect("STAGING 纹理应当能 Map(READ)");
        let pitch = mapped.RowPitch as usize;
        let row = (SIZE.1 / 2) as usize;
        let col = (SIZE.0 / 2) as usize;
        let base = mapped.pData as *const u8;
        let mut px = [0u8; 4];
        std::ptr::copy_nonoverlapping(base.add(row * pitch + col * 4), px.as_mut_ptr(), 4);
        rig.gpu.context.Unmap(&stg, 0);
        px
    }
}

/// 建一张按 `samples` 采样的离屏目标（自带 RTV）。
///
/// 采样数必须与渲染器一致：`Renderer::new` 用 `msaa_samples(device)` 建 PSO 的
/// `SampleDesc`，目标另用一个数就是「PSO 与 RTV 采样数不匹配」—— D3D11 调试层
/// 对这种不匹配报不报取决于驱动，报了这条 harness 第一帧就红。
fn offscreen_target(gpu: &Gpu, samples: u32) -> (ID3D11RenderTargetView, ID3D11Texture2D) {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: SIZE.0,
        Height: SIZE.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: BACKBUFFER_FORMAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: samples.max(1),
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        // ⚠ 多重采样纹理**不能**同时绑 SRV（D3D11 不允许采样 MSAA 纹理）——
        // 所以只给 RENDER_TARGET。这正是 `refresh_dst_copy` 必须先解析再拷的
        // 原因（见 renderer.rs 里那段）。
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe {
        gpu.device
            .CreateTexture2D(&desc, None, Some(&mut tex as *mut _))
    }
    .expect("应当能建离屏纹理");
    let tex = tex.expect("CreateTexture2D 成功时必给纹理");
    let mut rtv = None;
    unsafe {
        gpu.device
            .CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _))
    }
    .expect("应当能给离屏纹理建 RTV");
    (rtv.expect("CreateRenderTargetView 成功时必给视图"), tex)
}

// ─────────────────────────────── 场景构造 ───────────────────────────────

/// 一条 Quad 命令的 DisplayList。
fn quad(rect: Rect, filter: FilterChain, blend: BlendMode) -> DisplayList {
    let mut l = DisplayList::new();
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect,
            radius: zephyra_core::core::CornerRadius { radius: 12.0 },
            fill: Color::rgb(0.9, 0.9, 0.95),
            stroke: None,
            filter,
            blend,
        },
    );
    l
}

/// 画廊玻璃页那块玻璃的参数档（默认 48/40，见 10-22 日志的定档记录）。
fn refraction_chain() -> FilterChain {
    FilterChain::single(Filter::Refraction {
        strength: 48.0,
        bezel: 40.0,
        dispersion: 0.03,
        ior: 1.5,
    })
}

/// 画廊玻璃页的一整帧：背景 + 模糊 + 可分离混合 + 折射，四条命令互相叠。
fn glass_page() -> DisplayList {
    let mut l = DisplayList::new();
    let mut push = |order: u32, rect: Rect, filter: FilterChain, blend: BlendMode| {
        l.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Background,
                z: 0,
                order,
            },
            Command::Quad {
                rect,
                radius: zephyra_core::core::CornerRadius { radius: 12.0 },
                fill: Color::rgb(0.9, 0.9, 0.95),
                stroke: None,
                filter,
                blend,
            },
        );
    };
    push(
        0,
        Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32),
        FilterChain::none(),
        BlendMode::SrcOver,
    );
    push(
        1,
        Rect::new(8.0, 8.0, 100.0, 100.0),
        FilterChain::single(Filter::Blur {
            sigma_x: 5.0,
            sigma_y: 5.0,
        }),
        BlendMode::SrcOver,
    );
    push(
        2,
        Rect::new(148.0, 8.0, 100.0, 100.0),
        FilterChain::none(),
        BlendMode::Multiply,
    );
    push(
        3,
        Rect::new(60.0, 130.0, 136.0, 118.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    l
}

// ─────────────────────────────── 反向对照 ───────────────────────────────

/// 故意做一次**非法**拷贝，harness 必须抓到。
///
/// 尺寸不匹配的 `CopyResource` 是规格级硬规则（不靠驱动心情）。D3D11 没有
/// 命令列表，非法调用在**调用时**就进调试层队列 —— 所以这里只要一条判据：
/// 队列里出现 ERROR。抓不到 ⇒ 本文件其余「零 ERROR」断言恒真。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn the_gate_detects_an_illegal_copy_resource() {
    let _gate = gpu_gate();
    env::set_var("ZEPHYRA_D3D_DEBUG", "1");
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let queue = info_queue(&gpu.device);
    assert!(
        queue.is_some(),
        "调试层没挂上 —— 反向对照恒真，本文件失去意义"
    );
    let small = offscreen_target(&gpu, 1);
    // 大目标：同一格式、尺寸翻倍 —— 与 `small` 尺寸不匹配。
    let big_tex = {
        let base = texture_desc_like(&small.1);
        let desc = D3D11_TEXTURE2D_DESC {
            Width: SIZE.0 * 2,
            Height: SIZE.1 * 2,
            ..base
        };
        let mut tex = None;
        unsafe {
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut tex as *mut _))
        }
        .expect("建得出大目标");
        tex.expect("CreateTexture2D 成功时必给纹理")
    };
    unsafe {
        gpu.context.CopyResource(&small.1, &big_tex);
    }
    let severe = drain_severe_messages(queue.as_ref());
    assert!(
        !severe.is_empty(),
        "尺寸不匹配的 CopyResource（256² ← 512²）没进调试层队列 —— 本 harness \
         抓不到非法调用，本文件其余『零 ERROR』断言恒真"
    );
}

/// 取一张纹理的描述，只改尺寸（反向对照里要用）。
fn texture_desc_like(tex: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut d = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut d) };
    d
}

// ─────────────────────────────── 各路径门禁 ───────────────────────────────

/// 无滤镜的普通帧：也必须是干净的调用。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn plain_quads_produce_no_debug_errors() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(48.0, 48.0, 160.0, 160.0),
        FilterChain::none(),
        BlendMode::SrcOver,
    );
    for _ in 0..ROUNDS {
        let report = rig.frame(&list);
        assert!(report.quads > 0, "一条 Quad 都没画出来 —— 场景构造退化了");
        assert_eq!(report.offscreen_passes, 0, "恒等链开了离屏 pass");
    }
}

/// 模糊 / 投影的离屏链（内容 → RT_A → 模糊乒乓 → 合成回目标）。
///
/// 第二帧是关键：RT 对命中缓存、`refresh_dst_copy` 的副本已存在且停在
/// `PIXEL_SHADER_RESOURCE` —— 与第一帧（全新建、初始 `COPY_DEST`）走的是
/// 不同分支。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn blur_and_shadow_offscreen_chain_produce_no_debug_errors() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    for chain in [
        FilterChain::single(Filter::Blur {
            sigma_x: 5.0,
            sigma_y: 5.0,
        }),
        FilterChain::single(Filter::DropShadow {
            color: Color::rgb(0.0, 0.0, 0.0),
            offset: Point::new(8.0, 8.0),
            sigma: 4.0,
        }),
    ] {
        let list = quad(
            Rect::new(56.0, 56.0, 144.0, 144.0),
            chain,
            BlendMode::SrcOver,
        );
        for _ in 0..ROUNDS {
            let report = rig.frame(&list);
            assert!(report.offscreen_passes >= 1, "滤镜链没开出离屏 pass");
            assert_eq!(report.unimplemented_filters, 0, "滤镜未执行");
        }
    }
}

/// 可分离混合模式：PS 变体读**目标副本** ⇒ `refresh_dst_copy`。
///
/// 本设备支持 MSAA 时目标就是多重采样的，而 D3D11 **不允许采样 MSAA 纹理** ——
/// 所以副本那条路必须先解析再拷，这是 D3D11 侧最容易静默出错的一段。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn separable_blend_dst_copy_produce_no_debug_errors() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(48.0, 48.0, 160.0, 160.0),
        FilterChain::none(),
        // Multiply 是可分离模式 ⇒ 走 PS 变体 ⇒ 要读 dst。
        BlendMode::Multiply,
    );
    for _ in 0..ROUNDS {
        let report = rig.frame(&list);
        // 执行了就要减账（fx 按命令计、后端按段减，2026-10-24 两侧同修）。
        assert_eq!(
            report.unimplemented_blends, 0,
            "可分离混合未执行 —— PS 变体那条路没接上"
        );
    }
}

/// 折射：读**目标副本**当 `g_dst`。与模糊的根本区别是它采样的是元素背后的画面。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn refraction_produce_no_debug_errors() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(60.0, 60.0, 136.0, 136.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    for _ in 0..ROUNDS {
        let report = rig.frame(&list);
        assert!(
            report.refraction_passes >= 1,
            "折射没执行 —— 玻璃页会与『没设滤镜』无异"
        );
        assert_eq!(report.unimplemented_filters, 0, "滤镜未执行");
    }
}

/// 画廊玻璃页的一整帧：四条路径**同帧共存**。
///
/// D3D11 的 `refresh_dst_copy` 是**逐段**刷新的（每一段 PS 变体都重新拷一次
/// 目标），同帧多路径时它被反复调用 —— 副本资源的状态在帧内多次推进，
/// 正是状态谎报（`StateBefore` 与真实状态不符）的温床。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn a_full_glass_page_produce_no_debug_errors() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = glass_page();
    for _ in 0..3 {
        let report = rig.frame(&list);
        assert!(report.refraction_passes >= 1, "折射没执行");
        assert!(
            report.offscreen_passes >= 2,
            "离屏 pass 数 {sec} —— 模糊与折射两条链都应开出离屏",
            sec = report.offscreen_passes
        );
        assert_eq!(report.unimplemented_filters, 0, "滤镜未执行");
        assert_eq!(report.unimplemented_blends, 0, "可分离混合未执行");
    }
}

// ─────────────────────────────── 压力场景 ───────────────────────────────
//
// 上面的场景都是「一帧一两条滤镜命令」—— 那是稳态，也是最少分支的路径。
// 压力场景把同一批不变量推到量级上：帧内**上百条**带滤镜的命令、**十四趟**
// 的深链、以及逐帧翻页的 RT 重建。三类都零 ERROR 且计数为 0，才敢说
// 「这条链没问题」而不是「这条链在简单输入上没问题」。
//
// # 混合模式分两级断言（2026-10-25 起）
//
// - **系数可表达 15 个**（`Clear..Screen`）：合成趟已按命令的混合状态画，
//   所以压力页混入它们也必须是 `unimplemented_blends == 0`。
// - **PS 变体 14 个**（`Overlay..Luminosity`）：合成趟的源是 RT 内容而不是
//   图元几何，变体的 `ShadeSource` 不采样 `g_tex`，复用会把染色顶点色当源色
//   —— 这是**已知缺口**，`stress_filtered_ps_variant_blends_stay_counted`
//   钉住「计数器如实报数」，等新着色器入口落地时那条会红、提醒改断言。

/// 12 种链 × 12 个矩形轮转，一帧 144 条带滤镜的命令。
///
/// RT 对只有一个槽、按尺寸重建 —— 12 种不同 bounds 意味着帧内 12 次
/// **RT 重建 + 乒乓 + dst copy**，「稳态一帧一条滤镜」上永远走不到的分支
/// 全踩一遍。混合模式混入系数可表达的几个（Screen / Plus / Modulate / Xor），
/// 合成趟逐命令换混合状态，同样处于压力下。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_many_filtered_commands_stay_clean() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();

    let chains = [
        FilterChain::single(Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        }),
        FilterChain::single(Filter::Blur {
            sigma_x: 5.0,
            sigma_y: 3.0, // 各向异性：H/V 两趟 σ 不同
        }),
        FilterChain::single(Filter::DropShadow {
            color: Color::rgb(0.0, 0.0, 0.0),
            offset: Point::new(6.0, 8.0),
            sigma: 4.0,
        }),
        FilterChain::single(Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
            offset: Point::new(-10.0, -10.0), // 向左上投影： outset 四边不对称
            sigma: 2.0,
        }),
        refraction_chain(),
        FilterChain::single(Filter::Blur {
            sigma_x: 0.0,
            sigma_y: 0.0,
        }), // 恒等：不开离屏
        FilterChain::single(Filter::Blur {
            sigma_x: 8.0,
            sigma_y: 8.0,
        }),
        FilterChain::single(Filter::Refraction {
            strength: 12.0,
            bezel: 10.0,
            dispersion: 0.0,
            ior: 1.2,
        }),
        FilterChain::single(Filter::DropShadow {
            color: Color::rgb(0.0, 0.0, 0.0),
            offset: Point::new(0.0, 0.0), // 零偏移投影：redraw 分支照走
            sigma: 3.0,
        }),
        FilterChain::single(Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 0.0, // 一半恒等
        }),
        FilterChain::new([
            Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 2.0,
            },
            Filter::DropShadow {
                color: Color::rgb(0.0, 0.0, 0.0),
                offset: Point::new(4.0, 4.0),
                sigma: 2.0,
            },
        ]),
        FilterChain::new([
            Filter::Refraction {
                strength: 20.0,
                bezel: 8.0,
                dispersion: 0.5,
                ior: 1.8,
            },
            Filter::Blur {
                sigma_x: 1.0,
                sigma_y: 1.0,
            },
        ]),
    ];
    // 只混入**系数可表达**的模式：它们的混合状态在合成趟生效，必须全执行。
    let blends = [
        BlendMode::SrcOver,
        BlendMode::Screen,
        BlendMode::Plus,
        BlendMode::Modulate,
        BlendMode::Xor,
    ];

    let mut l = DisplayList::new();
    for i in 0..144u32 {
        // 12×12 网格，每格 16px 内容 + 4px 间隔；σ≤8 ⇒ outset≤24，RT 最大 ~64px。
        let (col, row) = (i % 12, i / 12);
        l.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: i,
            },
            Command::Quad {
                rect: Rect::new(4.0 + col as f32 * 20.0, 4.0 + row as f32 * 20.0, 16.0, 16.0),
                radius: zephyra_core::core::CornerRadius { radius: 3.0 },
                fill: Color::rgba(0.9, 0.9, 0.95, 0.8),
                stroke: None,
                filter: chains[(i as usize) % chains.len()].clone(),
                blend: blends[(i as usize / 3) % blends.len()],
            },
        );
    }
    l.sort();

    for round in 0..ROUNDS {
        let report = rig.frame(&l);
        assert!(
            report.unimplemented_filters == 0,
            "第 {round} 帧：144 条命令里有滤镜未执行"
        );
        assert_eq!(
            report.unimplemented_blends, 0,
            "第 {round} 帧：系数可表达的混合未执行 —— 合成趟没按命令换混合状态"
        );
        assert!(
            report.offscreen_passes >= 100,
            "第 {round} 帧：离屏 pass 只有 {sec} —— 144 条里约 130 条该开",
            sec = report.offscreen_passes
        );
        assert!(
            report.refraction_passes >= 12,
            "第 {round} 帧：折射趟只有 {sec}",
            sec = report.refraction_passes
        );
        assert!(
            report.quads >= 144,
            "第 {round} 帧：四边形只有 {sec}",
            sec = report.quads
        );
        assert_eq!(report.skipped, 0, "第 {round} 帧：有命令被静默跳过");
    }
}

/// PS 变体混合 + 滤镜：**已知缺口**的计数器钉。
///
/// 带滤镜的命令用 Multiply / SoftLight 时，合成趟退化成 `SrcOver`（源是 RT
/// 内容、变体的 `ShadeSource` 不采样 `g_tex`，要新的着色器入口才能接）——
/// 于是每条这样的命令在 `unimplemented_blends` 里**如实**记一笔。
///
/// # 这条测试为什么存在
///
/// 它是缺界的「反向门禁」：实现落地那天它会红（计数变成 0），提醒把断言
/// 改过来 + 把缺口记账划掉。没有它，「悄悄修好了但没人知道」和「一直没修」
/// 长得一模一样。同时 `offscreen_passes` 必须照常 > 0 —— 缺口只在混合，
/// 滤镜本身是执行了的。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_filtered_ps_variant_blends_stay_counted() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let mut l = DisplayList::new();
    for i in 0..12u32 {
        l.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: i,
            },
            Command::Quad {
                rect: Rect::new(
                    8.0 + (i % 4) as f32 * 60.0,
                    8.0 + (i / 4) as f32 * 60.0,
                    48.0,
                    48.0,
                ),
                radius: zephyra_core::core::CornerRadius { radius: 6.0 },
                fill: Color::rgba(0.9, 0.9, 0.95, 0.8),
                stroke: None,
                filter: FilterChain::single(Filter::Blur {
                    sigma_x: 3.0,
                    sigma_y: 3.0,
                }),
                // Multiply(24) 与 SoftLight(21) 都是 PS 变体模式。
                blend: if i % 2 == 0 {
                    BlendMode::Multiply
                } else {
                    BlendMode::SoftLight
                },
            },
        );
    }
    l.sort();
    for round in 0..ROUNDS {
        let report = rig.frame(&l);
        assert!(
            report.offscreen_passes >= 12,
            "第 {round} 帧：滤镜没执行（缺口只在混合，不该波及离屏）"
        );
        assert_eq!(report.unimplemented_filters, 0, "第 {round} 帧：滤镜未执行");
        assert_eq!(
            report.unimplemented_blends, 12,
            "第 {round} 帧：PS 变体混合的已知缺口记账变了 —— \
             若已实现合成趟变体，请把本条改成断言 0 并划掉 dev-log 里的缺口记录"
        );
    }
}

/// 合成趟必须用**命令自己的**混合模式（像素级门禁）。
///
/// # 判据为什么是 `Src`
///
/// `Src` = `ONE / ZERO`：合成结果必须**逐字节等于 RT 内容**，与背后画了什么
/// 无关。而缺陷版恒用 `SrcOver` —— 中心像素会变成 `RT + 0.2×背景`，与期望值
/// 差 20% 以上。取 `Src` 而不是 Multiply 之类，是因为它的期望值不依赖混合
/// 数学、只依赖「RT 内容是什么」，而后者由另一条不变量保证（离屏内容 = 该
/// 命令自己）。
///
/// 模糊取 σ=2、内容 200×200：中心处高斯质量漏出矩形的比例是 erfc(35) 量级，
/// 对 8 位输出没有影响。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn composite_uses_the_commands_own_blend_mode() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let mut l = DisplayList::new();
    // 背景：满屏 50% 灰 —— 与内容色差足够大，SrcOver 与 Src 的结果差 26/255。
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.5, 0.5, 0.5),
            stroke: None,
            filter: FilterChain::none(),
            blend: BlendMode::SrcOver,
        },
    );
    // 内容：200×200 白色 80% + 小模糊 + **Src** 混合。
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: Rect::new(28.0, 28.0, 200.0, 200.0),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgba(1.0, 1.0, 1.0, 0.8),
            stroke: None,
            filter: FilterChain::single(Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 2.0,
            }),
            blend: BlendMode::Src,
        },
    );
    l.sort();

    let report = rig.frame(&l);
    assert!(report.offscreen_passes >= 1, "内容没走离屏");
    let px = read_back_center(&rig);
    // RT 内容 = 预乘白 @0.8 = (204,204,204,204)（BGRA 字节序对灰度无差）。
    for (ch, name) in px.iter().zip(["B", "G", "R", "A"]) {
        assert!(
            (*ch as i32 - 204).abs() <= 2,
            "通道 {name} = {ch}：合成没用命令的 Src 混合 —— \
             期望 RT 内容原样落地 (204)，实测像被 SrcOver 成了 230"
        );
    }
}

/// 一条命令挂 8 个滤镜的深链：6 blur + shadow + refraction。
///
/// 乒乓趟数 = 2×6 + 2 = 14 趟，全在两张 RT 之间倒手；`redraw_original`
/// 同时被 shadow 与 refraction 置位 —— 合成后原图还要照常再画一遍。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_a_deep_filter_chain_ping_pongs_cleanly() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let chain = FilterChain::new([
        Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 1.5,
        },
        Filter::Blur {
            sigma_x: 3.0,
            sigma_y: 3.0,
        },
        Filter::DropShadow {
            color: Color::rgb(0.0, 0.0, 0.0),
            offset: Point::new(5.0, 5.0),
            sigma: 2.0,
        },
        Filter::Blur {
            sigma_x: 1.5,
            sigma_y: 2.5,
        },
        Filter::Refraction {
            strength: 30.0,
            bezel: 12.0,
            dispersion: 0.2,
            ior: 1.5,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
        Filter::Blur {
            sigma_x: 0.5,
            sigma_y: 0.5,
        },
    ]);
    let list = quad(
        Rect::new(48.0, 48.0, 160.0, 160.0),
        chain,
        BlendMode::SrcOver,
    );
    for round in 0..ROUNDS {
        let report = rig.frame(&list);
        assert_eq!(
            report.unimplemented_filters, 0,
            "第 {round} 帧：深链有滤镜未执行"
        );
        assert!(report.offscreen_passes >= 1, "第 {round} 帧：深链没开离屏");
        assert!(
            report.refraction_passes >= 1,
            "第 {round} 帧：深链里的折射没执行"
        );
    }
}

/// 两页差别极大的帧交替 6 轮：一页全大模糊、一页全折射。
///
/// RT 对按尺寸重建、dst copy 缓存按尺寸复用 —— 翻页让两者逐帧失效，
/// 每一帧走的都是「第一帧」那条分支（新建、初始状态、首次 clear）。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_alternating_pages_keep_the_rt_pair_honest() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();

    // 页 A：全屏大模糊（bounds = 整屏 + outset，RT 是最大的那种）。
    let mut page_blur = DisplayList::new();
    page_blur.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.8, 0.8, 0.85),
            stroke: None,
            filter: FilterChain::single(Filter::Blur {
                sigma_x: 10.0,
                sigma_y: 10.0,
            }),
            blend: BlendMode::SrcOver,
        },
    );

    // 页 B：四块小折射（bounds 小，RT 与页 A 差一个量级）。
    let mut page_glass = DisplayList::new();
    for i in 0..4u32 {
        page_glass.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: i,
            },
            Command::Quad {
                rect: Rect::new(
                    20.0 + (i % 2) as f32 * 110.0,
                    20.0 + (i / 2) as f32 * 110.0,
                    100.0,
                    100.0,
                ),
                radius: zephyra_core::core::CornerRadius { radius: 12.0 },
                fill: Color::rgb(0.9, 0.9, 0.95),
                stroke: None,
                filter: refraction_chain(),
                blend: BlendMode::SrcOver,
            },
        );
    }

    for round in 0..6 {
        let (list, want_refraction) = if round % 2 == 0 {
            (&page_blur, false)
        } else {
            (&page_glass, true)
        };
        let report = rig.frame(list);
        assert_eq!(report.unimplemented_filters, 0, "第 {round} 帧：滤镜未执行");
        assert_eq!(report.unimplemented_blends, 0, "第 {round} 帧：混合未执行");
        assert!(report.offscreen_passes >= 1, "第 {round} 帧：没开离屏");
        assert_eq!(
            report.refraction_passes >= 1,
            want_refraction,
            "第 {round} 帧：折射趟数与页面不符（blur 页折射={sec}）",
            sec = report.refraction_passes
        );
    }
}
