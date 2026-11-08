//! Instance / Device / Queue / Surface 的生命周期 —— 对应 Windows 那边
//! `zephyra-winit-d3d12::surface` 的角色，但用**裸 Vulkan(ash)** 实现。
//!
//! # 与 wgpu 版的差异
//!
//! wgpu 版（`zephyra-winit-vk::surface`）把设备+队列+swapchain 合在一个 `Gpu`
//! 结构里，因为 wgpu 的 `Device`/`Queue` 是不可分割的一对。裸 Vulkan 这边拆成
//! 进程级 [`Device`]（`ash::Instance` + `ash::Device` + `vk::Queue` + 命令池）与
//! 每窗口 [`Surface`]（`VkSurfaceKHR` + `VkSwapchainKHR` + 格式）——多窗口下
//! 前者只能有一份，后者必须每个窗口一份。
//!
//! # 平台 surface
//!
//! - Linux：**Wayland surface**（WSLg 原生合成路径；实测避开 X11 + libxkbcommon-x11）。
//!   `current_extent` 在 Wayland 上可为 `u32::MAX`，extent 由应用给（`window.inner_size()`）。
//! - Windows：**Win32 surface**（`VK_KHR_win32_surface`），使本 crate 在当前 Windows Vulkan 环境可运行。

use std::fmt;
use std::sync::{Arc, Mutex};

use ash::vk;
use winit::window::Window;

/// 渲染失败。**不带**平台错误码回码 —— 回码属于 `docs/dev-log/`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceError {
    what: &'static str,
    detail: String,
}

impl SurfaceError {
    pub fn new(what: &'static str, detail: impl Into<String>) -> Self {
        Self {
            what,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.what, self.detail)
    }
}

impl std::error::Error for SurfaceError {}

// ─────────────────────────── 验证层（ZEPHYRA_VK_DEBUG=1） ───────────────────────────

/// 验证层报上来的一条消息。
///
/// # 为什么没有「消息 ID」
///
/// D3D12 那边返回 ID 是为了让调用方**按消息**豁免（有的机器上某条消息是已知
/// 无害的）。Vulkan 侧没有等价的稳定数字 ID（`message_id_number` 是各实现自己
/// 编的号，跨层版本会变），所以豁免只能按**文本**做 —— 门禁因此只断言
/// 「ERROR 级为空」，不预设任何豁免。
#[derive(Debug, Clone)]
pub struct DebugMessage {
    pub severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    pub message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    /// 消息 ID 号（各实现自定，仅用于人读）。
    pub id_number: i32,
    pub text: String,
}

/// 验证层的消息收集器。`ZEPHYRA_VK_DEBUG=1` 时挂在 [`Device`] 上。
///
/// # 为什么是「收集」而不是「打印」
///
/// 与 D3D11 / D3D12 的调试层同一条教训：**消息是排队的**，回调错过了就再也没有
/// 第二次。Vulkan 的 `VK_EXT_debug_utils` 信使把消息推给我们的回调，而回调里
/// 只能做「收进来」这一件事 —— 打印交给 `drain`，断言交给门禁。
///
/// # 为什么 sink 是 `Arc<Mutex<..>>`
///
/// 验证层的回调可能来自**任何线程**（驱动自己起的工作线程），且同一进程里
/// 多个设备实例会共享同一个 layer。`Send + Sync` 的收集器是唯一安全形状。
pub struct DebugState {
    sink: Arc<Mutex<Vec<DebugMessage>>>,
    // 下面两个字段「没人读」是**故意的**：本 crate 的 Device 没有 Drop（进程级
    // 对象，统一靠进程退出回收，见 `Device::entry` 的同款处理）。它们存在的意义
    // 是持有句柄 —— 信使必须比 instance 活得短，而这里「谁都不手动销毁」，
    // 于是持有即为保活；真要显式销毁时（例如测试里建销多个设备）也从这两个
    // 字段拿句柄。
    #[allow(dead_code, reason = "只为持有信使句柄（销毁时要经它）")]
    utils: ash::extensions::ext::DebugUtils,
    #[allow(dead_code, reason = "只为持有信使句柄")]
    messenger: vk::DebugUtilsMessengerEXT,
}

impl DebugState {
    /// 取走并清空已收集的消息（与 D3D12 的 `DebugInfo::drain` 同语义）。
    pub fn drain(&self) -> Vec<DebugMessage> {
        // ⚠ 不用 `unwrap()`：回调在 FFI 边界上，panic 穿过去是 abort。
        // 万一持锁的线程正在 panic，也把数据拿出来，别把调用方一起拖下水。
        self.sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect()
    }

