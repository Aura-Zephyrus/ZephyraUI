//! UIA 的 COM 提供者 —— 把 [`Snapshot`]（纯数据）翻译成 UIA 认识的形状。
//!
//! # 为什么是「原始提供者」（raw provider）而不是虚拟代理
//!
//! UIA 有两套接入方式：
//!
//! - **虚拟代理**（virtual proxy）：自己实现一整套 `IUIAutomationXxx` 元素接口
//!   （几十个接口、每个几十个方法），UIA 拿它当真实控件；
//! - **原始提供者**（raw provider）：只实现 `IRawElementProviderSimple` +
//!   `IRawElementProviderFragment` 两个接口，其余（属性、图案）由 UIA 的代理层
//!   向我们**问**。
//!
//! 原始提供者是对的选择：本仓的控件是**每帧重建的无状态模板**，没有
//! 「一个元素对象」可以长期持有 —— 虚拟代理那套「元素对象持有状态」的前提
//! 压根不成立。原始提供者的模型是「**你问，我答**」，正好对上「每帧拍一份
//! 快照，谁问都答快照里的那份」。
//!
//! # 元素怎么认：按 key，不按快照下标
//!
//! 提供者对象是**长期存在**的（客户端拿到指针就攥着），而快照是**每帧换**
//! 的。所以提供者不能把快照「焊」在手里 —— 那样它会一直答旧值：用户勾了
//! 复选框，我们抛了「状态变了」事件，客户端回头来问，拿到的是**没勾**那
//! 份 —— 屏读器于是念了一个错的状态。
//!
//! 所以提供者只记**元素的稳定身份**（控件的 `key`），每次被问就去读**当前**
//! 快照。元素从树上消失时（换页了）属性查询报「不支持」—— 那正是 UIA 对
//! 「元素已不存在」的约定。
//!
//! # 一个对象，什么接口都自己实现
//!
//! [`ElementProvider`] 一个 COM 对象，同时实现五个接口：
//!
//! - `IRawElementProviderSimple` —— 属性（名字、角色、状态…）；
//! - `IRawElementProviderFragment` —— 片段导航（父子兄弟、矩形、焦点）；
//! - `IRawElementProviderFragmentRoot` —— 片段根的能力（按点找元素、当前焦点）；
//! - `IToggleProvider` —— 开关状态与切换动作；
//! - `IInvokeProvider` —— 「按一下」的动作。
//!
//! ⚠ 后两个是**原始提供者这一侧**的接口（SDK `UIAutomationCore.h`），
//! **不是** `IUIAutomationTogglePattern` / `IUIAutomationInvokePattern` ——
//! 后者是**客户端侧**接口（UIA 自己实现），IID 不同，混了会静默失效。
//!
//! # 为什么图案接口要长在元素**自己**身上
//!
//! 早先是「另造一个图案对象，`GetPatternProvider` 返回它」。**跨进程用不了**
//! —— UIA 拿到一个元素提供者之后，图案接口得从**同一个对象**上 QI 才拿得到；
//! 另造一个对象意味着 UIA 得把它**封送**回客户端，而原始提供者通道不做这件事。
//! 症状很有迷惑性：元素在、名字在、状态在，唯独图案取不到
//! （客户端 `GetCurrentPatternAs` 报「没有这个图案」）。
//!
//! 于是图案接口改由**元素对象自己**实现，`GetPatternProvider` 交出的就是
//! 它自己。（Flutter 的 Windows 无障碍实现同款做法。）

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use windows::core::{implement, ComObject, ComObjectInterface, Error, IUnknown, Interface};
use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayCreateVector, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0_0, VARIANT_0_0_0, VT_ARRAY, VT_I4, VT_R8,
};
use windows::Win32::UI::Accessibility::{
    IInvokeProvider, IInvokeProvider_Impl, IRawElementProviderFragment,
    IRawElementProviderFragmentRoot, IRawElementProviderFragmentRoot_Impl,
    IRawElementProviderFragment_Impl, IRawElementProviderSimple, IRawElementProviderSimple_Impl,
    ISelectionItemProvider, ISelectionItemProvider_Impl, IToggleProvider, IToggleProvider_Impl,
    NavigateDirection, NavigateDirection_FirstChild, NavigateDirection_LastChild,
    NavigateDirection_NextSibling, NavigateDirection_Parent, NavigateDirection_PreviousSibling,
    ProviderOptions, ProviderOptions_RefuseNonClientSupport, ProviderOptions_ServerSideProvider,
    ProviderOptions_UseComThreading, ToggleState as UiaToggleState, UIA_AutomationIdPropertyId,
    UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_ClassNamePropertyId, UIA_ControlTypePropertyId, UIA_FrameworkIdPropertyId,
    UIA_GroupControlTypeId, UIA_HasKeyboardFocusPropertyId, UIA_InvokePatternId,
    UIA_IsContentElementPropertyId, UIA_IsControlElementPropertyId, UIA_IsEnabledPropertyId,
    UIA_IsInvokePatternAvailablePropertyId, UIA_IsKeyboardFocusablePropertyId,
    UIA_IsOffscreenPropertyId, UIA_IsSelectionItemPatternAvailablePropertyId,
    UIA_IsTogglePatternAvailablePropertyId, UIA_NamePropertyId, UIA_NativeWindowHandlePropertyId,
    UIA_ProcessIdPropertyId, UIA_ProviderDescriptionPropertyId, UIA_RadioButtonControlTypeId,
    UIA_SelectionItemIsSelectedPropertyId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
    UIA_ToggleToggleStatePropertyId, UIA_WindowControlTypeId, UiaAppendRuntimeId,
    UiaHostProviderFromHwnd, UiaRect,
};

use crate::model::{Element, Role, Snapshot, ToggleState};

/// 提供者自报家门用的名字（`UIA_ProviderDescriptionPropertyId`）。
const PROVIDER_DESCRIPTION: &str = "ZephyraUI";

/// 无障碍诊断开关（环境变量 `ZEPHYRA_UIA_TRACE=1`）。
///
/// # 为什么要有它
///
/// UIA 是「**客户端来问、服务端答**」的模型 —— 客户端问什么、答得对不对，
/// 全在对面进程里。出问题时界面上一片正常，屏读器却念不出任何东西，
/// 而服务端这边**什么错都没报**。没有这层日志就只能靠猜。
///
/// 打开后每次客户端调用都会打印一行，形如
/// `[uia] Navigate dir=3 self=check:alpha -> check:beta`。
///
/// 默认**关**着 —— 它会往 stderr 打日志，不该在正常运行时刷屏。
pub fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ZEPHYRA_UIA_TRACE").is_ok_and(|v| v != "0"))
}

/// 打印一行诊断（`trace_enabled()` 为假时什么都不做）。
macro_rules! trace {
    ($($arg:tt)*) => {
        if $crate::provider::trace_enabled() {
            eprintln!("[uia] {}", format_args!($($arg)*));
        }
    };
}

/// 一个「空」的接口指针（COM 的 NULL）。
///
/// # 为什么需要它
///
/// UIA 的契约里有几处「**没有**这个东西」必须表达成「**成功 + 出参为 NULL**」，
/// 而不是错误码：
///
/// | 方法 | 没有时该怎样 |
/// |---|---|
/// | `Navigate` | 那个方向没有元素 ⇒ `S_OK` + NULL |
/// | `HostRawElementProvider` | 这个元素不是片段根 ⇒ NULL |
/// | `ElementProviderFromPoint` | 点上什么都没有 ⇒ NULL |
/// | `GetFocus` | 片段里没人有焦点 ⇒ NULL |
///
/// 报 `E_FAIL` 会被 UIA 当成「**出错了**」—— 遍历当场中断，而不是「走到头了」。
/// 这两件事在 UIA 眼里完全不同。
///
/// # ⚠ Rust 侧别让它被析构
///
/// COM 侧没有这个问题：`#[implement]` 生成的桩把返回值**移进**出参
/// （`pretval.write(transmute(ok__))`），不会析构；UIA 看到 NULL 就停，
/// 也不会去 Release。
///
/// 但**纯 Rust 调用方**（我们的单元测试）通过 windows-rs 的包装方法拿到它
/// 之后，`Type::from_abi` 会把空指针变成 `Err(Error::empty())`（码是 `S_OK`=0）
/// —— 那是**客户端包装层**的翻译，不是 COM 层的失败。要断言 COM 层的事
/// （「成功 + NULL」），测试得**打 vtable**（见测试里的 `navigate_raw`）。
fn null_interface<T: windows::core::Interface>() -> T {
    let raw: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY：`windows` 生成的接口类型都是 `#[repr(transparent)]` 包一个指针
    // （大小与 `*mut c_void` 相同）；`transmute_copy` 只拷位、不解引用。
    let iface = std::mem::ManuallyDrop::new(unsafe {
        std::mem::transmute_copy::<*mut core::ffi::c_void, T>(&raw)
    });
    // SAFETY：`ManuallyDrop` 不析构里面的值；这里只是把同样的位模式读出来，
    // 还给调用者（调用者也不会去析构它 —— 它是空指针）。
    unsafe { std::ptr::read(&*iface) }
}

