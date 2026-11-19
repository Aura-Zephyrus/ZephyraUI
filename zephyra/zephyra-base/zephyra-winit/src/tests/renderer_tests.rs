// 本文件由 renderer.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。

use super::*;

/// P0.1：`HookContext` 的具体上下文必须能向下转型，否则外部钩子（笔迹层）
/// 拿不到立即上下文，画不出任何像素。
#[test]
fn hook_ctx_is_downcastable() {
    use zephyra_render_ext::registration::HookContextExt;

    // 空指针只用于验证类型可转型；不调用访问器就不会解引用。
    let mut ctx = HookCtx {
        context: std::ptr::null(),
        device: std::ptr::null(),
        rtv: std::ptr::null(),
        size: (100.0, 80.0),
        frame_index: 7,
        ext_pool: None,
    };
    let dyn_ctx: &mut dyn zephyra_render_ext::registration::HookContext = &mut ctx;
    assert!(
        dyn_ctx.downcast_mut::<HookCtx>().is_some(),
        "downcast_mut::<HookCtx>() 必须成功 —— 钩子要据此拿立即上下文绘制"
    );
    assert_eq!(dyn_ctx.frame_index(), 7);
    assert_eq!(dyn_ctx.size(), (100.0, 80.0));
}
