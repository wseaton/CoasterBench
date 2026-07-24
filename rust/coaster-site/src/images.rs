//! The raster bits of the site: index thumbnails, the Open Graph card, and
//! the favicon. Everything is drawn from run screenshots plus a few filled
//! rectangles, so `image` alone covers it (no drawing crate needed).

use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, Rgb, RgbImage};

const PANEL: Rgb<u8> = Rgb([0xc2, 0xb2, 0x80]);
const PANEL_HI: Rgb<u8> = Rgb([0xdc, 0xcf, 0xa4]);
const PANEL_DARK: Rgb<u8> = Rgb([0x8c, 0x7a, 0x52]);
const TITLEBAR: Rgb<u8> = Rgb([0x5c, 0x3a, 0x24]);
const TITLEBAR_HI: Rgb<u8> = Rgb([0x8a, 0x5f, 0x3d]);
const INK: Rgb<u8> = Rgb([0x2a, 0x20, 0x15]);
const SKY: Rgb<u8> = Rgb([0x10, 0x20, 0x2e]);

/// Fills an inclusive rectangle, clipped to the image.
fn fill(img: &mut RgbImage, x0: i64, y0: i64, x1: i64, y1: i64, colour: Rgb<u8>) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    for y in y0.max(0)..=y1.min(h - 1) {
        for x in x0.max(0)..=x1.min(w - 1) {
            img.put_pixel(x as u32, y as u32, colour);
        }
    }
}

/// Draws an inclusive rectangle outline `width` pixels thick, inwards.
fn outline(img: &mut RgbImage, x0: i64, y0: i64, x1: i64, y1: i64, width: i64, colour: Rgb<u8>) {
    fill(img, x0, y0, x1, y0 + width - 1, colour);
    fill(img, x0, y1 - width + 1, x1, y1, colour);
    fill(img, x0, y0, x0 + width - 1, y1, colour);
    fill(img, x1 - width + 1, y0, x1, y1, colour);
}

/// Scales to cover `w`x`h` and center-crops. NEAREST when upscaling keeps the
/// pixel art crisp; LANCZOS is better going down.
fn cover_crop(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    let scale = (w as f64 / img.width() as f64).max(h as f64 / img.height() as f64);
    let filter = if scale > 1.0 {
        FilterType::Nearest
    } else {
        FilterType::Lanczos3
    };
    let scaled = img.resize_exact(
        ((img.width() as f64 * scale).round() as u32).max(w),
        ((img.height() as f64 * scale).round() as u32).max(h),
        filter,
    );
    let x = (scaled.width() - w) / 2;
    let y = (scaled.height() - h) / 2;
    scaled.crop_imm(x, y, w, h)
}

/// A 32x32 RCT-style window (tan bevel panel + brown titlebar) as favicon.ico.
pub fn write_favicon(out: &Path) -> Result<()> {
    let mut img = RgbImage::from_pixel(32, 32, SKY);
    fill(&mut img, 2, 4, 29, 27, PANEL);
    outline(&mut img, 2, 4, 29, 27, 2, INK);
    fill(&mut img, 4, 6, 27, 11, TITLEBAR);
    fill(&mut img, 4, 6, 27, 6, TITLEBAR_HI);
    fill(&mut img, 4, 25, 27, 25, PANEL_DARK);
    fill(&mut img, 27, 8, 27, 25, PANEL_DARK);
    fill(&mut img, 4, 8, 4, 24, PANEL_HI);
    let path = out.join("favicon.ico");
    DynamicImage::ImageRgb8(img)
        .save(&path)
        .with_context(|| format!("writing {}", path.display()))
}

/// Renders og-card.png: a screenshot center-cropped to 1200x630 inside an
/// RCT-style beveled panel frame.
pub fn write_og_card(shot: &Path, out: &Path) -> Result<()> {
    const W: u32 = 1200;
    const H: u32 = 630;
    const OUTER: i64 = 3;
    const BEVEL: i64 = 6;
    let border = OUTER + BEVEL + 2;
    let inner_w = W - 2 * border as u32;
    let inner_h = H - 2 * border as u32;

    let shot_img = image::open(shot).with_context(|| format!("reading {}", shot.display()))?;
    // Cover-fit so the whole ride reads at a glance (captures are cropped to
    // track bounds already).
    let cropped = cover_crop(&shot_img, inner_w, inner_h).to_rgb8();

    let mut card = RgbImage::from_pixel(W, H, PANEL);
    let (w, h) = (W as i64 - 1, H as i64 - 1);
    outline(&mut card, 0, 0, w, h, OUTER, INK);
    outline(
        &mut card,
        OUTER,
        OUTER,
        w - OUTER,
        h - OUTER,
        BEVEL,
        PANEL_HI,
    );
    // Bevel shading: dark on the bottom/right edges for the classic inset look.
    fill(
        &mut card,
        OUTER,
        h - OUTER - BEVEL,
        w - OUTER,
        h - OUTER,
        PANEL_DARK,
    );
    fill(
        &mut card,
        w - OUTER - BEVEL,
        OUTER,
        w - OUTER,
        h - OUTER,
        PANEL_DARK,
    );
    image::imageops::replace(&mut card, &cropped, border, border);

    let path = out.join("og-card.png");
    DynamicImage::ImageRgb8(card)
        .save(&path)
        .with_context(|| format!("writing {}", path.display()))
}

/// Writes a <=280px thumbnail of `shot` at `dest`, preserving aspect ratio.
pub fn write_thumbnail(shot: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let img = image::open(shot).with_context(|| format!("reading {}", shot.display()))?;
    img.resize(280, 280, FilterType::Lanczos3)
        .save(dest)
        .with_context(|| format!("writing {}", dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop_fills_the_target_exactly() {
        let src = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 50, SKY));
        let out = cover_crop(&src, 60, 40);
        assert_eq!((out.width(), out.height()), (60, 40));
        let up = cover_crop(&src, 400, 400);
        assert_eq!((up.width(), up.height()), (400, 400));
    }

    #[test]
    fn outline_draws_only_the_border() {
        let mut img = RgbImage::from_pixel(10, 10, PANEL);
        outline(&mut img, 0, 0, 9, 9, 2, INK);
        assert_eq!(img.get_pixel(0, 0), &INK);
        assert_eq!(img.get_pixel(1, 1), &INK);
        assert_eq!(img.get_pixel(2, 2), &PANEL);
        assert_eq!(img.get_pixel(9, 9), &INK);
    }
}
