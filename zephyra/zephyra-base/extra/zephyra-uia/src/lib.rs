//! UI Automation（UIA）接入 —— 让屏幕阅读器与自动化脚本**看得见**这个界面。
//!
//! # 这一层解决什么
//!
//! 自绘界面在 Windows 上对无障碍工具是**不存在**的：屏幕阅读器读不到任何
//! 东西，Narrator / Inspect 看不见控件，自动化脚本没法按控件操作。要让它们
//! 看得见，窗口必须对 UIA 交出一个**元素树**。
//!
//! 本 crate 交出的是一棵**原始提供者**（raw provider）树：
//!
//! ```text
//! 应用的状态模型（复选框：勾上 / 半选 / 禁用）
//!         │  宿主每帧从保留树派生
//!         ▼
//!   Snapshot（纯数据，平台中立）──► COM 提供者 ──► UIA 客户端
//!         ▲                              │
//!         └──── 动作排队 ◄────────────────┘
//! ```
//!
//! # 三层分工
//!
//! | 层 | 住在哪 | 职责 |
//! |---|---|---|
//! | 数据模型 | [`model`]（本 crate，平台中立） | 角色、开关状态、名字、矩形 |
//! | 派生规则 | [`rules`]（本 crate，平台中立） | 从保留树推出上面那些 |
//! | COM 接线 | [`provider`] / [`subclass`]（仅 Windows） | 把数据翻译成 UIA 的形状 |
//!
//! 宿主（`zephyra-winit*`）负责两件事：**每帧发布快照**、**取走动作**。
//! 应用本身**不需要**为无障碍写任何代码 —— 无障碍树是从**画出来的那棵树**
//! 派生的，所以它不会与画面漂移。
//!
//! # 现状（v1）
//!
//! 暴露的角色：复选框（Toggle 图案）、按钮（Invoke 图案）、分组、窗口根。
//! 单选钮与开关**暂未**暴露 —— 它们的状态不在控件自身的分类名里（单选钮的
//! 「选中」落在 `checked` 上但语义是 SelectionItem，开关的状态落在子节点
//! `switch-track` 上），要暴露得各自配一套图案。复选框族（本次的范围）
//! 是完整的。
//!
//! # 调试开关
//!
//! `ZEPHYRA_UIA_TRACE=1` 让服务端的每一次 UIA 调用都打一行日志
//! （`[uia] …`）。排查「客户端读不到某个属性 / 图案」时它是唯一的抓手 ——
//! UIA 的失败大多是**静默**的（元素在、名字在、唯独某个能力读不出来），
//! 没有这行日志就只能靠猜。
//!
//! 真机对照：
//!
//! ```bash
//! ZEPHYRA_UIA_TRACE=1 ./gallery.exe --route reference --family 1
//! cargo run -p zephyra-uia --example dump_tree -- "ZephyraUI (D3D12)"
//! ```

pub mod bridge;
pub mod model;
pub mod rules;

#[cfg(windows)]
pub mod provider;
#[cfg(windows)]
pub mod subclass;

pub use bridge::Bridge;
pub use model::{Element, Role, Snapshot, ToggleState};
pub use rules::WindowInfo;

use std::sync::{Arc, Mutex, RwLock};

/// 当前发布出去的快照。
///
/// 读侧（COM）拿 `Arc` 克隆，**不持锁**穿过调用；写侧（宿主）换指针。
/// 于是「渲染线程正在换快照」与「UIA 客户端正在读」不会互相等锁 ——
/// 客户端最多读到**旧一帧**的快照，那在 UIA 的语义里是可接受的（它本来
/// 就是「上一次绘制的样子」）。
static SNAPSHOT: RwLock<Option<Arc<Snapshot>>> = RwLock::new(None);

/// 排队中的动作（UIA → 应用）。
///
/// UIA 的动作是**外部发起**的：屏幕阅读器说「把这个勾上」，而状态住在应用里。
/// COM 侧不能回头去调应用（跨线程、跨借用、还可能重入渲染），所以只排队，
/// 由宿主在帧开头取走、按**正常点击路径**施加。
static ACTIONS: Mutex<Vec<Action>> = Mutex::new(Vec::new());

/// 一个来自无障碍侧的动作请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 「把这个复选框 / 开关切一下」（Toggle 图案）。
    Toggle(String),
    /// 「按一下这个按钮」（Invoke 图案）。
    Invoke(String),
    /// 「选中这个单选钮」（SelectionItem 图案）。
    ///
    /// 与 [`Action::Invoke`] 走到**同一条**应用路径（一次点击），但语义不同：
    /// 单选钮的「选中」由应用按单选组规则处理（同组其余取消），
    /// 而 Invoke 只是「按一下」。分成两个变体是为了让诊断日志读得出
    /// 「这是屏读器在挑单选钮」而不是「有人在按按钮」。
    Select(String),
    /// 「把焦点移到这个元素」。
    Focus(String),
}

