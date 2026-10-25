//! Laser engraving of a shaded image, line by line.
//!
//! The image is resampled to square dots one line interval across. Each row is
//! one sweep of the head; runs of dots at the same power become one move. Rows
//! alternate direction, and blank rows and blank row ends are skipped.

use crate::generate::{num, Writer};
use crate::job::{Operation, OperationKind};
use crate::relief::Heightmap;

/// Shades are rounded to this many power steps, which keeps neighbouring dots
/// of almost equal shade in one move.
const POWER_STEPS: f64 = 64.0;

/// Where the rounding error of a dot goes: (columns ahead, rows down, share
/// out of 48). This is the Jarvis, Judice and Ninke pattern, which spreads
/// the error over twelve neighbours. It leaves a finer, more even grain than
/// simpler patterns, without their stripes in flat areas.
const SPREAD: [(isize, usize, f64); 12] = [
    (1, 0, 7.0),
    (2, 0, 5.0),
    (-2, 1, 3.0),
    (-1, 1, 5.0),
    (0, 1, 7.0),
    (1, 1, 5.0),
    (2, 1, 3.0),
    (-2, 2, 1.0),
    (-1, 2, 3.0),
    (0, 2, 5.0),
    (1, 2, 3.0),
    (2, 2, 1.0),
];

/// Turns shades into dots that are fully on or off, whose density gives the
/// shade. Each dot's rounding error is passed to dots not yet visited. Rows
/// are worked in alternate directions, so the error does not always drift the
/// same way and leave slanting streaks.
fn dither_dots(dark: &mut [f64], columns: usize, rows: usize) {
    for row in 0..rows {
        let backwards = row % 2 == 1;
        for step in 0..columns {
            let column = if backwards { columns - 1 - step } else { step };
            let i = row * columns + column;
            let wanted = dark[i];
            let chosen = if wanted >= 0.5 { 1.0 } else { 0.0 };
            dark[i] = chosen;
            let error = wanted - chosen;
            for (ahead, down, share) in SPREAD {
                let across = column as isize + if backwards { -ahead } else { ahead };
                if across < 0 || across >= columns as isize || row + down >= rows {
                    continue;
                }
                dark[(row + down) * columns + across as usize] += error * share / 48.0;
            }
        }
    }
}

pub(crate) fn engrave(w: &mut Writer, operation: &Operation, map: &Heightmap, offset: [f64; 2]) {
    let OperationKind::LaserImage { interval, dither } = operation.kind else {
        return;
    };
    if !map.is_valid() {
        return;
    }
    let columns = (map.width_mm / interval).round().max(1.0) as usize;
    let rows = (map.height_mm / interval).round().max(1.0) as usize;
    let dot_width = map.width_mm / columns as f64;
    let dot_height = map.height_mm / rows as f64;

    // Darkness per dot: 0 is untouched, 1 is full power.
    let mut dark: Vec<f64> = (0..rows * columns)
        .map(|i| {
            let (row, column) = (i / columns, i % columns);
            1.0 - map.sample(
                (column as f64 + 0.5) * dot_width,
                (row as f64 + 0.5) * dot_height,
            )
        })
        .collect();

    if dither {
        dither_dots(&mut dark, columns, rows);
    } else {
        for d in &mut dark {
            *d = (d.clamp(0.0, 1.0) * POWER_STEPS).round() / POWER_STEPS;
        }
    }

    let full_power = w.machine.max_power_s * operation.power / 100.0;
    let feed = num(operation.feed);
    w.sketch = false;
    w.line("M4 S0");

    let mut forward = true;
    for _ in 0..operation.passes {
        for row in 0..rows {
            let shades = &dark[row * columns..(row + 1) * columns];
            let Some(first) = shades.iter().position(|&d| d > 0.0) else {
                continue;
            };
            let last = shades.iter().rposition(|&d| d > 0.0).expect("a dot is on");
            let y = offset[1] + (row as f64 + 0.5) * dot_height;
            let x_at = |edge: usize| offset[0] + edge as f64 * dot_width;

            // Dots from `first` to `last`, in the direction of this sweep.
            let order: Vec<usize> = if forward {
                (first..=last).collect()
            } else {
                (first..=last).rev().collect()
            };
            let start_edge = if forward { first } else { last + 1 };
            w.travel([x_at(start_edge), y], " S0");

            let mut n = 0;
            let mut first_move = true;
            while n < order.len() {
                let shade = shades[order[n]];
                let mut end = n;
                while end + 1 < order.len() && shades[order[end + 1]] == shade {
                    end += 1;
                }
                // The far edge of the last dot in this run.
                let edge = if forward { order[end] + 1 } else { order[end] };
                w.power = operation.power / 100.0 * shade;
                let speed = if first_move {
                    format!(" F{feed}")
                } else {
                    String::new()
                };
                w.cut(
                    [x_at(edge), y],
                    operation.feed,
                    &format!(" S{}{speed}", num(full_power * shade)),
                );
                first_move = false;
                n = end + 1;
            }
            forward = !forward;
        }
    }
    w.sketch = true;
}

