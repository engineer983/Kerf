//! The camera over the bed. The window itself opens the camera and takes the
//! still; this only keeps the latest one and redraws it to the bed. All the
//! working out is in `kerf_core::camera`, shared with the browser version.
//!
//! Nothing here reaches the machine.

use kerf_core::camera::{self, BedPictureRequest, BedSize, CameraFit, CameraStill, FitCheck};
use kerf_core::material::{self, BedRect, FindMaterialRequest, FoundMaterial};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::State;

use crate::{blocking, AppState};

#[derive(Serialize)]
pub(crate) struct StillSize {
    width: u32,
    height: u32,
}

/// Takes the still (a JPEG, usually) as the raw request body and keeps it.
#[tauri::command]
pub(crate) async fn set_camera_still(
    request: Request<'_>,
    state: State<'_, AppState>,
) -> Result<StillSize, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected the camera's picture as raw bytes.".into());
    };
    let bytes = bytes.clone();
    let still = blocking(move || CameraStill::load(&bytes)).await?;
    let size = StillSize {
        width: still.image.width(),
        height: still.image.height(),
    };
    *state.camera_still.lock().unwrap() = Some(still.image);
    Ok(size)
}

/// Checks a fit can be used, and whether it looks mirrored.
#[tauri::command]
pub(crate) fn check_camera_fit(fit: CameraFit) -> Result<FitCheck, String> {
    camera::check_fit(&fit)
}

/// The latest still redrawn as the bed seen from above, as a PNG file.
#[tauri::command]
pub(crate) async fn camera_bed_picture(
    fit: CameraFit,
    bed: BedSize,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let still = state
        .camera_still
        .lock()
        .unwrap()
        .clone()
        .ok_or("Take a picture with the camera first.")?;
    let request = BedPictureRequest { fit, bed };
    blocking(move || camera::camera_bed_picture_png(&still, &request))
        .await
        .map(Response::new)
}

/// Looks for the piece of material in the latest still. Only proposes an
/// outline: the interface shows it, and nothing changes until the person
/// says to use it.
#[tauri::command]
pub(crate) async fn find_material(
    fit: CameraFit,
    bed: BedSize,
    seed: Option<(f64, f64)>,
    stock: Option<BedRect>,
    thickness: f64,
    state: State<'_, AppState>,
) -> Result<FoundMaterial, String> {
    let still = state
        .camera_still
        .lock()
        .unwrap()
        .clone()
        .ok_or("Take a picture with the camera first.")?;
    let request = FindMaterialRequest {
        fit,
        bed,
        seed,
        stock,
        thickness,
    };
    blocking(move || material::find_material_request(&still, &request)).await
}

/// WebKitGTK refuses the camera unless the app turns media on and answers
/// its permission requests. Allows only video, and only for Kerf's own page.
#[cfg(target_os = "linux")]
pub(crate) fn allow_camera_on_linux(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::Manager;
    use webkit2gtk::glib::prelude::Cast;
    use webkit2gtk::{
        PermissionRequestExt, SettingsExt, UserMediaPermissionRequest,
        UserMediaPermissionRequestExt, WebViewExt,
    };

    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    window.with_webview(|webview| {
        let view = webview.inner();
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_media_stream(true);
        }
        view.connect_permission_request(|view, request| {
            // The signal doesn't say who is asking, but this window only ever
            // shows Kerf's own page; check that it still does.
            let own_page = view.uri().is_some_and(|uri| is_own_page(&uri));
            match request.downcast_ref::<UserMediaPermissionRequest>() {
                Some(media)
                    if own_page && media.is_for_video_device() && !media.is_for_audio_device() =>
                {
                    request.allow()
                }
                _ => request.deny(),
            }
            // Answered here, so the default handler doesn't run as well.
            true
        });
    })?;
    Ok(())
}

/// Whether an address is Kerf's own page: the bundled app, or the
/// development server when running from source.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_own_page(uri: &str) -> bool {
    let at = |origin: &str| {
        uri.strip_prefix(origin)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    at("tauri://localhost") || (cfg!(debug_assertions) && at("http://localhost:1420"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kerfs_own_page_counts() {
        assert!(is_own_page("tauri://localhost/index.html"));
        assert!(is_own_page("tauri://localhost"));
        assert!(!is_own_page("https://example.com/"));
        assert!(!is_own_page("tauri://localhost.example.com/"));
        assert!(!is_own_page(""));
    }
}
