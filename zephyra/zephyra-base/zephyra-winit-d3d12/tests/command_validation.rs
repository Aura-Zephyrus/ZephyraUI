//! D3D12 命令列表的**传参校验**：调试层开着录每一帧，零 ERROR 级消息才算过。
//!
//! # 为什么需要这个文件
//!
//! 2026-10-24 画廊冻死那条 bug 的形态是：
//!
//! ```text
//! [zephyra-d3d12] 提交失败: Close 命令列表失败: 参数错误 (0x80070057)
//! [zephyra-d3d12] 开帧失败: Reset 命令列表失败: 参数错误 (0x80070057)   ← 此后每帧
//! ```
//!
//! 一条非法命令录进命令列表 ⇒ `Close()` 回 `E_INVALIDARG` ⇒ 整帧提交失败 ⇒
//! 之后每帧 `Reset` 都失败 ⇒ 画面**永久冻结**。而非法命令**在记录时静默通过** ——
//! 驱动只在 `Close()` 才验证。也就是说：不跑到 `Close()`，「录进去的命令合法吗」
//! 这个问题没有答案；而 `E_INVALIDARG` **不带任何说明**，哪个字段错了只有调试层知道。
//!
//! # 这个文件怎么把那两个洞堵上
//!
//! 1. **调试层开着录**（`ZEPHYRA_D3D12_DEBUG=1`，且必须在建设备**之前**开）——
//!    资源状态、屏障前后态、拷贝/解析的参数合法性都在**记录时**校验，消息进
//!    `ID3D12InfoQueue`；
//! 2. **成功路径也抽干队列** —— 验证层是**排队**报的，「没看到错误」完全推不出
//!    「没有错误」。`d3d12_draw.rs` 的夹具只在**失败路径**上调 `drain_debug()`，
//!    而它默认又没开调试层，于是「命令列表合法」这个结论从来没有得到过证据。
//!
//! # 为什么必须有反向对照
//!
//! 本文件所有断言都是「调试层零 ERROR」。如果调试层根本没挂上（这台机器没装
//! 图形工具组件时 `D3D12GetDebugInterface` 会失败），`drain_debug()` 返回空 Vec，
//! 全部断言**恒真**。所以第一条测试故意录一条规格级非法的解析，要求 harness
//! **必须**抓到它。抓不到 ⇒ 其余测试失去意义。
//!
//! # 实测备忘：为什么反向对照用「尺寸不匹配」而不是「目标状态错」
//!
//! 冻死画廊那条 bug 的原形是「解析目标停在 `RENDER_TARGET`（合法状态是
//! `RESOLVE_DEST`）」。照抄那一版做对照，本机驱动**容忍**它：`Close()` 返回
//! 成功、调试层零消息。也就是说同一个错误在不同驱动上一个炸一个不炸 ——
//! 这正是这类 bug 必须靠 harness 兜、而不能靠「我记得它会炸」的原因。
//! 尺寸不匹配（源 256²、目标 128²）是规格级硬规则，任何驱动都拒。
//!
//! # 为什么本文件内部要串行跑
//!
//! `ID3D12InfoQueue` 是**进程级单例**（2026-10-25 两设备交叉探针实证）：
//! 对任何设备 QI 拿到的都是同一个队列，所有设备的消息都进这一个队列、
//! 任何设备的 drain 把全部消息抽走。并行跑会**偷消息**：轻则假红
//! （`separable_blend` 那个偶发 panic 就是偷到反向对照的两条 1050），
//! 重则**假绿**（真 ERROR 被别人的 drain 提前消费，本测试看到空队列）。
//! 所以每个测试开场拿进程级锁 `GPU_GATE` 全程持握，详见锁上的注释。
//!
//! # 怎么跑
//!
//! ```powershell
//! cargo test -p zephyra-winit-d3d12 --test command_validation -- --ignored --nocapture
//! ```
//!
//! 要真 GPU + 调试层，所以标 `#[ignore]`（与本仓其余真 GPU 探针一致）。
//! 调试层不可用的机器上，第一条测试会以「调试层没挂上」失败 —— 那是**环境**结论，
//! 不是代码结论，与 `tests/minimal.rs` 文件头那类记账相同。
#![cfg(windows)]
// 本测试要建真实 D3D12 设备，非 Windows 上没有这条路径。宿主 crate 整个是
// `#![cfg(windows)]`，而集成测试是**独立目标**、不受那个门禁保护 —— 不在这里补
// 一道，`cargo test --workspace` 在 Linux 上就编不过（实测 2026-10）。

use std::env;
use std::sync::Mutex;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
use zephyra_core::core::{Color, Point, Rect};
use zephyra_core::filter::{BlendMode, Filter, FilterChain};
use zephyra_core::paint::{Command, DisplayList};
use zephyra_winit_d3d12::{Gpu, Offscreen, RenderCtx, RenderReport, Renderer, TextureTable};

