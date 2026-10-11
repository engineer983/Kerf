//! Commands for drawings that arrive as lines: SVG documents and words set
//! in a font, which `kerf_core::drawing` reads, and, with Inkscape's help
//! where it is installed, PDF, AI and EPS files.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kerf_core::drawing::{drawn, text_made, TextMade, TextSpec};
use kerf_make::Sheet;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::blocking;

/// Inkscape is given this long to convert a file before it is stopped.
const INKSCAPE_TIMEOUT: Duration = Duration::from_secs(90);
/// File types Inkscape can turn into SVG.
const INKSCAPE_TYPES: [&str; 3] = ["pdf", "ai", "eps"];

/// Reads an SVG document the interface was given, by drop or paste.
#[tauri::command]
pub async fn read_svg(svg: String, name: String) -> Result<Sheet, String> {
    blocking(move || drawn(&svg, &name)).await
}

#[tauri::command]
pub async fn make_text(spec: TextSpec) -> Result<TextMade, String> {
    blocking(move || text_made(&spec)).await
}

/// The font families installed on this computer.
#[tauri::command]
pub async fn list_fonts() -> Result<Vec<String>, String> {
    blocking(|| Ok(kerf_gcode::font_families())).await
}
/// Where Inkscape's command-line program is, if it is installed.
fn inkscape() -> Option<PathBuf> {
    // On Windows the `.com` program is the one that waits and reports errors.
    let names: &[&str] = if cfg!(windows) {
        &["inkscape.com", "inkscape.exe"]
    } else {
        &["inkscape"]
    };
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(programs) = std::env::var_os(variable) {
            folders.push(Path::new(&programs).join("Inkscape").join("bin"));
        }
    }
    folders.push("/Applications/Inkscape.app/Contents/MacOS".into());
    folders
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| candidate.is_file())
}

/// Whether Inkscape is installed, which decides the file types on offer.
#[tauri::command]
pub async fn inkscape_found() -> Result<bool, String> {
    blocking(|| Ok(inkscape().is_some())).await
}

/// Has Inkscape turn `file` into a plain SVG, with text made into outlines,
/// and returns that SVG. Inkscape runs as a separate program and is stopped
/// if it takes too long.
fn convert_with_inkscape(program: &Path, file: &Path) -> Result<String, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_nanos())
        .unwrap_or(0);
    let output = std::env::temp_dir().join(format!("kerf-{}-{unique}.svg", std::process::id()));
    let mut command = Command::new(program);
    command
        .arg(file)
        .arg("--pages=1")
        .arg("--export-type=svg")
        .arg("--export-plain-svg")
        .arg("--export-text-to-path")
        .arg("--export-area-drawing")
        .arg(format!("--export-filename={}", output.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashes up while it runs.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Inkscape could not be started: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > INKSCAPE_TIMEOUT => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&output);
                return Err(
                    "Inkscape took too long to convert the file, so it was stopped.".into(),
                );
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("Inkscape stopped unexpectedly: {e}")),
        }
    }
    let svg = std::fs::read_to_string(&output);
    let _ = std::fs::remove_file(&output);
    svg.map_err(|_| "Inkscape could not convert that file.".into())
}

/// The sheet in the drawing at `path`: read directly if it is an SVG, and
/// through Inkscape if it is another type or the SVG has nothing readable.
fn open(path: &Path) -> Result<Sheet, String> {
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "drawing".into());
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "svg" {
        let svg = std::fs::read_to_string(path)
            .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        return match (drawn(&svg, &name), inkscape()) {
            (Ok(sheet), _) => Ok(sheet),
            // Inkscape understands features of SVG that this app does not.
            (Err(_), Some(program)) => drawn(&convert_with_inkscape(&program, path)?, &name),
            (Err(problem), None) => Err(problem),
        };
    }
    if !INKSCAPE_TYPES.contains(&extension.as_str()) {
        return Err(
            "That is not a drawing this app can open. Use an SVG, PDF, AI or EPS file.".into(),
        );
    }
    let program = inkscape().ok_or(
        "Opening PDF, AI and EPS files needs Inkscape, a free drawing program, to be installed. Save the drawing as an SVG instead, or install Inkscape from inkscape.org.",
    )?;
    drawn(&convert_with_inkscape(&program, path)?, &name)
}

#[derive(Serialize)]
pub struct Opened {
    name: String,
    sheet: Sheet,
}

/// Asks for a drawing file and reads it. Returns `None` if the dialog was cancelled.
#[tauri::command]
pub async fn open_drawing(app: AppHandle) -> Result<Option<Opened>, String> {
    blocking(move || {
        let mut types = vec!["svg"];
        if inkscape().is_some() {
            types.extend(INKSCAPE_TYPES);
        }
        let Some(chosen) = app
            .dialog()
            .file()
            .add_filter("Drawings", &types)
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = chosen.into_path().map_err(|e| e.to_string())?;
        let sheet = open(&path)?;
        Ok(Some(Opened {
            name: sheet.parts[0].clone(),
            sheet,
        }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_that_are_not_drawings_are_refused() {
        assert!(open(Path::new("holiday.docx"))
            .unwrap_err()
            .contains("not a drawing"));
        assert!(open(Path::new("missing-file.svg"))
            .unwrap_err()
            .contains("Could not read"));
    }

    /// Runs only where Inkscape is installed: a PDF made by Inkscape itself
    /// from an SVG comes back as the same drawing.
    #[test]
    fn inkscape_converts_a_pdf_where_it_is_installed() {
        let Some(program) = inkscape() else {
            return;
        };
        let folder = std::env::temp_dir().join(format!("kerf-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (svg, pdf) = (folder.join("plate.svg"), folder.join("plate.pdf"));
        std::fs::write(
            &svg,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="40mm" viewBox="0 0 80 40"><rect x="10" y="10" width="60" height="20" fill="none" stroke="#ff0000" stroke-width="0.2"/></svg>"##,
        )
        .unwrap();
        let made = Command::new(&program)
            .arg(&svg)
            .arg(format!("--export-filename={}", pdf.display()))
            .status()
            .unwrap();
        assert!(made.success() && pdf.is_file());

        let sheet = open(&pdf).unwrap();
        // The rectangle is 60 x 20 mm, give or take its line's own width.
        assert!((sheet.width_mm - 60.0).abs() < 0.5, "{}", sheet.width_mm);
        assert!((sheet.height_mm - 20.0).abs() < 0.5, "{}", sheet.height_mm);
        assert!(sheet.svg.contains("#ff0000"));
        assert_eq!(sheet.parts, ["plate"]);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
