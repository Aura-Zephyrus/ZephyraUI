//! 截图落盘：BMP（协议原生）→ PNG（人看的）。
//!
//! # 为什么转换发生在**客户端**
//!
//! 桥协议里跑的字节是 BMP —— 它是**像素的裸排布**，编码成本几乎为零，
//! 库层（`zephyra-fx::bridge`）不需要任何编解码依赖。
//!
//! PNG 是**给人看**的：浏览器、聊天软件、Markdown 预览都认它，而 BMP 常常
//! 只能靠看图软件打开。所以转换放在落盘那一步（客户端），协议里不掺格式选择
//! —— 应用不知道也不需要知道你要 PNG 还是 BMP。
//!
//! # 格式由**扩展名**决定
//!
//! `--out shot.png` ⇒ PNG；`--out shot.bmp` ⇒ 原样 BMP。
//!
//! 不引入「格式标志」：文件名后缀是既有的、所有人都懂的约定，而多一个标志
//! 就多一处「两处说的不是一回事」的可能。

use std::path::Path;

/// 把桥回的字节写到 `path`。
///
/// 扩展名 `.png` 时做 BMP → PNG 转换；其余（`.bmp` / 无扩展名）原样写。
pub(crate) fn write_shot(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let is_png = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("png"))
        .unwrap_or(false);
    let data = if is_png {
        bmp_to_png(bytes)?
    } else {
        bytes.to_vec()
    };
    std::fs::write(path, data).map_err(|e| format!("写 {} 失败：{e}", path.display()))
}

/// BMP（24bpp BGR，自底向上）→ PNG 字节。
///
/// # 输入长什么样
///
/// 桥发的 BMP 是 `zephyra_fx::pixels::bmp_bytes` 产出的那一种：
///
/// - 54 字节头（`BITMAPFILEHEADER` + `BITMAPINFOHEADER`）；
/// - 24bpp、BGR、**自底向上**（BMP 的原生方向）；
/// - 每行按 4 字节对齐（`stride = (w*3 + 3) & !3`）。
///
/// # 为什么自己解 BMP 而不用现成 crate
///
/// BMP 头是**定长**的固定布局，解它只需要读几个小端整数；而现成的
/// BMP 解码器会带来一整棵依赖树（一件事做十行的反面）。
///
/// ⚠ 行对齐与上下翻转是两个独立的坑：忘了对齐会让后面的行**逐行错位**
/// （症状是图斜着歪下去），忘了翻转会得到上下颠倒的图（症状是「有东西，
/// 但不对」—— 更难发现）。
pub(crate) fn bmp_to_png(bmp: &[u8]) -> Result<Vec<u8>, String> {
    let img = parse_bmp24(bmp)?;
    let (width, height) = (img.width, img.height);

    // PNG 自顶向下；`row(y)` 的 y 也是**自顶向下**计数（翻转已在里面做了）。
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height as usize {
        let row = img.row(y)?;
        for px in row.chunks_exact(3) {
            // BMP 是 BGR，PNG 是 RGB。
            rgb.push(px[2]);
            rgb.push(px[1]);
            rgb.push(px[0]);
        }
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        // PNG 的压缩（DEFLATE）**无损** —— 像素值一个不变。压缩级别只影响文件大小
        // 与耗时，不影响画面：诊断截图要能看出「这块是不是脏区外残留的」。
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("写 PNG 头失败：{e}"))?;
        writer
            .write_image_data(&rgb)
            .map_err(|e| format!("写 PNG 数据失败：{e}"))?;
    }
    Ok(out)
}

/// 一张解开的 24bpp BMP。
struct Bmp24<'a> {
    width: u32,
    height: u32,
    /// 每行的字节数（4 字节对齐后的）。
    stride: usize,
    /// 像素是否**自顶向下**存储（BMP 的原生方向是自底向上）。
    top_down: bool,
    pixels: &'a [u8],
}

impl Bmp24<'_> {
    /// 第 `y` 行（**自顶向下**计数）的有效像素（不含行尾对齐字节）。
    fn row(&self, y: usize) -> Result<&[u8], String> {
        // 自底向上存储时，文件里的第一行是**最下面**那一行。
        let file_row = if self.top_down {
            y
        } else {
            self.height as usize - 1 - y
        };
        let start = file_row * self.stride;
        let end = start + self.width as usize * 3;
        self.pixels
            .get(start..end)
            .ok_or_else(|| "BMP 像素数据不足（行越界）".to_string())
    }
}