/// 离屏目标边长（内容尺寸）。
const SIZE: (u32, u32) = (256, 256);
/// 图集边长。只用 Quad，不上传字形，64 足够。
const ATLAS: u32 = 64;
/// MSAA 采样数（与宿主默认档一致，见 `Swapchain::samples`）。
const MSAA_SAMPLES: u32 = 4;
/// 每个场景连录几帧。
///
/// 2 是底线：D3D12 的资源状态**跨帧存活**，离屏 RT 对跨帧复用 —— 「第一帧对、
/// 第二帧起错」正是状态机没记真实状态的形态（本仓踩过三次，见 `renderer.rs`
/// 里 `rt_barrier` 那段与 `the_second_frame_is_identical_*`）。
const ROUNDS: usize = 2;

// ─────────────────── 进程级串行锁 ───────────────────

/// 本测试二进制的进程级串行锁，每个测试开场拿、全程持握。
///
/// # 为什么必须串行（2026-10-25 两设备交叉探针实证）
///
/// `ID3D12InfoQueue` 是**进程级单例**：对**任何**设备 QI 拿到的都是同一个
/// 队列。所有设备的调试层消息都进这一个队列，而任何设备的 `drain` 会把
/// **全部**消息抽走 —— 探针里设备 A 什么都没录，A 的 drain 收到了设备 B
/// 非法 resolve 的两条 1050。（第一版探针曾得出相反结论，但那是因为
/// `Gpu::submit` 的失败路径内部就抽队列，消息在检查前已被 B 自己消费——
/// 假阴性，已修正重测。）
///
/// 并行跑会从两个方向破坏 harness：
/// 1. **假红**：反向对照录完非法 resolve、自己 drain 之前，另一测试的
///    `begin_frame`（内部就抽队列）把消息偷走 → 那测试帧尾看到
///    `[1050, 1050]` 直接 panic。`separable_blend` 那个「偶发红」就是
///    这个，不是串话、不是竞态条件，是共享队列被并行 drain。
/// 2. **假绿**（更危险）：某测试真产了 ERROR，被另一测试的 drain 先抽走
///    → 它看到空队列，「零 ERROR」断言放过 → 本文件的核心保证
///    （干净是真的干净）失效。
///
/// 方向 2 是加锁的真正理由：不只是消 flaky，是保住判别力。
static GPU_GATE: Mutex<()> = Mutex::new(());

fn gpu_gate() -> std::sync::MutexGuard<'static, ()> {
    // 测试 panic 时锁被 poison；`into_inner` 恢复它，让其余测试照跑。
    GPU_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ─────────────────────────────── 夹具 ───────────────────────────────

/// 录一帧、提交、等 GPU 做完，然后**在成功路径上**抽干调试层。
///
/// 三步各自都可能正是非法命令的现形处：
/// - `begin_frame` —— 上一帧 `Close()` 失败时这里连锁失败（冻结链第二环）；
/// - `record` —— 记录时静默通过的那些非法命令，调试层在这里记消息；
/// - `submit` —— `Close()` 回 `0x80070057` 的那一环。
///
/// `samples` 必须与 `target` 的采样数一致（PSO 的 `SampleDesc` 烤死在创建时）。
fn record_one(
    gpu: &mut Gpu,
    renderer: &mut Renderer,
    target: &Offscreen,
    textures: &TextureTable,
    list: &DisplayList,
    clear: Color,
) -> RenderReport {
    gpu.begin_frame().expect(
        "开帧失败 —— 若上一帧 Close() 回 0x80070057，这里会连锁失败\
         （画面永久冻结那条链的第二环）",
    );
    let report = renderer
        .record(RenderCtx {
            device: &gpu.device,
            list: gpu.list(),
            rtv: target.rtv,
            display_list: list,
            size: SIZE,
            clear,
            textures,
            // 折射 / 可分离混合要读目标副本，把**本帧绘制目标**给进去。
            rt: Some(&target.texture),
            damage: zephyra_core::frame::Damage::Full,
        })
        .unwrap_or_else(|e| {
            // 失败时先把队列抽干再 panic —— 否则只拿到一个 E_INVALIDARG，
            // 而「到底是哪条命令非法」只在消息队列里。
            gpu.drain_debug();
            panic!("记录失败: {e}");
        });
    gpu.submit().expect(
        "提交失败 —— Close() 回 E_INVALIDARG(0x80070057) = 命令列表里\
         录进了非法命令（画廊冻死那条 bug 的现形处）",
    );
    gpu.wait_gpu().expect("等 GPU 失败");
    // ★ 成功路径也必须抽干：验证层是排队报的。`d3d12_draw.rs` 的夹具只在
    // 失败路径上抽，等于「命令列表合法」永远得不到证据。
    let severe = gpu.drain_debug();
    assert!(
        severe.is_empty(),
        "调试层报了 {severe:?} 条 ERROR 级消息 —— 录进了非法/不一致的命令。\n\
         常见 ID：527 = 屏障 Before 状态与实际状态不符；880 = ResolveSubresource\
         参数不合法；820 只是性能提示（不进这个列表，drain 只收 ERROR 及以上）。\n\
         消息文本已打在 stderr 上（[d3d12 debug] 前缀）。"
    );
    report
}