#[cfg(test)]
mod tests {
    use crate::generate::{generate_with, Program};
    use crate::geometry::Artwork;
    use crate::job::{Job, Machine, MachineKind, Operation, OperationKind, Origin};
    use crate::relief::Heightmap;

    fn map(width: usize, height: usize, value: impl Fn(usize, usize) -> f32) -> Heightmap {
        Heightmap {
            width,
            height,
            data: (0..width * height)
                .map(|i| value(i % width, i / width))
                .collect(),
            width_mm: width as f64,
            height_mm: height as f64,
        }
    }

    fn job(dither: bool) -> Job {
        Job {
            machine: Machine {
                kind: MachineKind::Laser,
                bed_width: 100.0,
                bed_height: 100.0,
                max_power_s: 1000.0,
                travel_feed: 6000.0,
                origin: Origin::BackLeft,
                start_gcode: String::new(),
                end_gcode: String::new(),
                tool_change: Default::default(),
            },
            offset: [10.0, 20.0],
            material_thickness: None,
            cuts_parts: false,
            operations: vec![Operation {
                color: None,
                kind: OperationKind::LaserImage {
                    interval: 1.0,
                    dither,
                },
                feed: 3000.0,
                power: 80.0,
                passes: 1,
                tool: None,
            }],
        }
    }

    fn engrave(image: &Heightmap, dither: bool) -> Result<Program, String> {
        let nothing = Artwork {
            width: 0.0,
            height: 0.0,
            shapes: Vec::new(),
        };
        generate_with(&nothing, Some(image), &job(dither))
    }

    fn code(program: &Program) -> Vec<&str> {
        program
            .text
            .lines()
            .filter(|l| !l.starts_with(';'))
            .collect()
    }

    #[test]
    fn shades_become_power_and_equal_dots_merge() {
        // One row: white, black, black, mid-grey.
        let image = map(4, 1, |x, _| [1.0, 0.0, 0.0, 0.5][x]);
        let program = engrave(&image, false).unwrap();
        assert_eq!(
            code(&program),
            [
                "G21",
                "G90",
                "M4 S0",
                // The white dot at the start of the row is skipped.
                "G0 X11 Y20.5 S0",
                // Two black dots in one move at the operation's 80% power.
                "G1 X13 Y20.5 S800 F3000",
                "G1 X14 Y20.5 S400",
                "M5",
                "G0 X0 Y0",
            ]
        );
    }

    #[test]
    fn rows_alternate_direction_and_blank_rows_are_skipped() {
        let image = map(3, 3, |_, y| if y == 1 { 1.0 } else { 0.0 });
        let program = engrave(&image, false).unwrap();
        let moves: Vec<_> = code(&program)
            .into_iter()
            .filter(|l| l.starts_with("G0 X") || l.starts_with("G1"))
            .collect();
        assert_eq!(
            moves,
            [
                "G0 X10 Y20.5 S0",
                "G1 X13 Y20.5 S800 F3000",
                // The white middle row is skipped; the next sweep runs right to left.
                "G0 X13 Y22.5 S0",
                "G1 X10 Y22.5 S800 F3000",
                "G0 X0 Y0",
            ]
        );
    }

    #[test]
    fn dithering_uses_only_full_power_and_keeps_the_average_shade() {
        let image = map(40, 40, |_, _| 0.75);
        let program = engrave(&image, true).unwrap();
        let powers: Vec<f32> = program
            .moves
            .chunks_exact(4)
            .map(|m| m[3])
            .filter(|p| *p >= 0.0)
            .collect();
        assert!(powers.iter().all(|&p| p == 0.0 || (p - 0.8).abs() < 1e-6));

        // A quarter-dark image should have about a quarter of its length burned.
        let mut burned = 0.0;
        let mut previous = [program.moves[0], program.moves[1]];
        for m in program.moves.chunks_exact(4) {
            if m[3] > 0.0 {
                burned += (m[0] - previous[0]).abs();
            }
            previous = [m[0], m[1]];
        }
        let fraction = burned / 1600.0;
        assert!((fraction - 0.25).abs() < 0.03, "burned fraction {fraction}");
    }

    #[test]
    fn a_white_image_and_a_missing_image_are_errors() {
        let white = map(4, 4, |_, _| 1.0);
        assert!(engrave(&white, false).is_err());
        let nothing = Artwork {
            width: 0.0,
            height: 0.0,
            shapes: Vec::new(),
        };
        assert!(generate_with(&nothing, None, &job(false))
            .unwrap_err()
            .contains("needs an image"));
    }

    #[test]
    fn image_moves_are_left_out_of_the_preview_path() {
        let image = map(4, 4, |_, _| 0.0);
        let program = engrave(&image, false).unwrap();
        assert!(program.cut_path.is_empty());
        assert!(program.cut_mm > 0.0);
    }
}
