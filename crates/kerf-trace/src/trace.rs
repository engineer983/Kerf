//! Tracing a raster image into SVG paths with `vtracer`.

use image::DynamicImage;
use serde::{Deserialize, Serialize};
use visioncortex::PathSimplifyMode;
use vtracer::{ColorImage, ColorMode, Config, Hierarchical};

use crate::preprocess::{binarize, grayscale, Preprocess};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// One black shape per dark region, for cutting or single-colour engraving.
    Outline,
    /// One shape per colour region, without overlaps between shapes.
    Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceSettings {
    pub mode: Mode,
    /// Only used in [`Mode::Outline`].
    pub preprocess: Preprocess,
    /// Regions smaller than `speckle` × `speckle` pixels are dropped.
    pub speckle: u32,
    /// Fit smooth curves; otherwise output straight-edged polygons.
    pub curves: bool,
    /// Minimum turn, in degrees, that is kept as a sharp corner.
    pub corner_threshold: u32,
    /// Significant bits per colour channel, 1 to 8. Only used in [`Mode::Color`].
    pub color_precision: u32,
    /// Colour difference between layers. Only used in [`Mode::Color`].
    pub layer_difference: u32,
}

impl Default for TraceSettings {
    fn default() -> Self {
        Self {
            mode: Mode::Outline,
            preprocess: Preprocess::default(),
            speckle: 4,
            curves: true,
            corner_threshold: 60,
            color_precision: 6,
            layer_difference: 16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Traced {
    /// Size in pixels of the image that was traced; path coordinates use this space.
    pub width: u32,
    pub height: u32,
    pub path_count: usize,
    /// `<path>` elements, without the surrounding `<svg>`.
    pub body: String,
}

impl Traced {
    /// A complete SVG document sized in millimetres, keeping the aspect ratio.
    pub fn to_svg(&self, width_mm: f64) -> String {
        let height_mm = width_mm * self.height as f64 / self.width as f64;
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width_mm:.3}mm\" height=\"{height_mm:.3}mm\" viewBox=\"0 0 {} {}\">\n\
             {}</svg>\n",
            self.width, self.height, self.body
        )
    }
}

pub fn trace(img: &DynamicImage, settings: &TraceSettings) -> Result<Traced, String> {
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 {
        return Err("The image is empty.".into());
    }

    let pixels = match settings.mode {
        Mode::Outline => {
            let gray = grayscale(img, &settings.preprocess);
            DynamicImage::ImageLuma8(binarize(&gray, &settings.preprocess))
                .to_rgba8()
                .into_raw()
        }
        Mode::Color => img.to_rgba8().into_raw(),
    };

    let config = Config {
        color_mode: match settings.mode {
            Mode::Outline => ColorMode::Binary,
            Mode::Color => ColorMode::Color,
        },
        // Cutout keeps colour shapes from overlapping, so no area is engraved twice.
        hierarchical: Hierarchical::Cutout,
        filter_speckle: settings.speckle as usize,
        color_precision: settings.color_precision.clamp(1, 8) as i32,
        layer_difference: settings.layer_difference.min(255) as i32,
        mode: if settings.curves {
            PathSimplifyMode::Spline
        } else {
            PathSimplifyMode::Polygon
        },
        corner_threshold: settings.corner_threshold.min(180) as i32,
        ..Config::default()
    };

    let image = ColorImage {
        pixels,
        width: width as usize,
        height: height as usize,
    };
    let svg = vtracer::convert(image, config)?;

    Ok(Traced {
        width,
        height,
        path_count: svg.paths.len(),
        body: svg_body(&svg.to_string()).to_owned(),
    })
}

/// The markup between the opening `<svg ...>` tag and the closing `</svg>`.
fn svg_body(document: &str) -> &str {
    let start = document
        .find("<svg")
        .and_then(|open| document[open..].find('>').map(|close| open + close + 1));
    match (start, document.rfind("</svg>")) {
        (Some(start), Some(end)) if start <= end => document[start..end].trim_start(),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// White 64×48 image with a filled rectangle of `color`.
    fn rectangle(color: [u8; 4]) -> DynamicImage {
        let img = RgbaImage::from_fn(64, 48, |x, y| {
            if (16..48).contains(&x) && (12..36).contains(&y) {
                Rgba(color)
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn outline_traces_one_shape() {
        let traced = trace(&rectangle([0, 0, 0, 255]), &TraceSettings::default()).unwrap();
        assert_eq!((traced.width, traced.height), (64, 48));
        assert_eq!(traced.path_count, 1);
        assert_eq!(traced.body.matches("<path").count(), 1);
        assert!(!traced.body.contains("<svg"));
    }

    #[test]
    fn inverted_outline_traces_the_background() {
        let mut settings = TraceSettings::default();
        settings.preprocess.invert = true;
        let traced = trace(&rectangle([0, 0, 0, 255]), &settings).unwrap();
        // The white frame around the rectangle is now the single ink region.
        assert_eq!(traced.path_count, 1);
    }

    #[test]
    fn blank_image_has_no_paths() {
        let blank = DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([255; 4])));
        let settings = TraceSettings {
            preprocess: Preprocess {
                threshold: crate::Threshold::Manual(128),
                ..Preprocess::default()
            },
            ..TraceSettings::default()
        };
        let traced = trace(&blank, &settings).unwrap();
        assert_eq!(traced.path_count, 0);
        assert_eq!(traced.body, "");
    }

    #[test]
    fn color_mode_keeps_colors() {
        let settings = TraceSettings {
            mode: Mode::Color,
            ..TraceSettings::default()
        };
        let traced = trace(&rectangle([255, 0, 0, 255]), &settings).unwrap();
        assert!(traced.path_count >= 1);
        assert!(traced.body.to_lowercase().contains("#ff0000"));
    }

    #[test]
    fn svg_document_is_sized_in_mm() {
        let traced = trace(&rectangle([0, 0, 0, 255]), &TraceSettings::default()).unwrap();
        let svg = traced.to_svg(100.0);
        assert!(svg.contains("width=\"100.000mm\""));
        assert!(svg.contains("height=\"75.000mm\""));
        assert!(svg.contains("viewBox=\"0 0 64 48\""));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn empty_image_is_an_error() {
        let empty = DynamicImage::ImageRgba8(RgbaImage::new(0, 0));
        assert!(trace(&empty, &TraceSettings::default()).is_err());
    }
}
