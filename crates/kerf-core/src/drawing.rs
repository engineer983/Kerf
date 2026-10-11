//! Drawings that arrive as lines rather than pixels: SVG documents, and
//! words set in a font.
//!
//! Whatever the source, it ends up as one [`Sheet`]: plain paths in
//! millimetres, cropped to what is drawn, each in the colour it was given.

use kerf_gcode::Artwork;
use kerf_make::Sheet;
use serde::{Deserialize, Serialize};

use crate::FLATTEN_TOLERANCE_MM;

/// Colour given to lines that had none of their own.
const PLAIN_COLOR: &str = "#000000";
/// `svg` as a sheet of plain paths, cropped to what is drawn. Text is turned
/// into the outlines of its letters using this computer's fonts.
pub fn drawn(svg: &str, name: &str) -> Result<Sheet, String> {
    drawn_at(svg, name).map(|(sheet, _)| sheet)
}

/// As [`drawn`], also saying where on the original page the cropped sheet's
/// top-left corner was, in millimetres.
fn drawn_at(svg: &str, name: &str) -> Result<(Sheet, [f64; 2]), String> {
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
    let sheet = Sheet {
        svg: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.3}mm\" height=\"{height:.3}mm\" viewBox=\"0 0 {width:.3} {height:.3}\">\n\
             <g fill=\"none\" stroke-width=\"0.2\" stroke-linejoin=\"round\">\n{body}</g>\n</svg>\n"
        ),
        width_mm: width,
        height_mm: height,
        parts: vec![name.to_owned()],
    };
    Ok((sheet, [left, top]))
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSpec {
    pub text: String,
    /// The name of a font family installed on this computer.
    pub font: String,
    /// The font's size in millimetres. Capital letters stand about seven tenths of it.
    pub size_mm: f64,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    /// How far the words bend round a circle, in degrees: zero is straight,
    /// more than zero arches them over the top, less than zero curves them
    /// under the bottom. 180 is half way round.
    #[serde(default)]
    pub bend: f64,
    /// Set the words round something, such as a picture, in place of a
    /// straight line or a plain curve.
    #[serde(default)]
    pub around: Option<Around>,
}

/// The line that words follow round something.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Ring {
    /// The circle through the thing's corners.
    Circle,
    /// The oval through its corners, which hugs a wide or tall thing closer.
    Oval,
    /// Its own rectangle, with the corners rounded so the words can turn them.
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Around {
    /// The size of the thing the words go round, in millimetres.
    pub width: f64,
    pub height: f64,
    /// Clear space between the thing and the words.
    pub gap: f64,
    pub shape: Ring,
    /// Centred over its top, or under its bottom. Long text carries on down
    /// or up the sides.
    pub side: Side,
}

/// Words drawn as outlines.
#[derive(Debug, Serialize)]
pub struct TextMade {
    pub sheet: Sheet,
    /// For words set round something: where the sheet's top-left corner sits,
    /// in millimetres right of and below the centre of that thing.
    pub offset: Option<[f64; 2]>,
}

/// The height of a capital letter, as a share of the font's size, near enough
/// for keeping words clear of what they go round.
const CAPITAL_SHARE: f64 = 0.75;

