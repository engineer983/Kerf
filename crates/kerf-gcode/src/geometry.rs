//! Loading SVG artwork as flattened polylines in millimetres.

use usvg::tiny_skia_path::{PathSegment, Point};
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
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default())
            .map_err(|e| format!("Could not read the SVG: {e}"))?;
        let mut shapes = Vec::new();
        collect(tree.root(), tolerance.max(1e-4), &mut shapes);
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

pub(crate) fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn collect(group: &Group, tolerance: f64, shapes: &mut Vec<Shape>) {
    for node in group.children() {
        match node {
            Node::Group(group) => collect(group, tolerance, shapes),
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
                let Some(data) = path.data().clone().transform(path.abs_transform()) else {
                    continue;
                };
                let polylines = flatten(data.segments(), tolerance);
                if !polylines.is_empty() {
                    shapes.push(Shape { color, polylines });
                }
            }
            // Embedded images and text are not toolpaths.
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
    fn invalid_svg_is_an_error() {
        assert!(Artwork::from_svg("not svg", 0.05).is_err());
    }
}
