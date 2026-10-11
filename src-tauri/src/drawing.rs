//! Drawings that arrive as lines rather than pixels: SVG files, text set in a
//! font, and, with Inkscape's help where it is installed, PDF, AI and EPS.
//!
//! Whatever the source, it ends up as one [`Sheet`]: plain paths in
//! millimetres, cropped to what is drawn, each in the colour it was given.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kerf_gcode::Artwork;
use kerf_make::Sheet;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::{blocking, FLATTEN_TOLERANCE_MM};

/// Colour given to lines that had none of their own.
const PLAIN_COLOR: &str = "#000000";
/// Inkscape is given this long to convert a file before it is stopped.
const INKSCAPE_TIMEOUT: Duration = Duration::from_secs(90);
/// File types Inkscape can turn into SVG.
const INKSCAPE_TYPES: [&str; 3] = ["pdf", "ai", "eps"];

/// `svg` as a sheet of plain paths, cropped to what is drawn. Text is turned
/// into the outlines of its letters using this computer's fonts.
pub(crate) fn drawn(svg: &str, name: &str) -> Result<Sheet, String> {
    let artwork = Artwork::from_svg_with_text(svg, FLATTEN_TOLERANCE_MM)?;
    let points = || {
        artwork
            .shapes
            .iter()
            .flat_map(|shape| &shape.polylines)
            .flat_map(|polyline| &polyline.points)
    };
    if points().next().is_none() {
        return Err("There is nothing drawn in it that can be cut or engraved. Pictures inside a drawing are left out; import those as images.".into());
    }
    let (left, top, right, bottom) = points().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(left, top, right, bottom), p| {
            (
                left.min(p[0]),
                top.min(p[1]),
                right.max(p[0]),
                bottom.max(p[1]),
            )
        },
    );
    // A single straight line still needs some size to be placed by.
    let (width, height) = ((right - left).max(0.1), (bottom - top).max(0.1));

    let mut body = String::new();
    for shape in &artwork.shapes {
        let color = shape.color.as_deref().unwrap_or(PLAIN_COLOR);
        // A shape's outlines stay together in one path: an outline inside
        // another is a hole in it, as in the letter O, and would be filled
        // solid if it were drawn as a shape of its own.
        body.push_str("<path d=\"");
        for polyline in &shape.polylines {
            for (i, p) in polyline.points.iter().enumerate() {
                body.push_str(&format!(
                    "{}{:.3} {:.3}",
                    if i == 0 { "M" } else { "L" },
                    p[0] - left,
                    p[1] - top
                ));
            }
            if polyline.closed {
                body.push('Z');
            }
        }
        body.push_str(&format!("\" stroke=\"{color}\"/>\n"));
    }
    Ok(Sheet {
        svg: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.3}mm\" height=\"{height:.3}mm\" viewBox=\"0 0 {width:.3} {height:.3}\">\n\
             <g fill=\"none\" stroke-width=\"0.2\" stroke-linejoin=\"round\">\n{body}</g>\n</svg>\n"
        ),
        width_mm: width,
        height_mm: height,
        parts: vec![name.to_owned()],
    })
}

/// Reads an SVG document the interface was given, by drop or paste.
#[tauri::command]
pub async fn read_svg(svg: String, name: String) -> Result<Sheet, String> {
    blocking(move || drawn(&svg, &name)).await
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSpec {
    pub(crate) text: String,
    /// The name of a font family installed on this computer.
    pub(crate) font: String,
    /// The font's size in millimetres. Capital letters stand about seven tenths of it.
    pub(crate) size_mm: f64,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) align: Align,
    /// How far the words bend round a circle, in degrees: zero is straight,
    /// more than zero arches them over the top, less than zero curves them
    /// under the bottom. 180 is half way round.
    #[serde(default)]
    pub(crate) bend: f64,
}

/// The most of a circle that words are bent round, in degrees.
const MOST_BEND: f64 = 350.0;