/// 自报家门的框架名（`UIA_FrameworkIdPropertyId`）。
///
/// 「哪个 UI 框架造的」—— 调试时一眼看出数据是谁给的。
const FRAMEWORK_ID: &str = "Zephyra";

/// 角色对应的类名（`UIA_ClassNamePropertyId`）。
///
/// UIA 的惯例：类名是**框架自己的**分类名（`Button`、`CheckBox`…），
/// 不是控件的 key —— 客户端用它做样式匹配。
fn class_name(role: Role) -> &'static str {
    match role {
        Role::CheckBox => "CheckBox",
        Role::Button => "Button",
        // 开关在 UIA 里**没有**自己的 ControlType（只有 CheckBox），
        // 但类名是我们自己的分类名 —— 报 `Switch` 让 Inspect 里能把它
        // 与真正的复选框区分开。
        Role::Switch => "Switch",
        Role::RadioButton => "RadioButton",
        Role::Group => "Group",
        Role::Window => "Window",
    }
}

/// 一个元素的提供者。
///
/// 同时实现五个接口：
///
/// - `IRawElementProviderSimple` —— 属性（名字、角色、状态…）；
/// - `IRawElementProviderFragment` —— 片段导航（父子兄弟、矩形、焦点）；
/// - `IRawElementProviderFragmentRoot` —— 片段根的能力（按点找元素、当前焦点）；
/// - `IToggleProvider` —— 开关状态与切换动作；
/// - `IInvokeProvider` —— 「按一下」的动作。
///
/// ⚠ 后两个是**原始提供者这一侧**的接口，不是客户端侧的
/// `IUIAutomationTogglePattern` / `IUIAutomationInvokePattern` —— IID 不同。
///
/// # 为什么必须实现第三个
///
/// UIA 的客户端拿到一个原始提供者后，要先把「片段根」解析出来，才知道这棵树
/// 的根是谁、才能往下遍历。`FragmentRoot()` 报「不支持」的话，客户端会**当场
/// 放弃整棵树** —— 表现是 `FindAll` 返回 0 个元素、属性一个都读不到。
///
/// 这不是「锦上添花的接口」：它是**能不能被遍历**的分水岭。
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IToggleProvider,
    IInvokeProvider,
    ISelectionItemProvider
)]
struct ElementProvider {
    /// 元素身份：`None` = 窗口根，`Some(key)` = 按控件 key 认的那个元素。
    key: Option<String>,
}

impl ElementProvider {
    /// 在**当前**快照里找到本元素，然后跑一段闭包。
    ///
    /// 元素不在树上时返回 `None`（调用方据此报「不支持」）。
    fn with_element<R>(&self, f: impl FnOnce(&Element) -> R) -> Option<R> {
        let snapshot = crate::current()?;
        let el = match &self.key {
            None => snapshot.root_element()?,
            Some(k) => snapshot.find_by_key(k)?,
        };
        Some(f(el))
    }

    /// 角色的 UIA ControlType 值。
    fn control_type(role: Role) -> i32 {
        match role {
            Role::CheckBox => UIA_CheckBoxControlTypeId.0,
            // ⚠ UIA 的 ControlType 表里**没有** Switch —— 开关只能报成
            // CheckBox（外加 TogglePattern）。这是平台限制，见 `Role::Switch`
            // 的文档。
            Role::Switch => UIA_CheckBoxControlTypeId.0,
            Role::RadioButton => UIA_RadioButtonControlTypeId.0,
            Role::Button => UIA_ButtonControlTypeId.0,
            Role::Group => UIA_GroupControlTypeId.0,
            Role::Window => UIA_WindowControlTypeId.0,
        }
    }
}

/// 造一个元素的提供者（返回 `IRawElementProviderSimple`）。
///
/// `key = None` ⇒ 窗口根；`Some(key)` ⇒ 按控件 key 认的那个元素。
///
/// # 为什么公开
///
/// UIA 客户端拿到的是 `IRawElementProviderSimple*` —— 而那个指针只能从
/// `WM_GETOBJECT` 的返回值或**别的提供者**的 `Navigate` 里来。
///
/// 本函数让「造一个元素提供者」这件事可以被集成测试与将来的宿主代码直接用，
/// 而不必去碰 crate 内部的私有函数。
pub fn element_provider(key: Option<String>) -> IRawElementProviderSimple {
    let obj = ComObject::new(ElementProvider { key });
    obj.cast::<IRawElementProviderSimple>()
        .expect("ElementProvider 必然实现 IRawElementProviderSimple")
}

/// 造一个元素的片段提供者。
pub fn element_fragment_provider(key: Option<String>) -> IRawElementProviderFragment {
    let obj = ComObject::new(ElementProvider { key });
    obj.cast::<IRawElementProviderFragment>()
        .expect("ElementProvider 必然实现 IRawElementProviderFragment")
}

/// 造一个元素的片段根提供者。
///
/// 片段根 = 窗口根。它实现 `IRawElementProviderFragmentRoot`，所以能回答
/// 「按点找元素」「当前焦点在哪」。
pub fn element_fragment_root_provider(key: Option<String>) -> IRawElementProviderFragmentRoot {
    let obj = ComObject::new(ElementProvider { key });
    obj.cast::<IRawElementProviderFragmentRoot>()
        .expect("ElementProvider 必然实现 IRawElementProviderFragmentRoot")
}

/// 造一个元素的提供者（返回 `IRawElementProviderSimple`）。
fn provider_for(key: Option<String>) -> IRawElementProviderSimple {
    element_provider(key)
}

/// 造一个元素的片段提供者。
fn fragment_for(key: Option<String>) -> IRawElementProviderFragment {
    element_fragment_provider(key)
}

/// 片段根提供者（窗口根）。
///
/// **必须**是缓存里那**同一个** COM 对象 —— UIA 靠指针认出「这是同一个
/// 片段根」，每次现造一个会让客户端以为换了根。
fn fragment_root_for() -> IRawElementProviderFragmentRoot {
    root_provider()
        .cast::<IRawElementProviderFragmentRoot>()
        .expect("根提供者同时实现 IRawElementProviderFragmentRoot")
}

impl ElementProvider_Impl {
    /// 本元素的 key（根元素没有 ⇒ `None`）。
    fn element_key(&self) -> Option<String> {
        let key = self.key.as_ref()?;
        if key.is_empty() {
            return None;
        }
        Some(key.clone())
    }

    /// 本元素当前的开关状态（没有 Toggle 图案 ⇒ `None`）。
    fn toggle_state(&self) -> Option<ToggleState> {
        let snapshot = crate::current()?;
        let key = self.key.as_ref()?;
        snapshot.find_by_key(key)?.toggle
    }
}

