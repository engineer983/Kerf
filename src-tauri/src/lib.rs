use std::sync::{Arc, Mutex};

use kerf_gcode::{Artwork, Job, Program};
use kerf_trace::image::{self, DynamicImage};
use kerf_trace::{TraceSettings, Traced};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// Longest side, in pixels, of the copy used for fast preview traces.
const PREVIEW_MAX_PX: u32 = 1024;

struct LoadedImage {
    full: Arc<DynamicImage>,
    preview: Arc<DynamicImage>,
}

/// Curves are flattened to within this distance, in millimetres, for G-code.
const FLATTEN_TOLERANCE_MM: f64 = 0.05;
/// Lines of G-code returned to the UI as a preview.
const GCODE_HEAD_LINES: usize = 60;

#[derive(Default)]
struct AppState {
    image: Mutex<Option<LoadedImage>>,
    /// Text of the most recently generated program, kept here for export.
    gcode: Mutex<Option<String>>,
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

async fn blocking<T: Send + 'static>(
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
        let preview = if full.width().max(full.height()) > PREVIEW_MAX_PX {
            full.thumbnail(PREVIEW_MAX_PX, PREVIEW_MAX_PX)
        } else {
            full.clone()
        };
        Ok(LoadedImage {
            full: Arc::new(full),
            preview: Arc::new(preview),
        })
    })
    .await?;
    *state.image.lock().unwrap() = Some(loaded);
    Ok(())
}

#[tauri::command]
fn clear_image(state: State<'_, AppState>) {
    *state.image.lock().unwrap() = None;
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
    suggested_name: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    if !(width_mm.is_finite() && width_mm > 0.0) {
        return Err("The output width must be greater than zero.".into());
    }
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
        let svg = kerf_trace::trace(&image, &settings)?.to_svg(width_mm);
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

/// Traces at full resolution and generates G-code for `job`.
#[tauri::command]
async fn generate_gcode(
    settings: TraceSettings,
    width_mm: f64,
    job: Job,
    state: State<'_, AppState>,
) -> Result<Generated, String> {
    if !(width_mm.is_finite() && width_mm > 0.0) {
        return Err("The output width must be greater than zero.".into());
    }
    *state.gcode.lock().unwrap() = None;
    let image = state.image(false)?;
    let (mut program, text) = blocking(move || {
        let svg = kerf_trace::trace(&image, &settings)?.to_svg(width_mm);
        let artwork = Artwork::from_svg(&svg, FLATTEN_TOLERANCE_MM)?;
        let mut program = kerf_gcode::generate(&artwork, &job)?;
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
        .invoke_handler(tauri::generate_handler![
            app_info,
            load_image,
            clear_image,
            trace_image,
            export_svg,
            generate_gcode,
            export_gcode
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kerf");
}
