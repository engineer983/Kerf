//! Turning artwork and a job into a GRBL G-code program.

use std::fmt::Write as _;
use std::ops::Range;

use serde::Serialize;

use crate::geometry::{distance, Artwork, Polyline, Shape};
use crate::job::{ChangePlace, Job, Machine, Operation, OperationKind, Origin};
use crate::relief::Heightmap;
use crate::tool_change::{self, Layout, Plan, ProgramPart, PART_HEADER};

/// Seconds to wait for the spindle to reach speed before cutting.
pub(crate) const SPINDLE_DWELL_S: f64 = 2.0;
/// Slack when checking the job against the bed, for rounding in the artwork.
const BED_SLACK_MM: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Program {
    /// The whole program. Every part's stream, joined in order.
    pub text: String,
    pub line_count: usize,
    /// Extent of all cutting moves, in machine coordinates.
    pub bounds: Bounds,
    pub cut_mm: f64,
    pub travel_mm: f64,
    /// Rough estimate; ignores acceleration.
    pub seconds: f64,
    /// SVG path data for previews, in bed coordinates (Y down).
    pub cut_path: String,
    pub travel_path: String,
    /// Every move in order, for playback: x, y, z, power per move, in bed
    /// coordinates (Y down) with Z zero at the material's top. Power is -1 for
    /// a rapid move, otherwise 0 to 1 (always 1 for a router). The first entry
    /// is where the job starts.
    pub moves: Vec<f32>,
    /// The index in `moves` of the first move of each operation, in order, so
    /// that a preview can tell which bit made which cut.
    pub operation_starts: Vec<usize>,
    /// One per bit, in order, for a job whose bits are changed by hand;
    /// otherwise one for the whole job.
    pub parts: Vec<ProgramPart>,
    /// The job moves in machine coordinates (`G53`) to change bits, which
    /// needs a machine that has homed.
    pub needs_homing: bool,
    /// The bits are changed by an automatic tool changer at each `M6`.
    pub atc: bool,
    /// How high above the work zero the bit is raised at each change, when
    /// it is raised over the work zero.
    pub change_lift: Option<f64>,
    /// Where the job goes in machine coordinates, for the sender to check.
    pub machine_moves: Vec<[f64; 3]>,
    /// For a change at a fixed place: how far above the work zero the bit
    /// must be when it goes there (the larger of the lift and the job's
    /// highest safe height), for the sender to check against the work.
    pub change_clearance: Option<f64>,
    #[serde(skip)]
    pub(crate) layout: Layout,
}

pub fn generate(artwork: &Artwork, job: &Job) -> Result<Program, String> {
    generate_with(artwork, None, job)
}

/// Like [`generate`], with the height map that 3D relief operations carve.
pub fn generate_with(
    artwork: &Artwork,
    heightmap: Option<&Heightmap>,
    job: &Job,
) -> Result<Program, String> {
    generate_composed(artwork, heightmap, job, None, None)
}

/// Lettering and drawings laid out on the material beside the job's main
/// artwork, made by operations of their own.
pub struct Additions<'a> {
    /// Drawn where it sits on the bed, so it needs no offset.
    pub artwork: &'a Artwork,
    /// Run after the job's own marking operations and before anything of
    /// the job's that cuts pieces free (see [`Job::cuts_free`]).
    pub operations: &'a [Operation],
}

fn is_cutout(operation: &Operation) -> bool {
    matches!(
        operation.kind,
        OperationKind::CncCutout { .. } | OperationKind::LaserCutout { .. }
    )
}