impl IRawElementProviderSimple_Impl for ElementProvider_Impl {
    fn ProviderOptions(&self) -> windows::core::Result<ProviderOptions> {
        // ⚠ 这个值**不是**「有没有额外选项」的无害默认值 —— 它决定 UIA 怎么
        // 跨进程接上我们。
        //
        // UIA 的核心（uiautomationcore.dll）跑在**客户端**进程里，而提供者住
        // 在**应用**进程里 —— 两者天然不在一个进程。所以要报
        // `ServerSideProvider`：「我在服务端进程里，你得封送过来」。
        //
        // 报 0（None）的话，UIA 会按「客户端侧提供者」处理 —— 它以为拿到的是
        // 一个能直接用的本进程指针，于是连接建立失败，**静默退回系统默认的
        // HWND 代理**：窗口看得见，但底下自定义的控件一个都不存在
        // （`FindAll` 返回 0 个元素）。
        //
        // 三个标志的含义：
        //
        // - `ServerSideProvider` —— 提供者在服务端进程里（见上）。
        // - `UseComThreading` —— 我们是 COM 线程模型（`install` 里初始化了
        //   COM 公寓），UIA 会按 COM 规则把调用封送到我们的线程。
        // - `RefuseNonClientSupport` —— 我们**不**支持标题栏/边框那些非客户区
        //   元素，明确拒绝，省得 UIA 来问一圈。
        //
        // 不设 `HasNativeIAccessible` —— 那表示「同时还支持 MSAA」，我们没有。
        Ok(ProviderOptions_ServerSideProvider
            | ProviderOptions_UseComThreading
            | ProviderOptions_RefuseNonClientSupport)
    }

