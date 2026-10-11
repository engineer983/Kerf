//! Cutting a piece free: a line around the artwork, cut to full depth.
//!
//! The line is the artwork's rectangle (or the ellipse inside it) moved out or
//! in by an offset. The bit can run on the line, or to one side of it so that
//! the edge of the cut, not its middle, lands on the line. Small bridges of
//! material, called tabs, can be left so the piece does not come loose and get
//! thrown or gouged on the last pass.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use serde::{Deserialize, Serialize};

use crate::generate::{num, Writer, SPINDLE_DWELL_S};
use crate::geometry::{distance, Polyline};
use crate::job::{Operation, OperationKind};

/// Which side of the line the bit runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// The middle of the bit follows the line.
    On,
    /// The bit stays inside the line; a hole comes out the size of the line.
    Inside,
    /// The bit stays outside the line; the piece comes out the size of the line.
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outline {
    Rectangle,
    Ellipse,
}

/// Arcs are flattened to within this distance of the true curve.
const ARC_TOLERANCE_MM: f64 = 0.02;

/// The closed path of the bit's centre, starting and ending at the middle of
/// the front side. Coordinates are in bed space (Y down), so the front is the
/// side with the largest Y.
fn bit_path(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    corner: f64,
    outline: Outline,
) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    // Appends an arc about (cx, cy), with angles measured from +X towards +Y.
    let arc =
        |points: &mut Vec<[f64; 2]>, cx: f64, cy: f64, rx: f64, ry: f64, from: f64, to: f64| {
            let radius = rx.max(ry);
            let step = 2.0 * (1.0 - ARC_TOLERANCE_MM / radius).clamp(-1.0, 1.0).acos();
            let steps = ((to - from).abs() / step.max(1e-3)).ceil().max(1.0) as usize;
            for i in 1..=steps {
                let angle = from + (to - from) * i as f64 / steps as f64;
                points.push([cx + rx * angle.cos(), cy + ry * angle.sin()]);
            }
        };

    match outline {
        Outline::Ellipse => {
            let (cx, cy, rx, ry) = (x + width / 2.0, y + height / 2.0, width / 2.0, height / 2.0);
            points.push([cx, cy + ry]);
            arc(&mut points, cx, cy, rx, ry, FRAC_PI_2, FRAC_PI_2 - TAU);
        }
        Outline::Rectangle => {
            let c = corner.clamp(0.0, width.min(height) / 2.0);
            let (right, bottom) = (x + width, y + height);
            points.push([x + width / 2.0, bottom]);
            points.push([right - c, bottom]);
            if c > 0.0 {
                arc(&mut points, right - c, bottom - c, c, c, FRAC_PI_2, 0.0);
            }
            points.push([right, y + c]);
            if c > 0.0 {
                arc(&mut points, right - c, y + c, c, c, 0.0, -FRAC_PI_2);
            }
            points.push([x + c, y]);
            if c > 0.0 {
                arc(&mut points, x + c, y + c, c, c, -FRAC_PI_2, -PI);
            }
            points.push([x, bottom - c]);
            if c > 0.0 {
                arc(&mut points, x + c, bottom - c, c, c, PI, FRAC_PI_2);
            }
            points.push([x + width / 2.0, bottom]);
        }
    }
    points.dedup_by(|a, b| distance(*a, *b) < 1e-9);
    points
}

/// The path with each point's distance along it, and extra points added at
/// the distances in `marks` (which must be sorted).
fn with_marks(path: &[[f64; 2]], marks: &[f64]) -> Vec<([f64; 2], f64)> {
    let mut out = vec![(path[0], 0.0)];
    let mut travelled = 0.0;
    let mut next = 0;
    for pair in path.windows(2) {
        let length = distance(pair[0], pair[1]);
        while next < marks.len() && marks[next] <= travelled + length {
            let t = if length > 0.0 {
                (marks[next] - travelled) / length
            } else {
                0.0
            };
            if t > 1e-9 && t < 1.0 - 1e-9 {
                out.push((
                    [
                        pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                        pair[0][1] + (pair[1][1] - pair[0][1]) * t,
                    ],
                    marks[next],
                ));
            }
            next += 1;
        }
        travelled += length;
        out.push((pair[1], travelled));
    }
    out
}

