//! 窗口过程子类 —— 接管 `WM_GETOBJECT`。
//!
//! # `WM_GETOBJECT` 是 UIA 的入口
//!
//! UIA 客户端（屏幕阅读器、Inspect、自动化脚本）拿到一个窗口后，第一件事是
//! 发 `WM_GETOBJECT`，`lParam` 填 `UIA_ROOT_ELEMENT_ID`（-25），问「这个窗口
//! 的元素树从哪儿拿」。返回值的类型必须是 `IRawElementProviderSimple*`
//! （或 `IAccessible*`）。
//!
//! 所以这一层要做的只有一件事：**在 `WM_GETOBJECT` 时把根提供者交出去**，
//! 别的消息一律转给原过程。
//!
//! # 为什么自己子类化，而不是让 winit 代劳
//!
//! winit 0.30 没有 UIA 钩子；而 UIA 的入口**必须**是窗口过程（`WM_GETOBJECT`
//! 是发给窗口的消息，没有别的投递途径）。于是只能子类化 —— 与本仓既有的
//! 实时 resize 子类同一手法（`zephyra-platform-win` 的 `live_resize`）。

use std::sync::Mutex;

use windows::Win32::Foundation::{GetLastError, SetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::{IRawElementProviderSimple, UiaRootObjectId};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, WNDPROC,
};

/// `WM_GETOBJECT` —— UIA 客户端问「给我你的提供者」。
const WM_GETOBJECT: u32 = 0x003D;

/// 每个窗口的原过程。
///
/// **按 hwnd 存表**而不是 `GWL_USERDATA`：`GWL_USERDATA` 已经被 winit 用着
/// （它把 `WindowData` 挂在那儿），抢它会直接把 winit 的状态踩坏。
static PREV: Mutex<Vec<(isize, WNDPROC)>> = Mutex::new(Vec::new());

/// 查某个窗口的原过程。
fn previous_proc(hwnd: HWND) -> WNDPROC {
    let key = hwnd.0 as isize;
    let table = PREV.lock().unwrap_or_else(|e| e.into_inner());
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or(None)
}

/// 我们的窗口过程。
///
/// # SAFETY
///
/// 由 [`install`] 设到窗口上，由 [`uninstall`] 摘掉。除 `WM_GETOBJECT` 外
/// 一律转给原过程，所以原过程的返回值语义不变。
unsafe extern "system" fn uia_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // `UiaRootObjectId` 是 -25（`UIA_ROOT_ELEMENT_ID`）。`LPARAM` 的载荷是
    // `isize`，常量表里是 `i32`，比之前显式转。
    if msg == WM_GETOBJECT && lparam.0 == UiaRootObjectId as isize {
        let provider = crate::provider::root_provider();
        // ⚠ 必须走 `UiaReturnRawElementProvider`，**不能**把裸指针直接返回。
        //
        // 客户端（屏幕阅读器、Inspect）在**别的进程**里，它拿到的是本进程地址
        // 空间里的一个指针 —— 直接返回裸指针，客户端没法用它跨进程调用，
        // 于是它会**静默地退回默认代理**：窗口还在，但它下面的控件一个都不
        // 存在（`FindAll` 返回 0 个元素）。
        //
        // `UiaReturnRawElementProvider` 做两件事：把提供者按 UIA 的规则挂上去，
        // 并返回**封送后**的指针，客户端才跨得过去。
        //
        // 症状极隐蔽：`WM_GETOBJECT` 明明进了我们的窗口过程（加日志能看见），
        // 但客户端从头到尾**不查询**提供者 —— 看起来像「UIA 装上了但没效果」。
        return unsafe {
            windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(
                hwnd, wparam, lparam, &provider,
            )
        };
    }
    CallWindowProcW(previous_proc(hwnd), hwnd, msg, wparam, lparam)
}