impl Action {
    /// 动作针对的控件 key。
    pub fn key(&self) -> &str {
        match self {
            Action::Toggle(k) | Action::Invoke(k) | Action::Select(k) | Action::Focus(k) => k,
        }
    }
}

/// 发布一份新的无障碍树快照（宿主每帧调）。
///
/// 旧快照立刻作废 —— COM 侧还握着旧 `Arc` 的读者会继续看到旧值，直到它
/// 释放为止。这是刻意的：无障碍客户端本来就在读「上一次绘制的界面」。
///
/// # 顺带抛「状态变了」事件
///
/// 发布意味着「这一帧画出来的东西变了」。开关状态变了就抛一个
/// `AutomationPropertyChanged`（属性 `ToggleToggleState`）—— 屏读器正是靠它
/// 知道「复选框被勾上了」，否则它只能靠轮询。
///
/// 比较是**按 key** 的：同一个 key 的开关状态变了才抛。元素从树上消失
/// （换页了）不抛 —— 那是树结构的变化，由 UIA 自己的树遍历感知。
pub fn publish(snapshot: Snapshot) {
    let snapshot = Arc::new(snapshot);
    let previous = {
        let mut slot = SNAPSHOT.write().unwrap_or_else(|e| e.into_inner());
        slot.replace(Arc::clone(&snapshot))
    };
    #[cfg(windows)]
    if let Some(previous) = previous {
        raise_toggle_changes(&previous, &snapshot);
    }
}

/// 按 key 比较两份快照，把**开关状态变了**的元素报一遍。
#[cfg(windows)]
fn raise_toggle_changes(previous: &Snapshot, current: &Snapshot) {
    for el in &current.elements {
        let Some(new_state) = el.toggle else { continue };
        if el.key.is_empty() {
            continue;
        }
        let old_state = previous.find_by_key(&el.key).and_then(|e| e.toggle);
        let old_state = match old_state {
            Some(s) => s,
            // 新出现的元素：没有「旧状态」可言。
            // 抛「从不曾存在到存在」没有意义（UIA 用结构变化表达这件事），
            // 所以不抛。
            None => continue,
        };
        if old_state != new_state {
            raise_toggle_state_changed(&el.key, old_state, new_state);
        }
    }
}

/// 当前快照（没有发布过则 `None`）。
pub fn current() -> Option<Arc<Snapshot>> {
    SNAPSHOT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(Arc::clone)
}

/// 排一个动作（COM 侧调）。
pub(crate) fn push_action(action: Action) {
    let mut slot = ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
    slot.push(action);
}

/// 取走排队中的动作（宿主每帧开头调）。
///
/// 返回**并清空**队列 —— 动作是一次性的：UIA 的 Toggle 说的是「切一下」，
/// 不是「保持在这个状态」。
pub fn take_actions() -> Vec<Action> {
    let mut slot = ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *slot)
}

/// 排队中的动作数（诊断用）。
pub fn pending_action_count() -> usize {
    ACTIONS.lock().unwrap_or_else(|e| e.into_inner()).len()
}

// ----------------------------------------------------------- Windows 接线

/// 装 UIA 失败。
///
/// # 为什么不直接返回 `windows::core::Error`
///
/// 宿主（D3D11 / D3D12）**不一定**直接依赖 `windows` crate —— 让本 crate 的
/// 公开 API 带上 `windows::core::Error` 就等于把它泄漏出去，宿主要么多一个依赖，
/// 要么在自己的签名里写一个自己都不认识的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// 窗口过程子类化失败（`SetWindowLongPtrW` 返回 0 且 `GetLastError` 非零）。
    WindowProc(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::WindowProc(code) => {
                write!(f, "装 UIA 窗口过程失败（Windows 错误 {code}）")
            }
        }
    }
}

impl std::error::Error for Error {}

