use super::*;
use crate::renderer::{HookCtx, RenderCtx, RenderHook, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::wayland::EventLoopBuilderExtWayland;
use winit::window::Window;
use zephyra_core::core::Rect;
use zephyra_core::filter::BlendMode;
use zephyra_core::filter::FilterChain;
use zephyra_core::frame::Damage;
use zephyra_core::paint::{Command, DisplayList, Layer, RenderSource, SortKey};

const KIND: &str = "test:fill-red";
const BOUNDS: Rect = Rect::new(10.0, 11.0, 7.0, 9.0);

struct FillRed;

impl RenderHook for FillRed {
    fn draw(&mut self, ctx: HookCtx<'_>, bounds: Rect, source: &RenderSource) {
        assert_eq!(source.payload::<u32>(), Some(&42));
        let attachments = [vk::ClearAttachment {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            color_attachment: 0,
            clear_value: vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [1.0, 0.0, 0.0, 1.0],
                },
            },
        }];
        let rects = [vk::ClearRect {
            rect: vk::Rect2D {
                offset: vk::Offset2D {
                    x: bounds.x as i32,
                    y: bounds.y as i32,
                },
                extent: vk::Extent2D {
                    width: bounds.width as u32,
                    height: bounds.height as u32,
                },
            },
            base_array_layer: 0,
            layer_count: 1,
        }];
        let scissor = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: 1,
                height: 1,
            },
        }];
        unsafe {
            ctx.device.cmd_set_scissor(ctx.cmd, 0, &scissor);
            ctx.device
                .cmd_clear_attachments(ctx.cmd, &attachments, &rects);
        }
    }
}

#[test]
#[ignore = "requires a Wayland Vulkan device and VK_LAYER_KHRONOS_validation"]
fn custom_hook_pixels_and_command_validation() {
    std::env::set_var("ZEPHYRA_VK_DEBUG", "1");
    eprintln!("[vk-test] building event loop");
    let mut event_loop_builder = EventLoop::builder();
    event_loop_builder.with_any_thread(true);
    let event_loop = event_loop_builder.build().expect("create event loop");
    event_loop
        .run_app(&mut ValidationApp {
            window: None,
            tested: false,
        })
        .expect("run Wayland validation test");
}

struct ValidationApp {
    window: Option<Window>,
    tested: bool,
}

impl ApplicationHandler for ValidationApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        eprintln!("[vk-test] creating Wayland window");
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_inner_size(PhysicalSize::new(64, 64))
                    .with_title("zephyra-vulkan-render-hook-test"),
            )
            .expect("create Wayland window");
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if self.tested
            || !matches!(
                event,
                WindowEvent::Resized(_) | WindowEvent::RedrawRequested
            )
        {
            return;
        }
        self.tested = true;
        if let Some(window) = &self.window {
            run_render_hook_test(window);
        }
        event_loop.exit();
    }
}

fn run_render_hook_test(window: &Window) {
    eprintln!("[vk-test] creating Vulkan device with validation layer");
    let gpu = Gpu::new(window).expect("create Vulkan device");
    eprintln!("[vk-test] Vulkan device and swapchain ready");
    let debug = gpu
        .debug()
        .expect("validation layer and VK_EXT_debug_utils must be available");

    validation_layer_reverse_control(debug);
    eprintln!("[vk-test] reverse control passed");
    let size = gpu.extent;
    assert!(size.0 > 20 && size.1 > 20, "surface is too small: {size:?}");
    let mut renderer = Renderer::new(&gpu, 64, gpu.msaa_samples.as_raw()).expect("create renderer");
    renderer.register_hook(KIND, Box::new(FillRed));
    let framebuffer_views = create_framebuffer_views(&gpu, &mut renderer);
    eprintln!("[vk-test] renderer and framebuffers ready: {size:?}");

    let registered = custom_list(KIND);
    let (report, image_index) = render_one(&gpu, &mut renderer, &registered, &framebuffer_views);
    assert_eq!(report.custom_passes, 1);
    assert_eq!(report.unhandled_custom, 0);
    let pixels = read_last_frame(&gpu, image_index, size);
    assert_pixel_counts(
        &gpu,
        &pixels,
        BOUNDS.width as usize * BOUNDS.height as usize,
    );
    assert_no_validation_errors(debug, "registered hook frame");
    eprintln!("[vk-test] registered hook pixel check passed");

    let unregistered = custom_list("test:not-registered");
    let (report, image_index) = render_one(&gpu, &mut renderer, &unregistered, &framebuffer_views);
    assert_eq!(report.custom_passes, 0);
    assert_eq!(report.unhandled_custom, 1);
    let pixels = read_last_frame(&gpu, image_index, size);
    assert_pixel_counts(&gpu, &pixels, 0);
    assert_no_validation_errors(debug, "unregistered hook frame");
    eprintln!("[vk-test] unregistered hook pixel check passed");

    destroy_framebuffer_views(&gpu, framebuffer_views);
}

fn validation_layer_reverse_control(debug: &crate::surface::DebugState) {
    assert_no_validation_errors(debug, "before reverse control");
    let message = vk::DebugUtilsMessengerCallbackDataEXT::builder()
        .message_id_name(c"zephyra_reverse_control")
        .message_id_number(-1)
        .message(c"Vulkan debug callback reverse-control probe")
        .build();
    unsafe {
        debug.utils.submit_debug_utils_message(
            vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION,
            &message,
        )
    };
    let errors = debug.errors();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("reverse-control probe")),
        "debug callback failed the reverse control: {errors:#?}"
    );
}

