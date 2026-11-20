//! dev-only 的资源热重载设施。
//!
//! # 严格的 dev / release 隔离
//!
//! 本模块在 [`crate`] 的 `lib.rs` 里由 `#[cfg(debug_assertions)]` 声明：
//! **release 构建里这些代码根本不存在**，不是「运行时开关关掉了」。
//! release 的资源加载路径（`include_bytes!` / 一次性 parse）保持原样，
//! 不经过这里，也不读取任何 `ZEPHYRA_DEV_*` 环境变量。
//!
//! 这里只提供「检测变化 + 可替换持有者」的通用件；「拿到变化后怎么重建
//! 样式表 / UI」由 app 层决定（它才知道自己的文件路径与 IR 形态）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::{Instant, SystemTime};

use zephyra_core::node::Node;
use zephyra_core::styler::Styler;

/// 轮询间隔。与 CLI `dev` 同量级。
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// 增量文件监视器：稳态只 `stat` 已知文件，目录 mtime 变化才重扫。
///
/// 返回的是**变化过的路径**（新增 / 修改 / 删除），app 据此决定重载哪一份资源。
pub struct AssetWatcher {
    roots: Vec<PathBuf>,
    dirs: HashMap<PathBuf, Option<SystemTime>>,
    files: HashMap<PathBuf, SystemTime>,
    last_poll: Instant,
}

impl AssetWatcher {
    /// `roots` 为要递归监视的目录（通常是样式目录与 UI 源目录）。
    pub fn new(roots: &[PathBuf]) -> Self {
        let mut this = Self {
            roots: roots.to_vec(),
            dirs: HashMap::new(),
            files: HashMap::new(),
            last_poll: Instant::now(),
        };
        this.rescan();
        this
    }

    fn rescan(&mut self) {
        self.dirs.clear();
        self.files.clear();
        let roots = self.roots.clone();
        for root in &roots {
            collect(root, &mut self.dirs, &mut self.files, 0);
        }
    }

    /// 每帧调用；未到轮询间隔返回空列表。
    ///
    /// 有增删时重扫并把差异返回；原地写入时按 mtime 找出具体文件。
    pub fn poll(&mut self) -> Vec<PathBuf> {
        if self.last_poll.elapsed() < POLL_INTERVAL {
            return Vec::new();
        }
        self.last_poll = Instant::now();

        let mut structure_changed = false;
        for (path, stamp) in &mut self.dirs {
            let now = mtime(path);
            if now != *stamp {
                *stamp = now;
                structure_changed = true;
            }
        }
        if structure_changed {
            let before: Vec<PathBuf> = self.files.keys().cloned().collect();
            self.rescan();
            let after: Vec<PathBuf> = self.files.keys().cloned().collect();
            let mut changed: Vec<PathBuf> = before
                .iter()
                .chain(after.iter())
                .filter(|p| before.contains(p) != after.contains(p))
                .cloned()
                .collect();
            changed.sort();
            changed.dedup();
            return changed;
        }

        let mut changed = Vec::new();
        let mut missing = Vec::new();
        for (path, stamp) in &mut self.files {
            match mtime(path) {
                Some(now) if now != *stamp => {
                    *stamp = now;
                    changed.push(path.clone());
                }
                Some(_) => {}
                None => missing.push(path.clone()),
            }
        }
        for path in missing {
            self.files.remove(&path);
            changed.push(path);
        }
        changed.sort();
        changed.dedup();
        changed
    }
}

fn collect(
    dir: &Path,
    dirs: &mut HashMap<PathBuf, Option<SystemTime>>,
    files: &mut HashMap<PathBuf, SystemTime>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 16;
    if depth > MAX_DEPTH {
        return;
    }
    dirs.entry(dir.to_path_buf()).or_insert_with(|| mtime(dir));
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let name = entry.file_name();
            let ignored = matches!(
                name.to_str(),
                Some("target" | ".git" | "node_modules" | "gen" | ".idea" | ".vscode")
            );
            if !ignored {
                collect(&path, dirs, files, depth + 1);
            }
            continue;
        }
        if ft.is_file() && watched(&path) {
            if let Some(t) = mtime(&path) {
                files.insert(path, t);
            }
        }
    }
}

fn watched(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("zstyle" | "zypui")
    )
}

type ApplyFn = Arc<dyn Fn(&mut Node) + Send + Sync>;

/// 可替换 Styler 的远端把手：**只更新闭包，不动宿主手里的 `Box<dyn Styler>`**。
#[derive(Clone)]
pub struct StylerHandle(Arc<RwLock<ApplyFn>>);

impl StylerHandle {
    pub fn set(&self, apply: ApplyFn) {
        let mut slot = self.0.write().unwrap_or_else(|e| e.into_inner());
        *slot = apply;
    }
}

/// 装进宿主的 Styler：每次 `apply` 从共享槽里取最新的闭包。
pub struct ReloadableStyler(Arc<RwLock<ApplyFn>>);

impl Styler for ReloadableStyler {
    fn apply(&mut self, root: &mut Node) {
        let apply = self.0.read().unwrap_or_else(|e| e.into_inner());
        apply(root);
    }
}

/// 造一对「装进宿主的 Styler」与「用来替换它的把手」。
pub fn reloadable_styler(initial: ApplyFn) -> (ReloadableStyler, StylerHandle) {
    let slot = Arc::new(RwLock::new(initial));
    (ReloadableStyler(slot.clone()), StylerHandle(slot))
}

