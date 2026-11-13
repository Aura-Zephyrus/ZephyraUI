//! 输入路由：指针事件 → 命中测试 → 控件状态变化。
//!
//! # 为什么路由在核心里
//!
//! hover / press 是**保留式** UI 的核心概念：控件自己持有"我正在被悬停"这个状态，
//! 并据此改变外观。它不是"每个应用各写一遍的事件分发"——那样每个应用都会各自实现
//! 捕获、穿透、按下-抬起配对，行为必然不一致。
//!
//! # 事件产出是零分配的
//!
//! 一次输入事件最多产生一条 [`RoutedEvent`]（见 [`MAX_EVENTS_PER_INPUT`] 的推导）。
//! 所以 [`InputRouter::on_pointer`] 往调用方提供的固定容量缓冲里写，**不返回 `Vec`**。
//! 指针移动是每秒上百次的路径，在那里分配一次 `Vec` 就足以在帧时间上留下尖峰。
//!
//! # 悬停变化用 revision 表达，而不是事件
//!
//! 悬停进出可能一次事件里发生两次（从 A 移到 B：先 exit A 再 enter B）。若把它们
//! 也做成事件流，宿主就得维护一份"当前悬停集合"再和控件属性对账。这里让路由器**持有**
//! 状态并用 [`InputRouter::revision`] 暴露版本号：宿主只需比较 revision 是否变化，
//! 变了就重绘。这是保留式 UI 的常规做法（GPUI 的 `Window`、egui 的 `repaint` 都这么做），
//! 因为宿主真正需要的从来不是"发生了什么"，而是"要不要重画"。

use crate::context::Context;
use crate::core::Point;
use crate::hit::{hit_test, HitResult, HitTestRequest};
use crate::id::ElementKey;
use crate::input::gesture::StaticGesture;
use crate::input::manipulation::ManipulationDelta;
use crate::input::pointer::{PointerEvent, PointerPhase};
use crate::input::recognizer::{Gesture, Recognizer};

/// 单个输入事件最多产出的路由事件数。
///
/// 推导：一次 `Up` 至多产生
///
/// 1. `Release`（按下与抬起配对，恒有）；
/// 2. `Click`（抬起落在按下那一节点上）；
/// 3. 静态手势之一（`Tapped` / `DoubleTapped` / `Holding`）—— 互斥，至多一个；
/// 4. 操作结束（`Manipulation` 的「结束」变体）。
///
/// 取 4 是**刚好够**：宁可多留一个空位，也不要在热路径上做长度检查后
/// 静默丢弃事件（那会让一次点击消失且无从排查）。
pub const MAX_EVENTS_PER_INPUT: usize = 4;

/// 一条路由结果。
#[derive(Debug, Clone, PartialEq)]
pub enum RoutedEvent {
    /// 在某节点上按下。
    Press(ElementKey),
    /// 抬起。`pressed` 是按下时记录的节点，`released` 是抬起时命中的节点——
    /// 两者可以不同（按下按钮、拖出去松手），宿主据此决定"取消"还是"提交"。
    Release {
        /// 按下时命中的节点。
        pressed: ElementKey,
        /// 抬起时命中的节点（拖出控件外则缺省）。
        released: Option<ElementKey>,
    },
    /// 点击：按下与抬起落在**同一**节点上。
    Click(ElementKey),
    /// 手势（捏合增量）。
    Gesture(Gesture),
    /// 静态手势（官方指南的「静态手势事件」一档）。
    ///
    /// 与 [`RoutedEvent::Click`] **并存**而不是替代：点击是「按下与抬起落在
    /// 同一节点」，而轻点还有**位移容差**与**时长上限**两条判据 —— 一个
    /// 在按钮上按住 800 ms 再松开，点击是「点击」但**不是**「轻点」。
    Static(StaticGesture, ElementKey),
    /// 操作增量（多指平移 / 缩放 / 旋转 + 速度）。
    ///
    /// 官方指南的操作事件**只给数据、不给形式** —— 「这是捏合还是旋转」
    /// 由应用自己判断。
    Manipulation(ManipulationDelta),
    /// 操作结束（最后一根手指抬起），值是抬手瞬间的速度 —— 惯性的初值。
    ManipulationEnded {
        /// 平移速度（px/秒）。
        velocity_x: f32,
        /// 平移速度（px/秒）。
        velocity_y: f32,
    },
}

