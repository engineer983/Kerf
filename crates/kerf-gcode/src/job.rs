//! What to make: the machine, where the artwork sits, and the operations to run.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineKind {
    Laser,
    Cnc,
}

/// A GRBL machine whose work origin is the front-left corner of the bed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub kind: MachineKind,
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
    /// Follow each path with a router bit, stepping down to `depth`.
    /// The tool centre runs on the path; there is no tool-radius offset.
    CncProfile {
        depth: f64,
        depth_per_pass: f64,
        plunge_feed: f64,
        safe_z: f64,
        spindle_rpm: f64,
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
            OperationKind::LaserCut | OperationKind::LaserFill { .. } => {
                if machine.kind != MachineKind::Laser {
                    return Err("Laser operations need a laser machine profile.".into());
                }
                if !(0.0..=100.0).contains(&self.power) {
                    return Err("Power must be between 0 and 100 percent.".into());
                }
                if self.passes == 0 {
                    return Err("Passes must be at least 1.".into());
                }
                if let OperationKind::LaserFill { interval } = self.kind {
                    positive(interval, "Line interval")?;
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
        }
        Ok(())
    }
}
