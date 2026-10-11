//! Loading SVG artwork as flattened polylines in millimetres.

use std::sync::{Arc, OnceLock, RwLock};

use usvg::fontdb;
use usvg::tiny_skia_path::{PathSegment, Point, Transform};
use usvg::{Group, Node, Paint};

/// SVG user units are 1/96 inch.
const MM_PER_PX: f64 = 25.4 / 96.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    pub points: Vec<[f64; 2]>,
    pub closed: bool,
}

impl Polyline {
    pub fn length(&self) -> f64 {
        self.points.windows(2).map(|w| distance(w[0], w[1])).sum()
    }
}

/// One SVG path element: every subpath, plus the colour it was drawn in.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Fill colour, or stroke colour for unfilled paths, as `#rrggbb`.
    pub color: Option<String>,
    pub polylines: Vec<Polyline>,
}

/// Artwork in millimetres. The origin is the top-left corner and Y points down,
/// as in SVG.
#[derive(Debug, Clone, PartialEq)]
pub struct Artwork {
    pub width: f64,
    pub height: f64,
    pub shapes: Vec<Shape>,
}

impl Artwork {
    /// Parses an SVG document and flattens its curves so that no point of a
    /// polyline is further than `tolerance` millimetres from the true curve.
    pub fn from_svg(svg: &str, tolerance: f64) -> Result<Self, String> {
        Self::read(svg, tolerance, &usvg::Options::default())
    }

    /// As [`Artwork::from_svg`], and with any text in the document turned
    /// into the outlines of its letters, using the fonts installed on this
    /// computer and any added with [`add_font`]. Reading the installed fonts
    /// takes a moment the first time.
    pub fn from_svg_with_text(svg: &str, tolerance: f64) -> Result<Self, String> {
        let options = usvg::Options {
            fontdb: fonts(),
            ..usvg::Options::default()
        };
        Self::read(svg, tolerance, &options)
    }

    fn read(svg: &str, tolerance: f64, options: &usvg::Options) -> Result<Self, String> {
        let tree = usvg::Tree::from_str(svg, options)
            .map_err(|e| format!("Could not read the SVG: {e}"))?;
        let mut shapes = Vec::new();
        collect(
            tree.root(),
            tolerance.max(1e-4),
            Transform::identity(),
            &mut shapes,
        );
        Ok(Self {
            width: tree.size().width() as f64 * MM_PER_PX,
            height: tree.size().height() as f64 * MM_PER_PX,
            shapes,
        })
    }

    /// Distinct shape colours, in first-seen order.
    pub fn colors(&self) -> Vec<String> {
        let mut colors: Vec<String> = Vec::new();
        for color in self.shapes.iter().filter_map(|s| s.color.as_ref()) {
            if !colors.contains(color) {
                colors.push(color.clone());
            }
        }
        colors
    }
}

/// The fonts text can be set in: those installed on this computer, read once
/// and kept, and any added since.
fn font_store() -> &'static RwLock<Arc<fontdb::Database>> {
    static FONTS: OnceLock<RwLock<Arc<fontdb::Database>>> = OnceLock::new();
    FONTS.get_or_init(|| {
        #[allow(unused_mut)]
        let mut fonts = fontdb::Database::new();
        // A browser has no installed fonts to read.
        #[cfg(not(target_arch = "wasm32"))]
        fonts.load_system_fonts();
        RwLock::new(Arc::new(fonts))
    })
}

fn fonts() -> Arc<fontdb::Database> {
    font_store().read().unwrap().clone()
}

/// Adds the fonts in a font file (TrueType or OpenType, given as the file's
/// bytes) to those text can be set in, and returns their families' names.
/// A file with no font in it adds nothing.
pub fn add_font(file: Vec<u8>) -> Vec<String> {
    let mut store = font_store().write().unwrap();
    let before = store.len();
    Arc::make_mut(&mut store).load_font_data(file);
    family_names(store.faces().skip(before))
}

/// The names of the font families text can be set in, in order.
pub fn font_families() -> Vec<String> {
    family_names(fonts().faces())
}