/// 把窗口挂上我们的窗口过程。
///
/// # 与别的子类化的顺序
///
/// 本仓的实时 resize 也子类化同一个槽位。**UIA 要装在最后** —— 那样消息链
/// 是 `UIA → live_resize → winit`，两者都工作。
pub fn install(hwnd: isize) -> Result<(), crate::Error> {
    let h = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        // `SetWindowLongPtrW` 返回 0 既可能是「成功且原过程就是 0」，也可能是
        // 失败。判据用 `GetLastError`：文档说成功时它**不**被重置，所以先清。
        SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
        let prev = SetWindowLongPtrW(h, GWLP_WNDPROC, uia_proc as *const () as isize);
        if prev == 0 {
            let err = GetLastError();
            if err.0 != 0 {
                return Err(crate::Error::WindowProc(err.0));
            }
        }
        // `isize -> fn ptr_of` 不是「非原始类型转换」，Rust 不允许 `as`。
        let prev = std::mem::transmute::<isize, WNDPROC>(prev);
        let mut table = PREV.lock().unwrap_or_else(|e| e.into_inner());
        table.retain(|(k, _)| *k != hwnd);
        table.push((hwnd, prev));
    }
    Ok(())
}

/// 摘掉窗口过程（窗口销毁前调）。
pub fn uninstall(hwnd: isize) {
    let prev = {
        let mut table = PREV.lock().unwrap_or_else(|e| e.into_inner());
        table
            .iter()
            .position(|(k, _)| *k == hwnd)
            .map(|i| table.remove(i).1)
    };
    if let Some(p) = prev {
        if p.is_some() {
            let h = HWND(hwnd as *mut std::ffi::c_void);
            unsafe {
                let raw = std::mem::transmute::<WNDPROC, isize>(p);
                SetWindowLongPtrW(h, GWLP_WNDPROC, raw);
            }
        }
    }
    // 微软文档要求：窗口销毁时通知 UIA「这个窗口的提供者可以安全移除」。
    //
    // > "When a window that previously returned providers has been destroyed,
    // you should notify UI Automation by calling the UiaReturnRawElementProvider
    // function as follows: `UiaReturnRawElementProvider(hwnd, 0, 0, NULL)`.
    // This call tells UI Automation that it can safely remove all map entries
    // that refer to the specified window. … Microsoft recommends making this
    // call from the WM_DESTROY message handler of the window that returns the
    // UI Automation providers."
    //
    // 不做的话，UIA 的「提供者已抛事件」映射表会一直攥着我们的提供者 ——
    // 引用计数降不下去，对象泄漏；而且表里的条目指向一个已经没了的窗口。
    let h = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        let _ = windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(
            h,
            WPARAM(0),
            LPARAM(0),
            None::<&IRawElementProviderSimple>,
        );
    }
}

