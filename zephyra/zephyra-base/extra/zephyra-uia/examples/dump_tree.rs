//! 开发期工具：用 **UIA 客户端**读一棵窗口的元素树。
//!
//! # 为什么要有它
//!
//! `zephyra-uia` 的服务端做的是「交出一棵原始提供者树」。而「这棵树被
//! 客户端读起来对不对」只有**真的当一次客户端**才知道 ——
//! 客户端会走 `WM_GETOBJECT`、会 QI 图案接口、会按 `ControlType` 过滤、
//! 会读 `BoundingRectangle`。
//!
//! 这个工具就是那个客户端：它建一个 `IUIAutomation`、找到目标窗口、
//! 把它的元素树整棵 dump 出来。它跟 `Inspect.exe` 等价，但能在 CI/脚本里跑。
//!
//! # 用法
//!
//! ```text
//! cargo run -p zephyra-uia --example dump_tree -- <窗口标题片段>
//! ```
//!
//! 不给参数时 dump 整个桌面（元素会非常多，建议给个标题片段）。

#![cfg(windows)]

use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationTogglePattern, TreeScope_Children, TreeScope_Descendants, UIA_InvokePatternId,
    UIA_TogglePatternId,
};

/// 元素名（`BSTR` → `String`）。
fn bstr(b: windows::core::BSTR) -> String {
    b.to_string()
}

fn main() -> windows::core::Result<()> {
    // UIA 客户端要在 STA 里跑。
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let filter = std::env::args().nth(1);
    let filter = filter.as_deref().unwrap_or("");

    // SAFETY：COM 已初始化；`CUIAutomation8` 是系统提供的单例。
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)? };

    // 找目标窗口：桌面的子元素里，标题里含 `filter` 的那个。
    //
    // 过滤串为空时**列出全部**桌面窗口 —— 否则「没找到」这件事没法排查：
    // 到底是窗口不在这儿，还是标题对不上。
    let root = unsafe { automation.GetRootElement()? };
    let cond = unsafe { automation.CreateTrueCondition()? };
    let children = unsafe { root.FindAll(TreeScope_Children, &cond)? };
    let mut target: Option<IUIAutomationElement> = None;
    for i in 0..unsafe { children.Length()? } {
        let el = unsafe { children.GetElement(i)? };
        let name = bstr(unsafe { el.CurrentName()? });
        if filter.is_empty() || name.contains(filter) {
            println!("窗口：{name}");
            if target.is_none() {
                target = Some(el);
            }
            if !filter.is_empty() {
                break;
            }
        }
    }
    let Some(target) = target else {
        println!("没找到标题含 {filter:?} 的窗口");
        return Ok(());
    };

    // 客户端到底在跟**谁**说话？
    //
    // 这条很重要：窗口「看得见」不等于「跟我们的提供者说话」—— 系统默认的
    // HWND 代理也会给出一个元素（名字就是窗口标题）。只有 `ProviderDescription`
    // 能区分：我们的自报 "ZephyraUI"，系统代理报的是 Microsoft。
    let provider = bstr(unsafe { target.CurrentProviderDescription()? });
    println!("提供者：{provider}");

    let cond = unsafe { automation.CreateTrueCondition()? };
    let all = unsafe { target.FindAll(TreeScope_Descendants, &cond)? };
    let n = unsafe { all.Length()? };
    println!("元素数：{n}");
    println!(
        "{:<28} {:<10} {:<24} {:<7} {:<12} {:<7}",
        "Name", "Type", "AutomationId", "Enabled", "Toggle", "Invoke"
    );
    for i in 0..n {
        let el = unsafe { all.GetElement(i)? };
        let name = bstr(unsafe { el.CurrentName()? });
        let id = bstr(unsafe { el.CurrentAutomationId()? });
        let ctype = unsafe { el.CurrentControlType()? }.0;
        let enabled = unsafe { el.CurrentIsEnabled()? }.as_bool();
        let toggle = match unsafe {
            el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
        } {
            Ok(p) => match unsafe { p.CurrentToggleState()? }.0 {
                0 => "Off".to_string(),
                1 => "On".to_string(),
                2 => "Indeterminate".to_string(),
                other => format!("?{other}"),
            },
            Err(_) => "-".to_string(),
        };
        // Invoke 图案：按钮那一路（`IInvokeProvider`）。
        let invoke = match unsafe {
            el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
        } {
            Ok(_) => "yes".to_string(),
            Err(_) => "-".to_string(),
        };
        println!(
            "{:<28} {:<10} {:<24} {:<7} {:<12} {:<7}",
            name, ctype, id, enabled, toggle, invoke
        );
    }
    Ok(())
}
