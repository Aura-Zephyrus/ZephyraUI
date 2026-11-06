//! 窗口标识与创建描述。

use zephyra_app_config::{AppConfig, WindowOpts};

/// 本进程内一个窗口的稳定标识。
///
/// 与 `winit::window::WindowId` 的区别：那个是 winit 分配的不透明值，窗口销毁后
/// 可能被复用；本类型是单调递增的，销毁后**永不复用**。应用侧持有的是
/// [`crate::WindowHandle`]，不需要直接碰它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowKey(pub(crate) u64);

impl WindowKey {
    /// 数值形式（日志 / 序列化用）。
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// 一个**不来自** [`crate::WindowSet`] 的标识。
    ///
    /// 单窗口兼容路径（直接 `HostWindow::new_lenient_*` 后交给
    /// `winit::ApplicationHandler`）没有窗口集合，但仍需要一个键来满足宿主契约。
    /// 取 `u64::MAX` 是为了与集合自动分配的单调键（从 0 起）不撞。
    pub const fn unspecified() -> Self {
        Self(u64::MAX)
    }
}

impl std::fmt::Display for WindowKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "window#{}", self.0)
    }
}

/// 创建一个窗口的描述。
///
/// # 配置从哪来
///
/// [`WindowSpec::resolve`] 把这里的显式字段叠在 `ZephyraUI.json` 之上：
///
/// 1. 本结构体里显式写的字段；
/// 2. `windows.<name>` 命名定义（[`WindowSpec::named`]）；
/// 3. `window` 节（主窗口默认值）；
/// 4. 内置默认值。
///
/// # 为什么 owner 不放进 JSON
///
/// owner 是**运行期关系**（"这个窗口属于刚创建出来的那个"），不是静态外观。
/// 写进 JSON 只能按名字引用，而同一个名字可能被创建多次。所以它只在这里由代码指定。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowSpec {
    /// `windows.<name>` 节的名字。`None` = 主窗口定义。
    pub name: Option<String>,
    /// 标题覆盖。
    pub title: Option<String>,
    /// 初始客户区尺寸（物理像素）覆盖。
    pub size: Option<(u32, u32)>,
    /// 拥有者窗口。拥有者关闭时本窗口级联关闭。
    pub owner: Option<WindowKey>,
    /// 工具窗口样式（无任务栏项、小标题栏）。
    pub tool_window: bool,
    /// 始终置顶。
    pub always_on_top: bool,
    /// 可调整大小覆盖。
    pub resizable: Option<bool>,
    /// 透明覆盖。
    pub transparent: Option<bool>,
    /// 无边框覆盖。
    pub frameless: Option<bool>,
}

impl WindowSpec {
    /// 主窗口定义（`window` 节）。
    pub fn main() -> Self {
        Self::default()
    }

    /// 按 `windows.<name>` 定义创建。
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    /// 覆盖标题。
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// 覆盖初始客户区尺寸（物理像素）。
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }

    /// 指定拥有者窗口。
    pub fn owner(mut self, owner: WindowKey) -> Self {
        self.owner = Some(owner);
        self
    }

    /// 工具窗口样式。
    pub fn tool_window(mut self) -> Self {
        self.tool_window = true;
        self
    }

    /// 始终置顶。
    pub fn always_on_top(mut self) -> Self {
        self.always_on_top = true;
        self
    }

    /// 覆盖可调整大小。
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = Some(resizable);
        self
    }

    /// 把描述解析成后端可用的 [`WindowOpts`]。
    pub fn resolve(&self, config: &AppConfig) -> WindowOpts {
        let mut opts = match &self.name {
            Some(name) => WindowOpts::named(config, name),
            None => WindowOpts::main(config),
        };
        if let Some(title) = &self.title {
            opts.title = Some(title.clone());
        }
        if let Some(size) = self.size {
            opts.size = Some(size);
        }
        if let Some(transparent) = self.transparent {
            opts.transparent = Some(transparent);
        }
        if let Some(frameless) = self.frameless {
            opts.frameless = Some(frameless);
        }
        if let Some(resizable) = self.resizable {
            opts.resizable = Some(resizable);
        }
        if self.tool_window {
            opts.tool_window = Some(true);
        }
        if self.always_on_top {
            opts.always_on_top = Some(true);
        }
        opts
    }
}
