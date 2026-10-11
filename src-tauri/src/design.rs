//! Designs made from measurements instead of traced from a picture: boxes
//! and layered art. A design is exact lines in millimetres; while one is set
//! it takes the place of the traced picture everywhere, so it is shown, cut
//! and exported exactly as it was drawn.

use kerf_gcode::Artwork;
use kerf_make::{BoxSpec, LayerSpec, Part, Sheet};
use kerf_trace::image::GrayImage;
use kerf_trace::{TraceSettings, Traced};
use tauri::State;

use crate::{blocking, AppState, DepthOptions, FLATTEN_TOLERANCE_MM};

/// Pixels per millimetre of the space a design is shown in. The interface
/// draws traced paths in a whole-number pixel space; this is fine enough that
/// nothing visible is lost.
const SHOWN_PX_PER_MM: f64 = 20.0;
/// Clear space left between parts on a sheet, beyond the width of the cut.
const PART_GAP_MM: f64 = 3.0;

pub(crate) struct Design {
    /// The elements inside the design's `<svg>`, drawn in millimetres.
    body: String,
    width_mm: f64,
    height_mm: f64,
}

impl Design {
    fn new(svg: &str) -> Result<Self, String> {
        // Designs arrive already reduced to plain paths, so no fonts are needed.
        let artwork = Artwork::from_svg(svg, FLATTEN_TOLERANCE_MM)?;
        if artwork.shapes.is_empty() {
            return Err("The design has no lines to cut.".into());
        }
        let start = svg
            .find("<svg")
            .and_then(|open| svg[open..].find('>').map(|close| open + close + 1));
        let body = match (start, svg.rfind("</svg>")) {
            (Some(start), Some(end)) if start <= end => svg[start..end].trim().to_owned(),
            _ => return Err("The design is not a complete SVG.".into()),
        };
        Ok(Self {
            body,
            width_mm: artwork.width,
            height_mm: artwork.height,
        })
    }

    /// The design as the tracer would report it, for showing on the canvas.
    pub(crate) fn traced(&self) -> Traced {
        let width = (self.width_mm * SHOWN_PX_PER_MM).round().max(1.0);
        let height = (self.height_mm * SHOWN_PX_PER_MM).round().max(1.0);
        Traced {
            width: width as u32,
            height: height as u32,
            path_count: self.body.matches("<path").count(),
            body: format!(
                "<g transform=\"scale({:.6} {:.6})\">{}</g>",
                width / self.width_mm,
                height / self.height_mm,
                self.body
            ),
        }
    }

    /// A complete SVG document of the design at the given size.
    pub(crate) fn to_svg(&self, width_mm: f64, height_mm: f64) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width_mm:.3}mm\" height=\"{height_mm:.3}mm\" viewBox=\"0 0 {:.3} {:.3}\" preserveAspectRatio=\"none\">\n\
             {}\n</svg>\n",
            self.width_mm, self.height_mm, self.body
        )
    }
}

