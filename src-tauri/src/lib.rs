mod assistant;
mod design;
mod drawing;
mod gallery;
mod machine;
mod pictures;
mod web;

use std::sync::{Arc, Mutex};

use kerf_gcode::{Artwork, Heightmap, Job, OperationKind, Program};
use kerf_trace::image::{self, DynamicImage};
use kerf_trace::{TraceSettings, Traced};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// Longest side, in pixels, of the copy used for fast preview traces.
const PREVIEW_MAX_PX: u32 = 1024;

pub(crate) struct LoadedImage {
    /// The image as imported, kept so that a crop can be undone.
    original: Arc<DynamicImage>,
    /// The image being worked on: the original, or a crop of it.
    pub(crate) full: Arc<DynamicImage>,
    preview: Arc<DynamicImage>,
    /// Estimated depth for `full` (white is near), if the user has made one.
    pub(crate) depth: Option<Arc<DynamicImage>>,
}

impl LoadedImage {
    pub(crate) fn new(original: Arc<DynamicImage>, full: DynamicImage) -> Self {
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
}

/// Curves are flattened to within this distance, in millimetres, for G-code.
pub(crate) const FLATTEN_TOLERANCE_MM: f64 = 0.05;
/// Longest side, in pixels, of the height map used for relief carving. A
/// router bit cannot resolve more, and the carving time grows with it.
const RELIEF_MAX_PX: u32 = 1500;
/// Lines of G-code returned to the UI as a preview.
const GCODE_HEAD_LINES: usize = 60;

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) image: Mutex<Option<LoadedImage>>,
    /// A box or layered design, which takes the place of the traced picture.
    pub(crate) design: Mutex<Option<design::Design>>,
    /// Text of the most recently generated program, kept here for export and sending.
    pub(crate) gcode: Mutex<Option<String>>,
}

impl AppState {
    pub(crate) fn image(&self, preview: bool) -> Result<Arc<DynamicImage>, String> {
        let guard = self.image.lock().unwrap();
        let loaded = guard.as_ref().ok_or("No image is loaded.")?;
        Ok(if preview {
            loaded.preview.clone()
        } else {
            loaded.full.clone()
        })
    }
}

#[derive(Serialize)]
struct AppInfo {
    name: &'static str,
    version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Kerf",
        version: env!("CARGO_PKG_VERSION"),
    }
}

pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

/// Takes the encoded image file as the raw request body.
#[tauri::command]
async fn load_image(request: Request<'_>, state: State<'_, AppState>) -> Result<(), String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected the image file as raw bytes.".into());
    };
    let bytes = bytes.clone();
    let loaded = blocking(move || {
        let full = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        Ok(LoadedImage::new(Arc::new(full.clone()), full))
    })
    .await?;
    *state.image.lock().unwrap() = Some(loaded);
    // A new picture starts a new piece of work.
    *state.design.lock().unwrap() = None;
    Ok(())
}

#[tauri::command]
fn clear_image(state: State<'_, AppState>) {
    *state.image.lock().unwrap() = None;
    *state.design.lock().unwrap() = None;
}

/// Keeps only a rectangle of the image, given in pixels of the current image.
/// Any depth map is dropped, since it no longer matches.
#[tauri::command]
fn crop_image(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut guard = state.image.lock().unwrap();
    let loaded = guard.as_mut().ok_or("No image is loaded.")?;
    let (full_width, full_height) = (loaded.full.width(), loaded.full.height());
    if width == 0 || height == 0 || x >= full_width || y >= full_height {
        return Err("The crop is outside the image.".into());
    }
    let cropped =
        loaded
            .full
            .crop_imm(x, y, width.min(full_width - x), height.min(full_height - y));
    *loaded = LoadedImage::new(loaded.original.clone(), cropped);
    Ok(())
}

/// Undoes any crop, going back to the image as imported.
#[tauri::command]
fn reset_crop(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.image.lock().unwrap();
    let loaded = guard.as_mut().ok_or("No image is loaded.")?;
    *loaded = LoadedImage::new(loaded.original.clone(), (*loaded.original).clone());
    Ok(())
}

/// Takes an encoded greyscale depth picture of the current image (white is
/// near) as the raw request body. An empty body removes the depth map.
#[tauri::command]
async fn set_depth_map(request: Request<'_>, state: State<'_, AppState>) -> Result<(), String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected the depth picture as raw bytes.".into());
    };
    let depth = if bytes.is_empty() {
        None
    } else {
        let bytes = bytes.clone();
        Some(Arc::new(
            blocking(move || image::load_from_memory(&bytes).map_err(|e| e.to_string())).await?,
        ))
    };
    let mut guard = state.image.lock().unwrap();
    guard.as_mut().ok_or("No image is loaded.")?.depth = depth;
    Ok(())
}

#[tauri::command]
async fn trace_image(
    settings: TraceSettings,
    preview: bool,
    state: State<'_, AppState>,
) -> Result<Traced, String> {
    if let Some(design) = state.design.lock().unwrap().as_ref() {
        return Ok(design.traced());
    }
    let image = state.image(preview)?;
    blocking(move || kerf_trace::trace(&image, &settings)).await
}