    fn GetPatternProvider(
        &self,
        patternid: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> windows::core::Result<IUnknown> {
        // 元素不在树上 ⇒ 报「不支持」。
        let Some(el) = self.with_element(|el| el.clone()) else {
            trace!(
                "GetPatternProvider id={} self={:?} -> 元素不在树上",
                patternid.0,
                self.key
            );
            return Err(Error::from_hresult(E_FAIL));
        };
        trace!("GetPatternProvider id={} self={:?}", patternid.0, self.key);

        // ⚠ 图案接口是 **`IToggleProvider` / `IInvokeProvider`** —— 不是
        // `IUIAutomationTogglePattern` / `IUIAutomationInvokePattern`。
        //
        // 这是两套**不同 IID** 的接口，混了就会静默失败：
        // - `IUIAutomationXxxPattern` 是**客户端**侧的接口，UIA 自己实现，
        //   客户端拿到的是它的**代理**；
        // - `IToggleProvider` / `IInvokeProvider` 才是**原始提供者**这一侧
        //   要实现的接口（见 SDK 的 `UIAutomationCore.h`）。
        //
        // UIA 的服务端拿到我们交出的对象后，会 QI **`IID_IToggleProvider`** ——
        // 找不到就判定「这个元素不支持 Toggle 图案」，客户端于是读不到图案，
        // 而元素、名字、角色全都正常（极具迷惑性）。
        if patternid.0 == UIA_TogglePatternId.0 {
            // 单选钮**不**暴露 TogglePattern —— 它的「选中」是 SelectionItem。
            // 两套图案同时交出去会让屏读器既念「已勾选」又念「已选中」。
            if el.toggle.is_none() || el.role == Role::RadioButton {
                // 微软文档：这个元素不支持这个图案 ⇒ **成功 + NULL**（不是错误码）。
                return Ok(null_interface::<IUnknown>());
            }
            // ⚠ 交出去的是**这个元素对象自己**的图案接口，不是另一个对象。
            //
            // 微软文档给的例子就是这个做法：
            // > "In this example, ISelectionProvider is implemented in the
            // current object (this). *pRetVal = static_cast(this); AddRef();"
            //
            // 而且 UIA 拿到返回值之后**自己会 QI**：
            // > "It isn't necessary to cast the returned object to the interface
            // corresponding to the requested pattern: UI Automation calls
            // QueryInterface on the returned object as a separate step after
            // this to get the correct interface."
            let pattern = ComObjectInterface::<IToggleProvider>::as_interface_ref(self).to_owned();
            return pattern.cast::<IUnknown>();
        }
        if patternid.0 == UIA_InvokePatternId.0 {
            if el.role != Role::Button {
                return Ok(null_interface::<IUnknown>());
            }
            let pattern = ComObjectInterface::<IInvokeProvider>::as_interface_ref(self).to_owned();
            return pattern.cast::<IUnknown>();
        }
        if patternid.0 == UIA_SelectionItemPatternId.0 {
            if el.role != Role::RadioButton {
                return Ok(null_interface::<IUnknown>());
            }
            let pattern =
                ComObjectInterface::<ISelectionItemProvider>::as_interface_ref(self).to_owned();
            return pattern.cast::<IUnknown>();
        }
        // 不支持的图案 ⇒ 成功 + NULL（微软文档的写法：「this example returns a
        // null reference for all other control pattern identifiers」）。
        Ok(null_interface::<IUnknown>())
    }

    fn GetPropertyValue(
        &self,
        propertyid: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> windows::core::Result<VARIANT> {
        let Some(el) = self.with_element(|el| el.clone()) else {
            trace!(
                "GetPropertyValue id={} self={:?} -> 元素不在树上",
                propertyid.0,
                self.key
            );
            return Err(Error::from_hresult(E_FAIL));
        };
        trace!(
            "GetPropertyValue id={} self={:?} name={:?}",
            propertyid.0,
            self.key,
            el.name
        );
        match propertyid.0 {
            x if x == UIA_NamePropertyId.0 => Ok(VARIANT::from(el.name.as_str())),
            x if x == UIA_AutomationIdPropertyId.0 => Ok(VARIANT::from(el.key.as_str())),
            x if x == UIA_ProviderDescriptionPropertyId.0 => {
                Ok(VARIANT::from(PROVIDER_DESCRIPTION))
            }
            x if x == UIA_ControlTypePropertyId.0 => {
                Ok(VARIANT::from(ElementProvider::control_type(el.role)))
            }
            x if x == UIA_IsEnabledPropertyId.0 => Ok(VARIANT::from(el.enabled)),
            x if x == UIA_HasKeyboardFocusPropertyId.0 => Ok(VARIANT::from(el.focused)),
            x if x == UIA_IsOffscreenPropertyId.0 => Ok(VARIANT::from(el.offscreen)),
            x if x == UIA_IsTogglePatternAvailablePropertyId.0 => {
                // 单选钮不报 Toggle 图案（见 `GetPatternProvider`）。
                Ok(VARIANT::from(
                    el.toggle.is_some() && el.role != Role::RadioButton,
                ))
            }
            x if x == UIA_IsSelectionItemPatternAvailablePropertyId.0 => {
                Ok(VARIANT::from(el.role == Role::RadioButton))
            }
            x if x == UIA_SelectionItemIsSelectedPropertyId.0 => {
                if el.role != Role::RadioButton {
                    return Err(Error::from_hresult(E_FAIL));
                }
                Ok(VARIANT::from(el.toggle == Some(ToggleState::On)))
            }
            x if x == UIA_IsInvokePatternAvailablePropertyId.0 => {
                Ok(VARIANT::from(el.role == Role::Button))
            }
            // 这两个决定元素进不进「控件树 / 内容树」。
            //
            // 我们暴露出来的都是**用户能看见、能操作**的东西 ⇒ 两个都报 true。
            // 不报的话 UIA 会把它们当成装饰，客户端按控件视图遍历时
            // 一个都看不到（`FindAll` 返回 0 个元素）。
            x if x == UIA_IsControlElementPropertyId.0 => Ok(VARIANT::from(true)),
            x if x == UIA_IsContentElementPropertyId.0 => Ok(VARIANT::from(true)),
            x if x == UIA_FrameworkIdPropertyId.0 => Ok(VARIANT::from(FRAMEWORK_ID)),
            // 宿主窗口句柄。**顶层元素**要报真实 HWND，片段里的子元素报 0
            // （它们没有自己的窗口）。
            //
            // 这两个属性 UIA **一定会问**（连元素建缓存时都要），而且**不能报错**
            // —— 报「未实现」会让 UIA 认为这个元素的属性链不完整。
            x if x == UIA_NativeWindowHandlePropertyId.0 => {
                let hwnd = if el.key.is_empty() {
                    crate::host_hwnd().unwrap_or(0) as i32
                } else {
                    0
                };
                Ok(VARIANT::from(hwnd))
            }
            x if x == UIA_ProcessIdPropertyId.0 => Ok(VARIANT::from(std::process::id() as i32)),
            x if x == UIA_ClassNamePropertyId.0 => Ok(VARIANT::from(class_name(el.role))),
            x if x == UIA_IsKeyboardFocusablePropertyId.0 => Ok(VARIANT::from(true)),
            x if x == UIA_ToggleToggleStatePropertyId.0 => match el.toggle {
                Some(t) => Ok(VARIANT::from(t.uia_value())),
                None => Err(Error::from_hresult(E_FAIL)),
            },
            x if x == UIA_BoundingRectanglePropertyId.0 => Ok(bounding_rect_variant(&el)),
            _ => Err(Error::from_hresult(E_NOTIMPL)),
        }
    }

    fn HostRawElementProvider(&self) -> windows::core::Result<IRawElementProviderSimple> {
        // 微软文档的硬性要求（见 `HostRawElementProvider` 一节）：
        //
        // > "A host provider **must** be returned in the following cases: when the
        // element is a **fragment root**, when the element is a simple element
        // (such as a push button), and when the provider is a repositioning
        // placeholder. **In other cases, the property should be NULL.**"
        //
        // 我们只有**一个**片段根（窗口本身）：它必须交出宿主窗口的提供者 ——
        // UIA 靠它把自定义元素与窗口挂上勾（窗口标题、进程号、原生句柄、
        // 顶层元素的 runtime id 都从那儿来）。
        //
        // 片段里的其它元素（复选框、按钮……）**必须**是 NULL：它们不是独立的
        // HWND 宿主，返回窗口提供者会让 UIA 以为「这个元素就是那个窗口」。
        if self.key.is_none() {
            let Some(hwnd) = crate::host_hwnd() else {
                return Err(Error::from_hresult(E_FAIL));
            };
            let h = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
            // SAFETY：`hwnd` 是我们装 UIA 时拿到的、活着的宿主窗口句柄。
            return unsafe { UiaHostProviderFromHwnd(h) };
        }
        // 「没有」= 成功 + NULL（不是错误码）。
        Ok(null_interface::<IRawElementProviderSimple>())
    }
}

impl IRawElementProviderFragment_Impl for ElementProvider_Impl {
    fn Navigate(
        &self,
        direction: NavigateDirection,
    ) -> windows::core::Result<IRawElementProviderFragment> {
        // 微软文档的硬性要求：
        //
        // > "**Fragment roots do not enable navigation to a parent or siblings**;
        // navigation among fragment roots is handled by the default window
        // providers. Elements in fragments must navigate only to other elements
        // within that fragment."
        //
        // ⇒ 片段根（窗口）**不**导航到父 —— 客户端会改用系统的默认窗口提供者
        // 往上走。我们照做，别自作主张返回自己。
        let Some(key) = self
            .with_element(|el| navigate_target(el, direction))
            .flatten()
        else {
            trace!("Navigate dir={} self={:?} -> 没有", direction.0, self.key);
            // 「那个方向没有元素」= **成功 + NULL**，不是错误码。
            // 报 E_FAIL 会被 UIA 当成「出错了」，遍历当场断掉。
            return Ok(null_interface::<IRawElementProviderFragment>());
        };
        trace!(
            "Navigate dir={} self={:?} -> {key:?}",
            direction.0,
            self.key
        );
        Ok(fragment_for(Some(key)))
    }

    fn GetRuntimeId(&self) -> windows::core::Result<*mut SAFEARRAY> {
        // 微软文档的硬性要求：
        //
        // > "Implementations should return **NULL** for a **top-level element that
        // is hosted in a window**. Other elements should return an array that
        // contains **UiaAppendRuntimeId**, followed by a value that is unique
        // within an instance of the fragment."
        //
        // ⇒ 顶层元素（我们的窗口根）返回 NULL —— UIA 会改用**宿主提供者**的
        // runtime id（那个 HWND 的），那才是「顶层」该有的身份。
        // 片段里的其它元素返回 `[UiaAppendRuntimeId, 我们的稳定 id]`。
        if self.key.is_none() {
            return Ok(std::ptr::null_mut());
        }
        let id = self.with_element(|el| runtime_id_for(&el.key)).unwrap_or(0);
        Ok(unsafe { int_array(&[UiaAppendRuntimeId as i32, id]) })
    }

    fn BoundingRectangle(&self) -> windows::core::Result<UiaRect> {
        let Some(el) = self.with_element(|el| el.clone()) else {
            return Err(Error::from_hresult(E_FAIL));
        };
        Ok(UiaRect {
            left: el.rect.x as f64,
            top: el.rect.y as f64,
            width: el.rect.width as f64,
            height: el.rect.height as f64,
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> windows::core::Result<*mut SAFEARRAY> {
        // 微软文档：「Most providers return **NULL**。」
        //
        // 「内嵌片段根」指的是「这个元素里还嵌着**另一套**无障碍框架」——
        // 我们没有（界面是我们自己画的，整棵树就这一个片段）。
        // 返回空数组虽然也能跑，但那是「有 0 个内嵌片段根」；返回 NULL 才是
        // 文档里说的「没有这回事」。
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> windows::core::Result<()> {
        // 焦点住在应用的 `FocusState` 里，不由 COM 侧直接改 —— 排队，让宿主
        // 在下一帧按正常路径移动焦点。
        //
        // 微软文档：「Your implementation should update only its internal focus
        // state」—— 我们正是这么做的：排队 → 宿主改状态 → 重新画。
        let Some(key) = self.with_element(|el| el.key.clone()) else {
            return Err(Error::from_hresult(E_FAIL));
        };
        if key.is_empty() {
            return Err(Error::from_hresult(E_FAIL));
        }
        crate::push_action(crate::Action::Focus(key));
        Ok(())
    }

    fn FragmentRoot(
        &self,
    ) -> windows::core::Result<windows::Win32::UI::Accessibility::IRawElementProviderFragmentRoot>
    {
        // 微软文档：「A provider for a fragment root should return a pointer to
        // **its own** implementation of IRawElementProviderFragmentRoot.」
        // （.NET 版同一句：「should return itself」）
        //
        // ⇒ 片段根返回**它自己**；片段里的其它元素返回**片段根**。
        // 这两个都指向同一个 COM 对象（窗口根那个），只是换个接口视角。
        //
        // ⚠ 这里**不能**报「不支持」：客户端要先解析出片段根才知道这棵树的
        // 根是谁，报错的话它会当场放弃整棵树 —— 表现是 `FindAll` 返回 0 个
        // 元素、连属性都读不到。
        trace!("FragmentRoot self={:?}", self.key);
        Ok(fragment_root_for())
    }
}

impl IRawElementProviderFragmentRoot_Impl for ElementProvider_Impl {
    fn ElementProviderFromPoint(
        &self,
        x: f64,
        y: f64,
    ) -> windows::core::Result<IRawElementProviderFragment> {
        // 坐标是**屏幕**坐标 —— 快照里的矩形也是屏幕坐标，两边对得上。
        let hit = self.with_element(|_| hit_test(x, y));
        trace!("ElementProviderFromPoint ({x}, {y}) -> {hit:?}");
        let Some(key) = hit.flatten() else {
            // 点上什么都没有 ⇒ 成功 + NULL。
            return Ok(null_interface::<IRawElementProviderFragment>());
        };
        Ok(fragment_for(Some(key)))
    }

    fn GetFocus(&self) -> windows::core::Result<IRawElementProviderFragment> {
        // 微软文档：「Retrieves the element in this fragment that has the input
        // focus.」 —— 没人有焦点时是 NULL。
        let focused = crate::current()
            .and_then(|s| s.elements.iter().find(|e| e.focused).map(|e| e.key.clone()));
        trace!("GetFocus -> {focused:?}");
        let Some(key) = focused else {
            return Ok(null_interface::<IRawElementProviderFragment>());
        };
        Ok(fragment_for(Some(key)))
    }
}

/// Toggle 图案 —— **由元素对象自己实现**。
///
/// # ⚠ 接口名是 `IToggleProvider`，不是 `IUIAutomationTogglePattern`
///
/// 后者是**客户端**侧的接口（UIA 自己实现，客户端拿到的是它的代理）；
/// 原始提供者这一侧要实现的是 `IToggleProvider`（SDK `UIAutomationCore.h`）。
/// 两者 IID 不同 —— 实现错了，UIA 的 QI 必然失败，然后**静默**判定「这个元素
/// 没有 Toggle 图案」：元素在、名字在、角色在，唯独图案取不到。
///
/// # 为什么不是「另一个对象」
///
/// 图案接口要在**跨进程**的场景下用（屏幕阅读器在别的进程里）。UIA 只认
/// 「它已经握在手里的那个对象」的接口 —— 另造一个图案对象，UIA 就得把它
/// 封送回客户端，而它的原始提供者通道不做这件事。
///
/// 于是图案接口由**元素对象自己**实现，`GetPatternProvider` 交出的就是
/// 它自己。（Flutter 的 Windows 无障碍实现同款做法。）
impl IToggleProvider_Impl for ElementProvider_Impl {
    fn Toggle(&self) -> windows::core::Result<()> {
        let Some(key) = self.element_key() else {
            return Err(Error::from_hresult(E_FAIL));
        };
        trace!("Toggle key={key}");
        crate::push_action(crate::Action::Toggle(key));
        Ok(())
    }

    fn ToggleState(&self) -> windows::core::Result<UiaToggleState> {
        let Some(state) = self.toggle_state() else {
            return Err(Error::from_hresult(E_FAIL));
        };
        trace!("ToggleState -> {state:?}");
        Ok(match state {
            ToggleState::Off => UiaToggleState(0),
            ToggleState::On => UiaToggleState(1),
            ToggleState::Indeterminate => UiaToggleState(2),
        })
    }
}

/// Invoke 图案 —— 同样由元素对象自己实现（理由同上）。
impl IInvokeProvider_Impl for ElementProvider_Impl {
    fn Invoke(&self) -> windows::core::Result<()> {
        let Some(key) = self.element_key() else {
            return Err(Error::from_hresult(E_FAIL));
        };
        crate::push_action(crate::Action::Invoke(key));
        Ok(())
    }
}

/// SelectionItem 图案 —— 单选钮用这一套（理由同上：由元素对象自己实现）。
///
/// # 为什么单选钮不是 TogglePattern
///
/// 两者的**语义**不同：
///
/// | 图案 | 说的是 | 谁用 |
/// |---|---|---|
/// | Toggle | 「这个开关被打开了」 | 复选框、开关 |
/// | SelectionItem | 「在一组里挑中了我」 | 单选钮、列表项 |
///
/// 报错的屏读器会念「复选框，已勾选」，而用户听到的应该是「单选钮，
/// 已选中」—— 两句话对用户意味着完全不同的交互（前者可再点取消，
/// 后者点一下不会取消）。
///
/// # `AddToSelection` / `RemoveFromSelection` 为什么要报「不支持」
///
/// 单选组是**单选**容器：同时选中两项在语义上不存在，而「取消选中」同样
/// 不存在（单选组至少留一项）。给它们一个「看起来成功但什么都没发生」的
/// 返回值是更坏的答案 —— 客户端会以为操作生效了。
impl ISelectionItemProvider_Impl for ElementProvider_Impl {
    fn Select(&self) -> windows::core::Result<()> {
        let Some(key) = self.element_key() else {
            return Err(Error::from_hresult(E_FAIL));
        };
        trace!("SelectionItem.Select key={key}");
        crate::push_action(crate::Action::Select(key));
        Ok(())
    }

    fn AddToSelection(&self) -> windows::core::Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn RemoveFromSelection(&self) -> windows::core::Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn IsSelected(&self) -> windows::core::Result<windows::core::BOOL> {
        let Some(el) = self.with_element(|el| el.clone()) else {
            return Err(Error::from_hresult(E_FAIL));
        };
        if el.role != Role::RadioButton {
            return Err(Error::from_hresult(E_FAIL));
        }
        Ok(windows::core::BOOL::from(
            el.toggle == Some(ToggleState::On),
        ))
    }

    /// 选中项的**容器**（那个单选组）。
    ///
    /// # 为什么返回父元素而不是 null
    ///
    /// 客户端靠它知道「这些单选钮属于同一组」—— 上下键在组内移动就靠这条
    /// 关系。父元素不是分组（`Role::Group`）时退回 null：那时没有可报的容器，
    /// 而「成功 + NULL」是 UIA 表达「没有」的写法。
    fn SelectionContainer(&self) -> windows::core::Result<IRawElementProviderSimple> {
        let parent_key = self
            .with_element(|el| el.parent)
            .flatten()
            .and_then(|id| crate::current().and_then(|s| s.get(id).map(|p| p.key.clone())))
            .filter(|k| !k.is_empty());
        match parent_key {
            Some(key) => Ok(provider_for(Some(key))),
            None => Ok(null_interface::<IRawElementProviderSimple>()),
        }
    }
}

// --------------------------------------------------------------- 缓存与工具

// 根提供者的缓存。
//
// `WM_GETOBJECT` 返回的是**裸指针**，UIA 客户端不会替我们 AddRef —— 所以
// 这个对象必须一直活着。放在**线程局部**里：COM 指针不是 `Send`，而且它
// 只在 UI 线程上被碰（`WM_GETOBJECT` 就在那儿）。
thread_local! {
    static ROOT_PROVIDER: RefCell<Option<IRawElementProviderSimple>> = const { RefCell::new(None) };
}

/// 取（必要时造出）缓存的根提供者。
///
/// 永远返回 —— 缓存里没有就造一个。
pub fn root_provider() -> IRawElementProviderSimple {
    ROOT_PROVIDER.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(provider_for(None));
        }
        slot.borrow()
            .clone()
            .expect("缓存里刚放进去过一个，clone 必然成功")
    })
}

/// 导航目标：某个方向上的邻居元素的 key。
fn navigate_target(el: &Element, direction: NavigateDirection) -> Option<String> {
    let snapshot = crate::current()?;
    let target = match direction.0 {
        x if x == NavigateDirection_Parent.0 => el.parent,
        x if x == NavigateDirection_FirstChild.0 => el.children.first().copied(),
        x if x == NavigateDirection_LastChild.0 => el.children.last().copied(),
        x if x == NavigateDirection_NextSibling.0 => next_sibling(&snapshot, el).map(|e| e.id),
        x if x == NavigateDirection_PreviousSibling.0 => prev_sibling(&snapshot, el).map(|e| e.id),
        _ => None,
    };
    target
        .and_then(|id| snapshot.get(id))
        .map(|e| e.key.clone())
}

/// 下一个兄弟。
fn next_sibling<'a>(snapshot: &'a Snapshot, el: &Element) -> Option<&'a Element> {
    let parent = el.parent?;
    let siblings = &snapshot.get(parent)?.children;
    let idx = siblings.iter().position(|id| *id == el.id)?;
    snapshot.get(*siblings.get(idx + 1)?)
}

/// 上一个兄弟。
fn prev_sibling<'a>(snapshot: &'a Snapshot, el: &Element) -> Option<&'a Element> {
    let parent = el.parent?;
    let siblings = &snapshot.get(parent)?.children;
    let idx = siblings.iter().position(|id| *id == el.id)?;
    if idx == 0 {
        return None;
    }
    snapshot.get(*siblings.get(idx - 1)?)
}