    /// ERROR 级消息的文本（门禁断言用；WARNING / INFO 留给 `drain` 看全量）。
    ///
    /// 判据用 `contains` 而不是裸比数字：回调给的就是单独一位 severity，
    /// 而 ash 的 Flags 没公开 `.0`（且「ERROR 及更糟」的方向在 Vulkan 侧
    /// 没有 D3D 那个越发越轻的陷阱 —— 这里 VERBOSE < INFO < WARNING < ERROR，
    /// 回调只会带一位，`contains(ERROR)` 即精确命中）。
    pub fn errors(&self) -> Vec<String> {
        self.drain()
            .into_iter()
            .filter(|m| {
                m.severity
                    .contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
            })
            .map(|m| m.text)
            .collect()
    }

    /// 把已收集的消息打到 stderr（含 INFO / WARNING 全量）。
    pub fn dump(&self) {
        for m in self.drain() {
            eprintln!(
                "[vk] {:?} {:?} #{} {}",
                m.severity, m.message_type, m.id_number, m.text
            );
        }
    }
}

impl fmt::Debug for DebugState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self
            .sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        f.debug_struct("DebugState").field("pending", &n).finish()
    }
}

/// `VK_EXT_debug_utils` 的信使回调。
///
/// ⚠ **绝不能 panic**：它在 FFI 边界上被驱动调用，panic 穿出去是进程 abort。
/// 所以持锁用 `unwrap_or_else(into_inner)`，字符串转换用 `from_utf8_lossy`。
/// 签名是 `unsafe extern "system"`（ash 的 `PFN_vkDebugUtilsMessengerCallbackEXT`
/// 就是这么定的）—— 本函数体自身不碰任何裸指针之外的东西，unsafe 只用于
/// 满足类型。
///
/// 返回 `FALSE`（不抛 `VK_ERROR_VALIDATION_FAILED_EXT`）：本后端的策略是
/// 「收集 + 门禁断言」，不是「第一条错误就炸」—— 后者会把一次 run 的信息量
/// 砍成一条，而排队消息的后半常常才是根因（D3D11 那边同一条教训）。
unsafe extern "system" fn debug_messenger_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    user_data: *mut std::ffi::c_void,
) -> vk::Bool32 {
    if callback_data.is_null() {
        return vk::FALSE;
    }
    let data = unsafe { &*callback_data };
    let text = if data.p_message.is_null() {
        String::new()
    } else {
        // `p_message` 以 NUL 结尾（规范如此），但长度没给 —— 只能自己数。
        let mut n = 0usize;
        while unsafe { *data.p_message.add(n) } != 0 && n < 1 << 16 {
            n += 1;
        }
        String::from_utf8_lossy(unsafe {
            std::slice::from_raw_parts(data.p_message.cast::<u8>(), n)
        })
        .into_owned()
    };
    let msg = DebugMessage {
        severity,
        message_type,
        id_number: data.message_id_number,
        text,
    };
    if !user_data.is_null() {
        // user_data 指向 `DebugState` 建时 `Box::into_raw` 出来的 Arc 克隆。
        let sink = unsafe { &*user_data.cast::<Arc<Mutex<Vec<DebugMessage>>>>() };
        sink.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(msg);
    }
    vk::FALSE
}

/// 进程级 GPU 设备：instance + 物理设备 + 逻辑设备 + 队列 + 命令池。
///
/// # 为什么设备与 surface 分开
///
/// 多窗口下 **surface / swapchain 是每窗口一份**，而 instance / device / queue
/// 全进程只有一份。合在一个 `Device` 里会让第二个窗口只能「再建一个 device」——
/// 那是另一个 GPU 上下文，纹理、管线、描述符全都不共享，跨窗口共享状态就没了。
///
/// `format` 放在 [`Surface`] 而不是这里：管线把格式烤进 `VkPipeline`，而管线是
/// 每窗口一套（见 `Renderer` 的文档），格式跟着 surface 走。
pub struct Device {
    // ⚠⚠ **Entry 必须随 Device 活着**：ash::Entry 持有 libloading::Library（dlopen 的
    // libvulkan 句柄），drop 时 dlclose —— 之后 Instance/Device 函数表里的所有
    // 指针都指向已卸载的代码。症状不是「第一个调用就崩」而是**随机崩**：
    // Linux 的延迟卸载让前面的调用碰巧成功，直到某一次踩到被回收的内存。
    // 本机实测：demo 在「建顶点缓冲的 vkAllocateMemory」处 segfault，而它之前的
    // 五个调用全部成功 —— 与「库已卸载但页还在」的行为完全吻合。
    //
    // （探针不崩的原因：entry 在 main 里，活到进程结束。）
    #[allow(dead_code, reason = "只为持有 libvulkan 的 dlopen 句柄")]
    entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) device: ash::Device,
    pub(crate) queue: vk::Queue,
    pub(crate) physical: vk::PhysicalDevice,
    /// 只为持有所有权（loader 对象必须活到 Instance/Device 之后）。
    #[allow(dead_code, reason = "只为持有所有权")]
    pub(crate) queue_family: u32,
    pub(crate) khr_surface: ash::extensions::khr::Surface,
    /// 只为持有所有权：建 surface 要经它，Instance 销毁之前不能 drop。
    #[cfg(target_os = "linux")]
    #[allow(dead_code, reason = "只为持有所有权")]
    pub(crate) khr_wayland: ash::extensions::khr::WaylandSurface,
    #[cfg(windows)]
    #[allow(dead_code, reason = "只为持有所有权")]
    pub(crate) khr_win32: ash::extensions::khr::Win32Surface,
    pub(crate) khr_swapchain: ash::extensions::khr::Swapchain,
    /// MSAA 采样数。放在设备上而不是窗口上：`ZEPHYRA_MSAA` 是进程级变量，
    /// 让两个窗口取到不同采样数会让共享的图集/纹理无法互认。
    pub(crate) msaa_samples: vk::SampleCountFlags,
    /// 一次性命令池（读回 / 纹理上传等）。与 Device 同寿命。
    pub(crate) command_pool: vk::CommandPool,
    /// 验证层收集器（`ZEPHYRA_VK_DEBUG=1` 时挂上）。
    ///
    /// ⚠ 与两个 Windows 后端同一语义：没开调试层时**不是错误**，只是问不到
    /// 原因 —— 门禁在那种机器上必须能优雅跳过（`debug()` 回 `None`）。
    pub(crate) debug: Option<DebugState>,
}