fn family_names<'a>(faces: impl Iterator<Item = &'a fontdb::FaceInfo>) -> Vec<String> {
    let mut names: Vec<String> = faces
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

pub(crate) fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Gathers the paths under `group`. `outer` is a transform the paths' own
/// absolute transforms do not include, which is the case inside text.
fn collect(group: &Group, tolerance: f64, outer: Transform, shapes: &mut Vec<Shape>) {
    for node in group.children() {
        match node {
            Node::Group(group) => collect(group, tolerance, outer, shapes),
            Node::Path(path) if path.is_visible() => {
                let paint = path
                    .fill()
                    .map(|fill| fill.paint())
                    .or_else(|| path.stroke().map(|stroke| stroke.paint()));
                let color = match paint {
                    Some(Paint::Color(c)) => {
                        Some(format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue))
                    }
                    _ => None,
                };
                let placed = outer.pre_concat(path.abs_transform());
                let Some(data) = path.data().clone().transform(placed) else {
                    continue;
                };
                let polylines = flatten(data.segments(), tolerance);
                if !polylines.is_empty() {
                    shapes.push(Shape { color, polylines });
                }
            }
            // Text arrives already laid out as the outlines of its letters,
            // where fonts were given; without them it is left out.
            // The outlines are placed relative to the text, not the page.
            Node::Text(text) => collect(
                text.flattened(),
                tolerance,
                outer.pre_concat(text.abs_transform()),
                shapes,
            ),
            // Embedded images are not toolpaths.
            _ => {}
        }
    }
}

fn mm(p: Point) -> [f64; 2] {
    [p.x as f64 * MM_PER_PX, p.y as f64 * MM_PER_PX]
}

fn flatten(segments: impl Iterator<Item = PathSegment>, tolerance: f64) -> Vec<Polyline> {
    let mut done = Vec::new();
    let mut current: Vec<[f64; 2]> = Vec::new();

    let mut finish = |points: &mut Vec<[f64; 2]>, closed: bool| {
        let mut points = std::mem::take(points);
        points.dedup_by(|a, b| distance(*a, *b) < 1e-9);
        if closed && points.len() > 2 && distance(points[0], points[points.len() - 1]) > 1e-9 {
            points.push(points[0]);
        }
        if points.len() >= 2 {
            done.push(Polyline { points, closed });
        }
    };

    for segment in segments {
        match segment {
            PathSegment::MoveTo(p) => {
                finish(&mut current, false);
                current.push(mm(p));
            }
            PathSegment::LineTo(p) => current.push(mm(p)),
            PathSegment::QuadTo(c, p) => {
                let Some(&start) = current.last() else {
                    continue;
                };
                let (c, end) = (mm(c), mm(p));
                // Wang's formula for the number of segments within tolerance.
                let deviation =
                    (start[0] - 2.0 * c[0] + end[0]).hypot(start[1] - 2.0 * c[1] + end[1]);
                let steps = steps_for(0.25 * deviation / tolerance);
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let u = 1.0 - t;
                    current.push([
                        u * u * start[0] + 2.0 * u * t * c[0] + t * t * end[0],
                        u * u * start[1] + 2.0 * u * t * c[1] + t * t * end[1],
                    ]);
                }
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let Some(&start) = current.last() else {
                    continue;
                };
                let (c1, c2, end) = (mm(c1), mm(c2), mm(p));
                let d1 = (start[0] - 2.0 * c1[0] + c2[0]).hypot(start[1] - 2.0 * c1[1] + c2[1]);
                let d2 = (c1[0] - 2.0 * c2[0] + end[0]).hypot(c1[1] - 2.0 * c2[1] + end[1]);
                let steps = steps_for(0.75 * d1.max(d2) / tolerance);
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    current.push([
                        a * start[0] + b * c1[0] + c * c2[0] + d * end[0],
                        a * start[1] + b * c1[1] + c * c2[1] + d * end[1],
                    ]);
                }
            }
            PathSegment::Close => {
                let restart = current.first().copied();
                finish(&mut current, true);
                // Drawing may continue from the subpath's start without a MoveTo.
                current.extend(restart);
            }
        }
    }
    finish(&mut current, false);
    done
}

