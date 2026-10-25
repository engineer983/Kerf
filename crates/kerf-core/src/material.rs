//! Finding the piece of material in a camera picture of the bed.
//!
//! The still is redrawn as the bed seen from above (see [`crate::camera`]),
//! then a region is grown out from one spot on the material for as long as
//! the colour stays close and the surface stays plain. Honeycomb, slats and
//! clamps are busy, while plywood, MDF, acrylic and card are plain, so the
//! growth stops at the material's edge. The smallest rectangle round the
//! region is the material's outline.
//!
//! The answer is only ever a proposal, with plain-English warnings about
//! anything doubtful. The person checks it before it is used.
//!
//! Everything is in canvas bed millimetres: X from the left, Y down from the
//! back. Nothing here panics on odd input, because in the browser a panic
//! can't be recovered.

use std::collections::VecDeque;

use kerf_trace::image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::camera::{self, BedSize, CameraFit, Homography};

/// Longest side, in pixels, of the redrawn bed. 1 px per mm is enough for a
/// 1 mm answer; very large beds get fewer.
const LONGEST_PX: f64 = 1600.0;
/// Pixels per mm of the redrawn bed, before the cap above.
const PX_PER_MM: f64 = 1.0;
/// Side of the window the surface's texture is measured over, in mm.
const TEXTURE_WINDOW_MM: f64 = 5.0;
/// A starting spot busier than this (spread of brightness, 0 to 255) is not
/// plain material.
const PLAIN_SEED: f32 = 15.0;
/// The most texture the region may grow over, however busy the seed's
/// surroundings are.
const BUSY: f32 = 25.0;
/// The least difference in colour the region allows (0 to about 440).
const COLOUR_SLACK: f32 = 28.0;
/// Radius of the neighbourhood the seed's own limits come from, in mm.
const SEED_RADIUS_MM: f64 = 3.0;
/// Spacing of the spots tried when looking for a plain place to start, in mm.
const GRID_MM: f64 = 10.0;
/// Size of the smoothing of the outline (close, then open), in mm.
const TIDY_MM: f64 = 3.0;
/// The smallest material that is believed, in mm on each side.
const MIN_SIDE_MM: f64 = 20.0;
/// A region covering more than this share of what the camera sees is the bed.
const MAX_SHARE: f64 = 0.9;
/// A region filling less of its rectangle than this isn't rectangular.
const RECTANGLE_FILL: f64 = 0.9;
/// The share of its square a circle fills is π/4; this is how close counts.
const ROUND_FILL: (f64, f64) = (0.72, 0.84);
/// A round region fills nearly all of its own outline (its convex hull).
const ROUND_SOLIDITY: f64 = 0.95;
/// Angles above this, in degrees, are reported.
const ANGLE_WARN_DEG: f64 = 1.0;
/// Sizes further than this from the typed ones, in mm, are reported.
const SIZE_SLACK_MM: f64 = 10.0;
/// Material thicker than this, in mm, looks shifted in the picture.
const THICK_MM: f64 = 3.0;
/// Rectangles round the material within this share of the smallest count as
/// the same size. For a 300 × 200 mm sheet it is about a tenth of a degree.
const NEAR_TIE: f64 = 0.005;

const REFUSED: &str =
    "Kerf couldn't tell the material from the bed. Click on the material to help.";

/// A rectangle on the bed, in mm.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct BedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the person has already said about the material, for choosing where
/// to start and for checking the answer.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hints {
    /// Where the material is thought to be now.
    pub stock: Option<BedRect>,
    /// Its thickness, in mm.
    pub thickness: f64,
}

/// A [`find_material_request`] as the interface sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindMaterialRequest {
    pub fit: CameraFit,
    pub bed: BedSize,
    /// A place the person clicked on the material, in bed mm.
    #[serde(default)]
    pub seed: Option<(f64, f64)>,
    #[serde(default)]
    pub stock: Option<BedRect>,
    #[serde(default)]
    pub thickness: f64,
}

/// The material found in the picture, in canvas bed mm.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundMaterial {
    /// Back left, back right, front right, front left (before any turn).
    pub corners: [(f64, f64); 4],
    pub centre: (f64, f64),
    /// Along the side nearest to left-to-right.
    pub width: f64,
    /// Along the side nearest to back-to-front.
    pub length: f64,
    /// How far the material is turned, clockwise on screen, in degrees.
    pub angle_deg: f64,
    /// The largest rectangle square to the bed that fits inside the outline,
    /// round the same centre: what is used when the material is at an angle.
    pub straight_width: f64,
    pub straight_length: f64,
    pub warnings: Vec<String>,
}

/// Checks the request against the still, as for the bed picture, then finds
/// the material.
pub fn find_material_request(
    still: &RgbaImage,
    request: &FindMaterialRequest,
) -> Result<FoundMaterial, String> {
    let bed = request.bed;
    if (request.fit.bed_width - bed.width).abs() > 0.01
        || (request.fit.bed_height - bed.height).abs() > 0.01
    {
        return Err(
            "The bed size has changed since the camera was lined up. Line the camera up again."
                .into(),
        );
    }
    if still.width() != request.fit.picture_width || still.height() != request.fit.picture_height {
        return Err("This picture is a different size from the one the camera was lined up with. Line the camera up again.".into());
    }
    let mapping = Homography::from_points(&request.fit.points)?;
    find_material(
        still,
        &mapping,
        bed,
        request.seed,
        &Hints {
            stock: request.stock,
            thickness: request.thickness,
        },
    )
}

