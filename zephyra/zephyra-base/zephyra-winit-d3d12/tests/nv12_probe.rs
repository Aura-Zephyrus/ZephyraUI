//! NV12 SRV 能力的真机探针（默认 ignored，需要 Windows + D3D12 GPU）。
//!
//! 本仓的 D3D11 后端在部分适配器上建 NV12 SRV 会拿到 `0x80070057`（显式 desc
//! 被拒）。D3D12 这边改用**隐式派生**（`CreateShaderResourceView(res, None, …)`），
//! 本探针在真机上确认这条路径是否可用；结果只打到 stdout，由人记录，不因适配器
//! 能力差异而让门禁变红。

#![cfg(windows)]

use windows::core::Interface as _;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory4};

/// 在默认适配器上建 NV12 纹理 + 隐式 SRV，并打印 footprint 参数。
#[test]
#[ignore = "requires Windows + D3D12 GPU; NV12 SRV capability probe"]
fn nv12_texture_and_implicit_srv_probe() {
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.expect("CreateDXGIFactory1");
    let adapter = unsafe { factory.EnumAdapters1(0) }.expect("no adapter 0");
    let iunknown = adapter.cast::<windows::core::IUnknown>().expect("IUnknown");
    let mut device: Option<ID3D12Device> = None;
    unsafe { D3D12CreateDevice(&iunknown, D3D_FEATURE_LEVEL_11_0, &mut device) }
        .expect("D3D12CreateDevice failed");
    let device = device.expect("null device");

    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: 64,
        Height: 64,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    };
    let heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 1,
        VisibleNodeMask: 1,
    };
    let mut texture: Option<ID3D12Resource> = None;
    let created = unsafe {
        device.CreateCommittedResource(
            &heap,
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut texture as *mut _,
        )
    };
    let texture = match created {
        Ok(()) => texture.expect("null texture"),
        Err(e) => {
            eprintln!("[nv12-probe] CreateCommittedResource(NV12) FAILED: {e:?}");
            return;
        }
    };
    eprintln!("[nv12-probe] NV12 texture created OK");

    // 驱动自报的 NV12 sample 能力。
    let mut support = D3D12_FEATURE_DATA_FORMAT_SUPPORT {
        Format: DXGI_FORMAT_NV12,
        ..Default::default()
    };
    let support_ok = unsafe {
        device.CheckFeatureSupport(
            D3D12_FEATURE_FORMAT_SUPPORT,
            &mut support as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<D3D12_FEATURE_DATA_FORMAT_SUPPORT>() as u32,
        )
    };
    eprintln!(
        "[nv12-probe] CheckFeatureSupport(NV12): ok={support_ok:?} Support1={:?}",
        support.Support1
    );

    // 两个子资源的 footprint —— 验证 Y/UV 行距与偏移能被驱动给出。
    let mut footprints = [D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); 2];
    let mut total = 0u64;
    unsafe {
        device.GetCopyableFootprints(
            &desc,
            0,
            2,
            0,
            Some(footprints.as_mut_ptr()),
            None,
            None,
            Some(&mut total),
        );
    }
    eprintln!(
        "[nv12-probe] footprints: Y(offset={}, pitch={}) UV(offset={}, pitch={}) total={}",
        footprints[0].Offset,
        footprints[0].Footprint.RowPitch,
        footprints[1].Offset,
        footprints[1].Footprint.RowPitch,
        total
    );
    assert!(
        footprints[0].Footprint.RowPitch >= 64 && footprints[1].Footprint.RowPitch >= 64,
        "NV12 两平面行距必须至少装下 64 字节的 tight 行"
    );

    // 隐式 SRV：只建堆 + 让 D3D12 从资源推格式，不手写 desc。
    let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        NumDescriptors: 1,
        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
        NodeMask: 0,
    };
    let srv_heap: ID3D12DescriptorHeap =
        unsafe { device.CreateDescriptorHeap(&heap_desc) }.expect("CreateDescriptorHeap failed");
    let cpu = unsafe { srv_heap.GetCPUDescriptorHandleForHeapStart() };
    unsafe { device.CreateShaderResourceView(&texture, None, cpu) };
    eprintln!("[nv12-probe] implicit CreateShaderResourceView(NV12) call returned OK");
    let healthy = match unsafe { device.GetDeviceRemovedReason() } {
        Ok(()) => {
            eprintln!("[nv12-probe] device healthy after implicit NV12 SRV");
            true
        }
        Err(e) => {
            eprintln!("[nv12-probe] device removed after implicit NV12 SRV: {e:?}");
            false
        }
    };
    if !healthy {
        eprintln!(
            "[nv12-probe] CONCLUSION: this adapter reports SHADER_SAMPLE support \
                   but NV12 SRV removes the device — real-device pixel gate not possible here"
        );
    }

    // 对照组：RGBA 纹理也能建（确认堆/句柄本身没问题）。本机有「多次建资源触发
    // TDR」的历史，所以这里只报告、不 fail —— 上面那条才是 NV12 的判据。
    let rgba_desc = D3D12_RESOURCE_DESC {
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ..desc
    };
    let mut rgba: Option<ID3D12Resource> = None;
    let rgba = unsafe {
        device.CreateCommittedResource(
            &heap,
            D3D12_HEAP_FLAG_NONE,
            &rgba_desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut rgba as *mut _,
        )
    };
    match rgba {
        Ok(()) => eprintln!("[nv12-probe] RGBA control OK"),
        Err(e) => eprintln!("[nv12-probe] RGBA control FAILED (environment): {e:?}"),
    }
    if healthy {
        eprintln!("[nv12-probe] NV12 implicit SRV path is usable on this adapter");
    }
}

/// 对照组：完全不碰 NV12，只建 RGBA 纹理 + 隐式 SRV，看设备是否同样被移除。
/// 若这条也报 device removed，说明是本机图形栈的环境问题，与 NV12 无关。
#[test]
#[ignore = "requires Windows + D3D12 GPU; environment control probe"]
fn rgba_only_control_probe() {
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.expect("CreateDXGIFactory1");
    let adapter = unsafe { factory.EnumAdapters1(0) }.expect("no adapter 0");
    let iunknown = adapter.cast::<windows::core::IUnknown>().expect("IUnknown");
    let mut device: Option<ID3D12Device> = None;
    unsafe { D3D12CreateDevice(&iunknown, D3D_FEATURE_LEVEL_11_0, &mut device) }
        .expect("D3D12CreateDevice failed");
    let device = device.expect("null device");

    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: 64,
        Height: 64,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    };
    let heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 1,
        VisibleNodeMask: 1,
    };
    let mut texture: Option<ID3D12Resource> = None;
    unsafe {
        device.CreateCommittedResource(
            &heap,
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            &mut texture as *mut _,
        )
    }
    .expect("RGBA texture should create");
    let texture = texture.expect("null texture");
    eprintln!("[rgba-control] RGBA texture created OK");

    let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        NumDescriptors: 1,
        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
        NodeMask: 0,
    };
    let srv_heap: ID3D12DescriptorHeap =
        unsafe { device.CreateDescriptorHeap(&heap_desc) }.expect("CreateDescriptorHeap failed");
    let cpu = unsafe { srv_heap.GetCPUDescriptorHandleForHeapStart() };
    unsafe { device.CreateShaderResourceView(&texture, None, cpu) };
    eprintln!("[rgba-control] implicit SRV created OK");
    match unsafe { device.GetDeviceRemovedReason() } {
        Ok(()) => eprintln!("[rgba-control] device healthy"),
        Err(e) => eprintln!("[rgba-control] device removed (ENVIRONMENT): {e:?}"),
    }
}