/// 一套「开调试层的 GPU + 单采样/4× 两套渲染器与目标」。
///
/// 两套而不是一套：`Renderer::new` 的 `samples` 烤进 PSO 的 `SampleDesc`，
/// 必须与当帧绑定的 RTV 逐位一致。折射的两条岔路（多重采样 ⇒ resolve、
/// 单采样 ⇒ copy）要用不同的采样数才都走得到。
struct Rig {
    gpu: Gpu,
    r1: Renderer,
    t1: Offscreen,
    r4: Renderer,
    t4: Offscreen,
    textures: TextureTable,
}

impl Rig {
    fn new() -> Self {
        // ★ 调试层必须在建设备**之前**开（`Gpu::new` 读这个环境变量，且只在
        // `D3D12CreateDevice` 之前生效）。
        env::set_var("ZEPHYRA_D3D12_DEBUG", "1");
        // 滤镜整条关掉的话离屏段根本不规划，本文件全部断言恒真 —— 显式清掉。
        env::remove_var("ZEPHYRA_NO_FILTERS");
        let gpu = Gpu::new().expect("建不出 D3D12 设备");
        assert!(
            gpu.debug().is_some(),
            "调试层没挂上 —— 本文件所有『零 ERROR』断言都会恒真。\n\
             先跑 `cargo test -p zephyra-winit-d3d12 --test debug_probe -- --ignored \
             --nocapture` 确认这台机器拿得到 ID3D12InfoQueue。"
        );
        let t1 = Offscreen::new(&gpu.device, SIZE).expect("建不出单采样离屏目标");
        let r1 = Renderer::new(&gpu.device, ATLAS, 1).expect("建不出单采样渲染器");
        let t4 =
            Offscreen::new_msaa(&gpu.device, SIZE, MSAA_SAMPLES).expect("建不出 4× MSAA 离屏目标");
        let r4 = Renderer::new(&gpu.device, ATLAS, MSAA_SAMPLES).expect("建不出 4× 渲染器");
        Self {
            gpu,
            r1,
            t1,
            r4,
            t4,
            textures: TextureTable::new(),
        }
    }

    fn frame(&mut self, list: &DisplayList, msaa: bool, clear: Color) -> RenderReport {
        if msaa {
            record_one(
                &mut self.gpu,
                &mut self.r4,
                &self.t4,
                &self.textures,
                list,
                clear,
            )
        } else {
            record_one(
                &mut self.gpu,
                &mut self.r1,
                &self.t1,
                &self.textures,
                list,
                clear,
            )
        }
    }

    /// 用 **Rig 之外**的 4× 目标录一帧。
    ///
    /// 容量余量（源比内容大）与源比内容小两个场景要的是「目标 ≠ 内容尺寸」，
    /// 而 Rig 自带的两个目标都恰好等于内容尺寸 —— 那两场景用自带目标测出来
    /// 恒真。
    fn frame_on_foreign_4x(&mut self, list: &DisplayList, target: &Offscreen) -> RenderReport {
        record_one(
            &mut self.gpu,
            &mut self.r4,
            target,
            &self.textures,
            list,
            Color::rgb(0.0, 0.0, 0.0),
        )
    }
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
            decorations: zephyra_core::decoration::Decorations::none(),
        },
    );
    l
}

/// 带装饰（外投影 + 高光 + 内阴影）的玻璃 Quad —— 开关把手与材质页卡片的形态。
fn decorated_glass(rect: Rect, filter: FilterChain) -> DisplayList {
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
            // 与开关把手同形：**背景全透明**，视觉全由玻璃滤镜 + 装饰承担。
            fill: Color::rgba(1.0, 1.0, 1.0, 0.0),
            stroke: None,
            filter,
            blend: BlendMode::SrcOver,
            decorations: zephyra_core::decoration::Decorations::glass(),
        },
    );
    l
}

/// 画廊玻璃页那块玻璃的参数档（默认 48/40，见 10-22 日志的定档记录）。
fn refraction_chain() -> FilterChain {
    FilterChain::single(Filter::Refraction {
        // 玻璃**厚度**（像素）—— 同时是位移的上界（契约，见 filter.rs）。
        strength: 48.0,
        // 倒角带宽度（像素）：折射只发生在这条带子里。
        bezel: 40.0,
        // 色散量：R/B 沿法线反向拉开。
        dispersion: 0.03,
        // 折射率：冕牌玻璃约 1.5。
        ior: 1.5,
    })
}