// ⚠ 手写 `Send`：字段全是「句柄 + 函数表」。Vulkan 允许同一 device / queue 被
// 多线程使用（提交由驱动串行化），而本仓的跨线程访问全部经 `Mutex<Shared>`
// 加锁。`ash` 的扩展结构体里存裸句柄，不会自动 `Send`，这里显式声明。
unsafe impl Send for Device {}

/// 一个窗口的 surface + swapchain。
///
/// # 为什么 `format` 在这里
///
/// 管线把颜色格式烤进 `VkPipeline`（render pass 的附件格式），而 `Renderer` 是
/// **每窗口一份**（MSAA 目标、离屏 RT、顶点缓冲都与窗口尺寸/内容绑定）。格式
/// 跟着 surface 走，就不会出现「两个窗口格式不同却共用管线」。
pub struct Surface {
    /// 销毁 surface 要用 instance（`khr_surface` 内部持有它）。
    khr_surface: ash::extensions::khr::Surface,
    /// 销毁 swapchain 要用 device（`khr_swapchain` 内部持有它）。
    khr_swapchain: ash::extensions::khr::Swapchain,
    pub(crate) surface: vk::SurfaceKHR,
    pub(crate) swapchain: vk::SwapchainKHR,
    pub(crate) images: Vec<vk::Image>,
    pub(crate) format: vk::Format,
    pub(crate) extent: (u32, u32),
    pub(crate) present_mode: vk::PresentModeKHR,
    pub(crate) device_dead: bool,
}

