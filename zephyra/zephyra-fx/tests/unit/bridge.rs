//! 本文件由 `zephyra::zephyra-fx::src::bridge.rs` 的 `#[cfg(test)] mod tests` 抽出 —— 测试代码与生产代码分离。
//!
//! 模块路径不变（生产文件里仍用 `#[path = ...]` 声明），所以 `use super::*`
//! 与对父模块私有项的访问照旧。

use super::*;

/// 协议解析：好路径全收，坏路径**每一条都带可指认原因**。
#[test]
fn parses_known_commands() {
    assert_eq!(parse_command("ping"), Ok(Command::Ping));
    assert_eq!(parse_command("tree"), Ok(Command::Tree { json: false }));
    assert_eq!(
        parse_command("tree --json"),
        Ok(Command::Tree { json: true })
    );
    assert_eq!(parse_command("layout"), Ok(Command::Layout));
    assert_eq!(parse_command("stats"), Ok(Command::Stats { json: false }));
    assert_eq!(
        parse_command("stats --json"),
        Ok(Command::Stats { json: true })
    );
    assert_eq!(parse_command("shot"), Ok(Command::Shot { repaint: false }));
    assert_eq!(
        parse_command("shot --repaint"),
        Ok(Command::Shot { repaint: true })
    );
    assert_eq!(
        parse_command("props 0.1.2"),
        Ok(Command::Props {
            path: vec![0, 1, 2],
            json: false
        })
    );
    assert_eq!(
        parse_command("props 0.1.2 --json"),
        Ok(Command::Props {
            path: vec![0, 1, 2],
            json: true
        })
    );
    assert_eq!(
        parse_command("hover 120.5 48"),
        Ok(Command::Hover { x: 120.5, y: 48.0 })
    );
    assert_eq!(
        parse_command("click 10 20"),
        Ok(Command::Click { x: 10.0, y: 20.0 })
    );
    assert_eq!(parse_command("pointer_down"), Ok(Command::PointerDown));
    assert_eq!(parse_command("pointer_up"), Ok(Command::PointerUp));
    assert_eq!(
        parse_command("profile_start 30"),
        Ok(Command::ProfileStart {
            duration_seconds: 30
        })
    );
    assert_eq!(parse_command("profile_report"), Ok(Command::ProfileReport));
    assert_eq!(
        parse_command("press_key ArrowDown"),
        Ok(Command::PressKey {
            key: "ArrowDown".into()
        })
    );
    assert_eq!(
        parse_command("scroll 10 20 0 -3"),
        Ok(Command::Scroll {
            x: 10.0,
            y: 20.0,
            delta_x: 0.0,
            delta_y: -3.0,
        })
    );
    assert_eq!(parse_command("resources"), Ok(Command::Resources));
}

#[test]
fn rejects_bad_commands_with_a_reason() {
    for (line, why) in [
        ("", "空命令"),
        ("nope", "未知命令"),
        ("props", "缺路径"),
        ("props x.y", "路径段不是下标"),
        ("hover 1", "缺一个坐标"),
        ("hover a b", "坐标不是数字"),
        ("hover nan 0", "非有限数"),
        ("press_key", "缺键名"),
        ("press_key Enter key", "多余键名"),
        ("scroll 0 0 1", "滚轮参数不足"),
        ("scroll 0 0 nan 1", "滚轮非有限"),
        ("profile_start 0", "时长越界"),
        ("profile_start 301", "时长越界"),
    ] {
        let err = parse_command(line).unwrap_err();
        assert!(!err.is_empty(), "`{line}`（{why}）的错误文案不能是空的");
    }
}

/// `props` 的路径下标从 0 起——`0.0` 是合法的最深起点写法。
#[test]
fn path_is_zero_based() {
    assert_eq!(
        parse_command("props 0.0"),
        Ok(Command::Props {
            path: vec![0, 0],
            json: false
        })
    );
}

/// 按 key / path 定位的命令：解析形态。
///
/// 覆盖「按名字定位」与「按路径定位」两族，以及 `wait_frames`。
#[test]
fn parses_key_and_path_commands() {
    assert_eq!(
        parse_command("click_key switch:solid"),
        Ok(Command::ClickKey {
            key: "switch:solid".into()
        })
    );
    assert_eq!(
        parse_command("hover_key switch:solid"),
        Ok(Command::HoverKey {
            key: "switch:solid".into()
        })
    );
    assert_eq!(
        parse_command("click_path 0.1.1"),
        Ok(Command::ClickPath {
            path: vec![0, 1, 1]
        })
    );
    assert_eq!(
        parse_command("hover_path 0.1.1"),
        Ok(Command::HoverPath {
            path: vec![0, 1, 1]
        })
    );
    assert_eq!(
        parse_command("wait_frames 2"),
        Ok(Command::WaitFrames { frames: 2 })
    );
    assert_eq!(
        parse_command("wait_frames 0"),
        Ok(Command::WaitFrames { frames: 0 })
    );
}

/// 按 key / path 定位的命令：坏输入**每一条都带可指认原因**。
///
/// ⚠ 这些错误文案是排查时唯一能看到的东西（「应用收到了莫名其妙的命令」
/// 比「参数写错了」难查十倍）。
#[test]
fn rejects_malformed_key_and_path_commands() {
    for (line, needle) in [
        ("click_key", "click_key"),
        ("hover_key", "hover_key"),
        ("click_key a b", "只接受一个 key"),
        ("click_path", "click_path"),
        ("click_path 0.x", "不是下标"),
        ("wait_frames", "wait_frames"),
        ("wait_frames two", "不是 0..=10000 的整数"),
        ("wait_frames 999999", "不能超过 10000"),
        ("tree --xml", "只接受可选的 --json"),
        ("stats --xml", "只接受可选的 --json"),
        ("shot --nope", "只接受可选的 --repaint"),
    ] {
        let err = parse_command(line).unwrap_err();
        assert!(
            err.contains(needle),
            "`{line}` 的错误文案里应含 `{needle}`，实得：{err}"
        );
    }
}

