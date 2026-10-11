//! What Kerf does with a project, wherever it runs: holding the picture,
//! tracing it, turning it into heights, and generating the job. The desktop
//! app and the browser build both call into this, so they behave alike.
//!
//! Nothing here touches files, windows or the network.

pub mod design;
pub mod drawing;

use std::io::Cursor;
use std::sync::Arc;

use kerf_gcode::{Artwork, Heightmap, Job, OperationKind, Program};
use kerf_trace::image::{self, DynamicImage, ImageFormat};
use kerf_trace::TraceSettings;
use serde::{Deserialize, Serialize};

/// Longest side, in pixels, of the copy used for fast preview traces.
const PREVIEW_MAX_PX: u32 = 1024;

/// The picture being worked on.
pub struct LoadedImage {
    /// The image as imported, kept so that a crop can be undone.
    original: Arc<DynamicImage>,
    /// The image being worked on: the original, or a crop of it.
    pub full: Arc<DynamicImage>,
    /// A smaller copy, for quick preview traces.
    pub preview: Arc<DynamicImage>,
    /// Estimated depth for `full` (white is near), if the user has made one.
    pub depth: Option<Arc<DynamicImage>>,
}

impl LoadedImage {
    pub fn new(original: Arc<DynamicImage>, full: DynamicImage) -> Self {
        let preview = if full.width().max(full.height()) > PREVIEW_MAX_PX {
            full.thumbnail(PREVIEW_MAX_PX, PREVIEW_MAX_PX)
        } else {
            full.clone()
        };
        Self {
            original,
            full: Arc::new(full),
            preview: Arc::new(preview),
            depth: None,
        }
    }

    /// Reads an encoded image file: PNG, JPEG, WebP, BMP or GIF.
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let full = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        Ok(Self::new(Arc::new(full.clone()), full))
    }

    /// Keeps only a rectangle of the image, given in its own pixels. Any
    /// depth map is dropped, since it no longer matches.
    pub fn crop(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<(), String> {
        let (full_width, full_height) = (self.full.width(), self.full.height());
        if width == 0 || height == 0 || x >= full_width || y >= full_height {
            return Err("The crop is outside the image.".into());
        }
        let cropped =
            self.full
                .crop_imm(x, y, width.min(full_width - x), height.min(full_height - y));
        *self = Self::new(self.original.clone(), cropped);
        Ok(())
    }

    /// Undoes any crop, going back to the image as imported.
    pub fn reset_crop(&mut self) {
        *self = Self::new(self.original.clone(), (*self.original).clone());
    }
}

/// Reads an encoded depth picture (white is near). Nothing at all means none.
pub fn decode_depth(bytes: &[u8]) -> Result<Option<Arc<DynamicImage>>, String> {
    if bytes.is_empty() {
        return Ok(None);
    }
    image::load_from_memory(bytes)
        .map(|depth| Some(Arc::new(depth)))
        .map_err(|e| e.to_string())
}

/// A picture as a PNG file's bytes, for saving.
pub fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|e| format!("Could not encode the picture: {e}"))?;
    Ok(bytes.into_inner())
}

/// Curves are flattened to within this distance, in millimetres, for G-code.
pub const FLATTEN_TOLERANCE_MM: f64 = 0.05;
/// Longest side, in pixels, of the height map used for relief carving. A
/// router bit cannot resolve more, and the carving time grows with it.
const RELIEF_MAX_PX: u32 = 1500;
/// Lines of G-code returned to the interface as a preview.
const GCODE_HEAD_LINES: usize = 60;

/// A generated job, as the interface is told of it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    /// Statistics and preview paths. The program's text is handed over
    /// separately, since it can be very long.
    #[serde(flatten)]
    pub program: Program,
    /// The first lines of the program.
    pub head: String,
}

pub fn check_size(width_mm: f64, height_mm: f64) -> Result<(), String> {
    if width_mm.is_finite() && width_mm > 0.0 && height_mm.is_finite() && height_mm > 0.0 {
        Ok(())
    } else {
        Err("The artwork's width and height must be greater than zero.".into())
    }
}

/// Where a relief takes its heights from.
pub struct DepthSource {
    pub depth: Arc<DynamicImage>,
    /// How much of the picture's own detail is laid over the depth, 0 to 1.
    pub detail: f32,
    /// Depths below this share of the range become a flat background, 0 to 1.
    pub floor: f32,
}

/// The most the picture's own detail can add to or take from a depth relief,
/// as a share of its full depth.
const DETAIL_SHARE: f32 = 0.6;

