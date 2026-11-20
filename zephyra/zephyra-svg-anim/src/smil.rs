//! SVG SMIL 动画解析：从 SVG 文本里抽出 `<animate>` / `<animateTransform>` /
//! `<animateMotion>`，挂到对应的 `<path>` 上。
//!
//! # 与图标解析器的关系
//!
//! `zephyra_vector::icon::parse_svg` 只关心静态图标（viewBox + path），本模块在它
//! 之上补 SMIL —— 两段解析共用一套「不做完整 XML、只做鲁棒属性提取」的约定，
//! 所以 `<title>` / 注释 / 命名空间声明都不会干扰动画属性。
//!
//! # 支持范围
//!
//! - `<animate attributeName="opacity|fill|d">`
//! - `<animateTransform type="translate|scale|rotate">`
//! - `<animateMotion path="...">`
//! - 时间属性：`begin` / `dur` / `repeatCount` / `fill` / `calcMode` / `keyTimes` / `keySplines`
//! - 值属性：`values` / `from` / `to` / `by`
//!
//! 不识别的 `<animate>`（例如 `attributeName` 未知）被忽略而不是报错 ——
//! 一个 SVG 里有无关动画不该让整个图标加载失败。

use zephyra_bezier::Bezier;
use zephyra_vector::icon::{parse_path_data, parse_svg, FillRule, PaintMode};

use crate::types::*;

/// `#rgb` / `#rrggbb` / `#rrggbbaa` → RGBA(0..1)。
fn parse_hex_rgba(s: &str) -> Option<[f32; 4]> {
    let hex = s.strip_prefix('#')?;
    let ch = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
    match hex.len() {
        3 => Some([
            ch(0, 1)? as f32 / 15.0,
            ch(1, 1)? as f32 / 15.0,
            ch(2, 1)? as f32 / 15.0,
            1.0,
        ]),
        6 => Some([
            ch(0, 2)? as f32 / 255.0,
            ch(2, 2)? as f32 / 255.0,
            ch(4, 2)? as f32 / 255.0,
            1.0,
        ]),
        8 => Some([
            ch(0, 2)? as f32 / 255.0,
            ch(2, 2)? as f32 / 255.0,
            ch(4, 2)? as f32 / 255.0,
            ch(6, 2)? as f32 / 255.0,
        ]),
        _ => None,
    }
}