/// 按 key 点击：落点必须是**元素的中心**。
///
/// # 为什么这条要钉死
///
/// 「按名字点」的价值在于**不用抄坐标**。落点算错（偏到旁边的元素上）
/// 会表现为「点了没反应」，而那种症状与「事件路由坏了」完全没法区分。
#[test]
fn click_by_key_lands_on_the_element_center() {
    let root = laid_out_tree();
    let expected = nodes_by_key_name(&root, "switch:solid")[0];
    let rect = expected.arranged_rect();
    assert!(
        rect.width > 0.0 && rect.height > 0.0,
        "测试前提：布局必须真的摆了矩形，实得 {rect:?}"
    );
    let want = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);

    let mut host = MockHost::with_tree(root);
    let resp = execute(
        &Command::ClickKey {
            key: "switch:solid".into(),
        },
        &mut host,
    );
    let Response::Text(lines) = &resp else {
        panic!("click_key 必须返回文本")
    };
    assert!(lines[0].contains("switch:solid"), "{lines:?}");
    assert_eq!(host.clicks.len(), 1);
    assert!((host.clicks[0].0 - want.0).abs() < 0.5);
    assert!((host.clicks[0].1 - want.1).abs() < 0.5);
}

/// 按路径点击：落点同样落在**该节点**的中心。
#[test]
fn click_by_path_lands_on_the_node_center() {
    let root = laid_out_tree();
    let node = node_at_path(&root, &[0, 1]).expect("0.1 必须存在");
    let rect = node.arranged_rect();
    let want = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);

    let mut host = MockHost::with_tree(root);
    execute(&Command::ClickPath { path: vec![0, 1] }, &mut host);
    assert_eq!(host.clicks.len(), 1);
    assert!((host.clicks[0].0 - want.0).abs() < 0.5);
    assert!((host.clicks[0].1 - want.1).abs() < 0.5);
}

/// key 不存在 / 路径不存在：**报**错，且**不注入**任何输入。
///
/// ⚠ 「静默无动作」是最糟的失败形态 —— 调用方以为点过了。
#[test]
fn unknown_key_or_path_reports_and_injects_nothing() {
    let mut host = MockHost::with_tree(laid_out_tree());
    let resp = execute(
        &Command::ClickKey {
            key: "no:such:key".into(),
        },
        &mut host,
    );
    assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    assert!(host.clicks.is_empty(), "找不到就不该注入点击");

    let resp = execute(&Command::ClickPath { path: vec![0, 99] }, &mut host);
    assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    assert!(host.clicks.is_empty(), "找不到就不该注入点击");
}

/// 宿主**没有**布局树时，按 key 定位要明确报错，而不是静默无动作。
#[test]
fn key_click_without_a_tree_reports_an_error() {
    let mut host = MockHost::new();
    let resp = execute(
        &Command::ClickKey {
            key: "switch:solid".into(),
        },
        &mut host,
    );
    assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    assert!(host.clicks.is_empty());
}

/// `--repaint` 必须**真的**传到宿主（截图语义是这轮改动的主体）。
#[test]
fn shot_repaint_flag_reaches_the_host() {
    let mut host = MockHost::new();
    host.shot_bytes = Some(b"x".to_vec());
    execute(&Command::Shot { repaint: false }, &mut host);
    execute(&Command::Shot { repaint: true }, &mut host);
    assert_eq!(
        host.shot_opts,
        [
            ShotOptions { repaint: false },
            ShotOptions { repaint: true }
        ]
    );
}

/// `stats --json` 与文本形态**同源**：同一份数据、两种排版。
///
/// 钉住这条是为了防止「JSON 是把文本正则解析出来的」——那种实现会在
/// 文本格式一改就悄悄坏掉，而且不报任何错。
#[test]
fn stats_json_and_text_agree_on_the_same_numbers() {
    let mut host = MockHost::new();
    let Response::Text(text) = execute(&Command::Stats { json: false }, &mut host) else {
        panic!("stats 必须返回文本")
    };
    let Response::Text(json) = execute(&Command::Stats { json: true }, &mut host) else {
        panic!("stats --json 必须返回文本")
    };
    let text = text.join("");
    let json = json.join("");
    assert!(text.contains("quads=2"), "{text}");
    assert!(json.contains("\"quads\":2"), "{json}");
    assert!(json.contains("\"damage\":null"), "{json}");
}

/// 树里必须带**可见文字**。
///
/// # 为什么这条是门禁而不是「顺手加上」
///
/// 树里只有 key 时，AI 看到的是「所有导航项的 key 都叫 `label`」——
/// 它**分不出**是哪一个。而人看到的是「快速开始 / 样式 / 布局」。
///
/// 没有文字就只能对每个文本节点逐个调 `props`：一次 O(n) 次往返的排查。
#[test]
fn tree_carries_the_visible_text() {
    let root = laid_out_tree();
    let dump = tree_dump_of(&root);
    // 布局树里的文本节点带内容。
    let json = tree_json_of(&root);
    assert!(
        json.contains("\"text\":"),
        "JSON 树必须有 text 字段：{json}"
    );
    assert!(!dump.is_empty());
}

/// 文字里的**换行**必须被转义 —— 协议是行文本，一个裸换行会把一条
/// 命令劈成两条（症状：应用收到莫名其妙的命令，没人知道为什么）。
#[test]
fn inline_text_escapes_control_characters() {
    assert_eq!(escape_inline("a\nb"), "\"a\\nb\"");
    assert_eq!(escape_inline("a\"b"), "\"a\\\"b\"");
    assert_eq!(escape_inline("a\\b"), "\"a\\\\b\"");
    assert_eq!(escape_inline("a\tb"), "\"a\\tb\"");
    // 反解要还原。
    assert_eq!(unescape_inline("\"a\\nb\"").unwrap(), "a\nb");
    assert_eq!(unescape_inline("\"a b\"").unwrap(), "a b");
    assert_eq!(unescape_inline("plain").unwrap(), "plain");
    assert!(unescape_inline("\"unterminated").is_err());
}

/// `find`：按可见文字找元素，并顺带给出**最近的带 key 的祖先**。
///
/// owner 那一栏是重点：文字节点通常不是可点的那个（真正可点的是它的
/// 祖先控件），AI 要的是「点哪个」。
#[test]
fn find_locates_elements_by_visible_text() {
    let root = laid_out_tree();
    let hits = find_text_of(&root, "solid");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].text, "solid");
    // owner 是**最近的可点祖先**：文本节点自己也带 key（`label`），
    // 但点它多半没反应 —— 真正可点的是它的父控件。
    assert_eq!(
        hits[0].owner.as_deref(),
        Some("0.0"),
        "owner 应当是带 key 的父控件，不是文本节点自己：{hits:?}"
    );

    // 找不到就是空列表（不是错误）。
    assert!(find_text_of(&root, "no-such-text").is_empty());
}

