//! Saved projects: the picture, its settings and a thumbnail, kept in the
//! app's data folder so earlier work can be opened again.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use kerf_trace::image::{self, DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

use crate::{blocking, AppState, LoadedImage};

const IMAGE_FILE: &str = "image.png";
const DEPTH_FILE: &str = "depth.png";
const META_FILE: &str = "meta.json";
const SETTINGS_FILE: &str = "project.json";

/// What the gallery shows for a project.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    id: String,
    name: String,
    /// When it was last saved, as the UI formatted it (ISO 8601).
    saved_at: String,
    /// A small picture of the result, as a data URL.
    thumbnail: String,
}

fn projects_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not find the app's data folder: {e}"))?
        .join("projects");
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// The folder of one project. Ids come from the UI, so they are restricted to
/// characters that cannot name anything outside the projects folder.
fn project_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    let safe = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !safe {
        return Err("That is not a valid project id.".into());
    }
    Ok(projects_dir(app)?.join(id))
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|e| format!("Could not encode the picture: {e}"))?;
    Ok(bytes.into_inner())
}

/// Saves the picture being worked on, with `settings` (the UI's own record of
/// the project, which this side stores without reading).
#[tauri::command]
pub async fn save_project(
    app: AppHandle,
    meta: ProjectMeta,
    settings: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (picture, depth) = {
        let guard = state.image.lock().unwrap();
        let loaded = guard.as_ref().ok_or("There is no picture to save.")?;
        (loaded.full.clone(), loaded.depth.clone())
    };
    let dir = project_dir(&app, &meta.id)?;
    blocking(move || {
        let write = |name: &str, bytes: &[u8]| {
            fs::write(dir.join(name), bytes).map_err(|e| format!("Could not save {name}: {e}"))
        };
        fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        write(IMAGE_FILE, &encode_png(&picture)?)?;
        match depth {
            Some(depth) => write(DEPTH_FILE, &encode_png(&depth)?)?,
            // A depth map from an earlier save no longer applies.
            None => {
                let _ = fs::remove_file(dir.join(DEPTH_FILE));
            }
        }
        write(SETTINGS_FILE, settings.as_bytes())?;
        // Written last, so that a project only appears in the gallery once complete.
        let meta = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
        write(META_FILE, &meta)
    })
    .await
}

/// Saved projects, most recently saved first.
#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectMeta>, String> {
    let dir = projects_dir(&app)?;
    blocking(move || {
        let mut projects: Vec<ProjectMeta> = fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .filter_map(|entry| fs::read(entry.ok()?.path().join(META_FILE)).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();
        projects.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        Ok(projects)
    })
    .await
}

/// Makes a saved project's picture (and depth map) the one being worked on,
/// and returns its settings for the UI to apply.
#[tauri::command]
pub async fn open_project(
    app: AppHandle,
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let dir = project_dir(&app, &id)?;
    let (loaded, settings) = blocking(move || {
        let read = |name: &str| {
            fs::read(dir.join(name))
                .map_err(|e| format!("Could not open the project's {name}: {e}"))
        };
        let picture = image::load_from_memory(&read(IMAGE_FILE)?).map_err(|e| e.to_string())?;
        let mut loaded = LoadedImage::new(Arc::new(picture.clone()), picture);
        if let Ok(bytes) = fs::read(dir.join(DEPTH_FILE)) {
            loaded.depth = image::load_from_memory(&bytes).ok().map(Arc::new);
        }
        let settings = String::from_utf8(read(SETTINGS_FILE)?).map_err(|e| e.to_string())?;
        Ok((loaded, settings))
    })
    .await?;
    *state.image.lock().unwrap() = Some(loaded);
    // The interface sets a saved design again from the project's settings.
    *state.design.lock().unwrap() = None;
    *state.gcode.lock().unwrap() = None;
    Ok(settings)
}

/// One of a saved project's pictures, as PNG bytes: `image` or `depth`.
#[tauri::command]
pub async fn project_picture(
    app: AppHandle,
    id: String,
    which: String,
) -> Result<Response, String> {
    let file = match which.as_str() {
        "image" => IMAGE_FILE,
        "depth" => DEPTH_FILE,
        _ => return Err("Unknown picture.".into()),
    };
    let path = project_dir(&app, &id)?.join(file);
    blocking(move || fs::read(&path).map_err(|e| format!("Could not read {}: {e}", path.display())))
        .await
        .map(Response::new)
}

#[tauri::command]
pub async fn delete_project(app: AppHandle, id: String) -> Result<(), String> {
    let dir = project_dir(&app, &id)?;
    blocking(move || {
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| format!("Could not delete the project: {e}"))?;
        }
        Ok(())
    })
    .await
}
