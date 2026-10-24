//! What to make: the machine, where the artwork sits, and the operations to run.

use serde::{Deserialize, Serialize};

use crate::cutout::{Outline, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineKind {
    Laser,
    Cnc,
}

/// Which corner of the bed is X0 Y0. X always increases to the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// Y increases towards the back; the usual GRBL arrangement.
    #[default]
    FrontLeft,
    /// Y increases towards the front, as on machines that home to the back.
    BackLeft,
}

/// A machine that accepts GRBL-style G-code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub kind: MachineKind,
    #[serde(default)]
    pub origin: Origin,
    /// Lines the machine needs before and after every job, such as vendor
    /// commands. Emitted as given.
    #[serde(default)]
    pub start_gcode: String,
    #[serde(default)]
    pub end_gcode: String,
    /// Bed size in millimetres.
    pub bed_width: f64,
    pub bed_height: f64,
    /// The `S` value that means full power (GRBL setting `$30`).
    pub max_power_s: f64,
    /// Rapid speed in mm/min, used only to estimate job time.
    pub travel_feed: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OperationKind {
    /// Follow each path with the laser on.
    LaserCut,
    /// Fill closed shapes with horizontal scan lines `interval` millimetres apart.
    LaserFill { interval: f64 },
    /// Engrave the image itself, line by line, for photographs and shaded
    /// artwork. Darker areas get more power, up to the operation's power.
    LaserImage {
        /// Distance between lines, and the size of one engraved dot.
        interval: f64,
        /// Turn shades into a pattern of full-power dots (Floyd–Steinberg),
        /// which suits materials that do not darken evenly with power.
        dither: bool,
    },
    /// Follow each path with a router bit, stepping down to `depth`.
    /// The tool centre runs on the path; there is no tool-radius offset.
    CncProfile {
        depth: f64,
        depth_per_pass: f64,
        plunge_feed: f64,
        safe_z: f64,
        spindle_rpm: f64,
    },
    /// Carve the image as a 3D relief: brightness sets the height of the
    /// surface, from the stock top (Z0) down to `depth`. The bit sweeps the
    /// area in parallel rows, kept high enough that it never cuts below the
    /// surface anywhere under it.
    CncRelief {
        depth: f64,
        depth_per_pass: f64,
        plunge_feed: f64,
        safe_z: f64,
        spindle_rpm: f64,
        tool_diameter: f64,
        /// A ball-nose bit; otherwise a flat end mill.
        ball_nose: bool,
        /// Distance between rows.
        stepover: f64,
        /// Light areas of the image stand proud; otherwise dark areas do.
        light_is_high: bool,
        /// For a tapered ball nose: the angle of its sides from the axis, in
        /// degrees. Zero for a straight bit. `tool_diameter` is then the
        /// diameter of the ball at the tip.
        #[serde(default)]
        taper_angle: f64,
        /// The widest the bit gets, which limits how far a taper spreads.
        /// Zero if not known.
        #[serde(default)]
        shank_diameter: f64,
    },
    /// Cut a line around the artwork to full depth, to free the piece. The
    /// line is the artwork's rectangle, or the ellipse inside it, moved
    /// outwards by `offset` (inwards if negative).
    CncCutout {
        depth: f64,
        depth_per_pass: f64,
        plunge_feed: f64,
        safe_z: f64,
        spindle_rpm: f64,
        tool_diameter: f64,
        offset: f64,
        side: Side,
        outline: Outline,
        /// Radius of the line's corners, for a rectangle.
        corner_radius: f64,
        /// Bridges of material left to hold the piece; zero for none.
        tabs: u32,
        tab_width: f64,
        tab_height: f64,
    },
    /// The same line around the artwork, cut by a laser. It runs in the same
    /// job as any engraving, after it, since a laser has no bit to change.
    /// The operation's power, speed and passes are what cut through.
    LaserCutout {
        offset: f64,
        outline: Outline,
        corner_radius: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    /// Only shapes of this `#rrggbb` colour; `None` applies to every shape.
    pub color: Option<String>,
    pub kind: OperationKind,
    /// Cutting speed in mm/min.
    pub feed: f64,
    /// Laser power in percent, 0 to 100. Ignored for CNC operations.
    pub power: f64,
    /// How many times to repeat the operation. Ignored for CNC profiles,
    /// where depth sets the number of passes.
    pub passes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub machine: Machine,
    /// Top-left corner of the artwork on the bed, in millimetres from the
    /// bed's top-left (back-left) corner with Y pointing towards the front.
    pub offset: [f64; 2],
    pub operations: Vec<Operation>,
    /// The material's thickness in millimetres, when known. Router depths
    /// beyond it (plus a little, to cut cleanly through) are refused.
    #[serde(default)]
    pub material_thickness: Option<f64>,
}

/// How far a router may cut below the material, into whatever is under it,
/// to be sure of cutting right through.
pub const THROUGH_CUT_ALLOWANCE_MM: f64 = 1.0;

impl Job {
    /// Checks that hold across all of a job's operations, `added` included.
    pub(crate) fn check(&self, added: &[Operation]) -> Result<(), String> {
        let operations: Vec<&Operation> = self.operations.iter().chain(added).collect();

        // One job is cut with one bit: changing it means stopping and zeroing
        // Z again, which a single program cannot do.
        let mut bits: Vec<f64> = operations
            .iter()
            .filter_map(|o| o.kind.tool_diameter())
            .collect();
        bits.sort_by(f64::total_cmp);
        bits.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        if bits.len() > 1 {
            let sizes: Vec<String> = bits.iter().map(|d| format!("{d} mm")).collect();
            return Err(format!(
                "These operations use bits of different sizes ({}). Include the operations for one bit at a time, and zero Z again after changing it.",
                sizes.join(", ")
            ));
        }

        if let Some(thickness) = self.material_thickness {
            if !(thickness.is_finite() && thickness > 0.0) {
                return Err("The material's thickness must be greater than zero.".into());
            }
            let limit = thickness + THROUGH_CUT_ALLOWANCE_MM;
            for (index, operation) in operations.iter().enumerate() {
                if let Some(depth) = operation.kind.depth() {
                    if depth > limit + 1e-9 {
                        return Err(format!(
                            "Operation {} cuts {depth} mm deep, but the material is only {thickness} mm thick. Make it at most {limit} mm deep, or correct the thickness in step 2, Material.",
                            index + 1
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl OperationKind {
    /// How deep a router operation cuts; `None` for a laser.
    pub fn depth(&self) -> Option<f64> {
        match *self {
            Self::CncProfile { depth, .. }
            | Self::CncRelief { depth, .. }
            | Self::CncCutout { depth, .. } => Some(depth),
            _ => None,
        }
    }

    /// The router bit's diameter, for operations that are told it.
    pub fn tool_diameter(&self) -> Option<f64> {
        match *self {
            Self::CncRelief { tool_diameter, .. } | Self::CncCutout { tool_diameter, .. } => {
                Some(tool_diameter)
            }
            _ => None,
        }
    }
}

impl Operation {
    pub(crate) fn validate(&self, machine: &Machine) -> Result<(), String> {
        let positive = |value: f64, name: &str| {
            if value.is_finite() && value > 0.0 {
                Ok(())
            } else {
                Err(format!("{name} must be greater than zero."))
            }
        };
        positive(self.feed, "Speed")?;
        match self.kind {
            OperationKind::LaserCut
            | OperationKind::LaserFill { .. }
            | OperationKind::LaserImage { .. }
            | OperationKind::LaserCutout { .. } => {
                if machine.kind != MachineKind::Laser {
                    return Err("Laser operations need a laser machine profile.".into());
                }
                if !(0.0..=100.0).contains(&self.power) {
                    return Err("Power must be between 0 and 100 percent.".into());
                }
                if self.passes == 0 {
                    return Err("Passes must be at least 1.".into());
                }
                if let OperationKind::LaserFill { interval }
                | OperationKind::LaserImage { interval, .. } = self.kind
                {
                    positive(interval, "Line interval")?;
                }
                if let OperationKind::LaserCutout {
                    offset,
                    corner_radius,
                    ..
                } = self.kind
                {
                    if !offset.is_finite() || !(corner_radius.is_finite() && corner_radius >= 0.0) {
                        return Err("The cutout's offset and corner radius must be numbers, and the radius not negative.".into());
                    }
                }
            }
            OperationKind::CncProfile {
                depth,
                depth_per_pass,
                plunge_feed,
                safe_z,
                spindle_rpm,
            } => {
                if machine.kind != MachineKind::Cnc {
                    return Err("CNC operations need a CNC machine profile.".into());
                }
                positive(depth, "Depth")?;
                positive(depth_per_pass, "Depth per pass")?;
                positive(plunge_feed, "Plunge speed")?;
                positive(safe_z, "Safe height")?;
                positive(spindle_rpm, "Spindle speed")?;
            }
            OperationKind::CncRelief {
                depth,
                depth_per_pass,
                plunge_feed,
                safe_z,
                spindle_rpm,
                tool_diameter,
                stepover,
                ..
            } => {
                if machine.kind != MachineKind::Cnc {
                    return Err("CNC operations need a CNC machine profile.".into());
                }
                positive(depth, "Depth")?;
                positive(depth_per_pass, "Depth per pass")?;
                positive(plunge_feed, "Plunge speed")?;
                positive(safe_z, "Safe height")?;
                positive(spindle_rpm, "Spindle speed")?;
                positive(tool_diameter, "Bit diameter")?;
                positive(stepover, "Stepover")?;
                if stepover > tool_diameter {
                    return Err("The stepover cannot be larger than the bit diameter.".into());
                }
            }
            OperationKind::CncCutout {
                depth,
                depth_per_pass,
                plunge_feed,
                safe_z,
                spindle_rpm,
                tool_diameter,
                offset,
                corner_radius,
                tab_width,
                tab_height,
                ..
            } => {
                if machine.kind != MachineKind::Cnc {
                    return Err("CNC operations need a CNC machine profile.".into());
                }
                positive(depth, "Depth")?;
                positive(depth_per_pass, "Depth per pass")?;
                positive(plunge_feed, "Plunge speed")?;
                positive(safe_z, "Safe height")?;
                positive(spindle_rpm, "Spindle speed")?;
                positive(tool_diameter, "Bit diameter")?;
                let sensible = |v: f64| v.is_finite() && v >= 0.0;
                if !offset.is_finite()
                    || !sensible(corner_radius)
                    || !sensible(tab_width)
                    || !sensible(tab_height)
                {
                    return Err("The cutout's offset, corner radius and tab sizes must be numbers, and not negative except for the offset.".into());
                }
            }
        }
        Ok(())
    }
}