/// `type_text` 的参数是一个**整体**：带引号时可以含空格。
#[test]
fn type_text_takes_the_rest_of_the_line() {
    assert_eq!(
        parse_command("type_text \"hello world\""),
        Ok(Command::TypeText {
            text: "hello world".into()
        })
    );
    assert_eq!(
        parse_command("type_text hello"),
        Ok(Command::TypeText {
            text: "hello".into()
        })
    );
    assert!(parse_command("type_text").is_err());
    assert!(parse_command("type_text \"\"").is_err());
}

/// `type_text` 真的把文字交给了宿主（而不是只回一句「已输入」）。
#[test]
fn type_text_reaches_the_host() {
    let mut host = MockHost::new();
    let resp = execute(&Command::TypeText { text: "hi".into() }, &mut host);
    let Response::Text(lines) = &resp else {
        panic!("type_text 必须返回文本")
    };
    assert!(lines[0].contains("2"), "{lines:?}");
    assert_eq!(host.texts, vec!["hi".to_string()]);
}

/// `resize`：改窗口客户区尺寸（**物理**像素，与 `viewport` 同一坐标系）。
///
/// 为什么单独测：宽高是**无符号**的，负数会绕成天文数字 ——
/// 而 `resize -1 -1` 在解析层就该被挡住。
#[test]
fn resize_takes_physical_pixels() {
    assert_eq!(
        parse_command("resize 800 600"),
        Ok(Command::Resize {
            width: 800,
            height: 600
        })
    );
    for bad in [
        "resize",
        "resize 800",
        "resize a b",
        "resize -1 600",
        "resize 0 600",
    ] {
        assert!(parse_command(bad).is_err(), "`{bad}` 必须被解析层挡住");
    }
}

/// 命中测试：给出的必须是**最具体**的那个元素，且说清楚「能不能点」。
#[test]
fn hit_test_finds_the_most_specific_element() {
    let root = laid_out_tree();
    // 第一个控件的矩形：从布局结果里取，别写死。
    let child = node_at_path(&root, &[0, 0]).expect("0.0 必须存在");
    let r = child.arranged_rect();
    let x = r.x + r.width * 0.5;
    let y = r.y + r.height * 0.5;

    let hit = hit_at(&root, x, y).expect("点必须落在某个元素里");
    assert_eq!(hit.path, "0.0", "{hit:?}");
    assert_eq!(hit.key.as_deref(), Some("switch:solid"), "{hit:?}");
    // 可点的那个就是命中节点自己（它是 Interactive）。
    assert_eq!(hit.clickable.as_deref(), Some("0.0"), "{hit:?}");

    // 点在所有元素之外 ⇒ `None`（不是猜一个）。
    assert!(hit_at(&root, -100.0, -100.0).is_none());
}

// ── TCP 集成：真实 socket 上走一轮 ──────────────────────────────

/// 造一棵**摆好**的布局树：根下挂两个子元素，各有 key 与固定尺寸。
///
/// ⚠ 必须走一遍 `layout::layout`：节点的矩形由排列趟写入，而
/// `Node::set_rect` 是 core 的 crate 内 API —— 外面改不了。
/// 所以「按 key / path 定位」这件事没法用假矩形测，只能让布局真跑一遍。
fn laid_out_tree() -> zephyra_core::node::Node {
    use zephyra_core::core::Rect;
    use zephyra_core::element::Element;
    use zephyra_core::id::NodeTag;
    use zephyra_core::node::Node;
    use zephyra_core::property::PropertyValue;
    use zephyra_core::props;

    // 文本节点的内容是**本地**属性（应用给的），样式表里不会有。
    let text = |s: &'static str| PropertyValue::Str(std::sync::Arc::from(s));

    let element = Element::new(NodeTag::Box).with_key("root").with_children([
        Element::new(NodeTag::Control)
            .with_key("switch:solid")
            .with(&props::WIDTH, PropertyValue::float(100.0))
            .with(&props::HEIGHT, PropertyValue::float(40.0))
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .with_children([Element::new(NodeTag::Text)
                .with_key("label")
                .with(&props::TEXT, text("solid"))]),
        Element::new(NodeTag::Control)
            .with_key("switch:glass")
            .with(&props::WIDTH, PropertyValue::float(100.0))
            .with(&props::HEIGHT, PropertyValue::float(40.0))
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .with_children([Element::new(NodeTag::Text)
                .with_key("label")
                .with(&props::TEXT, text("glass"))]),
    ]);
    let mut node = Node::from_element(&element);
    zephyra_core::layout::layout(
        &mut node,
        Rect::new(0.0, 0.0, 800.0, 600.0),
        zephyra_core::context::dark_context(),
    );
    node
}

/// 记录注入调用的 mock 宿主。树/属性是固定文本——这里测的是**桥**，
/// 不是宿主的 dump 质量（后者在后端的集成测试里）。
struct MockHost {
    clicks: Vec<(f32, f32)>,
    hovers: Vec<(f32, f32)>,
    keys: Vec<String>,
    scrolls: Vec<(f32, f32, f32, f32)>,
    pointer_button_events: Vec<bool>,
    shot_bytes: Option<Vec<u8>>,
    /// 注入过的文字（`type_text`）。
    texts: Vec<String>,
    /// 布局树（按 key / path 定位用）。`None` = 本宿主不提供树。
    root: Option<zephyra_core::node::Node>,
    /// 渲染过的帧数（`wait_frames` 按它算）。
    ///
    /// 用 `Arc<AtomicU64>` 而不是裸字段：TCP 测试里宿主住在工作线程上，
    /// 而测试线程要能**推进**帧数来让 `wait_frames` 达标。
    frames: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 每次截图请求的选项（用来验 `--repaint` 是否真的传到宿主）。
    shot_opts: Vec<ShotOptions>,
}

impl MockHost {
    fn new() -> Self {
        Self {
            clicks: Vec::new(),
            hovers: Vec::new(),
            keys: Vec::new(),
            scrolls: Vec::new(),
            pointer_button_events: Vec::new(),
            shot_bytes: None,
            texts: Vec::new(),
            root: None,
            frames: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            shot_opts: Vec::new(),
        }
    }