/// 按屏幕坐标找命中的元素（**最深**的那个）。
///
/// # 为什么取「最深」的
///
/// 复选框的矩形套在组容器里，点在复选框上时，组容器的矩形也包含这个点。
/// 命中应当落在**最具体**的那个元素上 —— 这正是 UIA 的 `ElementProviderFromPoint`
/// 的语义（它对应「鼠标点在这儿会点到谁」）。
///
/// 深度 = 祖先个数。快照里 `parent` 构成一棵树，所以深度的算法就是顺着
/// `parent` 往上数。
fn hit_test(x: f64, y: f64) -> Option<String> {
    let snapshot = crate::current()?;
    let mut best: Option<(usize, &Element)> = None;
    for el in &snapshot.elements {
        if !contains(el, x, y) {
            continue;
        }
        let depth = depth_of(&snapshot, el);
        // 同深度时后来者胜（快照是文档序，后来的更靠后 —— 视觉上通常更靠上）。
        if best.map(|(d, _)| depth >= d).unwrap_or(true) {
            best = Some((depth, el));
        }
    }
    best.map(|(_, el)| el.key.clone())
}

/// 元素在快照里的深度（根 = 0）。
fn depth_of(snapshot: &Snapshot, el: &Element) -> usize {
    let mut depth = 0;
    let mut cursor = el.parent;
    while let Some(id) = cursor {
        depth += 1;
        cursor = snapshot.get(id).and_then(|p| p.parent);
    }
    depth
}