/// 画廊玻璃页的一整帧：背景 + 模糊 + 可分离混合 + 折射，四条命令互相叠。
///
/// 单路径测试各自只喂一条命令，而真实页面是**同帧多路径** —— 描述符堆槽位、
/// 离屏 RT 对、目标副本全都在同一条命令列表里被多条路径轮流摸，槽位写错的
/// 症状（「有混合模式时模糊结果整个错掉」）只在同帧共存时出现。
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
                decorations: zephyra_core::decoration::Decorations::none(),
            },
        );
    };
    // 0. 整屏底（无滤镜，被后面三块盖住中心）。
    push(
        0,
        Rect::new(0.0, 0.0, SIZE.0 as f32, SIZE.1 as f32),
        FilterChain::none(),
        BlendMode::SrcOver,
    );
    // 1. 模糊块（离屏链：内容 → RT_A → 模糊乒乓 → 合成）。
    push(
        1,
        Rect::new(8.0, 8.0, 100.0, 100.0),
        FilterChain::single(Filter::Blur {
            sigma_x: 5.0,
            sigma_y: 5.0,
        }),
        BlendMode::SrcOver,
    );
    // 2. 可分离混合块（PS 变体 ⇒ 读目标副本 ⇒ 单采样拷贝岔路）。
    push(
        2,
        Rect::new(148.0, 8.0, 100.0, 100.0),
        FilterChain::none(),
        BlendMode::Multiply,
    );
    // 3. 玻璃块（折射 ⇒ 目标副本 ⇒ resolve 岔路）。
    push(
        3,
        Rect::new(60.0, 130.0, 136.0, 118.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    l
}

// ─────────────────────────────── 反向对照 ───────────────────────────────

/// 故意录一条**非法**解析，harness 必须抓到。
///
/// 用尺寸不匹配版（源 256² 4×、目标 128² 1×）：`ResolveSubresource` 要求源与
/// 目标逐位同尺寸，规格级硬规则，不靠驱动心情。复刻冻死画廊那条 bug 的
/// 「目标停在 `RENDER_TARGET`」版在本机驱动上被容忍（`Close()` 成功、调试层
/// 沉默）—— 实测差异记在文件头，那正是这类错误必须靠 harness 兜的原因。
///
/// 这条测试红 ⇒ 本文件其余「零 ERROR」断言全部恒真，没有意义。
/// 它绿 ⇒ 其余测试的「干净」是真的干净。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn the_gate_detects_an_illegal_resolve() {
    let _gate = gpu_gate();
    env::set_var("ZEPHYRA_D3D12_DEBUG", "1");
    let gpu = Gpu::new().expect("建不出 D3D12 设备");
    assert!(
        gpu.debug().is_some(),
        "调试层没挂上 —— 反向对照恒真，本文件失去意义"
    );
    gpu.begin_frame().expect("开帧失败");
    let src = Offscreen::new_msaa(&gpu.device, SIZE, MSAA_SAMPLES).expect("建不出源目标");
    // 目标**故意**小一半：尺寸不匹配的解析，规格级非法。
    let dst = Offscreen::new(&gpu.device, (SIZE.0 / 2, SIZE.1 / 2)).expect("建不出目标");
    let list = gpu.list();
    unsafe {
        list.ResolveSubresource(&dst.texture, 0, &src.texture, 0, DXGI_FORMAT_R8G8B8A8_UNORM);
    }
    let close = unsafe { list.Close() };
    let severe = gpu.drain_debug();
    assert!(
        close.is_err() || !severe.is_empty(),
        "尺寸不匹配的解析既没让 Close() 失败、也没进调试层队列 —— 本 harness \
         抓不到非法解析，本文件其余『零 ERROR』断言恒真"
    );
}

// ─────────────────────────────── 串行锁的环境前提 ───────────────────────────────