/// Lines of text are set this far apart, as a multiple of the font's size.
const LINE_SPACING: f64 = 1.25;

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The words in `spec`, as the outlines of their letters.
pub(crate) fn text_sheet(spec: &TextSpec) -> Result<Sheet, String> {
    let lines: Vec<&str> = spec.text.lines().map(str::trim_end).collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        return Err("Type the words to make first.".into());
    }
    if !(spec.size_mm.is_finite() && spec.size_mm > 0.0 && spec.size_mm <= 1000.0) {
        return Err("The size must be between 0 and 1000 mm.".into());
    }
    let first = lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(&"text");
    let name: String = first.trim().chars().take(40).collect();
    let no_shapes = |_| {
        format!(
            "The font {} has no shapes for those characters. Choose another font.",
            spec.font
        )
    };
    if spec.bend.abs() >= 1.0 {
        return drawn(&bent_text(spec, &lines)?, &name).map_err(no_shapes);
    }
    // The text is set in the middle of a page far larger than it could need,
    // one unit to the millimetre, then cropped to its own outline.
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
    let page = ((widest + 2.0) * spec.size_mm * 3.0)
        .max((lines.len() as f64 + 2.0) * spec.size_mm * LINE_SPACING * 2.0);
    let anchor = match spec.align {
        Align::Left => "start",
        Align::Center => "middle",
        Align::Right => "end",
    };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{page:.1}mm\" height=\"{page:.1}mm\" viewBox=\"0 0 {page:.1} {page:.1}\">\n"
    );
    for (i, line) in lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text xml:space=\"preserve\" x=\"{:.1}\" y=\"{:.2}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" text-anchor=\"{anchor}\" fill=\"{PLAIN_COLOR}\">{}</text>\n",
            page / 2.0,
            spec.size_mm * (2.0 + i as f64 * LINE_SPACING),
            escaped(&spec.font),
            spec.size_mm,
            if spec.bold { "bold" } else { "normal" },
            if spec.italic { "italic" } else { "normal" },
            escaped(line),
        ));
    }
    svg.push_str("</svg>\n");
    drawn(&svg, &name).map_err(no_shapes)
}

/// An SVG of the lines of `spec` bent round a circle. The widest line, set
/// straight, is measured first: the circle is the one on which that line
/// takes up the angle asked for.
fn bent_text(spec: &TextSpec, lines: &[&str]) -> Result<String, String> {
    let straight = |line: &str| {
        text_sheet(&TextSpec {
            text: line.to_owned(),
            bend: 0.0,
            ..spec.clone()
        })
        .map(|sheet| sheet.width_mm)
        .unwrap_or(0.0)
    };
    let widest = lines.iter().map(|line| straight(line)).fold(0.0, f64::max);
    if widest <= 0.0 {
        return Err(format!(
            "The font {} has no shapes for those characters. Choose another font.",
            spec.font
        ));
    }
    let over = spec.bend > 0.0;
    let angle = spec.bend.abs().min(MOST_BEND).to_radians();
    let spacing = spec.size_mm * LINE_SPACING;
    // The circle is opened out, if need be, until every line fits: under an
    // arch the later lines sit inside it, and need some circle left to sit on.
    let room = if over {
        (lines.len() - 1) as f64 * spacing + spec.size_mm
    } else {
        spec.size_mm
    };
    let radius = (widest / angle).max(room);
    // Later lines sit inside an arch, and outside a curve under the bottom,
    // so that they read downwards either way.
    let line_radius = |i: usize| radius + i as f64 * if over { -spacing } else { spacing };
    let reach = radius + lines.len() as f64 * spacing + spec.size_mm * 2.0;
    let (page, centre) = (reach * 2.0, reach);
    // Each line runs along most of a circle, centred on its top or bottom,
    // so that there is always room for the words to be centred on it.
    let half = (MOST_BEND / 2.0).to_radians();
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{page:.1}mm\" height=\"{page:.1}mm\" viewBox=\"0 0 {page:.1} {page:.1}\">\n<defs>\n"
    );
    let mut placed = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let r = line_radius(i);
        if r <= spec.size_mm * 0.25 {
            // The circle is too tight to take this many lines.
            break;
        }
        let (dx, dy) = (r * half.sin(), r * half.cos());
        let (y, sweep) = if over {
            (centre - dy, 1)
        } else {
            (centre + dy, 0)
        };
        svg.push_str(&format!(
            "<path id=\"line{i}\" d=\"M{:.3} {y:.3}A{r:.3} {r:.3} 0 1 {sweep} {:.3} {y:.3}\"/>\n",
            centre - dx,
            centre + dx,
        ));
        placed.push((i, *line));
    }
    svg.push_str("</defs>\n");
    for (i, line) in placed {
        svg.push_str(&format!(
            "<text xml:space=\"preserve\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" text-anchor=\"middle\" fill=\"{PLAIN_COLOR}\"><textPath xlink:href=\"#line{i}\" startOffset=\"50%\">{}</textPath></text>\n",
            escaped(&spec.font),
            spec.size_mm,
            if spec.bold { "bold" } else { "normal" },
            if spec.italic { "italic" } else { "normal" },
            escaped(line.trim()),
        ));
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

#[tauri::command]
pub async fn make_text(spec: TextSpec) -> Result<Sheet, String> {
    blocking(move || text_sheet(&spec)).await
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