/// Like [`generate_with`], for a job with things added beside its main
/// artwork. Whatever order the operations are given in, those that cut
/// pieces free run after the rest, and cutouts last of all, so that nothing
/// is engraved on a piece that has already been cut free.
/// `cutout_around` is the rectangle, in bed coordinates as x, y, width and
/// height, that cutouts go round in place of the main artwork's own: the
/// space taken up by everything on the material.
pub fn generate_composed(
    artwork: &Artwork,
    heightmap: Option<&Heightmap>,
    job: &Job,
    additions: Option<&Additions>,
    cutout_around: Option<[f64; 4]>,
) -> Result<Program, String> {
    let added = additions.map_or(&[][..], |a| a.operations);
    if job.operations.is_empty() && added.is_empty() {
        return Err("Add at least one operation.".into());
    }
    for operation in job.operations.iter().chain(added) {
        operation.validate(&job.machine)?;
    }
    job.check(added)?;
    // Each operation, with whether it works on the additions. Whatever cuts
    // pieces free runs after everything that marks them, or a piece could
    // drop or shift before it is marked: the main artwork's marks, then the
    // additions, then its cuts right through, then cutouts.
    let main = |cuts: bool, cutouts: bool| {
        job.operations
            .iter()
            .filter(move |o| job.cuts_free(o) == cuts && is_cutout(o) == cutouts)
            .map(|o| (o, false))
    };
    let mut through: Vec<(&Operation, bool)> = main(true, false).collect();
    if job.cuts_parts && job.machine.kind == crate::job::MachineKind::Laser {
        // A design's lines burn weakest first, so its marks are made before
        // the parts are cut free however they are listed. Equal powers keep
        // their listed order.
        through.sort_by(|a, b| a.0.power.total_cmp(&b.0.power));
    }
    let order: Vec<(&Operation, bool)> = main(false, false)
        .chain(added.iter().map(|o| (o, true)))
        .chain(through)
        .chain(main(true, true))
        .collect();
    let (cut_size, cut_at) = match cutout_around {
        Some([x, y, width, height]) => ([width, height], [x, y]),
        None => ([artwork.width, artwork.height], job.offset),
    };

    let running: Vec<&Operation> = order.iter().map(|(o, _)| *o).collect();
    let plan = Plan::new(job, &running);
    let tool_at = |i: usize| running.get(i).and_then(|o| o.tool.as_ref());

    let mut w = Writer::new(&job.machine);
    w.line(&format!(
        "; Generated by Kerf {}",
        env!("CARGO_PKG_VERSION")
    ));
    w.lines(&job.machine.start_gcode);
    let header = w.out.clone();
    if plan.plain() {
        // A job with one bit is written exactly as it always was.
        w.line("G21");
        w.line("G90");
    } else {
        w.line(PART_HEADER);
    }
    if let (true, Some(first)) = (plan.atc, tool_at(0)) {
        // An automatic changer loads the first bit before anything else,
        // with the spindle raised clear first.
        w.z_move(plan.lift, None);
        w.line(&format!("T{} M6", first.number));
        // The changer may leave the head anywhere: travel before plunging.
        w.force_travel = true;
    }

    // Where each part starts in the text, where its operations start and
    // end, and its first operation.
    struct Mark {
        start: usize,
        body: Range<usize>,
        first_operation: usize,
    }
    let mut marks = vec![Mark {
        start: 0,
        body: w.out.len()..0,
        first_operation: 0,
    }];
    let mut change_moves = Vec::new();
    let changes = plan.starts.len();

    let mut operation_starts = Vec::with_capacity(order.len());
    for (index, (operation, on_additions)) in order.into_iter().enumerate() {
        if let Some(change) = plan.starts.iter().position(|&s| s == index) {
            let next = tool_at(index);
            if plan.atc {
                tool_change::write_atc_change(&mut w, &plan, next);
            } else {
                if let Some(last) = marks.last_mut() {
                    last.body.end = w.out.len();
                }
                change_moves.push(tool_change::write_change(
                    &mut w,
                    &plan,
                    change + 1,
                    changes,
                    next,
                ));
                let start = w.out.len();
                w.line(&format!(
                    "; Part {} of {}: {}",
                    change + 2,
                    changes + 1,
                    tool_change::fit(next)
                ));
                w.line(PART_HEADER);
                // A part always says where it starts in X and Y before it
                // plunges: the change place may be anywhere.
                w.force_travel = true;
                marks.push(Mark {
                    start,
                    body: w.out.len()..0,
                    first_operation: index,
                });
            }
        }
        operation_starts.push(w.moves.len() / 4);
        let (artwork, offset) = match additions {
            Some(additions) if on_additions => (additions.artwork, [0.0, 0.0]),
            _ => (artwork, job.offset),
        };
        let shapes: Vec<&Shape> = artwork
            .shapes
            .iter()
            .filter(|s| operation.color.is_none() || s.color == operation.color)
            .collect();
        let placed: Vec<Vec<Polyline>> = shapes
            .iter()
            .map(|shape| {
                shape
                    .polylines
                    .iter()
                    .map(|polyline| Polyline {
                        points: polyline
                            .points
                            .iter()
                            .map(|p| [p[0] + offset[0], p[1] + offset[1]])
                            .collect(),
                        closed: polyline.closed,
                    })
                    .collect()
            })
            .collect();

        match operation.kind {
            OperationKind::LaserCut => {
                let polylines: Vec<&Polyline> = placed.iter().flatten().collect();
                laser_cut(&mut w, operation, &polylines);
            }
            OperationKind::LaserFill { interval } => {
                laser_fill(&mut w, operation, &placed, interval)
            }
            OperationKind::CncProfile { .. } => {
                let polylines: Vec<&Polyline> = placed.iter().flatten().collect();
                cnc_profile(&mut w, operation, &polylines);
            }
            OperationKind::LaserImage { .. } | OperationKind::CncRelief { .. } if on_additions => {
                return Err("Added text and drawings are lines, so they cannot be engraved as a picture or carved as a relief.".into());
            }
            OperationKind::LaserImage { .. } => {
                let image =
                    heightmap.ok_or("Image engraving needs an image to take shades from.")?;
                crate::image_engrave::engrave(&mut w, operation, image, job.offset);
            }
            OperationKind::LaserCutout { .. } => {
                let line = crate::cutout::laser_line(operation, cut_size, cut_at)?;
                laser_cut(&mut w, operation, &[&line]);
            }
            OperationKind::CncCutout { .. } => {
                crate::cutout::cut(&mut w, operation, cut_size, cut_at)?;
            }
            OperationKind::CncRelief { .. } => {
                let heightmap =
                    heightmap.ok_or("A 3D relief needs an image to take heights from.")?;
                crate::relief::carve(&mut w, operation, heightmap, job.offset)?;
            }
        }
    }

    let Some(bounds) = w.bounds else {
        return Err("The operations do not match any shape in the artwork.".into());
    };
    if bounds.min_x < -BED_SLACK_MM
        || bounds.min_y < -BED_SLACK_MM
        || bounds.max_x > job.machine.bed_width + BED_SLACK_MM
        || bounds.max_y > job.machine.bed_height + BED_SLACK_MM
    {
        return Err(format!(
            "The job (X {:.1} to {:.1}, Y {:.1} to {:.1} mm) does not fit the {} × {} mm bed.",
            bounds.min_x,
            bounds.max_x,
            bounds.min_y,
            bounds.max_y,
            job.machine.bed_width,
            job.machine.bed_height
        ));
    }

    if let Some(last) = marks.last_mut() {
        last.body.end = w.out.len();
    }
    w.line("M5");
    w.travel(w.origin(), "");
    let end_start = w.out.len();
    w.lines(&job.machine.end_gcode);
    let end = w.out[end_start..].to_owned();

    let operation_count = operation_starts.len();
    let text_len = w.out.len();
    let lines_before = |at: usize| w.out[..at].matches('\n').count();
    let parts: Vec<ProgramPart> = marks
        .iter()
        .enumerate()
        .map(|(i, mark)| {
            let stream_end = marks.get(i + 1).map_or(text_len, |m| m.start);
            let last_operation = marks
                .get(i + 1)
                .map_or(operation_count, |m| m.first_operation);
            ProgramPart {
                tool: tool_at(mark.first_operation).cloned(),
                operations: mark.first_operation..last_operation,
                first_line: lines_before(mark.start),
                line_count: w.out[mark.start..stream_end].matches('\n').count(),
                stream: mark.start..stream_end,
                body: mark.body.clone(),
            }
        })
        .collect();
    let split = plan.split();
    let over_work_zero = plan.place == ChangePlace::OverWorkZero;
    let machine_moves = match plan.place {
        ChangePlace::Machine { position } if split => vec![position],
        _ => Vec::new(),
    };

    Ok(Program {
        line_count: w.out.lines().count(),
        text: w.out,
        bounds,
        cut_mm: w.cut_mm,
        travel_mm: w.travel_mm,
        seconds: w.seconds,
        cut_path: w.cut_path,
        travel_path: w.travel_path,
        moves: w.moves,
        operation_starts,
        parts,
        needs_homing: !machine_moves.is_empty(),
        atc: plan.atc,
        change_lift: ((split && over_work_zero) || plan.atc).then_some(plan.lift),
        change_clearance: (split && !over_work_zero).then_some(plan.lift),
        machine_moves,
        layout: Layout {
            header,
            end,
            spin_down: plan.spin_down,
            lift: plan.lift,
            change_moves,
        },
    })
}

