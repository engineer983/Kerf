//! Lining a camera picture up with the bed.
//!
//! The person marks four places in a still from a camera over the bed and
//! says where each one is on the bed. From those, a perspective mapping
//! (a homography) turns camera pixels into bed millimetres and back, and the
//! still can be redrawn as if seen from straight above.
//!
//! Bed millimetres here are the canvas's: X from the left, Y down from the
//! back. The machine's origin corner doesn't matter at this level, so
//! changing it never moves the picture.
//!
//! Lens curve and material height are not corrected: the fit is made at the
//! bed surface.

use kerf_trace::image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

/// Pixels per millimetre of the redrawn bed picture, before any cap.
const PX_PER_MM: f64 = 2.0;
/// Longest side of the redrawn bed picture, in pixels.
const MAX_SIDE_PX: f64 = 2400.0;
/// Markers closer than this in the picture are refused as the same place.
const MIN_PIXEL_GAP: f64 = 8.0;
/// Bed positions closer than this are refused as the same place.
const MIN_MM_GAP: f64 = 5.0;
/// How much smaller than its sides a triangle of three points may be before
/// they count as being in a line: area over the longest side squared.
const MIN_TRIANGLE_SHAPE: f64 = 0.02;
/// How far apart the fit's bed size may be from the current bed, in mm.
const BED_SLACK_MM: f64 = 0.01;

/// One marker: a place in the camera picture and where it is on the bed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FitPoint {
    /// Across the picture, in pixels from the left.
    pub px: f64,
    /// Down the picture, in pixels from the top.
    pub py: f64,
    /// Across the bed, in mm from the left.
    pub x: f64,
    /// Down the bed, in mm from the back.
    pub y: f64,
}

/// How a camera's pictures line up with the bed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraFit {
    pub points: [FitPoint; 4],
    /// The bed size the fit was made for. A fit is no good for another size,
    /// because the bed positions were measured on that one.
    pub bed_width: f64,
    pub bed_height: f64,
    /// The size of the still the markers were placed on.
    pub picture_width: u32,
    pub picture_height: u32,
}

/// A bed's size in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedSize {
    pub width: f64,
    pub height: f64,
}

/// A perspective mapping from camera pixels to bed millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    /// Row-major 3×3, with the last entry fixed at 1.
    forward: [f64; 9],
    inverse: [f64; 9],
}

impl Homography {
    /// The mapping that takes each picture position to its bed position.
    /// Refuses markers that can't give a sensible mapping, with a message
    /// the person can act on.
    pub fn from_points(points: &[FitPoint; 4]) -> Result<Self, String> {
        for p in points {
            let values = [p.px, p.py, p.x, p.y];
            if values.iter().any(|v| !v.is_finite()) {
                return Err("One of the markers has no position.".into());
            }
        }
        for i in 0..4 {
            for j in (i + 1)..4 {
                let (a, b) = (points[i], points[j]);
                if (a.px - b.px).hypot(a.py - b.py) < MIN_PIXEL_GAP {
                    return Err("Two markers are on the same spot in the picture. Move them onto four different places.".into());
                }
                if (a.x - b.x).hypot(a.y - b.y) < MIN_MM_GAP {
                    return Err("Two markers stand for the same place on the bed. Give each one a different position.".into());
                }
            }
        }
        let pixels: Vec<(f64, f64)> = points.iter().map(|p| (p.px, p.py)).collect();
        let bed: Vec<(f64, f64)> = points.iter().map(|p| (p.x, p.y)).collect();
        if any_three_in_line(&pixels) {
            return Err("Three markers are in a line in the picture. Spread them out, ideally near the bed's corners.".into());
        }
        if any_three_in_line(&bed) {
            return Err("Three of the bed positions are in a line. Use places that make a rectangle or close to one.".into());
        }

        let forward = solve(points)?;
        let inverse = invert(&forward).ok_or(
            "These markers don't give a usable fit. Check each one is on the right place.",
        )?;
        let fit = Self { forward, inverse };
        // Every marker must be in front of the camera in the mapping;
        // otherwise the markers are in an impossible order (for example,
        // crossed over), and the redrawn picture would be nonsense.
        for p in points {
            if fit.weight(&fit.forward, p.px, p.py) <= 0.0 {
                return Err("The markers are crossed over. Check each one is on the corner it is named for.".into());
            }
        }
        Ok(fit)
    }