impl Around {
    /// The closed line the words follow, as SVG path data about (`cx`, `cy`),
    /// with its length. `out` is how far outside the thing it runs. It starts
    /// opposite the side the words are on, so that half way along it is the
    /// middle of that side, and it runs so that the words read left to right
    /// there: clockwise for the top, anticlockwise for the bottom.
    fn line(&self, cx: f64, cy: f64, out: f64) -> (String, f64) {
        use std::f64::consts::{PI, SQRT_2};
        let top = self.side == Side::Top;
        // Down the page is positive; the line starts on this side of the centre.
        let start = if top { 1.0 } else { -1.0 };
        let sweep = if top { 1 } else { 0 };
        match self.shape {
            Ring::Circle | Ring::Oval => {
                let (rx, ry) = if self.shape == Ring::Circle {
                    let r = self.width.hypot(self.height) / 2.0 + out;
                    (r, r)
                } else {
                    (self.width / SQRT_2 + out, self.height / SQRT_2 + out)
                };
                let d = format!(
                    "M{cx:.3} {:.3}A{rx:.3} {ry:.3} 0 1 {sweep} {cx:.3} {:.3}A{rx:.3} {ry:.3} 0 1 {sweep} {cx:.3} {:.3}",
                    cy + start * ry,
                    cy - start * ry,
                    cy + start * ry,
                );
                // Ramanujan's approximation, exact for a circle.
                let length = PI * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
                (d, length)
            }
            Ring::Frame => {
                let (hw, hh) = (self.width / 2.0 + out, self.height / 2.0 + out);
                let c = hw.min(hh) * 0.35;
                // The far side from the words, then round by the left.
                let (near, far) = (cy - start * hh, cy + start * hh);
                let toward = |from: f64, by: f64| from - start * by;
                let d = format!(
                    "M{cx:.3} {far:.3}L{:.3} {far:.3}A{c:.3} {c:.3} 0 0 {sweep} {:.3} {:.3}L{:.3} {:.3}A{c:.3} {c:.3} 0 0 {sweep} {:.3} {near:.3}L{:.3} {near:.3}A{c:.3} {c:.3} 0 0 {sweep} {:.3} {:.3}L{:.3} {:.3}A{c:.3} {c:.3} 0 0 {sweep} {:.3} {far:.3}Z",
                    cx - hw + c,
                    cx - hw,
                    toward(far, c),
                    cx - hw,
                    toward(near, -c),
                    cx - hw + c,
                    cx + hw - c,
                    cx + hw,
                    toward(near, -c),
                    cx + hw,
                    toward(far, c),
                    cx + hw - c,
                );
                (d, 4.0 * (hw + hh) - 8.0 * c + 2.0 * PI * c)
            }
        }
    }
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

/// The words in `spec` as outlines, and for words set round something, where
/// they land relative to it.
pub fn text_made(spec: &TextSpec) -> Result<TextMade, String> {
    let Some(around) = spec.around else {
        return text_sheet(spec).map(|sheet| TextMade {
            sheet,
            offset: None,
        });
    };
    let lines: Vec<&str> = spec
        .text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err("Type the words to make first.".into());
    }
    if !(spec.size_mm.is_finite() && spec.size_mm > 0.0 && spec.size_mm <= 1000.0) {
        return Err("The size must be between 0 and 1000 mm.".into());
    }
    let sized = |v: f64| v.is_finite() && v > 0.0;
    if !sized(around.width) || !sized(around.height) || !around.gap.is_finite() {
        return Err("There is nothing on the material for the words to go round.".into());
    }
    let straight = TextSpec {
        bend: 0.0,
        around: None,
        ..spec.clone()
    };
    let spacing = spec.size_mm * LINE_SPACING;
    let top = around.side == Side::Top;
    // Over the top, letters stand outside their line, so the last line is the
    // one nearest the thing. Underneath they hang inside it, so every line is
    // pushed out by a letter's height and the first is nearest.
    let out = |i: usize| {
        around.gap
            + if top {
                (lines.len() - 1 - i) as f64 * spacing
            } else {
                spec.size_mm * CAPITAL_SHARE + i as f64 * spacing
            }
    };
    let reach = around.width.hypot(around.height) / 2.0
        + around.gap.max(0.0)
        + (lines.len() as f64 + 2.0) * spacing;
    let (page, centre) = (reach * 2.0, reach);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{page:.1}mm\" height=\"{page:.1}mm\" viewBox=\"0 0 {page:.1} {page:.1}\">\n<defs>\n"
    );
    for (i, line) in lines.iter().enumerate() {
        let (d, length) = around.line(centre, centre, out(i));
        let needed = text_sheet(&TextSpec {
            text: (*line).to_owned(),
            ..straight.clone()
        })
        .map(|sheet| sheet.width_mm)
        .unwrap_or(0.0);
        if needed > length * 0.95 {
            return Err(format!(
                "\"{line}\" is too long to go round at this size: it needs {needed:.0} mm and there is {length:.0} mm. Make the letters smaller, the gap larger, or the words fewer."
            ));
        }
        svg.push_str(&format!("<path id=\"line{i}\" d=\"{d}\"/>\n"));
    }
    svg.push_str("</defs>\n");
    for (i, line) in lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text xml:space=\"preserve\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" text-anchor=\"middle\" fill=\"{PLAIN_COLOR}\"><textPath xlink:href=\"#line{i}\" startOffset=\"50%\">{}</textPath></text>\n",
            escaped(&spec.font),
            spec.size_mm,
            if spec.bold { "bold" } else { "normal" },
            if spec.italic { "italic" } else { "normal" },
            escaped(line),
        ));
    }
    svg.push_str("</svg>\n");
    let name: String = lines[0].chars().take(40).collect();
    let (sheet, [left, top_edge]) = drawn_at(&svg, &name).map_err(|_| {
        format!(
            "The font {} has no shapes for those characters. Choose another font.",
            spec.font
        )
    })?;
    Ok(TextMade {
        sheet,
        offset: Some([left - centre, top_edge - centre]),
    })
}

