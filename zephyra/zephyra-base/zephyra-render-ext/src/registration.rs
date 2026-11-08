//! Hook/扩展注册契约（§4.2 全量表面，§17 P1-2/P1-4/S2；dyn-compatible 适配）。

use std::any::Any;
use std::borrow::Cow;

use crate::types::{ColorSpace, ExternalTextureFormat, Rect};

// ---------- RenderSource (S2: 调用期借用) ----------
pub struct RenderSource<'a> {
    pub resource_key: Option<&'a str>,
    payload: Option<&'a dyn Any>,
}
impl<'a> RenderSource<'a> {
    pub fn new(resource_key: Option<&'a str>, payload: Option<&'a dyn Any>) -> Self {
        Self {
            resource_key,
            payload,
        }
    }
    pub fn none() -> Self {
        Self::new(None, None)
    }
    pub fn resource_key(&self) -> Option<&str> {
        self.resource_key
    }
    pub fn payload<T: 'static>(&self) -> Option<&T> {
        self.payload.and_then(|p| p.downcast_ref())
    }
}
impl core::fmt::Debug for RenderSource<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderSource")
            .field("resource_key", &self.resource_key)
            .finish_non_exhaustive()
    }
}

/// HRTB（S2）。
pub type DrawHook =
    dyn for<'a> FnMut(&mut dyn HookContext, Rect, &RenderSource<'a>) + Send + 'static;