impl fmt::Debug for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Surface")
            .field("format", &self.format)
            .field("extent", &self.extent)
            .field("dead", &self.device_dead)
            .finish()
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // ⚠ 必须先于 `Device` 析构：`HostWindow` 里 `surface` 声明在共享设备之前
        // （字段按声明顺序析构）。顺序反了就是「拿已销毁的 instance 去 destroy」。
        unsafe {
            if self.swapchain != vk::SwapchainKHR::null() {
                self.khr_swapchain.destroy_swapchain(self.swapchain, None);
            }
            if self.surface != vk::SurfaceKHR::null() {
                self.khr_surface.destroy_surface(self.surface, None);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "renderer_tests.rs"]
mod renderer_tests;

impl Device {
    /// 建设备、队列与命令池。
    ///
    /// # 为什么不需要窗口
    ///
    /// 多窗口共享同一个设备，因此选物理设备时**不能**用某个窗口的 surface 去
    /// 校验 present 支持。这里只挑「有 graphics 队列」的物理设备；present 支持
    /// 由 [`Surface::new`] 逐窗口校验（见那里「第二个窗口 present 不了」的注释）。
    ///
    /// `ZEPHYRA_VK_DEBUG=1` 时开 Khronos 验证层（`VK_LAYER_KHRONOS_validation`）
    /// 并挂 `VK_EXT_debug_utils` 信使 —— 与两个 Windows 后端同一约定
    /// （`ZEPHYRA_D3D_DEBUG` / `ZEPHYRA_D3D12_DEBUG`）。层或扩展**装不上时静默
    /// 继续**（`debug` 留 `None`）：没装 LunarG SDK 的机器不该因此起不来，
    /// 只是那条机器上验证门禁会跳过。
    pub fn new() -> Result<Self, SurfaceError> {
        // ── instance ──
        let entry = unsafe { ash::Entry::load() }
            .map_err(|e| SurfaceError::new("dlopen libvulkan", e.to_string()))?;
        let app_name = std::ffi::CString::new("zephyra-vulkan").unwrap();
        // 验证层开关。必须在**建 instance 之前**定：层与扩展都是创建时参数，
        // 事后无法追加（与 D3D11/12 的调试层同一个约束）。
        let debug_on = std::env::var("ZEPHYRA_VK_DEBUG").as_deref() == Ok("1");
        let mut ext_names: Vec<std::ffi::CString> =
            vec![std::ffi::CString::new("VK_KHR_surface").unwrap()];
        #[cfg(target_os = "linux")]
        ext_names.push(std::ffi::CString::new("VK_KHR_wayland_surface").unwrap());
        #[cfg(windows)]
        ext_names.push(std::ffi::CString::new("VK_KHR_win32_surface").unwrap());
        let layer_name = std::ffi::CString::new("VK_LAYER_KHRONOS_validation").unwrap();
        let validation_available = debug_on
            && entry
                .enumerate_instance_layer_properties()
                .map(|ls| {
                    ls.iter().any(|l| {
                        unsafe { std::ffi::CStr::from_ptr(l.layer_name.as_ptr()) }.to_bytes()
                            == b"VK_LAYER_KHRONOS_validation"
                    })
                })
                .unwrap_or(false);
        if debug_on {
            ext_names.push(std::ffi::CString::new("VK_EXT_debug_utils").unwrap());
            if !validation_available {
                eprintln!(
                    "[vk] ZEPHYRA_VK_DEBUG=1 but VK_LAYER_KHRONOS_validation not found; continuing without validation layer"
                );
            }
        }
        let ext_ptrs: Vec<*const std::ffi::c_char> = ext_names.iter().map(|e| e.as_ptr()).collect();
        // ⚠ `ApplicationInfo` 与层指针数组必须是**命名绑定**：builder 借的是它们的
        // 地址，链在临时值上时活不到 `create_instance`（E0716）。
        let app_info = vk::ApplicationInfo::builder()
            .application_name(app_name.as_c_str())
            .application_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::API_VERSION_1_1)
            .build();
        let layer_ptrs = [layer_name.as_ptr()];
        let mut info = vk::InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_extension_names(&ext_ptrs);
        if validation_available {
            info = info.enabled_layer_names(&layer_ptrs);
        }
        let built = info.build();
        let instance = match unsafe { entry.create_instance(&built, None) } {
            Ok(i) => i,
            Err(e) if validation_available => {
                eprintln!("[vk] enabling VK_LAYER_KHRONOS_validation failed ({e}); retrying without validation layer");
                let info2 = vk::InstanceCreateInfo::builder()
                    .application_info(&app_info)
                    .enabled_extension_names(&ext_ptrs)
                    .build();
                unsafe { entry.create_instance(&info2, None) }
                    .map_err(|e2| SurfaceError::new("vkCreateInstance", e2.to_string()))?
            }
            Err(e) => return Err(SurfaceError::new("vkCreateInstance", e.to_string())),
        };

        // ── 验证信使 ──
        //
        // 建在 instance 之后、device 之前：设备创建期的消息同样要收。
        //
        // ⚠ 生命周期：信使必须先于 instance 销毁，而本 crate 的 Device **没有
        // Drop**（进程级对象，统一靠进程退出回收）—— 与既有约定一致，代价是
        // 验证层对象活到进程结束。`sink` 的 Arc 通过 `Box::into_raw` 交给回调，
        // `DebugState` 持有另一份保活；两者都故意泄漏，不参与任何提前释放。
        let debug = if debug_on {
            let utils = ash::extensions::ext::DebugUtils::new(&entry, &instance);
            let sink: Arc<Mutex<Vec<DebugMessage>>> = Arc::new(Mutex::new(Vec::new()));
            // 回调拿到的裸指针：`Arc` 克隆进 Box 再 into_raw，`DebugState` 持有
            // 原 Arc 保活（见上方生命周期注释）。显式标注指针类型 —— `cast()`
            // 的目标推断不出来。
            let user_data: *mut std::ffi::c_void =
                Box::into_raw(Box::new(Arc::clone(&sink))).cast();
            let create = vk::DebugUtilsMessengerCreateInfoEXT::builder()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                        | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(debug_messenger_callback))
                .user_data(user_data)
                .build();
            match unsafe { utils.create_debug_utils_messenger(&create, None) } {
                Ok(messenger) => Some(DebugState {
                    sink,
                    utils,
                    messenger,
                }),
                Err(e) => {
                    // 建不出信使（老驱动没有 debug_utils）就退回「无调试」。
                    // user_data 对应的 Box 此刻已成孤儿 —— 回收它，别泄漏。
                    drop(unsafe {
                        Box::from_raw(user_data.cast::<Arc<Mutex<Vec<DebugMessage>>>>())
                    });
                    eprintln!("[vk] 建不出 debug messenger（{e}）—— 验证层静默关闭");
                    None
                }
            }
        } else {
            None
        };

        // ── 物理设备 + 队列 ──
        let physical = unsafe { instance.enumerate_physical_devices() }
            .map_err(|e| SurfaceError::new("enumerate_physical_devices", e.to_string()))?
            .into_iter()
            .find(|p| {
                unsafe { instance.get_physical_device_queue_family_properties(*p) }
                    .iter()
                    .any(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS) && f.queue_count > 0)
            })
            .ok_or_else(|| SurfaceError::new("no physical device", "no graphics queue"))?;
        let queue_family = unsafe { instance.get_physical_device_queue_family_properties(physical) }
            .iter()
            .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS) && f.queue_count > 0)
            .unwrap() as u32;
        let swapchain_name = std::ffi::CString::new("VK_KHR_swapchain").unwrap();
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::builder()
                    .queue_create_infos(&[vk::DeviceQueueCreateInfo::builder()
                        .queue_family_index(queue_family)
                        .queue_priorities(&[1.0])
                        .build()])
                    .enabled_extension_names(&[swapchain_name.as_ptr()])
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateDevice", e.to_string()))?;
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        // MSAA 由环境变量 ZEPHYRA_MSAA 控制（与两个 Windows 后端同一约定）。
        let msaa_samples = if std::env::var("ZEPHYRA_MSAA").as_deref() == Ok("0") {
            vk::SampleCountFlags::TYPE_1
        } else {
            vk::SampleCountFlags::TYPE_4
        };

        // 命令池（读回 / 纹理上传等一次性命令用）。
        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::builder()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("command pool", e.to_string()))?;

        // 扩展函数表：内部只 clone 句柄，必须在 `entry` / `instance` 被移进
        // 结构体之前构造。
        let khr_surface = ash::extensions::khr::Surface::new(&entry, &instance);
        #[cfg(target_os = "linux")]
        let khr_wayland = ash::extensions::khr::WaylandSurface::new(&entry, &instance);
        #[cfg(windows)]
        let khr_win32 = ash::extensions::khr::Win32Surface::new(&entry, &instance);
        let khr_swapchain = ash::extensions::khr::Swapchain::new(&instance, &device);

        Ok(Self {
            entry,
            instance,
            device,
            queue,
            physical,
            queue_family,
            khr_surface,
            #[cfg(target_os = "linux")]
            khr_wayland,
            #[cfg(windows)]
            khr_win32,
            khr_swapchain,
            msaa_samples,
            command_pool,
            debug,
        })
    }

    /// 验证层收集器（没开 `ZEPHYRA_VK_DEBUG=1` 或装不上层时为 `None`）。
    ///
    /// 与 D3D12 的 `Gpu::debug()` 同一形状：门禁靠它判断「这台机器能不能
    /// 问驱动原因」，拿不到就跳过而不是恒真。
    pub fn debug(&self) -> Option<&DebugState> {
        self.debug.as_ref()
    }
}

