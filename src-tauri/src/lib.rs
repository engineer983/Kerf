mod assistant;
mod design;
mod drawing;
mod gallery;
mod machine;
mod pictures;
mod web;

use std::sync::{Arc, Mutex};

use kerf_core::design::Design;
use kerf_core::{Added, DepthOptions, Generated, JobInput, LoadedImage};
use kerf_gcode::Job;
use kerf_trace::image::DynamicImage;
use kerf_trace::{TraceSettings, Traced};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
pub(crate) struct AppState {
    pub(crate) image: Mutex<Option<LoadedImage>>,
    /// A box or layered design, which takes the place of the traced picture.
    pub(crate) design: Mutex<Option<Design>>,
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

    /// The loaded picture's depth map, if one has been made.
    pub(crate) fn depth_map(&self) -> Option<Arc<DynamicImage>> {
        let guard = self.image.lock().unwrap();
        guard.as_ref().and_then(|loaded| loaded.depth.clone())
    }

    /// The design at the given size, where one takes the picture's place.
    fn design_svg(&self, width_mm: f64, height_mm: f64) -> Option<String> {
        let guard = self.design.lock().unwrap();
        guard
            .as_ref()
            .map(|design| design.to_svg(width_mm, height_mm))
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
    let loaded = blocking(move || LoadedImage::load(&bytes)).await?;
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
    guard
        .as_mut()
        .ok_or("No image is loaded.")?
        .crop(x, y, width, height)
}

/// Undoes any crop, going back to the image as imported.
#[tauri::command]
fn reset_crop(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.image.lock().unwrap();
    guard.as_mut().ok_or("No image is loaded.")?.reset_crop();
    Ok(())
}

/// Takes an encoded greyscale depth picture of the current image (white is
/// near) as the raw request body. An empty body removes the depth map.
#[tauri::command]
async fn set_depth_map(request: Request<'_>, state: State<'_, AppState>) -> Result<(), String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected the depth picture as raw bytes.".into());
    };
    let bytes = bytes.clone();
    let depth = blocking(move || kerf_core::decode_depth(&bytes)).await?;
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
    kerf_core::check_size(width_mm, height_mm)?;
    let image = state.image(false)?;
    let design = state.design_svg(width_mm, height_mm);
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
    *state.gcode.lock().unwrap() = None;
    let input = JobInput {
        image: state.image(false)?,
        design_svg: state.design_svg(width_mm, height_mm),
        depth_map: state.depth_map(),
        settings,
        width_mm,
        height_mm,
        job,
        depth,
        added,
        cutout_around,
    };
    let (generated, text) = blocking(move || kerf_core::generate(input)).await?;
    *state.gcode.lock().unwrap() = Some(text);
    Ok(generated)
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