/// 解 BMP 头。
///
/// 只支持 **24bpp 未压缩**（BI_RGB）—— 桥发的那一种。别的格式明确报错：
/// 静默按 24bpp 解一张 8bpp 的图，得到的是一张「看起来像噪声」的图，
/// 那比报错难查得多。
fn parse_bmp24(bmp: &[u8]) -> Result<Bmp24<'_>, String> {
    if bmp.len() < 54 {
        return Err("BMP 太短（连头都不够）".into());
    }
    if &bmp[0..2] != b"BM" {
        return Err("不是 BMP 文件（缺 BM 魔数）".into());
    }
    let le_u32 = |off: usize| -> u32 {
        u32::from_le_bytes([bmp[off], bmp[off + 1], bmp[off + 2], bmp[off + 3]])
    };
    let le_i32 = |off: usize| -> i32 {
        i32::from_le_bytes([bmp[off], bmp[off + 1], bmp[off + 2], bmp[off + 3]])
    };
    let le_u16 = |off: usize| -> u16 { u16::from_le_bytes([bmp[off], bmp[off + 1]]) };

    let data_offset = le_u32(10) as usize;
    let header_size = le_u32(14);
    if header_size < 40 {
        return Err(format!("BMP 头格式不认识（头长 {header_size}）"));
    }
    let width = le_i32(18);
    let height = le_i32(22);
    let bpp = le_u16(28);
    let compression = le_u32(30);
    if bpp != 24 {
        return Err(format!("只支持 24bpp BMP，实得 {bpp}bpp"));
    }
    if compression != 0 {
        return Err(format!(
            "只支持未压缩 BMP（BI_RGB），实得压缩方式 {compression}"
        ));
    }
    if width <= 0 || height == 0 {
        return Err(format!("BMP 尺寸非法（{width}×{height}）"));
    }
    // 负高度 = 自顶向下存储（少见但合法）。这里取绝对值并**记住方向**。
    let top_down = height < 0;
    let height = height.unsigned_abs();
    let pixels = bmp
        .get(data_offset..)
        .ok_or_else(|| "BMP 像素偏移越界".to_string())?;
    Ok(Bmp24 {
        width: width as u32,
        height,
        stride: (width as usize * 3 + 3) & !3,
        top_down,
        pixels,
    })
}

/// 裁剪出一块矩形（**自顶向下**坐标）并**整数倍放大**，返回新的 BMP 字节。
///
/// # 为什么在**客户端**裁
///
/// 宿主那边要改的是「读回多大一块」—— 那是 GPU 侧的活（拷贝区、
/// 渲染目标尺寸），改起来要动两个后端。而客户端手里**已经有整张图**，
/// 裁一块只是搬字节。
///
/// 放大用**最近邻**（整数倍复制像素）：诊断截图要看的是「这块有没有脏东西」，
/// 插值会造出原图没有的边缘颜色 —— 那正好是要找的东西被改掉了。
///
/// # 越界怎么办
///
/// 矩形超出画面时**报错**，不静默裁成能放下的那块：静默裁剪会让
/// 「我截的是把手，结果图里是别的东西」无从查证。
pub(crate) fn crop_bmp(
    bmp: &[u8],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    scale: u32,
) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 {
        return Err("裁剪区域宽高不能为 0".into());
    }
    if scale == 0 || scale > 16 {
        return Err(format!("放大倍数要在 1..=16 之间，实得 {scale}"));
    }
    let img = parse_bmp24(bmp)?;
    if x + w > img.width || y + h > img.height {
        return Err(format!(
            "裁剪区域 {x},{y} {w}×{h} 超出画面 {}×{}",
            img.width, img.height
        ));
    }

    // 先取**自顶向下**的 RGB 行（丢掉 BGR 顺序的负担，后面统一写 BGR）。
    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(h as usize);
    for row_y in 0..h {
        let row = img.row((y + row_y) as usize)?;
        let mut rgb = Vec::with_capacity(w as usize * 3);
        for px in row.chunks_exact(3).skip(x as usize).take(w as usize) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        rows.push(rgb);
    }

    // 放大：每个像素复制 scale×scale 次。
    let out_w = w * scale;
    let out_h = h * scale;
    let mut out_rows: Vec<Vec<u8>> = Vec::with_capacity(out_h as usize);
    for row in &rows {
        let mut line = Vec::with_capacity(out_w as usize * 3);
        for px in row.chunks_exact(3) {
            for _ in 0..scale {
                // 回 BGR（BMP 的通道序）。
                line.extend_from_slice(&[px[2], px[1], px[0]]);
            }
        }
        // 纵向放大：同一行写 scale 次。
        for _ in 0..scale {
            out_rows.push(line.clone());
        }
    }

    Ok(encode_bmp24(out_w, out_h, &out_rows))
}