impl Surface {
    /// 为 `window` 建 surface + swapchain。
    ///
    /// # 为什么在这里校验 present 支持
    ///
    /// 设备是**跨窗口共享**的（[`Device::new`] 只按 graphics 队列挑物理设备），
    /// 所以「这台物理设备能不能向这个窗口的 surface 呈现」只能逐窗口问。
    /// 不校验的症状是：第二个窗口 `vkCreateSwapchainKHR` 静默失败，或者更糟 ——
    /// 创建成功但 present 一直返回 out-of-date，画面停在黑色。
    pub fn new(device: &Device, window: &Window) -> Result<Self, SurfaceError> {
        let surface = create_platform_surface(device, window)?;

        // ⚠ 共享设备按 graphics 队列挑，而 present 支持是 **surface 相关**的：
        // 一个物理设备有 graphics 队列，不代表它能向这个窗口呈现。
        let supported = unsafe {
            device.khr_surface.get_physical_device_surface_support(
                device.physical,
                device.queue_family,
                surface,
            )
        }
        .map_err(|e| SurfaceError::new("surface support", e.to_string()))?;
        if !supported {
            unsafe { device.khr_surface.destroy_surface(surface, None) };
            return Err(SurfaceError::new(
                "surface support",
                "physical device cannot present to this window",
            ));
        }

        let caps = unsafe {
            device
                .khr_surface
                .get_physical_device_surface_capabilities(device.physical, surface)
        }
        .map_err(|e| SurfaceError::new("surface capabilities", e.to_string()))?;
        if std::env::var("ZEPHYRA_VK_DEBUG").as_deref() == Ok("1") {
            eprintln!(
                "[zephyra-vulkan] caps current={}x{} min={}x{} max={}x{} transform={:?}",
                caps.current_extent.width,
                caps.current_extent.height,
                caps.min_image_extent.width,
                caps.min_image_extent.height,
                caps.max_image_extent.width,
                caps.max_image_extent.height,
                caps.current_transform
            );
        }
        let (mut w, mut h) = if caps.current_extent.width == u32::MAX {
            let s = window.inner_size();
            (s.width.max(1), s.height.max(1))
        } else {
            let cw = caps.current_extent.width;
            let ch = caps.current_extent.height;
            // Guard against bogus driver-reported extents (observed on some Windows ICDs).
            let sane = cw > 0 && ch > 0 && cw <= 16384 && ch <= 16384;
            if sane {
                (cw, ch)
            } else {
                let s = window.inner_size();
                eprintln!(
                    "[zephyra-vulkan] surface current_extent {}x{} looks invalid; using window {}x{} (caps min {}x{} max {}x{})",
                    cw, ch, s.width, s.height,
                    caps.min_image_extent.width, caps.min_image_extent.height,
                    caps.max_image_extent.width, caps.max_image_extent.height
                );
                (s.width.max(1), s.height.max(1))
            }
        };
        if w == 0 || h == 0 {
            unsafe { device.khr_surface.destroy_surface(surface, None) };
            return Err(SurfaceError::new("surface extent", format!("{w}x{h}")));
        }
        let max_w = {
            let m = caps.max_image_extent.width;
            if m == 0 || m > 16384 {
                16384
            } else {
                m
            }
        };
        let max_h = {
            let m = caps.max_image_extent.height;
            if m == 0 || m > 16384 {
                16384
            } else {
                m
            }
        };
        let min_w = caps.min_image_extent.width.max(1).min(max_w);
        let min_h = caps.min_image_extent.height.max(1).min(max_h);
        w = w.clamp(min_w, max_w);
        h = h.clamp(min_h, max_h);
        let formats = unsafe {
            device
                .khr_surface
                .get_physical_device_surface_formats(device.physical, surface)
        }
        .map_err(|e| SurfaceError::new("surface formats", e.to_string()))?;
        // ⚠ 1:1 的关键取舍：优先 **UNORM（非 sRGB）**，与 Windows D3D12 同一行为。
        //
        // 实测（WSL2/llvmpipe，2026-10）：sRGB 附件按规范被硬件**自动编码** ——
        // 着色器写线性值 16，swapchain 落 71（= encode(16)），屏幕观感比 Windows
        // 亮一档。Windows 那边的交换链是 B8G8R8A8_UNORM（非 sRGB），着色器值
        // 直通。要「输出一致」，两边必须同一种直通行为 —— 所以这里选 UNORM。
        let chosen = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .or_else(|| {
                formats
                    .iter()
                    .find(|f| f.format == vk::Format::R8G8B8A8_UNORM)
            })
            .or_else(|| {
                formats
                    .iter()
                    .find(|f| f.format == vk::Format::B8G8R8A8_SRGB)
            })
            .ok_or_else(|| SurfaceError::new("surface format", "no supported format"))?;
        let format = chosen.format;
        let color_space = chosen.color_space;
        if !caps
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_SRC)
        {
            return Err(SurfaceError::new(
                "swapchain usage",
                "TRANSFER_SRC not supported (needed for frame dump)",
            ));
        }
        let alpha = if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
        {
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
        } else if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
        {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        } else {
            vk::CompositeAlphaFlagsKHR::INHERIT
        };
        let mut min_image_count = caps.min_image_count.max(2);
        if caps.max_image_count > 0 && min_image_count > caps.max_image_count {
            min_image_count = caps.max_image_count;
        }
        let mut pre_transform = caps.current_transform;
        if !caps.supported_transforms.contains(pre_transform) {
            pre_transform = vk::SurfaceTransformFlagsKHR::IDENTITY;
        }
        let create_info = vk::SwapchainCreateInfoKHR::builder()
            .surface(surface)
            .min_image_count(min_image_count)
            .image_format(format)
            .image_color_space(color_space)
            .image_extent(vk::Extent2D {
                width: w,
                height: h,
            })
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(pre_transform)
            .composite_alpha(alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .build();
        let swapchain = unsafe { device.khr_swapchain.create_swapchain(&create_info, None) }
            .map_err(|e| {
                SurfaceError::new(
                    "vkCreateSwapchainKHR",
                    format!(
                        "{e}; {w}x{h} fmt={format:?} cs={color_space:?} images={min_image_count} usage={:?} alpha={alpha:?} pre={pre_transform:?}",
                        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC
                    ),
                )
            })?;
        let images = unsafe { device.khr_swapchain.get_swapchain_images(swapchain) }
            .map_err(|e| SurfaceError::new("get_swapchain_images", e.to_string()))?;

        Ok(Self {
            khr_surface: ash::extensions::khr::Surface::new(&device.entry, &device.instance),
            khr_swapchain: ash::extensions::khr::Swapchain::new(&device.instance, &device.device),
            surface,
            swapchain,
            images,
            format,
            extent: (w, h),
            present_mode: vk::PresentModeKHR::FIFO,
            device_dead: false,
        })
    }