/// 钉住 `GPU_GATE` 依赖的那个环境事实：**info queue 是进程级单例**。
///
/// 两台设备，B 录非法 resolve，A 的 drain 必须收到 B 的消息。收到 ⇒ 任何
/// 设备的 drain 会抽走全部消息 ⇒ 并行跑必然互相偷 ⇒ 锁不是可优化项。
/// 收不到 ⇒ 运行时行为变了（或这台机器的调试层实现不同），锁的前提失效，
/// 该重新评估（而不是继续无条件串行拖慢门禁）。
///
/// 这条测试红 ≠ 有代码 bug，是「环境假设变了」的信号，按文件头同款记账
/// 处理。（2026-10-25 由一次性探针转正：第一版探针被 `submit` 失败路径的
/// 内部抽队列误导成假阴性，删掉 submit 后实测 A 确实收到 B 的两条 1050。）
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn the_info_queue_is_a_process_wide_singleton() {
    let _gate = gpu_gate();
    env::set_var("ZEPHYRA_D3D12_DEBUG", "1");
    let a = Gpu::new().expect("建不出设备 A");
    let b = Gpu::new().expect("建不出设备 B");
    a.begin_frame().expect("A 开帧");
    b.begin_frame().expect("B 开帧");
    // 在 B 上录一条尺寸不匹配的非法 resolve（与反向对照同形）。
    let src = Offscreen::new_msaa(&b.device, SIZE, MSAA_SAMPLES).expect("建不出源目标");
    let dst = Offscreen::new(&b.device, (SIZE.0 / 2, SIZE.1 / 2)).expect("建不出目标");
    unsafe {
        b.list()
            .ResolveSubresource(&dst.texture, 0, &src.texture, 0, DXGI_FORMAT_R8G8B8A8_UNORM);
    }
    let close_b = unsafe { b.list().Close() };
    // ★ 不 submit、不 wait：那两条路径内部会抽队列，把消息提前消费掉
    //   （第一版探针的假阴性就是这么来的）。
    // A 什么都没录，直接抽干它的队列。
    let stolen = a.drain_debug();
    assert!(
        !stolen.is_empty(),
        "A 的队列是空的 —— info queue 不再是进程级单例？\
         （B Close={close_b:?}）若是，GPU_GATE 的前提变了，重新评估串行化"
    );
}

// ─────────────────────────────── 各路径门禁 ───────────────────────────────

/// 无滤镜的普通帧：离屏链不开，也必须是干净的命令。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn plain_quads_record_no_illegal_commands() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(48.0, 48.0, 160.0, 160.0),
        FilterChain::none(),
        BlendMode::SrcOver,
    );
    for _ in 0..ROUNDS {
        let report = rig.frame(&list, false, Color::rgb(0.0, 0.0, 0.0));
        assert!(report.quads > 0, "一条 Quad 都没画出来 —— 场景构造退化了");
        assert_eq!(
            report.offscreen_passes, 0,
            "恒等链开了离屏 pass —— 每帧多一次全屏 RT 绑定，纯浪费"
        );
    }
}

/// 模糊 / 投影的离屏链（内容 → RT_A → 模糊乒乓 → 合成回目标）。
///
/// 跨帧那两帧是关键：RT 对是复用的，而资源状态跨帧存活 ——
/// 「上一帧结束停在什么状态」必须被记住，否则第二帧从错误起点推进
/// （本仓症状：第二帧模糊偏亮，调试层消息 527）。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn blur_and_shadow_offscreen_chain_records_no_illegal_commands() {
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
            let report = rig.frame(&list, false, Color::rgb(0.0, 0.0, 0.0));
            assert!(
                report.offscreen_passes >= 1,
                "滤镜链没开出离屏 pass —— 效果被静默跳过（画面「没设滤镜」）"
            );
            assert_eq!(
                report.unimplemented_filters, 0,
                "滤镜未执行 —— GPU 侧离屏链没接上"
            );
        }
    }
}

/// 可分离混合模式：PS 变体读**目标副本** ⇒ `refresh_dst_copy` 的单采样拷贝岔路
/// （屏障成对：目标 RENDER_TARGET → COPY_SOURCE → RENDER_TARGET，副本 → COPY_DEST
/// → PIXEL_SHADER_RESOURCE）。四条屏障的 `StateBefore` 只要说一次谎，
/// 调试层就会记 527。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn separable_blend_dst_copy_records_no_illegal_commands() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(48.0, 48.0, 160.0, 160.0),
        FilterChain::none(),
        // Multiply 是可分离模式 ⇒ 走 PS 变体 ⇒ 要读 dst。
        BlendMode::Multiply,
    );
    for _ in 0..ROUNDS {
        let report = rig.frame(&list, false, Color::rgb(0.0, 0.0, 0.0));
        assert_eq!(
            report.unimplemented_blends, 0,
            "可分离混合未执行 —— PS 变体那条路没接上"
        );
    }
}

/// 折射的 **resolve 岔路**：多重采样源 ⇒ `ResolveSubresource`。
///
/// 这一条正是 2026-10-24 冻死画廊的路径：目标状态必须 `RESOLVE_DEST`、
/// 源目标必须逐位同尺寸、副本必须单采样。当时三条各错过一遍，
/// 而错误全都在 `Close()` 才现形。本测试要求它每一帧都干净。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn refraction_resolve_path_records_no_illegal_commands() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(60.0, 60.0, 136.0, 136.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    for round in 0..ROUNDS {
        let report = rig.frame(&list, true, Color::rgb(0.0, 0.0, 0.0));
        assert!(
            report.refraction_passes >= 1,
            "第 {round} 帧折射没执行 —— 玻璃页会与『没设滤镜』无异"
        );
        assert!(
            report.offscreen_passes >= 1,
            "第 {round} 帧没开离屏 pass —— 折射链没被规划"
        );
        assert_eq!(report.unimplemented_filters, 0, "滤镜未执行");
    }
}

