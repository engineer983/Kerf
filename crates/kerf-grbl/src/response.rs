//! Parsing the lines GRBL sends back.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MachineState {
    Idle,
    Run,
    Hold,
    Jog,
    Alarm,
    Door,
    Check,
    Home,
    Sleep,
    /// A state this parser does not know; some firmware adds its own.
    Other,
}

impl MachineState {
    fn parse(text: &str) -> Self {
        // Hold and Door carry a sub-state, as in `Hold:0`.
        match text.split(':').next().unwrap_or("") {
            "Idle" => Self::Idle,
            "Run" => Self::Run,
            "Hold" => Self::Hold,
            "Jog" => Self::Jog,
            "Alarm" => Self::Alarm,
            "Door" => Self::Door,
            "Check" => Self::Check,
            "Home" => Self::Home,
            "Sleep" => Self::Sleep,
            _ => Self::Other,
        }
    }
}

/// A `<...>` real-time status report.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: MachineState,
    /// Machine position, if the report carries `MPos`.
    pub machine_position: Option<[f64; 3]>,
    /// Work position, if the report carries `WPos`.
    pub work_position: Option<[f64; 3]>,
    /// Work coordinate offset; GRBL only includes it every few reports.
    pub work_offset: Option<[f64; 3]>,
    pub feed: Option<f64>,
    pub spindle: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Ok,
    Error(u32),
    Alarm(u32),
    Status(Status),
    /// The banner sent after a reset, with the firmware version, e.g. `1.1h`.
    Welcome(String),
    /// A `$n=value` line from the `$$` settings dump.
    Setting(u32, String),
    /// Anything else, such as `[MSG:...]` feedback.
    Other(String),
}

impl Response {
    pub fn parse(line: &str) -> Self {
        let line = line.trim();
        if line == "ok" {
            return Self::Ok;
        }
        if let Some(code) = line
            .strip_prefix("error:")
            .and_then(|c| c.trim().parse().ok())
        {
            return Self::Error(code);
        }
        if let Some(code) = line
            .strip_prefix("ALARM:")
            .and_then(|c| c.trim().parse().ok())
        {
            return Self::Alarm(code);
        }
        if let Some(body) = line.strip_prefix('<').and_then(|l| l.strip_suffix('>')) {
            if let Some(status) = parse_status(body) {
                return Self::Status(status);
            }
        }
        if let Some(rest) = line.strip_prefix("Grbl ") {
            let version = rest.split_whitespace().next().unwrap_or("");
            return Self::Welcome(version.to_owned());
        }
        if let Some((number, value)) = line.strip_prefix('$').and_then(|l| l.split_once('=')) {
            if let Ok(number) = number.parse() {
                return Self::Setting(number, value.trim().to_owned());
            }
        }
        Self::Other(line.to_owned())
    }
}

fn parse_status(body: &str) -> Option<Status> {
    let mut fields = body.split('|');
    let mut status = Status {
        state: MachineState::parse(fields.next()?),
        machine_position: None,
        work_position: None,
        work_offset: None,
        feed: None,
        spindle: None,
    };
    for field in fields {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        let numbers: Vec<f64> = value.split(',').filter_map(|n| n.parse().ok()).collect();
        match key {
            "MPos" => status.machine_position = xyz(&numbers),
            "WPos" => status.work_position = xyz(&numbers),
            "WCO" => status.work_offset = xyz(&numbers),
            "FS" => {
                status.feed = numbers.first().copied();
                status.spindle = numbers.get(1).copied();
            }
            "F" => status.feed = numbers.first().copied(),
            _ => {}
        }
    }
    Some(status)
}

fn xyz(numbers: &[f64]) -> Option<[f64; 3]> {
    // Machines with more than three axes report extra values; keep X, Y and Z.
    (numbers.len() >= 3).then(|| [numbers[0], numbers[1], numbers[2]])
}

/// What a GRBL 1.1 `error:n` code means, for the codes a sender commonly meets.
pub fn describe_error(code: u32) -> &'static str {
    match code {
        1 => "G-code words consist of a letter and a value; a letter was missing.",
        2 => "A numeric value was missing or malformed.",
        3 => "The '$' system command was not recognised.",
        8 => "That command can only be used while the machine is idle.",
        9 => "G-code is locked out during an alarm or jog state.",
        15 => "The jog target is outside the machine's travel.",
        20 => "Unsupported or invalid G-code command.",
        22 => "No feed rate has been set.",
        24 => "Two G-code commands that both need the axis words were on one line.",
        33 => "The move's target is invalid.",
        _ => "See the GRBL documentation for this error code.",
    }
}

/// What a GRBL 1.1 `ALARM:n` code means.
pub fn describe_alarm(code: u32) -> &'static str {
    match code {
        1 => "A hard limit switch was triggered. Position is probably lost; re-home.",
        2 => "The move would exceed the machine's travel (soft limit).",
        3 => "Reset while moving. Position is probably lost; re-home.",
        4 | 5 => "The probe did not behave as expected.",
        6 => "Homing was reset before it finished.",
        7 => "The safety door was opened during homing.",
        8 | 9 => "Homing failed: a limit switch was not found or did not clear.",
        _ => "See the GRBL documentation for this alarm code.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_acknowledgements() {
        assert_eq!(Response::parse("ok\r"), Response::Ok);
        assert_eq!(Response::parse("error:20"), Response::Error(20));
        assert_eq!(Response::parse("ALARM:2"), Response::Alarm(2));
    }

    #[test]
    fn parses_a_status_report() {
        let Response::Status(status) =
            Response::parse("<Hold:0|MPos:10.000,-5.500,2.000|FS:1200,8000|WCO:1.000,2.000,3.000>")
        else {
            panic!("expected a status report");
        };
        assert_eq!(status.state, MachineState::Hold);
        assert_eq!(status.machine_position, Some([10.0, -5.5, 2.0]));
        assert_eq!(status.work_offset, Some([1.0, 2.0, 3.0]));
        assert_eq!((status.feed, status.spindle), (Some(1200.0), Some(8000.0)));
        assert_eq!(status.work_position, None);
    }

    #[test]
    fn parses_work_position_and_unknown_states() {
        let Response::Status(status) = Response::parse("<Tool|WPos:1.000,2.000,3.000,4.000|F:500>")
        else {
            panic!("expected a status report");
        };
        assert_eq!(status.state, MachineState::Other);
        assert_eq!(status.work_position, Some([1.0, 2.0, 3.0]));
        assert_eq!(status.feed, Some(500.0));
    }

    #[test]
    fn parses_banner_settings_and_messages() {
        assert_eq!(
            Response::parse("Grbl 1.1h ['$' for help]"),
            Response::Welcome("1.1h".into())
        );
        assert_eq!(
            Response::parse("$130=750.000"),
            Response::Setting(130, "750.000".into())
        );
        assert_eq!(
            Response::parse("[MSG:Caution: Unlocked]"),
            Response::Other("[MSG:Caution: Unlocked]".into())
        );
    }
}