/// Finds the material in `still`, starting from `seed` (bed mm) if given.
pub fn find_material(
    still: &RgbaImage,
    fit: &Homography,
    bed: BedSize,
    seed: Option<(f64, f64)>,
    hints: &Hints,
) -> Result<FoundMaterial, String> {
    if !(bed.width.is_finite() && bed.height.is_finite()) || bed.width <= 0.0 || bed.height <= 0.0 {
        return Err("The bed needs a size before the material can be found.".into());
    }
    if still.width() == 0 || still.height() == 0 {
        return Err("The camera's picture is empty. Take a new one.".into());
    }
    let scale = PX_PER_MM.min(LONGEST_PX / bed.width.max(bed.height));
    let grid = Grid::warp(still, fit, bed, scale);
    if grid.valid_count == 0 {
        return Err("The camera can't see any of the bed. Line the camera up again.".into());
    }

    let texture = grid.texture(((TEXTURE_WINDOW_MM * scale) / 2.0).floor().max(1.0) as usize);
    let colour = grid.blurred();

    let start = match seed {
        Some((x, y)) => {
            let at = grid
                .pixel_at(x, y)
                .ok_or("That spot is off the bed. Click on the material.")?;
            if !grid.valid[at] {
                return Err("The camera can't see that spot. Click on the material where the camera can see it.".into());
            }
            at
        }
        None => grid.choose_seed(&texture, hints.stock).ok_or(REFUSED)?,
    };
    if texture[start] > PLAIN_SEED {
        return Err(REFUSED.into());
    }

    let mut region = grid.grow(start, &texture, &colour, scale);
    grid.tidy(&mut region, ((TIDY_MM * scale).round() as usize).max(1));
    let region = grid.component(&region, start).ok_or(REFUSED)?;

    let area_px = region.iter().filter(|&&r| r).count();
    if area_px as f64 > MAX_SHARE * grid.valid_count as f64 {
        return Err(REFUSED.into());
    }

    let points = grid.row_ends(&region, scale);
    let hull = convex_hull(points);
    let Some(mut rect) = smallest_rectangle(&hull) else {
        return Err(REFUSED.into());
    };
    // The hull runs through pixel centres; the material reaches half a pixel further.
    let (mut width, mut length) = (rect.width + 1.0 / scale, rect.length + 1.0 / scale);
    if width < MIN_SIDE_MM || length < MIN_SIDE_MM {
        return Err(REFUSED.into());
    }

    let mut warnings = Vec::new();
    if grid.touches_edge(&region) {
        warnings
            .push("Part of the material is outside the picture, so that edge is a guess.".into());
    }
    let area_mm = area_px as f64 / (scale * scale);
    let fill = area_mm / (width * length);
    let solidity = area_mm / polygon_area(&hull).max(1e-9);
    if fill < RECTANGLE_FILL {
        if (ROUND_FILL.0..=ROUND_FILL.1).contains(&fill) && solidity > ROUND_SOLIDITY {
            // Any square fits round a circle about as well, so the one
            // square to the bed is used: it isn't turned at all.
            rect = bounding_box(&hull);
            (width, length) = (rect.width + 1.0 / scale, rect.length + 1.0 / scale);
            warnings.push(
                "It looks round. The outline is the square around it, so its middle is right but its corners are off the material.".into(),
            );
        } else {
            warnings.push("It doesn't look like a rectangle. Check the outline.".into());
        }
    }
    if rect.angle_deg.abs() > ANGLE_WARN_DEG {
        warnings.push(format!(
            "The material is at an angle of {}°.",
            degrees(rect.angle_deg.abs())
        ));
    }
    if let Some(stock) = hints.stock {
        if (width - stock.width).abs() > SIZE_SLACK_MM
            || (length - stock.height).abs() > SIZE_SLACK_MM
        {
            warnings.push(format!(
                "It looks {} × {} mm, but the material is set to {} × {} mm.",
                width.round(),
                length.round(),
                stock.width.round(),
                stock.height.round()
            ));
        }
    }
    // No camera height is known yet, so the shift can't be corrected.
    if hints.thickness > THICK_MM {
        warnings.push(
            "Thick material looks shifted in the camera picture, so check the far edges.".into(),
        );
    }

    let (straight_width, straight_length) = straight_inside(width, length, rect.angle_deg);
    Ok(FoundMaterial {
        corners: corners(rect.centre, width, length, rect.angle_deg),
        centre: rect.centre,
        width,
        length,
        angle_deg: rect.angle_deg,
        straight_width,
        straight_length,
        warnings,
    })
}

/// An angle for a message: whole degrees, or tenths below 2°.
fn degrees(angle: f64) -> String {
    if angle < 2.0 {
        format!("{angle:.1}")
    } else {
        format!("{angle:.0}")
    }
}