/// 点在元素的矩形里吗。
fn contains(el: &Element, x: f64, y: f64) -> bool {
    let r = el.rect;
    x >= r.x as f64 && x < (r.x + r.width) as f64 && y >= r.y as f64 && y < (r.y + r.height) as f64
}

/// 稳定的运行时 id —— 按 key 登记，第一次见到时分配。
///
/// # 为什么不能直接用快照下标
///
/// 快照每帧重造，下标会随树结构变化而移位（换页、加一个控件）。runtime id
/// 是 UIA 用来**跨调用认出同一个元素**的凭据 —— 它一变，客户端就会以为
/// 「换了一个元素」，于是重新念一遍。
fn runtime_id_for(key: &str) -> i32 {
    static REGISTRY: std::sync::LazyLock<Mutex<HashMap<String, i32>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    static NEXT: AtomicI32 = AtomicI32::new(1);
    if key.is_empty() {
        return 0;
    }
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(id) = reg.get(key) {
        return *id;
    }
    // 登记表封顶：key 是动态生成的（列表项之类）时不会无限涨。
    // 撞上上限就退化成「按 key 哈希」—— 不保证唯一，但比没有强。
    if reg.len() >= 4096 {
        let mut h: i32 = 0;
        for b in key.bytes() {
            h = h.wrapping_mul(31).wrapping_add(b as i32);
        }
        return h & 0x7fff_ffff;
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    reg.insert(key.to_string(), id);
    id
}

/// 整数数组（runtime id、子元素 id 都用它）。
///
/// # SAFETY
///
/// 返回的 `SAFEARRAY*` 交给调用方（UIA 会 `SafeArrayDestroy`）。
unsafe fn int_array(values: &[i32]) -> *mut SAFEARRAY {
    let psa = SafeArrayCreateVector(VT_I4, 0, values.len() as u32);
    if psa.is_null() {
        return std::ptr::null_mut();
    }
    if values.is_empty() {
        return psa;
    }
    // 直接拿数据指针写 —— 比逐元素 SafeArrayPutElement 少一层「下标数组」
    // 的语义争议（一维数组的下标到底传指针还是传 NULL，各家说法不一，
    // 写数据最没有争议）。
    let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
    if SafeArrayAccessData(psa, &mut data).is_ok() && !data.is_null() {
        std::ptr::copy_nonoverlapping(values.as_ptr(), data as *mut i32, values.len());
        let _ = SafeArrayUnaccessData(psa);
    }
    psa
}

/// `BoundingRectangle` 属性 —— 四个 double 的数组（left / top / width / height）。
fn bounding_rect_variant(el: &Element) -> VARIANT {
    let values = [
        el.rect.x as f64,
        el.rect.y as f64,
        el.rect.width as f64,
        el.rect.height as f64,
    ];
    unsafe {
        let psa = SafeArrayCreateVector(VT_R8, 0, values.len() as u32);
        if psa.is_null() {
            return VARIANT::default();
        }
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        if SafeArrayAccessData(psa, &mut data).is_ok() && !data.is_null() {
            std::ptr::copy_nonoverlapping(values.as_ptr(), data as *mut f64, values.len());
            let _ = SafeArrayUnaccessData(psa);
        }
        let mut out = VARIANT::default();
        // `ManuallyDrop` 不自动 DerefMut —— 显式解引用再写字段。
        let inner: &mut VARIANT_0_0 = &mut out.Anonymous.Anonymous;
        inner.vt = VARENUM(VT_R8.0 | VT_ARRAY.0);
        inner.Anonymous = VARIANT_0_0_0 { parray: psa };
        out
    }
}

/// 抛「开关状态变了」事件。
///
/// UIA 没有独立的 Toggle 事件 —— 客户端听的是 `AutomationPropertyChanged`，
/// 属性是 `ToggleToggleState`。这里把新旧值都报上去，客户端才知道是
/// 「勾上了」还是「半选了」。
pub(crate) fn raise_toggle_state_changed(key: &str, old: ToggleState, new: ToggleState) -> bool {
    let Some(snapshot) = crate::current() else {
        return false;
    };
    let Some(el) = snapshot.find_by_key(key) else {
        return false;
    };
    let provider = provider_for(Some(key.to_string()));
    // ⚠ 单选钮的「选中」在 UIA 里**不是** ToggleToggleState，而是
    // SelectionItem.IsSelected（一个 BOOL）。报错属性时客户端收不到
    // 它在听的那条事件 —— 屏读器于是永远不念「已选中」。
    let (property, old, new) = if el.role == Role::RadioButton {
        (
            UIA_SelectionItemIsSelectedPropertyId,
            VARIANT::from(old == ToggleState::On),
            VARIANT::from(new == ToggleState::On),
        )
    } else {
        (
            UIA_ToggleToggleStatePropertyId,
            VARIANT::from(old.uia_value()),
            VARIANT::from(new.uia_value()),
        )
    };
    unsafe {
        windows::Win32::UI::Accessibility::UiaRaiseAutomationPropertyChangedEvent(
            &provider, property, &old, &new,
        )
    }
    .is_ok()
}

// ------------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Element;
    use windows::core::{Interface, BSTR};
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
        SafeArrayUnaccessData,
    };
    use windows::Win32::UI::Accessibility::{UIA_ItemStatusPropertyId, UIA_PROPERTY_ID};
    use zephyra_core::core::Rect;

    /// 一棵典型的树：窗口根 → 分组 → 复选框。
    ///
    /// 根的 `children` 是**暴露元素**的下标（`rules::build` 就是这么产的）。
    /// 分组套着复选框 —— 这样「按点命中」才有意义（复选框的矩形在分组的
    /// 矩形里面）。
    fn sample_snapshot(state: ToggleState) -> Snapshot {
        let mut snap = Snapshot::default();
        snap.elements.push(Element {
            id: 0,
            parent: None,
            key: String::new(),
            name: "Zephyra Gallery".into(),
            role: Role::Window,
            toggle: None,
            enabled: true,
            focused: false,
            offscreen: false,
            rect: Rect::new(0.0, 0.0, 800.0, 600.0),
            children: vec![1],
        });
        snap.elements.push(Element {
            id: 1,
            parent: Some(0),
            key: "group:checks".into(),
            name: "Select all".into(),
            role: Role::Group,
            toggle: None,
            enabled: true,
            focused: false,
            offscreen: false,
            rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            children: vec![2],
        });
        snap.elements.push(Element {
            id: 2,
            parent: Some(1),
            key: "check:alpha".into(),
            name: "Enable feature".into(),
            role: Role::CheckBox,
            toggle: Some(state),
            enabled: true,
            focused: false,
            offscreen: false,
            rect: Rect::new(10.0, 20.0, 22.0, 22.0),
            children: vec![],
        });
        snap
    }

    /// 打 `Navigate` 的**原始** vtable，拿真正的 COM 结果（HRESULT + 出参）。
    ///
    /// # 为什么不用 windows-rs 的包装方法
    ///
    /// 包装方法在出参拿到之后会走 `Type::from_abi` —— 它见到空指针就返回
    /// `Err(Error::empty())`，而 `Error::empty()` 的码恰好是 `S_OK`（0）。
    ///
    /// 也就是说：「COM 层成功地交出 NULL」在 Rust 侧被翻译成了「一个码为 0 的
    /// 错误」。那是**客户端包装层**的事，不是 COM 层的结果 —— 而我们要断言的
    /// 恰恰是 COM 层（微软文档：「pRetVal … or **NULL** if there is no element
    /// in that direction」）。所以只能打 vtable。
    ///
    /// 返回的出参指针**归调用方**：非空时得自己 Release（这里用不到 ——
    /// 断言的两处都是 NULL）。
    fn navigate_raw(
        f: &IRawElementProviderFragment,
        direction: NavigateDirection,
    ) -> (windows::core::HRESULT, *mut core::ffi::c_void) {
        // SAFETY：COM 调用；`f` 由调用方持有，调用期间有效。
        unsafe {
            let vtbl = windows::core::Interface::vtable(f);
            let mut out: *mut core::ffi::c_void = std::ptr::null_mut();
            let hr = (vtbl.Navigate)(
                windows::core::Interface::as_raw(f),
                direction,
                &mut out as *mut *mut core::ffi::c_void,
            );
            (hr, out)
        }
    }

    /// 打 `ElementProviderFromPoint` 的原始 vtable（同 [`navigate_raw`] 的理由）。
    fn element_from_point_raw(
        f: &windows::Win32::UI::Accessibility::IRawElementProviderFragmentRoot,
        x: f64,
        y: f64,
    ) -> (windows::core::HRESULT, *mut core::ffi::c_void) {
        // SAFETY：COM 调用；`f` 由调用方持有，调用期间有效。
        unsafe {
            let vtbl = windows::core::Interface::vtable(f);
            let mut out: *mut core::ffi::c_void = std::ptr::null_mut();
            let hr = (vtbl.ElementProviderFromPoint)(
                windows::core::Interface::as_raw(f),
                x,
                y,
                &mut out as *mut *mut core::ffi::c_void,
            );
            (hr, out)
        }
    }

    /// 断言「那个方向没有元素」= **成功 + NULL**。
    fn assert_no_element(hr: windows::core::HRESULT, out: *mut core::ffi::c_void) {
        assert_eq!(
            hr.0, 0,
            "「没有元素」必须是成功（S_OK），不是错误码 —— 报 E_FAIL 会被 UIA \
             当成「出错了」，遍历当场断掉"
        );
        assert!(
            out.is_null(),
            "「没有元素」必须交出 NULL 出参（微软文档：pRetVal … or NULL）"
        );
    }

    /// 打 `GetPatternProvider` 的原始 vtable（同 [`navigate_raw`] 的理由）。
    fn get_pattern_provider_raw(
        p: &IRawElementProviderSimple,
        patternid: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> (windows::core::HRESULT, *mut core::ffi::c_void) {
        // SAFETY：COM 调用；`p` 由调用方持有，调用期间有效。
        unsafe {
            let vtbl = windows::core::Interface::vtable(p);
            let mut out: *mut core::ffi::c_void = std::ptr::null_mut();
            let hr = (vtbl.GetPatternProvider)(
                windows::core::Interface::as_raw(p),
                patternid,
                &mut out as *mut *mut core::ffi::c_void,
            );
            (hr, out)
        }
    }

    /// 读一个属性。
    fn prop(p: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> VARIANT {
        // SAFETY：COM 调用；`p` 由本测试持有，调用期间有效。
        unsafe { p.GetPropertyValue(id) }.expect("该属性应当可读")
    }

    /// 读一个字符串属性。
    fn bstr_of(p: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> String {
        let v = prop(p, id);
        let b = BSTR::try_from(&v).expect("该属性应当是 VT_BSTR");
        b.to_string()
    }

    /// 读一个整数属性。
    fn i32_of(p: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> i32 {
        let v = prop(p, id);
        i32::try_from(&v).expect("该属性应当是 VT_I4")
    }

    /// 读一个布尔属性。
    fn bool_of(p: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> bool {
        let v = prop(p, id);
        bool::try_from(&v).expect("该属性应当是 VT_BOOL")
    }

    /// 把片段提供者转回简单提供者（同一个 COM 对象，只是换个 QI 视角）。
    fn simple_of(f: &IRawElementProviderFragment) -> IRawElementProviderSimple {
        f.cast::<IRawElementProviderSimple>()
            .expect("元素同时实现两个接口")
    }

    /// 读 runtime id（我们自己造的那个 SAFEARRAY 归我们销毁）。
    fn runtime_id_of(f: &IRawElementProviderFragment) -> Vec<i32> {
        // SAFETY：COM 调用；数组由我们造，归我们销毁。
        unsafe {
            let psa = f.GetRuntimeId().expect("应当有 runtime id");
            assert!(!psa.is_null());
            let ub = SafeArrayGetUBound(psa, 1).expect("读上界");
            let lb = SafeArrayGetLBound(psa, 1).expect("读下界");
            let n = (ub - lb + 1) as usize;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            assert!(SafeArrayAccessData(psa, &mut data).is_ok(), "取数据指针");
            assert!(!data.is_null());
            let out = std::slice::from_raw_parts(data as *const i32, n).to_vec();
            let _ = SafeArrayUnaccessData(psa);
            let _ = SafeArrayDestroy(psa);
            out
        }
    }

    #[test]
    fn properties_answer_from_the_snapshot() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Indeterminate));
        let p = provider_for(Some("check:alpha".to_string()));

        assert_eq!(bstr_of(&p, UIA_NamePropertyId), "Enable feature");
        assert_eq!(bstr_of(&p, UIA_AutomationIdPropertyId), "check:alpha");
        assert_eq!(bstr_of(&p, UIA_ProviderDescriptionPropertyId), "ZephyraUI");
        assert_eq!(
            i32_of(&p, UIA_ControlTypePropertyId),
            UIA_CheckBoxControlTypeId.0,
            "复选框的 ControlType 必须是 CheckBox"
        );
        assert_eq!(
            i32_of(&p, UIA_ToggleToggleStatePropertyId),
            2,
            "半选在 UIA 里是 ToggleState=2"
        );
        assert!(bool_of(&p, UIA_IsTogglePatternAvailablePropertyId));
        assert!(bool_of(&p, UIA_IsEnabledPropertyId));
        assert!(!bool_of(&p, UIA_HasKeyboardFocusPropertyId));
    }

    /// **本模块最关键的一条**：提供者不能把快照「焊」在手里。
    ///
    /// 如果它焊住旧快照，那么：用户勾上复选框 → 我们抛了「状态变了」事件 →
    /// 客户端回头来问 → 拿到的是**没勾**那一份 —— 屏读器于是念了一个错的状态。
    #[test]
    fn provider_reads_the_latest_snapshot_not_the_one_it_was_born_with() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let p = provider_for(Some("check:alpha".to_string()));
        assert_eq!(i32_of(&p, UIA_ToggleToggleStatePropertyId), 0);

        // 换一帧（用户点了）。
        crate::publish(sample_snapshot(ToggleState::On));
        assert_eq!(
            i32_of(&p, UIA_ToggleToggleStatePropertyId),
            1,
            "换帧后必须答新状态，不能答旧值"
        );
    }

    #[test]
    fn toggle_pattern_reports_state_and_queues_an_action() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Indeterminate));
        let _ = crate::take_actions();
        let p = provider_for(Some("check:alpha".to_string()));

        // SAFETY：COM 调用。
        let unknown =
            unsafe { p.GetPatternProvider(UIA_TogglePatternId) }.expect("复选框应当有 Toggle 图案");
        // ⚠ 这是**原始提供者**那一侧的图案接口（`IToggleProvider`），
        // 不是客户端侧的 `IUIAutomationTogglePattern` —— 两者 IID 不同。
        let toggle = unknown
            .cast::<IToggleProvider>()
            .expect("QI 到 Toggle 图案接口");
        // SAFETY：COM 调用。
        assert_eq!(
            unsafe { toggle.ToggleState() }.expect("当前状态").0,
            2,
            "半选 = ToggleState 2"
        );

        // SAFETY：COM 调用。
        unsafe { toggle.Toggle() }.expect("Toggle 应当成功");
        let actions = crate::take_actions();
        assert_eq!(actions.len(), 1, "Toggle 应当排一个动作");
        assert_eq!(actions[0], crate::Action::Toggle("check:alpha".into()));
    }

    /// 不支持某个图案的元素，要交出 **成功 + NULL**。
    ///
    /// 微软文档（`GetPatternProvider`）：
    /// > "Receives a pointer to the object that supports the control pattern, or
    /// > **NULL** if the control pattern is not supported."
    ///
    /// 报 E_FAIL 会被 UIA 当成「出错了」，而不是「这个元素没这个图案」。
    #[test]
    fn elements_without_the_pattern_refuse_it() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let p = provider_for(Some("group:checks".to_string()));

        let (hr, out) = get_pattern_provider_raw(&p, UIA_TogglePatternId);
        assert_eq!(hr.0, 0, "「不支持这个图案」必须是成功，不是错误码");
        assert!(out.is_null(), "「不支持这个图案」必须交出 NULL");

        assert!(!bool_of(&p, UIA_IsTogglePatternAvailablePropertyId));
    }

    #[test]
    fn runtime_id_is_stable_across_snapshots() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let f = fragment_for(Some("check:alpha".to_string()));
        let a = runtime_id_of(&f);

        // 换一帧：树里的下标可能移位，但 runtime id 必须不变 ——
        // 否则客户端会以为「换了一个元素」，于是把同一个复选框重新念一遍。
        crate::publish(sample_snapshot(ToggleState::On));
        let b = runtime_id_of(&f);
        assert_eq!(a, b, "runtime id 必须跨帧稳定");
        assert_eq!(a.len(), 2, "runtime id = 前缀 + 我们分配的稳定 id");
        assert_eq!(a[0], UiaAppendRuntimeId as i32);
    }

    #[test]
    fn fragment_navigation_walks_the_tree() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));

        let root = fragment_for(None);
        // SAFETY：COM 调用。
        let group =
            unsafe { root.Navigate(NavigateDirection_FirstChild) }.expect("根应当有第一个子元素");
        assert_eq!(
            bstr_of(&simple_of(&group), UIA_AutomationIdPropertyId),
            "group:checks"
        );

        // SAFETY：COM 调用。
        let child = unsafe { group.Navigate(NavigateDirection_FirstChild) }
            .expect("分组应当有第一个子元素");
        assert_eq!(
            bstr_of(&simple_of(&child), UIA_AutomationIdPropertyId),
            "check:alpha"
        );

        // SAFETY：COM 调用。
        let parent = unsafe { child.Navigate(NavigateDirection_Parent) }.expect("子应当能回到父");
        assert_eq!(
            bstr_of(&simple_of(&parent), UIA_AutomationIdPropertyId),
            "group:checks"
        );

        // SAFETY：COM 调用。
        let grandparent =
            unsafe { parent.Navigate(NavigateDirection_Parent) }.expect("组应当能回到根");
        assert_eq!(
            bstr_of(&simple_of(&grandparent), UIA_AutomationIdPropertyId),
            "",
            "根元素没有 AutomationId"
        );

        // 叶子没有子元素 ⇒ 「那个方向没有元素」。
        let (hr, out) = navigate_raw(&child, NavigateDirection_FirstChild);
        assert_no_element(hr, out);
    }

    /// 片段根**不**导航到父 —— 这是微软文档的明文要求。
    ///
    /// > "**Fragment roots do not enable navigation to a parent or siblings**;
    /// > navigation among fragment roots is handled by the default window
    /// > providers."
    ///
    /// 「往上走」是系统默认窗口提供者的活儿，我们不插手。
    ///
    /// 「没有元素」要表达成 **成功 + NULL**，不是错误码 ——
    /// 报 E_FAIL 会被 UIA 当成「出错了」，而不是「走到头了」。
    #[test]
    fn the_fragment_root_has_no_parent() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));

        let root = fragment_for(None);
        let (hr, out) = navigate_raw(&root, NavigateDirection_Parent);
        assert_no_element(hr, out);
    }

    /// 片段根**必须**能解析出来 —— 客户端拿不到它就放弃整棵树。
    #[test]
    fn the_fragment_root_is_available() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));

        let root = fragment_for(None);
        // SAFETY：COM 调用。
        let fragment_root = unsafe { root.FragmentRoot() }.expect("片段根必须可解析");
        // 片段根同时也是简单提供者（同一个 COM 对象的另一个视角）。
        let simple = fragment_root
            .cast::<IRawElementProviderSimple>()
            .expect("片段根同时实现 IRawElementProviderSimple");
        assert_eq!(
            i32_of(&simple, UIA_ControlTypePropertyId),
            UIA_WindowControlTypeId.0
        );
    }

    /// 按点命中：落在复选框上就该是复选框，不是套着它的分组。
    #[test]
    fn hit_testing_prefers_the_deepest_element() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));

        let root = element_fragment_root_provider(None);
        // 复选框的矩形 (10,20)-(32,42) 也在分组的矩形 (0,0)-(200,100) 里面。
        // SAFETY：COM 调用。
        let hit = unsafe { root.ElementProviderFromPoint(15.0, 25.0) }.expect("应当命中");
        assert_eq!(
            bstr_of(&simple_of(&hit), UIA_AutomationIdPropertyId),
            "check:alpha",
            "命中最深的那个"
        );

        // 落在分组里、复选框外 ⇒ 命中分组。
        // SAFETY：COM 调用。
        let hit = unsafe { root.ElementProviderFromPoint(100.0, 80.0) }.expect("应当命中分组");
        assert_eq!(
            bstr_of(&simple_of(&hit), UIA_AutomationIdPropertyId),
            "group:checks"
        );

        // 落在所有元素之外 ⇒ 没有命中（成功 + NULL，不是错误码）。
        let (hr, out) = element_from_point_raw(&root, 5000.0, 5000.0);
        assert_no_element(hr, out);
    }

    /// 当前焦点 = 快照里带 `FOCUSED` 的那个元素。
    #[test]
    fn get_focus_reports_the_focused_element() {
        let _g = crate::test_lock::guard();
        let mut snap = sample_snapshot(ToggleState::Off);
        snap.elements[2].focused = true;
        crate::publish(snap);

        let root = element_fragment_root_provider(None);
        // SAFETY：COM 调用。
        let focused = unsafe { root.GetFocus() }.expect("应当能答出焦点");
        assert_eq!(
            bstr_of(&simple_of(&focused), UIA_AutomationIdPropertyId),
            "check:alpha"
        );
    }

    #[test]
    fn set_focus_queues_a_focus_action() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let _ = crate::take_actions();
        let f = fragment_for(Some("check:alpha".to_string()));
        // SAFETY：COM 调用。
        unsafe { f.SetFocus() }.expect("SetFocus 应当成功");
        let actions = crate::take_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], crate::Action::Focus("check:alpha".into()));
    }

    #[test]
    fn bounding_rectangle_comes_from_the_snapshot() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let f = fragment_for(Some("check:alpha".to_string()));
        // SAFETY：COM 调用。
        let r = unsafe { f.BoundingRectangle() }.expect("应当有矩形");
        assert_eq!(r.left, 10.0);
        assert_eq!(r.top, 20.0);
        assert_eq!(r.width, 22.0);
        assert_eq!(r.height, 22.0);
    }

    #[test]
    fn an_element_that_left_the_tree_reports_failure() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let p = provider_for(Some("check:gone".to_string()));
        // SAFETY：COM 调用。
        assert!(
            unsafe { p.GetPropertyValue(UIA_NamePropertyId) }.is_err(),
            "元素不在树上时属性查询应报「不支持」"
        );
        // SAFETY：COM 调用。
        assert!(unsafe { p.GetPatternProvider(UIA_TogglePatternId) }.is_err());
    }

    #[test]
    fn unknown_properties_are_reported_as_not_implemented() {
        let _g = crate::test_lock::guard();
        crate::publish(sample_snapshot(ToggleState::Off));
        let p = provider_for(Some("check:alpha".to_string()));
        // `UIA_ItemStatusPropertyId` 是本实现明确不提供的属性。
        // SAFETY：COM 调用。
        let err = unsafe { p.GetPropertyValue(UIA_ItemStatusPropertyId) }
            .expect_err("未实现的属性应报错");
        assert_eq!(err.code().0, E_NOTIMPL.0);
    }
}
