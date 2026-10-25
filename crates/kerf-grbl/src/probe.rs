//! Building the lines that set Z with a touch plate.
//!
//! Every line is absolute (`G90`) in work coordinates, so nothing is left in
//! relative mode if the sequence stops part way. The sender sends them one at
//! a time, each only after the one before has been answered, and the line
//! that sets Z (`G10`) only after the probe has reported a touch.

/// The default distance the bit may go down looking for the plate.
pub const DEFAULT_TRAVEL: f64 = 10.0;
/// The default speeds of the first, quick touch and the second, slow one.
pub const DEFAULT_FAST_FEED: f64 = 100.0;
pub const DEFAULT_SLOW_FEED: f64 = 25.0;
/// How far the bit backs off between the two touches.
pub const BACK_OFF: f64 = 2.0;
/// How far the slow touch may go: past the first touch by a millimetre.
pub const SLOW_TRAVEL: f64 = 3.0;
/// How far above the plate the bit is left once Z is set.
pub const LIFT_AFTER: f64 = 5.0;

/// The first, quick touch: down at most `travel` from `work_z`.
pub fn fast_line(work_z: f64, travel: f64, feed: f64) -> String {
    format!("G90 G38.2 Z{:.3} F{feed:.0}", work_z - travel)
}

/// Backs off above the first touch, which was at `touched_work_z`.
pub fn back_off_line(touched_work_z: f64) -> String {
    format!("G90 G0 Z{:.3}", touched_work_z + BACK_OFF)
}

/// The second, slow touch, from where [`back_off_line`] left the bit.
pub fn slow_line(touched_work_z: f64, feed: f64) -> String {
    format!(
        "G90 G38.2 Z{:.3} F{feed:.0}",
        touched_work_z + BACK_OFF - SLOW_TRAVEL
    )
}

/// Sets the active coordinate system's Z so that the bit, touching the plate
/// at machine Z `touched_machine_z`, is `plate` above the work zero. Only Z
/// is named, so the X/Y zero cannot change.
pub fn set_z_line(touched_machine_z: f64, plate: f64) -> String {
    format!("G10 L2 P0 Z{:.3}", touched_machine_z - plate)
}

/// Lifts the bit clear of the plate once Z is set.
pub fn lift_line(plate: f64) -> String {
    format!("G90 G0 Z{:.3}", plate + LIFT_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_lines_are_exact() {
        assert_eq!(fast_line(6.0, 10.0, 100.0), "G90 G38.2 Z-4.000 F100");
        assert_eq!(back_off_line(-1.5), "G90 G0 Z0.500");
        assert_eq!(slow_line(-1.5, 25.0), "G90 G38.2 Z-2.500 F25");
        assert_eq!(set_z_line(-20.25, 12.7), "G10 L2 P0 Z-32.950");
        assert_eq!(lift_line(12.7), "G90 G0 Z17.700");
        for line in [fast_line(6.0, 10.0, 100.0), set_z_line(-20.0, 10.0)] {
            assert!(!line.contains('X') && !line.contains('Y'), "{line}");
            assert!(!line.contains('M'), "{line}");
        }
    }
}