fn laser_cut(w: &mut Writer, operation: &Operation, polylines: &[&Polyline]) {
    let power = w.machine.max_power_s * operation.power / 100.0;
    w.power = operation.power / 100.0;
    w.line("M4 S0");
    for polyline in order_by_travel(polylines, w.position) {
        let mut points = polyline.points.clone();
        // With the laser off, a rapid move carries S0 so nothing burns in transit
        // even if GRBL's laser mode ($32) is disabled.
        w.travel(points[0], " S0");
        for pass in 0..operation.passes {
            if pass > 0 && !polyline.closed {
                points.reverse();
            }
            for (i, &point) in points.iter().enumerate().skip(1) {
                if pass == 0 && i == 1 {
                    w.cut(
                        point,
                        operation.feed,
                        &format!(" S{} F{}", num(power), num(operation.feed)),
                    );
                } else {
                    w.cut(point, operation.feed, "");
                }
            }
        }
    }
}

fn laser_fill(w: &mut Writer, operation: &Operation, shapes: &[Vec<Polyline>], interval: f64) {
    let ys = shapes
        .iter()
        .flatten()
        .flat_map(|p| p.points.iter().map(|p| p[1]));
    let (top, bottom) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| {
        (lo.min(y), hi.max(y))
    });
    if !top.is_finite() {
        return;
    }
    let power = w.machine.max_power_s * operation.power / 100.0;
    w.power = operation.power / 100.0;
    let rows = ((bottom - top) / interval).ceil() as usize;
    let on = format!(" S{} F{}", num(power), num(operation.feed));

    w.line("M4 S0");
    for _ in 0..operation.passes {
        for row in 0..rows {
            let y = top + (row as f64 + 0.5) * interval;
            let mut spans = scanline_spans(shapes, y);
            // Sweep back and forth to avoid a long return move after every row.
            if row % 2 == 1 {
                spans.reverse();
                for span in &mut spans {
                    span.swap(0, 1);
                }
            }
            for [from, to] in spans {
                w.travel([from, y], " S0");
                w.cut([to, y], operation.feed, &on);
            }
        }
    }
}

/// The parts of the horizontal line at `y` that lie inside any shape, left to
/// right. Each shape is filled by the even-odd rule, so subpaths cut holes.
fn scanline_spans(shapes: &[Vec<Polyline>], y: f64) -> Vec<[f64; 2]> {
    let mut spans: Vec<[f64; 2]> = Vec::new();
    for shape in shapes {
        let mut crossings = Vec::new();
        for polyline in shape {
            let points = &polyline.points;
            // An open subpath is closed implicitly, as SVG does when filling.
            let closing = [points[points.len() - 1], points[0]];
            let edges = points.windows(2).map(|e| [e[0], e[1]]).chain([closing]);
            for [a, b] in edges {
                if (a[1] <= y) != (b[1] <= y) {
                    crossings.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        crossings.sort_by(f64::total_cmp);
        spans.extend(crossings.chunks_exact(2).map(|pair| [pair[0], pair[1]]));
    }

    // Merge spans from overlapping shapes so no stretch is burned twice.
    spans.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f64; 2]> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span[0] <= last[1] => last[1] = last[1].max(span[1]),
            _ => merged.push(span),
        }
    }
    merged.retain(|span| span[1] - span[0] > 1e-6);
    merged
}

fn cnc_profile(w: &mut Writer, operation: &Operation, polylines: &[&Polyline]) {
    let OperationKind::CncProfile {
        depth,
        depth_per_pass,
        plunge_feed,
        safe_z,
        spindle_rpm,
    } = operation.kind
    else {
        return;
    };
    let passes = (depth / depth_per_pass - 1e-9).ceil().max(1.0) as usize;

    w.power = 1.0;
    w.z_move(safe_z, None);
    w.line(&format!("M3 S{}", num(spindle_rpm)));
    w.line(&format!("G4 P{}", num(SPINDLE_DWELL_S)));
    w.seconds += SPINDLE_DWELL_S;

    for polyline in order_by_travel(polylines, w.position) {
        let mut points = polyline.points.clone();
        w.travel(points[0], "");
        for pass in 1..=passes {
            let target = -(depth_per_pass * pass as f64).min(depth);
            w.z_move(target, Some(plunge_feed));
            if pass > 1 && !polyline.closed {
                points.reverse();
            }
            for (i, &point) in points.iter().enumerate().skip(1) {
                let feed = if i == 1 {
                    format!(" F{}", num(operation.feed))
                } else {
                    String::new()
                };
                w.cut(point, operation.feed, &feed);
            }
        }
        w.z_move(safe_z, None);
    }
}

