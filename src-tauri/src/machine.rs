//! Commands for talking to a connected GRBL machine.

use std::sync::Mutex;

use kerf_gcode::MachineKind;
use kerf_grbl::probe::{DEFAULT_FAST_FEED, DEFAULT_SLOW_FEED};
use kerf_grbl::{
    Connection, JobOptions, MachineState, PortInfo, ProbeSettings, Tool, DEFAULT_BAUD,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::{blocking, AppState, StoredProgram};

/// Name of the event that carries every [`kerf_grbl::Event`] to the UI.
const MACHINE_EVENT: &str = "machine";

#[derive(Default)]
pub struct MachineLink(Mutex<Option<Connection>>);

impl MachineLink {
    fn with<T>(&self, action: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let guard = self.0.lock().unwrap();
        action(guard.as_ref().ok_or("No machine is connected.")?)
    }

    /// Closes the connection, which resets the machine if it is still running.
    pub fn close(&self) {
        drop(self.0.lock().unwrap().take());
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MachineAction {
    /// A line typed into the console.
    Line {
        text: String,
    },
    /// A relative move, in millimetres, at `feed` mm/min.
    Jog {
        x: f64,
        y: f64,
        z: f64,
        feed: f64,
    },
    CancelJog,
    /// Starts moving along one axis while the button or key is held. Each
    /// of `x`, `y`, `z` is −1, 0 or 1, and only one may be non-zero.
    HoldJogStart {
        x: f64,
        y: f64,
        z: f64,
        feed: f64,
    },
    /// The button or key is still held; without this the move stops.
    HoldJogAlive,
    HoldJogStop,
    /// Moves back to the work zero in X and Y, at the current height.
    GoToXyZero {
        feed: f64,
    },
    Home,
    /// Clears an alarm lock.
    Unlock,
    /// Makes the current XY position (or Z position) the work origin, in the
    /// coordinate system in use. XY is refused while a job waits for a bit
    /// change.
    SetOrigin {
        z: bool,
    },
    /// Traces a rectangle in work coordinates, without cutting.
    Frame {
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        feed: f64,
    },
    Hold,
    Resume,
    Reset,
    /// Streams the generated program, which must be the one with this id:
    /// the one the interface shows.
    StartJob {
        program_id: u64,
    },
    StopJob,
    /// Streams the next part of a job waiting for a bit change, once the
    /// sender's checks pass. `plate_cleared` is the person's tick that the
    /// touch plate's clip is off the bit.
    ContinueJob {
        plate_cleared: bool,
    },
    /// Sets Z with a touch plate `plate` mm thick, going down at most `travel` mm.
    ProbeZ {
        plate: f64,
        travel: f64,
    },
    /// Watches for the probe pin, so the person can check the plate and clip work.
    ProbeTest,
}

fn finite(values: &[f64]) -> Result<(), String> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err("A coordinate or speed is not a number.".into())
    }
}

/// A held jog's direction: one axis only, so that a stuck key can only ever
/// drive one axis.
fn hold_direction(axes: [f64; 3]) -> Result<[i8; 3], String> {
    let one_axis = "Hold to move goes along one axis at a time.";
    if axes.iter().any(|&v| v != -1.0 && v != 0.0 && v != 1.0)
        || axes.iter().filter(|&&v| v != 0.0).count() != 1
    {
        return Err(one_axis.into());
    }
    Ok(axes.map(|v| v as i8))
}

#[tauri::command]
pub fn list_ports() -> Result<Vec<PortInfo>, String> {
    kerf_grbl::list_ports()
}

#[tauri::command]
pub async fn connect_machine(
    app: AppHandle,
    port: String,
    link: State<'_, MachineLink>,
) -> Result<(), String> {
    // Close any existing connection first, so its port is free again.
    link.0.lock().unwrap().take();
    let connection = blocking(move || {
        let transport = kerf_grbl::serial::open(&port, DEFAULT_BAUD)?;
        Ok(Connection::open(transport, move |event| {
            let _ = app.emit(MACHINE_EVENT, event);
        }))
    })
    .await?;
    *link.0.lock().unwrap() = Some(connection);
    Ok(())
}

#[tauri::command]
pub async fn disconnect_machine(link: State<'_, MachineLink>) -> Result<(), String> {
    let connection = {
        let mut guard = link.0.lock().unwrap();
        // Disconnecting resets a running machine, losing its position, so it is
        // refused while the machine may be working. A lost port is different:
        // it is reported, and the machine's own stop is the way to stop it.
        if let Some(state) = guard.as_ref().and_then(Connection::state) {
            if !matches!(
                state,
                MachineState::Idle | MachineState::Alarm | MachineState::Sleep
            ) {
                return Err("Stop the machine before disconnecting.".into());
            }
        }
        guard.take()
    };
    // Dropping joins the worker thread, so do it off the async runtime.
    blocking(move || {
        drop(connection);
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn machine_action(
    action: MachineAction,
    link: State<'_, MachineLink>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    link.with(|machine| match action {
        MachineAction::Line { text } => machine.send_line(&text),
        MachineAction::Jog { x, y, z, feed } => {
            finite(&[x, y, z, feed])?;
            if feed <= 0.0 {
                return Err("The jog speed must be greater than zero.".into());
            }
            // Refused while the laser or spindle is on.
            machine.jog(x, y, z, feed)
        }
        MachineAction::CancelJog => machine.cancel_jog(),
        MachineAction::HoldJogStart { x, y, z, feed } => {
            finite(&[x, y, z, feed])?;
            machine.start_held_jog(hold_direction([x, y, z])?, feed)
        }
        MachineAction::HoldJogAlive => machine.keep_held_jog(),
        MachineAction::HoldJogStop => machine.stop_held_jog(),
        MachineAction::GoToXyZero { feed } => {
            finite(&[feed])?;
            if feed <= 0.0 {
                return Err("The speed must be greater than zero.".into());
            }
            machine.go_to_xy_zero(feed)
        }
        MachineAction::Home => machine.send_line("$H"),
        MachineAction::Unlock => machine.send_line("$X"),
        MachineAction::SetOrigin { z: false } => machine.zero_xy(),
        MachineAction::SetOrigin { z: true } => machine.zero_z(),
        MachineAction::Frame {
            min_x,
            min_y,
            max_x,
            max_y,
            feed,
        } => {
            finite(&[min_x, min_y, max_x, max_y, feed])?;
            if feed <= 0.0 {
                return Err("The framing speed must be greater than zero.".into());
            }
            // Jog moves only: they cannot turn on a spindle or laser, and are
            // refused while one is already on.
            machine.frame(min_x, min_y, max_x, max_y, feed)
        }
        MachineAction::Hold => machine.hold(),
        MachineAction::Resume => machine.resume(),
        MachineAction::Reset => machine.reset(),
        MachineAction::StartJob { program_id } => {
            let gcode = state.gcode.lock().unwrap();
            let program = StoredProgram::matching(&gcode, program_id)?;
            let tool = match program.kind {
                MachineKind::Laser => Tool::Laser,
                MachineKind::Cnc => Tool::Spindle,
            };
            let program = &program.program;
            if program.parts.len() <= 1 && !program.atc && !program.needs_homing {
                // One bit: exactly as before.
                return machine.start_job(&program.text, tool);
            }
            let options = JobOptions {
                needs_homing: program.needs_homing,
                atc: program.atc,
                change_lift: program.change_lift,
                machine_moves: program.machine_moves.clone(),
                change_clearance: program.change_clearance,
            };
            machine.start_parts(&program.part_streams(), tool, &options)
        }
        MachineAction::StopJob => machine.stop_job(),
        MachineAction::ContinueJob { plate_cleared } => machine.continue_job(plate_cleared),
        MachineAction::ProbeZ { plate, travel } => {
            finite(&[plate, travel])?;
            machine.probe_z(ProbeSettings {
                plate,
                travel,
                fast_feed: DEFAULT_FAST_FEED,
                slow_feed: DEFAULT_SLOW_FEED,
            })
        }
        MachineAction::ProbeTest => machine.probe_test(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_jog_rejects_two_axes() {
        assert_eq!(hold_direction([0.0, -1.0, 0.0]), Ok([0, -1, 0]));
        assert_eq!(hold_direction([0.0, 0.0, 1.0]), Ok([0, 0, 1]));
        for axes in [
            [1.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [2.0, 0.0, 0.0],
        ] {
            assert!(hold_direction(axes).is_err(), "{axes:?}");
        }
    }
}
