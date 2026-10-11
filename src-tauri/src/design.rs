//! Commands for designs made from measurements: boxes and layered art. The
//! work itself is in `kerf_core::design`.

use kerf_core::design::Design;
use kerf_core::DepthOptions;
use kerf_make::{BoxSpec, LayerSpec, Sheet};
use kerf_trace::TraceSettings;
use tauri::State;

use crate::{blocking, AppState};

/// Makes `svg` the design being worked on, or with `None` goes back to the
/// traced picture.
#[tauri::command]
pub fn set_design(svg: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let design = svg.as_deref().map(Design::new).transpose()?;
    *state.design.lock().unwrap() = design;
    *state.gcode.lock().unwrap() = None;
    Ok(())
}

/// The panels of a finger-jointed box, laid out on sheets no larger than the
/// material allows.
#[tauri::command]
pub async fn make_box(
    spec: BoxSpec,
    sheet_width: f64,
    sheet_height: f64,
) -> Result<Vec<Sheet>, String> {
    blocking(move || kerf_core::design::box_sheets(&spec, sheet_width, sheet_height)).await
}

/// The loaded picture sliced into layers to stack, laid out on sheets.
#[tauri::command]
pub async fn make_layers(
    spec: LayerSpec,
    settings: TraceSettings,
    light_is_high: bool,
    depth: DepthOptions,
    sheet_width: f64,
    sheet_height: f64,
    state: State<'_, AppState>,
) -> Result<Vec<Sheet>, String> {
    let image = state.image(false)?;
    let depth = if depth.enabled {
        Some(depth.source(state.depth_map())?)
    } else {
        None
    };
    blocking(move || {
        kerf_core::design::layer_sheets(
            &image,
            depth.as_ref(),
            &settings,
            light_is_high,
            &spec,
            sheet_width,
            sheet_height,
        )
    })
    .await
}