/// The corners of a turned rectangle: back left, back right, front right,
/// front left, before the turn.
pub fn corners(centre: (f64, f64), width: f64, length: f64, angle_deg: f64) -> [(f64, f64); 4] {
    let (s, c) = angle_deg.to_radians().sin_cos();
    let (u, v) = ((c, s), (-s, c));
    let at = |a: f64, b: f64| {
        (
            centre.0 + a * width / 2.0 * u.0 + b * length / 2.0 * v.0,
            centre.1 + a * width / 2.0 * u.1 + b * length / 2.0 * v.1,
        )
    };
    [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]
}

/// The largest rectangle square to the bed, round the same centre, that fits
/// inside a `width` × `length` rectangle turned by `angle_deg`.
pub fn straight_inside(width: f64, length: f64, angle_deg: f64) -> (f64, f64) {
    let (s, c) = {
        let (s, c) = angle_deg.to_radians().sin_cos();
        (s.abs(), c.abs())
    };
    let (hw, hl) = (width / 2.0, length / 2.0);
    if s < 1e-9 {
        return (width, length);
    }
    if c < 1e-9 {
        return (length, width);
    }
    // Half-sides a (across) and b (down) must satisfy, measured along the
    // turned rectangle's own sides: a·c + b·s ≤ hw and a·s + b·c ≤ hl.
    let fits = |a: f64, b: f64| {
        a >= 0.0 && b >= 0.0 && a * c + b * s <= hw + 1e-9 && a * s + b * c <= hl + 1e-9
    };
    let mut best = (0.0, 0.0);
    let mut consider = |a: f64, b: f64| {
        if fits(a, b) && a * b > best.0 * best.1 {
            best = (a, b);
        }
    };
    // The best on each limit alone, and where the two limits meet.
    consider(hw / (2.0 * c), hw / (2.0 * s));
    consider(hl / (2.0 * s), hl / (2.0 * c));
    let det = c * c - s * s;
    if det.abs() > 1e-12 {
        consider((hw * c - hl * s) / det, (hl * c - hw * s) / det);
    }
    (best.0 * 2.0, best.1 * 2.0)
}

/// The redrawn bed: one cell per pixel, row by row from the back-left corner.
struct Grid {
    width: usize,
    height: usize,
    /// Colour, 0 to 255 for each of red, green and blue.
    rgb: Vec<[f32; 3]>,
    /// Whether the camera can see the cell.
    valid: Vec<bool>,
    valid_count: usize,
    scale: f64,
}

impl Grid {
    fn warp(still: &RgbaImage, fit: &Homography, bed: BedSize, scale: f64) -> Self {
        let width = ((bed.width * scale).round() as usize).max(1);
        let height = ((bed.height * scale).round() as usize).max(1);
        let mut rgb = vec![[0.0; 3]; width * height];
        let mut valid = vec![false; width * height];
        let (in_w, in_h) = (still.width() as f64, still.height() as f64);
        for oy in 0..height {
            let y_mm = (oy as f64 + 0.5) / scale;
            for ox in 0..width {
                let x_mm = (ox as f64 + 0.5) / scale;
                let Some((px, py)) = fit.to_pixel(x_mm, y_mm) else {
                    continue;
                };
                let (sx, sy) = (px - 0.5, py - 0.5);
                if !(sx >= -0.5 && sy >= -0.5 && sx <= in_w - 0.5 && sy <= in_h - 0.5) {
                    continue;
                }
                // A see-through part of the still is something the camera didn't see.
                let nearest = still.get_pixel(
                    (px.floor().max(0.0) as u32).min(still.width() - 1),
                    (py.floor().max(0.0) as u32).min(still.height() - 1),
                );
                if nearest.0[3] < 128 {
                    continue;
                }
                let p = camera::sample(still, sx, sy).0;
                let i = oy * width + ox;
                rgb[i] = [p[0] as f32, p[1] as f32, p[2] as f32];
                valid[i] = true;
            }
        }
        let valid_count = valid.iter().filter(|&&v| v).count();
        Self {
            width,
            height,
            rgb,
            valid,
            valid_count,
            scale,
        }
    }

    fn pixel_at(&self, x: f64, y: f64) -> Option<usize> {
        let (px, py) = (x * self.scale, y * self.scale);
        if !(px >= 0.0 && py >= 0.0 && px < self.width as f64 && py < self.height as f64) {
            return None;
        }
        Some(py as usize * self.width + px as usize)
    }

    /// Sums over the visible cells, as running totals so any window is quick.
    fn integral(&self, value: impl Fn(usize) -> f64) -> Vec<f64> {
        let w = self.width + 1;
        let mut sums = vec![0.0; w * (self.height + 1)];
        for y in 0..self.height {
            let mut row = 0.0;
            for x in 0..self.width {
                let i = y * self.width + x;
                if self.valid[i] {
                    row += value(i);
                }
                sums[(y + 1) * w + x + 1] = sums[y * w + x + 1] + row;
            }
        }
        sums
    }