    /// 带一棵布局树的宿主（`click_key` / `click_path` 用得上）。
    fn with_tree(root: zephyra_core::node::Node) -> Self {
        Self {
            root: Some(root),
            ..Self::new()
        }
    }
}

impl BridgeHost for MockHost {
    fn viewport(&self) -> (f32, f32) {
        (800.0, 600.0)
    }
    fn tree_dump(&self) -> String {
        "0 Container root 0,0 800x600\n  0.0 Text title 0,0 120x24".into()
    }
    fn props_dump(&self, path: &[usize]) -> Option<String> {
        match path {
            [0] => Some("tag Container\nkey root".into()),
            _ => None,
        }
    }
    fn stats(&self) -> FrameStats {
        FrameStats {
            quads: 2,
            ..FrameStats::default()
        }
    }
    fn inject_hover(&mut self, x: f32, y: f32) {
        self.hovers.push((x, y));
    }
    fn inject_click(&mut self, x: f32, y: f32) {
        self.clicks.push((x, y));
    }
    fn inject_pointer_down(&mut self) {
        self.pointer_button_events.push(true);
    }
    fn inject_pointer_up(&mut self) {
        self.pointer_button_events.push(false);
    }
    fn inject_key(&mut self, key: &str) {
        self.keys.push(key.to_string());
    }

    fn inject_scroll(&mut self, x: f32, y: f32, delta_x: f32, delta_y: f32) {
        self.scrolls.push((x, y, delta_x, delta_y));
    }
    fn inject_text(&mut self, text: &str) {
        self.texts.push(text.to_string());
    }
    fn screenshot(&mut self, opts: ShotOptions) -> Option<Vec<u8>> {
        self.shot_opts.push(opts);
        self.shot_bytes.clone()
    }

    fn root(&self) -> Option<&zephyra_core::node::Node> {
        self.root.as_ref()
    }