/// 装上 UIA：把窗口过程子类化，接管 `WM_GETOBJECT`。
///
/// # 与别的窗口过程子类如何相处
///
/// 本仓已有两处子类化（`zephyra-platform-win` 的实时 resize）。它们都是
/// `SetWindowLongPtrW(GWLP_WNDPROC)` —— **同一把锁**，所以顺序有意义：
///
/// - **先**装 resize、**后**装 UIA ⇒ 消息链是
///   `UIA → live_resize → winit`，两者都工作 ✓；
/// - 反过来 ⇒ UIA 的消息先到 resize，resize 只处理 `WM_SIZE`、其余转给
///   原过程 —— 那条链仍然通 ✓（因为 resize 装的时候拿到的「原过程」是
///   winit 的，UIA 装在它**前面**，于是 resize 转发的目标里没有 UIA）。
///
/// 所以：**UIA 装在最后**。
///
/// # 失败怎么办
///
/// `SetWindowLongPtrW` 失败时返回错误，宿主应当**继续跑**（只是没有无障碍）——
/// 无障碍是附加能力，不该让整个窗口开不起来。
#[cfg(windows)]
pub fn install(hwnd: isize) -> Result<(), Error> {
    ensure_com();
    HOST_HWND.store(hwnd, std::sync::atomic::Ordering::Release);
    subclass::install(hwnd)
}

/// 摘掉 UIA 的窗口过程子类（窗口销毁前调）。
#[cfg(windows)]
pub fn uninstall(hwnd: isize) {
    subclass::uninstall(hwnd);
    HOST_HWND.store(0, std::sync::atomic::Ordering::Release);
    release_com();
}

/// 宿主窗口句柄（`HostRawElementProvider` 要用）。
///
/// 微软文档对 `HostRawElementProvider` 的要求是硬性的：
///
/// > "A host provider **must** be returned in the following cases: when the element
/// > is a **fragment root**, when the element is a simple element (such as a push
/// > button), and when the provider is a repositioning placeholder."
///
/// 也就是说：**片段根必须交出宿主窗口的提供者**（`UiaHostProviderFromHwnd`）。
/// UIA 靠它把「我们画的控件」与「这个窗口」挂上勾 —— 窗口标题、进程号、
/// 原生句柄、以及**顶层元素的 runtime id** 都从那儿来。
///
/// # 为什么是一个全局
///
/// 本 crate 目前只支持**一个**宿主窗口（`install` 收一个 hwnd）。
/// 片段根的提供者不知道自己是哪个窗口的 —— 它只知道「我就是根」。
#[cfg(windows)]
static HOST_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 宿主窗口句柄（没装过 UIA 时是 `None`）。
#[cfg(windows)]
pub fn host_hwnd() -> Option<isize> {
    let h = HOST_HWND.load(std::sync::atomic::Ordering::Acquire);
    if h == 0 {
        None
    } else {
        Some(h)
    }
}

// ------------------------------------------------------------ COM 公寓

// 本线程被本 crate 初始化 COM 的次数（对应 `CoUninitialize` 的次数）。
//
// COM 的公寓是**每线程**的 —— 不是每窗口。窗口可以有多个，但它们通常在
// 同一个 UI 线程上。
#[cfg(windows)]
thread_local! {
    static COM_REFS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// 确保当前线程有一个 COM 公寓。
///
/// # 为什么必须有
///
/// UIA 的提供者是**跨进程**被调用的：客户端在别的进程里，它发起的每一次
/// 调用都要经 COM 封送（marshal）过来。而封送**要求线程已经初始化了 COM**。
///
/// 没有这一步的症状极其隐蔽：
///
/// - `WM_GETOBJECT` 正常交出了根提供者，窗口在客户端里**看得见**；
/// - 客户端问「你的片段根是谁」，我们答了 —— 但那个接口指针**封送不过去**
///   （没有 COM 公寓 ⇒ `CO_E_NOTINITIALIZED`）；
/// - 客户端于是**当场放弃整棵树**：`FindAll` 返回 0 个元素，一个控件都读不到。
///
/// 服务端这边**一个错都没有**，日志里只有一句「FragmentRoot 被调了」。
///
/// # 选哪个公寓
///
/// `COINIT_APARTMENTTHREADED`（STA）—— UI 线程本来就该是 STA：它有消息
/// 泵，而 STA 的封送回调正是靠消息泵转发的。线程若已经是别的公寓
/// （`RPC_E_CHANGED_MODE`），说明别人先初始化了 —— 那也算「就绪」，只是这份
/// 引用不是我们的，不记进 [`COM_REFS`]。
#[cfg(windows)]
fn ensure_com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    COM_REFS.with(|refs| {
        if refs.get() > 0 {
            refs.set(refs.get() + 1);
            return;
        }
        // SAFETY：在 UI 线程上、装 UIA 之前调用。COM 的初始化是每线程一次性的，
        // 重复调用是允许的（引用计数 +1）。
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_ok() {
            // S_OK / S_FALSE 都算我们持有一份引用。
            refs.set(1);
        }
    });
}