/// **带装饰的玻璃仍然要真的折射**。
///
/// # 这条钉的是什么
///
/// 合成趟曾经写成 `if hollow_shadow { 外投影 } else { 折射 / 透镜 / 磨砂 }`
/// —— 两者被当成互斥。于是所有带装饰的玻璃（开关把手、材质页卡片）把滤镜
/// 整趟丢掉，屏幕上只剩高光与内阴影。它**不会崩、不会告警**，而且
/// 「没有折射的玻璃」比「折射没接上」更像正常效果，所以只能靠正面计数抓。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn a_decorated_glass_still_runs_the_refraction_pass() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = decorated_glass(Rect::new(60.0, 60.0, 136.0, 136.0), refraction_chain());
    for round in 0..ROUNDS {
        let report = rig.frame(&list, true, Color::rgb(0.2, 0.2, 0.25));
        assert!(
            report.refraction_passes >= 1,
            "第 {round} 帧带装饰的玻璃没走折射 —— 外投影把滤镜挤掉了"
        );
        assert!(report.offscreen_passes >= 1, "第 {round} 帧没开离屏 pass");
        assert_eq!(report.unimplemented_filters, 0, "滤镜未执行");
    }
}

/// 折射的 **copy 岔路**：单采样源 ⇒ `CopyTextureRegion` + 源框。
///
/// 与 resolve 岔路共用同一个 `refresh_dst_copy`，分岔看源资源的
/// `SampleDesc.Count`。源框取内容区、屏障成对 —— 任一项错都会冻死。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn refraction_copy_path_records_no_illegal_commands() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = quad(
        Rect::new(60.0, 60.0, 136.0, 136.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    for round in 0..ROUNDS {
        let report = rig.frame(&list, false, Color::rgb(0.0, 0.0, 0.0));
        assert!(
            report.refraction_passes >= 1,
            "第 {round} 帧折射没执行 —— 单采样下玻璃会消失"
        );
        assert_eq!(report.unimplemented_filters, 0, "滤镜未执行");
    }
}

/// 容量余量档：源比内容大（`Swapchain::resize` 的 25% 余量就长这样）。
///
/// 期望行为是**放弃本次折射**（退回 `SrcOver`，玻璃还在、只是不弯），而不是
/// 录一条尺寸不匹配的解析进去冻死整个应用。这条钉的是 10-24 日志里
/// 「已知边界」那条保守选择的**行为**：以前它只有注释，没有门禁。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn refraction_skips_instead_of_freezing_when_the_source_is_larger() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    // 384² 的目标、256² 的内容 —— 与宿主「MSAA 纹理按容量建」同形。
    let slack =
        Offscreen::new_msaa(&rig.gpu.device, (384, 384), MSAA_SAMPLES).expect("建不出余量目标");
    let list = quad(
        Rect::new(60.0, 60.0, 136.0, 136.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    for round in 0..ROUNDS {
        let report = rig.frame_on_foreign_4x(&list, &slack);
        assert_eq!(
            report.refraction_passes, 0,
            "第 {round} 帧源比内容大却仍然折射 —— 要么录进了尺寸不匹配的解析\
             （冻死），要么守卫没生效"
        );
    }
}

/// 源比内容小：守卫必须同样拒绝。
///
/// 这一档钉的是一个**潜在**漏洞：resolve 守卫原来拿 `min(size, src)` 当判据，
/// 源更小时 min 等于源、检查通过，于是一张小图被解析进大副本 —— 尺寸不匹配
/// 的解析是规格级非法调用，录进去就冻死。宿主当前传的源不会小于内容
/// （MSAA 纹理与后缓冲都 ≥ 内容尺寸），所以这是「还没炸」而不是「不会炸」。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn refraction_skips_instead_of_freezing_when_the_source_is_smaller() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    // 128² 的目标、256² 的内容 —— 源比内容小。
    let small =
        Offscreen::new_msaa(&rig.gpu.device, (128, 128), MSAA_SAMPLES).expect("建不出小目标");
    let list = quad(
        Rect::new(20.0, 20.0, 88.0, 88.0),
        refraction_chain(),
        BlendMode::SrcOver,
    );
    for round in 0..ROUNDS {
        let report = rig.frame_on_foreign_4x(&list, &small);
        assert_eq!(
            report.refraction_passes, 0,
            "第 {round} 帧源比内容小却仍然折射 —— 尺寸不匹配的解析被录进去了，\
             换台严格的驱动就是 Close() 0x80070057 + 永久冻结"
        );
    }
}