/// Traces at full resolution and writes the SVG where the user chooses.
/// Returns the saved path, or `None` if the dialog was cancelled.
#[tauri::command]
async fn export_svg(
    app: AppHandle,
    settings: TraceSettings,
    width_mm: f64,
    height_mm: f64,
    suggested_name: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    check_size(width_mm, height_mm)?;
    let image = state.image(false)?;
    let design = state
        .design
        .lock()
        .unwrap()
        .as_ref()
        .map(|design| design.to_svg(width_mm, height_mm));
    blocking(move || {
        let Some(target) = app
            .dialog()
            .file()
            .add_filter("SVG", &["svg"])
            .set_file_name(suggested_name)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = target.into_path().map_err(|e| e.to_string())?;
        let svg = match design {
            Some(svg) => svg,
            None => kerf_trace::trace(&image, &settings)?.to_svg(width_mm, height_mm),
        };
        std::fs::write(&path, svg)
            .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Generated {
    /// Statistics and preview paths; `text` is emptied and held in [`AppState`].
    #[serde(flatten)]
    program: Program,
    /// The first lines of the program.
    head: String,
}

fn check_size(width_mm: f64, height_mm: f64) -> Result<(), String> {
    if width_mm.is_finite() && width_mm > 0.0 && height_mm.is_finite() && height_mm > 0.0 {
        Ok(())
    } else {
        Err("The artwork's width and height must be greater than zero.".into())
    }
}

/// Where a relief takes its heights from.
pub(crate) struct DepthSource {
    depth: Arc<DynamicImage>,
    /// How much of the picture's own detail is laid over the depth, 0 to 1.
    detail: f32,
    /// Depths below this share of the range become a flat background, 0 to 1.
    floor: f32,
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
pub(crate) fn heightmap(
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
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DepthOptions {
    /// Carve from the depth map rather than from brightness.
    pub(crate) enabled: bool,
    detail: f32,
    floor: f32,
}

/// The loaded picture's depth map, with how it is to be used.
pub(crate) fn depth_source(
    state: &AppState,
    options: &DepthOptions,
) -> Result<DepthSource, String> {
    let guard = state.image.lock().unwrap();
    let map = guard.as_ref().and_then(|loaded| loaded.depth.clone());
    Ok(DepthSource {
        depth: map.ok_or("Estimate the depth first, or use brightness.")?,
        detail: options.detail,
        floor: options.floor,
    })
}

/// Text and drawings added to the material beside the main artwork.
#[derive(serde::Deserialize)]
struct Added {
    /// An SVG the size of the bed, with each addition drawn where it sits.
    svg: String,
    operations: Vec<kerf_gcode::Operation>,
}

/// Traces at full resolution and generates G-code for `job`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn generate_gcode(
    settings: TraceSettings,
    width_mm: f64,
    height_mm: f64,
    job: Job,
    depth: DepthOptions,
    // Text and drawings added beside the main artwork, cut in the same job.
    added: Option<Added>,
    // The space everything takes up on the bed (x, y, width, height), which
    // cutouts go round.
    cutout_around: Option<[f64; 4]>,
    state: State<'_, AppState>,
) -> Result<Generated, String> {
    check_size(width_mm, height_mm)?;
    *state.gcode.lock().unwrap() = None;
    let image = state.image(false)?;
    let is_relief = job
        .operations
        .iter()
        .any(|o| matches!(o.kind, OperationKind::CncRelief { .. }));
    let depth = if depth.enabled && is_relief {
        Some(depth_source(&state, &depth)?)
    } else {
        None
    };
    let design = state
        .design
        .lock()
        .unwrap()
        .as_ref()
        .map(|design| design.to_svg(width_mm, height_mm));
    let (mut program, text) = blocking(move || {
        let carves_relief = job.operations.iter().any(|o| {
            matches!(
                o.kind,
                OperationKind::CncRelief { .. } | OperationKind::LaserImage { .. }
            )
        });
        if design.is_some() && carves_relief {
            return Err("This design is made of lines to cut, so it cannot be engraved as a picture or carved as a relief. Use an operation that follows the lines.".into());
        }
        let svg = match design {
            Some(svg) => svg,
            None => kerf_trace::trace(&image, &settings)?.to_svg(width_mm, height_mm),
        };
        let artwork = Artwork::from_svg(&svg, FLATTEN_TOLERANCE_MM)?;
        let heightmap = carves_relief
            .then(|| heightmap(&image, depth.as_ref(), &settings, width_mm, height_mm));
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
        Ok((program, text))
    })
    .await?;
    let head = text
        .lines()
        .take(GCODE_HEAD_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    program.text.clear();
    *state.gcode.lock().unwrap() = Some(text);
    Ok(Generated { program, head })
}

/// Writes the last generated program where the user chooses.
/// Returns the saved path, or `None` if the dialog was cancelled.
#[tauri::command]
async fn export_gcode(
    app: AppHandle,
    suggested_name: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let text = state
        .gcode
        .lock()
        .unwrap()
        .clone()
        .ok_or("Generate the G-code first.")?;
    blocking(move || {
        let Some(target) = app
            .dialog()
            .file()
            .add_filter("G-code", &["gcode", "nc"])
            .set_file_name(suggested_name)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = target.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path, text)
            .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(machine::MachineLink::default())
        .invoke_handler(tauri::generate_handler![
            app_info,
            load_image,
            clear_image,
            crop_image,
            reset_crop,
            set_depth_map,
            trace_image,
            export_svg,
            generate_gcode,
            export_gcode,
            machine::list_ports,
            machine::connect_machine,
            machine::disconnect_machine,
            machine::machine_action,
            web::fetch_page_text,
            gallery::save_project,
            gallery::list_projects,
            gallery::open_project,
            gallery::project_picture,
            gallery::delete_project,
            assistant::set_api_key,
            assistant::has_api_key,
            assistant::delete_api_key,
            assistant::assistant_send,
            assistant::assistant_models,
            pictures::generate_picture,
            pictures::search_pictures,
            pictures::fetch_picture,
            design::set_design,
            design::make_box,
            design::make_layers,
            drawing::read_svg,
            drawing::open_drawing,
            drawing::inkscape_found,
            drawing::list_fonts,
            drawing::make_text
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kerf");
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
