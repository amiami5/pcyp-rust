//! アプリのアイコン (青い丸に白い再生の三角)。
//!
//! アプリの窓とタスクトレイのアイコンに使うほか、ビルドのとき (src/build.rs) に .ico を作って exe に埋め込む。
//! 外部のクレートを使わないので、build.rs からも `#[path]` でそのまま読み込める。

/// 1 ピクセルを縦横この数に分けて塗りの割合を取り、縁をなめらかにする
const SUPERSAMPLE: u32 = 4;

/// アイコンの RGBA (上の行から)。
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut v = vec![0u8; (size * size * 4) as usize];
    let c = size as f32 / 2.0;
    let r = c - 0.5;
    let n = SUPERSAMPLE;
    for y in 0..size {
        for x in 0..size {
            // 丸の中の点と、そのうち三角の中の点を数える
            let (mut in_circle, mut in_tri) = (0u32, 0u32);
            for sy in 0..n {
                for sx in 0..n {
                    let fx = x as f32 + (sx as f32 + 0.5) / n as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / n as f32;
                    let (dx, dy) = (fx - c, fy - c);
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    in_circle += 1;
                    let (tx, ty) = (dx / r, dy / r);
                    if tx > -0.35 && tx < 0.55 && ty.abs() < (0.55 - tx) * 0.62 {
                        in_tri += 1;
                    }
                }
            }
            if in_circle == 0 {
                continue;
            }
            // 丸の中は青と白を、三角の割合で混ぜる
            let t = in_tri as f32 / in_circle as f32;
            let mix = |blue: f32, white: f32| (blue + (white - blue) * t).round() as u8;
            let alpha = (in_circle as f32 / (n * n) as f32 * 255.0).round() as u8;
            let i = ((y * size + x) * 4) as usize;
            v[i..i + 4].copy_from_slice(&[mix(30.0, 255.0), mix(110.0, 255.0), mix(220.0, 255.0), alpha]);
        }
    }
    v
}

/// .ico ファイルの中身。大きさごとに 32 ビットの画像 (BMP の形) を入れる。
/// build.rs だけで使う (アプリ本体では使わない)。
#[allow(dead_code)]
pub fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| ico_image(s)).collect();
    let mut out = Vec::new();
    // ICONDIR: 予約、種類 (1 = アイコン)、枚数
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        // ICONDIRENTRY: 幅と高さ (256 は 0)、色数、予約、プレーン、ビット数、大きさ、位置
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// .ico の中の 1 枚: BITMAPINFOHEADER、下の行からの BGRA、透明の AND マスク
#[allow(dead_code)]
fn ico_image(size: u32) -> Vec<u8> {
    let rgba = icon_rgba(size);
    let mut out = Vec::new();
    let header: [u32; 10] = [40, size, size * 2, 0, 0, 0, 0, 0, 0, 0];
    for (i, v) in header.iter().enumerate() {
        match i {
            // biPlanes と biBitCount は 2 バイトずつ
            3 => {
                out.extend_from_slice(&1u16.to_le_bytes());
                out.extend_from_slice(&32u16.to_le_bytes());
            }
            _ => out.extend_from_slice(&v.to_le_bytes()),
        }
    }
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    // AND マスク (1 ビット、行は 4 バイト単位)。透明なところを 1 にする
    let row_bytes = size.div_ceil(32) * 4;
    for y in (0..size).rev() {
        let mut row = vec![0u8; row_bytes as usize];
        for x in 0..size {
            if rgba[((y * size + x) * 4 + 3) as usize] == 0 {
                row[(x / 8) as usize] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ico_layout() {
        let sizes = [16, 32, 256];
        let ico = ico_file(&sizes);
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 3, 0]);
        // 1 枚目: 16x16、32 ビット
        assert_eq!(&ico[6..10], &[16, 16, 0, 0]);
        assert_eq!(u16::from_le_bytes([ico[12], ico[13]]), 32);
        // 256 は 0 と書く
        assert_eq!(&ico[6 + 32..6 + 34], &[0, 0]);
        // 大きさの合計が合う (ヘッダー 40 + 画素 + マスク)
        let expect: usize = 6 + 16 * 3 + sizes.iter().map(|&s| 40 + (s * s * 4) as usize + (s.div_ceil(32) * 4 * s) as usize).sum::<usize>();
        assert_eq!(ico.len(), expect);
    }

    #[test]
    fn center_is_white_and_corner_is_transparent() {
        let s = 64;
        let v = icon_rgba(s);
        let px = |x: u32, y: u32| &v[((y * s + x) * 4) as usize..((y * s + x) * 4 + 4) as usize];
        assert_eq!(px(0, 0)[3], 0);
        assert_eq!(px(s / 2 + 2, s / 2), &[255, 255, 255, 255]);
        assert_eq!(px(s / 2, 4)[3], 255);
    }
}