/// Heights for a relief from an estimated depth map (`shape`, white is near)
/// and the picture it was estimated from.
///
/// The depth model gives the broad form but smooths away what is drawn on it,
/// so the picture's own detail is laid back over the form. Two bands are used:
/// a fine one for linework such as feathers, hair and fabric, and a wider one
/// for larger marks such as lettering, which the depth model sees as flat.
/// Broad light and dark areas are left out of both, so a dark coat does not
/// become a hollow. Depths below `floor` are flattened into a level background
/// and the rest stretched over the full depth, so the subject is not squeezed
/// into a small part of it.
fn blend_depth(
    shape: &image::GrayImage,
    photo: &image::GrayImage,
    detail: f32,
    floor: f32,
) -> Vec<f32> {
    let longest = shape.width().max(shape.height()) as f32;
    let fine = image::imageops::blur(photo, (longest / 250.0).max(1.0));
    let wide = image::imageops::blur(photo, (longest / 30.0).max(2.0));
    let strength = detail.clamp(0.0, 1.0) * DETAIL_SHARE;
    let floor = floor.clamp(0.0, 0.95);
    shape
        .as_raw()
        .iter()
        .zip(photo.as_raw())
        .zip(fine.as_raw().iter().zip(wide.as_raw()))
        .map(|((&z, &pixel), (&fine, &wide))| {
            let form = ((z as f32 / 255.0 - floor) / (1.0 - floor)).max(0.0);
            let linework = (pixel as f32 - fine as f32) / 255.0;
            let marks = (fine as f32 - wide as f32) / 255.0;
            (form + (linework * 1.5 + marks) * strength).clamp(0.0, 1.0)
        })
        .collect()
}

/// The image as heights: white is 1.0, black is 0.0, after the trace
/// settings' brightness, contrast and blur. With a depth source the heights
/// come from the depth map instead, with the photo's fine texture added.
pub fn heightmap(
    image: &DynamicImage,
    depth: Option<&DepthSource>,
    settings: &TraceSettings,
    width_mm: f64,
    height_mm: f64,
) -> Heightmap {
    let resized;
    let image = if image.width().max(image.height()) > RELIEF_MAX_PX {
        resized = image.thumbnail(RELIEF_MAX_PX, RELIEF_MAX_PX);
        &resized
    } else {
        image
    };
    let (width, height) = (image.width(), image.height());
    let data: Vec<f32> = match depth {
        None => kerf_trace::preprocess::grayscale(image, &settings.preprocess)
            .as_raw()
            .iter()
            .map(|&v| v as f32 / 255.0)
            .collect(),
        Some(source) => {
            let stretched =
                source
                    .depth
                    .resize_exact(width, height, image::imageops::FilterType::Triangle);
            let shape = kerf_trace::preprocess::grayscale(&stretched, &settings.preprocess);
            blend_depth(&shape, &image.to_luma8(), source.detail, source.floor)
        }
    };
    Heightmap {
        width: width as usize,
        height: height as usize,
        data,
        width_mm,
        height_mm,
    }
}

/// How a relief uses the estimated depth map.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthOptions {
    /// Carve from the depth map rather than from brightness.
    pub enabled: bool,
    pub detail: f32,
    pub floor: f32,
}

impl DepthOptions {
    /// The depth map with how it is to be used, given the one that is loaded.
    pub fn source(&self, depth: Option<Arc<DynamicImage>>) -> Result<DepthSource, String> {
        Ok(DepthSource {
            depth: depth.ok_or("Estimate the depth first, or use brightness.")?,
            detail: self.detail,
            floor: self.floor,
        })
    }
}

/// Text and drawings added to the material beside the main artwork.
#[derive(Deserialize)]
pub struct Added {
    /// An SVG the size of the bed, with each addition drawn where it sits.
    pub svg: String,
    pub operations: Vec<kerf_gcode::Operation>,
}

/// Everything a job is generated from.
pub struct JobInput {
    pub image: Arc<DynamicImage>,
    /// The design, at the artwork's size, where one takes the picture's place.
    pub design_svg: Option<String>,
    /// The loaded depth map, if there is one.
    pub depth_map: Option<Arc<DynamicImage>>,
    pub settings: TraceSettings,
    pub width_mm: f64,
    pub height_mm: f64,
    pub job: Job,
    pub depth: DepthOptions,
    /// Text and drawings added beside the main artwork, cut in the same job.
    pub added: Option<Added>,
    /// The space everything takes up on the bed (x, y, width, height), which
    /// cutouts go round.
    pub cutout_around: Option<[f64; 4]>,
}