    /// 重新创建 swapchain（窗口 resize 后调用）。
    pub fn resize(&mut self, device: &Device, window: &Window) -> Result<(), SurfaceError> {
        // 销毁旧 swapchain。
        unsafe {
            self.khr_swapchain.destroy_swapchain(self.swapchain, None);
        }
        let (w, h) = {
            let s = window.inner_size();
            (s.width, s.height)
        };
        if w == 0 || h == 0 {
            return Ok(());
        }
        let caps = unsafe {
            self.khr_surface
                .get_physical_device_surface_capabilities(device.physical, self.surface)
        }
        .map_err(|e| SurfaceError::new("surface capabilities", e.to_string()))?;
        let (w, h) = if caps.current_extent.width == u32::MAX {
            (w, h)
        } else {
            (caps.current_extent.width, caps.current_extent.height)
        };
        self.extent = (w, h);
        self.swapchain = unsafe {
            self.khr_swapchain.create_swapchain(
                &vk::SwapchainCreateInfoKHR::builder()
                    .surface(self.surface)
                    .min_image_count(caps.min_image_count.max(2))
                    .image_format(self.format)
                    .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                    .image_extent(vk::Extent2D {
                        width: w,
                        height: h,
                    })
                    .image_array_layers(1)
                    .image_usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    )
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(pick_composite_alpha(caps.supported_composite_alpha))
                    .present_mode(self.present_mode)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateSwapchainKHR", e.to_string()))?;
        self.images = unsafe { self.khr_swapchain.get_swapchain_images(self.swapchain) }
            .map_err(|e| SurfaceError::new("get_swapchain_images", e.to_string()))?;
        Ok(())
    }

    /// 取得下一张交换链图并等待 acquire fence。
    pub fn acquire(&self, device: &Device) -> Result<(u32, vk::Semaphore), SurfaceError> {
        let sem = unsafe {
            device
                .device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        }
        .map_err(|e| SurfaceError::new("create semaphore", e.to_string()))?;
        let (idx, _) = unsafe {
            self.khr_swapchain
                .acquire_next_image(self.swapchain, u64::MAX, sem, vk::Fence::null())
        }
        .map_err(|e| SurfaceError::new("acquire_next_image", e.to_string()))?;
        Ok((idx, sem))
    }

    /// 提交命令缓冲并 present。
    pub fn submit_and_present(
        &self,
        device: &Device,
        cmd: vk::CommandBuffer,
        wait_sem: vk::Semaphore,
        signal_sem: vk::Semaphore,
        image_idx: u32,
        fence: vk::Fence,
    ) -> Result<(), SurfaceError> {
        unsafe {
            device.device.reset_fences(&[fence]).ok();
            device
                .device
                .queue_submit(
                    device.queue,
                    &[vk::SubmitInfo::builder()
                        .wait_semaphores(&[wait_sem])
                        .wait_dst_stage_mask(&[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT])
                        .command_buffers(&[cmd])
                        .signal_semaphores(&[signal_sem])
                        .build()],
                    fence,
                )
                .ok();
            device.device.wait_for_fences(&[fence], true, u64::MAX).ok();
            self.khr_swapchain
                .queue_present(
                    device.queue,
                    &vk::PresentInfoKHR::builder()
                        .wait_semaphores(&[signal_sem])
                        .swapchains(&[self.swapchain])
                        .image_indices(&[image_idx])
                        .build(),
                )
                .ok();
        }
        Ok(())
    }

    /// 读回一张交换链图的像素（用于 `ZEPHYRA_FRAME_DUMP`）。
    ///
    /// 与 D3D12 那份「MSAA 再 resolve 一次」同构：这里读的是**交换链图**，
    /// 而它只在 `PRESENT_SRC` 布局下可读 —— 所以先转到 `TRANSFER_SRC` 再拷。
    pub fn read_back(
        &self,
        device: &Device,
        image: vk::Image,
        w: u32,
        h: u32,
    ) -> Result<Vec<u8>, SurfaceError> {
        let byte_len = (w * h * 4) as vk::DeviceSize;
        let buffer = unsafe {
            device.device.create_buffer(
                &vk::BufferCreateInfo::builder()
                    .size(byte_len)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("readback buffer", e.to_string()))?;
        let breq = unsafe { device.device.get_buffer_memory_requirements(buffer) };
        let bmem = unsafe {
            device.device.allocate_memory(
                &vk::MemoryAllocateInfo::builder()
                    .allocation_size(breq.size)
                    .memory_type_index(pick_type(
                        &device.instance,
                        device.physical,
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )?)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("readback memory", e.to_string()))?;
        unsafe { device.device.bind_buffer_memory(buffer, bmem, 0) }.ok();

        let cmd = unsafe {
            device.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::builder()
                    .command_pool(device.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1)
                    .build(),
            )
        }
        .map_err(|e| SurfaceError::new("readback cmd", e.to_string()))?[0];

        unsafe {
            device
                .device
                .begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::builder()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                        .build(),
                )
                .ok();
            // swapchain 图当前在 PRESENT_SRC；转到 TRANSFER_SRC 才能拷。
            let bar = vk::ImageMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::MEMORY_READ)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .build();
            device.device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[bar],
            );
            device.device.cmd_copy_image_to_buffer(
                cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[vk::BufferImageCopy::builder()
                    .image_subresource(
                        vk::ImageSubresourceLayers::builder()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_array_layer(0)
                            .layer_count(1)
                            .build(),
                    )
                    .image_extent(vk::Extent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    })
                    .build()],
            );
            // ⚠ 拷完必须转回 PRESENT_SRC：调用方紧接着要 present，而布局回不去
            // 的话后续 present 是非法状态。漏掉的症状是「第一次 dump 后窗口黑屏」。
            let sub = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };
            let back = vk::ImageMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ)
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(sub)
                .build();
            let backs = [back];
            device.device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &backs,
            );
            device.device.end_command_buffer(cmd).ok();
        }
        let fence = unsafe {
            device
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|e| SurfaceError::new("readback fence", e.to_string()))?;
        unsafe {
            device
                .device
                .queue_submit(
                    device.queue,
                    &[vk::SubmitInfo::builder().command_buffers(&[cmd]).build()],
                    fence,
                )
                .ok();
            device.device.wait_for_fences(&[fence], true, u64::MAX).ok();
        }
        let ptr = unsafe {
            device
                .device
                .map_memory(bmem, 0, byte_len, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| SurfaceError::new("map_memory", e.to_string()))?;
        let data =
            unsafe { std::slice::from_raw_parts(ptr as *const u8, byte_len as usize) }.to_vec();
        // 清理（读回是一次性的，每次都建/销）。
        unsafe {
            device.device.unmap_memory(bmem);
            device.device.destroy_fence(fence, None);
            device
                .device
                .free_command_buffers(device.command_pool, &[cmd]);
            device.device.destroy_buffer(buffer, None);
            device.device.free_memory(bmem, None);
        }
        Ok(data)
    }
}