    /// The window of radius `r` round a cell, clipped to the grid: (x0, y0, x1, y1), end exclusive.
    fn window(&self, i: usize, r: usize) -> (usize, usize, usize, usize) {
        let (x, y) = (i % self.width, i / self.width);
        (
            x.saturating_sub(r),
            y.saturating_sub(r),
            (x + r + 1).min(self.width),
            (y + r + 1).min(self.height),
        )
    }

    fn window_sum(&self, sums: &[f64], (x0, y0, x1, y1): (usize, usize, usize, usize)) -> f64 {
        let w = self.width + 1;
        sums[y1 * w + x1] - sums[y0 * w + x1] - sums[y1 * w + x0] + sums[y0 * w + x0]
    }

    /// For each cell, the spread of brightness in the window round it.
    fn texture(&self, r: usize) -> Vec<f32> {
        let luma = |i: usize| {
            let [r, g, b] = self.rgb[i];
            (0.299 * r + 0.587 * g + 0.114 * b) as f64
        };
        let count = self.integral(|_| 1.0);
        let sum = self.integral(luma);
        let squares = self.integral(|i| luma(i) * luma(i));
        (0..self.width * self.height)
            .map(|i| {
                if !self.valid[i] {
                    return f32::INFINITY;
                }
                let window = self.window(i, r);
                let n = self.window_sum(&count, window).max(1.0);
                let mean = self.window_sum(&sum, window) / n;
                let spread = (self.window_sum(&squares, window) / n - mean * mean).max(0.0);
                spread.sqrt() as f32
            })
            .collect()
    }

    /// The colours lightly blurred (3 × 3, visible cells only), so speckle
    /// doesn't stop the region.
    fn blurred(&self) -> Vec<[f32; 3]> {
        let count = self.integral(|_| 1.0);
        let channels: Vec<Vec<f64>> = (0..3)
            .map(|c| self.integral(|i| self.rgb[i][c] as f64))
            .collect();
        (0..self.width * self.height)
            .map(|i| {
                if !self.valid[i] {
                    return [0.0; 3];
                }
                let window = self.window(i, 1);
                let n = self.window_sum(&count, window).max(1.0);
                [0, 1, 2].map(|c| (self.window_sum(&channels[c], window) / n) as f32)
            })
            .collect()
    }

    /// A plain place to start: the middle of where the material is thought
    /// to be if that is plain, or else the plainest spot on a 10 mm grid,
    /// nearest the middle of the bed when there are several.
    fn choose_seed(&self, texture: &[f32], stock: Option<BedRect>) -> Option<usize> {
        if let Some(stock) = stock {
            let at = self.pixel_at(stock.x + stock.width / 2.0, stock.y + stock.height / 2.0);
            if let Some(at) = at.filter(|&at| self.valid[at] && texture[at] <= PLAIN_SEED) {
                return Some(at);
            }
        }
        let step = ((GRID_MM * self.scale).round() as usize).max(1);
        let (mx, my) = (self.width as f64 / 2.0, self.height as f64 / 2.0);
        let mut best: Option<(f32, f64, usize)> = None;
        for y in (step / 2..self.height).step_by(step) {
            for x in (step / 2..self.width).step_by(step) {
                let i = y * self.width + x;
                if !self.valid[i] || texture[i] > PLAIN_SEED {
                    continue;
                }
                // Within one level of brightness counts as equally plain.
                let plain = texture[i].round();
                let distance = (x as f64 - mx).hypot(y as f64 - my);
                if best.map_or(true, |(p, d, _)| plain < p || (plain == p && distance < d)) {
                    best = Some((plain, distance, i));
                }
            }
        }
        best.map(|(_, _, i)| i)
    }

    /// The four neighbours of a cell that are on the grid.
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> {
        let (x, y, w, h) = (i % self.width, i / self.width, self.width, self.height);
        [
            (x > 0).then(|| i - 1),
            (x + 1 < w).then(|| i + 1),
            (y > 0).then(|| i - w),
            (y + 1 < h).then(|| i + w),
        ]
        .into_iter()
        .flatten()
    }

