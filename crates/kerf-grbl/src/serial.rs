//! Finding and opening serial ports.

use std::time::Duration;

use serde::Serialize;
use serialport::SerialPortType;

use crate::connection::Transport;

/// GRBL's standard speed.
pub const DEFAULT_BAUD: u32 = 115_200;
/// Short, so the connection's worker thread stays responsive between reads.
const READ_TIMEOUT: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    /// What to pass to [`open`], such as `COM5` or `/dev/ttyUSB0`.
    pub name: String,
    /// The USB product or manufacturer name, when the system knows it.
    pub description: Option<String>,
}

pub fn list_ports() -> Result<Vec<PortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| e.to_string())?;
    Ok(ports
        .into_iter()
        .map(|port| PortInfo {
            name: port.port_name,
            description: match port.port_type {
                SerialPortType::UsbPort(usb) => usb.product.or(usb.manufacturer),
                SerialPortType::BluetoothPort => Some("Bluetooth".to_owned()),
                _ => None,
            },
        })
        .collect())
}

pub fn open(name: &str, baud: u32) -> Result<Box<dyn Transport>, String> {
    let port = serialport::new(name, baud)
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|e| format!("Could not open {name}: {e}"))?;
    Ok(Box::new(port))
}