fn create_platform_surface(
    device: &Device,
    window: &Window,
) -> Result<vk::SurfaceKHR, SurfaceError> {
    #[cfg(target_os = "linux")]
    {
        let (dpy, wl_surface) =
            wayland_handles(window).map_err(|e| SurfaceError::new("wayland handles", e))?;
        unsafe {
            device.khr_wayland.create_wayland_surface(
                &vk::WaylandSurfaceCreateInfoKHR::builder()
                    .display(dpy)
                    .surface(wl_surface)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateWaylandSurfaceKHR", e.to_string()))
    }
    #[cfg(windows)]
    {
        let (hwnd, hinstance) =
            win32_handles(window).map_err(|e| SurfaceError::new("win32 handles", e))?;
        unsafe {
            device.khr_win32.create_win32_surface(
                &vk::Win32SurfaceCreateInfoKHR::builder()
                    .hinstance(hinstance)
                    .hwnd(hwnd)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateWin32SurfaceKHR", e.to_string()))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (device, window);
        Err(SurfaceError::new(
            "platform surface",
            "unsupported platform",
        ))
    }
}

#[cfg(target_os = "linux")]
fn wayland_handles(
    window: &Window,
) -> Result<(*mut std::ffi::c_void, *mut std::ffi::c_void), String> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let dpy = match window.display_handle().map_err(|e| e.to_string())?.as_raw() {
        RawDisplayHandle::Wayland(h) => h.display.as_ptr(),
        _ => return Err("winit did not give a Wayland display handle".into()),
    };
    let surface = match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
        RawWindowHandle::Wayland(h) => h.surface.as_ptr(),
        _ => return Err("winit did not give a Wayland surface handle".into()),
    };
    Ok((dpy, surface))
}

#[cfg(windows)]
fn win32_handles(window: &Window) -> Result<(vk::HINSTANCE, vk::HWND), String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
        RawWindowHandle::Win32(h) => {
            let hwnd = h.hwnd.get() as vk::HWND;
            let hinstance = match h.hinstance {
                Some(i) => i.get() as vk::HINSTANCE,
                None => get_module_handle(),
            };
            Ok((hinstance, hwnd))
        }
        _ => Err("winit did not give a Win32 window handle".into()),
    }
}