    fn rendered_frames(&self) -> u64 {
        self.frames.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[test]
fn performance_profiler_is_opt_in_bounded_and_formats_frame_detail() {
    let mut profiler = PerformanceProfiler::new();
    assert!(profiler.report().unwrap_err().contains("尚未启动"));
    assert!(profiler.start(0).is_err());
    assert!(profiler.start(301).is_err());
    profiler.start(1).unwrap();
    profiler.record(FrameTiming {
        update_us: 120,
        render_submit_us: 80,
        total_us: 210,
        stages: None,
    });
    let report = format_profile_report(
        Instant::now(),
        &[ProfileSample {
            elapsed_ms: 1,
            timing: FrameTiming {
                update_us: 120,
                render_submit_us: 80,
                total_us: 210,
                stages: None,
            },
        }],
    );
    assert!(report.contains("PROFILE_STATE complete"));
    assert!(report.contains("frame_index elapsed_ms total_us update_layout_us render_submit_us"));
    assert!(report.contains("1 1 210 120 80"));
    // 分段不可用时明确说「unavailable」，而不是印一排 0 让人以为
    // 「这五段不花钱」。
    assert!(report.contains("stage_diff unavailable"), "{report}");
}

#[test]
fn performance_profiler_stops_at_the_frame_limit() {
    let mut profiler = PerformanceProfiler::new();
    profiler.start(1).unwrap();
    for _ in 0..3000 {
        profiler.record(FrameTiming {
            update_us: 1,
            render_submit_us: 2,
            total_us: 3,
            stages: None,
        });
    }
    assert!(!profiler.is_running());
    assert!(profiler.report().unwrap().contains("frames=3000"));
}

/// 采样报告里的 fps 与**分段百分位**。
///
/// ⚠ 这条钉的是「fps 报的是**渲染帧率**」—— 宿主必须只记真的重画了的帧。
/// 若把空转的 tick 也记进来，fps 会变成一个假数字（实测 2500 fps 而
/// 应用只画了 9 帧）。
#[test]
fn profile_report_carries_fps_and_per_stage_percentiles() {
    let stages = zephyra_core::frame::StageTimings {
        diff_us: 10,
        style_us: 20,
        scan_us: 30,
        layout_us: 40,
        paint_us: 50,
        total_us: 150,
    };
    let report = format_profile_report(
        Instant::now(),
        &[
            ProfileSample {
                elapsed_ms: 1,
                timing: FrameTiming {
                    update_us: 100,
                    render_submit_us: 20,
                    total_us: 150,
                    stages: Some(stages),
                },
            },
            ProfileSample {
                elapsed_ms: 2,
                timing: FrameTiming {
                    update_us: 100,
                    render_submit_us: 20,
                    total_us: 150,
                    stages: Some(stages),
                },
            },
        ],
    );
    assert!(report.contains("frames=2"), "{report}");
    assert!(report.contains("fps="), "{report}");
    assert!(report.contains("stage_diff min=10us"), "{report}");
    assert!(report.contains("stage_paint min=50us"), "{report}");
    assert!(report.contains("n=2"), "{report}");
}

/// 一个采样都没有时，报告必须说清「没有重绘」而不是印一个 0 的 fps。
#[test]
fn profile_report_explains_an_idle_app() {
    let report = format_profile_report(Instant::now(), &[]);
    assert!(report.contains("frames=0"), "{report}");
    assert!(report.contains("静止"), "{report}");
    assert!(!report.contains("fps="), "{report}");
}

#[test]
fn dispatches_pointer_button_edges_independently() {
    let mut host = MockHost::new();
    assert_eq!(
        execute(&Command::PointerDown, &mut host),
        Response::Text(vec!["pointer_down".into()])
    );
    assert_eq!(
        execute(&Command::PointerUp, &mut host),
        Response::Text(vec!["pointer_up".into()])
    );
    assert_eq!(host.pointer_button_events, [true, false]);
}

#[test]
fn dispatches_key_scroll_and_text_layout() {
    let mut host = MockHost::new();
    assert_eq!(
        execute(
            &Command::PressKey {
                key: "Enter".into()
            },
            &mut host
        ),
        Response::Text(vec!["pressed Enter".into()])
    );
    assert_eq!(host.keys, ["Enter"]);
    execute(
        &Command::Scroll {
            x: 4.0,
            y: 5.0,
            delta_x: 0.0,
            delta_y: -2.0,
        },
        &mut host,
    );
    assert_eq!(host.scrolls, [(4.0, 5.0, 0.0, -2.0)]);
    let Response::Text(layout) = execute(&Command::Layout, &mut host) else {
        panic!("layout 必须返回文本")
    };
    assert!(layout[0].contains("800x600"));
    assert!(layout[2].contains("absolute-x"));
    assert!(layout.iter().any(|line| line.contains("0.0 Text title")));
}

/// 客户端读一条响应：读到 `OK` 行或 `ERR` 行为止（`BIN` 时先消费载荷）。
///
/// ⚠ 终止条件**必须**包含 ERR：ERR 行后面没有 `OK`（ERR 本身就是终止行）——
/// 漏掉这条的版本在这里**死等**，测试直接挂死（实测）。
fn read_response(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        // 一字节一读在测试里够慢也够对；不需要行缓冲的复杂度。
        stream.read_exact(&mut byte).unwrap();
        buf.push(byte[0]);
        let s = String::from_utf8_lossy(&buf);
        if s.ends_with("\nOK\n") {
            // `BIN` 响应的头部之后还有载荷 + 尾部 `\nOK\n`——上面的判断
            // 只在「载荷读完后」才成立，正好是我们要的。
            return s.into_owned();
        }
        // ERR 是单行终止：换行即完整。
        if s.starts_with("ERR ") && s.ends_with('\n') {
            return s.into_owned();
        }
        if s.starts_with("BIN ") {
            // 头行是 `BIN <len> <note>\n`——note 变长，头行以**首个换行**结束；
            // 之后是 len 字节载荷 + `\nOK\n`。
            if let Some(hend) = s.find('\n') {
                let rest = &s[4..hend];
                if let Some(sp) = rest.find(' ') {
                    if let Ok(len) = rest[..sp].parse::<usize>() {
                        let need = hend + 1 + len + "\nOK\n".len();
                        if buf.len() >= need {
                            return String::from_utf8_lossy(&buf).into_owned();
                        }
                    }
                }
            }
        }
    }
}

/// 绑定随机端口，返回（桥，端口）。
fn bridge_on_ephemeral_port() -> (DebugBridge, u16) {
    // :0 由 OS 分配——测试不抢固定端口。
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    (
        DebugBridge {
            listener: Some(listener),
            link: None,
            conns: Vec::new(),
        },
        port,
    )
}

/// MCP 拓扑：应用连出到 MCP，连上后走的是**同一套**桥协议（读命令、回响应）。
#[test]
fn connects_out_to_mcp_and_serves_the_same_protocol() {
    use std::sync::atomic::{AtomicBool, Ordering};

    // 假 MCP：监听、接受应用的连入、发一条 ping、读回响应。
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    let mcp = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // 应用连上后的第一帧是身份行（P1-4）；不先读掉，下面的响应解析会错位。
        let mut hello = [0u8; 256];
        let n = stream.read(&mut hello).unwrap();
        let hello = std::str::from_utf8(&hello[..n]).unwrap().to_string();
        assert!(hello.starts_with("hello "), "第一帧必须是 hello: {hello:?}");
        tx.send(hello).unwrap();
        stream.write_all(b"ping\n").unwrap();
        tx.send(read_response(&mut stream)).unwrap();
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let bridge = DebugBridge {
        listener: None,
        link: Some(OutboundLink::new(addr)),
        conns: Vec::new(),
    };
    let mut host = MockHost::new();
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let worker = std::thread::spawn(move || {
        let mut bridge = bridge;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !bridge.has_client() && std::time::Instant::now() < deadline {
            bridge.poll(&mut host);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // 连上后再 poll 若干次，让它读到命令并把响应写出去。
        for _ in 0..30 {
            bridge.poll(&mut host);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        worker_done.store(true, Ordering::Release);
    });

    let hello = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(hello.starts_with("hello "), "{hello}");
    let response = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(response.contains("pong"), "{response}");
    worker.join().unwrap();
    mcp.join().unwrap();
}

#[test]
fn full_round_trip_over_tcp() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    host.shot_bytes = Some(b"BMP!".to_vec());
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let (host_tx, host_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut bridge = bridge;
        while !worker_done.load(Ordering::Acquire) {
            bridge.poll(&mut host);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        host_tx.send(host).unwrap();
    });
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();

    // 文本命令。⚠ 逐条发收：桥在一次 poll 里会把凑齐的多条命令**连续**
    // 回包，客户端按「读到 OK 为止」切分时无法回头再切——协议是逐条的，
    // 测试也逐条走。
    for (cmd, expect) in [
        ("ping", "pong"),
        ("tree", "0.0 Text title"),
        ("viewport", "800 600"),
        ("stats", "quads=2"),
    ] {
        client.write_all(cmd.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        let r = read_response(&mut client);
        assert!(r.contains(expect), "`{cmd}` 的响应缺 `{expect}`：{r}");
        assert!(r.ends_with("OK\n"), "文本响应以 OK 终止：{r}");
    }

    // 注入命令到达宿主（各自一轮，响应都要读走——否则残留会污染下轮切分）。
    client.write_all(b"click 33 44\n").unwrap();
    assert!(read_response(&mut client).contains("clicked"));
    client.write_all(b"hover 1 2\n").unwrap();
    assert!(read_response(&mut client).contains("hovered"));

    // 无效路径得到 ERR，且连接不断（下一条命令还能走）。
    client.write_all(b"props 9.9\n").unwrap();
    let r = read_response(&mut client);
    assert!(r.starts_with("ERR "), "{r}");
    client.write_all(b"tree\n").unwrap();
    let r = read_response(&mut client);
    assert!(r.contains("Text title"), "{r}");

    // 二进制响应。
    client.write_all(b"shot\n").unwrap();
    let r = read_response(&mut client);
    assert!(r.contains("BIN 4 bmp 24bpp"), "{r}");
    assert!(r.ends_with("BMP!\nOK\n"), "{r}");
    drop(client);
    done.store(true, Ordering::Release);
    let host = host_rx.recv().unwrap();
    worker.join().unwrap();
    assert_eq!(host.clicks, vec![(33.0, 44.0)]);
    assert_eq!(host.hovers, vec![(1.0, 2.0)]);
}

/// `wait_frames` 是**延迟应答**：等应用真的画够 N 帧才回话。
///
/// 钉住两件事：
///
/// 1. 帧数不够时**不能**提前回话 —— 提前回话等于「等到了动画结束」是假的，
///    而那正是这个命令存在的理由；
/// 2. 达标才回，且回的话里带真的帧数。
#[test]
fn wait_frames_answers_only_after_the_app_rendered_enough() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    let frames = host.frames.clone();
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let worker = std::thread::spawn(move || {
        let mut bridge = bridge;
        while !worker_done.load(Ordering::Acquire) {
            bridge.poll(&mut host);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.write_all(b"wait_frames 3\n").unwrap();

    // 帧数还不够：桥**不该**回话。读一次应当超时而不是拿到回答。
    client
        .set_read_timeout(Some(std::time::Duration::from_millis(150)))
        .unwrap();
    let mut probe = [0u8; 1];
    let early = client.read(&mut probe);
    assert!(
        early.is_err(),
        "帧数不够就回话 = 「等到了动画结束」是假的；实得：{early:?}"
    );
    client.set_read_timeout(None).unwrap();

    // 应用画够了 → 桥这时才回话。
    frames.store(3, Ordering::Release);
    let r = read_response(&mut client);
    assert!(r.contains("waited 3 rendered frames"), "{r}");

    done.store(true, Ordering::Release);
    worker.join().unwrap();
}

/// `wait_frames` 在应用**静止**时不能吊着不回。
///
/// # 为什么这条重要
///
/// AI 的典型用法是「点一下 → 等画面稳定 → 截图」。而静止的应用**永远出不了**
/// 那么多帧 —— 于是每次 `wait_frames` 都要等满 2 秒。一个排查会话几十次调用，
/// 就是一分多钟纯等待。
///
/// 所以「静止」也是一个**结局**，而且要说清楚实情。
#[test]
fn wait_frames_reports_that_the_app_settled_instead_of_hanging() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    // 宿主**不出帧**（静止的应用）。
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let worker = std::thread::spawn(move || {
        let mut bridge = bridge;
        while !worker_done.load(Ordering::Acquire) {
            bridge.poll(&mut host);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let started = Instant::now();
    client.write_all(b"wait_frames 5\n").unwrap();
    let r = read_response(&mut client);
    let elapsed = started.elapsed();
    assert!(r.contains("settled"), "{r}");
    // ⚠ 必须**快**：静止的应用不该让调用方等满超时。
    assert!(
        elapsed < WAIT_FRAMES_TIMEOUT,
        "静止时的等待不该拖到超时（实得 {elapsed:?}）"
    );

    done.store(true, Ordering::Release);
    worker.join().unwrap();
}

#[test]
fn large_screenshot_is_drained_across_nonblocking_polls() {
    use std::io::{BufRead, BufReader, Read};
    use std::sync::atomic::{AtomicBool, Ordering};

    let (bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    let mut payload = vec![0x5a; 1024 * 1024];
    payload[100..104].copy_from_slice(b"\nOK\n");
    host.shot_bytes = Some(payload.clone());
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let worker = std::thread::spawn(move || {
        let mut bridge = bridge;
        while !worker_done.load(Ordering::Acquire) {
            bridge.poll(&mut host);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.write_all(b"shot\n").unwrap();
    let mut reader = BufReader::new(client);
    let mut header = String::new();
    reader.read_line(&mut header).unwrap();
    assert_eq!(header, format!("BIN {} bmp 24bpp\n", payload.len()));
    let mut received = vec![0; payload.len()];
    reader.read_exact(&mut received).unwrap();
    assert_eq!(received, payload);
    let mut terminator = [0; 4];
    reader.read_exact(&mut terminator).unwrap();
    assert_eq!(&terminator, b"\nOK\n");

    done.store(true, Ordering::Release);
    worker.join().unwrap();
}

#[test]
fn half_line_waits_for_the_rest() {
    let (mut bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.set_nonblocking(false).unwrap();
    bridge.poll(&mut host);

    // 半行：桥不该把它当命令，也不该回任何东西。
    client.write_all(b"pi").unwrap();
    bridge.poll(&mut host);
    client.set_nonblocking(true).unwrap();
    let mut byte = [0u8; 1];
    assert!(client.read(&mut byte).unwrap_or(0) == 0, "半行不该产生响应");

    // 补上后半行：凑齐了才执行。
    client.set_nonblocking(false).unwrap();
    client.write_all(b"ng\n").unwrap();
    bridge.poll(&mut host);
    assert!(read_response(&mut client).contains("pong"));
}

#[test]
fn client_disconnect_is_recovered_from() {
    let (mut bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    {
        let c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.set_nonblocking(false).unwrap();
        bridge.poll(&mut host);
        assert!(bridge.has_client());
        drop(c);
    }
    // 第一次 poll 读到 0 字节 → 清连接；第二次 poll → 重新 accept 下一个。
    bridge.poll(&mut host);
    assert!(!bridge.has_client(), "对端关闭后连接槽应当清空");
    let _c2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
    bridge.poll(&mut host);
    assert!(bridge.has_client(), "新客户端应当能接上");
}

/// **两个客户端同时在线**，各自的命令都能到达宿主。
///
/// # 为什么这条是必须的
///
/// v1 只有一个连接槽：第二个客户端挂在 backlog 里，**一条命令也进不来**。
/// 而调试器的用法恰恰是「一边 `debug watch` 盯着，一边 `bridge click` 去点」
/// —— 那时点击那条命令根本发不过去，症状是「点了没反应」，而应用一切正常。
///
/// 这条测试就是那个场景的最小复现。
#[test]
fn two_clients_can_both_send_commands() {
    let (mut bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    let mut a = TcpStream::connect(("127.0.0.1", port)).unwrap();
    bridge.poll(&mut host);
    let mut b = TcpStream::connect(("127.0.0.1", port)).unwrap();
    bridge.poll(&mut host);

    assert!(bridge.has_client());
    // 两个客户端各发一条，各自的应答都要能读回来。
    a.write_all(b"ping\n").unwrap();
    b.write_all(b"viewport\n").unwrap();
    bridge.poll(&mut host);
    assert!(
        read_response(&mut a).contains("pong"),
        "第一个客户端的应答丢了"
    );
    assert!(
        read_response(&mut b).contains("800 600"),
        "第二个客户端的应答丢了 —— 多连接支持坏了"
    );
}

/// 「有客户端但没数据」≠「有活要干」。
///
/// 宿主靠这个区别决定控制流：Poll（全速空转）还是 WaitUntil（低频重试）。
/// 判据错了就是 2500 Hz 空转（实测），白烧 CPU。
#[test]
fn pending_work_is_false_for_an_idle_connected_client() {
    let (mut bridge, port) = bridge_on_ephemeral_port();
    let mut host = MockHost::new();
    let _client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    bridge.poll(&mut host);
    assert!(bridge.has_client());
    assert!(
        !bridge.has_pending_work(),
        "连上但没数据时不该判成「有活要干」—— 否则宿主只能全速空转"
    );
}

/// `touch` 命令的解析：相位与身份。
///
/// # 为什么要有它
///
/// 没有触摸屏的机器上，触控那条路**根本没法验** —— 而「触控能不能用」
/// 恰恰是最该验的一件事。这条命令给了 AI 与人一条**与真实设备同形**的路。
#[test]
fn touch_command_parses_the_phase_and_the_finger_id() {
    assert_eq!(
        parse_command("touch down 3 100 200"),
        Ok(Command::Touch {
            phase: TouchPhase::Down,
            id: 3,
            x: 100.0,
            y: 200.0
        })
    );
    assert_eq!(
        parse_command("touch move 0 1.5 2.5"),
        Ok(Command::Touch {
            phase: TouchPhase::Move,
            id: 0,
            x: 1.5,
            y: 2.5
        })
    );
    assert_eq!(
        parse_command("touch up 1 0 0"),
        Ok(Command::Touch {
            phase: TouchPhase::Up,
            id: 1,
            x: 0.0,
            y: 0.0
        })
    );
    // ⚠ **取消**是一个独立相位 —— 它**不是**抬起（官方指南：「指针取消：在
    // 异常失去触控交互时发生」）。
    assert_eq!(
        parse_command("touch cancel 1 0 0"),
        Ok(Command::Touch {
            phase: TouchPhase::Cancel,
            id: 1,
            x: 0.0,
            y: 0.0
        })
    );
    // 错的相位 / 少的参数 ⇒ 报错，不静默当成抬起。
    assert!(parse_command("touch hover 1 0 0").is_err());
    assert!(parse_command("touch down 1 0").is_err());
    assert!(parse_command("touch down x 0 0").is_err());
}

/// 跳过的帧**不**覆盖「上一次画出来的东西」。
///
/// # 为什么这条是门禁
///
/// 跳过的帧画了 **0** 个四边形。直接写进去的话，`stats` 会报
/// `quads=0` —— 读的人以为「屏幕上是空的」，而画面上明明有 73 个节点。
/// 症状：`zephyra perf watch` 在静止的应用上那一列恒为 0。
#[test]
fn a_skipped_frame_keeps_the_last_drawn_quads() {
    let drawn = FrameStats {
        quads: 98,
        skipped: 2,
        dirtied: 1,
        needs_layout: 0,
        redrawn: true,
        hovered: Some("nav".into()),
        damage: Some(zephyra_core::frame::Damage::Full),
        stages: None,
        frames: 5,
        input_latency_us: None,
        render_submit_us: None,
        offscreen_passes: 3,
        backdrop_blur_passes: 1,
        lens_passes: 1,
        refraction_passes: 1,
    };
    // 跳过的这一帧：画了 0 个四边形。
    let current = FrameStats {
        quads: 0,
        skipped: 0,
        dirtied: 0,
        needs_layout: 0,
        redrawn: false,
        hovered: Some("other".into()),
        damage: Some(zephyra_core::frame::Damage::Full),
        stages: None,
        frames: 5,
        input_latency_us: None,
        render_submit_us: None,
        offscreen_passes: 0,
        backdrop_blur_passes: 0,
        lens_passes: 0,
        refraction_passes: 0,
    };
    let merged = stats_after_skipped(&drawn, &current);
    assert_eq!(merged.quads, 98, "画面上还是那 98 个四边形");
    assert_eq!(merged.skipped, 2);
    // 悬停是「现在」的 —— 那一帧没画，但鼠标确实在那里。
    assert_eq!(merged.hovered.as_deref(), Some("other"));
    // 「这一帧有没有画」仍然是 false（读的人要知道这一帧被跳过了）。
    assert!(!merged.redrawn);
}

/// 断连钩子：连接消失的那一帧通知宿主一次，之后不再打扰。
///
/// 它是「拖动中途 MCP 被杀」的唯一防线——宿主靠它补抬指针。
#[test]
fn disconnect_notifies_host_once_and_then_stays_quiet() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingHost {
        inner: MockHost,
        disconnects: std::sync::Arc<AtomicUsize>,
    }
    impl BridgeHost for CountingHost {
        fn viewport(&self) -> (f32, f32) {
            self.inner.viewport()
        }
        fn tree_dump(&self) -> String {
            self.inner.tree_dump()
        }
        fn props_dump(&self, path: &[usize]) -> Option<String> {
            self.inner.props_dump(path)
        }
        fn stats(&self) -> FrameStats {
            self.inner.stats()
        }
        fn root(&self) -> Option<&zephyra_core::node::Node> {
            self.inner.root()
        }
        fn inject_hover(&mut self, x: f32, y: f32) {
            self.inner.inject_hover(x, y);
        }
        fn inject_click(&mut self, x: f32, y: f32) {
            self.inner.inject_click(x, y);
        }
        fn inject_pointer_down(&mut self) {
            self.inner.inject_pointer_down();
        }
        fn inject_pointer_up(&mut self) {
            self.inner.inject_pointer_up();
        }
        fn inject_key(&mut self, key: &str) {
            self.inner.inject_key(key);
        }
        fn inject_scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32) {
            self.inner.inject_scroll(x, y, dx, dy);
        }
        fn inject_text(&mut self, text: &str) {
            self.inner.inject_text(text);
        }
        fn screenshot(&mut self, opts: ShotOptions) -> Option<Vec<u8>> {
            self.inner.screenshot(opts)
        }
        fn on_bridge_disconnect(&mut self) {
            self.disconnects.fetch_add(1, Ordering::AcqRel);
        }
    }

    let (mut bridge, port) = bridge_on_ephemeral_port();
    let mut host = CountingHost {
        inner: MockHost::new(),
        disconnects: std::sync::Arc::new(AtomicUsize::new(0)),
    };
    {
        let c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.set_nonblocking(false).unwrap();
        bridge.poll(&mut host);
        assert!(bridge.has_client());
        drop(c);
    }
    // 断连被察觉的那一帧：钩子调用一次。
    bridge.poll(&mut host);
    assert_eq!(host.disconnects.load(Ordering::Acquire), 1);
    // 之后的空 poll 不再打扰宿主（否则宿主每帧都在做无用收尾）。
    bridge.poll(&mut host);
    bridge.poll(&mut host);
    assert_eq!(
        host.disconnects.load(Ordering::Acquire),
        1,
        "钩子只该在连接消失的那一帧调用"
    );
    // 新客户端连上又断开：再通知一次（每次断连都算数）。
    {
        let c2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
        bridge.poll(&mut host);
        assert!(bridge.has_client());
        drop(c2);
    }
    bridge.poll(&mut host);
    assert_eq!(host.disconnects.load(Ordering::Acquire), 2);
}

/// 连接状态三态：旧模式无连出；连出模式区分拨号在途 / 连上 / 断开重连。
#[test]
fn bridge_state_distinguishes_outbound_phases() {
    // 旧模式（只监听）没有「连出去」这回事。
    let (bridge, _port) = bridge_on_ephemeral_port();
    assert_eq!(bridge.state(), super::BridgeState::Inactive);
    assert!(!bridge.is_connected());
    assert!(!bridge.is_connecting());

    // 假 MCP：监听、等应用连入。
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let mut bridge = DebugBridge {
        listener: None,
        link: Some(OutboundLink::new(addr)),
        conns: Vec::new(),
    };
    // OutboundLink::new 立刻 spawn，连接线程在途。
    assert_eq!(bridge.state(), super::BridgeState::Connecting);
    assert!(bridge.is_connecting());
    assert!(!bridge.is_connected());

    let acceptor = std::thread::spawn(move || listener.accept().unwrap());
    let mut host = MockHost::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !bridge.has_client() && std::time::Instant::now() < deadline {
        bridge.poll(&mut host);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    acceptor.join().unwrap();
    assert_eq!(bridge.state(), super::BridgeState::Connected);
    assert!(bridge.is_connected());
    assert!(!bridge.is_connecting());

    // 连出流断开（模拟 MCP 重启）：进入退避重连，既非连上也非拨号在途。
    bridge.conns.clear();
    bridge.link.as_mut().unwrap().disconnected();
    assert_eq!(bridge.state(), super::BridgeState::Reconnecting);
    assert!(!bridge.is_connected());
    assert!(!bridge.is_connecting());
}

/// P2-5：显式配置能在没有环境变量的情况下声明连入 MCP，
/// 并让 hello 帧报应用自己的身份（不是框架默认值）。
#[test]
fn with_config_declares_outbound_and_app_identity() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mcp = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = [0u8; 256];
        let n = stream.read(&mut line).unwrap();
        tx.send(String::from_utf8_lossy(&line[..n]).into_owned())
            .unwrap();
    });

    let config = super::BridgeConfig {
        connect_to: Some(addr),
        app_name: Some("my-app".into()),
        app_version: Some("9.9.9".into()),
        ..Default::default()
    };
    let mut bridge = DebugBridge::with_config(config).unwrap();
    let mut host = MockHost::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !bridge.is_connected() && std::time::Instant::now() < deadline {
        bridge.poll(&mut host);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // 让身份帧真的被 flush 出去（poll_connection 先 flush 再读命令）。
    for _ in 0..10 {
        bridge.poll(&mut host);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    mcp.join().unwrap();
    let hello = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(hello.trim(), "hello my-app 9.9.9");
}

/// 没声明任何拓扑 ⇒ 默认关闭，与「不设环境变量」同款零开销。
#[test]
fn with_empty_config_stays_disabled() {
    assert!(DebugBridge::with_config(super::BridgeConfig::default()).is_none());
}

/// 槽位被旧模式连接占满时，连出的流会被静默丢弃 —— 这时必须退避，
/// 否则 connecting 清空 + next_try 还是过去时刻 ⇒ 每帧 spawn 一个 connect 线程。
#[test]
fn full_slot_outbound_backs_off_instead_of_spamming_threads() {
    // 用 8 条真 TCP 连接占满 conns（MAX_CLIENTS）。
    let fill = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let fill_port = fill.local_addr().unwrap().port();
    let mut peers = Vec::new();
    let mut conns = Vec::new();
    for _ in 0..8 {
        let stream = TcpStream::connect(("127.0.0.1", fill_port)).unwrap();
        stream.set_nonblocking(true).unwrap();
        let (peer, _) = fill.accept().unwrap();
        peers.push(peer); // 保活对端，免得连接被判定为已死
        conns.push(super::Connection {
            stream,
            buf: Vec::new(),
            write_buf: Vec::new(),
            write_offset: 0,
            pending_wait: None,
        });
    }

    // 假 MCP：接受应用连入（这条流随后会被 push_connection 因满额丢弃）。
    let mcp = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = mcp.local_addr().unwrap();
    let acceptor = std::thread::spawn(move || mcp.accept());

    let mut bridge = DebugBridge {
        listener: None,
        link: Some(super::OutboundLink::new(addr)),
        conns,
    };
    let mut host = MockHost::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while bridge.link.as_ref().unwrap().connecting && std::time::Instant::now() < deadline {
        bridge.poll(&mut host);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let _ = acceptor.join().unwrap();

    let link = bridge.link.as_ref().unwrap();
    assert!(!link.connecting, "连接结果应当已回收");
    assert_eq!(bridge.conns.len(), 8, "连出的流因满额被丢弃");
    assert!(
        link.next_try > std::time::Instant::now(),
        "满额必须退避；否则下一帧立刻重连，形成每帧建线程的风暴"
    );
    // state() 看的是「有没有任意客户端」，8 条旧模式连接会让它报 Connected；
    // 这里钉住的是连出侧确实退避了，而不是每帧重建线程。
    assert!(!bridge.is_connecting());
    drop(peers);
}