/// 无 HookCtx 关联类型（P1-2）。`window: None` = 全局；`Some(id)` = 窗口作用域（D3D12/多窗口）。
pub trait RenderExtension {
    fn register_hook(&mut self, kind: Cow<'static, str>, hook: Box<DrawHook>);
    fn register_hook_for_window(
        &mut self,
        _window: u64,
        kind: Cow<'static, str>,
        hook: Box<DrawHook>,
    ) {
        self.register_hook(kind, hook);
    }
    fn unregister_hook(&mut self, kind: &str, window: Option<u64>) -> bool;
    fn has_hook(&self, kind: &str, window: Option<u64>) -> bool;
    fn registered_kinds(&self) -> Vec<Cow<'static, str>>;
}
pub trait ExtensionFactory: Send + Sync {
    fn create_and_register(
        &self,
        register: &mut dyn FnMut(Cow<'static, str>, Box<DrawHook>),
    ) -> Box<dyn Any + Send + Sync>;
}

// ---------- Supporting types (§4.2) ----------
macro_rules! opaque_handle {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name(pub u64);
    };
}
opaque_handle!(PipelineId);
opaque_handle!(BufferHandle);
opaque_handle!(DescriptorSetHandle);
opaque_handle!(SamplerHandle);
opaque_handle!(TextureViewHandle);
opaque_handle!(ImageViewHandle);
opaque_handle!(TextureHandle);
opaque_handle!(AccelerationStructureHandle);
opaque_handle!(RayTracingPipelineHandle);
opaque_handle!(ShaderBindingTableHandle);
opaque_handle!(SemaphoreHandle);
opaque_handle!(TimerQueryHandle);
opaque_handle!(ResourceHandle);
opaque_handle!(HeapHandle);
opaque_handle!(TransientHeapHandle);
opaque_handle!(GpuTextureId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RenderPassType {
    #[default]
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderTargetInfo {
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub is_srgb: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DepthStencilInfo {
    pub width: u32,
    pub height: u32,
    pub format: ExternalTextureFormat,
    pub sample_count: u32,
    pub has_depth: bool,
    pub has_stencil: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BlendFactor {
    #[default]
    One,
    Zero,
    SrcColor,
    OneMinusSrcColor,
    DstColor,
    OneMinusDstColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
    ConstantColor,
    OneMinusConstantColor,
    ConstantAlpha,
    OneMinusConstantAlpha,
    SrcAlphaSaturate,
    Src1Color,
    OneMinusSrc1Color,
    Src1Alpha,
    OneMinusSrc1Alpha,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BlendOp {
    #[default]
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ColorWriteMask(pub u8);
impl ColorWriteMask {
    pub const RED: Self = Self(1 << 0);
    pub const GREEN: Self = Self(1 << 1);
    pub const BLUE: Self = Self(1 << 2);
    pub const ALPHA: Self = Self(1 << 3);
    pub const ALL: Self = Self(0xF);
    pub const NONE: Self = Self(0);
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CompareOp {
    #[default]
    Less,
    Never,
    Equal,
    LessOrEqual,
    Greater,
    NotEqual,
    GreaterOrEqual,
    Always,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StencilOp {
    #[default]
    Keep,
    Zero,
    Replace,
    IncrementAndClamp,
    DecrementAndClamp,
    Invert,
    IncrementAndWrap,
    DecrementAndWrap,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StencilFaceState {
    pub fail_op: StencilOp,
    pub pass_op: StencilOp,
    pub depth_fail_op: StencilOp,
    pub compare_op: CompareOp,
    pub compare_mask: u32,
    pub write_mask: u32,
    pub reference: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CullMode {
    #[default]
    Back,
    None,
    Front,
    FrontAndBack,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FrontFace {
    #[default]
    CounterClockwise,
    Clockwise,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PolygonMode {
    #[default]
    Fill,
    Line,
    Point,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ConservativeRasterMode {
    #[default]
    Off,
    On,
    PostDepth,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MultisampleState {
    pub sample_count: u32,
    pub sample_mask: u32,
    pub alpha_to_coverage_enabled: bool,
    pub alpha_to_one_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BufferBinding {
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
    pub stride: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DescriptorSetBinding {
    pub set_index: u32,
    pub descriptor_set: DescriptorSetHandle,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SamplerBinding {
    pub binding: u32,
    pub sampler: SamplerHandle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextureLayout {
    #[default]
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextureBinding {
    pub binding: u32,
    pub texture_view: TextureViewHandle,
    pub layout: Option<TextureLayout>,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StorageImageBinding {
    pub binding: u32,
    pub image_view: ImageViewHandle,
    pub layout: Option<TextureLayout>,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StorageBufferBinding {
    pub binding: u32,
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ConstantBufferBinding {
    pub binding: u32,
    pub buffer: BufferHandle,
    pub offset: u64,
    pub size: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AccelerationStructureBinding {
    pub binding: u32,
    pub acceleration_structure: AccelerationStructureHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ShadingRate {
    #[default]
    Rate1x1,
    Rate1x2,
    Rate2x1,
    Rate2x2,
    Rate2x4,
    Rate4x2,
    Rate4x4,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum VrsCombiner {
    #[default]
    Passthrough,
    Override,
    Min,
    Max,
    Sum,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VrsState {
    pub enabled: bool,
    pub shading_rate: ShadingRate,
    pub combiners: [VrsCombiner; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PipelineStageFlags(pub u32);
impl PipelineStageFlags {
    pub const TOP_OF_PIPE: Self = Self(1 << 0);
    pub const DRAW_INDIRECT: Self = Self(1 << 1);
    pub const VERTEX_INPUT: Self = Self(1 << 2);
    pub const VERTEX_SHADER: Self = Self(1 << 3);
    pub const FRAGMENT_SHADER: Self = Self(1 << 7);
    pub const COMPUTE_SHADER: Self = Self(1 << 11);
    pub const TRANSFER: Self = Self(1 << 12);
    pub const ALL_COMMANDS: Self = Self(1 << 16);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SemaphoreWait {
    pub semaphore: SemaphoreHandle,
    pub stage_mask: PipelineStageFlags,
    pub value: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SemaphoreSignal {
    pub semaphore: SemaphoreHandle,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ShaderStage {
    #[default]
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShaderDebugInfo {
    pub shader_stage: ShaderStage,
    pub entry_point: String,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CaptureMarker {
    pub capture_id: u64,
    pub frame_number: u64,
    pub timestamp_ns: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MemoryBudget {
    pub total_budget_bytes: u64,
    pub current_usage_bytes: u64,
    pub available_bytes: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ResourceResidency {
    pub resource: ResourceHandle,
    pub resident: bool,
    pub priority: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseButtonState(pub u32);
impl MouseButtonState {
    pub const LEFT: Self = Self(1 << 0);
    pub const RIGHT: Self = Self(1 << 1);
    pub const MIDDLE: Self = Self(1 << 2);
    pub fn empty() -> Self {
        Self(0)
    }
    pub fn contains(self, o: Self) -> bool {
        (self.0 & o.0) == o.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers(pub u32);
impl KeyModifiers {
    pub const SHIFT: Self = Self(1 << 0);
    pub const CTRL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const SUPER: Self = Self(1 << 3);
    pub fn contains(self, o: Self) -> bool {
        (self.0 & o.0) == o.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    Escape,
    Space,
    Enter,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Left,
    Right,
    Up,
    Down,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyboardState {
    pub pressed_keys: Vec<KeyCode>,
    pub modifiers: KeyModifiers,
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchPoint {
    pub id: u64,
    pub position: [f32; 2],
    pub pressure: f32,
    pub radius: [f32; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum XrViewConfiguration {
    #[default]
    PrimaryMono,
    PrimaryStereo,
    SecondaryMono,
}

// ---------- HookContext (§4.2, P1-4 dedup, dyn-compatible) ----------
#[allow(unused_variables)]
pub trait HookContext {
    fn render_pass_name(&self) -> Option<&str> {
        None
    }
    fn render_pass_type(&self) -> RenderPassType {
        RenderPassType::Unknown
    }
    fn subpass_index(&self) -> u32 {
        0
    }
    fn frame_index(&self) -> u64;
    fn prev_frame_index(&self) -> Option<u64> {
        None
    }
    fn frame_time(&self) -> f32 {
        0.0
    }
    fn delta_time(&self) -> f32 {
        0.0
    }
    fn frame_timestamp_ns(&self) -> u64 {
        0
    }
    fn is_first_frame(&self) -> bool {
        false
    }
    fn resource_version(&self) -> u64 {
        0
    }
    fn size(&self) -> (f32, f32);
    fn viewport(&self) -> Rect {
        let (w, h) = self.size();
        Rect::new(0.0, 0.0, w, h)
    }
    fn scissor_rect(&self) -> Option<Rect> {
        self.damage()
    }
    fn damage(&self) -> Option<Rect> {
        None
    }
    fn view_count(&self) -> u32 {
        1
    }
    fn view_index(&self) -> u32 {
        0
    }
    fn is_offscreen(&self) -> bool {
        false
    }
    fn offscreen_pass_index(&self) -> Option<usize> {
        None
    }
    fn render_target_info(&self) -> Option<RenderTargetInfo> {
        None
    }
    fn depth_stencil_info(&self) -> Option<DepthStencilInfo> {
        None
    }
    fn dpi_scale(&self) -> f32 {
        1.0
    }
    fn dpi_scale_xy(&self) -> (f32, f32) {
        let s = self.dpi_scale();
        (s, s)
    }
    fn color_space(&self) -> ColorSpace {
        ColorSpace::Srgb
    }
    fn is_srgb(&self) -> bool {
        true
    }
    fn sample_count(&self) -> u32 {
        1
    }
    fn sample_quality(&self) -> u32 {
        0
    }
    fn view_matrix(&self) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn projection_matrix(&self) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn view_projection_matrix(&self) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn inverse_view_matrix(&self) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn inverse_projection_matrix(&self) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn inverse_view_projection_matrix(&self) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn camera_position(&self) -> Option<[f32; 3]> {
        None
    }
    fn camera_forward(&self) -> Option<[f32; 3]> {
        None
    }
    fn camera_up(&self) -> Option<[f32; 3]> {
        None
    }
    fn camera_near(&self) -> f32 {
        0.1
    }
    fn camera_far(&self) -> f32 {
        1000.0
    }
    fn camera_fov_y(&self) -> f32 {
        core::f32::consts::FRAC_PI_3
    }
    fn camera_aspect(&self) -> f32 {
        let (w, h) = self.size();
        w / h.max(1.0)
    }
    fn graphics_pipeline_id(&self) -> Option<PipelineId> {
        None
    }
    fn compute_pipeline_id(&self) -> Option<PipelineId> {
        None
    }
    fn ray_tracing_pipeline_id(&self) -> Option<PipelineId> {
        None
    }
    fn graphics_pipeline_name(&self) -> Option<&str> {
        None
    }
    fn blend_state(&self) -> Option<BlendState> {
        None
    }
    fn depth_stencil_state(&self) -> Option<DepthStencilState> {
        None
    }
    fn rasterizer_state(&self) -> Option<RasterizerState> {
        None
    }
    fn multisample_state(&self) -> Option<MultisampleState> {
        None
    }
    fn bound_vertex_buffers(&self) -> &[BufferBinding] {
        &[]
    }
    fn bound_index_buffer(&self) -> Option<BufferBinding> {
        None
    }
    fn bound_descriptor_sets(&self) -> &[DescriptorSetBinding] {
        &[]
    }
    fn push_constants(&self) -> &[u8] {
        &[]
    }
    fn bound_samplers(&self) -> &[SamplerBinding] {
        &[]
    }
    fn bound_textures(&self) -> &[TextureBinding] {
        &[]
    }
    fn bound_storage_images(&self) -> &[StorageImageBinding] {
        &[]
    }
    fn bound_storage_buffers(&self) -> &[StorageBufferBinding] {
        &[]
    }
    fn bound_constant_buffers(&self) -> &[ConstantBufferBinding] {
        &[]
    }
    fn bound_acceleration_structures(&self) -> &[AccelerationStructureBinding] {
        &[]
    }
    fn top_level_acceleration_structure(&self) -> Option<AccelerationStructureHandle> {
        None
    }
    fn ray_tracing_pipeline(&self) -> Option<RayTracingPipelineHandle> {
        None
    }
    fn shader_binding_table(&self) -> Option<ShaderBindingTableHandle> {
        None
    }
    fn mesh_pipeline_id(&self) -> Option<PipelineId> {
        None
    }
    fn task_pipeline_id(&self) -> Option<PipelineId> {
        None
    }
    fn vrs_state(&self) -> Option<VrsState> {
        None
    }
    fn shading_rate_image(&self) -> Option<TextureHandle> {
        None
    }
    fn sync_fence(&self) -> Option<&dyn crate::sync::GpuFence> {
        None
    }
    fn wait_fences(&self) -> &[&dyn crate::sync::GpuFence] {
        &[]
    }
    fn signal_fences(&self) -> &[&dyn crate::sync::GpuFence] {
        &[]
    }
    fn wait_semaphores(&self) -> &[SemaphoreWait] {
        &[]
    }
    fn signal_semaphores(&self) -> &[SemaphoreSignal] {
        &[]
    }
    fn debug_label(&self) -> Option<&str> {
        None
    }
    fn pass_debug_label(&self) -> Option<&str> {
        None
    }
    fn command_buffer_debug_label(&self) -> Option<&str> {
        None
    }
    fn gpu_timestamp_ns(&self) -> Option<u64> {
        None
    }
    fn gpu_timer_query(&self) -> Option<TimerQueryHandle> {
        None
    }
    fn capture_marker(&self) -> Option<CaptureMarker> {
        None
    }
    fn shader_debug_info(&self) -> Option<&ShaderDebugInfo> {
        None
    }
    fn memory_budget(&self) -> Option<MemoryBudget> {
        None
    }
    fn resource_residency(&self) -> &[ResourceResidency] {
        &[]
    }
    fn resource_heap(&self) -> Option<HeapHandle> {
        None
    }
    fn transient_heap(&self) -> Option<TransientHeapHandle> {
        None
    }
    fn render_pass_version(&self) -> u64 {
        0
    }
    fn pipeline_cache_version(&self) -> u64 {
        0
    }
    fn mouse_position(&self) -> Option<[f32; 2]> {
        None
    }
    fn mouse_buttons(&self) -> MouseButtonState {
        MouseButtonState::empty()
    }
    fn keyboard_state(&self) -> Option<&KeyboardState> {
        None
    }
    fn touches(&self) -> &[TouchPoint] {
        &[]
    }
    fn xr_session_active(&self) -> bool {
        false
    }
    fn xr_view_configuration(&self) -> XrViewConfiguration {
        XrViewConfiguration::PrimaryMono
    }
    fn xr_view_projection_matrix(&self, _view_index: u32) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn xr_view_matrix(&self, _view_index: u32) -> Option<[[f32; 4]; 4]> {
        None
    }
    fn xr_gaze_point(&self) -> Option<[f32; 3]> {
        None
    }
    fn user_data(&self) -> &dyn Any {
        static E: () = ();
        &E
    }
    fn user_data_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub trait HookContextExt {
    fn downcast_ref<T: 'static>(&self) -> Option<&T>;
    fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T>;
    fn get_user_data<T: 'static>(&self) -> Option<&T>;
    fn get_user_data_mut<T: 'static>(&mut self) -> Option<&mut T>;
}
impl HookContextExt for dyn HookContext + '_ {
    fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }
    fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }
    fn get_user_data<T: 'static>(&self) -> Option<&T> {
        self.user_data().downcast_ref::<T>()
    }
    fn get_user_data_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.user_data_mut()?.downcast_mut::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ctx(u64, (f32, f32));
    impl HookContext for Ctx {
        fn frame_index(&self) -> u64 {
            self.0
        }
        fn size(&self) -> (f32, f32) {
            self.1
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    #[test]
    fn render_source_borrow() {
        let s = RenderSource::new(Some("k"), Some(&123u32));
        assert_eq!(s.resource_key(), Some("k"));
        assert_eq!(s.payload::<u32>(), Some(&123));
        assert_eq!(s.payload::<i32>(), None);
        assert!(RenderSource::none().resource_key().is_none());
    }

    #[test]
    fn render_source_resource_key_selects_imported_payload() {
        // §17/P0-3: hook dispatch by source.kind; imported resource chosen by resource_key.
        // RenderSource carries both: key for lookup identity, payload for imported object.
        let imported = 99u64;
        let src = RenderSource::new(Some("video.frame"), Some(&imported));
        assert_eq!(src.resource_key(), Some("video.frame"));
        assert_eq!(src.payload::<u64>(), Some(&99));
        let no_import = RenderSource::new(Some("missing"), None);
        assert_eq!(no_import.resource_key(), Some("missing"));
        assert!(no_import.payload::<u64>().is_none());
    }

    #[test]
    fn draw_hook_hrtb() {
        let mut hook: Box<DrawHook> = Box::new(|ctx, _r, src| {
            assert_eq!(ctx.frame_index(), 1);
            assert_eq!(src.resource_key(), Some("tex"));
            assert!(ctx.downcast_ref::<Ctx>().is_some());
        });
        let mut ctx = Ctx(1, (10.0, 10.0));
        hook(
            &mut ctx,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            &RenderSource::new(Some("tex"), None),
        );
    }

    #[test]
    fn hook_context_default_matrix_core() {
        let mut ctx = Ctx(7, (1920.0, 1080.0));
        assert_eq!(ctx.frame_index(), 7);
        assert_eq!(ctx.prev_frame_index(), None);
        assert_eq!(ctx.frame_time(), 0.0);
        assert!(!ctx.is_first_frame());
        assert_eq!(ctx.viewport(), Rect::new(0.0, 0.0, 1920.0, 1080.0));
        assert!(!ctx.is_offscreen());
        assert_eq!(ctx.dpi_scale(), 1.0);
        assert_eq!(ctx.sample_count(), 1);
        assert_eq!(ctx.color_space(), ColorSpace::Srgb);
        assert_eq!(ctx.render_pass_type(), RenderPassType::Unknown);
        assert!(ctx.render_target_info().is_none());
        assert!(ctx.depth_stencil_info().is_none());
        assert!(ctx.user_data_mut().is_none());
    }

    #[test]
    fn hook_context_default_matrix_pipeline_bindings() {
        let ctx = Ctx(0, (1.0, 1.0));
        assert!(ctx.graphics_pipeline_id().is_none());
        assert!(ctx.ray_tracing_pipeline_id().is_none());
        assert!(ctx.mesh_pipeline_id().is_none());
        assert!(ctx.blend_state().is_none());
        assert!(ctx.depth_stencil_state().is_none());
        assert!(ctx.rasterizer_state().is_none());
        assert!(ctx.bound_vertex_buffers().is_empty());
        assert!(ctx.bound_index_buffer().is_none());
        assert!(ctx.bound_textures().is_empty());
        assert!(ctx.bound_acceleration_structures().is_empty());
        assert!(ctx.vrs_state().is_none());
        assert!(ctx.sync_fence().is_none());
        assert!(ctx.wait_semaphores().is_empty());
        assert!(ctx.memory_budget().is_none());
        assert!(ctx.shader_debug_info().is_none());
    }

    #[test]
    fn hook_context_full_coverage() {
        // §12/contract: exercise the full default HookContext matrix (no-GPU).
        let ctx = Ctx(9, (640.0, 480.0));
        let _ = ctx.render_pass_name();
        let _ = ctx.render_pass_type();
        let _ = ctx.subpass_index();
        let _ = ctx.prev_frame_index();
        let _ = ctx.frame_time();
        let _ = ctx.delta_time();
        let _ = ctx.frame_timestamp_ns();
        let _ = ctx.is_first_frame();
        let _ = ctx.resource_version();
        let _ = ctx.viewport();
        let _ = ctx.scissor_rect();
        let _ = ctx.damage();
        let _ = ctx.view_count();
        let _ = ctx.view_index();
        let _ = ctx.is_offscreen();
        let _ = ctx.offscreen_pass_index();
        let _ = ctx.render_target_info();
        let _ = ctx.depth_stencil_info();
        let _ = ctx.dpi_scale_xy();
        let _ = ctx.color_space();
        let _ = ctx.is_srgb();
        let _ = ctx.sample_count();
        let _ = ctx.sample_quality();
        let _ = ctx.view_matrix();
        let _ = ctx.projection_matrix();
        let _ = ctx.view_projection_matrix();
        let _ = ctx.inverse_view_matrix();
        let _ = ctx.inverse_projection_matrix();
        let _ = ctx.inverse_view_projection_matrix();
        let _ = ctx.camera_position();
        let _ = ctx.camera_forward();
        let _ = ctx.camera_up();
        let _ = ctx.camera_near();
        let _ = ctx.camera_far();
        let _ = ctx.camera_fov_y();
        let _ = ctx.camera_aspect();
        let _ = ctx.graphics_pipeline_id();
        let _ = ctx.compute_pipeline_id();
        let _ = ctx.ray_tracing_pipeline_id();
        let _ = ctx.graphics_pipeline_name();
        let _ = ctx.blend_state();
        let _ = ctx.depth_stencil_state();
        let _ = ctx.rasterizer_state();
        let _ = ctx.multisample_state();
        let _ = ctx.bound_vertex_buffers();
        let _ = ctx.bound_index_buffer();
        let _ = ctx.bound_descriptor_sets();
        let _ = ctx.push_constants();
        let _ = ctx.bound_samplers();
        let _ = ctx.bound_textures();
        let _ = ctx.bound_storage_images();
        let _ = ctx.bound_storage_buffers();
        let _ = ctx.bound_constant_buffers();
        let _ = ctx.bound_acceleration_structures();
        let _ = ctx.top_level_acceleration_structure();
        let _ = ctx.ray_tracing_pipeline();
        let _ = ctx.shader_binding_table();
        let _ = ctx.mesh_pipeline_id();
        let _ = ctx.task_pipeline_id();
        let _ = ctx.vrs_state();
        let _ = ctx.shading_rate_image();
        let _ = ctx.sync_fence();
        let _ = ctx.wait_fences();
        let _ = ctx.signal_fences();
        let _ = ctx.wait_semaphores();
        let _ = ctx.signal_semaphores();
        let _ = ctx.debug_label();
        let _ = ctx.pass_debug_label();
        let _ = ctx.command_buffer_debug_label();
        let _ = ctx.gpu_timestamp_ns();
        let _ = ctx.gpu_timer_query();
        let _ = ctx.capture_marker();
        let _ = ctx.shader_debug_info();
        let _ = ctx.memory_budget();
        let _ = ctx.resource_residency();
        let _ = ctx.resource_heap();
        let _ = ctx.transient_heap();
        let _ = ctx.render_pass_version();
        let _ = ctx.pipeline_cache_version();
        let _ = ctx.mouse_position();
        let _ = ctx.mouse_buttons();
        let _ = ctx.keyboard_state();
        let _ = ctx.touches();
        let _ = ctx.xr_session_active();
        let _ = ctx.xr_view_configuration();
        let _ = ctx.xr_view_projection_matrix(0);
        let _ = ctx.xr_view_matrix(0);
        let _ = ctx.xr_gaze_point();
        let _ = ctx.user_data();
        let c: &dyn HookContext = &ctx;
        let _ = HookContextExt::downcast_ref::<Ctx>(c);
        let _ = HookContextExt::get_user_data::<()>(c);
    }

    #[test]
    fn hook_context_default_matrix_camera_input_xr() {
        let ctx = Ctx(0, (100.0, 50.0));
        assert!(ctx.view_matrix().is_none());
        assert!(ctx.inverse_view_projection_matrix().is_none());
        assert_eq!(ctx.camera_near(), 0.1);
        assert_eq!(ctx.camera_far(), 1000.0);
        assert!((ctx.camera_aspect() - 2.0).abs() < 1e-6);
        assert!(ctx.mouse_position().is_none());
        assert!(ctx.keyboard_state().is_none());
        assert!(ctx.touches().is_empty());
        assert!(!ctx.xr_session_active());
        assert_eq!(
            ctx.xr_view_configuration(),
            XrViewConfiguration::PrimaryMono
        );
        assert!(ctx.xr_gaze_point().is_none());
    }

    #[test]
    fn type_surface_smoke() {
        let _ = BlendState {
            enabled: true,
            src_color: BlendFactor::SrcAlpha,
            dst_color: BlendFactor::OneMinusSrcAlpha,
            src_alpha: BlendFactor::One,
            dst_alpha: BlendFactor::Zero,
            color_op: BlendOp::Add,
            alpha_op: BlendOp::Add,
            color_write_mask: ColorWriteMask::ALL,
        };
        let _ = DepthStencilState::default();
        let _ = RasterizerState {
            cull_mode: CullMode::None,
            front_face: FrontFace::Clockwise,
            polygon_mode: PolygonMode::Line,
            depth_bias: 0,
            depth_bias_clamp: 0.0,
            slope_scaled_depth_bias: 0.0,
            depth_clip_enabled: true,
            scissor_enabled: false,
            multisample_enabled: false,
            antialiased_line_enabled: false,
            conservative_rasterization: ConservativeRasterMode::Off,
        };
        let _ = VrsState {
            enabled: false,
            shading_rate: ShadingRate::Rate1x1,
            combiners: [VrsCombiner::Passthrough, VrsCombiner::Override],
        };
        let _ = KeyCode::Escape;
        let _ = ShaderStage::Fragment;
        let _ = TextureLayout::ShaderReadOnlyOptimal;
    }

    #[test]
    fn render_extension_contract_mock() {
        use std::collections::HashMap;
        #[derive(Default)]
        struct R {
            g: HashMap<String, Box<DrawHook>>,
            w: HashMap<(u64, String), Box<DrawHook>>,
        }
        impl RenderExtension for R {
            fn register_hook(&mut self, kind: Cow<'static, str>, hook: Box<DrawHook>) {
                self.g.insert(kind.into_owned(), hook);
            }
            fn register_hook_for_window(
                &mut self,
                window: u64,
                kind: Cow<'static, str>,
                hook: Box<DrawHook>,
            ) {
                self.w.insert((window, kind.into_owned()), hook);
            }
            fn unregister_hook(&mut self, kind: &str, window: Option<u64>) -> bool {
                match window {
                    None => self.g.remove(kind).is_some(),
                    Some(w) => self.w.remove(&(w, kind.to_string())).is_some(),
                }
            }
            fn has_hook(&self, kind: &str, window: Option<u64>) -> bool {
                match window {
                    None => self.g.contains_key(kind),
                    Some(w) => self.w.contains_key(&(w, kind.to_string())),
                }
            }
            fn registered_kinds(&self) -> Vec<Cow<'static, str>> {
                self.g.keys().cloned().map(Cow::Owned).collect()
            }
        }
        let mut r = R::default();
        r.register_hook("a".into(), Box::new(|_, _, _| {}));
        r.register_hook_for_window(7, "b".into(), Box::new(|_, _, _| {}));
        assert!(r.has_hook("a", None));
        assert!(r.has_hook("b", Some(7)));
        assert!(r.unregister_hook("b", Some(7)));
        assert!(!r.has_hook("b", Some(7)));
        assert!(r.unregister_hook("a", None));
    }
}
