//! GRBL / grblHAL sender.
//!
//! Built so far: response parsing and the character-counting stream
//! bookkeeping, neither of which touches a serial port.
//! Planned: serial port discovery and connection, status polling, real-time
//! commands (hold, resume, reset), jogging, framing and pre-flight checks.

pub mod response;
pub mod stream;

pub use response::{describe_alarm, describe_error, MachineState, Response, Status};
pub use stream::{StreamError, Streamer, RX_BUFFER_BYTES};