// ------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    /// 三个测试共用**同一份**全局 `PREV` 表 ⇒ 必须串行。
    ///
    /// 这与 `zephyra-platform-win` 的实时 resize 测试同理由：被测对象本身就是
    /// 进程级单例（按 hwnd 存表的全局），串行化是对被测语义的忠实反映。
    static SERIAL: Mutex<()> = Mutex::new(());

    /// 一个真实的消息窗口（窗口过程是 `DefWindowProcW`）。
    ///
    /// # 为什么用真窗口而不是直接调 `uia_proc`
    ///
    /// `uia_proc` 的 `WM_GETOBJECT` 分支会调 `CallWindowProcW(previous_proc(hwnd), …)`，
    /// 而那需要真实的 hwnd 才能不炸。用真窗口还顺带验了**整条链路**：
    /// `SendMessageW` → 我们的窗口过程 → 交出提供者 —— 这正是 UIA 客户端走的路。
    fn make_window() -> Option<isize> {
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, RegisterClassW, WNDCLASSW};

        // 宽字符串常量（PCWSTR 要 `*const u16`，不能用字符串字面量）。
        const CLASS: &[u16] = &[
            90u16, 101, 112, 104, 121, 114, 97, 85, 73, 65, 84, 101, 115, 116, 0,
        ];
        /// 窗口标题，宽字符（"t\0"）。
        static TITLE_BUF: [u16; 2] = [116, 0];
        let title = windows::core::PCWSTR(TITLE_BUF.as_ptr());

        // SAFETY：`hwnd` 由本函数建、由调用方负责；窗口过程是 `DefWindowProcW`。
        unsafe extern "system" fn passthrough(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(h, m, w, l)
        }

        unsafe {
            let hinst = GetModuleHandleW(None).ok()?;
            let cls = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(passthrough),
                hInstance: HINSTANCE(hinst.0),
                lpszClassName: windows::core::PCWSTR(CLASS.as_ptr()),
                ..Default::default()
            });
            // 「类已存在」**不是**失败：多个测试用**同一个类名**，而 cargo test
            // 默认并行 —— 第二个注册时必然撞上 `ERROR_CLASS_ALREADY_EXISTS`。
            if cls == 0
                && windows::Win32::Foundation::GetLastError()
                    != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
            {
                return None;
            }
            let hwnd = match CreateWindowExW(
                Default::default(),
                windows::core::PCWSTR(CLASS.as_ptr()),
                title,
                windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(HINSTANCE(hinst.0)),
                None,
            ) {
                Ok(h) => h,
                Err(_) => return None,
            };
            if hwnd.0.is_null() {
                return None;
            }
            Some(hwnd.0 as isize)
        }
    }

    /// 往窗口发一条消息，拿返回值。
    fn send(hwnd: isize, msg: u32, lparam: isize) -> LRESULT {
        // SAFETY：hwnd 是活窗口。
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                HWND(hwnd as *mut std::ffi::c_void),
                msg,
                Some(WPARAM(0)),
                Some(LPARAM(lparam)),
            )
        }
    }

    /// 发一份探针快照：窗口根 + 一个复选框。
    ///
    /// 有孩子才验得了「树」：客户端拿到的元素就是窗口根，`FindAll` 找的是
    /// **后代** —— 没有后代就永远只能验到一个元素。
    fn publish_minimal_snapshot() {
        use crate::model::{Element, Role, Snapshot, ToggleState};
        let mut snap = Snapshot::default();
        snap.elements.push(Element {
            id: 0,
            parent: None,
            key: String::new(),
            name: "probe".into(),
            role: Role::Window,
            toggle: None,
            enabled: true,
            focused: false,
            offscreen: false,
            rect: zephyra_core::core::Rect::new(0.0, 0.0, 100.0, 100.0),
            children: vec![1],
        });
        snap.elements.push(Element {
            id: 1,
            parent: Some(0),
            key: "check:alpha".into(),
            name: "Enable feature".into(),
            role: Role::CheckBox,
            toggle: Some(ToggleState::On),
            enabled: true,
            focused: false,
            offscreen: false,
            rect: zephyra_core::core::Rect::new(10.0, 20.0, 22.0, 22.0),
            children: vec![],
        });
        crate::publish(snap);
    }

    /// `WM_GETOBJECT` + `UIA_ROOT_ELEMENT_ID` 必须交出**一个连接 key**。
    ///
    /// 这是 UIA 客户端拿到一个窗口后的**第一件事** —— 不交出连接，整棵无障碍
    /// 树就不存在，屏读器会念「空白」。
    ///
    /// # ⚠ 返回值**不是指针**
    ///
    /// 微软文档（`UiaReturnRawElementProvider` 的 Return value）：
    ///
    /// > "The key for the client process to connect to the server process
    /// > through UI Automation."
    ///
    /// 它是**给客户端进程用的连接票据**，不是 `IRawElementProviderSimple*`。
    /// 把它当指针解引用 ⇒ 访问违规（我们踩过一次）。
    #[test]
    fn wm_getobject_hands_out_the_root_provider() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("装上 UIA 窗口过程");
        publish_minimal_snapshot();

        let ret = send(hwnd, WM_GETOBJECT, UiaRootObjectId as isize);
        assert_ne!(ret.0, 0, "必须交出一个非零的连接 key");
        // ⚠ **不要**把 `ret.0` 当指针解引用 —— 它是 key，不是指针。
        // （拿它去读 vtable ⇒ 访问违规。）

        uninstall(hwnd);
    }

    /// 对照组：**原生 Win32 复选框**能不能通过同一套客户端 API 拿到 Toggle 图案。
    ///
    /// 这是定位「到底是我们提供者的锅、还是客户端调用姿势的锅」的**分水岭**：
    /// - 原生控件能拿到 ⇒ 客户端 API 没问题，问题在我们的提供者；
    /// - 原生控件也拿不到 ⇒ 是我们调用客户端 API 的姿势不对。
    #[test]
    fn a_native_checkbox_gives_its_toggle_pattern_to_a_uia_client() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, RegisterClassW, BS_CHECKBOX, WNDCLASSW, WS_VISIBLE,
        };

        // SAFETY：窗口过程是 `DefWindowProcW`。
        unsafe extern "system" fn passthrough(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(h, m, w, l)
        }

        // 宽字符串常量（PCWSTR 要 `*const u16`，不能用字符串字面量）。
        static CLASS: &[u16] = &[
            90u16, 101, 112, 104, 121, 114, 97, 85, 73, 65, 84, 101, 115, 116, 0,
        ];
        /// 原生按钮类名，宽字符（"BUTTON\0"）。
        static BTN_CLASS: &[u16] = &[66, 85, 84, 84, 79, 78, 0];
        /// 原生按钮标题，宽字符（"native\0"）。
        static BTN_TITLE: &[u16] = &[110, 97, 116, 105, 118, 101, 0];

        let hwnd = unsafe {
            let hinst = GetModuleHandleW(None).expect("拿不到模块句柄");
            let cls = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(passthrough),
                hInstance: HINSTANCE(hinst.0),
                lpszClassName: windows::core::PCWSTR(CLASS.as_ptr()),
                ..Default::default()
            });
            if cls == 0
                && windows::Win32::Foundation::GetLastError()
                    != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
            {
                panic!("注册窗口类失败");
            }
            // 原生按钮类自带 Toggle 图案 —— 正好当对照组。

            CreateWindowExW(
                Default::default(),
                windows::core::PCWSTR(BTN_CLASS.as_ptr()),
                windows::core::PCWSTR(BTN_TITLE.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    WS_VISIBLE.0 | BS_CHECKBOX as u32,
                ),
                0,
                0,
                20,
                20,
                None,
                None,
                Some(HINSTANCE(hinst.0)),
                None,
            )
            .expect("建不出原生按钮")
        };

        // SAFETY：COM 公寓只在本测试线程里用，成对初始化/反初始化。
        unsafe {
            let hr = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
            assert!(hr.0 == 0 || hr.0 == 1, "COM 初始化失败：{:#x}", hr.0);

            let autom: windows::Win32::UI::Accessibility::IUIAutomation =
                windows::Win32::System::Com::CoCreateInstance(
                    &windows::Win32::UI::Accessibility::CUIAutomation8,
                    None::<&windows::core::IUnknown>,
                    windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
                )
                .expect("应当能创建 UIA 客户端对象");

            let element = autom
                .ElementFromHandle(hwnd)
                .expect("应当能从窗口句柄拿到元素");
            let toggle: windows::Win32::UI::Accessibility::IUIAutomationTogglePattern = element
                .GetCurrentPatternAs(windows::Win32::UI::Accessibility::UIA_TogglePatternId)
                .expect("原生复选框应当有 Toggle 图案");
            assert_eq!(
                toggle.CurrentToggleState().expect("应当有状态").0,
                0,
                "原生复选框默认是未勾选（ToggleState = Off）"
            );
            windows::Win32::System::Com::CoUninitialize();
        }
    }

    /// 端到端：**真正的 UIA 客户端 API** 能从这个窗口拿到我们的元素树。
    ///
    /// 走的是 `IUIAutomation::ElementFromHandle` —— 与屏幕阅读器、
    /// Inspect.exe 完全同一条路：UIA 核心发 `WM_GETOBJECT` → 拿连接 key →
    /// 接上我们的提供者 → 把元素树交给客户端。
    ///
    /// 这条比「发个消息看返回值」强得多：它验的是**整条链路**，而不是
    /// 「我们的窗口过程有没有被叫到」。
    #[test]
    fn a_uia_client_can_read_our_tree_from_the_window_handle() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("装上 UIA 窗口过程");
        publish_minimal_snapshot();

        // SAFETY：COM 公寓只在本测试线程里用，成对初始化/反初始化。
        unsafe {
            let hr = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
            assert!(
                hr.0 == 0 || hr.0 == 1,
                "COM 初始化失败：{:#x}（1 = S_FALSE，本来就初始化过）",
                hr.0
            );

            let autom: windows::Win32::UI::Accessibility::IUIAutomation =
                windows::Win32::System::Com::CoCreateInstance(
                    &windows::Win32::UI::Accessibility::CUIAutomation8,
                    None::<&windows::core::IUnknown>,
                    windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
                )
                .expect("应当能创建 UIA 客户端对象");

            let element = autom
                .ElementFromHandle(HWND(hwnd as *mut std::ffi::c_void))
                .expect("应当能从窗口句柄拿到元素");
            let name = element.CurrentName().expect("元素应当有名字");
            assert_eq!(name.to_string(), "probe", "读到的应当是我们报的名字");

            // 元素树里应当能按控件树视图找到我们的复选框。
            let condition = autom.CreateTrueCondition().expect("应当能造条件");
            let found = element
                .FindAll(
                    windows::Win32::UI::Accessibility::TreeScope_Descendants,
                    &condition,
                )
                .expect("应当能遍历");
            assert_eq!(
                found.Length().expect("应当能数出元素"),
                1,
                "树上应当正好有一个后代（那个复选框）"
            );

            // 复选框的角色、名字、状态，全部走**真正的 UIA 客户端**读一遍。
            let check = found.GetElement(0).expect("应当能取到第一个后代");
            assert_eq!(
                check.CurrentName().expect("复选框应当有名字").to_string(),
                "Enable feature"
            );
            assert_eq!(
                check.CurrentControlType().expect("应当有角色").0,
                windows::Win32::UI::Accessibility::UIA_CheckBoxControlTypeId.0,
                "角色必须是 CheckBox"
            );
            assert_eq!(
                check
                    .CurrentAutomationId()
                    .expect("应当有 AutomationId")
                    .to_string(),
                "check:alpha"
            );

            // ⭐ 图案：客户端从元素上 QI 出 Toggle 图案，读它的状态。
            //
            // 这正是之前**取不到**的那一环 —— 元素、名字、角色都在，唯独
            // `GetCurrentPatternAs` 报「没有这个图案」。
            let toggle: windows::Win32::UI::Accessibility::IUIAutomationTogglePattern = check
                .GetCurrentPatternAs(windows::Win32::UI::Accessibility::UIA_TogglePatternId)
                .expect("复选框应当有 Toggle 图案");
            assert_eq!(
                toggle.CurrentToggleState().expect("应当有状态").0,
                1,
                "勾上了 ⇒ ToggleState = On(1)"
            );

            // 客户端自己也算得出「这个元素支持 Toggle 图案」。
            let available = check
                .GetCurrentPropertyValue(
                    windows::Win32::UI::Accessibility::UIA_IsTogglePatternAvailablePropertyId,
                )
                .expect("应当读得到这个属性");
            assert_eq!(
                windows::Win32::System::Variant::VARIANT::from(true),
                available,
                "客户端应当认为这个元素支持 Toggle 图案"
            );

            windows::Win32::System::Com::CoUninitialize();
        }

        uninstall(hwnd);
    }

    /// 别的消息必须**原样**转给原过程 —— 我们只多管了 `WM_GETOBJECT` 一个。
    ///
    /// 这条守住「子类化没有把窗口搞坏」。子类化最常见的事故就是忘了转消息，
    /// 症状是「某类操作无声失效」，极难归因。
    #[test]
    fn other_messages_forward_to_the_original_procedure() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("装上 UIA 窗口过程");

        // `WM_NCCALCSIZE` 的返回值由 `DefWindowProcW` 决定（通常 0）——
        // 关键不是它的值，而是**它不是我们那个提供者指针**。
        let ret = send(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::WM_NCCALCSIZE,
            0,
        );
        let provider_ptr =
            windows::core::Interface::as_raw(&crate::provider::root_provider()) as isize;
        assert_ne!(
            ret.0, provider_ptr,
            "非 WM_GETOBJECT 的消息不该返回提供者指针"
        );

        uninstall(hwnd);
    }

    /// 摘掉子类之后，`WM_GETOBJECT` 就不再由我们接管。
    #[test]
    fn uninstall_stops_answering_wm_getobject() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("装上 UIA 窗口过程");
        let before = send(hwnd, WM_GETOBJECT, UiaRootObjectId as isize);
        assert_ne!(before.0, 0);

        uninstall(hwnd);
        let after = send(hwnd, WM_GETOBJECT, UiaRootObjectId as isize);
        assert_eq!(
            after.0, 0,
            "摘掉之后应当回到原过程的行为（DefWindowProc 对 WM_GETOBJECT 返回 0）"
        );
    }
}