fn steps_for(ratio: f64) -> usize {
    (ratio.sqrt().ceil() as usize).clamp(1, 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(body: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 200 100">{body}</svg>"#
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn size_and_viewbox_scale_to_mm() {
        let art =
            Artwork::from_svg(&svg(r##"<path d="M0 0H200V100H0Z" fill="#000"/>"##), 0.05).unwrap();
        assert!(close(art.width, 100.0) && close(art.height, 50.0));
        let polyline = &art.shapes[0].polylines[0];
        assert!(polyline.closed);
        assert_eq!(polyline.points.len(), 5);
        assert!(close(polyline.points[2][0], 100.0) && close(polyline.points[2][1], 50.0));
        assert!(close(polyline.length(), 300.0));
    }

    #[test]
    fn transforms_are_applied() {
        let art = Artwork::from_svg(
            &svg(r##"<g transform="translate(20,10)"><path d="M0 0L10 0" stroke="#f00" fill="none"/></g>"##),
            0.05,
        )
        .unwrap();
        let polyline = &art.shapes[0].polylines[0];
        assert!(!polyline.closed);
        assert!(close(polyline.points[0][0], 10.0) && close(polyline.points[0][1], 5.0));
        assert_eq!(art.shapes[0].color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn curves_stay_within_tolerance() {
        // A circle of radius 40 units = 20 mm, centred at (50 mm, 25 mm).
        let art = Artwork::from_svg(
            &svg(r##"<circle cx="100" cy="50" r="40" fill="#000"/>"##),
            0.01,
        )
        .unwrap();
        let polyline = &art.shapes[0].polylines[0];
        assert!(polyline.closed);
        for p in &polyline.points {
            let radius = (p[0] - 50.0).hypot(p[1] - 25.0);
            assert!((radius - 20.0).abs() < 0.02, "radius {radius}");
        }
        // Chords are shorter than the arc, by less than the tolerance allows.
        let circumference = 2.0 * std::f64::consts::PI * 20.0;
        assert!((polyline.length() - circumference).abs() < 0.1);
    }

    #[test]
    fn subpaths_and_colors_are_kept() {
        let art = Artwork::from_svg(
            &svg(
                r##"<path d="M0 0H50V50H0Z M10 10H40V40H10Z" fill="#102030"/>
                   <path d="M100 0H150V50H100Z" fill="#102030"/>
                   <path d="M0 60H10" stroke="#00ff00" fill="none"/>"##,
            ),
            0.05,
        )
        .unwrap();
        assert_eq!(art.shapes.len(), 3);
        assert_eq!(art.shapes[0].polylines.len(), 2);
        assert_eq!(art.colors(), ["#102030", "#00ff00"]);
    }

    #[test]
    fn text_becomes_the_outlines_of_its_letters_given_fonts() {
        let families = font_families();
        // Machines that build this without any fonts have nothing to check.
        let Some(family) = families.first() else {
            return;
        };
        let document = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="50mm" viewBox="0 0 200 50"><text x="5" y="30" font-family="{}" font-size="20">Hi</text></svg>"##,
            family.replace('&', "&amp;").replace('"', "&quot;")
        );
        // Without fonts the text is left out; with them it is closed outlines.
        assert!(Artwork::from_svg(&document, 0.05)
            .unwrap()
            .shapes
            .is_empty());
        let art = Artwork::from_svg_with_text(&document, 0.05).unwrap();
        let outlines: Vec<&Polyline> = art.shapes.iter().flat_map(|s| &s.polylines).collect();
        // Symbol fonts may draw nothing for these letters.
        if !outlines.is_empty() {
            assert!(outlines.iter().all(|p| p.closed));
            let tallest = outlines
                .iter()
                .flat_map(|p| &p.points)
                .map(|p| p[1])
                .fold(f64::MIN, f64::max);
            assert!(tallest <= 50.0);
            // A 20-unit font on a page of one unit to the millimetre: the
            // letters are millimetres tall, not a quarter of that.
            let highest = outlines
                .iter()
                .flat_map(|p| &p.points)
                .map(|p| p[1])
                .fold(f64::MAX, f64::min);
            if families.iter().any(|f| f == "Arial") && family == "Arial" {
                assert!(
                    (10.0..20.0).contains(&(tallest - highest)),
                    "{}",
                    tallest - highest
                );
            }
        }
        assert!(families
            .windows(2)
            .all(|pair| pair[0].to_lowercase() <= pair[1].to_lowercase()));
    }

    #[test]
    fn invalid_svg_is_an_error() {
        assert!(Artwork::from_svg("not svg", 0.05).is_err());
    }
}