/// Orders paths to shorten travel moves, nearest start point first, except
/// that anything inside a closed path is cut before it. Cutting an outline
/// first can free the piece, which a router bit may then throw, or which can
/// shift or tip up before the cuts inside it are made.
fn order_by_travel<'a>(polylines: &[&'a Polyline], from: [f64; 2]) -> Vec<&'a Polyline> {
    let boxes: Vec<[f64; 4]> = polylines.iter().map(|p| bounding_box(&p.points)).collect();
    // For each path, the closed paths it lies inside, and for each closed
    // path, how many paths inside it are still to be cut.
    let mut containers: Vec<Vec<usize>> = vec![Vec::new(); polylines.len()];
    let mut uncut_inside = vec![0usize; polylines.len()];
    for (outer, outline) in polylines.iter().enumerate() {
        if !outline.closed || outline.points.len() < 3 {
            continue;
        }
        for (inner, path) in polylines.iter().enumerate() {
            // A strictly smaller box rules out cycles, so some path is always
            // ready to cut.
            if inner != outer
                && box_within(boxes[inner], boxes[outer])
                && box_area(boxes[inner]) < box_area(boxes[outer]) - 1e-9
                && inside(path.points[0], &outline.points)
            {
                containers[inner].push(outer);
                uncut_inside[outer] += 1;
            }
        }
    }

    let mut remaining: Vec<usize> = (0..polylines.len()).collect();
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut position = from;
    while !remaining.is_empty() {
        let (slot, _) = remaining
            .iter()
            .enumerate()
            .filter(|(_, &i)| uncut_inside[i] == 0)
            .map(|(slot, &i)| (slot, distance(position, polylines[i].points[0])))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .expect("a path with nothing left inside it");
        let index = remaining.swap_remove(slot);
        for &outer in &containers[index] {
            uncut_inside[outer] -= 1;
        }
        let polyline = polylines[index];
        // A closed path, or an open one cut an even number of times, ends where it starts.
        position = polyline.points[0];
        ordered.push(polyline);
    }
    ordered
}