/// 解析带 SMIL 动画的 SVG 文本。
pub fn parse_animated_svg(text: &str) -> Result<AnimatedSvg, SvgAnimError> {
    let icon = parse_svg(text).map_err(|e| match e {
        zephyra_vector::icon::SvgError::NoSvgRoot => SvgAnimError::NoSvgRoot,
        zephyra_vector::icon::SvgError::NoViewBox => SvgAnimError::NoViewBox,
        zephyra_vector::icon::SvgError::BadViewBox => SvgAnimError::BadViewBox,
        zephyra_vector::icon::SvgError::NoPath => SvgAnimError::NoPath,
    })?;

    // 把 SVG 切成「一个 <path> 一段」，每段里找它自己的动画子元素。
    // <animate*> 必须出现在它要作用的那条 <path> 之后、下一条 <path> 之前。
    let mut paths: Vec<AnimatedPath> = Vec::new();
    let mut rest = text;
    // 跳过 <svg ...>，从第一个 <path 开始逐段处理。
    if let Some((_, after)) = next_tag(rest, "<svg") {
        rest = after;
    } else {
        return Err(SvgAnimError::NoSvgRoot);
    }

    while let Some((path_attrs, after_path)) = next_tag(rest, "<path") {
        rest = after_path;
        let paint_mode = if path_attrs.contains("fill=\"none\"") {
            PaintMode::None
        } else {
            // 解析 path 上的具体 fill 颜色（如 <set> 的基值来源）。
            attr(path_attrs, "fill")
                .and_then(parse_hex_rgba)
                .map(PaintMode::Solid)
                .unwrap_or(PaintMode::CurrentColor)
        };
        let fill_rule = if path_attrs.contains("fill-rule=\"evenodd\"")
            || path_attrs.contains("fill-rule='evenodd'")
        {
            FillRule::EvenOdd
        } else {
            FillRule::NonZero
        };
        let base = attr(path_attrs, "d")
            .and_then(|d| parse_path_data(d).ok())
            .unwrap_or_default();

        // 收集紧跟这条 <path> 的动画元素（直到下一条 <path>）。
        let mut animations = Vec::new();
        let mut anim_rest = rest;
        loop {
            let mut found: Option<(&str, &str, &str)> = None; // (tag, attrs, after)
            for tag in [
                "<animateMotion",
                "<animateTransform",
                "<animateColor",
                "<set",
                "<animate",
            ] {
                if let Some((attrs, after)) = next_tag(anim_rest, tag) {
                    // <animate> 是 <animateTransform> / <animateMotion> 的前缀：
                    // 若更长的标签在同一位置命中，就跳过短的那个。
                    if tag == "<animate" {
                        let shadowed = ["<animateMotion", "<animateTransform"]
                            .iter()
                            .any(|long| next_tag(anim_rest, long).map(|(_, a)| a) == Some(after));
                        if shadowed {
                            continue;
                        }
                    }
                    // 越靠前的匹配，after 越长 —— 选它。
                    if found.is_none() || after.len() > found.unwrap().2.len() {
                        found = Some((tag, attrs, after));
                    }
                }
            }
            match found {
                Some((tag, attrs, after)) => {
                    if let Some(anim) = parse_animation(tag, attrs, &base) {
                        animations.push(anim);
                    }
                    anim_rest = after;
                }
                None => break,
            }
        }

        paths.push(AnimatedPath {
            base,
            paint_mode,
            fill_rule,
            animations,
        });
    }

    if paths.is_empty() {
        return Err(SvgAnimError::NoPath);
    }
    Ok(AnimatedSvg {
        view_box: icon.view_box,
        paths,
    })
}