#[cfg(windows)]
fn get_module_handle() -> vk::HINSTANCE {
    // Win32: GetModuleHandleW(NULL) returns the module handle for the current process.
    // Vulkan requires a valid HINSTANCE for Win32 surface creation.
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lpModuleName: *const u16) -> *mut std::ffi::c_void;
    }
    unsafe { GetModuleHandleW(std::ptr::null()) }
}

pub(crate) fn pick_type(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    want: vk::MemoryPropertyFlags,
) -> Result<u32, SurfaceError> {
    let props = unsafe { instance.get_physical_device_memory_properties(physical) };
    (0..props.memory_type_count)
        .find(|i| {
            props.memory_types[*i as usize]
                .property_flags
                .contains(want)
        })
        .ok_or_else(|| SurfaceError::new("memory type", format!("no type with {want:?}")))
}

fn pick_composite_alpha(supported: vk::CompositeAlphaFlagsKHR) -> vk::CompositeAlphaFlagsKHR {
    if supported.contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED) {
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
    } else if supported.contains(vk::CompositeAlphaFlagsKHR::OPAQUE) {
        vk::CompositeAlphaFlagsKHR::OPAQUE
    } else if supported.contains(vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED) {
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED
    } else {
        vk::CompositeAlphaFlagsKHR::INHERIT
    }
}
