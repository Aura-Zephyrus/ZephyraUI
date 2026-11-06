//! 渲染后端契约。

use winit::event_loop::ActiveEventLoop;

use zephyra_fx::View;

use crate::handle::WindowHandle;
use crate::host::WindowHost;
use crate::spec::WindowSpec;

/// 一个渲染后端。
///
/// # 为什么用 GAT 而不是 `Box<dyn View>`
///
/// 每个窗口的 `View` 类型可以不同（主窗口 `Gallery`、工具窗口 `Inspector`），
/// 而 `HostWindow<V>` 必须保留具体类型（它要调 `V::view`）。GAT 让
/// `Backend::create::<Gallery>` 与 `Backend::create::<Inspector>` 各返回自己的
/// 具体宿主类型，装进 `Box<dyn WindowHost>` 之后类型才被擦除。
///
/// # 后端持有的是**共享资源**
///
/// 实现通常长这样：
///
/// ```text
/// struct D3d12Backend { shared: Arc<Mutex<Shared>> }
/// ```
///
/// `Shared` 里是设备 / 队列 / 渲染器（PSO、图集、纹理表）。`create` 只新建
/// 本窗口的交换链、DComp visual 与内容树。
pub trait Backend {
    /// 本后端创建的窗口宿主类型。
    type Window<V: View + 'static>: WindowHost + 'static;

    /// 创建一个顶层窗口。
    fn create<V: View + 'static>(
        &mut self,
        event_loop: &ActiveEventLoop,
        view: V,
        spec: &WindowSpec,
        handle: WindowHandle,
    ) -> Result<Self::Window<V>, String>;
}