fn parse_animation(tag: &str, attrs: &str, base: &[Segment]) -> Option<Animation> {
    // <set> 用 dur 但不一定有 values，单独解析时间。
    if tag == "<set" {
        let timing = parse_timing(attrs)?;
        let attribute_name = attr(attrs, "attributeName")?.to_string();
        let to = attr(attrs, "to")?.to_string();
        return Some(Animation::Set(SetAnim {
            timing,
            attribute_name,
            to,
        }));
    }
    let timing = parse_timing(attrs)?;
    match tag {
        // 废弃但仍常见的 <animateColor>：与 fill 动画同义。
        "<animateColor" => {
            let values = parse_color_values(attrs)?;
            Some(Animation::Fill(FillAnim { timing, values }))
        }
        "<animateMotion" => {
            let path = attr(attrs, "path")
                .and_then(|d| parse_path_data(d).ok())
                .unwrap_or_default();
            if path.is_empty() {
                return None;
            }
            let key_points = attr(attrs, "keyPoints")
                .map(parse_f32_list)
                .unwrap_or_default();
            Some(Animation::Motion(MotionAnim {
                timing,
                path,
                rotate_mode: match attr(attrs, "rotate") {
                    Some("auto") => MotionRotate::Auto,
                    Some("auto-reverse") => MotionRotate::AutoReverse,
                    _ => MotionRotate::None,
                },
                mpath_href: attr(attrs, "mpath")
                    .or_else(|| attr(attrs, "href"))
                    .map(|s| s.to_string()),
                key_points,
            }))
        }
        "<animateTransform" => {
            let kind = match attr(attrs, "type")? {
                "translate" => TransformKind::Translate,
                "scale" => TransformKind::Scale,
                "rotate" => TransformKind::Rotate,
                "skewX" => TransformKind::SkewX,
                "skewY" => TransformKind::SkewY,
                _ => return None,
            };
            let values = parse_f32x3_values(attrs, &[0.0, 0.0, 0.0])?;
            Some(Animation::Transform(TransformAnim {
                timing,
                kind,
                values,
            }))
        }
        "<animate" => {
            let name = attr(attrs, "attributeName")?;
            match name {
                "opacity" => {
                    let values = parse_f32_values(attrs, 1.0)?;
                    Some(Animation::Opacity(OpacityAnim { timing, values }))
                }
                "fill" => {
                    let values = parse_color_values(attrs)?;
                    Some(Animation::Fill(FillAnim { timing, values }))
                }
                "d" => {
                    let values = parse_path_values(attrs, base)?;
                    Some(Animation::Path(PathAnim { timing, values }))
                }
                "x" | "y" | "width" | "height" | "r" | "cx" | "cy" => {
                    let values = parse_f32_values(attrs, 0.0)?;
                    Some(Animation::Geometry(GeometryAnim {
                        timing,
                        attribute_name: name.to_string(),
                        values,
                    }))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// 解析 SMIL 的公共时间属性。缺 `dur` 视为非法（0 时长动画没有意义）。
fn parse_timing(attrs: &str) -> Option<Timing> {
    let dur_ms = parse_time(attr(attrs, "dur")?)?;
    if dur_ms <= 0.0 {
        return None;
    }
    let begin_ms = attr(attrs, "begin").and_then(parse_time).unwrap_or(0.0);
    let repeat = match attr(attrs, "repeatCount") {
        Some("indefinite") => Repeat::Indefinite,
        Some(s) => Repeat::Count(s.parse().ok()?),
        None => match attr(attrs, "repeatDur") {
            Some("indefinite") => Repeat::Indefinite,
            Some(v) => parse_time(v)
                .filter(|d| *d > 0.0)
                .map(Repeat::Duration)
                .unwrap_or(Repeat::Count(1)),
            None => Repeat::Count(1),
        },
    };
    let fill = if attr(attrs, "fill") == Some("freeze") {
        FillMode::Freeze
    } else {
        FillMode::Remove
    };
    let calc_mode = match attr(attrs, "calcMode") {
        Some("discrete") => CalcMode::Discrete,
        Some("paced") => CalcMode::Paced,
        Some("spline") => CalcMode::Spline,
        _ => CalcMode::Linear,
    };
    let key_times = attr(attrs, "keyTimes")
        .map(parse_f32_list)
        .unwrap_or_default();
    let key_splines = attr(attrs, "keySplines")
        .map(parse_splines)
        .unwrap_or_default();
    let end_ms = attr(attrs, "end").and_then(parse_time);
    let additive_sum = attr(attrs, "additive") == Some("sum");
    let accumulate_sum = attr(attrs, "accumulate") == Some("sum");
    Some(Timing {
        begin_ms,
        dur_ms,
        repeat,
        fill,
        calc_mode,
        key_times,
        key_splines,
        end_ms,
        additive_sum,
        accumulate_sum,
    })
}

/// SMIL 时钟：`7878ms` / `1.5s` / `0.25s`。
fn parse_time(s: &str) -> Option<f32> {
    let s = s.trim();
    // SMIL clock-value: hh:mm:ss.frac / mm:ss.frac
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        let nums: Option<Vec<f32>> = parts.iter().map(|p| p.parse::<f32>().ok()).collect();
        let nums = nums?;
        let ms = match nums.len() {
            2 => nums[0] * 60_000.0 + nums[1] * 1000.0,
            3 => nums[0] * 3_600_000.0 + nums[1] * 60_000.0 + nums[2] * 1000.0,
            _ => return None,
        };
        return (ms.is_finite() && ms >= 0.0).then_some(ms);
    }
    for (suffix, scale) in [
        ("ms", 1.0),
        ("min", 60_000.0),
        ("h", 3_600_000.0),
        ("s", 1000.0),
    ] {
        if let Some(v) = s.strip_suffix(suffix) {
            return v
                .trim()
                .parse::<f32>()
                .ok()
                .map(|v| v * scale)
                .filter(|v| v.is_finite() && *v >= 0.0);
        }
    }
    // 裸数字按秒。
    s.parse::<f32>()
        .ok()
        .map(|v| v * 1000.0)
        .filter(|v| v.is_finite() && *v >= 0.0)
}

fn parse_f32_list(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ';' || c == ',' || c.is_ascii_whitespace())
        .filter_map(|t| t.trim().parse::<f32>().ok())
        .collect()
}

fn parse_splines(s: &str) -> Vec<Bezier> {
    let nums = parse_f32_list(s);
    nums.chunks_exact(4)
        .filter_map(|c| {
            if c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
                Some(Bezier::new(c[0], c[1], c[2], c[3]))
            } else {
                None
            }
        })
        .collect()
}

fn parse_f32_values(attrs: &str, base: f32) -> Option<Values<f32>> {
    if let Some(v) = attr(attrs, "values") {
        let list = parse_f32_list(v);
        if list.is_empty() {
            return None;
        }
        return Some(Values::List(list));
    }
    if let (Some(from), Some(to)) = (attr(attrs, "from"), attr(attrs, "to")) {
        let a = from.trim().parse().ok()?;
        let b = to.trim().parse().ok()?;
        return Some(Values::FromTo(a, b));
    }
    if let Some(by) = attr(attrs, "by") {
        return Some(Values::By(by.trim().parse().ok()?));
    }
    // 只有 to：从基值到目标。
    if let Some(to) = attr(attrs, "to") {
        return Some(Values::FromTo(base, to.trim().parse().ok()?));
    }
    None
}

fn parse_f32x3_values(attrs: &str, base: &[f32; 3]) -> Option<Values<[f32; 3]>> {
    // translate / scale 通常只有 2 个分量，rotate 是 3 个（angle cx cy）。
    let parse_triple = |s: &str| -> Option<[f32; 3]> {
        let v = parse_f32_list(s);
        match v.len() {
            1 => Some([v[0], 0.0, 0.0]), // skewX/skewY 只有一个角度
            2 => Some([v[0], v[1], 0.0]),
            3 => Some([v[0], v[1], v[2]]),
            _ => None,
        }
    };
    if let Some(v) = attr(attrs, "values") {
        let triples = v
            .split(';')
            .filter_map(|s| parse_triple(s.trim()))
            .collect::<Vec<_>>();
        if !triples.is_empty() {
            return Some(Values::List(triples));
        }
    }
    if let (Some(from), Some(to)) = (attr(attrs, "from"), attr(attrs, "to")) {
        return Some(Values::FromTo(parse_triple(from)?, parse_triple(to)?));
    }
    if let Some(by) = attr(attrs, "by") {
        return Some(Values::By(parse_triple(by)?));
    }
    if let Some(to) = attr(attrs, "to") {
        return Some(Values::FromTo(*base, parse_triple(to)?));
    }
    let _ = base;
    None
}

fn parse_color_values(attrs: &str) -> Option<Values<[f32; 4]>> {
    let parse_color = |s: &str| -> Option<[f32; 4]> {
        let s = s.trim();
        let named = match s {
            "red" => Some([1.0, 0.0, 0.0, 1.0]),
            "green" => Some([0.0, 0.5, 0.0, 1.0]),
            "blue" => Some([0.0, 0.0, 1.0, 1.0]),
            "white" => Some([1.0, 1.0, 1.0, 1.0]),
            "black" => Some([0.0, 0.0, 0.0, 1.0]),
            "transparent" => Some([0.0, 0.0, 0.0, 0.0]),
            _ => None,
        };
        if named.is_some() {
            return named;
        }
        if let Some(inner) = s
            .strip_prefix("rgba(")
            .and_then(|x| x.strip_suffix(')'))
            .or_else(|| s.strip_prefix("rgb(").and_then(|x| x.strip_suffix(')')))
        {
            let v: Vec<f32> = inner
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            if v.len() >= 3 {
                return Some([
                    (v[0] / 255.0).min(1.0),
                    (v[1] / 255.0).min(1.0),
                    (v[2] / 255.0).min(1.0),
                    if v.len() > 3 { v[3].min(1.0) } else { 1.0 },
                ]);
            }
        }
        let hex = s.strip_prefix('#')?;
        let ch = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
        match hex.len() {
            3 => Some([
                ch(0, 1)? as f32 / 15.0,
                ch(1, 1)? as f32 / 15.0,
                ch(2, 1)? as f32 / 15.0,
                1.0,
            ]),
            6 => Some([
                ch(0, 2)? as f32 / 255.0,
                ch(2, 2)? as f32 / 255.0,
                ch(4, 2)? as f32 / 255.0,
                1.0,
            ]),
            8 => Some([
                ch(0, 2)? as f32 / 255.0,
                ch(2, 2)? as f32 / 255.0,
                ch(4, 2)? as f32 / 255.0,
                ch(6, 2)? as f32 / 255.0,
            ]),
            _ => None,
        }
    };
    if let Some(v) = attr(attrs, "values") {
        // 注意：颜色值里没有分号冲突，`;` 是 SMIL 的分隔符。
        let list = v.split(';').filter_map(parse_color).collect::<Vec<_>>();
        if !list.is_empty() {
            return Some(Values::List(list));
        }
    }
    if let (Some(from), Some(to)) = (attr(attrs, "from"), attr(attrs, "to")) {
        return Some(Values::FromTo(parse_color(from)?, parse_color(to)?));
    }
    if let Some(to) = attr(attrs, "to") {
        return Some(Values::FromTo([1.0, 1.0, 1.0, 1.0], parse_color(to)?));
    }
    None
}

fn parse_path_values(attrs: &str, base: &[Segment]) -> Option<Values<Vec<Segment>>> {
    if let Some(v) = attr(attrs, "values") {
        // 路径的 `d` 里可能含分号吗？SVG path data 用空格/逗号，不用分号 —— 安全。
        let list = v
            .split(';')
            .filter_map(|s| parse_path_data(s.trim()).ok())
            .collect::<Vec<_>>();
        if !list.is_empty() {
            return Some(Values::List(list));
        }
    }
    if let (Some(from), Some(to)) = (attr(attrs, "from"), attr(attrs, "to")) {
        return Some(Values::FromTo(
            parse_path_data(from).ok()?,
            parse_path_data(to).ok()?,
        ));
    }
    if let Some(to) = attr(attrs, "to") {
        return Some(Values::FromTo(base.to_vec(), parse_path_data(to).ok()?));
    }
    None
}

/// 在文本中找下一个 `<tag`，返回 (属性串, `>` 之后的剩余文本)。
/// 引号内的 `>` 不算闭合，与 vector 的 svg.rs 同款约定。
fn next_tag<'a>(text: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(rel) = text[from..].find(tag) {
        let start = from + rel;
        let after = start + tag.len();
        match bytes.get(after) {
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'>') | Some(b'/') => {}
            _ => {
                from = after;
                continue;
            }
        }
        let mut i = after;
        let mut quote = 0u8;
        while i < bytes.len() {
            let b = bytes[i];
            if quote != 0 {
                if b == quote {
                    quote = 0;
                }
            } else if b == b'"' || b == b'\'' {
                quote = b;
            } else if b == b'>' {
                return Some((&text[after..i], &text[i + 1..]));
            }
            i += 1;
        }
        return None;
    }
    None
}

/// 读属性值（双引号 / 单引号均可）。
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let attr_name = &attrs[start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                if attr_name == name {
                    return Some(&attrs[v_start..i]);
                }
            }
            Some(_) => {
                let v_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if attr_name == name {
                    return Some(&attrs[v_start..i]);
                }
            }
            None => break,
        }
    }
    None
}