fn custom_list(kind: &str) -> DisplayList {
    let mut list = DisplayList::new();
    list.push(
        SortKey {
            layer: Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Custom {
            bounds: BOUNDS,
            source: RenderSource::new(kind.to_owned(), 42_u32),
            filter: FilterChain::default(),
            blend: BlendMode::SrcOver,
        },
    );
    list
}

fn create_framebuffer_views(
    gpu: &Gpu,
    renderer: &mut Renderer,
) -> Vec<(vk::ImageView, vk::Framebuffer)> {
    let extent = gpu.extent;
    let msaa_view = renderer.ensure_msaa(gpu, extent);
    gpu.images
        .iter()
        .map(|&image| {
            let view = unsafe {
                gpu.device
                    .create_image_view(
                        &vk::ImageViewCreateInfo::builder()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(gpu.format)
                            .subresource_range(vk::ImageSubresourceRange {
                                aspect_mask: vk::ImageAspectFlags::COLOR,
                                base_mip_level: 0,
                                level_count: 1,
                                base_array_layer: 0,
                                layer_count: 1,
                            }),
                        None,
                    )
                    .expect("create swapchain image view")
            };
            let attachments = msaa_view.map_or_else(|| vec![view], |msaa| vec![msaa, view]);
            let framebuffer = unsafe {
                gpu.device
                    .create_framebuffer(
                        &vk::FramebufferCreateInfo::builder()
                            .render_pass(renderer.msaa_render_pass)
                            .attachments(&attachments)
                            .width(extent.0)
                            .height(extent.1)
                            .layers(1),
                        None,
                    )
                    .expect("create swapchain framebuffer")
            };
            (view, framebuffer)
        })
        .collect()
}

fn destroy_framebuffer_views(gpu: &Gpu, views: Vec<(vk::ImageView, vk::Framebuffer)>) {
    unsafe {
        for (view, framebuffer) in views {
            gpu.device.destroy_framebuffer(framebuffer, None);
            gpu.device.destroy_image_view(view, None);
        }
        gpu.device.device_wait_idle().expect("wait for GPU");
    }
}

fn render_one(
    gpu: &Gpu,
    renderer: &mut Renderer,
    list: &DisplayList,
    framebuffers: &[(vk::ImageView, vk::Framebuffer)],
) -> (zephyra_fx::RenderReport, u32) {
    let report = renderer
        .prepare(
            gpu,
            &RenderCtx {
                display_list: list,
                size: gpu.extent,
                damage: Damage::Full,
                atlas_pixels: &[],
                atlas_size: 64,
                atlas_revision: 0,
                filters_enabled: true,
                default_blend: BlendMode::SrcOver,
            },
        )
        .expect("prepare frame");
    let (image_index, wait_sem) = gpu.acquire().expect("acquire swapchain image");
    let cmd = unsafe {
        gpu.device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::builder()
                    .command_pool(gpu.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .expect("allocate frame command buffer")[0]
    };
    let (_, framebuffer) = framebuffers[image_index as usize];
    unsafe {
        gpu.device
            .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())
            .expect("begin frame command buffer");
    }
    renderer.record_frame(gpu, cmd, framebuffer, list, BlendMode::SrcOver);
    unsafe {
        gpu.device
            .end_command_buffer(cmd)
            .expect("end frame command buffer");
    }
    let signal_sem = unsafe {
        gpu.device
            .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            .expect("create render semaphore")
    };
    let fence = unsafe {
        gpu.device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .expect("create render fence")
    };
    gpu.submit_and_present(cmd, wait_sem, signal_sem, image_index, fence)
        .expect("submit and present frame");
    unsafe {
        gpu.device.device_wait_idle().expect("wait for render");
        gpu.device.free_command_buffers(gpu.command_pool, &[cmd]);
        gpu.device.destroy_fence(fence, None);
        gpu.device.destroy_semaphore(wait_sem, None);
        gpu.device.destroy_semaphore(signal_sem, None);
    }
    (report, image_index)
}

fn read_last_frame(gpu: &Gpu, image_index: u32, size: (u32, u32)) -> Vec<u8> {
    gpu.read_back(gpu.images[image_index as usize], size.0, size.1)
        .expect("read back swapchain image")
}

fn assert_pixel_counts(gpu: &Gpu, bytes: &[u8], expected_red: usize) {
    let (red, green, blue) = match gpu.format {
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => (0, 1, 2),
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => (2, 1, 0),
        format => panic!("unexpected swapchain format: {format:?}"),
    };
    let mut red_pixels = 0;
    let mut black_pixels = 0;
    for pixel in bytes.chunks_exact(4) {
        if pixel[red] == 255 && pixel[green] == 0 && pixel[blue] == 0 {
            red_pixels += 1;
        }
        if pixel[red] == 0 && pixel[green] == 0 && pixel[blue] == 0 {
            black_pixels += 1;
        }
    }
    assert_eq!(red_pixels, expected_red, "red pixel count");
    assert_eq!(
        black_pixels,
        bytes.len() / 4 - expected_red,
        "all non-red pixels must be black"
    );
}

fn assert_no_validation_errors(debug: &crate::surface::DebugState, phase: &str) {
    let errors = debug.errors();
    assert!(
        errors.is_empty(),
        "{phase} produced Vulkan errors: {errors:#?}"
    );
}