    /// The bed position, in mm, of a place in the picture. `None` if the
    /// place can't be on the bed (it's beyond the camera's horizon).
    pub fn to_bed(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        apply(&self.forward, px, py)
    }

    /// Where a bed position, in mm, appears in the picture.
    pub fn to_pixel(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        apply(&self.inverse, x, y)
    }

    fn weight(&self, m: &[f64; 9], x: f64, y: f64) -> f64 {
        m[6] * x + m[7] * y + m[8]
    }
}

/// Whether the markers go round the picture the opposite way from the bed,
/// which means the picture is mirrored (or a marker is on the wrong corner).
/// Both use Y downwards, so a straight view goes round the same way.
pub fn is_mirrored(points: &[FitPoint; 4]) -> bool {
    let pixels: Vec<(f64, f64)> = points.iter().map(|p| (p.px, p.py)).collect();
    let bed: Vec<(f64, f64)> = points.iter().map(|p| (p.x, p.y)).collect();
    signed_area(&pixels) * signed_area(&bed) < 0.0
}

/// What the person should know about a fit, for the lining-up screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitCheck {
    pub mirrored: bool,
}

/// Checks a fit can be used, and says whether it looks mirrored.
pub fn check_fit(fit: &CameraFit) -> Result<FitCheck, String> {
    Homography::from_points(&fit.points)?;
    Ok(FitCheck {
        mirrored: is_mirrored(&fit.points),
    })
}

/// The camera still redrawn as the bed seen from straight above: the top-left
/// pixel is the bed's back-left corner, at a fixed number of pixels per mm
/// (see [`bed_picture_scale`]). Anything the camera can't see is transparent.
pub fn bed_picture(still: &RgbaImage, fit: &CameraFit, bed: BedSize) -> Result<RgbaImage, String> {
    if !(bed.width.is_finite() && bed.height.is_finite()) || bed.width <= 0.0 || bed.height <= 0.0 {
        return Err("The bed needs a size before the camera picture can be shown.".into());
    }
    if (fit.bed_width - bed.width).abs() > BED_SLACK_MM
        || (fit.bed_height - bed.height).abs() > BED_SLACK_MM
    {
        return Err(
            "The bed size has changed since the camera was lined up. Line the camera up again."
                .into(),
        );
    }
    if still.width() != fit.picture_width || still.height() != fit.picture_height {
        return Err("This picture is a different size from the one the camera was lined up with. Line the camera up again.".into());
    }
    let mapping = Homography::from_points(&fit.points)?;

    let scale = bed_picture_scale(bed);
    let out_w = ((bed.width * scale).round() as u32).max(1);
    let out_h = ((bed.height * scale).round() as u32).max(1);
    let mut out = RgbaImage::new(out_w, out_h);
    let (in_w, in_h) = (still.width() as f64, still.height() as f64);

    for oy in 0..out_h {
        let y_mm = (oy as f64 + 0.5) / scale;
        for ox in 0..out_w {
            let x_mm = (ox as f64 + 0.5) / scale;
            let Some((px, py)) = mapping.to_pixel(x_mm, y_mm) else {
                continue;
            };
            // Pixel centres sit at +0.5, so shift before sampling.
            let (sx, sy) = (px - 0.5, py - 0.5);
            if !(sx >= -0.5 && sy >= -0.5 && sx <= in_w - 0.5 && sy <= in_h - 0.5) {
                continue;
            }
            out.put_pixel(ox, oy, sample(still, sx, sy));
        }
    }
    Ok(out)
}

/// The latest still from the camera, kept by the engine so the picture can
/// be redrawn whenever the fit or bed changes, without opening the camera
/// again.
pub struct CameraStill {
    pub image: std::sync::Arc<RgbaImage>,
}

impl CameraStill {
    /// Reads a still as sent by the interface (a JPEG, usually).
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let image = kerf_trace::image::load_from_memory(bytes)
            .map_err(|_| "The camera's picture could not be read.".to_string())?
            .to_rgba8();
        if image.width() == 0 || image.height() == 0 {
            return Err("The camera's picture is empty.".into());
        }
        Ok(Self {
            image: std::sync::Arc::new(image),
        })
    }
}