/// 画廊玻璃页的一整帧：四条路径**同帧共存**，连录三帧。
///
/// 单路径测试各自干净推不出同帧干净：SRV 堆槽位（`DST_SLOT` 与离屏 B 的
/// 槽位只差一个常量）、离屏 RT 对的复用、目标副本的跨帧状态，全都只在
/// 多条路径轮流摸同一条命令列表时才会打架。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn a_full_glass_page_records_no_illegal_commands() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let list = glass_page();
    for round in 0..3 {
        let report = rig.frame(&list, true, Color::rgb(0.0, 0.0, 0.0));
        assert!(report.refraction_passes >= 1, "第 {round} 帧折射没执行");
        assert!(
            report.offscreen_passes >= 2,
            "第 {round} 帧离屏 pass 数 {sec} —— 模糊与折射两条链都应开出离屏",
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
// 的深链、以及逐帧翻页的资源重建。都零 ERROR 且计数为 0，才敢说「这条链
// 没问题」而不是「这条链在简单输入上没问题」。
//
// # 混合模式分两级断言（2026-10-25 起，与 D3D11 侧同构）
//
// - **系数可表达 15 个**（`Clear..Screen`）：合成趟已按命令的混合状态选 PSO，
//   压力页混入它们也必须是 `unimplemented_blends == 0`。
// - **PS 变体 14 个**（`Overlay..Luminosity`）：合成趟的源是 RT 内容而不是
//   图元几何，变体的 `ShadeSource` 不采样 `g_tex` —— **已知缺口**，
//   `stress_filtered_ps_variant_blends_stay_counted` 钉住「计数器如实报数」。

/// 12 种链 × 12 个矩形轮转，一帧 144 条带滤镜的命令，两种采样数各跑一轮。
///
/// RT 对按尺寸重建、描述符堆槽位逐段推进、目标副本逐段刷新 —— 帧内上百条
/// 离屏命令把这些全部推到量级上。单采样走 copy 岔路、4× 走 resolve 岔路，
/// 两条都必须在压力下干净。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_many_filtered_commands_record_no_illegal_commands() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();

    let chains = [
        FilterChain::single(Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        }),
        FilterChain::single(Filter::Blur {
            sigma_x: 5.0,
            sigma_y: 3.0,
        }),
        FilterChain::single(Filter::DropShadow {
            color: Color::rgb(0.0, 0.0, 0.0),
            offset: Point::new(6.0, 8.0),
            sigma: 4.0,
        }),
        FilterChain::single(Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
            offset: Point::new(-10.0, -10.0),
            sigma: 2.0,
        }),
        refraction_chain(),
        FilterChain::single(Filter::Blur {
            sigma_x: 0.0,
            sigma_y: 0.0,
        }),
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
            offset: Point::new(0.0, 0.0),
            sigma: 3.0,
        }),
        FilterChain::single(Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 0.0,
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
    let blends = [
        BlendMode::SrcOver,
        BlendMode::Screen,
        BlendMode::Plus,
        BlendMode::Modulate,
        BlendMode::Xor,
    ];

    let mut l = DisplayList::new();
    for i in 0..144u32 {
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
                decorations: zephyra_core::decoration::Decorations::none(),
            },
        );
    }
    l.sort();

    for msaa in [false, true] {
        for round in 0..ROUNDS {
            let report = rig.frame(&l, msaa, Color::rgb(0.0, 0.0, 0.0));
            let tag = if msaa { "4×" } else { "1×" };
            assert_eq!(
                report.unimplemented_filters, 0,
                "{tag} 第 {round} 帧：144 条命令里有滤镜未执行"
            );
            assert_eq!(
                report.unimplemented_blends, 0,
                "{tag} 第 {round} 帧：系数可表达的混合未执行 —— 合成趟没按命令选 PSO"
            );
            assert!(
                report.offscreen_passes >= 100,
                "{tag} 第 {round} 帧：离屏 pass 只有 {sec}",
                sec = report.offscreen_passes
            );
            assert!(
                report.refraction_passes >= 12,
                "{tag} 第 {round} 帧：折射趟只有 {sec}",
                sec = report.refraction_passes
            );
            assert!(
                report.quads >= 144,
                "{tag} 第 {round} 帧：四边形只有 {sec}",
                sec = report.quads
            );
            assert_eq!(report.skipped, 0, "{tag} 第 {round} 帧：有命令被静默跳过");
        }
    }
}

/// PS 变体混合 + 滤镜：**已知缺口**的计数器钉（与 D3D11 侧同构）。
///
/// 实现落地那天它会红（计数变 0），提醒把断言改过来并划掉 dev-log 的缺口记录。
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
                blend: if i % 2 == 0 {
                    BlendMode::Multiply
                } else {
                    BlendMode::SoftLight
                },
                decorations: zephyra_core::decoration::Decorations::none(),
            },
        );
    }
    l.sort();
    for round in 0..ROUNDS {
        let report = rig.frame(&l, false, Color::rgb(0.0, 0.0, 0.0));
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

/// 一条命令挂 8 个滤镜的深链：6 blur + shadow + refraction。
///
/// 乒乓趟数 = 14 趟，全在两张 RT 之间倒手；`redraw_original` 同时被 shadow 与
/// refraction 置位。两种采样数各跑一轮 —— 单采样 copy 岔路与 4× resolve 岔路
/// 在长链上各被摸 ROUNDS 次。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_a_deep_filter_chain_records_no_illegal_commands() {
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
    for msaa in [false, true] {
        for round in 0..ROUNDS {
            let tag = if msaa { "4×" } else { "1×" };
            let report = rig.frame(&list, msaa, Color::rgb(0.0, 0.0, 0.0));
            assert_eq!(
                report.unimplemented_filters, 0,
                "{tag} 第 {round} 帧：深链有滤镜未执行"
            );
            assert!(
                report.offscreen_passes >= 1,
                "{tag} 第 {round} 帧：深链没开离屏"
            );
            assert!(
                report.refraction_passes >= 1,
                "{tag} 第 {round} 帧：深链里的折射没执行"
            );
        }
    }
}

