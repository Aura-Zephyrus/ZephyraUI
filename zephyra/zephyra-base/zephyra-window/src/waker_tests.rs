//! P0.3 纯逻辑门禁：无窗口、无事件循环。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::AppWaker;

fn assert_send_sync_clone<T: Send + Sync + Clone>() {}

/// `AppWaker` 必须能 `move` 进 `std::thread::spawn` —— 这是 P0.3 的全部意义。
#[test]
fn app_waker_is_send_sync_clone() {
    assert_send_sync_clone::<AppWaker>();
}

#[test]
fn post_drains_in_fifo_order_and_counts_wakes() {
    let alive = Arc::new(AtomicBool::new(true));
    let waker = AppWaker::new(None, alive);
    waker.post(1u32);
    waker.post(2u32);
    waker.wake();

    let d = waker.drain();
    assert_eq!(d.wakes.len(), 3, "post ×2 + wake ×1 = 3 次唤醒");
    assert_eq!(d.wakes, vec![1, 2, 3], "唤醒序号必须单调");
    assert_eq!(
        d.messages
            .iter()
            .map(|m| *m.downcast_ref::<u32>().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "消息必须按投递顺序取出"
    );

    // 排空后再取是空的。
    let d2 = waker.drain();
    assert!(d2.wakes.is_empty() && d2.messages.is_empty());
}

#[test]
fn post_after_close_is_silently_dropped() {
    let alive = Arc::new(AtomicBool::new(true));
    let waker = AppWaker::new(None, alive.clone());
    alive.store(false, std::sync::atomic::Ordering::Relaxed);

    waker.post(42u32);
    waker.wake();
    let d = waker.drain();
    assert!(d.wakes.is_empty() && d.messages.is_empty());
}

#[test]
fn clones_share_the_same_queue() {
    let alive = Arc::new(AtomicBool::new(true));
    let a = AppWaker::new(None, alive);
    let b = a.clone();
    a.post(7u32);
    let d = b.drain();
    assert_eq!(d.messages.len(), 1, "clone 必须共享同一条队列");
}