/// 固定容量的事件缓冲。
#[derive(Debug, Clone, Default)]
pub struct RoutedEvents {
    items: [Option<RoutedEvent>; MAX_EVENTS_PER_INPUT],
    len: usize,
}

impl RoutedEvents {
    /// 空缓冲。
    pub fn new() -> Self {
        Self {
            items: Default::default(),
            len: 0,
        }
    }

    /// 已产出条数。
    pub fn len(&self) -> usize {
        self.len
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 逐条取出。
    pub fn iter(&self) -> impl Iterator<Item = &RoutedEvent> {
        self.items[..self.len].iter().filter_map(|e| e.as_ref())
    }

    fn clear(&mut self) {
        for slot in &mut self.items[..self.len] {
            *slot = None;
        }
        self.len = 0;
    }

    fn push(&mut self, event: RoutedEvent) {
        if self.len >= MAX_EVENTS_PER_INPUT {
            // 不静默丢弃：超长意味着上面的容量推导错了，那是个必须暴露的 bug。
            // 但热路径不能 panic，所以退化成"忽略"并在 debug 下可见。
            debug_assert!(
                false,
                "路由事件超出 MAX_EVENTS_PER_INPUT={MAX_EVENTS_PER_INPUT}；\
                 单个输入事件最多产生 1 条，出现 3 条说明状态机有 bug"
            );
            return;
        }
        self.items[self.len] = Some(event);
        self.len += 1;
    }
}

/// 输入路由器：手势识别 + 命中测试 + 悬停/按下状态。
///
/// # 捕获（capture）语义
///
/// 按下时记录 `pressed`；随后即使指针移出该节点，抬起仍归它所有（`released` 为
/// `None`）。这与 WinUI / 浏览器一致：按下按钮后横向拖出去松手，应当**取消**而不是
/// 触发另一个按钮。若抬起时才重新命中，最常见的故障是"在 A 上按下、拖到 B 上松开，
/// 于是 B 被点了一下"——那会误触发用户没有点的东西。
#[derive(Clone)]
pub struct InputRouter {
    recognizer: Recognizer,
    /// 当前悬停的可交互节点。
    hovered: Option<ElementKey>,
    /// **主**按下态：最近一次按下的那个节点。
    ///
    /// 应用面（[`InputState::pressed`]）报的是它 —— 多数 UI 只有一个指针。
    ///
    /// ⚠ 多指同时按下时它**只代表一个**；要画全部按下态请用
    /// [`Self::pressed_keys`]。
    pressed: Option<ElementKey>,
    /// 按下态**按指针**记：`(指针 id, 按下的节点)`。
    ///
    /// # 为什么不是一个槽
    ///
    /// 两根手指同时按两个控件时，一个槽会让第二根**覆盖**第一根，而抬起时
    /// 只取走一个 ⇒ 第一根手指**收不到 `Release`** ⇒ 那个控件**卡在按下态**
    /// （永远回不到正常外观）。
    ///
    /// 而且配对会**反**：实测（两指分别按 a / b，先抬 a）产出的是
    /// `Release { pressed: "b", released: "a" }` —— 「按下的是 b、松开的是 a」。
    /// 那会让**错误的控件**收到提交。
    ///
    /// 触屏上「两根手指各按一处」很常见（多指缩放时手指常常各压一处），
    /// 所以这不是边角情形。
    pressed_by_pointer: [Option<(u64, ElementKey)>; crate::input::pointer::MAX_POINTERS],
    /// 悬停/按下状态版本号；宿主比较它决定要不要重绘。
    revision: u64,
    /// 复用的命中测试结果，避免每次指针移动重新分配。
    hit: HitResult,
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl InputRouter {
    /// 新建（无悬停、无按下）。
    pub fn new() -> Self {
        Self {
            recognizer: Recognizer::new(),
            hovered: None,
            pressed: None,
            pressed_by_pointer: std::array::from_fn(|_| None),
            revision: 0,
            hit: HitResult::default(),
        }
    }

    /// 当前**所有**被按下的节点（每个按下中的指针一个）。
    ///
    /// # 为什么需要它
    ///
    /// 多指同时按两个控件时，[`Self::pressed`] 只代表**一个** —— 而应用要画
    /// **两个**按下态。所以这里给出**全部**。
    ///
    /// 绝大多数应用只有一个指针，这个列表长度为 1 —— 于是「按 id 找自己」的
    /// 那条路（而不是「只有一个按下态」）对多指也对。
    pub fn pressed_keys(&self) -> impl Iterator<Item = &ElementKey> {
        self.pressed_by_pointer
            .iter()
            .filter_map(|slot| slot.as_ref().map(|(_, k)| k))
    }

    /// 按下态**按指针**记下。
    ///
    /// 同一根手指重复按下 ⇒ 覆盖它自己那一条（不新增条目）。
    fn set_pressed_for(&mut self, pointer: u64, key: ElementKey) {
        // 优先复用「这根指针」已有的槽；没有就找第一个空槽。
        let mut target: Option<usize> = None;
        for (i, slot) in self.pressed_by_pointer.iter().enumerate() {
            match slot {
                Some((id, _)) if *id == pointer => {
                    target = Some(i);
                    break;
                }
                Some(_) => {}
                None => {
                    target.get_or_insert(i);
                }
            }
        }
        match target {
            Some(i) => {
                self.pressed_by_pointer[i] = Some((pointer, key.clone()));
                // 「主按下态」= **最近一次**按下的那个。
                self.pressed = Some(key);
            }
            None => {
                // 表满（超过 MAX_POINTERS 根手指同时按着）：退化成「只有一个按下态」。
                self.pressed = Some(key);
            }
        }
    }

    /// 取回**这根指针**的按下态，并把它从表里去掉。
    ///
    /// 取走之后，「主按下态」落到**剩下的**按下态上（一个都没有 ⇒ `None`）。
    fn take_pressed_for(&mut self, pointer: u64) -> Option<ElementKey> {
        let mut taken = None;
        for slot in self.pressed_by_pointer.iter_mut() {
            if let Some((id, _)) = slot {
                if *id == pointer {
                    taken = slot.take().map(|(_, k)| k);
                    break;
                }
            }
        }
        self.pressed = self
            .pressed_by_pointer
            .iter()
            .find_map(|s| s.as_ref().map(|(_, k)| k.clone()));
        taken
    }

    /// 悬停 / 按下状态的版本号。
    ///
    /// 任何一次导致"是否悬停"或"是否按下"发生变化的输入都会让它递增。宿主在每帧
    /// 末尾比较新旧值即可决定是否需要重绘——不必自己追踪状态。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 当前悬停的可交互节点。
    pub fn hovered(&self) -> Option<&ElementKey> {
        self.hovered.as_ref()
    }

    /// 按下时被捕获的节点。
    pub fn pressed(&self) -> Option<&ElementKey> {
        self.pressed.as_ref()
    }

    /// 手势识别器（只读；多指手势的出口）。
    pub fn recognizer(&self) -> &Recognizer {
        &self.recognizer
    }

    /// 当前活跃指针数。
    pub fn pointer_count(&self) -> usize {
        self.recognizer.pointer_count()
    }

    /// 最近一次命中测试的结果（复用缓冲，勿长期持有）。
    pub fn last_hit(&self) -> &HitResult {
        &self.hit
    }

    /// 全量复位（窗口失焦等异常恢复路径）。
    ///
    /// 必须同时清掉手势状态与按下状态：若按下状态残留，失焦后第一次点击会被判为
    /// "抬起"，从而误触发一个用户以为只是取消了的操作。
    pub fn reset(&mut self) {
        self.recognizer.reset();
        self.hovered = None;
        self.pressed = None;
        self.pressed_by_pointer = std::array::from_fn(|_| None);
        self.revision += 1;
        self.hit.clear();
    }

    /// 处理一个指针事件，产出路由结果。
    ///
    /// `root` 必须是**已布局**的树——命中测试读 [`crate::node::Node::rect`]，
    /// 未布局时所有矩形都是零，命中测试会一律返回"没点到"。
    pub fn on_pointer(
        &mut self,
        root: &crate::node::Node,
        ev: PointerEvent,
        ctx: Context<'_>,
        out: &mut RoutedEvents,
    ) {
        out.clear();
        let gesture = self.recognizer.on_pointer(ev);

        // 命中测试对三相位都需要：Down 要找按下目标，Move 要更新悬停，
        // Up 要知道松开了没有（可能已在控件外）。指针离开窗口时位置可能无意义，
        // 但 Move 事件一定带位置，Down/Up 也带，所以这里不需要额外的存在性判断。
        let hit_any = hit_test(
            root,
            HitTestRequest {
                point: Point::new(ev.x, ev.y),
            },
            ctx,
            &mut self.hit,
        );
        let target = if hit_any {
            self.hit
                .target()
                .and_then(|n| n.id.clone().map(|id| (id, n.rect)))
        } else {
            None
        };

        match ev.phase {
            PointerPhase::Down => {
                if let Some((id, _)) = &target {
                    // 按**指针**记下 —— 多指各按一个控件时不能互相覆盖（见
                    // `pressed_by_pointer` 的说明）。
                    self.set_pressed_for(ev.id, id.clone());
                    out.push(RoutedEvent::Press(id.clone()));
                    self.revision += 1;
                }
                // ⚠ 触控**不产生悬停**。
                //
                // 官方指南：触控是**双状态**的 —— 「没有可触发其他视觉反馈的
                // 悬停状态」。手指按下时给的反馈是**按下**（pressed），
                // 不是悬停。若触控也写悬停，控件就会同时亮出 hover 与 pressed
                // 两种外观 —— 而触屏上根本没有「悬停」这回事。
                if !ev.source.can_hover() && self.hovered.is_some() {
                    self.hovered = None;
                    self.revision += 1;
                }
            }
            PointerPhase::Move => {
                // 悬停只在没有按下捕获时跟随——拖动过程中让悬停乱跳会让按钮闪。
                //
                // ⚠ 触控**不**走这条路径（双状态，见上）。
                if self.pressed.is_none() && ev.source.can_hover() {
                    let next = target.map(|(id, _)| id);
                    if next != self.hovered {
                        self.hovered = next;
                        self.revision += 1;
                    }
                }
            }
            PointerPhase::Up => {
                let released = target.as_ref().map(|(id, _)| id.clone());
                // ⚠ 按**指针**取回按下态 —— 多指各按一个控件时，取走的是
                // **这根手指**按下的那个（见 `pressed_by_pointer` 的说明）。
                if let Some(pressed) = self.take_pressed_for(ev.id) {
                    let same = released.as_ref() == Some(&pressed);
                    out.push(RoutedEvent::Release {
                        pressed: pressed.clone(),
                        released: released.clone(),
                    });
                    if same {
                        out.push(RoutedEvent::Click(pressed.clone()));
                    }
                    // 静态手势：轻点 / 双击 / 长按（官方指南的「静态手势事件」）。
                    //
                    // ⚠ 判据里已经含了「位移容差」与「时长上限」—— 所以在按钮上
                    // 按住不动 800 ms 再松开：产出 `Click`（按下抬起同节点），
                    // 但**不**产出 `Tapped`（超时了）。这正是指南要的区分。
                    if let (Some(gesture), Some((id, _))) = (gesture.static_gesture, &target) {
                        out.push(RoutedEvent::Static(gesture, id.clone()));
                    }
                    // ⚠️ `pressed` 被清空同样要 `revision++`。
                    //
                    // 这一行曾经是**整个交互链路最隐蔽的一处漏**：宿主要靠
                    // `revision` 变化决定「这一帧要不要重绘」，而 `Click` 之后
                    // 应用通常立刻改了状态（切页、勾选、展开…）。若此时 hover
                    // 恰好没变（鼠标没动 —— 点按钮时最常见），`revision` 不变 ⇒
                    // 宿主判定「什么都没变」⇒ **不重绘**。
                    //
                    // 症状：第一次点击「没反应」，把鼠标移开再移回来才生效 ——
                    // 而画面上没有任何异常，看不出是重绘漏了。
                    self.revision += 1;
                }
                // 抬起后悬停立刻跟上指针当前位置：一次"按下→拖出→拖回"结束时，
                // 悬停应已经是"在按钮上"，否则画面会停在无高亮状态直到下次移动。
                //
                // ⚠ 触控不更新悬停（双状态）。
                if self.pressed.is_none() && ev.source.can_hover() {
                    let next = released;
                    if next != self.hovered {
                        self.hovered = next;
                        self.revision += 1;
                    }
                }
            }
            PointerPhase::Cancel => {
                // ⚠ **取消不是抬起**：清掉按下态，**不**产出点击 / 轻点。
                //
                // 官方指南：「指针取消：在**异常失去**触控交互时发生」。Windows 11
                // 默认把三指 / 四指手势交给系统 —— 那时应用收到的是取消。
                //
                // 若把它当抬起：系统接管手势 ⇒ 应用以为用户松手了 ⇒
                // **误提交一次点击**（按下的按钮被点掉）。那是「我明明没点它，
                // 它自己动了」这类投诉的来源。
                if let Some(pressed) = self.take_pressed_for(ev.id) {
                    out.push(RoutedEvent::Release {
                        pressed,
                        released: None,
                    });
                    self.revision += 1;
                }
                if !ev.source.can_hover() && self.hovered.is_some() {
                    self.hovered = None;
                    self.revision += 1;
                }
            }
        }

        // 手势与 hover/press 是两条正交通道：捏合时也要出悬停变化，
        // 否则多指操作会让界面高亮停留在第一根手指落点。
        if gesture.pinch.is_some() || gesture.pinch_started {
            out.push(RoutedEvent::Gesture(gesture));
        }
        // 操作增量（多指平移 / 缩放 / 旋转 + 速度）。
        if let Some(delta) = gesture.manipulation {
            if !delta.is_idle() {
                out.push(RoutedEvent::Manipulation(delta));
            }
        }
        // 操作结束：抬手瞬间的速度（惯性的初值）。
        if let Some((vx, vy)) = gesture.manipulation_ended {
            out.push(RoutedEvent::ManipulationEnded {
                velocity_x: vx,
                velocity_y: vy,
            });
        }
    }

    /// 每帧调用：检查长按是否到期。
    ///
    /// # 为什么需要它
    ///
    /// 长按是**时间到了**才发生的事，而那一刻**没有任何输入事件**（手指不动）。
    /// 所以它只能靠**时钟**驱动 —— 宿主每帧调一次。
    ///
    /// 返回的节点是**按下时**那个（不是当前指针位置）：手指按住不动时，
    /// 位置可能已经因为抖动偏了几像素。
    pub fn tick(&mut self, time_ms: u64) -> Option<(StaticGesture, ElementKey)> {
        let gesture = self.recognizer.tick(time_ms)?;
        let pressed = self.pressed.clone()?;
        Some((gesture, pressed))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/touch.rs"]
mod touch_tests;

#[cfg(test)]
#[path = "../../tests/unit/input/router.rs"]
mod router_tests;