/// 编一张 24bpp BMP。
///
/// `rows` 是**自顶向下**的行（BGR，每行 `width*3` 字节）。
/// BMP 的原生方向是自底向上 —— 所以写的时候**倒着写**。
fn encode_bmp24(width: u32, height: u32, rows: &[Vec<u8>]) -> Vec<u8> {
    let stride = (width as usize * 3 + 3) & !3;
    let pixel_bytes = stride * height as usize;
    let mut out = Vec::with_capacity(54 + pixel_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    // 自底向上：最后一行先写。
    for row in rows.iter().rev() {
        let mut line = row.clone();
        line.resize(stride, 0);
        out.extend_from_slice(&line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一张 2×2 的 24bpp BMP（自底向上，BGR）。
    ///
    /// 像素（自顶向下看）：
    ///
    /// ```text
    /// 红 绿
    /// 蓝 白
    /// ```
    fn sample_bmp() -> Vec<u8> {
        let (w, h) = (2u32, 2u32);
        let stride = (w as usize * 3 + 3) & !3; // 2*3=6 → 8
        let mut pixels = vec![0u8; stride * h as usize];
        // BMP 自底向上：文件里的第一行是**最下面**那一行（蓝、白）。
        let bottom = &mut pixels[0..stride];
        bottom[0..3].copy_from_slice(&[255, 0, 0]); // 蓝（BGR）
        bottom[3..6].copy_from_slice(&[255, 255, 255]); // 白
        let top = &mut pixels[stride..stride * 2];
        top[0..3].copy_from_slice(&[0, 0, 255]); // 红（BGR）
        top[3..6].copy_from_slice(&[0, 255, 0]); // 绿

        let mut out = Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((54 + pixels.len()) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&pixels);
        out
    }

    /// PNG 编码必须**逐像素**还原原图 —— 上下颠倒、通道互换都会在这里露馅。
    #[test]
    fn bmp_to_png_preserves_pixels_top_down() {
        let png = bmp_to_png(&sample_bmp()).unwrap();
        let decoded = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoded.read_info().unwrap();
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.width, 2);
        assert_eq!(info.height, 2);
        let rgb = &buf[..info.buffer_size()];
        // 自顶向下：红、绿、蓝、白。
        assert_eq!(&rgb[0..3], &[255, 0, 0], "左上角应当是红");
        assert_eq!(&rgb[3..6], &[0, 255, 0], "右上角应当是绿");
        assert_eq!(&rgb[6..9], &[0, 0, 255], "左下角应当是蓝");
        assert_eq!(&rgb[9..12], &[255, 255, 255], "右下角应当是白");
    }

    #[test]
    fn bmp_to_png_rejects_non_bmp_input() {
        assert!(bmp_to_png(b"not a bmp").is_err());
        assert!(bmp_to_png(&[]).is_err());
    }

    /// 位深不是 24 的 BMP 要**明确报错** —— 静默按 24bpp 解会得到噪声图。
    #[test]
    fn bmp_to_png_rejects_other_bit_depths() {
        let mut bmp = sample_bmp();
        bmp[28] = 8; // 改成 8bpp
        let err = bmp_to_png(&bmp).unwrap_err();
        assert!(err.contains("24bpp"), "{err}");
    }

    /// 扩展名决定格式：`.png` 转、其余原样。
    #[test]
    fn write_shot_picks_the_format_by_extension() {
        let dir = std::env::temp_dir().join(format!("zephyra-shot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let png_path = dir.join("a.png");
        write_shot(&png_path, &sample_bmp()).unwrap();
        let bytes = std::fs::read(&png_path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG 签名不对");

        let bmp_path = dir.join("a.bmp");
        write_shot(&bmp_path, &sample_bmp()).unwrap();
        assert_eq!(std::fs::read(&bmp_path).unwrap(), sample_bmp());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 裁剪 + 放大：像素必须**逐个**对上，且放大不引入新颜色。
    ///
    /// ⚠ 上下颠倒是这里最容易出的错（BMP 自底向上）—— 所以断言里写死了
    /// 「裁左上角拿到的是红」。
    #[test]
    fn crop_and_scale_preserve_the_pixels() {
        let cropped = crop_bmp(&sample_bmp(), 0, 0, 1, 1, 1).unwrap();
        let img = parse_bmp24(&cropped).unwrap();
        assert_eq!((img.width, img.height), (1, 1));
        let row = img.row(0).unwrap();
        // 左上角是红（BGR 记法）。
        assert_eq!(&row[..3], &[0, 0, 255], "左上角应当是红");

        // 放大 3 倍 ⇒ 3×3，每个像素颜色不变。
        let scaled = crop_bmp(&sample_bmp(), 0, 0, 1, 1, 3).unwrap();
        let img = parse_bmp24(&scaled).unwrap();
        assert_eq!((img.width, img.height), (3, 3));
        for y in 0..3 {
            let row = img.row(y).unwrap();
            assert_eq!(&row[..3], &[0, 0, 255], "第 {y} 行应当还是红");
        }
    }

    /// 裁剪区域越界要**报错** —— 静默裁成能放下的那块会让「我截的是把手」
    /// 变成「我截的是别的东西」，而且无从查证。
    #[test]
    fn crop_rejects_a_rect_outside_the_image() {
        let err = crop_bmp(&sample_bmp(), 1, 0, 5, 1, 1).unwrap_err();
        assert!(err.contains("超出"), "{err}");
        assert!(crop_bmp(&sample_bmp(), 0, 0, 0, 1, 1).is_err());
        assert!(crop_bmp(&sample_bmp(), 0, 0, 1, 1, 0).is_err());
    }
}
