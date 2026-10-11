//! Boxes held together by finger joints.
//!
//! Every panel starts as a plain rectangle of the box's outside size, and has
//! notches taken out of its edges one material-thickness deep. Where two
//! panels meet, one has notches exactly where the other has none, so the
//! panels interlock and no two of them ever claim the same space: the
//! thickness of all the panels adds up to the box's shell with nothing left
//! over and nothing missing. The tests check exactly that.

use std::collections::HashMap;

use serde::Deserialize;

use crate::{outline_path, Part, CUT_COLOR};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxSpec {
    /// Outside size, in millimetres.
    pub width: f64,
    pub depth: f64,
    pub height: f64,
    /// Thickness of the material.
    pub thickness: f64,
    /// The width to aim for in each finger; zero chooses one to suit.
    pub finger_width: f64,
    /// A closed box with a top panel, rather than an open one.
    pub lid: bool,
    /// Width of the cut itself: a laser's kerf, or a router bit's diameter.
    /// Outlines are moved outwards by half of it, so parts come out to size.
    pub cut_width: f64,
    /// Carry the cut into inside corners, so a round bit leaves room for the
    /// square corner of the finger that fits there.
    pub dogbones: bool,
}

/// How one edge of a panel is cut.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Edge {
    /// Left straight.
    Plain,
    /// Fingers at both ends: the second, fourth... segments are notched.
    Tabs,
    /// Notches at both ends: the first, third... segments are notched.
    Slots,
}

impl Edge {
    fn notched(self, segment: usize) -> bool {
        match self {
            Edge::Plain => false,
            Edge::Tabs => segment % 2 == 1,
            Edge::Slots => segment % 2 == 0,
        }
    }
}

/// The smallest gap, in millimetres, left between cuts on a finger or notch.
const SMALLEST_FEATURE: f64 = 0.2;

/// The panels of the box, with the cut width allowed for.
pub fn box_parts(spec: &BoxSpec) -> Result<Vec<Part>, String> {
    let BoxSpec {
        width,
        depth,
        height,
        thickness: t,
        ..
    } = *spec;
    let sizes = [("width", width), ("depth", depth), ("height", height)];
    if !sizes.iter().all(|(_, v)| v.is_finite() && *v > 0.0) || !(t.is_finite() && t > 0.0) {
        return Err(
            "The box's sizes and the material's thickness must be greater than zero.".into(),
        );
    }
    if !(spec.cut_width.is_finite() && spec.cut_width >= 0.0) {
        return Err("The width of the cut cannot be less than zero.".into());
    }
    let mut counts = [0usize; 3];
    for (i, (name, length)) in sizes.iter().enumerate() {
        counts[i] = finger_count(name, *length, spec)?;
    }
    let [across, back, up] = counts;

    let top = if spec.lid { Edge::Tabs } else { Edge::Plain };
    // Edges are given as top, right, bottom, left.
    let wall = [top, Edge::Tabs, Edge::Tabs, Edge::Tabs];
    let side = [top, Edge::Slots, Edge::Tabs, Edge::Slots];
    let flat = [Edge::Slots; 4];
    let mut panels = vec![
        ("front", width, height, across, up, wall),
        ("back", width, height, across, up, wall),
        ("left side", depth, height, back, up, side),
        ("right side", depth, height, back, up, side),
        ("bottom", width, depth, across, back, flat),
    ];
    if spec.lid {
        panels.push(("lid", width, depth, across, back, flat));
    }

    let half_cut = spec.cut_width / 2.0;
    panels
        .into_iter()
        .map(|(name, w, h, columns, rows, edges)| {
            let outline = panel_outline(w, h, t, columns, rows, edges)?;
            let points = allow_for_cut(&outline, half_cut, spec.dogbones);
            Ok(Part {
                name: name.to_owned(),
                width: w + spec.cut_width,
                height: h + spec.cut_width,
                // Moved so that the widened outline starts at the origin.
                markup: outline_path(
                    &points
                        .iter()
                        .map(|[x, y]| [x + half_cut, y + half_cut])
                        .collect::<Vec<_>>(),
                    CUT_COLOR,
                ),
            })
        })
        .collect()
}

