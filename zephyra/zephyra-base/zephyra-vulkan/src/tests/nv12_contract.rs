//! NV12 多平面上传 + 管线选择的纯逻辑门禁。
//!
//! 这些断言**不依赖真机 GPU**：钉住的是传给 Vulkan API 的参数（平面尺寸 /
//! aspect / 格式 / pitch）与「NV12 格式选到 NV12 管线」的纯逻辑分派。
//! 真机像素路径由 `src/renderer_tests.rs` 的 ignore 用例与上层 gallery 承担。

use super::*;
use ash::vk;
use zephyra_core::filter::BlendMode;
use zephyra_render_ext::prelude::ExternalTextureFormat;

// ── 上传参数 ───────────────────────────────────────────────────────────────

/// 格式必须是 core Vulkan 1.0 的双平面 4:2:0，两个 plane view 分别是 R8/R8G8。
#[test]
fn nv12_image_and_plane_view_formats_are_core_and_consistent() {
    assert_eq!(NV12_IMAGE_FORMAT, vk::Format::G8_B8R8_2PLANE_420_UNORM);
    assert_eq!(NV12_Y_VIEW_FORMAT, vk::Format::R8_UNORM);
    assert_eq!(NV12_UV_VIEW_FORMAT, vk::Format::R8G8_UNORM);
}

/// plane 序号 → aspect mask：Y=PLANE_0，UV=PLANE_1。
#[test]
fn nv12_plane_aspect_maps_y_and_uv_to_distinct_planes() {
    assert_eq!(nv12_plane_aspect(0), vk::ImageAspectFlags::PLANE_0);
    assert_eq!(nv12_plane_aspect(1), vk::ImageAspectFlags::PLANE_1);
}

/// 1080p 紧排：Y 用整幅尺寸、UV 用半幅尺寸，UV 的 bufferRowLength 是 pitch/2。
#[test]
fn nv12_upload_plan_uses_full_luma_and_half_chroma_extent() {
    let [y, uv] = nv12_upload_plan((1920, 1080), 1920, 1920).expect("valid 1080p plan");
    assert_eq!(y.plane, 0);
    assert_eq!(y.extent, (1920, 1080));
    assert_eq!(y.row_length_texels, 1920);
    assert_eq!(nv12_plane_aspect(y.plane), vk::ImageAspectFlags::PLANE_0);

    assert_eq!(uv.plane, 1);
    assert_eq!(uv.extent, (960, 540));
    assert_eq!(uv.row_length_texels, 960); // uv_pitch 1920 bytes / 2 = 960 RG8 texels
    assert_eq!(nv12_plane_aspect(uv.plane), vk::ImageAspectFlags::PLANE_1);
}

/// 带行距的帧：上传参数必须按 pitch 而不是按 width 算。
#[test]
fn nv12_upload_plan_honors_padded_row_pitches() {
    let [y, uv] = nv12_upload_plan((1280, 720), 1320, 1360).expect("padded plan");
    assert_eq!(y.extent, (1280, 720));
    assert_eq!(y.row_length_texels, 1320);
    assert_eq!(uv.extent, (640, 360));
    assert_eq!(uv.row_length_texels, 680); // 1360 / 2
}

/// 非法几何必须报错而非静默生成错位平面（奇数尺寸会让色度错开半像素）。
#[test]
fn nv12_upload_plan_rejects_invalid_geometry() {
    assert!(nv12_upload_plan((0, 1080), 0, 0).is_err(), "zero width");
    assert!(
        nv12_upload_plan((1920, 0), 1920, 1920).is_err(),
        "zero height"
    );
    assert!(
        nv12_upload_plan((1921, 1080), 1921, 1920).is_err(),
        "odd width"
    );
    assert!(
        nv12_upload_plan((1920, 1081), 1920, 1920).is_err(),
        "odd height"
    );
    assert!(
        nv12_upload_plan((1920, 1080), 1024, 1920).is_err(),
        "y pitch < width"
    );
    assert!(
        nv12_upload_plan((1920, 1080), 1920, 959).is_err(),
        "odd uv pitch"
    );
    assert!(
        nv12_upload_plan((1920, 1080), 1920, 800).is_err(),
        "uv pitch < half_w*2"
    );
}

// ── 管线选择 ───────────────────────────────────────────────────────────────

/// NV12 格式 → NV12 族；所有 RGBA 族格式仍走原管线（RGBA 行为不变）。
#[test]
fn nv12_format_selects_nv12_family_rgba_formats_stay_original() {
    assert_eq!(
        pipeline_family(ExternalTextureFormat::Nv12),
        PipelineFamily::Nv12
    );
    assert_eq!(
        pipeline_family(ExternalTextureFormat::Rgba8UnormSrgb),
        PipelineFamily::Rgba
    );
    assert_eq!(
        pipeline_family(ExternalTextureFormat::Bgra8Unorm),
        PipelineFamily::Rgba
    );
    assert_eq!(
        pipeline_family(ExternalTextureFormat::I420),
        PipelineFamily::Rgba
    );
    assert!(uses_nv12_pipeline(ExternalTextureFormat::Nv12));
    assert!(!uses_nv12_pipeline(ExternalTextureFormat::Rgba8UnormSrgb));
}

/// 同一混合模式：NV12 选 NV12 族 + 同一槽位，RGBA 选原族 + 同一槽位。
#[test]
fn choose_pipeline_routes_same_blend_to_family_specific_pipeline() {
    assert_eq!(
        choose_pipeline(BlendMode::SrcOver, ExternalTextureFormat::Nv12),
        Some((PipelineFamily::Nv12, SelectedPipeline::Fixed(3)))
    );
    assert_eq!(
        choose_pipeline(BlendMode::SrcOver, ExternalTextureFormat::Rgba8UnormSrgb),
        Some((PipelineFamily::Rgba, SelectedPipeline::Fixed(3)))
    );
    // PS 变体族也要分家（overlay 是第 0 档 PS 变体）。
    assert_eq!(
        choose_pipeline(BlendMode::Overlay, ExternalTextureFormat::Nv12),
        Some((PipelineFamily::Nv12, SelectedPipeline::Ps(0)))
    );
    assert_eq!(
        choose_pipeline(BlendMode::Overlay, ExternalTextureFormat::Rgba8UnormSrgb),
        Some((PipelineFamily::Rgba, SelectedPipeline::Ps(0)))
    );
}

/// 槽位选择必须覆盖全部 15 档系数混合 + 14 档 PS 变体，不能被 `?` 提前截断。
#[test]
fn choose_pipeline_covers_all_fixed_and_ps_variant_slots() {
    assert_eq!(
        choose_pipeline(BlendMode::Clear, ExternalTextureFormat::Nv12),
        Some((PipelineFamily::Nv12, SelectedPipeline::Fixed(0)))
    );
    assert_eq!(
        choose_pipeline(BlendMode::Screen, ExternalTextureFormat::Nv12),
        Some((PipelineFamily::Nv12, SelectedPipeline::Fixed(14)))
    );
    assert_eq!(
        choose_pipeline(BlendMode::Luminosity, ExternalTextureFormat::Nv12),
        Some((PipelineFamily::Nv12, SelectedPipeline::Ps(13)))
    );
}