/// How wide to let rows of parts run: wide enough to make a compact, roughly
/// square block of them rather than one long strip, which wastes less of a
/// sheet and suits the pieces of material people have. Never wider than the
/// machine allows, nor narrower than the widest part.
fn row_width(parts: &[Part], gap: f64, limit: f64) -> f64 {
    let area: f64 = parts
        .iter()
        .map(|p| (p.width + gap) * (p.height + gap))
        .sum();
    let widest = parts.iter().map(|p| p.width).fold(0.0, f64::max);
    (area.sqrt() * 1.3).max(widest).min(limit.max(widest))
}

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
/// machine can reach.
#[tauri::command]
pub async fn make_box(
    spec: BoxSpec,
    sheet_width: f64,
    sheet_height: f64,
) -> Result<Vec<Sheet>, String> {
    blocking(move || {
        let parts = kerf_make::box_parts(&spec)?;
        let gap = PART_GAP_MM + spec.cut_width;
        kerf_make::lay_out(
            &parts,
            row_width(&parts, gap, sheet_width),
            sheet_height,
            gap,
        )
    })
    .await
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
        Some(crate::depth_source(&state, &depth)?)
    } else {
        None
    };
    blocking(move || {
        // The size in millimetres does not matter here: only the heights are used.
        let map = crate::heightmap(&image, depth.as_ref(), &settings, 1.0, 1.0);
        let heights = GrayImage::from_fn(map.width as u32, map.height as u32, |x, y| {
            let height = map.data[y as usize * map.width + x as usize];
            let height = if light_is_high { height } else { 1.0 - height };
            kerf_trace::image::Luma([(height.clamp(0.0, 1.0) * 255.0).round() as u8])
        });
        let parts = kerf_make::layer_parts(&heights, &spec)?;
        let width = row_width(&parts, PART_GAP_MM, sheet_width);
        kerf_make::lay_out(&parts, width, sheet_height, PART_GAP_MM)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::{drawn, text_sheet, Align, TextSpec};

    #[test]
    fn a_drawing_is_cropped_to_what_is_drawn_and_keeps_its_colours() {
        // A page far larger than the two lines on it.
        let sheet = drawn(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="200mm" viewBox="0 0 300 200">
                <rect x="100" y="50" width="40" height="20" fill="#ff0000"/>
                <path d="M110 90H180" stroke="#0000ff" fill="none"/></svg>"##,
            "sign",
        )
        .unwrap();
        assert!((sheet.width_mm - 80.0).abs() < 0.01 && (sheet.height_mm - 40.0).abs() < 0.01);
        assert_eq!(sheet.parts, ["sign"]);
        let design = Design::new(&sheet.svg).unwrap();
        let again = Artwork::from_svg(&design.to_svg(80.0, 40.0), FLATTEN_TOLERANCE_MM).unwrap();
        assert_eq!(again.colors(), ["#ff0000", "#0000ff"]);
        assert!(again.shapes[0].polylines[0].closed);
        assert!(!again.shapes[1].polylines[0].closed);
        // The rectangle now starts at the corner.
        assert!(again.shapes[0].polylines[0]
            .points
            .iter()
            .any(|p| p[0].abs() < 0.01 && p[1].abs() < 0.01));

        assert!(drawn("not svg", "x").is_err());
        assert!(drawn(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"/>"#,
            "x"
        )
        .unwrap_err()
        .contains("nothing"));
    }

    #[test]
    fn text_is_drawn_at_the_size_asked_for() {
        let Some(font) = kerf_gcode::font_families().into_iter().find(|name| {
            ["Arial", "DejaVu Sans", "Liberation Sans", "Helvetica"].contains(&name.as_str())
        }) else {
            // No ordinary font to measure with on this machine.
            return;
        };
        let spec = |text: &str, size_mm: f64| TextSpec {
            text: text.into(),
            font: font.clone(),
            size_mm,
            bold: false,
            italic: false,
            align: Align::Center,
            bend: 0.0,
        };
        let one = text_sheet(&spec("HELLO", 20.0)).unwrap();
        // Capital letters stand about seven tenths of the size.
        assert!((12.0..17.0).contains(&one.height_mm), "{}", one.height_mm);
        assert!(one.width_mm > 50.0);
        let big = text_sheet(&spec("HELLO", 40.0)).unwrap();
        assert!((big.width_mm / one.width_mm - 2.0).abs() < 0.05);
        let two = text_sheet(&spec("HELLO\nA <&> B", 20.0)).unwrap();
        assert!(two.height_mm > one.height_mm + 15.0);
        assert!(Design::new(&two.svg).is_ok());
        // The hole in a letter stays part of that letter, so that engraving
        // the word solid leaves the hole clear.
        let o = text_sheet(&spec("O", 20.0)).unwrap();
        let read = Artwork::from_svg(&o.svg, FLATTEN_TOLERANCE_MM).unwrap();
        assert_eq!(read.shapes.len(), 1);
        assert_eq!(read.shapes[0].polylines.len(), 2);
        // Bent half way round a circle, the words span the circle's width,
        // which is 2/pi of their straight length, and stand much taller.
        let straight = text_sheet(&spec("HELLO WORLD", 20.0)).unwrap();
        let bent = |bend: f64, text: &str| {
            text_sheet(&TextSpec {
                bend,
                ..spec(text, 20.0)
            })
            .unwrap()
        };
        let arch = bent(180.0, "HELLO WORLD");
        let share = arch.width_mm / straight.width_mm;
        assert!((0.6..0.95).contains(&share), "{share}");
        assert!(arch.height_mm > straight.height_mm * 2.0);
        let under = bent(-180.0, "HELLO WORLD");
        // Curved under, the letters stand inside the circle, not outside it,
        // so the same words come out narrower by twice their height.
        let narrower = arch.width_mm - under.width_mm;
        assert!((15.0..45.0).contains(&narrower), "{narrower}");
        assert!(under.height_mm > straight.height_mm * 2.0);
        // Two lines still fit, one inside the other, however tight the curve:
        // the circle opens out until the second line has room.
        assert!(bent(120.0, "HELLO WORLD\nAGAIN").height_mm > 20.0);
        let tight_one = bent(300.0, "GARDEN");
        let tight_two = bent(300.0, "GARDEN\nRULES");
        assert!(tight_two.width_mm > tight_one.width_mm + 10.0);
        assert!(text_sheet(&spec("   ", 20.0)).is_err());
        assert!(text_sheet(&spec("HELLO", 0.0)).is_err());
    }

    #[test]
    fn a_design_is_shown_and_exported_at_its_own_size() {
        let spec = BoxSpec {
            width: 100.0,
            depth: 80.0,
            height: 50.0,
            thickness: 3.0,
            finger_width: 0.0,
            lid: false,
            cut_width: 0.15,
            dogbones: false,
        };
        let parts = kerf_make::box_parts(&spec).unwrap();
        let sheets = kerf_make::lay_out(&parts, 400.0, 400.0, 3.0).unwrap();
        assert_eq!(sheets.len(), 1);
        let design = Design::new(&sheets[0].svg).unwrap();
        assert!((design.width_mm - sheets[0].width_mm).abs() < 0.01);

        let traced = design.traced();
        assert_eq!(traced.path_count, 5);
        assert_eq!(
            traced.width,
            (sheets[0].width_mm * SHOWN_PX_PER_MM).round() as u32
        );

        // Read back as artwork, every panel is there at its true size.
        let again = Artwork::from_svg(
            &design.to_svg(design.width_mm, design.height_mm),
            FLATTEN_TOLERANCE_MM,
        )
        .unwrap();
        assert_eq!(again.shapes.len(), 5);
        assert!((again.width - design.width_mm).abs() < 0.01);
        let widest = again
            .shapes
            .iter()
            .flat_map(|s| &s.polylines)
            .flat_map(|p| &p.points)
            .map(|p| p[0])
            .fold(0.0, f64::max);
        assert!((widest - design.width_mm).abs() < 0.01, "{widest}");
        assert_eq!(again.colors(), [kerf_make::CUT_COLOR]);
    }

    #[test]
    fn parts_are_packed_into_a_block_rather_than_a_strip() {
        let part = |width: f64, height: f64| Part {
            name: String::new(),
            width,
            height,
            markup: String::new(),
        };
        // Five 120 x 160 layers: three to a row, not five in a line.
        let layers = vec![part(120.0, 160.0); 5];
        let width = row_width(&layers, 3.0, 730.0);
        assert!((366.0..489.0).contains(&width), "{width}");
        // A small machine still limits the row, and one wide part still fits.
        assert_eq!(row_width(&layers, 3.0, 300.0), 300.0);
        assert_eq!(row_width(&[part(500.0, 20.0)], 3.0, 730.0), 500.0);
    }

    #[test]
    fn what_is_not_a_design_is_refused() {
        assert!(Design::new("not svg").is_err());
        assert!(Design::new(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10mm\" height=\"10mm\"></svg>"
        )
        .is_err());
    }
}