/// How many segments an edge of `length` is divided into: an odd number, so
/// that both ends are alike and the joint looks the same from either side.
fn finger_count(name: &str, length: f64, spec: &BoxSpec) -> Result<usize, String> {
    let t = spec.thickness;
    // A finger narrower than twice the thickness is fragile, and one narrower
    // than the cut cannot be made at all.
    let narrowest = (2.0 * t).max(spec.cut_width + SMALLEST_FEATURE);
    let wanted = if spec.finger_width > 0.0 {
        spec.finger_width.max(narrowest)
    } else {
        (3.0 * t).max(narrowest)
    };
    let mut count = (length / wanted).floor() as usize;
    if count % 2 == 0 {
        count = count.saturating_sub(1);
    }
    let count = count.max(3);
    if length / count as f64 + 1e-9 < narrowest {
        return Err(format!(
            "The box's {name} of {length:.0} mm is too small for fingers in this material: it needs at least {:.0} mm.",
            (narrowest * 3.0).ceil()
        ));
    }
    Ok(count)
}

/// The outline of one panel before the cut is allowed for, as the corners of
/// a closed loop. `columns` and `rows` are the numbers of segments along the
/// horizontal and vertical edges.
fn panel_outline(
    width: f64,
    height: f64,
    t: f64,
    columns: usize,
    rows: usize,
    edges: [Edge; 4],
) -> Result<Vec<[f64; 2]>, String> {
    let [top, right, bottom, left] = edges;
    // The panel is cut into a grid of cells along every line where its
    // outline could turn. Cells inside a notch are dropped, and the outline
    // is whatever boundary the remaining cells have.
    let breaks = |length: f64, count: usize| {
        let mut at = vec![0.0, t, length - t, length];
        at.extend((1..count).map(|i| length * i as f64 / count as f64));
        at.sort_by(f64::total_cmp);
        at.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        at
    };
    let (xs, ys) = (breaks(width, columns), breaks(height, rows));
    let segment =
        |at: f64, length: f64, count: usize| ((at / length * count as f64) as usize).min(count - 1);
    let present = |i: usize, j: usize| {
        let (x, y) = ((xs[i] + xs[i + 1]) / 2.0, (ys[j] + ys[j + 1]) / 2.0);
        let (column, row) = (segment(x, width, columns), segment(y, height, rows));
        !((y < t && top.notched(column))
            || (y > height - t && bottom.notched(column))
            || (x < t && left.notched(row))
            || (x > width - t && right.notched(row)))
    };
    let (nx, ny) = (xs.len() - 1, ys.len() - 1);
    let filled = |i: isize, j: isize| {
        i >= 0
            && j >= 0
            && (i as usize) < nx
            && (j as usize) < ny
            && present(i as usize, j as usize)
    };

    // Every side of a kept cell that faces a dropped one is a piece of the
    // outline. Each is walked with the material on the same hand.
    let mut next: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    for i in 0..nx {
        for j in 0..ny {
            if !present(i, j) {
                continue;
            }
            let (a, b) = (i as isize, j as isize);
            let sides = [
                (filled(a, b - 1), (i, j), (i + 1, j)),
                (filled(a + 1, b), (i + 1, j), (i + 1, j + 1)),
                (filled(a, b + 1), (i + 1, j + 1), (i, j + 1)),
                (filled(a - 1, b), (i, j + 1), (i, j)),
            ];
            for (covered, from, to) in sides {
                if !covered && next.insert(from, to).is_some() {
                    return Err("The box's panels could not be drawn at these sizes.".into());
                }
            }
        }
    }
    let Some(&start) = next.keys().min() else {
        return Err("The box's panels could not be drawn at these sizes.".into());
    };
    let mut corners: Vec<(usize, usize)> = Vec::new();
    let mut at = start;
    loop {
        corners.push(at);
        at = next
            .remove(&at)
            .ok_or("The box's panels could not be drawn at these sizes.")?;
        if at == start {
            break;
        }
    }
    // Anything left over would be a second, separate piece.
    if !next.is_empty() {
        return Err("The fingers are too small for this material; make the box larger or the fingers wider.".into());
    }

    // Only the corners where the outline turns are kept.
    let count = corners.len();
    let turns = |k: usize| {
        let (before, here, after) = (
            corners[(k + count - 1) % count],
            corners[k],
            corners[(k + 1) % count],
        );
        (here.0 == before.0) != (after.0 == here.0)
    };
    Ok((0..count)
        .filter(|&k| turns(k))
        .map(|k| [xs[corners[k].0], ys[corners[k].1]])
        .collect())
}