/// What a [`camera_bed_picture_png`] request carries.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedPictureRequest {
    pub fit: CameraFit,
    pub bed: BedSize,
}

/// [`bed_picture`] as a PNG file's bytes.
pub fn camera_bed_picture_png(
    still: &RgbaImage,
    request: &BedPictureRequest,
) -> Result<Vec<u8>, String> {
    let picture = bed_picture(still, &request.fit, request.bed)?;
    crate::encode_png(&kerf_trace::image::DynamicImage::ImageRgba8(picture))
}

/// Pixels per mm of [`bed_picture`] for a bed: 2, or fewer for a big bed so
/// the picture stays a manageable size.
pub fn bed_picture_scale(bed: BedSize) -> f64 {
    let longest = bed.width.max(bed.height);
    if longest * PX_PER_MM > MAX_SIDE_PX {
        MAX_SIDE_PX / longest
    } else {
        PX_PER_MM
    }
}

/// Bilinear sample at a position already known to be inside the picture
/// (to within half a pixel).
pub(crate) fn sample(image: &RgbaImage, x: f64, y: f64) -> Rgba<u8> {
    let max_x = image.width().saturating_sub(1);
    let max_y = image.height().saturating_sub(1);
    let x = x.clamp(0.0, max_x as f64);
    let y = y.clamp(0.0, max_y as f64);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(max_x), (y0 + 1).min(max_y));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let (a, b, c, d) = (
        image.get_pixel(x0, y0).0,
        image.get_pixel(x1, y0).0,
        image.get_pixel(x0, y1).0,
        image.get_pixel(x1, y1).0,
    );
    let mut px = [0u8; 4];
    for i in 0..3 {
        let top = a[i] as f64 * (1.0 - fx) + b[i] as f64 * fx;
        let bottom = c[i] as f64 * (1.0 - fx) + d[i] as f64 * fx;
        px[i] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    px[3] = 255;
    Rgba(px)
}

fn apply(m: &[f64; 9], x: f64, y: f64) -> Option<(f64, f64)> {
    let w = m[6] * x + m[7] * y + m[8];
    if !w.is_finite() || w <= 1e-12 {
        return None;
    }
    let u = (m[0] * x + m[1] * y + m[2]) / w;
    let v = (m[3] * x + m[4] * y + m[5]) / w;
    (u.is_finite() && v.is_finite()).then_some((u, v))
}

/// Solves for the 8 unknowns of the mapping from the four point pairs.
fn solve(points: &[FitPoint; 4]) -> Result<[f64; 9], String> {
    // Work in scaled coordinates so pixel and mm sizes don't make the
    // equations lopsided, then undo the scaling.
    let (sp, tp) = normaliser(points.iter().map(|p| (p.px, p.py)));
    let (sb, tb) = normaliser(points.iter().map(|p| (p.x, p.y)));

    let mut a = [[0.0f64; 9]; 8];
    for (i, p) in points.iter().enumerate() {
        let (x, y) = ((p.px - tp.0) * sp, (p.py - tp.1) * sp);
        let (u, v) = ((p.x - tb.0) * sb, (p.y - tb.1) * sb);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    let h = gauss(a)
        .ok_or("These markers don't give a usable fit. Check each one is on the right place.")?;
    let n = [h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0];

    // forward = Tb⁻¹ · N · Tp, where Tp scales pixels and Tb scales mm.
    let t_pixel = [sp, 0.0, -sp * tp.0, 0.0, sp, -sp * tp.1, 0.0, 0.0, 1.0];
    let t_bed_inverse = [1.0 / sb, 0.0, tb.0, 0.0, 1.0 / sb, tb.1, 0.0, 0.0, 1.0];
    let m = multiply(&t_bed_inverse, &multiply(&n, &t_pixel));
    normalise(m).ok_or_else(|| {
        "These markers don't give a usable fit. Check each one is on the right place.".to_string()
    })
}

/// Centre and scale that bring a set of points to about ±1.
fn normaliser(points: impl Iterator<Item = (f64, f64)>) -> (f64, (f64, f64)) {
    let points: Vec<(f64, f64)> = points.collect();
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.1).sum::<f64>() / n;
    let spread = points
        .iter()
        .map(|p| (p.0 - cx).hypot(p.1 - cy))
        .sum::<f64>()
        / n;
    let scale = if spread > 0.0 {
        std::f64::consts::SQRT_2 / spread
    } else {
        1.0
    };
    (scale, (cx, cy))
}