/// 放掉本 crate 在当前线程上持有的 COM 引用。
#[cfg(windows)]
fn release_com() {
    use windows::Win32::System::Com::CoUninitialize;

    COM_REFS.with(|refs| {
        let n = refs.get();
        if n == 0 {
            return;
        }
        refs.set(n - 1);
        if n - 1 == 0 {
            // SAFETY：与 `ensure_com` 的 `CoInitializeEx` 一一配对，且同线程。
            unsafe { CoUninitialize() };
        }
    });
}

/// 抛一个「开关状态变了」的无障碍事件。
///
/// UIA 没有独立的「Toggle 状态变了」事件 —— 客户端听的是
/// `AutomationPropertyChanged`，属性是 `ToggleToggleState`。
///
/// `old` / `new` 都要报：客户端据此知道是「勾上了」还是「半选了」——
/// 只报新的，客户端只能知道「变了」，不知道变成什么。
///
/// 返回是否真的抛了（找不到元素 / 不在 Windows 上则 `false`）。
#[cfg(windows)]
pub fn raise_toggle_state_changed(key: &str, old: ToggleState, new: ToggleState) -> bool {
    provider::raise_toggle_state_changed(key, old, new)
}

/// 从窗口句柄取一份 [`WindowInfo`]（窗口标题 + 客户区的屏幕坐标矩形）。
///
/// # 为什么坐标要「屏幕」的
///
/// UIA 的 `BoundingRectangle` 是**屏幕坐标**，而布局出来的矩形是**客户区坐标**
/// —— 两者差一个窗口偏移（客户区左上角在屏幕上的位置）。这个偏移只有 Win32
/// 知道（`ClientToScreen`），所以由宿主在发布快照前算好。
///
/// 拿不到（无效句柄 / 非 Win32）时退化成「原点 + 零尺寸」——那意味着所有元素
/// 的矩形都在屏幕原点处，无障碍树仍可用，只是位置信息不准。
#[cfg(windows)]
pub fn window_info(hwnd: isize) -> WindowInfo {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowTextW};

    let h = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);

    // 客户区左上角的屏幕坐标。
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY：`h` 是有效句柄时 `ClientToScreen` 写回原点；失败时它返回 FALSE
    // 且不写 —— 那时保留 (0,0)。
    let ok = unsafe { ClientToScreen(h, &mut origin) }.as_bool();

    // 客户区尺寸。
    let mut rc = RECT::default();
    let sized = unsafe { GetClientRect(h, &mut rc) }.is_ok();

    // 窗口标题。
    let mut buf = [0u16; 256];
    let len = unsafe { GetWindowTextW(h, &mut buf) };
    let name = if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    };

    WindowInfo {
        name,
        client: if ok && sized {
            zephyra_core::core::Rect::new(
                origin.x as f32,
                origin.y as f32,
                (rc.right - rc.left) as f32,
                (rc.bottom - rc.top) as f32,
            )
        } else {
            zephyra_core::core::Rect::new(0.0, 0.0, 0.0, 0.0)
        },
    }
}

// ----------------------------------------------------------- 测试用的串行锁

/// 全局状态（快照 + 动作队列）是**进程级单例** ⇒ 改它们的测试必须串行。
///
/// 这不是「测试写得不乖」而是**被测对象本身就是进程级单例**（快照只有一个、
/// 动作队列只有一个）。所以串行化是对被测语义的忠实反映，不是给测试打补丁。
///
/// 用 `unwrap_or_else(|e| e.into_inner())` 而不是 `unwrap()`：某个测试 panic
/// 时锁被毒化，后面的测试应当**继续跑**（它们仍能验证各自的东西），
/// 而不是级联地报「上一个测试挂了」。
#[cfg(test)]
pub(crate) mod test_lock {
    use std::sync::{Mutex, MutexGuard};

    static GLOBAL: Mutex<()> = Mutex::new(());

    /// 拿锁。
    pub fn guard() -> MutexGuard<'static, ()> {
        GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_then_read() {
        let _g = crate::test_lock::guard();
        let mut snap = Snapshot::default();
        snap.elements.push(Element {
            id: 0,
            parent: None,
            key: String::new(),
            name: "w".into(),
            role: Role::Window,
            toggle: None,
            enabled: true,
            focused: false,
            offscreen: false,
            rect: zephyra_core::core::Rect::new(0.0, 0.0, 1.0, 1.0),
            children: vec![],
        });
        publish(snap);
        assert!(current().is_some());
    }

    #[test]
    fn actions_are_taken_once() {
        let _g = crate::test_lock::guard();
        let _ = take_actions();
        push_action(Action::Toggle("check:alpha".into()));
        push_action(Action::Focus("check:beta".into()));
        assert_eq!(pending_action_count(), 2);
        let got = take_actions();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].key(), "check:alpha");
        assert!(take_actions().is_empty());
    }
}