/// The words in `spec`, straight or on a plain curve, as the outlines of their letters.
pub fn text_sheet(spec: &TextSpec) -> Result<Sheet, String> {
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
            around: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Words in a plain font, where this machine has one.
    fn words(text: &str, around: Option<Around>) -> Option<TextSpec> {
        let font = kerf_gcode::font_families().into_iter().find(|name| {
            ["Arial", "DejaVu Sans", "Liberation Sans", "Helvetica"].contains(&name.as_str())
        })?;
        Some(TextSpec {
            text: text.into(),
            font,
            size_mm: 10.0,
            bold: false,
            italic: false,
            align: Align::Center,
            bend: 0.0,
            around,
        })
    }

    fn round(shape: Ring, side: Side) -> Option<Around> {
        Some(Around {
            width: 100.0,
            height: 60.0,
            gap: 5.0,
            shape,
            side,
        })
    }

    #[test]
    fn words_go_round_a_picture_clear_of_it_and_centred_on_it() {
        for shape in [Ring::Circle, Ring::Oval, Ring::Frame] {
            let Some(spec) = words("HAPPY BIRTHDAY", round(shape, Side::Top)) else {
                return;
            };
            let made = text_made(&spec).unwrap();
            let [dx, dy] = made.offset.unwrap();
            let (left, right) = (dx, dx + made.sheet.width_mm);
            let (top, bottom) = (dy, dy + made.sheet.height_mm);
            // Centred side to side on the picture, which spans -50..50 by -30..30.
            assert!((left + right).abs() < 3.0, "{shape:?}: {left} to {right}");
            // Above its middle, and reaching above its top edge.
            assert!(top < -30.0 && bottom < 30.0, "{shape:?}: {top} to {bottom}");
            // No letter comes inside the picture's rectangle less the gap:
            // every point of the words is outside it.
            let art = Artwork::from_svg(&made.sheet.svg, FLATTEN_TOLERANCE_MM).unwrap();
            let inside = art
                .shapes
                .iter()
                .flat_map(|s| &s.polylines)
                .flat_map(|p| &p.points)
                .filter(|p| (p[0] + dx).abs() < 49.0 && (p[1] + dy).abs() < 29.0)
                .count();
            assert_eq!(inside, 0, "{shape:?}");

            let under = text_made(&TextSpec {
                around: round(shape, Side::Bottom),
                ..spec.clone()
            })
            .unwrap();
            let [ux, uy] = under.offset.unwrap();
            assert!((ux * 2.0 + under.sheet.width_mm).abs() < 3.0);
            assert!(
                uy + under.sheet.height_mm > 30.0 && uy > -30.0,
                "{shape:?}: {uy}"
            );
        }
    }

    #[test]
    fn words_too_long_to_go_round_are_refused() {
        let Some(spec) = words(
            &"ROUND AND ROUND ".repeat(12),
            round(Ring::Frame, Side::Top),
        ) else {
            return;
        };
        assert!(text_made(&spec).unwrap_err().contains("too long"));
        let nothing = Around {
            width: 0.0,
            ..round(Ring::Circle, Side::Top).unwrap()
        };
        assert!(text_made(&TextSpec {
            around: Some(nothing),
            ..words("HI", None).unwrap()
        })
        .is_err());
        // Without anything to go round, the words are simply drawn.
        assert!(text_made(&words("HI", None).unwrap())
            .unwrap()
            .offset
            .is_none());
    }
}