    /// Grows the region from `start`, one neighbour at a time. A queue, not
    /// recursion: the browser's stack is small.
    fn grow(&self, start: usize, texture: &[f32], colour: &[[f32; 3]], scale: f64) -> Vec<bool> {
        let n = self.width * self.height;
        // The limits come from the seed's own neighbourhood.
        let r = ((SEED_RADIUS_MM * scale).round() as usize).max(1);
        let (x0, y0, x1, y1) = self.window(start, r);
        let mut near = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * self.width + x;
                if self.valid[i] && texture[i] <= PLAIN_SEED {
                    near.push(i);
                }
            }
        }
        if near.is_empty() {
            near.push(start);
        }
        let count = near.len() as f32;
        let mean_texture = near.iter().map(|&i| texture[i]).sum::<f32>() / count;
        let mut mean = [0.0f32; 3];
        for &i in &near {
            for c in 0..3 {
                mean[c] += colour[i][c] / count;
            }
        }
        let spread = (near
            .iter()
            .map(|&i| distance2(colour[i], mean))
            .sum::<f32>()
            / count)
            .sqrt();
        let texture_limit = (mean_texture * 2.0 + 6.0).min(BUSY);
        let colour_limit = (spread * 4.0).max(COLOUR_SLACK);

        let mut region = vec![false; n];
        let mut sum = [0.0f64; 3];
        let mut size = 0.0f64;
        let mut queue = VecDeque::from([start]);
        region[start] = true;
        while let Some(i) = queue.pop_front() {
            for c in 0..3 {
                sum[c] += colour[i][c] as f64;
            }
            size += 1.0;
            let running = [0, 1, 2].map(|c| (sum[c] / size) as f32);
            for j in self.neighbours(i) {
                if region[j] || !self.valid[j] || texture[j] > texture_limit {
                    continue;
                }
                if distance2(colour[j], running) > colour_limit * colour_limit {
                    continue;
                }
                region[j] = true;
                queue.push_back(j);
            }
        }

        // The texture window straddles the edge, so the growth stops a
        // little short of it. Take in the last few cells by colour alone,
        // using the colours before blurring, which are sharp at the edge.
        let mean = [0, 1, 2].map(|c| (sum[c] / size.max(1.0)) as f32);
        let reach = ((TEXTURE_WINDOW_MM * scale) / 2.0).ceil() as usize + 2;
        let mut edge: VecDeque<(usize, usize)> =
            (0..n).filter(|&i| region[i]).map(|i| (i, 0)).collect();
        while let Some((i, depth)) = edge.pop_front() {
            if depth >= reach {
                continue;
            }
            for j in self.neighbours(i) {
                if region[j] || !self.valid[j] {
                    continue;
                }
                if distance2(self.rgb[j], mean) > colour_limit * colour_limit {
                    continue;
                }
                region[j] = true;
                edge.push_back((j, depth + 1));
            }
        }
        region
    }

    /// Fills holes (knots, old engraving, a pencil mark), then smooths the
    /// outline: closes small gaps, then opens away thin strands.
    fn tidy(&self, region: &mut [bool], r: usize) {
        // Anything that can't be reached from the edge of the grid without
        // crossing the region is a hole in it.
        let n = self.width * self.height;
        let mut outside = vec![false; n];
        let mut queue = VecDeque::new();
        for i in 0..n {
            let (x, y) = (i % self.width, i / self.width);
            let border = x == 0 || y == 0 || x + 1 == self.width || y + 1 == self.height;
            if border && !region[i] {
                outside[i] = true;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            for j in self.neighbours(i) {
                if !outside[j] && !region[j] {
                    outside[j] = true;
                    queue.push_back(j);
                }
            }
        }
        for i in 0..n {
            region[i] = !outside[i];
        }

        let grown = self.spread(region, r, true);
        let closed = self.spread(&grown, r, false);
        let shrunk = self.spread(&closed, r, false);
        let opened = self.spread(&shrunk, r, true);
        for i in 0..n {
            region[i] = opened[i] && self.valid[i];
        }
    }

    /// A square of radius `r` passed over the region: with `grow`, a cell is
    /// in if any cell under the square is; otherwise only if all are. The
    /// square is cut off at the grid's edge.
    fn spread(&self, region: &[bool], r: usize, grow: bool) -> Vec<bool> {
        let pass = |input: &[bool], across: bool| -> Vec<bool> {
            let (lines, len) = if across {
                (self.height, self.width)
            } else {
                (self.width, self.height)
            };
            let at = |line: usize, k: usize| {
                if across {
                    line * self.width + k
                } else {
                    k * self.width + line
                }
            };
            let mut out = vec![false; input.len()];
            let mut counts = vec![0usize; len + 1];
            for line in 0..lines {
                for k in 0..len {
                    counts[k + 1] = counts[k] + input[at(line, k)] as usize;
                }
                for k in 0..len {
                    let (a, b) = (k.saturating_sub(r), (k + r + 1).min(len));
                    let set = counts[b] - counts[a];
                    out[at(line, k)] = if grow { set > 0 } else { set == b - a };
                }
            }
            out
        };
        pass(&pass(region, true), false)
    }

    /// The connected part of the region that holds `start`, if it does.
    fn component(&self, region: &[bool], start: usize) -> Option<Vec<bool>> {
        if !region.get(start).copied().unwrap_or(false) {
            return None;
        }
        let mut out = vec![false; region.len()];
        let mut queue = VecDeque::from([start]);
        out[start] = true;
        while let Some(i) = queue.pop_front() {
            for j in self.neighbours(i) {
                if region[j] && !out[j] {
                    out[j] = true;
                    queue.push_back(j);
                }
            }
        }
        Some(out)
    }

    /// The centres of the first and last region cell on each row, in mm:
    /// enough for the convex hull.
    fn row_ends(&self, region: &[bool], scale: f64) -> Vec<(f64, f64)> {
        let mut points = Vec::new();
        for y in 0..self.height {
            let row = &region[y * self.width..(y + 1) * self.width];
            let first = row.iter().position(|&r| r);
            let last = row.iter().rposition(|&r| r);
            if let (Some(a), Some(b)) = (first, last) {
                let cy = (y as f64 + 0.5) / scale;
                points.push(((a as f64 + 0.5) / scale, cy));
                points.push(((b as f64 + 0.5) / scale, cy));
            }
        }
        points
    }

    /// Whether the region reaches the edge of what the camera sees (or of the bed).
    fn touches_edge(&self, region: &[bool]) -> bool {
        (0..region.len()).any(|i| {
            if !region[i] {
                return false;
            }
            let (x, y) = (i % self.width, i / self.width);
            x == 0
                || y == 0
                || x + 1 == self.width
                || y + 1 == self.height
                || self.neighbours(i).any(|j| !self.valid[j])
        })
    }
}