/// 两页差别极大的帧交替 6 轮：一页全大模糊、一页全折射。
///
/// RT 对按尺寸重建、目标副本缓存按尺寸复用 —— 翻页让两者逐帧失效，
/// 每一帧走的都是「第一帧」那条分支（新建、初始状态）。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn stress_alternating_pages_keep_resources_honest() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();

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
            decorations: zephyra_core::decoration::Decorations::none(),
        },
    );

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
                decorations: zephyra_core::decoration::Decorations::none(),
            },
        );
    }

    for round in 0..6 {
        let (list, want_refraction) = if round % 2 == 0 {
            (&page_blur, false)
        } else {
            (&page_glass, true)
        };
        let report = rig.frame(list, true, Color::rgb(0.0, 0.0, 0.0));
        assert_eq!(report.unimplemented_filters, 0, "第 {round} 帧：滤镜未执行");
        assert!(report.offscreen_passes >= 1, "第 {round} 帧：没开离屏");
        assert_eq!(
            report.refraction_passes >= 1,
            want_refraction,
            "第 {round} 帧：折射趟数与页面不符（refraction={sec}）",
            sec = report.refraction_passes
        );
    }
}

/// 录一帧并回读整张目标（成功路径同样抽干调试层）。
///
/// 与 `d3d12_draw.rs` 的 `render_and_readback` 同构，但复用 Rig 的渲染器与
/// 目标 —— 压力门禁要的是「同一对资源连录多帧」那条路上的像素，不是新建一套。
fn frame_with_readback(rig: &mut Rig, list: &DisplayList, clear: Color) -> (Vec<u8>, RenderReport) {
    rig.gpu.begin_frame().expect("开帧失败");
    let report = rig
        .r1
        .record(RenderCtx {
            device: &rig.gpu.device,
            list: rig.gpu.list(),
            rtv: rig.t1.rtv,
            display_list: list,
            size: SIZE,
            clear,
            textures: &rig.textures,
            rt: Some(&rig.t1.texture),
            damage: zephyra_core::frame::Damage::Full,
        })
        .unwrap_or_else(|e| {
            rig.gpu.drain_debug();
            panic!("记录失败: {e}");
        });
    let staging = rig
        .t1
        .record_readback(&rig.gpu.device, rig.gpu.list())
        .expect("记录回读拷贝失败");
    rig.gpu.submit().expect("提交失败");
    rig.gpu.wait_gpu().expect("等 GPU 失败");
    let severe = rig.gpu.drain_debug();
    assert!(severe.is_empty(), "调试层报了 {severe:?} 条 ERROR 级消息");
    (Offscreen::map(&staging, SIZE), report)
}

/// 合成趟必须用**命令自己的**混合模式（像素级门禁，与 D3D11 侧同构）。
///
/// 判据用 `Src`（`ONE / ZERO`）：合成结果必须逐字节等于 RT 内容。缺陷版恒用
/// `SrcOver`，中心像素会多出 0.2×背景 —— 差 26/255，远超舍入。
#[test]
#[ignore = "要真 GPU + 调试层。跑法见文件头"]
fn composite_uses_the_commands_own_blend_mode() {
    let _gate = gpu_gate();
    let mut rig = Rig::new();
    let mut l = DisplayList::new();
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
            decorations: zephyra_core::decoration::Decorations::none(),
        },
    );
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
            decorations: zephyra_core::decoration::Decorations::none(),
        },
    );
    l.sort();

    let (px, report) = frame_with_readback(&mut rig, &l, Color::rgb(0.0, 0.0, 0.0));
    assert!(report.offscreen_passes >= 1, "内容没走离屏");
    let center = ((SIZE.1 / 2) as usize * SIZE.0 as usize + (SIZE.0 / 2) as usize) * 4;
    for (ch, name) in px[center..center + 4].iter().zip(["R", "G", "B", "A"]) {
        assert!(
            (*ch as i32 - 204).abs() <= 2,
            "通道 {name} = {ch}：合成没用命令的 Src 混合 —— \
             期望 RT 内容原样落地 (204)，实测像被 SrcOver 成了 230"
        );
    }
}