/// `[min_x, min_y, max_x, max_y]`.
fn bounding_box(points: &[[f64; 2]]) -> [f64; 4] {
    points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

fn box_within(inner: [f64; 4], outer: [f64; 4]) -> bool {
    inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
}

fn box_area(b: [f64; 4]) -> f64 {
    (b[2] - b[0]) * (b[3] - b[1])
}

/// Whether `point` is inside the polygon `outline`, by the even-odd rule.
fn inside(point: [f64; 2], outline: &[[f64; 2]]) -> bool {
    let mut result = false;
    let mut previous = outline[outline.len() - 1];
    for &current in outline {
        if (current[1] > point[1]) != (previous[1] > point[1]) {
            let x = current[0]
                + (point[1] - current[1]) / (previous[1] - current[1]) * (previous[0] - current[0]);
            if point[0] < x {
                result = !result;
            }
        }
        previous = current;
    }
    result
}

/// Formats a number with up to three decimals and no trailing zeros.
pub(crate) fn num(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// Accumulates G-code text along with statistics and preview paths.
/// Positions are in bed coordinates (Y down); output is in machine coordinates.
pub(crate) struct Writer<'a> {
    pub(crate) machine: &'a Machine,
    out: String,
    position: [f64; 2],
    bounds: Option<Bounds>,
    cut_mm: f64,
    travel_mm: f64,
    pub(crate) seconds: f64,
    cut_path: String,
    travel_path: String,
    /// Height of the tool; callers that put Z on a cutting line set it first.
    pub(crate) z: f64,
    /// Laser power, 0 to 1, recorded with each cutting move.
    pub(crate) power: f64,
    /// Whether cutting moves are added to the preview path. Image engraving
    /// turns this off: its hundreds of thousands of short moves would make
    /// the path too large to draw.
    pub(crate) sketch: bool,
    /// The next travel is written even if it goes nowhere: after a move
    /// the writer does not follow (such as one in machine coordinates), the
    /// head may not be where it thinks.
    pub(crate) force_travel: bool,
    moves: Vec<f32>,
}

impl<'a> Writer<'a> {
    fn new(machine: &'a Machine) -> Self {
        let mut writer = Self {
            machine,
            out: String::new(),
            position: [0.0, 0.0],
            bounds: None,
            cut_mm: 0.0,
            travel_mm: 0.0,
            seconds: 0.0,
            cut_path: String::new(),
            travel_path: String::new(),
            z: 0.0,
            power: 1.0,
            sketch: true,
            force_travel: false,
            moves: Vec::new(),
        };
        // Jobs are assumed to start with the head at the machine origin.
        writer.position = writer.origin();
        writer.record(-1.0);
        writer
    }

    pub(crate) fn line(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn machine_xy(&self, p: [f64; 2]) -> [f64; 2] {
        match self.machine.origin {
            Origin::FrontLeft => [p[0], self.machine.bed_height - p[1]],
            Origin::BackLeft => p,
        }
    }

    /// The machine's X0 Y0 in bed coordinates.
    fn origin(&self) -> [f64; 2] {
        match self.machine.origin {
            Origin::FrontLeft => [0.0, self.machine.bed_height],
            Origin::BackLeft => [0.0, 0.0],
        }
    }

    /// Emits each non-blank line of a user-supplied block.
    fn lines(&mut self, block: &str) {
        for line in block.lines().map(str::trim).filter(|l| !l.is_empty()) {
            self.line(line);
        }
    }

    fn include(&mut self, p: [f64; 2]) {
        let [x, y] = self.machine_xy(p);
        let b = self.bounds.get_or_insert(Bounds {
            min_x: x,
            min_y: y,
            max_x: x,
            max_y: y,
        });
        b.min_x = b.min_x.min(x);
        b.min_y = b.min_y.min(y);
        b.max_x = b.max_x.max(x);
        b.max_y = b.max_y.max(y);
    }

    pub(crate) fn travel(&mut self, to: [f64; 2], suffix: &str) {
        let length = distance(self.position, to);
        if length < 1e-9 && !self.force_travel {
            return;
        }
        self.force_travel = false;
        let [x, y] = self.machine_xy(to);
        self.line(&format!("G0 X{} Y{}{suffix}", num(x), num(y)));
        segment(&mut self.travel_path, self.position, to);
        self.travel_mm += length;
        self.seconds += length / self.machine.travel_feed * 60.0;
        self.position = to;
        self.record(-1.0);
    }

    /// A rapid back over the origin, written even when already there, so a
    /// bit change always says where it happens.
    pub(crate) fn travel_to_origin(&mut self) {
        let origin = self.origin();
        if distance(self.position, origin) < 1e-9 {
            let [x, y] = self.machine_xy(origin);
            self.line(&format!("G0 X{} Y{}", num(x), num(y)));
        } else {
            self.travel(origin, "");
        }
    }

    /// How much text has been written so far.
    pub(crate) fn written(&self) -> usize {
        self.out.len()
    }

    /// The text written since `start`.
    pub(crate) fn written_since(&self, start: usize) -> &str {
        &self.out[start..]
    }

    pub(crate) fn cut(&mut self, to: [f64; 2], feed: f64, suffix: &str) {
        let [x, y] = self.machine_xy(to);
        self.line(&format!("G1 X{} Y{}{suffix}", num(x), num(y)));
        if self.sketch {
            segment(&mut self.cut_path, self.position, to);
        }
        self.include(self.position);
        self.include(to);
        let length = distance(self.position, to);
        self.cut_mm += length;
        self.seconds += length / feed * 60.0;
        self.position = to;
        self.record(self.power);
    }

    /// A vertical move: a rapid when `feed` is `None`, otherwise a plunge or lift at that feed.
    pub(crate) fn z_move(&mut self, z: f64, feed: Option<f64>) {
        match feed {
            Some(feed) => {
                self.line(&format!("G1 Z{} F{}", num(z), num(feed)));
                self.seconds += (self.z - z).abs() / feed * 60.0;
            }
            None => self.line(&format!("G0 Z{}", num(z))),
        }
        self.z = z;
        self.record(if feed.is_some() { self.power } else { -1.0 });
    }

    fn record(&mut self, power: f64) {
        self.moves.extend([
            self.position[0] as f32,
            self.position[1] as f32,
            self.z as f32,
            power as f32,
        ]);
    }
}

fn segment(path: &mut String, from: [f64; 2], to: [f64; 2]) {
    let _ = write!(
        path,
        "M{} {}L{} {}",
        num(from[0]),
        num(from[1]),
        num(to[0]),
        num(to[1])
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::MachineKind;

    fn machine(kind: MachineKind) -> Machine {
        Machine {
            kind,
            bed_width: 100.0,
            bed_height: 100.0,
            max_power_s: 1000.0,
            travel_feed: 6000.0,
            origin: Origin::FrontLeft,
            start_gcode: String::new(),
            end_gcode: String::new(),
            tool_change: Default::default(),
        }
    }

    #[test]
    fn back_left_origin_keeps_y_pointing_forward() {
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.machine.origin = Origin::BackLeft;
        job.machine.start_gcode = "M14S1\n\n  M15S1\n".into();
        job.machine.end_gcode = "M6".into();
        let program = generate(&square(), &job).unwrap();
        // The artwork's top-left is 30 mm from the back, which is now Y30.
        assert_eq!(
            code(&program),
            [
                "M14S1",
                "M15S1",
                "G21",
                "G90",
                "M4 S0",
                "G0 X20 Y30 S0",
                "G1 X30 Y30 S500 F600",
                "G1 X30 Y40",
                "G1 X20 Y40",
                "G1 X20 Y30",
                "M5",
                "G0 X0 Y0",
                "M6",
            ]
        );
        assert_bounds(&program, [20.0, 30.0, 30.0, 40.0]);
    }

    fn artwork(size_mm: u32, body: &str) -> Artwork {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size_mm}mm" height="{size_mm}mm" viewBox="0 0 {size_mm} {size_mm}">{body}</svg>"#
        );
        Artwork::from_svg(&svg, 0.05).unwrap()
    }

    fn square() -> Artwork {
        artwork(10, r##"<path d="M0 0H10V10H0Z" fill="#000000"/>"##)
    }

    fn laser_job(kind: OperationKind, passes: u32) -> Job {
        Job {
            machine: machine(MachineKind::Laser),
            offset: [20.0, 30.0],
            material_thickness: None,
            cuts_parts: false,
            operations: vec![Operation {
                color: None,
                kind,
                feed: 600.0,
                power: 50.0,
                passes,
                tool: None,
            }],
        }
    }

    /// The order in which each loop of a ring is cut, by its first cutting
    /// move's X: the outer square starts at X20, the inner one at X30.
    fn first_cuts(program: &Program) -> Vec<&str> {
        code(program)
            .into_iter()
            .filter(|l| l.starts_with("G0 X"))
            .collect()
    }

    fn ring() -> Artwork {
        // An outer square starting at the corner nearest the origin, and a hole.
        artwork(
            40,
            r##"<path d="M0 0H40V40H0Z M10 10H30V30H10Z" fill="#000000"/>"##,
        )
    }

    #[test]
    fn holes_are_cut_before_the_outline_round_them() {
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.offset = [10.0, 10.0];
        let program = generate(&ring(), &job).unwrap();
        let rapids = first_cuts(&program);
        // The inner loop's corner (bed 20, 20) is reached before the outer one's (10, 10).
        let inner = rapids
            .iter()
            .position(|l| l.starts_with("G0 X20 Y80"))
            .expect("inner loop");
        let outer = rapids
            .iter()
            .position(|l| l.starts_with("G0 X10 Y90"))
            .expect("outer loop");
        assert!(inner < outer, "{rapids:?}");
    }

    #[test]
    fn a_router_cuts_holes_before_the_outline_too() {
        let job = Job {
            machine: machine(MachineKind::Cnc),
            offset: [10.0, 10.0],
            material_thickness: Some(3.0),
            cuts_parts: false,
            operations: vec![Operation {
                color: None,
                kind: OperationKind::CncProfile {
                    depth: 3.0,
                    depth_per_pass: 1.5,
                    plunge_feed: 200.0,
                    safe_z: 5.0,
                    spindle_rpm: 16000.0,
                },
                feed: 800.0,
                power: 0.0,
                passes: 1,
                tool: None,
            }],
        };
        let program = generate(&ring(), &job).unwrap();
        let rapids = first_cuts(&program);
        let inner = rapids
            .iter()
            .position(|l| l.starts_with("G0 X20 Y80"))
            .expect("inner loop");
        let outer = rapids
            .iter()
            .position(|l| l.starts_with("G0 X10 Y90"))
            .expect("outer loop");
        assert!(inner < outer, "{rapids:?}");
    }

    #[test]
    fn separate_shapes_are_still_ordered_by_distance() {
        let squares = artwork(
            40,
            r##"<path d="M30 30H40V40H30Z" fill="#000000"/><path d="M0 0H10V10H0Z" fill="#000000"/>"##,
        );
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.offset = [0.0, 0.0];
        let program = generate(&squares, &job).unwrap();
        let rapids = first_cuts(&program);
        // From X0 Y0 the square at machine X30 Y70 (76 mm away) comes before
        // the one at X0 Y100, even though it is listed first in neither.
        assert_eq!(rapids.len(), 3, "{rapids:?}");
        assert!(rapids[0].starts_with("G0 X30 Y70"), "{rapids:?}");
    }

    /// Program lines without the leading comment.
    fn code(program: &Program) -> Vec<&str> {
        program
            .text
            .lines()
            .filter(|l| !l.starts_with(';'))
            .collect()
    }

    /// Compares bounds as `[min_x, min_y, max_x, max_y]`, allowing for the
    /// single-precision coordinates that the SVG parser produces.
    fn assert_bounds(program: &Program, expected: [f64; 4]) {
        let b = program.bounds;
        let actual = [b.min_x, b.min_y, b.max_x, b.max_y];
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                (a - e).abs() < 1e-4,
                "bounds {actual:?}, expected {expected:?}"
            );
        }
    }

    #[test]
    fn laser_cut_square_matches_expected_program() {
        let program = generate(&square(), &laser_job(OperationKind::LaserCut, 1)).unwrap();
        // The square's top-left is 30 mm from the back of a 100 mm bed, so its
        // top edge is at machine Y 70 and its bottom edge at Y 60.
        assert_eq!(
            code(&program),
            [
                "G21",
                "G90",
                "M4 S0",
                "G0 X20 Y70 S0",
                "G1 X30 Y70 S500 F600",
                "G1 X30 Y60",
                "G1 X20 Y60",
                "G1 X20 Y70",
                "M5",
                "G0 X0 Y0",
            ]
        );
        assert_bounds(&program, [20.0, 60.0, 30.0, 70.0]);
        assert!((program.cut_mm - 40.0).abs() < 1e-3);
        assert_eq!(program.line_count, 11);
    }

    #[test]
    fn every_rapid_before_the_end_turns_the_laser_off() {
        let art = artwork(
            40,
            r##"<path d="M0 0H10V10H0Z" fill="#000000"/><path d="M20 20H30V30H20Z" fill="#000000"/>"##,
        );
        let program = generate(&art, &laser_job(OperationKind::LaserCut, 1)).unwrap();
        let lines = code(&program);
        let end = lines.iter().position(|l| *l == "M5").unwrap();
        let rapids: Vec<_> = lines[..end]
            .iter()
            .filter(|l| l.starts_with("G0"))
            .collect();
        assert_eq!(rapids.len(), 2);
        assert!(rapids.iter().all(|l| l.ends_with(" S0")));
    }

    #[test]
    fn extra_passes_repeat_closed_paths_and_reverse_open_ones() {
        let closed = generate(&square(), &laser_job(OperationKind::LaserCut, 3)).unwrap();
        assert!((closed.cut_mm - 120.0).abs() < 1e-3);
        assert_eq!(
            code(&closed).iter().filter(|l| l.starts_with("G0")).count(),
            2
        );

        let line = artwork(10, r##"<path d="M0 5H10" stroke="#000000" fill="none"/>"##);
        let open = generate(&line, &laser_job(OperationKind::LaserCut, 2)).unwrap();
        let cuts: Vec<_> = code(&open)
            .into_iter()
            .filter(|l| l.starts_with("G1"))
            .collect();
        assert_eq!(cuts, ["G1 X30 Y65 S500 F600", "G1 X20 Y65"]);
    }

    #[test]
    fn color_selects_shapes() {
        let art = artwork(
            40,
            r##"<path d="M0 0H10V10H0Z" fill="#ff0000"/><path d="M20 20H30V30H20Z" fill="#0000ff"/>"##,
        );
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.operations[0].color = Some("#0000ff".into());
        let program = generate(&art, &job).unwrap();
        assert_bounds(&program, [40.0, 40.0, 50.0, 50.0]);

        job.operations[0].color = Some("#00ff00".into());
        assert!(generate(&art, &job).unwrap_err().contains("do not match"));
    }

    #[test]
    fn job_outside_the_bed_is_refused() {
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.offset = [95.0, 30.0];
        assert!(generate(&square(), &job)
            .unwrap_err()
            .contains("does not fit"));
        job.offset = [20.0, -1.0];
        assert!(generate(&square(), &job)
            .unwrap_err()
            .contains("does not fit"));
    }

    #[test]
    fn invalid_settings_are_refused() {
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.operations[0].power = 120.0;
        assert!(generate(&square(), &job).is_err());

        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.operations[0].feed = 0.0;
        assert!(generate(&square(), &job).is_err());

        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.machine.kind = MachineKind::Cnc;
        assert!(generate(&square(), &job)
            .unwrap_err()
            .contains("laser machine"));

        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.operations.clear();
        assert!(generate(&square(), &job).is_err());
    }

    #[test]
    fn fill_sweeps_rows_back_and_forth() {
        let program = generate(
            &square(),
            &laser_job(OperationKind::LaserFill { interval: 2.5 }, 1),
        )
        .unwrap();
        let moves: Vec<_> = code(&program)
            .into_iter()
            .filter(|l| l.starts_with("G0") || l.starts_with("G1"))
            .collect();
        // Four rows through a 10 mm square, centred in each 2.5 mm band.
        assert_eq!(
            moves,
            [
                "G0 X20 Y68.75 S0",
                "G1 X30 Y68.75 S500 F600",
                "G0 X30 Y66.25 S0",
                "G1 X20 Y66.25 S500 F600",
                "G0 X20 Y63.75 S0",
                "G1 X30 Y63.75 S500 F600",
                "G0 X30 Y61.25 S0",
                "G1 X20 Y61.25 S500 F600",
                "G0 X0 Y0",
            ]
        );
        assert!((program.cut_mm - 40.0).abs() < 1e-3);
    }

    #[test]
    fn fill_leaves_holes_and_merges_overlaps() {
        // A 10 mm square with a 4 mm hole in the middle.
        let ring = artwork(
            10,
            r##"<path d="M0 0H10V10H0Z M3 3V7H7V3Z" fill="#000000"/>"##,
        );
        let spans = scanline_spans(&[ring.shapes[0].polylines.clone()], 5.0);
        assert_eq!(spans.len(), 2);
        assert!((spans[0][1] - 3.0).abs() < 1e-3 && (spans[1][0] - 7.0).abs() < 1e-3);
        assert_eq!(
            scanline_spans(&[ring.shapes[0].polylines.clone()], 1.0).len(),
            1
        );

        // Two separate shapes that overlap between x = 5 and x = 10.
        let overlap = artwork(
            20,
            r##"<path d="M0 0H10V10H0Z" fill="#000000"/><path d="M5 0H15V10H5Z" fill="#000000"/>"##,
        );
        let shapes: Vec<_> = overlap.shapes.iter().map(|s| s.polylines.clone()).collect();
        let spans = scanline_spans(&shapes, 5.0);
        assert_eq!(spans.len(), 1);
        assert!(spans[0][0].abs() < 1e-3 && (spans[0][1] - 15.0).abs() < 1e-3);
    }

    #[test]
    fn cnc_profile_steps_down_to_depth() {
        let job = Job {
            machine: machine(MachineKind::Cnc),
            offset: [20.0, 30.0],
            material_thickness: None,
            cuts_parts: false,
            operations: vec![Operation {
                color: None,
                kind: OperationKind::CncProfile {
                    depth: 2.5,
                    depth_per_pass: 1.0,
                    plunge_feed: 200.0,
                    safe_z: 5.0,
                    spindle_rpm: 12000.0,
                },
                feed: 800.0,
                power: 0.0,
                passes: 1,
                tool: None,
            }],
        };
        let program = generate(&square(), &job).unwrap();
        let lines = code(&program);
        assert_eq!(
            &lines[..6],
            ["G21", "G90", "G0 Z5", "M3 S12000", "G4 P2", "G0 X20 Y70"]
        );
        let plunges: Vec<_> = lines.iter().filter(|l| l.starts_with("G1 Z")).collect();
        assert_eq!(plunges, [&"G1 Z-1 F200", &"G1 Z-2 F200", &"G1 Z-2.5 F200"]);
        assert_eq!(lines[6..8], ["G1 Z-1 F200", "G1 X30 Y70 F800"]);
        assert_eq!(lines[lines.len() - 3..], ["G0 Z5", "M5", "G0 X0 Y0"]);
        // Three laps of a 40 mm perimeter, and no XY move happens below the surface with G0.
        assert!((program.cut_mm - 120.0).abs() < 1e-3);
        assert!(!program.text.contains(" S0"));
    }

    #[test]
    fn nearby_paths_are_cut_first() {
        let art = artwork(
            60,
            r##"<path d="M50 50H55" stroke="#000000" fill="none"/><path d="M0 50H5" stroke="#000000" fill="none"/>"##,
        );
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.offset = [0.0, 0.0];
        let program = generate(&art, &job).unwrap();
        let first_rapid = code(&program)
            .into_iter()
            .find(|l| l.starts_with("G0"))
            .unwrap();
        // The machine starts at the front-left origin, nearest the path at x = 0.
        assert_eq!(first_rapid, "G0 X0 Y50 S0");
    }

    #[test]
    fn numbers_are_trimmed() {
        assert_eq!(num(10.0), "10");
        assert_eq!(num(2.5), "2.5");
        assert_eq!(num(1.23456), "1.235");
        assert_eq!(num(-0.0001), "0");
    }

    #[test]
    fn moves_are_recorded_in_order_for_playback() {
        let program = generate(&square(), &laser_job(OperationKind::LaserCut, 1)).unwrap();
        let moves: Vec<[f32; 4]> = program
            .moves
            .chunks_exact(4)
            .map(|m| [m[0].round(), m[1].round(), m[2], m[3]])
            .collect();
        // Bed coordinates, Y down: the job starts and ends at the front-left origin.
        assert_eq!(
            moves,
            [
                [0.0, 100.0, 0.0, -1.0],
                [20.0, 30.0, 0.0, -1.0],
                [30.0, 30.0, 0.0, 0.5],
                [30.0, 40.0, 0.0, 0.5],
                [20.0, 40.0, 0.0, 0.5],
                [20.0, 30.0, 0.0, 0.5],
                [0.0, 100.0, 0.0, -1.0],
            ]
        );
    }

    #[test]
    fn router_moves_record_height() {
        let job = Job {
            machine: machine(MachineKind::Cnc),
            offset: [20.0, 30.0],
            material_thickness: None,
            cuts_parts: false,
            operations: vec![Operation {
                color: None,
                kind: OperationKind::CncProfile {
                    depth: 1.0,
                    depth_per_pass: 1.0,
                    plunge_feed: 200.0,
                    safe_z: 5.0,
                    spindle_rpm: 12000.0,
                },
                feed: 800.0,
                power: 0.0,
                passes: 1,
                tool: None,
            }],
        };
        let program = generate(&square(), &job).unwrap();
        let moves: Vec<&[f32]> = program.moves.chunks_exact(4).collect();
        // Start, lift to safe height, rapid over, plunge, then cut at depth.
        assert_eq!(moves[1][2..], [5.0, -1.0]);
        assert_eq!(moves[2][2..], [5.0, -1.0]);
        assert_eq!(moves[3][2..], [-1.0, 1.0]);
        assert!(moves[4..8].iter().all(|m| m[2..] == [-1.0, 1.0]));
        assert_eq!(moves[moves.len() - 1][2..], [5.0, -1.0]);
    }

    /// Lettering on the panel: a 6 mm line across the middle of the square,
    /// drawn where it sits on the bed.
    fn lettering() -> Artwork {
        artwork(100, r##"<path d="M22 35H28" stroke="#000" fill="none"/>"##)
    }

    fn router(depth: f64) -> Operation {
        Operation {
            color: None,
            kind: OperationKind::CncProfile {
                depth,
                depth_per_pass: depth,
                plunge_feed: 200.0,
                safe_z: 5.0,
                spindle_rpm: 12000.0,
            },
            feed: 800.0,
            power: 0.0,
            passes: 1,
            tool: None,
        }
    }

    #[test]
    fn a_laser_marks_a_panel_before_cutting_it_free() {
        // A box's panel outline, cut at full power, with lettering added on it.
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.cuts_parts = true;
        job.operations[0].power = 100.0;
        job.operations[0].feed = 300.0;
        let text = lettering();
        let added = [Operation {
            power: 20.0,
            feed: 3000.0,
            ..job.operations[0].clone()
        }];
        let additions = Additions {
            artwork: &text,
            operations: &added,
        };
        let program = generate_composed(&square(), None, &job, Some(&additions), None).unwrap();
        assert_eq!(
            code(&program),
            [
                "G21",
                "G90",
                "M4 S0",
                "G0 X22 Y65 S0",
                "G1 X28 Y65 S200 F3000",
                "M4 S0",
                "G0 X20 Y70 S0",
                "G1 X30 Y70 S1000 F300",
                "G1 X30 Y60",
                "G1 X20 Y60",
                "G1 X20 Y70",
                "M5",
                "G0 X0 Y0",
            ]
        );
    }

    #[test]
    fn a_router_marks_a_panel_before_cutting_it_free() {
        // 3 mm material: a shallow line on the main artwork, a cut right
        // through it, and lettering added, listed with the cut first.
        let job = Job {
            machine: machine(MachineKind::Cnc),
            offset: [20.0, 30.0],
            material_thickness: Some(3.0),
            cuts_parts: false,
            operations: vec![router(3.0), router(0.5)],
        };
        let text = lettering();
        let added = [router(0.25)];
        let additions = Additions {
            artwork: &text,
            operations: &added,
        };
        let program = generate_composed(&square(), None, &job, Some(&additions), None).unwrap();
        let lines = code(&program);
        let at = |line: &str| lines.iter().position(|l| *l == line).unwrap();
        // The shallow line, then the lettering, then the cut through.
        assert!(at("G1 Z-0.5 F200") < at("G1 Z-0.25 F200"));
        assert!(at("G1 Z-0.25 F200") < at("G1 Z-3 F200"));
        assert_eq!(program.operation_starts.len(), 3);
        assert_eq!(lines[lines.len() - 3..], ["G0 Z5", "M5", "G0 X0 Y0"]);

        // A design's parts are cut free last even if the cut stops short.
        let mut parts = job.clone();
        parts.cuts_parts = true;
        parts.operations = vec![router(2.0)];
        let program = generate_composed(&square(), None, &parts, Some(&additions), None).unwrap();
        let lines = code(&program);
        let at = |line: &str| lines.iter().position(|l| *l == line).unwrap();
        assert!(at("G1 Z-0.25 F200") < at("G1 Z-2 F200"));
    }

    #[test]
    fn a_layers_design_burns_its_marks_before_its_cut() {
        // Listed with the full-power cut above the 15% registration marks.
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.cuts_parts = true;
        job.operations[0].power = 100.0;
        job.operations.push(Operation {
            power: 15.0,
            ..job.operations[0].clone()
        });
        let program = generate(&square(), &job).unwrap();
        let lines = code(&program);
        let at = |s: &str| lines.iter().position(|l| l.contains(s)).unwrap();
        assert!(at("S150 F600") < at("S1000 F600"), "{lines:?}");
        // Not a design: the listed order stands.
        job.cuts_parts = false;
        let program = generate(&square(), &job).unwrap();
        let lines = code(&program);
        let at = |s: &str| lines.iter().position(|l| l.contains(s)).unwrap();
        assert!(at("S1000 F600") < at("S150 F600"), "{lines:?}");
    }

    #[test]
    fn without_additions_marks_still_come_before_a_cut_through() {
        // Listed cut first; the fill still runs before the line.
        let mut job = laser_job(OperationKind::LaserCut, 1);
        job.operations.push(Operation {
            kind: OperationKind::LaserFill { interval: 5.0 },
            power: 20.0,
            ..job.operations[0].clone()
        });
        let program = generate(&square(), &job).unwrap();
        let fill = program.text.find("S200").unwrap();
        let cut = program.text.find("S500").unwrap();
        assert!(fill < cut);
        // The fill first and the line second is left exactly as listed.
        job.operations.reverse();
        assert_eq!(generate(&square(), &job).unwrap().text, program.text);
    }
}