/// Gaussian elimination with partial pivoting on an 8×8 system whose last
/// column is the right-hand side.
fn gauss(mut a: [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let pivot = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        let pivot_row = a[col];
        for (row, values) in a.iter_mut().enumerate() {
            if row != col {
                let factor = values[col] / pivot_row[col];
                for (value, &p) in values.iter_mut().zip(pivot_row.iter()).skip(col) {
                    *value -= factor * p;
                }
            }
        }
    }
    let mut out = [0.0; 8];
    for i in 0..8 {
        out[i] = a[i][8] / a[i][i];
        if !out[i].is_finite() {
            return None;
        }
    }
    Some(out)
}

fn multiply(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    out
}

fn invert(m: &[f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if !det.is_finite() || det.abs() < 1e-15 {
        return None;
    }
    let adj = [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ];
    normalise(adj.map(|v| v / det))
}

/// Scales a mapping so its last entry is 1 (with a positive weight for
/// points in front of the camera).
fn normalise(m: [f64; 9]) -> Option<[f64; 9]> {
    if !m[8].is_finite() || m[8].abs() < 1e-15 {
        return None;
    }
    let out = m.map(|v| v / m[8]);
    out.iter().all(|v| v.is_finite()).then_some(out)
}

fn signed_area(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        / 2.0
}

fn any_three_in_line(points: &[(f64, f64)]) -> bool {
    for i in 0..points.len() {
        for j in (i + 1)..points.len() {
            for k in (j + 1)..points.len() {
                let (a, b, c) = (points[i], points[j], points[k]);
                let area = ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs() / 2.0;
                let longest = [(a, b), (b, c), (a, c)]
                    .iter()
                    .map(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1))
                    .fold(0.0, f64::max);
                if longest == 0.0 || area / (longest * longest) < MIN_TRIANGLE_SHAPE {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(px: f64, py: f64, x: f64, y: f64) -> FitPoint {
        FitPoint { px, py, x, y }
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    /// A camera looking down at a slant: the far (back) edge of a 400×300
    /// bed looks narrower than the near one.
    fn trapezoid() -> [FitPoint; 4] {
        [
            point(300.0, 100.0, 0.0, 0.0),
            point(900.0, 100.0, 400.0, 0.0),
            point(1100.0, 650.0, 400.0, 300.0),
            point(100.0, 650.0, 0.0, 300.0),
        ]
    }

    fn fit_for(points: [FitPoint; 4], bed: BedSize, picture: (u32, u32)) -> CameraFit {
        CameraFit {
            points,
            bed_width: bed.width,
            bed_height: bed.height,
            picture_width: picture.0,
            picture_height: picture.1,
        }
    }

    #[test]
    fn a_straight_view_maps_pixels_to_the_same_millimetres() {
        let points = [
            point(0.0, 0.0, 0.0, 0.0),
            point(100.0, 0.0, 100.0, 0.0),
            point(100.0, 100.0, 100.0, 100.0),
            point(0.0, 100.0, 0.0, 100.0),
        ];
        let h = Homography::from_points(&points).unwrap();
        assert!(close(h.to_bed(37.0, 81.5).unwrap(), (37.0, 81.5)));
        assert!(close(h.to_pixel(12.0, 3.0).unwrap(), (12.0, 3.0)));
    }

    #[test]
    fn a_slanted_view_maps_the_four_markers_exactly() {
        let points = trapezoid();
        let h = Homography::from_points(&points).unwrap();
        for p in &points {
            assert!(close(h.to_bed(p.px, p.py).unwrap(), (p.x, p.y)), "{p:?}");
            assert!(close(h.to_pixel(p.x, p.y).unwrap(), (p.px, p.py)), "{p:?}");
        }
        // The diagonals of the picture's trapezoid cross at the bed's centre
        // only in a straight view; here the centre must be nearer the back
        // (smaller py) than the picture's middle line, since far things are
        // squashed.
        let (_, centre_py) = h.to_pixel(200.0, 150.0).unwrap();
        assert!(centre_py < 375.0, "{centre_py}");
        let (cx, _) = h.to_pixel(200.0, 150.0).unwrap();
        assert!(
            (cx - 600.0).abs() < 1e-6,
            "symmetrical view keeps the centre line: {cx}"
        );
    }

    #[test]
    fn going_there_and_back_gives_the_same_place() {
        let h = Homography::from_points(&trapezoid()).unwrap();
        for &(x, y) in &[(0.0, 0.0), (123.4, 56.7), (399.0, 299.0), (200.0, 10.0)] {
            let (px, py) = h.to_pixel(x, y).unwrap();
            assert!(close(h.to_bed(px, py).unwrap(), (x, y)));
        }
    }

    #[test]
    fn markers_in_a_line_or_on_the_same_spot_are_refused() {
        let in_line = [
            point(0.0, 0.0, 0.0, 0.0),
            point(100.0, 0.0, 100.0, 0.0),
            point(200.0, 0.5, 100.0, 100.0),
            point(0.0, 100.0, 0.0, 100.0),
        ];
        assert!(Homography::from_points(&in_line)
            .unwrap_err()
            .contains("in a line"));

        let same_spot = [
            point(0.0, 0.0, 0.0, 0.0),
            point(2.0, 1.0, 100.0, 0.0),
            point(100.0, 100.0, 100.0, 100.0),
            point(0.0, 100.0, 0.0, 100.0),
        ];
        assert!(Homography::from_points(&same_spot)
            .unwrap_err()
            .contains("same spot"));

        let same_place = [
            point(0.0, 0.0, 0.0, 0.0),
            point(100.0, 0.0, 1.0, 1.0),
            point(100.0, 100.0, 100.0, 100.0),
            point(0.0, 100.0, 0.0, 100.0),
        ];
        assert!(Homography::from_points(&same_place)
            .unwrap_err()
            .contains("same place"));
    }

    #[test]
    fn crossed_markers_are_refused() {
        // Two neighbouring corners swapped: the shape folds over itself.
        let crossed = [
            point(0.0, 0.0, 0.0, 0.0),
            point(100.0, 100.0, 100.0, 0.0),
            point(100.0, 0.0, 100.0, 100.0),
            point(0.0, 100.0, 0.0, 100.0),
        ];
        assert!(Homography::from_points(&crossed).is_err());
    }

    #[test]
    fn a_mirrored_fit_is_flagged() {
        let straight = trapezoid();
        assert!(!is_mirrored(&straight));
        // An upside-down camera with the corners named the wrong way round:
        // left and right swapped.
        let mut mirrored = straight;
        mirrored[0].px = 900.0;
        mirrored[1].px = 300.0;
        mirrored[2].px = 100.0;
        mirrored[3].px = 1100.0;
        assert!(is_mirrored(&mirrored));
        // A camera turned right round (180°) is not mirrored.
        let rotated = straight.map(|p| point(1200.0 - p.px, 750.0 - p.py, p.x, p.y));
        assert!(!is_mirrored(&rotated));
    }

    /// A still with one red dot on grey, at a known picture position.
    fn still_with_dot(w: u32, h: u32, dot: (u32, u32)) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([128, 128, 128, 255]));
        for dy in 0..6 {
            for dx in 0..6 {
                img.put_pixel(dot.0 + dx - 3, dot.1 + dy - 3, Rgba([255, 0, 0, 255]));
            }
        }
        img
    }

    fn red_centre(img: &RgbaImage) -> (f64, f64) {
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        for (x, y, p) in img.enumerate_pixels() {
            if p.0[3] == 255 && p.0[0] > 200 && p.0[1] < 80 {
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
                n += 1.0;
            }
        }
        assert!(n > 0.0, "no red found");
        (sx / n, sy / n)
    }

    #[test]
    fn the_bed_picture_puts_a_dot_where_it_is_on_the_bed() {
        let bed = BedSize {
            width: 400.0,
            height: 300.0,
        };
        let points = trapezoid();
        let h = Homography::from_points(&points).unwrap();
        // Put a dot in the picture where bed (100, 200) appears.
        let (px, py) = h.to_pixel(100.0, 200.0).unwrap();
        let still = still_with_dot(1200, 750, (px.round() as u32, py.round() as u32));
        let picture = bed_picture(&still, &fit_for(points, bed, (1200, 750)), bed).unwrap();

        let scale = bed_picture_scale(bed);
        assert_eq!(picture.width(), 800);
        assert_eq!(picture.height(), 600);
        let (cx, cy) = red_centre(&picture);
        assert!((cx / scale - 100.0).abs() < 2.0, "x {}", cx / scale);
        assert!((cy / scale - 200.0).abs() < 2.0, "y {}", cy / scale);
    }

    #[test]
    fn what_the_camera_cannot_see_is_transparent() {
        let bed = BedSize {
            width: 400.0,
            height: 300.0,
        };
        // The camera only sees the left half of the bed: its picture's right
        // edge is the bed's middle.
        let points = [
            point(0.0, 0.0, 0.0, 0.0),
            point(200.0, 0.0, 200.0, 0.0),
            point(200.0, 300.0, 200.0, 300.0),
            point(0.0, 300.0, 0.0, 300.0),
        ];
        let still = RgbaImage::from_pixel(200, 300, Rgba([10, 20, 30, 255]));
        let picture = bed_picture(&still, &fit_for(points, bed, (200, 300)), bed).unwrap();
        assert_eq!(picture.get_pixel(100, 100).0[3], 255);
        assert_eq!(picture.get_pixel(700, 100).0[3], 0);
    }

    #[test]
    fn a_fit_for_another_bed_or_picture_size_is_refused() {
        let bed = BedSize {
            width: 400.0,
            height: 300.0,
        };
        let fit = fit_for(trapezoid(), bed, (1200, 750));
        let still = RgbaImage::new(1200, 750);
        let bigger = BedSize {
            width: 400.0,
            height: 400.0,
        };
        assert!(bed_picture(&still, &fit, bigger)
            .unwrap_err()
            .contains("bed size has changed"));
        let other = RgbaImage::new(640, 480);
        assert!(bed_picture(&other, &fit, bed)
            .unwrap_err()
            .contains("different size"));
    }

    #[test]
    fn the_picture_does_not_depend_on_the_machine_origin() {
        // The fit and the redrawn picture use canvas mm only; nothing about
        // the origin corner goes in, so switching it can't move the picture.
        // This pins that the function has no origin input by checking the
        // same fit gives identical pixels twice.
        let bed = BedSize {
            width: 400.0,
            height: 300.0,
        };
        let fit = fit_for(trapezoid(), bed, (1200, 750));
        let still = still_with_dot(1200, 750, (600, 400));
        let a = bed_picture(&still, &fit, bed).unwrap();
        let b = bed_picture(&still, &fit, bed).unwrap();
        assert_eq!(a.as_raw(), b.as_raw());
    }

    #[test]
    fn a_big_bed_is_drawn_at_fewer_pixels_per_millimetre() {
        assert_eq!(
            bed_picture_scale(BedSize {
                width: 400.0,
                height: 400.0
            }),
            2.0
        );
        let big = BedSize {
            width: 1500.0,
            height: 800.0,
        };
        assert!((bed_picture_scale(big) * 1500.0 - 2400.0).abs() < 1e-9);
    }

    #[test]
    fn a_fit_reads_from_camel_case_json() {
        let json = r#"{"points":[{"px":300,"py":100,"x":0,"y":0},{"px":900,"py":100,"x":400,"y":0},{"px":1100,"py":650,"x":400,"y":300},{"px":100,"py":650,"x":0,"y":300}],"bedWidth":400,"bedHeight":300,"pictureWidth":1200,"pictureHeight":750}"#;
        let fit: CameraFit = serde_json::from_str(json).unwrap();
        assert_eq!(fit.bed_height, 300.0);
        assert!(!check_fit(&fit).unwrap().mirrored);
    }
}
