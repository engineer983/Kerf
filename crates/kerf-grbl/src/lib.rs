//! GRBL / grblHAL sender.
//!
//! Built so far: response parsing, character-counting streaming, and a
//! threaded connection with status polling, manual commands, hold, resume,
//! reset and job control. Tested against a simulated controller only.
//! Before a job starts, the machine's laser mode (`$32`) is checked against
//! the tool the job uses.

pub mod connection;
pub mod response;
pub mod serial;
pub mod stream;

pub use connection::{Connection, Event, JobState, Tool, Transport};
pub use response::{describe_alarm, describe_error, MachineState, Response, Status};
pub use serial::{list_ports, PortInfo, DEFAULT_BAUD};
pub use stream::{StreamError, Streamer, RX_BUFFER_BYTES};
