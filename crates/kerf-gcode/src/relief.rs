//! 3D relief carving from a height map.
//!
//! The bit sweeps the area in parallel rows. At every point its tip is held at
//! the lowest height where no part of the bit is below the wanted surface, so
//! a wide or round bit leaves material in tight valleys rather than gouging
//! the walls beside them. Material is removed in levels of `depth_per_pass`;
//! the last level is the finished surface.

use crate::generate::{num, Writer, SPINDLE_DWELL_S};
use crate::job::{Operation, OperationKind};

/// Heights on a pixel grid: 1.0 is the stock top, 0.0 the deepest cut.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightmap {
    pub width: usize,
    pub height: usize,
    /// Row-major, top row first, `width * height` values from 0.0 to 1.0.
    pub data: Vec<f32>,
    /// Physical size in millimetres.
    pub width_mm: f64,
    pub height_mm: f64,
}

impl Heightmap {
    pub(crate) fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.data.len() == self.width * self.height
            && self.width_mm > 0.0
            && self.height_mm > 0.0
    }

    /// Bilinear sample at a position in millimetres. Positions outside the map
    /// take the nearest edge value, so the carving runs cleanly off its border.
    pub(crate) fn sample(&self, x_mm: f64, y_mm: f64) -> f64 {
        let fx =
            (x_mm / self.width_mm * self.width as f64 - 0.5).clamp(0.0, (self.width - 1) as f64);
        let fy =
            (y_mm / self.height_mm * self.height as f64 - 0.5).clamp(0.0, (self.height - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let at = |x: usize, y: usize| self.data[y * self.width + x] as f64;
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

/// Collinear points closer than this to the simplified path are dropped.
const SIMPLIFY_TOLERANCE_MM: f64 = 0.01;
/// Heights within this distance count as equal when skipping finished rows.
const LEVEL_EPSILON_MM: f64 = 1e-6;

/// The shape of a bit's cutting end.
#[derive(Debug, Clone, Copy)]
struct Bit {
    /// Radius of a flat end, or of the ball at the tip.
    radius: f64,
    ball_nose: bool,
    /// Angle of the sides from the axis in radians; zero for a straight bit.
    taper: f64,
}

impl Bit {
    /// How far above the tip the bit's surface is, at a distance from its axis.
    fn lift(&self, r: f64) -> f64 {
        if !self.ball_nose {
            return 0.0;
        }
        // On a tapered bit the ball meets the cone where their slopes match.
        let tangent = self.radius * self.taper.cos();
        if self.taper > 0.0 && r > tangent {
            self.radius * (1.0 - self.taper.sin()) + (r - tangent) / self.taper.tan()
        } else {
            let r = r.min(self.radius);
            self.radius - (self.radius * self.radius - r * r).sqrt()
        }
    }

    /// The radius beyond which the bit cannot touch a surface at most `depth`
    /// deep: a taper keeps widening, but its upper part stays above the work.
    fn reach(&self, depth: f64, shank_diameter: f64) -> f64 {
        if !self.ball_nose || self.taper <= 0.0 {
            return self.radius;
        }
        let spread = self.radius * self.taper.cos() + depth * self.taper.tan();
        if shank_diameter > 0.0 {
            spread.min(shank_diameter / 2.0).max(self.radius)
        } else {
            spread
        }
    }
}

/// Points on the bit's cutting surface: horizontal offset from its axis, and
/// how far above the tip the surface is there.
fn tool_profile(bit: Bit, radius: f64, spacing: f64) -> Vec<([f64; 2], f64)> {
    let reach = (radius / spacing).ceil() as i64;
    let mut points = Vec::new();
    for iy in -reach..=reach {
        for ix in -reach..=reach {
            let (dx, dy) = (ix as f64 * spacing, iy as f64 * spacing);
            let r = dx.hypot(dy);
            if r <= radius + 1e-9 {
                // Each sample stands for the cell around it, so use the lowest
                // part of the bit over that cell. This errs towards leaving
                // material rather than cutting into a wall.
                points.push(([dx, dy], bit.lift((r - spacing).max(0.0))));
            }
        }
    }
    points
}

/// Ramer–Douglas–Peucker over (x, z); returns the indices to keep.
fn simplify(xs: &[f64], zs: &[f64]) -> Vec<usize> {
    let mut keep = vec![false; xs.len()];
    keep[0] = true;
    keep[xs.len() - 1] = true;
    let mut spans = vec![(0, xs.len() - 1)];
    while let Some((first, last)) = spans.pop() {
        let (ax, az, bx, bz) = (xs[first], zs[first], xs[last], zs[last]);
        let length = (bx - ax).hypot(bz - az);
        let mut worst = (0.0, first);
        for i in first + 1..last {
            let distance = if length < 1e-12 {
                (xs[i] - ax).hypot(zs[i] - az)
            } else {
                ((bx - ax) * (az - zs[i]) - (ax - xs[i]) * (bz - az)).abs() / length
            };
            if distance > worst.0 {
                worst = (distance, i);
            }
        }
        if worst.0 > SIMPLIFY_TOLERANCE_MM {
            keep[worst.1] = true;
            spans.push((first, worst.1));
            spans.push((worst.1, last));
        }
    }
    (0..xs.len()).filter(|&i| keep[i]).collect()
}

pub(crate) fn carve(
    w: &mut Writer,
    operation: &Operation,
    map: &Heightmap,
    offset: [f64; 2],
) -> Result<(), String> {
    let OperationKind::CncRelief {
        depth,
        depth_per_pass,
        plunge_feed,
        safe_z,
        spindle_rpm,
        tool_diameter,
        ball_nose,
        stepover,
        light_is_high,
        taper_angle,
        shank_diameter,
    } = operation.kind
    else {
        return Ok(());
    };
    if !map.is_valid() {
        return Err("The height map for the relief is empty.".into());
    }

    let bit = Bit {
        radius: tool_diameter / 2.0,
        ball_nose,
        taper: taper_angle.clamp(0.0, 60.0).to_radians(),
    };
    let radius = bit.reach(depth, shank_diameter);
    let pixel = map.width_mm / map.width as f64;
    // Fine enough to follow the image, without sampling far below what the bit can resolve.
    let step_x = pixel
        .clamp(tool_diameter / 40.0, tool_diameter / 4.0)
        .max(0.02);
    // Sample the bit's footprint at least twice per pixel, so no pixel under it
    // is missed; the lower bound keeps very fine images affordable.
    let spacing = (pixel / 2.0).min(radius / 8.0).max(radius / 20.0);
    let profile = tool_profile(bit, radius, spacing);

    let columns = (map.width_mm / step_x).ceil() as usize;
    let xs: Vec<f64> = (0..=columns)
        .map(|i| (i as f64 * step_x).min(map.width_mm))
        .collect();
    let row_count = (map.height_mm / stepover).ceil() as usize;
    let ys: Vec<f64> = (0..=row_count)
        .map(|j| (j as f64 * stepover).min(map.height_mm))
        .collect();

    // Wanted surface height (Z, zero at the stock top) at a point in millimetres.
    let surface = |x: f64, y: f64| {
        let h = map.sample(x, y);
        -depth * if light_is_high { 1.0 - h } else { h }
    };
    // Tip height at each row position: the bit rests on the highest point under it.
    let rows: Vec<Vec<f64>> = ys
        .iter()
        .map(|&y| {
            xs.iter()
                .map(|&x| {
                    profile
                        .iter()
                        .map(|&([dx, dy], lift)| surface(x + dx, y + dy) - lift)
                        .fold(f64::NEG_INFINITY, f64::max)
                        .min(0.0)
                })
                .collect()
        })
        .collect();

    if rows.iter().flatten().all(|&z| z > -LEVEL_EPSILON_MM) {
        return Err(
            "The image is flat at the surface height, so there is nothing to carve.".into(),
        );
    }

    w.power = 1.0;
    w.z_move(safe_z, None);
    w.line(&format!("M3 S{}", num(spindle_rpm)));
    w.line(&format!("G4 P{}", num(SPINDLE_DWELL_S)));
    w.seconds += SPINDLE_DWELL_S;

    let levels = (depth / depth_per_pass - 1e-9).ceil().max(1.0) as usize;
    for level in 1..=levels {
        let floor = -(depth_per_pass * level as f64).min(depth);
        let previous_floor = -(depth_per_pass * (level - 1) as f64);
        // Where the bit was left after the previous row of this level.
        let mut last: Option<(usize, f64)> = None;
        let mut forward = true;

        for (j, row) in rows.iter().enumerate() {
            // Nothing in this row lies below what earlier levels already cut.
            if row.iter().all(|&z| z >= previous_floor - LEVEL_EPSILON_MM) {
                continue;
            }
            let zs: Vec<f64> = row.iter().map(|&z| z.max(floor)).collect();
            let mut order = simplify(&xs, &zs);
            if !forward {
                order.reverse();
            }
            let y = offset[1] + ys[j];
            let start = [offset[0] + xs[order[0]], y];
            let start_z = zs[order[0]];

            match last {
                // Step across to the next row at the higher of the two heights,
                // so the link move cannot cut below either row.
                Some((previous, end_z)) if previous + 1 == j => {
                    let link_z = end_z.max(start_z);
                    if link_z > end_z + LEVEL_EPSILON_MM {
                        w.z_move(link_z, Some(operation.feed));
                    }
                    w.cut(start, operation.feed, &format!(" F{}", num(operation.feed)));
                    if link_z > start_z + LEVEL_EPSILON_MM {
                        w.z_move(start_z, Some(plunge_feed));
                    }
                }
                _ => {
                    w.z_move(safe_z, None);
                    w.travel(start, "");
                    w.z_move(start_z, Some(plunge_feed));
                }
            }

            for (n, &i) in order.iter().enumerate().skip(1) {
                let feed = if n == 1 {
                    format!(" F{}", num(operation.feed))
                } else {
                    String::new()
                };
                w.z = zs[i];
                w.cut(
                    [offset[0] + xs[i], y],
                    operation.feed,
                    &format!(" Z{}{feed}", num(zs[i])),
                );
            }
            last = Some((j, zs[order[order.len() - 1]]));
            forward = !forward;
        }
    }
    w.z_move(safe_z, None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::generate_with;
    use crate::geometry::Artwork;
    use crate::job::{Job, Machine, MachineKind, Origin};

    fn map(width: usize, height: usize, value: impl Fn(usize, usize) -> f32) -> Heightmap {
        Heightmap {
            width,
            height,
            data: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| value(x, y))
                .collect(),
            width_mm: width as f64 * 0.5,
            height_mm: height as f64 * 0.5,
        }
    }

    fn relief(ball_nose: bool) -> OperationKind {
        OperationKind::CncRelief {
            depth: 2.0,
            depth_per_pass: 0.75,
            plunge_feed: 200.0,
            safe_z: 5.0,
            spindle_rpm: 12000.0,
            tool_diameter: 3.0,
            ball_nose,
            stepover: 1.0,
            light_is_high: true,
            taper_angle: 0.0,
            shank_diameter: 0.0,
        }
    }

    fn job(kind: OperationKind) -> Job {
        Job {
            machine: Machine {
                kind: MachineKind::Cnc,
                bed_width: 100.0,
                bed_height: 100.0,
                max_power_s: 1000.0,
                travel_feed: 6000.0,
                origin: Origin::BackLeft,
                start_gcode: String::new(),
                end_gcode: String::new(),
            },
            offset: [10.0, 10.0],
            material_thickness: None,
            operations: vec![Operation {
                color: None,
                kind,
                feed: 800.0,
                power: 0.0,
                passes: 1,
            }],
        }
    }

    fn empty_artwork() -> Artwork {
        Artwork {
            width: 0.0,
            height: 0.0,
            shapes: Vec::new(),
        }
    }

    /// One parsed motion line: rapid or cut, and the position after it.
    struct Move {
        rapid: bool,
        xy_changed: bool,
        position: [f64; 3],
    }

    fn moves(text: &str) -> Vec<Move> {
        let mut position = [0.0, 0.0, 0.0];
        let mut out = Vec::new();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let rapid = match words.next() {
                Some("G0") => true,
                Some("G1") => false,
                _ => continue,
            };
            let mut xy_changed = false;
            for word in words {
                let (axis, value) = word.split_at(1);
                let value: f64 = value.parse().unwrap();
                match axis {
                    "X" => (position[0], xy_changed) = (value, true),
                    "Y" => (position[1], xy_changed) = (value, true),
                    "Z" => position[2] = value,
                    _ => {}
                }
            }
            out.push(Move {
                rapid,
                xy_changed,
                position,
            });
        }
        out
    }

    #[test]
    fn flat_grey_is_cut_in_levels_down_to_its_height() {
        let grey = map(40, 20, |_, _| 0.5);
        let program = generate_with(&empty_artwork(), Some(&grey), &job(relief(false))).unwrap();
        let mut depths: Vec<f64> = moves(&program.text)
            .iter()
            .filter(|m| !m.rapid && m.xy_changed)
            .map(|m| m.position[2])
            .collect();
        depths.sort_by(f64::total_cmp);
        depths.dedup();
        // Half-height grey is 1 mm deep: one level at 0.75, then the finished surface.
        assert_eq!(depths, [-1.0, -0.75]);
        // 10 mm of rows at 1 mm stepover, cut twice.
        let rows = moves(&program.text)
            .iter()
            .filter(|m| !m.rapid && m.xy_changed)
            .count();
        assert!(rows >= 22, "{rows} cutting moves");
    }

    #[test]
    fn the_bit_never_goes_below_the_surface() {
        // A cliff: full height on the left, full depth on the right, plus a narrow slot.
        let cliff = map(60, 24, |x, _| {
            if x < 30 || (40..42).contains(&x) {
                1.0
            } else {
                0.0
            }
        });
        for ball_nose in [false, true] {
            let program =
                generate_with(&empty_artwork(), Some(&cliff), &job(relief(ball_nose))).unwrap();
            let radius = 1.5;
            let mut deepest: f64 = 0.0;
            for m in moves(&program.text).iter().filter(|m| !m.rapid) {
                let [x, y, z] = m.position;
                deepest = deepest.min(z);
                // Check the whole footprint of the bit against the wanted surface.
                for a in 0..24 {
                    for ring in 0..=6 {
                        let r = radius * ring as f64 / 6.0;
                        let angle = a as f64 * std::f64::consts::TAU / 24.0;
                        let (px, py) = (x - 10.0 + r * angle.cos(), y - 10.0 + r * angle.sin());
                        let lift = if ball_nose {
                            radius - (radius * radius - r * r).sqrt()
                        } else {
                            0.0
                        };
                        // At a vertical wall any sideways error is a large height
                        // error, so the tolerance is sideways: the bit may not
                        // cut below the surface found within 0.2 mm of each point.
                        let wanted = [[0.0, 0.0], [0.2, 0.0], [-0.2, 0.0], [0.0, 0.2], [0.0, -0.2]]
                            .iter()
                            .map(|d| -2.0 * (1.0 - cliff.sample(px + d[0], py + d[1])))
                            .fold(f64::INFINITY, f64::min);
                        assert!(
                            z + lift >= wanted - 0.02,
                            "bit at ({x}, {y}, {z}) cuts {} mm below the surface",
                            wanted - z - lift
                        );
                    }
                }
            }
            assert!(
                (deepest + 2.0).abs() < 1e-6,
                "reached {deepest}, expected the full 2 mm"
            );
        }
    }

    #[test]
    fn a_wide_bit_stays_out_of_a_narrow_slot() {
        // A 1 mm wide slot in a full-height block; the bit is 3 mm wide.
        let slot = map(60, 20, |x, _| if (29..31).contains(&x) { 0.0 } else { 1.0 });
        let error = generate_with(&empty_artwork(), Some(&slot), &job(relief(false))).unwrap_err();
        assert!(error.contains("nothing to carve"), "{error}");
    }

    #[test]
    fn rapids_only_cross_the_work_at_the_safe_height() {
        let ramp = map(40, 20, |x, _| x as f32 / 39.0);
        let program = generate_with(&empty_artwork(), Some(&ramp), &job(relief(true))).unwrap();
        let all = moves(&program.text);
        assert!(all.iter().any(|m| m.rapid && m.xy_changed));
        for m in all.iter().filter(|m| m.rapid && m.xy_changed) {
            assert_eq!(
                m.position[2], 5.0,
                "rapid across the work at Z{}",
                m.position[2]
            );
        }
        let lines: Vec<&str> = program.text.lines().collect();
        assert_eq!(lines[lines.len() - 3..], ["G0 Z5", "M5", "G0 X0 Y0"]);
    }

    #[test]
    fn dark_can_be_made_the_high_side() {
        let dark = map(40, 20, |_, _| 0.0);
        // With light as high, black is the deepest cut...
        assert!(generate_with(&empty_artwork(), Some(&dark), &job(relief(false))).is_ok());
        // ...and with dark as high it is the untouched stock top.
        let OperationKind::CncRelief {
            depth,
            depth_per_pass,
            plunge_feed,
            safe_z,
            spindle_rpm,
            tool_diameter,
            ball_nose,
            stepover,
            ..
        } = relief(false)
        else {
            unreachable!()
        };
        let flipped = OperationKind::CncRelief {
            depth,
            depth_per_pass,
            plunge_feed,
            safe_z,
            spindle_rpm,
            tool_diameter,
            ball_nose,
            stepover,
            light_is_high: false,
            taper_angle: 0.0,
            shank_diameter: 0.0,
        };
        assert!(generate_with(&empty_artwork(), Some(&dark), &job(flipped)).is_err());
    }

    #[test]
    fn settings_and_inputs_are_checked() {
        let grey = map(40, 20, |_, _| 0.5);
        assert!(generate_with(&empty_artwork(), None, &job(relief(false)))
            .unwrap_err()
            .contains("needs an image"));

        let mut wide = job(relief(false));
        if let OperationKind::CncRelief { stepover, .. } = &mut wide.operations[0].kind {
            *stepover = 4.0;
        }
        assert!(generate_with(&empty_artwork(), Some(&grey), &wide)
            .unwrap_err()
            .contains("stepover"));

        let mut laser = job(relief(false));
        laser.machine.kind = MachineKind::Laser;
        assert!(generate_with(&empty_artwork(), Some(&grey), &laser).is_err());

        let mut off_bed = job(relief(false));
        off_bed.offset = [95.0, 10.0];
        assert!(generate_with(&empty_artwork(), Some(&grey), &off_bed)
            .unwrap_err()
            .contains("does not fit"));
    }

    #[test]
    fn a_tapered_ball_nose_reaches_into_narrow_slots_but_spares_their_walls() {
        // A 2 mm wide, 2 mm deep slot. A 3 mm straight bit cannot enter it at
        // all; a tapered bit with a 0.5 mm ball can reach the bottom.
        let slot = map(60, 20, |x, _| if (28..32).contains(&x) { 0.0 } else { 1.0 });
        let tapered = OperationKind::CncRelief {
            depth: 2.0,
            depth_per_pass: 2.0,
            plunge_feed: 200.0,
            safe_z: 5.0,
            spindle_rpm: 12000.0,
            tool_diameter: 0.5,
            ball_nose: true,
            stepover: 0.5,
            light_is_high: true,
            taper_angle: 5.5,
            shank_diameter: 6.35,
        };
        let program = generate_with(&empty_artwork(), Some(&slot), &job(tapered)).unwrap();
        let cuts: Vec<_> = moves(&program.text)
            .into_iter()
            .filter(|m| !m.rapid)
            .collect();
        let deepest = cuts.iter().map(|m| m.position[2]).fold(0.0, f64::min);
        assert!(deepest < -1.9, "only reached {deepest}");

        // Away from the slot the bit must stay at the surface: its widening
        // sides would otherwise cut the slot's edges.
        let bit = Bit {
            radius: 0.25,
            ball_nose: true,
            taper: 5.5_f64.to_radians(),
        };
        for m in &cuts {
            let [x, y, z] = m.position;
            for i in -40..=40 {
                let dx = i as f64 * 0.02;
                let wanted = [-0.2, 0.0, 0.2]
                    .iter()
                    .map(|d| -2.0 * (1.0 - slot.sample(x - 10.0 + dx + d, y - 10.0)))
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    z + bit.lift(dx.abs()) >= wanted - 0.02,
                    "bit at ({x}, {z}) cuts below the surface {dx} mm from its axis"
                );
            }
        }
    }

    #[test]
    fn bit_shapes_have_the_expected_profiles() {
        let flat = Bit {
            radius: 1.5,
            ball_nose: false,
            taper: 0.0,
        };
        assert_eq!(flat.lift(1.0), 0.0);
        let ball = Bit {
            radius: 1.5,
            ball_nose: true,
            taper: 0.0,
        };
        assert!((ball.lift(1.5) - 1.5).abs() < 1e-9);
        assert_eq!(ball.reach(3.0, 0.0), 1.5);

        let angle = 10.0_f64.to_radians();
        let tapered = Bit {
            radius: 0.25,
            ball_nose: true,
            taper: angle,
        };
        // The same as a plain ball near the tip.
        assert!((tapered.lift(0.1) - (0.25 - (0.25_f64 * 0.25 - 0.01).sqrt())).abs() < 1e-9);
        // Beyond the ball, the surface rises along the cone.
        let (near, far) = (tapered.lift(1.0), tapered.lift(2.0));
        assert!((far - near - 1.0 / angle.tan()).abs() < 1e-9);
        // The cone starts where the ball leaves off, without a step.
        let tangent = 0.25 * angle.cos();
        assert!((tapered.lift(tangent + 1e-9) - tapered.lift(tangent - 1e-9)).abs() < 1e-6);
        // It spreads with depth, but never past the shank.
        assert!(tapered.reach(3.0, 0.0) > tapered.reach(1.0, 0.0));
        assert_eq!(tapered.reach(100.0, 6.35), 3.175);
    }

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0];
        assert_eq!(simplify(&xs, &[0.0, 0.0, 0.0, 0.0, 0.0]), [0, 4]);
        assert_eq!(simplify(&xs, &[0.0, 0.0, -1.0, -1.0, -1.0]), [0, 1, 2, 4]);
    }
}
