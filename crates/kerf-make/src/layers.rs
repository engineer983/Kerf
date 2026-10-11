//! A picture cut as a stack of layers.
//!
//! The picture is read as heights, and sliced at evenly spaced levels like
//! the contour lines of a map. The first layer is a whole base; each layer
//! after it is the part of the picture that stands above the next level, to
//! be glued on top of the one before. Stacked, they step up towards the
//! picture's high points.

use kerf_trace::image::{imageops, DynamicImage, GrayImage, Luma};
use kerf_trace::{trace, Mode, Preprocess, Threshold, TraceSettings};
use serde::Deserialize;

use crate::{outline_path, Part, CUT_COLOR, MARK_COLOR};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerSpec {
    /// How many layers to cut, counting the base.
    pub layers: u32,
    /// Width of the finished piece in millimetres; its height follows the picture.
    pub width_mm: f64,
    /// Pieces smaller than this across, in millimetres, are left out: they
    /// would be too small to handle and glue.
    pub smallest_piece_mm: f64,
    /// Draw on each layer where the next one sits, as a guide for gluing.
    pub marks: bool,
}

pub const MOST_LAYERS: u32 = 12;
/// Longest side, in pixels, the heights are sliced at. Finer would only
/// trace the picture's noise.
const SLICE_PX: u32 = 1000;

/// The layers of `heights` (white is highest), bottom first.
pub fn layer_parts(heights: &GrayImage, spec: &LayerSpec) -> Result<Vec<Part>, String> {
    if !(2..=MOST_LAYERS).contains(&spec.layers) {
        return Err(format!("Choose between 2 and {MOST_LAYERS} layers."));
    }
    if !(spec.width_mm.is_finite() && spec.width_mm > 0.0) {
        return Err("The width must be greater than zero.".into());
    }
    if heights.width() == 0 || heights.height() == 0 {
        return Err("The picture is empty.".into());
    }

    let longest = heights.width().max(heights.height());
    let sized = if longest > SLICE_PX {
        DynamicImage::ImageLuma8(heights.clone())
            .thumbnail(SLICE_PX, SLICE_PX)
            .to_luma8()
    } else {
        heights.clone()
    };
    // Smoothed first, so that layer edges are clean curves rather than a
    // record of every speck in the picture.
    let longest = sized.width().max(sized.height()) as f32;
    let smooth = imageops::blur(&sized, (longest / 250.0).max(1.0));
    let (px_wide, px_high) = (smooth.width(), smooth.height());
    let mm_per_px = spec.width_mm / px_wide as f64;
    let height_mm = px_high as f64 * mm_per_px;
    let speckle = (spec.smallest_piece_mm.max(0.0) / mm_per_px)
        .round()
        .max(2.0) as u32;

    // The outline of everything standing at or above each level, in pixels.
    let mut regions: Vec<String> = Vec::new();
    for layer in 1..spec.layers {
        let level = (255.0 * layer as f64 / spec.layers as f64).round() as u8;
        let mask = GrayImage::from_fn(px_wide, px_high, |x, y| {
            Luma([if smooth.get_pixel(x, y).0[0] >= level {
                0
            } else {
                255
            }])
        });
        let settings = TraceSettings {
            mode: Mode::Outline,
            preprocess: Preprocess {
                threshold: Threshold::Manual(128),
                ..Preprocess::default()
            },
            speckle,
            ..TraceSettings::default()
        };
        let traced = trace(&DynamicImage::ImageLuma8(mask), &settings)?;
        if traced.path_count == 0 {
            // Nothing stands this high, so nothing stands higher either.
            break;
        }
        regions.push(traced.body);
    }
    if regions.is_empty() {
        return Err("The picture is too even to slice into layers. Try Optimise picture first, or fewer layers.".into());
    }

    let scaled = |region: &str, color: &str| {
        format!(
            "<g transform=\"scale({mm_per_px:.6})\">{}</g>",
            restroke(region, color)
        )
    };
    let frame = [
        [0.0, 0.0],
        [spec.width_mm, 0.0],
        [spec.width_mm, height_mm],
        [0.0, height_mm],
    ];
    let count = regions.len() + 1;
    Ok((0..count)
        .map(|index| {
            let mut markup = match index {
                0 => outline_path(&frame, CUT_COLOR),
                _ => scaled(&regions[index - 1], CUT_COLOR),
            };
            if let (true, Some(next)) = (spec.marks, regions.get(index)) {
                // Marks come first, so they are made before the piece is cut free.
                markup = scaled(next, MARK_COLOR) + &markup;
            }
            Part {
                name: if index == 0 {
                    "layer 1 (the base)".to_owned()
                } else {
                    format!("layer {}", index + 1)
                },
                width: spec.width_mm,
                height: height_mm,
                markup,
            }
        })
        .collect())
}

