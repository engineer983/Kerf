//! Building jog lines.
//!
//! Every line is a GRBL `$J=` jog, which cannot carry `M` or `S` words, so it
//! can never turn a spindle or laser on. Each carries `G21` (millimetres).
//! Steps and held segments are relative (`G91`); going to zero and framing
//! are absolute (`G90`) in work coordinates. None of these changes the
//! machine's modal state, because a jog's words apply to that line only.

/// The shortest time a held segment may cover, from GRBL's jogging guide.
pub const MIN_SEGMENT_TIME: f64 = 0.05;
/// The longest, so that a single segment never carries far.
pub const MAX_SEGMENT_TIME: f64 = 0.2;
/// The planner blocks GRBL's guide assumes when sizing segments.
const PLANNER_BLOCKS: f64 = 15.0;

/// A relative move of `dx`, `dy`, `dz` millimetres at `feed` mm/min. Axes
/// that do not move are left out.
pub fn step_line(dx: f64, dy: f64, dz: f64, feed: f64) -> String {
    let mut line = String::from("$J=G91 G21");
    for (axis, distance) in [('X', dx), ('Y', dy), ('Z', dz)] {
        if distance != 0.0 {
            line.push_str(&format!(" {axis}{distance:.3}"));
        }
    }
    format!("{line} F{feed:.0}")
}

/// An absolute move to `x`, `y` in work coordinates, at the current height.
pub fn to_xy_line(x: f64, y: f64, feed: f64) -> String {
    format!("$J=G90 G21 X{x:.3} Y{y:.3} F{feed:.0}")
}

/// Back to the work zero in X and Y. Z is never moved.
pub fn go_to_xy_zero_line(feed: f64) -> String {
    to_xy_line(0.0, 0.0, feed)
}

/// The five moves that trace a rectangle, ending where they began.
pub fn frame_lines(min_x: f64, min_y: f64, max_x: f64, max_y: f64, feed: f64) -> Vec<String> {
    [
        (min_x, min_y),
        (max_x, min_y),
        (max_x, max_y),
        (min_x, max_y),
        (min_x, min_y),
    ]
    .into_iter()
    .map(|(x, y)| to_xy_line(x, y, feed))
    .collect()
}

/// One short relative move along `direction` (each axis −1, 0 or 1), as far
/// as `feed` mm/min carries in `dt` seconds.
pub fn held_segment(direction: [i8; 3], feed: f64, dt: f64) -> String {
    let length = feed / 60.0 * dt;
    let [x, y, z] = direction.map(|d| f64::from(d.signum()) * length);
    step_line(x, y, z, feed)
}

/// How long each held segment should take, in seconds, at `feed` mm/min with
/// the axis's acceleration `accel` in mm/s². From GRBL's jogging guide: long
/// enough that the planner's 15 blocks cover the distance needed to stop,
/// with 20% to spare, but at least 0.05 s and at most 0.2 s.
pub fn segment_time(feed: f64, accel: f64) -> f64 {
    let v = feed / 60.0;
    let dt = 1.2 * v * v / (2.0 * accel * (PLANNER_BLOCKS - 1.0));
    dt.clamp(MIN_SEGMENT_TIME, MAX_SEGMENT_TIME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_names_only_the_axes_that_move() {
        assert_eq!(step_line(0.0, 1.0, 0.0, 1000.0), "$J=G91 G21 Y1.000 F1000");
        assert_eq!(step_line(-0.1, 0.0, 0.0, 300.0), "$J=G91 G21 X-0.100 F300");
        assert_eq!(step_line(0.0, 0.0, 10.0, 200.0), "$J=G91 G21 Z10.000 F200");
    }

    #[test]
    fn going_to_zero_moves_x_and_y_only() {
        let line = go_to_xy_zero_line(3000.0);
        assert_eq!(line, "$J=G90 G21 X0.000 Y0.000 F3000");
        assert!(!line.contains('Z'));
    }

    #[test]
    fn framing_traces_the_rectangle_and_returns() {
        assert_eq!(
            frame_lines(1.0, 2.0, 30.5, 40.25, 3000.0),
            [
                "$J=G90 G21 X1.000 Y2.000 F3000",
                "$J=G90 G21 X30.500 Y2.000 F3000",
                "$J=G90 G21 X30.500 Y40.250 F3000",
                "$J=G90 G21 X1.000 Y40.250 F3000",
                "$J=G90 G21 X1.000 Y2.000 F3000",
            ]
        );
    }

    #[test]
    fn a_held_segment_covers_feed_times_dt() {
        // 1000 mm/min for 0.05 s is 0.833 mm.
        assert_eq!(
            held_segment([1, 0, 0], 1000.0, 0.05),
            "$J=G91 G21 X0.833 F1000"
        );
        // 3000 mm/min for 0.2 s is 10 mm.
        assert_eq!(
            held_segment([0, -1, 0], 3000.0, 0.2),
            "$J=G91 G21 Y-10.000 F3000"
        );
        assert_eq!(
            held_segment([0, 0, 1], 200.0, 0.05),
            "$J=G91 G21 Z0.167 F200"
        );
    }

    #[test]
    fn segment_time_is_floored_capped_and_otherwise_follows_the_guide() {
        // Slow and brisk: the floor.
        assert_eq!(segment_time(300.0, 500.0), MIN_SEGMENT_TIME);
        // Fast and sluggish: the cap.
        assert_eq!(segment_time(6000.0, 10.0), MAX_SEGMENT_TIME);
        // In between: 50 mm/s at 1000 mm/s² gives 1.2 × 2500 / (2 × 1000 × 14).
        let dt = segment_time(3000.0, 1000.0);
        assert!((dt - 3000.0 / 28_000.0).abs() < 1e-12, "{dt}");
    }
}