fn distance2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|c| (a[c] - b[c]) * (a[c] - b[c])).sum()
}

fn cross(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// The convex hull (monotone chain), going round in order, without repeats.
pub fn convex_hull(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    points.retain(|p| p.0.is_finite() && p.1.is_finite());
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn polygon_area(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

/// A turned rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    pub centre: (f64, f64),
    pub width: f64,
    pub length: f64,
    /// Clockwise on screen, from -45° to 45°.
    pub angle_deg: f64,
}

/// The smallest rectangle round a convex hull. One of its sides always lies
/// along a side of the hull, so each side is tried (rotating calipers).
pub fn smallest_rectangle(hull: &[(f64, f64)]) -> Option<Rectangle> {
    if hull.len() < 3 {
        return None;
    }
    let mut candidates: Vec<(f64, Rectangle)> = Vec::new();
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        if length < 1e-12 {
            continue;
        }
        let u = ((b.0 - a.0) / length, (b.1 - a.1) / length);
        let v = (-u.1, u.0);
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in hull {
            let pu = p.0 * u.0 + p.1 * u.1;
            let pv = p.0 * v.0 + p.1 * v.1;
            u0 = u0.min(pu);
            u1 = u1.max(pu);
            v0 = v0.min(pv);
            v1 = v1.max(pv);
        }
        let (cu, cv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
        let centre = (cu * u.0 + cv * v.0, cu * u.1 + cv * v.1);
        candidates.push((
            (u1 - u0) * (v1 - v0),
            straighten(centre, u1 - u0, v1 - v0, u.1.atan2(u.0).to_degrees()),
        ));
    }
    let smallest = candidates
        .iter()
        .map(|(area, _)| *area)
        .fold(f64::INFINITY, f64::min);
    // A round or ragged piece has many rectangles of almost the same size;
    // the one least turned is the most useful.
    candidates
        .into_iter()
        .filter(|(area, _)| *area <= smallest * (1.0 + NEAR_TIE))
        .min_by(|a, b| a.1.angle_deg.abs().total_cmp(&b.1.angle_deg.abs()))
        .map(|(_, rect)| rect)
}

/// The rectangle square to the bed round some points.
fn bounding_box(points: &[(f64, f64)]) -> Rectangle {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in points {
        (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
    }
    Rectangle {
        centre: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
        width: x1 - x0,
        length: y1 - y0,
        angle_deg: 0.0,
    }
}

/// Turns the description of a rectangle round so its angle is within ±45°,
/// swapping its sides as needed: width is then the side nearest to
/// left-to-right.
fn straighten(centre: (f64, f64), width: f64, length: f64, angle_deg: f64) -> Rectangle {
    // A rectangle looks the same turned by 180°.
    let mut angle = angle_deg.rem_euclid(180.0);
    let (mut width, mut length) = (width, length);
    if angle > 135.0 {
        angle -= 180.0;
    } else if angle > 45.0 {
        angle -= 90.0;
        std::mem::swap(&mut width, &mut length);
    }
    Rectangle {
        centre,
        width,
        length,
        angle_deg: angle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::FitPoint;
    use kerf_trace::image::Rgba;

    /// Pixels per mm of the synthetic camera pictures.
    const K: f64 = 2.0;
    const BED: BedSize = BedSize {
        width: 400.0,
        height: 300.0,
    };
    const SHEET: [u8; 3] = [205, 170, 120];

    /// A camera looking straight down, `K` pixels per mm, seeing the bed
    /// from (0, 0) to (`seen_width`, `seen_height`) mm.
    fn straight_fit(seen_width: f64, seen_height: f64) -> Homography {
        let p = |x: f64, y: f64| FitPoint {
            px: x * K,
            py: y * K,
            x,
            y,
        };
        Homography::from_points(&[
            p(0.0, 0.0),
            p(seen_width, 0.0),
            p(seen_width, seen_height),
            p(0.0, seen_height),
        ])
        .unwrap()
    }

    /// A picture of a honeycomb-like bed (a 4 mm checker of dark and light)
    /// with things drawn over it: each says the colour at a bed position, or
    /// none for the bed.
    fn picture(
        seen_width: f64,
        seen_height: f64,
        things: &[&dyn Fn(f64, f64) -> Option<[u8; 3]>],
    ) -> RgbaImage {
        RgbaImage::from_fn(
            (seen_width * K) as u32,
            (seen_height * K) as u32,
            |px, py| {
                let (x, y) = ((px as f64 + 0.5) / K, (py as f64 + 0.5) / K);
                for thing in things {
                    if let Some([r, g, b]) = thing(x, y) {
                        // A little speckle, as a real camera has.
                        let n = ((px * 7 + py * 13) % 7) as u8;
                        return Rgba([
                            r.saturating_add(n),
                            g.saturating_add(n),
                            b.saturating_add(n),
                            255,
                        ]);
                    }
                }
                let dark = ((x / 4.0).floor() as i64 + (y / 4.0).floor() as i64) % 2 == 0;
                if dark {
                    Rgba([40, 40, 45, 255])
                } else {
                    Rgba([225, 225, 230, 255])
                }
            },
        )
    }

    /// A rectangle turned by `angle` round its centre.
    fn sheet(
        centre: (f64, f64),
        width: f64,
        length: f64,
        angle: f64,
        colour: [u8; 3],
    ) -> impl Fn(f64, f64) -> Option<[u8; 3]> {
        let (s, c) = angle.to_radians().sin_cos();
        move |x, y| {
            let (dx, dy) = (x - centre.0, y - centre.1);
            let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
            (u.abs() <= width / 2.0 && v.abs() <= length / 2.0).then_some(colour)
        }
    }

    fn find(
        still: &RgbaImage,
        fit: &Homography,
        seed: Option<(f64, f64)>,
    ) -> Result<FoundMaterial, String> {
        find_material(still, fit, BED, seed, &Hints::default())
    }

    fn near(a: (f64, f64), b: (f64, f64), within: f64) -> bool {
        (a.0 - b.0).hypot(a.1 - b.1) <= within
    }

    #[test]
    fn a_square_sheet_on_honeycomb_is_found_within_a_millimetre() {
        let rect = sheet((200.0, 150.0), 300.0, 200.0, 0.0, SHEET);
        let still = picture(400.0, 300.0, &[&rect]);
        let found = find(&still, &straight_fit(400.0, 300.0), None).unwrap();
        let expected = [(50.0, 50.0), (350.0, 50.0), (350.0, 250.0), (50.0, 250.0)];
        for (got, want) in found.corners.iter().zip(expected) {
            assert!(near(*got, want, 1.0), "corner {got:?}, expected {want:?}");
        }
        assert!(found.angle_deg.abs() < 0.5, "angle {}", found.angle_deg);
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    }

    #[test]
    fn a_turned_sheet_gives_its_angle_and_centre() {
        let rect = sheet((210.0, 140.0), 300.0, 200.0, 5.0, SHEET);
        let still = picture(400.0, 300.0, &[&rect]);
        let found = find(&still, &straight_fit(400.0, 300.0), None).unwrap();
        assert!(
            (found.angle_deg - 5.0).abs() < 0.5,
            "angle {}",
            found.angle_deg
        );
        assert!(
            near(found.centre, (210.0, 140.0), 1.0),
            "centre {:?}",
            found.centre
        );
        assert!((found.width - 300.0).abs() < 2.0 && (found.length - 200.0).abs() < 2.0);
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("at an angle of 5°")),
            "{:?}",
            found.warnings
        );
        // What would be used is square to the bed and inside the sheet.
        assert!(found.straight_width < 300.0 && found.straight_length < 200.0);
    }

    #[test]
    fn a_seed_on_the_bed_is_refused_rather_than_merged() {
        let rect = sheet((200.0, 150.0), 300.0, 200.0, 0.0, SHEET);
        let still = picture(400.0, 300.0, &[&rect]);
        let result = find(&still, &straight_fit(400.0, 300.0), Some((20.0, 20.0)));
        match result {
            Err(e) => assert!(e.contains("Click on the material"), "{e}"),
            Ok(found) => assert!(found.width < 60.0 && found.length < 60.0, "{found:?}"),
        }
    }

    #[test]
    fn a_clamp_of_another_colour_is_left_out() {
        let rect = sheet((200.0, 150.0), 300.0, 200.0, 0.0, SHEET);
        let clamp = sheet((365.0, 150.0), 30.0, 20.0, 0.0, [120, 120, 125]);
        let still = picture(400.0, 300.0, &[&rect, &clamp]);
        let found = find(&still, &straight_fit(400.0, 300.0), None).unwrap();
        assert!((found.width - 300.0).abs() < 1.5, "width {}", found.width);
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
    }

    #[test]
    fn a_clamp_of_the_same_colour_is_flagged_by_the_shape() {
        let rect = sheet((180.0, 150.0), 260.0, 200.0, 0.0, SHEET);
        let clamp = sheet((340.0, 150.0), 80.0, 16.0, 0.0, SHEET);
        let still = picture(400.0, 300.0, &[&rect, &clamp]);
        let found = find(&still, &straight_fit(400.0, 300.0), None).unwrap();
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("doesn't look like a rectangle")),
            "{:?}",
            found.warnings
        );
    }

    #[test]
    fn a_sheet_running_out_of_the_cameras_view_is_flagged() {
        // The camera sees only the left 300 mm of the bed.
        let rect = sheet((260.0, 150.0), 200.0, 150.0, 0.0, SHEET);
        let still = picture(300.0, 300.0, &[&rect]);
        let found = find(&still, &straight_fit(300.0, 300.0), Some((220.0, 150.0))).unwrap();
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("outside the picture")),
            "{:?}",
            found.warnings
        );
    }

    #[test]
    fn a_round_piece_has_its_centre_found() {
        let disc = |x: f64, y: f64| ((x - 230.0).hypot(y - 160.0) <= 80.0).then_some(SHEET);
        let still = picture(400.0, 300.0, &[&disc]);
        let found = find(&still, &straight_fit(400.0, 300.0), None).unwrap();
        assert!(
            near(found.centre, (230.0, 160.0), 1.0),
            "centre {:?}",
            found.centre
        );
        assert!(
            found.warnings.iter().any(|w| w.contains("looks round")),
            "{:?}",
            found.warnings
        );
        assert_eq!(
            found.angle_deg, 0.0,
            "a round piece is never reported as turned"
        );
    }

    #[test]
    fn a_picture_all_one_colour_is_refused() {
        let plain = |_: f64, _: f64| Some(SHEET);
        let still = picture(400.0, 300.0, &[&plain]);
        let error = find(&still, &straight_fit(400.0, 300.0), None).unwrap_err();
        assert!(error.contains("Click on the material"), "{error}");
    }

    #[test]
    fn the_size_typed_is_compared_and_thick_material_is_flagged() {
        let rect = sheet((200.0, 150.0), 300.0, 200.0, 0.0, SHEET);
        let still = picture(400.0, 300.0, &[&rect]);
        let hints = Hints {
            stock: Some(BedRect {
                x: 0.0,
                y: 0.0,
                width: 250.0,
                height: 200.0,
            }),
            thickness: 18.0,
        };
        let found = find_material(&still, &straight_fit(400.0, 300.0), BED, None, &hints).unwrap();
        assert!(
            found
                .warnings
                .iter()
                .any(|w| w.contains("300 × 200 mm") && w.contains("250 × 200 mm")),
            "{:?}",
            found.warnings
        );
        assert!(
            found.warnings.iter().any(|w| w.contains("Thick material")),
            "{:?}",
            found.warnings
        );
    }

    #[test]
    fn the_straight_part_stays_inside_and_keeps_its_centre() {
        for &(width, length, angle) in &[
            (300.0, 200.0, 4.0),
            (300.0, 200.0, -12.0),
            (200.0, 300.0, 30.0),
            (100.0, 100.0, 45.0),
            (500.0, 40.0, 3.0),
            (300.0, 200.0, 0.0),
        ] {
            let (w, l) = straight_inside(width, length, angle);
            assert!(w > 0.0 && l > 0.0, "{width} {length} {angle}: {w} {l}");
            if angle == 0.0 {
                assert_eq!((w, l), (width, length));
            }
            // Each corner of the straight part, centred where the turned one is, lies inside it.
            let (s, c) = angle.to_radians().sin_cos();
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let (dx, dy) = (a * w / 2.0, b * l / 2.0);
                let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
                assert!(
                    u.abs() <= width / 2.0 + 1e-6 && v.abs() <= length / 2.0 + 1e-6,
                    "{width} {length} {angle}: corner ({u}, {v})"
                );
            }
        }
    }

    #[test]
    fn odd_pictures_never_panic() {
        let fit = straight_fit(400.0, 300.0);
        assert!(find(&RgbaImage::new(0, 0), &fit, None).is_err());
        assert!(find(&RgbaImage::new(1, 1), &fit, None).is_err());
        assert!(find(&RgbaImage::new(1, 1), &fit, Some((0.1, 0.1))).is_err());
        let clear = RgbaImage::from_pixel(800, 600, Rgba([200, 200, 200, 0]));
        assert!(find(&clear, &fit, None).is_err());
        assert!(find(&clear, &fit, Some((200.0, 150.0))).is_err());
        let still = picture(400.0, 300.0, &[]);
        assert!(find(&still, &fit, Some((-5.0, f64::NAN))).is_err());
        assert!(find_material(
            &still,
            &fit,
            BedSize {
                width: 0.0,
                height: 0.0
            },
            None,
            &Hints::default()
        )
        .is_err());
    }

    #[test]
    fn a_request_for_another_bed_is_refused() {
        let still = picture(400.0, 300.0, &[]);
        let p = |x: f64, y: f64| FitPoint {
            px: x * K,
            py: y * K,
            x,
            y,
        };
        let request = FindMaterialRequest {
            fit: CameraFit {
                points: [p(0.0, 0.0), p(400.0, 0.0), p(400.0, 300.0), p(0.0, 300.0)],
                bed_width: 400.0,
                bed_height: 300.0,
                picture_width: 800,
                picture_height: 600,
            },
            bed: BedSize {
                width: 500.0,
                height: 300.0,
            },
            seed: None,
            stock: None,
            thickness: 3.0,
        };
        assert!(find_material_request(&still, &request)
            .unwrap_err()
            .contains("bed size has changed"));
    }
}