/// Traces at full resolution and generates the G-code. Returns what the
/// interface is shown, and the program's full text.
pub fn generate(input: JobInput) -> Result<(Generated, String), String> {
    let JobInput {
        image,
        design_svg,
        depth_map,
        settings,
        width_mm,
        height_mm,
        job,
        depth,
        added,
        cutout_around,
    } = input;
    check_size(width_mm, height_mm)?;
    let is_relief = job
        .operations
        .iter()
        .any(|o| matches!(o.kind, OperationKind::CncRelief { .. }));
    let depth = if depth.enabled && is_relief {
        Some(depth.source(depth_map)?)
    } else {
        None
    };
    let carves_relief = job.operations.iter().any(|o| {
        matches!(
            o.kind,
            OperationKind::CncRelief { .. } | OperationKind::LaserImage { .. }
        )
    });
    if design_svg.is_some() && carves_relief {
        return Err("This design is made of lines to cut, so it cannot be engraved as a picture or carved as a relief. Use an operation that follows the lines.".into());
    }
    let svg = match design_svg {
        Some(svg) => svg,
        None => kerf_trace::trace(&image, &settings)?.to_svg(width_mm, height_mm),
    };
    let artwork = Artwork::from_svg(&svg, FLATTEN_TOLERANCE_MM)?;
    let heightmap =
        carves_relief.then(|| heightmap(&image, depth.as_ref(), &settings, width_mm, height_mm));
    let lettering = added
        .as_ref()
        .map(|added| Artwork::from_svg(&added.svg, FLATTEN_TOLERANCE_MM))
        .transpose()?;
    let additions = match (&lettering, &added) {
        (Some(artwork), Some(added)) => Some(kerf_gcode::Additions {
            artwork,
            operations: &added.operations,
        }),
        _ => None,
    };
    let mut program = kerf_gcode::generate_composed(
        &artwork,
        heightmap.as_ref(),
        &job,
        additions.as_ref(),
        cutout_around,
    )?;
    let text = std::mem::take(&mut program.text);
    let head = text
        .lines()
        .take(GCODE_HEAD_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    Ok((Generated { program, head }, text))
}
#[cfg(test)]
mod tests {
    use super::*;
    use kerf_trace::image::{GrayImage, Luma};

    /// A flat mid-grey form with a picture on it.
    fn blended(photo: &GrayImage, detail: f32, floor: f32) -> Vec<f32> {
        let shape = GrayImage::from_pixel(photo.width(), photo.height(), Luma([128]));
        blend_depth(&shape, photo, detail, floor)
    }

    #[test]
    fn detail_carves_thin_lines_into_a_smooth_form() {
        // A light picture with one thin dark line down the middle.
        let photo = GrayImage::from_fn(200, 100, |x, _| {
            Luma([if (99..101).contains(&x) { 20 } else { 220 }])
        });
        let plain = blended(&photo, 0.0, 0.0);
        assert!(
            plain.iter().all(|&h| (h - 128.0 / 255.0).abs() < 1e-6),
            "no detail leaves the form untouched"
        );

        let detailed = blended(&photo, 1.0, 0.0);
        let at = |x: usize| detailed[50 * 200 + x];
        // The line is cut into the form; far from it the form is unchanged.
        assert!(
            at(100) < at(20) - 0.2,
            "line {} against surround {}",
            at(100),
            at(20)
        );
        assert!((at(20) - 128.0 / 255.0).abs() < 0.03);
    }

    #[test]
    fn broad_dark_areas_do_not_become_hollows() {
        // The left half is dark, the right half light: a coat, not a hole.
        let photo = GrayImage::from_fn(400, 100, |x, _| Luma([if x < 200 { 30 } else { 230 }]));
        let detailed = blended(&photo, 1.0, 0.0);
        let at = |x: usize| detailed[50 * 400 + x];
        // Well inside each half the height is the form's, whatever its shade.
        assert!(
            (at(40) - at(360)).abs() < 0.05,
            "dark {} against light {}",
            at(40),
            at(360)
        );
    }

    #[test]
    fn the_floor_flattens_the_background_and_stretches_the_rest() {
        let photo = GrayImage::from_pixel(4, 1, Luma([128]));
        let shape = GrayImage::from_fn(4, 1, |x, _| Luma([[0, 51, 153, 255][x as usize]]));
        let heights = blend_depth(&shape, &photo, 0.0, 0.2);
        // 0 and 0.2 are both background; 0.6 lands halfway; 1.0 stays the top.
        assert!(heights[0].abs() < 1e-6 && heights[1].abs() < 1e-6);
        assert!((heights[2] - 0.5).abs() < 0.01 && (heights[3] - 1.0).abs() < 1e-6);
    }
}
