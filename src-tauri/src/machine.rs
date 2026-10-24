//! Commands for talking to a connected GRBL machine.

use std::sync::Mutex;

use kerf_gcode::MachineKind;
use kerf_grbl::{Connection, MachineState, PortInfo, Tool, DEFAULT_BAUD};
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
    Home,
    /// Clears an alarm lock.
    Unlock,
    /// Makes the current XY position (or Z position) the work origin.
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
}

fn finite(values: &[f64]) -> Result<(), String> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err("A coordinate or speed is not a number.".into())
    }
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
            let mut line = String::from("$J=G91 G21");
            for (axis, distance) in [('X', x), ('Y', y), ('Z', z)] {
                if distance != 0.0 {
                    line.push_str(&format!(" {axis}{distance:.3}"));
                }
            }
            machine.send_line(&format!("{line} F{feed:.0}"))
        }
        MachineAction::CancelJog => machine.cancel_jog(),
        MachineAction::Home => machine.send_line("$H"),
        MachineAction::Unlock => machine.send_line("$X"),
        MachineAction::SetOrigin { z: false } => machine.send_line("G10 L20 P1 X0 Y0"),
        MachineAction::SetOrigin { z: true } => machine.send_line("G10 L20 P1 Z0"),
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
            // Jog moves only: they cannot turn on a spindle or laser.
            let corners = [
                (min_x, min_y),
                (max_x, min_y),
                (max_x, max_y),
                (min_x, max_y),
                (min_x, min_y),
            ];
            for (x, y) in corners {
                machine.send_line(&format!("$J=G90 G21 X{x:.3} Y{y:.3} F{feed:.0}"))?;
            }
            Ok(())
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
            machine.start_job(&program.text, tool)
        }
        MachineAction::StopJob => machine.stop_job(),
    })
}