/// The tracer draws filled shapes; these are lines to follow, in `color`.
fn restroke(body: &str, color: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("fill=\"") {
        out.push_str(&rest[..start]);
        out.push_str(&format!("stroke=\"{color}\""));
        let after = &rest[start + 6..];
        rest = after.find('"').map_or("", |end| &after[end + 1..]);
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A round hill: highest in the middle, falling away to the edges.
    fn hill() -> GrayImage {
        GrayImage::from_fn(200, 150, |x, y| {
            let distance = (x as f64 - 100.0).hypot(y as f64 - 75.0);
            Luma([(255.0 - distance * 2.6).clamp(0.0, 255.0) as u8])
        })
    }

    fn spec(layers: u32) -> LayerSpec {
        LayerSpec {
            layers,
            width_mm: 100.0,
            smallest_piece_mm: 3.0,
            marks: true,
        }
    }

    #[test]
    fn a_hill_becomes_a_base_and_rings_stepping_up() {
        let parts = layer_parts(&hill(), &spec(5)).unwrap();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].name, "layer 1 (the base)");
        assert_eq!(parts[4].name, "layer 5");
        for part in &parts {
            assert_eq!((part.width, part.height), (100.0, 75.0));
            // Every layer is cut out; nothing is left filled in.
            assert!(part.markup.contains(&format!("stroke=\"{CUT_COLOR}\"")));
            assert!(!part.markup.contains("fill="));
        }
        // Each layer but the top one shows where the next sits.
        assert!(parts[..4].iter().all(|p| p.markup.contains(MARK_COLOR)));
        assert!(!parts[4].markup.contains(MARK_COLOR));
        // The base is the whole rectangle.
        assert!(parts[0].markup.contains("L100.000 75.000"));
        // Paths are drawn in pixels and scaled to millimetres: 100 mm over 200 px.
        assert!(parts[1].markup.contains("scale(0.500000)"));
    }

    #[test]
    fn marks_can_be_left_out_and_empty_levels_are_skipped() {
        let mut plain = spec(4);
        plain.marks = false;
        assert!(layer_parts(&hill(), &plain)
            .unwrap()
            .iter()
            .all(|p| !p.markup.contains(MARK_COLOR)));

        // A picture that only reaches half height has no upper layers.
        let low = GrayImage::from_fn(120, 120, |x, _| Luma([if x < 60 { 0 } else { 120 }]));
        let parts = layer_parts(&low, &spec(8)).unwrap();
        assert_eq!(parts.len(), 4);
    }

    #[test]
    fn pictures_that_cannot_be_sliced_are_refused() {
        let flat = GrayImage::from_pixel(100, 100, Luma([10]));
        assert!(layer_parts(&flat, &spec(4))
            .unwrap_err()
            .contains("too even"));
        assert!(layer_parts(&hill(), &spec(1)).is_err());
        assert!(layer_parts(&hill(), &spec(40)).is_err());
        assert!(layer_parts(
            &hill(),
            &LayerSpec {
                width_mm: 0.0,
                ..spec(4)
            }
        )
        .is_err());
    }

    #[test]
    fn fills_become_strokes() {
        assert_eq!(
            restroke(
                "<path d=\"M0 0\" fill=\"#000000\" transform=\"translate(1,2)\"/>",
                "#123456"
            ),
            "<path d=\"M0 0\" stroke=\"#123456\" transform=\"translate(1,2)\"/>"
        );
    }
}
