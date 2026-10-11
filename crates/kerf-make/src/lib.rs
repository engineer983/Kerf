//! Designs that are made rather than traced: boxes with finger joints, and
//! pictures cut as a stack of layers.
//!
//! Each maker produces [`Part`]s, flat pieces drawn as SVG paths in
//! millimetres. [`lay_out`] then arranges the parts on as many sheets as they
//! need, and each [`Sheet`] is a complete SVG document that can be cut.
//!
//! Lines to cut through are drawn in [`CUT_COLOR`] and lines to mark lightly
//! in [`MARK_COLOR`], so that a job can give each its own settings.

mod boxes;
mod layers;

use serde::Serialize;

pub use boxes::{box_parts, BoxSpec};
pub use layers::{layer_parts, LayerSpec};

/// Lines that are cut right through the material.
pub const CUT_COLOR: &str = "#000000";
/// Lines that are only marked on the surface, as a guide for assembly.
pub const MARK_COLOR: &str = "#1e6fd9";

/// One flat piece, drawn within a box from the origin to its width and height.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub width: f64,
    pub height: f64,
    /// SVG elements in millimetres. Paths set their own stroke colour.
    pub markup: String,
}

/// Parts arranged to be cut from one piece of material.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    /// A complete SVG document, sized in millimetres.
    pub svg: String,
    pub width_mm: f64,
    pub height_mm: f64,
    /// The names of the parts on this sheet, in reading order.
    pub parts: Vec<String>,
}

/// Arranges `parts` in rows on sheets no larger than `max_width` by
/// `max_height`, `gap` millimetres apart. Parts keep their order, so that a
/// numbered set stays in sequence.
pub fn lay_out(
    parts: &[Part],
    max_width: f64,
    max_height: f64,
    gap: f64,
) -> Result<Vec<Sheet>, String> {
    if let Some(part) = parts
        .iter()
        .find(|p| p.width > max_width + 1e-6 || p.height > max_height + 1e-6)
    {
        return Err(format!(
            "The {} is {:.0} x {:.0} mm, which is larger than the {:.0} x {:.0} mm the machine can reach. Make it smaller.",
            part.name, part.width, part.height, max_width, max_height
        ));
    }

    struct Placed<'a> {
        part: &'a Part,
        x: f64,
        y: f64,
    }
    let mut sheets: Vec<Vec<Placed>> = vec![Vec::new()];
    // The pen: where the next part goes, and how tall the current row is.
    let (mut x, mut y, mut row_height) = (0.0_f64, 0.0_f64, 0.0_f64);
    for part in parts {
        if x > 0.0 && x + part.width > max_width + 1e-6 {
            x = 0.0;
            y += row_height + gap;
            row_height = 0.0;
        }
        if y > 0.0 && y + part.height > max_height + 1e-6 {
            sheets.push(Vec::new());
            (x, y, row_height) = (0.0, 0.0, 0.0);
        }
        sheets.last_mut().unwrap().push(Placed { part, x, y });
        x += part.width + gap;
        row_height = row_height.max(part.height);
    }

    Ok(sheets
        .into_iter()
        .filter(|placed| !placed.is_empty())
        .map(|placed| {
            let width = placed
                .iter()
                .map(|p| p.x + p.part.width)
                .fold(0.0, f64::max);
            let height = placed
                .iter()
                .map(|p| p.y + p.part.height)
                .fold(0.0, f64::max);
            let body: String = placed
                .iter()
                .map(|p| {
                    format!(
                        "<g transform=\"translate({:.3} {:.3})\">{}</g>\n",
                        p.x, p.y, p.part.markup
                    )
                })
                .collect();
            Sheet {
                svg: format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.3}mm\" height=\"{height:.3}mm\" viewBox=\"0 0 {width:.3} {height:.3}\">\n\
                     <g fill=\"none\" stroke-width=\"0.2\" stroke-linejoin=\"round\">\n{body}</g>\n</svg>\n"
                ),
                width_mm: width,
                height_mm: height,
                parts: placed.iter().map(|p| p.part.name.clone()).collect(),
            }
        })
        .collect())
}

/// A closed outline as an SVG path in `color`.
pub(crate) fn outline_path(points: &[[f64; 2]], color: &str) -> String {
    let mut d = String::new();
    for (i, [x, y]) in points.iter().enumerate() {
        d.push_str(&format!("{}{x:.3} {y:.3}", if i == 0 { "M" } else { "L" }));
    }
    format!("<path d=\"{d}Z\" stroke=\"{color}\"/>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, width: f64, height: f64) -> Part {
        Part {
            name: name.into(),
            width,
            height,
            markup: outline_path(
                &[[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
                CUT_COLOR,
            ),
        }
    }

    #[test]
    fn parts_fill_rows_then_sheets_in_order() {
        let parts = [
            part("a", 60.0, 40.0),
            part("b", 60.0, 50.0),
            part("c", 60.0, 40.0),
            part("d", 60.0, 40.0),
        ];
        // Two fit across; two rows fit down.
        let sheets = lay_out(&parts, 130.0, 100.0, 5.0).unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].parts, ["a", "b", "c", "d"]);
        assert_eq!(sheets[0].width_mm, 125.0);
        // The second row starts below the taller part of the first.
        assert_eq!(sheets[0].height_mm, 95.0);
        assert!(sheets[0].svg.contains("translate(65.000 0.000)"));
        assert!(sheets[0].svg.contains("translate(0.000 55.000)"));
        assert!(sheets[0].svg.contains("width=\"125.000mm\""));

        // A shorter sheet takes one row, so the rest go on a second.
        let sheets = lay_out(&parts, 130.0, 60.0, 5.0).unwrap();
        assert_eq!(sheets.len(), 2);
        assert_eq!(sheets[1].parts, ["c", "d"]);
        assert_eq!(sheets[1].height_mm, 40.0);
    }

    #[test]
    fn a_part_too_big_for_the_machine_is_refused() {
        let problem = lay_out(&[part("lid", 300.0, 40.0)], 200.0, 200.0, 5.0).unwrap_err();
        assert!(
            problem.contains("lid") && problem.contains("300"),
            "{problem}"
        );
        assert!(lay_out(&[], 100.0, 100.0, 5.0).unwrap().is_empty());
    }
}
