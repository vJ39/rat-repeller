//! 起動時のスプラッシュ画面(AAピクセルアート表示)。
//!
//! `PixelCanvas`(ハーフブロック疑似2倍解像度)にロゴ画像を焼き込む。
//! Sixel/Kitty/iTerm2等の画像プロトコルには依存しない。

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, Rgba};
use ratatui::style::Color;

use crate::pixel_canvas::PixelCanvas;

/// 端末セル`target_cols`×`target_rows`ぶんの画面全体を覆うサイズでキャンバスを組み立てる。
/// 1端末セル=横1論理ピクセル・縦2論理ピクセル(ハーフブロック疑似2倍解像度)。
/// 画像全体が欠けずに収まる倍率(contain)で拡大縮小し、余った分は背景色のまま残す。
pub fn build_canvas(image: &DynamicImage, target_cols: u16, target_rows: u16) -> PixelCanvas {
    let background = Color::Black;
    let (src_w, src_h) = image.dimensions();

    let target_w = (target_cols as u32).max(1);
    let target_h = (target_rows as u32).max(1) * 2;

    let scale_w = target_w as f32 / src_w.max(1) as f32;
    let scale_h = target_h as f32 / src_h.max(1) as f32;
    let scale = scale_w.min(scale_h);
    let resized_w = ((src_w as f32) * scale).round().max(1.0) as u32;
    let resized_h = ((src_h as f32) * scale).round().max(1.0) as u32;

    let resized = image
        .resize_exact(resized_w, resized_h, FilterType::Lanczos3)
        .to_rgba8();

    let offset_x = (target_w as i64 - resized_w as i64) / 2;
    let offset_y = (target_h as i64 - resized_h as i64) / 2;

    let mut canvas = PixelCanvas::new(target_w as usize, target_h as usize, background);
    for (x, y, Rgba([r, g, b, _a])) in resized.enumerate_pixels() {
        let cx = x as i64 + offset_x;
        let cy = y as i64 + offset_y;
        if cx < 0 || cy < 0 || cx >= target_w as i64 || cy >= target_h as i64 {
            continue;
        }
        canvas.set(cx as usize, cy as usize, Color::Rgb(*r, *g, *b));
    }

    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_image(w: u32, h: u32) -> DynamicImage {
        DynamicImage::new_rgb8(w, h)
    }

    #[test]
    fn build_canvas_produces_non_empty_output() {
        let image = dummy_image(90, 30);
        let canvas = build_canvas(&image, 80, 24);
        let lines = canvas.to_lines(1.0);
        assert!(!lines.is_empty());
    }

    #[test]
    fn build_canvas_fills_the_exact_requested_terminal_size() {
        let image = dummy_image(90, 30);
        let canvas = build_canvas(&image, 100, 40);
        let lines = canvas.to_lines(1.0);
        assert_eq!(lines.len(), 40, "行数は要求したtarget_rowsと一致するはず");
        assert_eq!(
            lines[0].spans.len(),
            100,
            "幅は要求したtarget_colsと一致するはず"
        );
    }

    #[test]
    fn build_canvas_handles_a_tiny_terminal_without_panicking() {
        let image = dummy_image(90, 30);
        let canvas = build_canvas(&image, 1, 1);
        let lines = canvas.to_lines(1.0);
        assert!(!lines.is_empty());
    }
}