/// Moves a square-cornered outline away from the material by `half_cut`, so
/// that a cut of that half-width leaves the part at its true size. With
/// `dogbones`, each inside corner gets a short diagonal excursion that takes
/// the cut right into the corner.
///
/// The outline must be walked as [`panel_outline`] walks it, with the
/// material towards (-dy, dx) of each step.
fn allow_for_cut(outline: &[[f64; 2]], half_cut: f64, dogbones: bool) -> Vec<[f64; 2]> {
    let count = outline.len();
    // Each step runs along one axis, so its direction is -1, 0 or 1 on each.
    let sign = |d: f64| if d.abs() < 1e-9 { 0.0 } else { d.signum() };
    let direction = |from: [f64; 2], to: [f64; 2]| [sign(to[0] - from[0]), sign(to[1] - from[1])];
    let mut moved = Vec::with_capacity(count);
    for k in 0..count {
        let here = outline[k];
        let arriving = direction(outline[(k + count - 1) % count], here);
        let leaving = direction(here, outline[(k + 1) % count]);
        // Away from the material, for each of the two edges meeting here.
        let away = [arriving[1] + leaving[1], -arriving[0] - leaving[0]];
        let corner = [here[0] + away[0] * half_cut, here[1] + away[1] * half_cut];
        moved.push(corner);
        // Turning away from the material makes an inside corner.
        let inside = arriving[0] * leaving[1] - arriving[1] * leaving[0] < 0.0;
        if dogbones && inside && half_cut > 0.0 {
            // The cut's centre stops half a cut short of the true corner, so
            // its edge just reaches it.
            let reach = half_cut / std::f64::consts::SQRT_2;
            moved.push([here[0] + away[0] * reach, here[1] + away[1] * reach]);
            moved.push(corner);
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> BoxSpec {
        BoxSpec {
            width: 120.0,
            depth: 80.0,
            height: 60.0,
            thickness: 3.0,
            finger_width: 0.0,
            lid: true,
            cut_width: 0.0,
            dogbones: false,
        }
    }

    /// Corner points of a part's outline, read back from its path.
    fn points(part: &Part) -> Vec<[f64; 2]> {
        let d = part.markup.split('"').nth(1).unwrap();
        d.trim_end_matches('Z')
            .split(['M', 'L'])
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (x, y) = pair.split_once(' ').unwrap();
                [x.parse().unwrap(), y.parse().unwrap()]
            })
            .collect()
    }

    fn area(points: &[[f64; 2]]) -> f64 {
        let n = points.len();
        (0..n)
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % n]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0
    }

    fn material(parts: &[Part], thickness: f64) -> f64 {
        parts.iter().map(|p| area(&points(p)) * thickness).sum()
    }

    #[test]
    fn the_panels_of_a_closed_box_make_up_its_shell_exactly() {
        let s = spec();
        let parts = box_parts(&s).unwrap();
        assert_eq!(parts.len(), 6);
        let t = s.thickness;
        let shell = s.width * s.depth * s.height
            - (s.width - 2.0 * t) * (s.depth - 2.0 * t) * (s.height - 2.0 * t);
        assert!((material(&parts, t) - shell).abs() < 1e-6);
    }

    #[test]
    fn the_same_holds_for_an_open_box_and_other_sizes() {
        for (width, depth, height, t, finger) in [
            (120.0, 80.0, 60.0, 3.0, 0.0),
            (200.0, 150.0, 45.5, 6.35, 20.0),
            (60.0, 60.0, 60.0, 4.0, 9.0),
        ] {
            let s = BoxSpec {
                width,
                depth,
                height,
                thickness: t,
                finger_width: finger,
                lid: false,
                ..spec()
            };
            let parts = box_parts(&s).unwrap();
            assert_eq!(parts.len(), 5);
            let shell =
                width * depth * height - (width - 2.0 * t) * (depth - 2.0 * t) * (height - t);
            assert!(
                (material(&parts, t) - shell).abs() < 1e-6,
                "{width} x {depth} x {height}"
            );
        }
    }

    #[test]
    fn outlines_have_square_corners_and_fit_their_panel() {
        for part in box_parts(&spec()).unwrap() {
            let outline = points(&part);
            let n = outline.len();
            for i in 0..n {
                let (a, b) = (outline[i], outline[(i + 1) % n]);
                assert!(
                    (a[0] - b[0]).abs() < 1e-9 || (a[1] - b[1]).abs() < 1e-9,
                    "{}",
                    part.name
                );
                assert!(a[0] >= -1e-9 && a[0] <= part.width + 1e-9);
                assert!(a[1] >= -1e-9 && a[1] <= part.height + 1e-9);
            }
        }
    }

    #[test]
    fn allowing_for_the_cut_grows_every_part_by_its_width() {
        let plain = box_parts(&spec()).unwrap();
        let allowed = box_parts(&BoxSpec {
            cut_width: 0.2,
            ..spec()
        })
        .unwrap();
        for (before, after) in plain.iter().zip(&allowed) {
            assert!((after.width - before.width - 0.2).abs() < 1e-9);
            let (xs, ys): (Vec<f64>, Vec<f64>) = points(after).iter().map(|p| (p[0], p[1])).unzip();
            assert!(xs.iter().cloned().fold(f64::MAX, f64::min).abs() < 1e-6);
            assert!((ys.iter().cloned().fold(f64::MIN, f64::max) - after.height).abs() < 1e-6);
            // A wider part by half a cut all round has more area.
            assert!(area(&points(after)) > area(&points(before)));
            assert_eq!(points(after).len(), points(before).len());
        }
    }

    #[test]
    fn dogbones_reach_into_every_inside_corner() {
        let bit = 3.175;
        let s = BoxSpec {
            cut_width: bit,
            ..spec()
        };
        let plain = box_parts(&s).unwrap();
        let relieved = box_parts(&BoxSpec {
            dogbones: true,
            ..s.clone()
        })
        .unwrap();
        let nominal = box_parts(&spec()).unwrap();
        for ((before, after), true_size) in plain.iter().zip(&relieved).zip(&nominal) {
            let extra = points(after).len() - points(before).len();
            assert!(extra > 0 && extra % 2 == 0, "{}", before.name);
            // Each excursion ends half a cut from a true inside corner.
            let corners = points(true_size);
            let reached = points(after)
                .iter()
                .filter(|p| {
                    !points(before)
                        .iter()
                        .any(|q| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-6)
                })
                .all(|p| {
                    corners.iter().any(|c| {
                        ((p[0] - bit / 2.0 - c[0]).hypot(p[1] - bit / 2.0 - c[1]) - bit / 2.0).abs()
                            < 1e-3
                    })
                });
            assert!(reached, "{}", before.name);
        }
    }

    #[test]
    fn a_box_too_small_for_its_material_is_refused() {
        let problem = box_parts(&BoxSpec {
            height: 10.0,
            ..spec()
        })
        .unwrap_err();
        assert!(problem.contains("height"), "{problem}");
        // A bit wider than the fingers could be.
        assert!(box_parts(&BoxSpec {
            depth: 18.0,
            cut_width: 6.35,
            ..spec()
        })
        .is_err());
        assert!(box_parts(&BoxSpec {
            depth: 20.0,
            cut_width: 6.35,
            ..spec()
        })
        .is_ok());
        assert!(box_parts(&BoxSpec {
            thickness: 0.0,
            ..spec()
        })
        .is_err());
    }
}