/// 泛型可替换持有者：app 用它替换 `Ui` / `UiRuntime` 这类本来放在 `OnceLock`
/// 里的实例。克隆出来的把手共享同一份值。
pub struct Reloadable<T>(Arc<RwLock<T>>);

impl<T> Clone for Reloadable<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Reloadable<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(RwLock::new(value)))
    }

    pub fn get(&self) -> RwLockReadGuard<'_, T> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn replace(&self, value: T) {
        let mut slot = self.0.write().unwrap_or_else(|e| e.into_inner());
        *slot = value;
    }
}

/// 文件变化回调（type alias 只为压掉 clippy 的 type-complexity）。
type ChangeCallback = Box<dyn FnMut(&[PathBuf])>;

/// 宿主每帧轮询的 dev 重载器。
pub struct DevReloader {
    watcher: AssetWatcher,
    on_change: ChangeCallback,
}

impl DevReloader {
    pub fn new(roots: &[PathBuf], on_change: impl FnMut(&[PathBuf]) + 'static) -> Self {
        eprintln!(
            "[zephyra-dev] hot reload armed for .zstyle / .zypui in {} dir(s)",
            roots.len()
        );
        Self {
            watcher: AssetWatcher::new(roots),
            on_change: Box::new(on_change),
        }
    }

    pub fn poll(&mut self) {
        let changed = self.watcher.poll();
        if !changed.is_empty() {
            // 统一的「检测到变化」提示：任何装了 DevReloader 的 app 都有这行，
            // 后面紧跟 app 自己打印的「重载成功 / 失败」结果。
            eprintln!(
                "[zephyra-dev] assets changed: {}",
                changed
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            (self.on_change)(&changed);
        }
    }
}

/// 从 `ZEPHYRA_DEV_ASSETS` 读目录列表（`;` 或 `:` 分隔）。
///
/// release 构建里没有任何代码调用这个函数 —— 环境变量不是运行时开关。
pub fn assets_from_env() -> Vec<PathBuf> {
    let Ok(raw) = std::env::var("ZEPHYRA_DEV_ASSETS") else {
        return Vec::new();
    };
    // split_paths 用平台分隔符（Windows ';' / Unix ':'），不会把 C:\ 拆坏。
    std::env::split_paths(&raw)
        .filter(|p| !p.as_os_str().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "zephyra-dev-reload-{tag}-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ))
    }

    #[test]
    fn watcher_reports_modified_and_added_files() {
        let root = scratch("assets");
        std::fs::create_dir_all(&root).unwrap();
        let style = root.join("a.zstyle");
        std::fs::write(&style, "a { }\n").unwrap();
        let mut watcher = AssetWatcher::new(std::slice::from_ref(&root));

        assert!(watcher.poll().is_empty(), "刚建好不该报变化");
        std::thread::sleep(POLL_INTERVAL);

        std::fs::write(&style, "a { width: 1px; }\n").unwrap();
        let changed = watcher.poll();
        assert!(changed.contains(&style), "改文件应被检出: {changed:?}");

        let added = root.join("b.zypui");
        std::fs::write(&added, "<component/>\n").unwrap();
        std::thread::sleep(POLL_INTERVAL);
        let changed = watcher.poll();
        assert!(changed.contains(&added), "新文件应被检出: {changed:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn styler_handle_swaps_the_installed_styler() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let c1 = calls.clone();
        let (mut styler, handle) = reloadable_styler(Arc::new(move |_root: &mut Node| {
            c1.fetch_add(1, Ordering::SeqCst);
        }));
        let c2 = calls.clone();
        handle.set(Arc::new(move |_root: &mut Node| {
            c2.fetch_add(100, Ordering::SeqCst);
        }));

        let element = zephyra_core::element::Element::new(zephyra_core::id::NodeTag::Box);
        let mut node = Node::from_element(&element);
        Styler::apply(&mut styler, &mut node);
        assert_eq!(calls.load(Ordering::SeqCst), 100, "应使用替换后的闭包");
    }

    #[test]
    fn watcher_reports_deleted_files() {
        let root = scratch("deleted");
        std::fs::create_dir_all(&root).unwrap();
        let style = root.join("gone.zstyle");
        std::fs::write(&style, "a { }\n").unwrap();
        let mut watcher = AssetWatcher::new(std::slice::from_ref(&root));
        assert!(watcher.poll().is_empty());

        std::fs::remove_file(&style).unwrap();
        std::thread::sleep(POLL_INTERVAL);
        let changed = watcher.poll();
        assert!(changed.contains(&style), "删除应被检出: {changed:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reloadable_replaces_value_and_clones_share_it() {
        let cell = Reloadable::new(1u32);
        let clone = cell.clone();
        assert_eq!(*cell.get(), 1);
        clone.replace(7);
        assert_eq!(*cell.get(), 7, "克隆把手替换后原持有者应看到新值");
    }

    #[test]
    fn dev_reloader_invokes_callback_only_on_change() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = scratch("callback");
        std::fs::create_dir_all(&root).unwrap();
        let style = root.join("a.zstyle");
        std::fs::write(&style, "a { }\n").unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let mut reloader = DevReloader::new(std::slice::from_ref(&root), move |paths| {
            if !paths.is_empty() {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });

        reloader.poll();
        assert_eq!(calls.load(Ordering::SeqCst), 0, "无变化不应调回调");
        std::thread::sleep(POLL_INTERVAL);
        std::fs::write(&style, "a { width: 2px; }\n").unwrap();
        std::thread::sleep(POLL_INTERVAL);
        reloader.poll();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "有变化应调一次回调");

        let _ = std::fs::remove_dir_all(&root);
    }
}
