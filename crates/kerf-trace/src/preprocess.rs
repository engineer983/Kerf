//! Turns a source image into the black-and-white mask that gets traced.

use image::{DynamicImage, GrayImage, Luma};
use serde::{Deserialize, Serialize};

/// Pixel value for ink (the part that will be traced) in a binarized mask.
pub const INK: u8 = 0;
/// Pixel value for background in a binarized mask.
pub const PAPER: u8 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "level", rename_all = "camelCase")]
pub enum Threshold {
    /// Pick the level automatically with Otsu's method.
    Otsu,
    /// Pixels at or below this level become ink.
    Manual(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Preprocess {
    /// -1.0 (black) to 1.0 (white); 0.0 leaves the image unchanged.
    pub brightness: f32,
    /// -1.0 (flat grey) to 1.0 (maximum); 0.0 leaves the image unchanged.
    pub contrast: f32,
    /// Gaussian blur radius in pixels; 0.0 disables it.
    pub blur_sigma: f32,
    pub threshold: Threshold,
    /// Trace the light areas instead of the dark ones.
    pub invert: bool,
}

impl Default for Preprocess {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            blur_sigma: 0.0,
            threshold: Threshold::Otsu,
            invert: false,
        }
    }
}

/// Converts to greyscale (transparency becomes white), then applies
/// brightness, contrast and blur.
pub fn grayscale(img: &DynamicImage, p: &Preprocess) -> GrayImage {
    let rgba = img.to_rgba8();
    let contrast = p.contrast.clamp(-1.0, 0.999);
    let gain = (1.0 + contrast) / (1.0 - contrast);
    let offset = p.brightness.clamp(-1.0, 1.0);

    let mut gray = GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let alpha = a as f32 / 255.0;
        let luma = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;
        // Composite over white, as an image viewer with a white page would.
        let v = luma * alpha + (1.0 - alpha);
        let v = (v - 0.5) * gain + 0.5 + offset;
        Luma([(v.clamp(0.0, 1.0) * 255.0).round() as u8])
    });

    if p.blur_sigma > 0.0 {
        gray = image::imageops::blur(&gray, p.blur_sigma);
    }
    gray
}

/// Otsu's method: the level that best separates dark from light pixels.
/// Pixels at or below the returned level belong to the dark class.
pub fn otsu_level(gray: &GrayImage) -> u8 {
    let mut histogram = [0u64; 256];
    for p in gray.pixels() {
        histogram[p.0[0] as usize] += 1;
    }
    let total = gray.pixels().len() as f64;
    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(v, &n)| v as f64 * n as f64)
        .sum();

    let mut best_level = 0u8;
    let mut best_variance = 0.0;
    let mut weight_dark = 0.0;
    let mut sum_dark = 0.0;
    for (level, &count) in histogram.iter().enumerate() {
        weight_dark += count as f64;
        sum_dark += level as f64 * count as f64;
        let weight_light = total - weight_dark;
        if weight_dark == 0.0 || weight_light == 0.0 {
            continue;
        }
        let mean_dark = sum_dark / weight_dark;
        let mean_light = (sum_all - sum_dark) / weight_light;
        let variance = weight_dark * weight_light * (mean_dark - mean_light).powi(2);
        if variance > best_variance {
            best_variance = variance;
            best_level = level as u8;
        }
    }
    best_level
}

/// Reduces a greyscale image to [`INK`] and [`PAPER`] pixels.
pub fn binarize(gray: &GrayImage, p: &Preprocess) -> GrayImage {
    let level = match p.threshold {
        Threshold::Otsu => otsu_level(gray),
        Threshold::Manual(level) => level,
    };
    GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let dark = gray.get_pixel(x, y).0[0] <= level;
        Luma([if dark != p.invert { INK } else { PAPER }])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn two_tone() -> GrayImage {
        GrayImage::from_fn(8, 8, |x, _| Luma([if x < 4 { 50 } else { 200 }]))
    }

    #[test]
    fn otsu_separates_two_tones() {
        let level = otsu_level(&two_tone());
        assert!((50..200).contains(&level), "level was {level}");
    }

    #[test]
    fn binarize_marks_dark_pixels_as_ink() {
        let mask = binarize(&two_tone(), &Preprocess::default());
        assert_eq!(mask.get_pixel(0, 0).0[0], INK);
        assert_eq!(mask.get_pixel(7, 0).0[0], PAPER);
    }

    #[test]
    fn invert_swaps_ink_and_paper() {
        let p = Preprocess {
            invert: true,
            ..Preprocess::default()
        };
        let mask = binarize(&two_tone(), &p);
        assert_eq!(mask.get_pixel(0, 0).0[0], PAPER);
        assert_eq!(mask.get_pixel(7, 0).0[0], INK);
    }

    #[test]
    fn manual_threshold_is_inclusive() {
        let gray = GrayImage::from_fn(3, 1, |x, _| Luma([[99, 100, 101][x as usize]]));
        let p = Preprocess {
            threshold: Threshold::Manual(100),
            ..Preprocess::default()
        };
        let mask = binarize(&gray, &p);
        let values: Vec<u8> = mask.pixels().map(|p| p.0[0]).collect();
        assert_eq!(values, [INK, INK, PAPER]);
    }

    #[test]
    fn transparent_pixels_become_white() {
        let img = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 0]));
        let gray = grayscale(&DynamicImage::ImageRgba8(img), &Preprocess::default());
        assert_eq!(gray.get_pixel(0, 0).0[0], 255);
    }

    #[test]
    fn brightness_and_contrast_shift_values() {
        let img = DynamicImage::ImageLuma8(GrayImage::from_pixel(1, 1, Luma([64])));
        let brighter = Preprocess {
            brightness: 0.25,
            ..Preprocess::default()
        };
        assert!(grayscale(&img, &brighter).get_pixel(0, 0).0[0] > 64);
        let punchier = Preprocess {
            contrast: 0.5,
            ..Preprocess::default()
        };
        assert!(grayscale(&img, &punchier).get_pixel(0, 0).0[0] < 64);
    }
}