/// The line a laser follows to cut the piece free: the artwork's outline
/// moved out by the operation's offset. A laser's cut is too narrow to need
/// keeping to one side of the line.
pub(crate) fn laser_line(
    operation: &Operation,
    artwork: [f64; 2],
    offset: [f64; 2],
) -> Result<Polyline, String> {
    let OperationKind::LaserCutout {
        offset: margin,
        outline,
        corner_radius,
    } = operation.kind
    else {
        return Err("This is not a laser cutout.".into());
    };
    if artwork[0] <= 0.0 || artwork[1] <= 0.0 {
        return Err("There is no artwork to cut around.".into());
    }
    let (width, height) = (artwork[0] + 2.0 * margin, artwork[1] + 2.0 * margin);
    if width <= 0.0 || height <= 0.0 {
        return Err("The offset is so far inwards that nothing is left to cut around.".into());
    }
    Ok(Polyline {
        points: bit_path(
            offset[0] - margin,
            offset[1] - margin,
            width,
            height,
            corner_radius,
            outline,
        ),
        closed: true,
    })
}

pub(crate) fn cut(
    w: &mut Writer,
    operation: &Operation,
    artwork: [f64; 2],
    offset: [f64; 2],
) -> Result<(), String> {
    let OperationKind::CncCutout {
        depth,
        depth_per_pass,
        plunge_feed,
        safe_z,
        spindle_rpm,
        tool_diameter,
        offset: margin,
        side,
        outline,
        corner_radius,
        tabs,
        tab_width,
        tab_height,
    } = operation.kind
    else {
        return Ok(());
    };
    if artwork[0] <= 0.0 || artwork[1] <= 0.0 {
        return Err("There is no artwork to cut around.".into());
    }

    let radius = tool_diameter / 2.0;
    // How far the bit's centre sits from the artwork's edge.
    let grow = margin
        + match side {
            Side::On => 0.0,
            Side::Outside => radius,
            Side::Inside => -radius,
        };
    let (width, height) = (artwork[0] + 2.0 * grow, artwork[1] + 2.0 * grow);
    if width <= 0.0 || height <= 0.0 {
        return Err("The offset is so far inwards that nothing is left to cut around.".into());
    }
    // Moving a rounded corner outwards enlarges its radius; inwards shrinks it.
    let corner = match side {
        Side::On => corner_radius,
        Side::Outside => corner_radius + radius,
        Side::Inside => (corner_radius - radius).max(0.0),
    };
    let path = bit_path(
        offset[0] - grow,
        offset[1] - grow,
        width,
        height,
        corner,
        outline,
    );
    let length: f64 = path.windows(2).map(|p| distance(p[0], p[1])).sum();

    // Tabs are spaced evenly, clear of the start point. A tab's width is the
    // material left, so the bit's centre is lifted for that plus its own width.
    let tab_top = -(depth - tab_height.clamp(0.0, depth));
    let lifted = tab_width + tool_diameter;
    let tabs = if tab_width > 0.0 && tab_height > 0.0 && lifted * (tabs as f64) < length * 0.8 {
        tabs as usize
    } else {
        0
    };
    let spans: Vec<[f64; 2]> = (0..tabs)
        .map(|i| {
            let centre = length * (i as f64 + 0.5) / tabs as f64;
            [centre - lifted / 2.0, centre + lifted / 2.0]
        })
        .collect();
    let marks: Vec<f64> = spans.iter().flatten().copied().collect();
    let marked = with_marks(&path, &marks);
    let in_tab = |s: f64| {
        spans
            .iter()
            .any(|span| s > span[0] + 1e-9 && s < span[1] - 1e-9)
    };

    w.power = 1.0;
    w.z_move(safe_z, None);
    w.line(&format!("M3 S{}", num(spindle_rpm)));
    w.line(&format!("G4 P{}", num(SPINDLE_DWELL_S)));
    w.seconds += SPINDLE_DWELL_S;
    w.travel(path[0], "");

    let levels = (depth / depth_per_pass - 1e-9).ceil().max(1.0) as usize;
    let feed = format!(" F{}", num(operation.feed));
    for level in 1..=levels {
        let floor = -(depth_per_pass * level as f64).min(depth);
        w.z_move(floor, Some(plunge_feed));
        let mut first = true;
        for pair in marked.windows(2) {
            let ((_, from), (to, until)) = (pair[0], pair[1]);
            // Over a tab the bit rides at the tab's top instead of the level's floor.
            let wanted = if floor < tab_top && in_tab((from + until) / 2.0) {
                tab_top
            } else {
                floor
            };
            if (w.z - wanted).abs() > 1e-9 {
                let lifting = wanted > w.z;
                w.z_move(
                    wanted,
                    Some(if lifting { operation.feed } else { plunge_feed }),
                );
                first = true;
            }
            w.cut(to, operation.feed, if first { &feed } else { "" });
            first = false;
        }
    }
    w.z_move(safe_z, None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::generate;
    use crate::geometry::Artwork;
    use crate::job::{Job, Machine, MachineKind, Origin};

    fn cutout(side: Side, margin: f64, tabs: u32) -> OperationKind {
        OperationKind::CncCutout {
            depth: 6.0,
            depth_per_pass: 2.0,
            plunge_feed: 200.0,
            safe_z: 5.0,
            spindle_rpm: 16000.0,
            tool_diameter: 4.0,
            offset: margin,
            side,
            outline: Outline::Rectangle,
            corner_radius: 0.0,
            tabs,
            tab_width: 6.0,
            tab_height: 1.5,
        }
    }

    fn job(kind: OperationKind) -> Job {
        Job {
            machine: Machine {
                kind: MachineKind::Cnc,
                bed_width: 200.0,
                bed_height: 200.0,
                max_power_s: 1000.0,
                travel_feed: 6000.0,
                origin: Origin::BackLeft,
                start_gcode: String::new(),
                end_gcode: String::new(),
            },
            offset: [50.0, 60.0],
            operations: vec![Operation {
                color: None,
                kind,
                feed: 800.0,
                power: 0.0,
                passes: 1,
            }],
        }
    }

    /// A 40 × 30 mm piece of artwork with nothing drawn on it.
    fn artwork() -> Artwork {
        Artwork {
            width: 40.0,
            height: 30.0,
            shapes: Vec::new(),
        }
    }

    fn bounds(kind: OperationKind) -> [f64; 4] {
        let b = generate(&artwork(), &job(kind)).unwrap().bounds;
        [b.min_x, b.min_y, b.max_x, b.max_y].map(|v| (v * 1000.0).round() / 1000.0)
    }

    #[test]
    fn the_bit_runs_on_inside_or_outside_the_line() {
        // The artwork spans X 50–90 and Y 60–90; the line is 5 mm outside it.
        assert_eq!(bounds(cutout(Side::On, 5.0, 0)), [45.0, 55.0, 95.0, 95.0]);
        // A 4 mm bit sits 2 mm further out, so its inner edge is on the line...
        assert_eq!(
            bounds(cutout(Side::Outside, 5.0, 0)),
            [43.0, 53.0, 97.0, 97.0]
        );
        // ...or 2 mm further in, so its outer edge is.
        assert_eq!(
            bounds(cutout(Side::Inside, 5.0, 0)),
            [47.0, 57.0, 93.0, 93.0]
        );
        // A negative offset puts the line inside the artwork.
        assert_eq!(bounds(cutout(Side::On, -5.0, 0)), [55.0, 65.0, 85.0, 85.0]);
    }

    #[test]
    fn cuts_a_closed_loop_down_to_full_depth_in_levels() {
        let program = generate(&artwork(), &job(cutout(Side::On, 0.0, 0))).unwrap();
        let moves: Vec<&[f32]> = program.moves.chunks_exact(4).collect();
        let cutting: Vec<&&[f32]> = moves.iter().filter(|m| m[3] >= 0.0).collect();
        let mut depths: Vec<f32> = cutting.iter().map(|m| m[2]).collect();
        depths.sort_by(f32::total_cmp);
        depths.dedup();
        assert_eq!(depths, [-6.0, -4.0, -2.0]);
        // Three laps of a 40 × 30 rectangle.
        assert!(
            (program.cut_mm - 3.0 * 140.0).abs() < 0.01,
            "cut {}",
            program.cut_mm
        );
        // Each lap ends where it began: the middle of the front side.
        let last_cut = cutting[cutting.len() - 1];
        assert_eq!((last_cut[0], last_cut[1]), (70.0, 90.0));
        assert!(program.text.contains("G1 Z-6 F200"));
    }

    #[test]
    fn tabs_are_left_at_the_bottom_of_the_cut() {
        let program = generate(&artwork(), &job(cutout(Side::On, 0.0, 4))).unwrap();
        let moves: Vec<&[f32]> = program.moves.chunks_exact(4).collect();
        // The tab top is 1.5 mm above the 6 mm floor. Only the last level goes
        // below it, so only there is the bit lifted, once per tab.
        let lifts = moves
            .windows(2)
            .filter(|p| p[0][2] == -6.0 && p[1][2] == -4.5 && p[0][..2] == p[1][..2])
            .count();
        assert_eq!(lifts, 4);
        // Length cut at the tab height: four tabs, each 6 mm plus the 4 mm bit.
        let on_tabs: f32 = moves
            .windows(2)
            .filter(|p| p[1][3] >= 0.0 && p[0][2] == -4.5 && p[1][2] == -4.5)
            .map(|p| (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]))
            .sum();
        assert!((on_tabs - 40.0).abs() < 0.01, "tab length {on_tabs}");
        // No tab sits on the start point, where the bit plunges.
        assert_eq!(
            moves
                .iter()
                .filter(|m| m[3] >= 0.0)
                .map(|m| m[2])
                .fold(0.0, f32::min),
            -6.0
        );
    }

    #[test]
    fn rounded_corners_and_ellipses_stay_inside_the_rectangle() {
        let OperationKind::CncCutout { .. } = cutout(Side::On, 0.0, 0) else {
            unreachable!()
        };
        for (outline, corner_radius) in [(Outline::Rectangle, 8.0), (Outline::Ellipse, 0.0)] {
            let path = bit_path(0.0, 0.0, 40.0, 30.0, corner_radius, outline);
            assert_eq!(path[0], [20.0, 30.0]);
            assert!(
                distance(path[0], path[path.len() - 1]) < 1e-6,
                "{outline:?} is not closed"
            );
            for p in &path {
                assert!(
                    (-1e-6..=40.0 + 1e-6).contains(&p[0]) && (-1e-6..=30.0 + 1e-6).contains(&p[1])
                );
            }
            // The corners are cut off: nothing reaches the rectangle's corner.
            assert!(path.iter().all(|p| distance(*p, [40.0, 30.0]) > 2.0));
        }
        // An ellipse touches the middle of each side.
        let ellipse = bit_path(0.0, 0.0, 40.0, 30.0, 0.0, Outline::Ellipse);
        // (Within a step of the flattened curve.)
        assert!(ellipse.iter().any(|p| distance(*p, [40.0, 15.0]) < 1.0));
        assert!(ellipse.iter().any(|p| distance(*p, [20.0, 0.0]) < 1.0));
    }

    fn laser_job(margin: f64, passes: u32) -> Job {
        let mut job = job(OperationKind::LaserCutout {
            offset: margin,
            outline: Outline::Rectangle,
            corner_radius: 0.0,
        });
        job.machine.kind = MachineKind::Laser;
        job.operations[0].power = 80.0;
        job.operations[0].passes = passes;
        job
    }

    #[test]
    fn a_laser_cuts_the_same_line_with_its_own_power_and_passes() {
        let program = generate(&artwork(), &laser_job(5.0, 2)).unwrap();
        let b = program.bounds;
        assert_eq!(
            [b.min_x, b.min_y, b.max_x, b.max_y].map(|v| (v * 1000.0).round() / 1000.0),
            [45.0, 55.0, 95.0, 95.0]
        );
        // Two laps of a 50 x 40 rectangle, at 80% of full power, with no Z moves.
        assert!(
            (program.cut_mm - 2.0 * 180.0).abs() < 0.01,
            "cut {}",
            program.cut_mm
        );
        assert!(program.text.contains("S800 F800"));
        assert!(!program.text.contains('Z'));
        assert!(generate(&artwork(), &laser_job(-20.0, 1))
            .unwrap_err()
            .contains("nothing is left"));
    }

    #[test]
    fn a_laser_engraves_first_and_cuts_out_last_in_one_job() {
        let mut job = laser_job(3.0, 1);
        job.operations.insert(
            0,
            Operation {
                color: None,
                kind: OperationKind::LaserCut,
                feed: 3000.0,
                power: 20.0,
                passes: 1,
            },
        );
        let drawn = Artwork::from_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path d="M10 10H30" stroke="#000" fill="none"/></svg>"##,
            0.05,
        )
        .unwrap();
        let program = generate(&drawn, &job).unwrap();
        assert_eq!(program.operation_starts.len(), 2);
        let engrave = program.text.find("S200 F3000").unwrap();
        let cut = program.text.find("S800 F800").unwrap();
        assert!(engrave < cut);
        // One program, with the laser switched off only once, at the end.
        assert_eq!(program.text.matches("M5").count(), 1);
        // A router cannot run it.
        job.machine.kind = MachineKind::Cnc;
        assert!(generate(&drawn, &job).is_err());
    }

    #[test]
    fn a_sign_engraves_its_lettering_then_cuts_round_everything() {
        // A plain 40 x 30 piece of artwork at (50, 60), with a line of
        // lettering below it at Y 100, and the cutout listed first.
        let mut job = laser_job(2.0, 1);
        job.operations.push(Operation {
            color: None,
            kind: OperationKind::LaserCut,
            feed: 3000.0,
            power: 20.0,
            passes: 1,
        });
        let base = Artwork::from_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path d="M5 5H35" stroke="#000" fill="none"/></svg>"##,
            0.05,
        )
        .unwrap();
        let lettering = Artwork::from_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path d="M40 100H110" stroke="#000" fill="none"/></svg>"##,
            0.05,
        )
        .unwrap();
        let added = [Operation {
            color: None,
            kind: OperationKind::LaserCut,
            feed: 1500.0,
            power: 40.0,
            passes: 1,
        }];
        let additions = crate::generate::Additions {
            artwork: &lettering,
            operations: &added,
        };
        // Everything on the material spans X 40-110 and Y 60-100.
        let around = Some([40.0, 60.0, 70.0, 40.0]);
        let program =
            crate::generate::generate_composed(&base, None, &job, Some(&additions), around)
                .unwrap();

        assert_eq!(program.operation_starts.len(), 3);
        let (main, added_at, cut) = (
            program.text.find("S200 F3000").unwrap(),
            program.text.find("S400 F1500").unwrap(),
            program.text.find("S800 F800").unwrap(),
        );
        assert!(
            main < added_at && added_at < cut,
            "the cutout must come last"
        );
        // The lettering is cut where it was drawn, with no offset added.
        assert!(program.text.contains("X110 Y100"));
        // The cut goes 2 mm outside everything, not just the main artwork.
        let b = program.bounds;
        assert_eq!(
            [b.min_x, b.min_y, b.max_x, b.max_y].map(|v| (v * 1000.0).round() / 1000.0),
            [38.0, 58.0, 112.0, 102.0]
        );

        // A picture operation has nothing to work from in lettering.
        let picture = [Operation {
            kind: OperationKind::LaserImage {
                interval: 0.1,
                dither: true,
            },
            ..added[0].clone()
        }];
        let wrong = crate::generate::Additions {
            artwork: &lettering,
            operations: &picture,
        };
        assert!(crate::generate::generate_composed(&base, None, &job, Some(&wrong), None).is_err());
    }

    #[test]
    fn impossible_cutouts_are_refused() {
        assert!(generate(&artwork(), &job(cutout(Side::On, -20.0, 0)))
            .unwrap_err()
            .contains("nothing is left"));
        let mut laser = job(cutout(Side::On, 0.0, 0));
        laser.machine.kind = MachineKind::Laser;
        assert!(generate(&artwork(), &laser).is_err());
        let mut off_bed = job(cutout(Side::Outside, 60.0, 0));
        off_bed.offset = [10.0, 10.0];
        assert!(generate(&artwork(), &off_bed)
            .unwrap_err()
            .contains("does not fit"));
    }
}
