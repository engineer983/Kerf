mod machine;
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

struct LoadedImage {
    /// The image as imported, kept so that a crop can be undone.
    original: Arc<DynamicImage>,
    /// The image being worked on: the original, or a crop of it.
    full: Arc<DynamicImage>,
    preview: Arc<DynamicImage>,
    /// Estimated depth for `full` (white is near), if the user has made one.
    depth: Option<Arc<DynamicImage>>,
}

impl LoadedImage {
    fn new(original: Arc<DynamicImage>, full: DynamicImage) -> Self {
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
const FLATTEN_TOLERANCE_MM: f64 = 0.05;
/// Longest side, in pixels, of the height map used for relief carving. A
/// router bit cannot resolve more, and the carving time grows with it.
const RELIEF_MAX_PX: u32 = 1500;
/// Lines of G-code returned to the UI as a preview.
const GCODE_HEAD_LINES: usize = 60;

#[derive(Default)]
pub(crate) struct AppState {
    image: Mutex<Option<LoadedImage>>,
    /// Text of the most recently generated program, kept here for export and sending.
    pub(crate) gcode: Mutex<Option<String>>,
}

impl AppState {
    fn image(&self, preview: bool) -> Result<Arc<DynamicImage>, String> {
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
    Ok(())
}

#[tauri::command]
fn clear_image(state: State<'_, AppState>) {
    *state.image.lock().unwrap() = None;
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
        let svg = kerf_trace::trace(&image, &settings)?.to_svg(width_mm, height_mm);
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
struct DepthSource {
    depth: Arc<DynamicImage>,
    /// How much of the photo's fine texture is laid over the depth, 0 to 1.
    detail: f32,
}

/// The image as heights: white is 1.0, black is 0.0, after the trace
/// settings' brightness, contrast and blur. With a depth source the heights
/// come from the depth map instead, with the photo's fine texture added.
fn heightmap(
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
            // The photo minus a blurred copy of itself is its fine texture:
            // hair, fabric and edges, without the broad light and dark areas.
            let photo = image.to_luma8();
            let broad = image::imageops::blur(&photo, width.max(height) as f32 / 120.0);
            let strength = source.detail.clamp(0.0, 1.0) * 0.35;
            shape
                .as_raw()
                .iter()
                .zip(photo.as_raw().iter().zip(broad.as_raw()))
                .map(|(&z, (&fine, &smooth))| {
                    let texture = (fine as f32 - smooth as f32) / 255.0;
                    (z as f32 / 255.0 + texture * strength).clamp(0.0, 1.0)
                })
                .collect()
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

/// Traces at full resolution and generates G-code for `job`.
#[tauri::command]
async fn generate_gcode(
    settings: TraceSettings,
    width_mm: f64,
    height_mm: f64,
    job: Job,
    // Carve reliefs from the estimated depth map rather than from brightness.
    use_depth: bool,
    detail: f32,
    state: State<'_, AppState>,
) -> Result<Generated, String> {
    check_size(width_mm, height_mm)?;
    *state.gcode.lock().unwrap() = None;
    let image = state.image(false)?;
    let is_relief = job
        .operations
        .iter()
        .any(|o| matches!(o.kind, OperationKind::CncRelief { .. }));
    let depth = if use_depth && is_relief {
        let guard = state.image.lock().unwrap();
        let depth = guard.as_ref().and_then(|loaded| loaded.depth.clone());
        Some(DepthSource {
            depth: depth.ok_or("Estimate the depth first, or carve from brightness.")?,
            detail,
        })
    } else {
        None
    };
    let (mut program, text) = blocking(move || {
        let svg = kerf_trace::trace(&image, &settings)?.to_svg(width_mm, height_mm);
        let artwork = Artwork::from_svg(&svg, FLATTEN_TOLERANCE_MM)?;
        let carves_relief = job.operations.iter().any(|o| {
            matches!(
                o.kind,
                OperationKind::CncRelief { .. } | OperationKind::LaserImage { .. }
            )
        });
        let heightmap = carves_relief
            .then(|| heightmap(&image, depth.as_ref(), &settings, width_mm, height_mm));
        let mut program = kerf_gcode::generate_with(&artwork, heightmap.as_ref(), &job)?;
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
            web::fetch_page_text
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kerf");
}
