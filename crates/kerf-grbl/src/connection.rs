//! A live connection to a GRBL controller.
//!
//! One worker thread owns the transport. It polls status, sends manual lines
//! and streams jobs, and reports everything that happens through a callback.

use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::response::{describe_alarm, describe_error, MachineState, Response, Status};
use crate::stream::{Streamer, RX_BUFFER_BYTES};

const FEED_HOLD: u8 = b'!';
const CYCLE_START: u8 = b'~';
const STATUS_QUERY: u8 = b'?';
const SOFT_RESET: u8 = 0x18;
const JOG_CANCEL: u8 = 0x85;

const STATUS_INTERVAL: Duration = Duration::from_millis(200);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// How long a stop waits for the machine to report it has halted before
/// resetting anyway.
const STOP_TIMEOUT: Duration = Duration::from_secs(3);
/// GRBL's laser mode setting. With it on, the laser is off whenever the
/// machine is not moving, and power changes do not stop motion.
const LASER_MODE: u32 = 32;

/// What the machine cuts with, which decides how GRBL's laser mode must be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Needs laser mode on (`$32=1`), or the laser stays on when the machine
    /// pauses.
    Laser,
    /// Needs laser mode off (`$32=0`), or the spindle stops on every rapid
    /// move and restarts without waiting to reach speed.
    Spindle,
}

/// Anything that carries bytes to and from the controller. A serial port in
/// the app; a simulated controller in tests.
pub trait Transport: Read + Write + Send {}
impl<T: Read + Write + Send> Transport for T {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Running,
    Finished,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    Status {
        state: MachineState,
        /// Position in work coordinates, when the controller reports enough to know it.
        position: Option<[f64; 3]>,
        feed: Option<f64>,
        spindle: Option<f64>,
    },
    /// The controller restarted and identified itself.
    Welcome {
        version: String,
    },
    Setting {
        number: u32,
        value: String,
    },
    /// Any other line from the controller, for the console.
    Line {
        text: String,
    },
    /// A manually sent line was rejected.
    CommandError {
        line: String,
        code: u32,
        description: String,
    },
    Alarm {
        code: u32,
        description: String,
    },
    Job {
        state: JobState,
        acknowledged: usize,
        total: usize,
        /// Why the job failed, when it did.
        message: Option<String>,
    },
    Disconnected {
        reason: Option<String>,
    },
}

enum Command {
    Line(String),
    Realtime(u8),
    Reset,
    StartJob(Streamer),
    StopJob,
    Disconnect,
}

pub struct Connection {
    commands: Sender<Command>,
    state: Arc<Mutex<Option<MachineState>>>,
    /// The machine's `$` settings, as last read.
    settings: Arc<Mutex<HashMap<u32, String>>>,
    worker: Option<JoinHandle<()>>,
}

impl Connection {
    pub fn open(
        transport: Box<dyn Transport>,
        on_event: impl FnMut(Event) + Send + 'static,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let state = Arc::new(Mutex::new(None));
        let settings = Arc::new(Mutex::new(HashMap::new()));
        let worker = Worker {
            port: transport,
            on_event,
            commands: receiver,
            state: state.clone(),
            settings: settings.clone(),
            job: None,
            waiting_job: None,
            reset_pending: None,
            // Not every board restarts and announces itself when the port
            // opens, so the settings are asked for straight away.
            manual_queue: VecDeque::from(["$$".to_owned()]),
            manual_in_flight: VecDeque::new(),
            work_offset: [0.0; 3],
            last_status_query: Instant::now() - STATUS_INTERVAL,
            last_progress: Instant::now(),
            partial: Vec::new(),
        };
        let worker = std::thread::spawn(move || worker.run());
        Self {
            commands,
            state,
            settings,
            worker: Some(worker),
        }
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "The machine is no longer connected.".to_owned())
    }

    /// Sends one line of G-code or a `$` command. Refused while a job runs.
    pub fn send_line(&self, line: &str) -> Result<(), String> {
        let line = line.trim();
        if line.is_empty() || line.len() + 1 > RX_BUFFER_BYTES {
            return Err("The command is empty or too long.".into());
        }
        self.send(Command::Line(line.to_owned()))
    }

    /// Pauses motion; the spindle or laser state follows the controller's settings.
    pub fn hold(&self) -> Result<(), String> {
        self.send(Command::Realtime(FEED_HOLD))
    }

    pub fn resume(&self) -> Result<(), String> {
        self.send(Command::Realtime(CYCLE_START))
    }

    pub fn cancel_jog(&self) -> Result<(), String> {
        self.send(Command::Realtime(JOG_CANCEL))
    }

    /// Soft-resets the controller immediately, abandoning any job.
    pub fn reset(&self) -> Result<(), String> {
        self.send(Command::Reset)
    }

    /// Starts streaming `program`. The machine must be idle, with laser mode
    /// set to suit `tool`.
    pub fn start_job(&self, program: &str, tool: Tool) -> Result<(), String> {
        match *self.state.lock().unwrap() {
            Some(MachineState::Idle) => {}
            Some(state) => {
                return Err(format!(
                    "The machine must be idle to start a job; it is {state:?}."
                ))
            }
            None => return Err("The machine has not reported its state yet.".into()),
        }
        self.check_laser_mode(tool)?;
        let streamer = Streamer::new(program)?;
        if streamer.progress().1 == 0 {
            return Err("The program is empty.".into());
        }
        self.send(Command::StartJob(streamer))
    }

    /// Halts motion with a feed hold, then resets once the machine has stopped,
    /// so that the position is not lost.
    pub fn stop_job(&self) -> Result<(), String> {
        self.send(Command::StopJob)
    }

    /// The most recently reported machine state.
    pub fn state(&self) -> Option<MachineState> {
        *self.state.lock().unwrap()
    }

    fn check_laser_mode(&self, tool: Tool) -> Result<(), String> {
        let settings = self.settings.lock().unwrap();
        let Some(value) = settings.get(&LASER_MODE) else {
            return Err("Kerf has not read the machine's settings yet, so it cannot check laser mode ($32). Wait a moment, or send $$ from the console, and try again.".into());
        };
        let on = value.trim().parse::<f64>().is_ok_and(|v| v == 1.0);
        match tool {
            Tool::Laser if !on => Err(format!(
                "Laser mode is off on this machine ($32={value}). Without it the laser stays on whenever the machine pauses, and the head stops at every change of power. Send $32=1 from the console, then start again."
            )),
            Tool::Spindle if on => Err(
                "Laser mode is on ($32=1), so the spindle would stop on every rapid move and restart without waiting to reach speed. For a router, send $32=0 from the console, then start again.".into(),
            ),
            _ => Ok(()),
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Disconnect);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Worker<F> {
    port: Box<dyn Transport>,
    on_event: F,
    commands: Receiver<Command>,
    state: Arc<Mutex<Option<MachineState>>>,
    settings: Arc<Mutex<HashMap<u32, String>>>,
    job: Option<Streamer>,
    /// A job that starts once earlier manual lines have been acknowledged.
    waiting_job: Option<Streamer>,
    /// When a feed hold was sent that is to be followed by a soft reset, once
    /// the machine has come to rest (or after `STOP_TIMEOUT`). Resetting while
    /// still moving would lose the position.
    reset_pending: Option<Instant>,
    manual_queue: VecDeque<String>,
    /// Manual lines sent but not yet acknowledged.
    manual_in_flight: VecDeque<String>,
    work_offset: [f64; 3],
    last_status_query: Instant,
    last_progress: Instant,
    partial: Vec<u8>,
}

impl<F: FnMut(Event)> Worker<F> {
    fn run(mut self) {
        let reason = match self.serve() {
            Ok(()) => {
                self.shut_down();
                None
            }
            Err(e) => Some(e.to_string()),
        };
        *self.state.lock().unwrap() = None;
        (self.on_event)(Event::Disconnected { reason });
    }

    /// Runs until asked to disconnect (`Ok`) or the transport fails (`Err`).
    fn serve(&mut self) -> std::io::Result<()> {
        let mut chunk = [0u8; 256];
        loop {
            loop {
                match self.commands.try_recv() {
                    Ok(Command::Disconnect) | Err(TryRecvError::Disconnected) => return Ok(()),
                    Ok(command) => self.handle(command)?,
                    Err(TryRecvError::Empty) => break,
                }
            }

            self.pump()?;

            if self.last_status_query.elapsed() >= STATUS_INTERVAL {
                self.port.write_all(&[STATUS_QUERY])?;
                self.last_status_query = Instant::now();
            }
            if self
                .reset_pending
                .is_some_and(|since| since.elapsed() >= STOP_TIMEOUT)
            {
                self.finish_stop()?;
            }

            match self.port.read(&mut chunk) {
                Ok(n) => {
                    for &byte in &chunk[..n] {
                        if byte == b'\n' {
                            let line = String::from_utf8_lossy(&self.partial).into_owned();
                            self.partial.clear();
                            if !line.trim().is_empty() {
                                self.on_line(&line)?;
                            }
                        } else {
                            self.partial.push(byte);
                        }
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn handle(&mut self, command: Command) -> std::io::Result<()> {
        match command {
            Command::Line(line) => {
                if self.job.is_some() || self.waiting_job.is_some() {
                    self.emit(Event::Line {
                        text: format!("Not sent while a job is running: {line}"),
                    });
                } else {
                    self.manual_queue.push_back(line);
                }
            }
            Command::Realtime(CYCLE_START) if self.reset_pending.is_some() => {
                self.emit(Event::Line {
                    text: "Not resumed: the machine is being stopped.".into(),
                });
            }
            Command::Realtime(byte) => self.port.write_all(&[byte])?,
            Command::Reset => {
                self.port.write_all(&[SOFT_RESET])?;
                self.reset_pending = None;
                self.abandon(JobState::Stopped, None);
            }
            Command::StartJob(streamer) => self.waiting_job = Some(streamer),
            Command::StopJob => {
                if self.job.is_some() || self.is_moving() {
                    // Also when no job is streaming: the machine may still be
                    // working through the last lines of one, or a jog.
                    self.port.write_all(&[FEED_HOLD])?;
                    self.reset_pending.get_or_insert_with(Instant::now);
                } else if self.waiting_job.is_some() {
                    self.abandon(JobState::Stopped, None);
                }
            }
            Command::Disconnect => {}
        }
        Ok(())
    }

    /// Whether the last report says the machine is, or may be, in motion or
    /// with its spindle or laser on.
    fn is_moving(&self) -> bool {
        !matches!(
            *self.state.lock().unwrap(),
            None | Some(MachineState::Idle | MachineState::Alarm | MachineState::Sleep)
        )
    }

    /// Run when the connection is closed on purpose. Anything still running is
    /// reset, since once the port is closed nothing more can stop it.
    fn shut_down(&mut self) {
        if self.job.is_some() || self.reset_pending.is_some() || self.is_moving() {
            let _ = self.port.write_all(&[SOFT_RESET]);
            let _ = self.port.flush();
        }
        self.abandon(JobState::Stopped, None);
    }

    /// Writes whatever the controller currently has room for.
    fn pump(&mut self) -> std::io::Result<()> {
        if self.job.is_none()
            && self.waiting_job.is_some()
            && self.manual_queue.is_empty()
            && self.manual_in_flight.is_empty()
        {
            self.job = self.waiting_job.take();
            self.emit_progress(JobState::Running, None);
        }

        if let Some(job) = &mut self.job {
            if self.reset_pending.is_none() {
                while let Some(line) = job.next_line() {
                    self.port.write_all(line.as_bytes())?;
                    self.port.write_all(b"\n")?;
                }
            }
            return Ok(());
        }

        while let Some(line) = self.manual_queue.front() {
            let in_flight: usize = self.manual_in_flight.iter().map(|l| l.len() + 1).sum();
            if in_flight + line.len() + 1 > RX_BUFFER_BYTES {
                break;
            }
            self.port.write_all(line.as_bytes())?;
            self.port.write_all(b"\n")?;
            let line = self.manual_queue.pop_front().expect("front exists");
            self.manual_in_flight.push_back(line);
        }
        Ok(())
    }

    fn on_line(&mut self, line: &str) -> std::io::Result<()> {
        match Response::parse(line) {
            Response::Ok => {
                if let Some(job) = &mut self.job {
                    job.on_ok();
                    if job.is_finished() {
                        self.emit_progress(JobState::Finished, None);
                        self.job = None;
                    } else if self.last_progress.elapsed() >= PROGRESS_INTERVAL {
                        self.emit_progress(JobState::Running, None);
                    }
                } else if let Some(line) = self.manual_in_flight.pop_front() {
                    // A changed setting, such as `$32=1`, is read back so the
                    // checks before a job see it.
                    if is_setting_change(&line) {
                        self.manual_queue.push_back("$$".to_owned());
                    }
                }
            }
            Response::Error(code) => {
                if let Some(job) = &mut self.job {
                    job.on_error(code);
                    let message = job.error().map(|e| {
                        format!(
                            "Line {} ({}) was rejected with error {}: {} The machine has been stopped.",
                            e.line_index + 1,
                            e.line,
                            e.code,
                            describe_error(e.code)
                        )
                    });
                    // Lines already buffered would otherwise keep running. A
                    // hold alone leaves a spindle turning, so once the machine
                    // has come to rest it is reset, as Stop does.
                    self.port.write_all(&[FEED_HOLD])?;
                    self.abandon(JobState::Failed, message);
                    self.reset_pending.get_or_insert_with(Instant::now);
                } else if let Some(line) = self.manual_in_flight.pop_front() {
                    self.emit(Event::CommandError {
                        line,
                        code,
                        description: describe_error(code).to_owned(),
                    });
                }
            }
            Response::Alarm(code) => {
                let description = describe_alarm(code).to_owned();
                self.abandon(
                    JobState::Failed,
                    Some(format!("Alarm {code}: {description}")),
                );
                self.emit(Event::Alarm { code, description });
            }
            Response::Status(status) => self.on_status(status)?,
            Response::Welcome(version) => {
                // The controller has restarted, so everything queued is gone.
                self.reset_pending = None;
                self.abandon(JobState::Stopped, None);
                self.settings.lock().unwrap().clear();
                self.emit(Event::Welcome { version });
                self.manual_queue.push_back("$$".to_owned());
            }
            Response::Setting(number, value) => {
                self.settings.lock().unwrap().insert(number, value.clone());
                self.emit(Event::Setting { number, value });
            }
            Response::Other(text) => self.emit(Event::Line { text }),
        }
        Ok(())
    }

    fn on_status(&mut self, status: Status) -> std::io::Result<()> {
        if let Some(offset) = status.work_offset {
            self.work_offset = offset;
        }
        let position = status.work_position.or_else(|| {
            status
                .machine_position
                .map(|m| [0, 1, 2].map(|i| m[i] - self.work_offset[i]))
        });
        *self.state.lock().unwrap() = Some(status.state);
        self.emit(Event::Status {
            state: status.state,
            position,
            feed: status.feed,
            spindle: status.spindle,
        });
        // `Hold:1` means still slowing down; only `Hold:0` is at rest.
        let at_rest = match status.state {
            MachineState::Idle => true,
            MachineState::Hold => status.sub_state == Some(0),
            _ => false,
        };
        if self.reset_pending.is_some() && at_rest {
            self.finish_stop()?;
        }
        Ok(())
    }

    fn finish_stop(&mut self) -> std::io::Result<()> {
        self.port.write_all(&[SOFT_RESET])?;
        self.reset_pending = None;
        self.abandon(JobState::Stopped, None);
        Ok(())
    }

    /// Forgets all queued work, reporting the end of a job if one was active.
    /// A pending reset is kept.
    fn abandon(&mut self, state: JobState, message: Option<String>) {
        if self.job.is_some() || self.waiting_job.is_some() {
            self.emit_progress(state, message);
        }
        self.job = None;
        self.waiting_job = None;
        self.manual_queue.clear();
        self.manual_in_flight.clear();
    }

    fn emit_progress(&mut self, state: JobState, message: Option<String>) {
        let (acknowledged, total) = self
            .job
            .as_ref()
            .or(self.waiting_job.as_ref())
            .map(Streamer::progress)
            .unwrap_or((0, 0));
        self.last_progress = Instant::now();
        self.emit(Event::Job {
            state,
            acknowledged,
            total,
            message,
        });
    }

    fn emit(&mut self, event: Event) {
        (self.on_event)(event);
    }
}

/// A line such as `$32=1`, but not a jog (`$J=...`).
fn is_setting_change(line: &str) -> bool {
    line.strip_prefix('$')
        .and_then(|rest| rest.split_once('='))
        .is_some_and(|(number, _)| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simulated GRBL controller behind an in-memory transport.
    #[derive(Default)]
    struct Sim {
        partial: String,
        /// Lines received but not yet executed; this is the 127-byte buffer.
        buffered: VecDeque<String>,
        outgoing: VecDeque<u8>,
        executed: Vec<String>,
        realtime: Vec<u8>,
        overflowed: bool,
        held: bool,
        alarm: bool,
        /// A line that is answered with `error:20`.
        reject: Option<String>,
        /// The `$32` (laser mode) value reported by `$$`; not reported if `None`.
        laser_mode: Option<&'static str>,
        /// Status reports still to say `Hold:1` (slowing down) after a hold.
        settling: u32,
        /// How many status reports to say `Hold:1` for after each feed hold.
        settle_reports: u32,
        /// A reset arrived while the machine was still slowing down.
        reset_while_moving: bool,
    }

    impl Sim {
        fn say(&mut self, text: &str) {
            self.outgoing.extend(text.bytes());
            self.outgoing.extend(b"\r\n");
        }
    }

    #[derive(Clone, Default)]
    struct SimPort(Arc<Mutex<Sim>>);

    impl Write for SimPort {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let mut sim = self.0.lock().unwrap();
            for &byte in bytes {
                match byte {
                    STATUS_QUERY => {
                        let state = if sim.alarm {
                            "Alarm"
                        } else if sim.held && sim.settling > 0 {
                            sim.settling -= 1;
                            "Hold:1"
                        } else if sim.held {
                            "Hold:0"
                        } else if sim.buffered.is_empty() {
                            "Idle"
                        } else {
                            "Run"
                        };
                        sim.say(&format!(
                            "<{state}|MPos:5.000,6.000,7.000|FS:0,0|WCO:1.000,1.000,1.000>"
                        ));
                        sim.realtime.push(byte);
                    }
                    FEED_HOLD => {
                        if !sim.held {
                            sim.settling = sim.settle_reports;
                        }
                        sim.held = true;
                        sim.realtime.push(byte);
                    }
                    CYCLE_START => {
                        sim.held = false;
                        sim.realtime.push(byte);
                    }
                    SOFT_RESET => {
                        sim.reset_while_moving |= sim.held && sim.settling > 0;
                        sim.settling = 0;
                        sim.buffered.clear();
                        sim.partial.clear();
                        sim.held = false;
                        sim.realtime.push(byte);
                        sim.say("Grbl 1.1h ['$' for help]");
                    }
                    b'\n' => {
                        let line = std::mem::take(&mut sim.partial);
                        sim.buffered.push_back(line);
                        let bytes: usize = sim.buffered.iter().map(|l| l.len() + 1).sum();
                        sim.overflowed |= bytes > RX_BUFFER_BYTES;
                    }
                    _ => sim.partial.push(byte as char),
                }
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Read for SimPort {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            {
                let mut sim = self.0.lock().unwrap();
                // Execute one buffered line per read, like a busy controller.
                if !sim.held {
                    if let Some(line) = sim.buffered.pop_front() {
                        if sim.reject.as_deref() == Some(line.as_str()) {
                            sim.say("error:20");
                        } else {
                            if line == "$$" {
                                sim.say("$130=750.000");
                                if let Some(mode) = sim.laser_mode {
                                    sim.say(&format!("$32={mode}"));
                                }
                            }
                            if let Some(mode) = line.strip_prefix("$32=") {
                                sim.laser_mode = Some(if mode == "1" { "1" } else { "0" });
                            }
                            sim.say("ok");
                        }
                        sim.executed.push(line);
                    }
                }
                if !sim.outgoing.is_empty() {
                    let n = out.len().min(sim.outgoing.len());
                    for slot in &mut out[..n] {
                        *slot = sim.outgoing.pop_front().expect("length checked");
                    }
                    return Ok(n);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
            Err(ErrorKind::TimedOut.into())
        }
    }

    struct Harness {
        sim: Arc<Mutex<Sim>>,
        connection: Connection,
        events: Receiver<Event>,
    }

    fn connect(setup: impl FnOnce(&mut Sim)) -> Harness {
        let port = SimPort::default();
        port.0.lock().unwrap().laser_mode = Some("1");
        setup(&mut port.0.lock().unwrap());
        let (sender, events) = mpsc::channel();
        let connection = Connection::open(Box::new(port.clone()), move |event| {
            let _ = sender.send(event);
        });
        Harness {
            sim: port.0,
            connection,
            events,
        }
    }

    impl Harness {
        /// Waits for an event that `pick` accepts, ignoring the others.
        fn wait_for<T>(&self, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let event = self
                    .events
                    .recv_timeout(remaining)
                    .expect("timed out waiting for an event");
                if let Some(found) = pick(&event) {
                    return found;
                }
            }
        }

        fn wait_for_state(&self, wanted: MachineState) {
            self.wait_for(|e| {
                matches!(e, Event::Status { state, .. } if *state == wanted).then_some(())
            });
        }

        fn wait_for_job_end(&self) -> (JobState, usize, usize, Option<String>) {
            self.wait_for(|e| match e {
                Event::Job {
                    state,
                    acknowledged,
                    total,
                    message,
                } if *state != JobState::Running => {
                    Some((*state, *acknowledged, *total, message.clone()))
                }
                _ => None,
            })
        }
    }

    fn program(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("G1 X{i}.5 Y{} S500 F3000\n", i * 3))
            .collect()
    }

    #[test]
    fn reports_work_position_from_machine_position_and_offset() {
        let h = connect(|_| {});
        let position = h.wait_for(|e| match e {
            Event::Status { position, .. } => Some(*position),
            _ => None,
        });
        assert_eq!(position, Some([4.0, 5.0, 6.0]));
    }

    #[test]
    fn streams_a_job_to_completion_without_overflow() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        let text = program(400);
        h.connection.start_job(&text, Tool::Laser).unwrap();

        let (state, acknowledged, total, _) = h.wait_for_job_end();
        assert_eq!((state, acknowledged, total), (JobState::Finished, 400, 400));
        let sim = h.sim.lock().unwrap();
        assert!(!sim.overflowed);
        // The settings query sent on connecting runs first.
        assert_eq!(sim.executed[0], "$$");
        assert_eq!(sim.executed[1..], text.lines().collect::<Vec<_>>());
    }

    #[test]
    fn a_rejected_line_fails_the_job_and_pauses_the_machine() {
        let h = connect(|sim| sim.reject = Some("BAD".into()));
        h.wait_for_state(MachineState::Idle);
        h.connection
            .start_job(&format!("G21\nBAD\n{}", program(200)), Tool::Laser)
            .unwrap();

        let (state, _, _, message) = h.wait_for_job_end();
        assert_eq!(state, JobState::Failed);
        let message = message.unwrap();
        assert!(
            message.contains("Line 2 (BAD)") && message.contains("error 20"),
            "{message}"
        );
        // Once at rest the machine is reset, so a spindle does not keep turning.
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        let sim = h.sim.lock().unwrap();
        let hold = sim.realtime.iter().position(|&b| b == FEED_HOLD);
        let reset = sim.realtime.iter().position(|&b| b == SOFT_RESET);
        assert!(
            hold.is_some() && reset.is_some() && hold < reset,
            "{:?}",
            sim.realtime
        );
        assert!(sim.executed.len() < 202);
    }

    #[test]
    fn stop_holds_then_resets_and_sends_nothing_more() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        h.connection
            .start_job(&program(20_000), Tool::Laser)
            .unwrap();
        h.wait_for(|e| {
            matches!(
                e,
                Event::Job {
                    state: JobState::Running,
                    ..
                }
            )
            .then_some(())
        });
        h.connection.stop_job().unwrap();

        let (state, acknowledged, total, _) = h.wait_for_job_end();
        assert_eq!((state, total), (JobState::Stopped, 20_000));
        assert!(acknowledged < total);

        let executed_at_stop = h.sim.lock().unwrap().executed.len();
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        std::thread::sleep(Duration::from_millis(100));
        let sim = h.sim.lock().unwrap();
        let hold = sim
            .realtime
            .iter()
            .position(|&b| b == FEED_HOLD)
            .expect("a feed hold");
        let reset = sim
            .realtime
            .iter()
            .position(|&b| b == SOFT_RESET)
            .expect("a reset");
        assert!(hold < reset, "the hold must come before the reset");
        // Only the settings query sent after the restart ran; no more of the job.
        let after: Vec<_> = sim.executed[executed_at_stop..].iter().collect();
        assert!(after.iter().all(|line| *line == "$$"), "{after:?}");
    }

    #[test]
    fn a_job_is_refused_unless_the_machine_is_idle() {
        let h = connect(|sim| sim.alarm = true);
        assert!(h
            .connection
            .start_job("G21\n", Tool::Laser)
            .unwrap_err()
            .contains("not reported"));
        h.wait_for_state(MachineState::Alarm);
        assert!(h
            .connection
            .start_job("G21\n", Tool::Laser)
            .unwrap_err()
            .contains("must be idle"));
        h.sim.lock().unwrap().alarm = false;
        h.wait_for_state(MachineState::Idle);
        assert!(h
            .connection
            .start_job("; only a comment\n", Tool::Laser)
            .unwrap_err()
            .contains("empty"));
    }

    #[test]
    fn manual_lines_report_errors_with_a_description() {
        let h = connect(|sim| sim.reject = Some("G999".into()));
        h.wait_for_state(MachineState::Idle);
        h.connection.send_line("G999").unwrap();
        let (line, code, description) = h.wait_for(|e| match e {
            Event::CommandError {
                line,
                code,
                description,
            } => Some((line.clone(), *code, description.clone())),
            _ => None,
        });
        assert_eq!((line.as_str(), code), ("G999", 20));
        assert!(description.contains("Unsupported"));
    }

    #[test]
    fn a_restart_queries_the_settings() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        h.connection.reset().unwrap();
        h.wait_for(|e| matches!(e, Event::Welcome { version } if version == "1.1h").then_some(()));
        let setting = h.wait_for(|e| match e {
            Event::Setting { number, value } => Some((*number, value.clone())),
            _ => None,
        });
        assert_eq!(setting, (130, "750.000".into()));
    }

    fn wait_for_running(h: &Harness) {
        h.wait_for(|e| {
            matches!(
                e,
                Event::Job {
                    state: JobState::Running,
                    ..
                }
            )
            .then_some(())
        });
    }

    #[test]
    fn stop_waits_until_the_machine_is_at_rest_before_resetting() {
        let h = connect(|sim| sim.settle_reports = 4);
        h.wait_for_state(MachineState::Idle);
        h.connection
            .start_job(&program(20_000), Tool::Laser)
            .unwrap();
        wait_for_running(&h);
        h.connection.stop_job().unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Stopped);
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        let sim = h.sim.lock().unwrap();
        assert!(sim.realtime.contains(&SOFT_RESET));
        assert!(!sim.reset_while_moving, "reset while still slowing down");
    }

    #[test]
    fn resume_is_ignored_while_a_failed_job_is_being_stopped() {
        let h = connect(|sim| {
            sim.reject = Some("BAD".into());
            // Long enough that the resume arrives before the machine is at rest.
            sim.settle_reports = 1_000;
        });
        h.wait_for_state(MachineState::Idle);
        h.connection
            .start_job(&format!("BAD\n{}", program(200)), Tool::Laser)
            .unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Failed);
        h.connection.resume().unwrap();
        h.wait_for(|e| {
            matches!(e, Event::Line { text } if text.starts_with("Not resumed")).then_some(())
        });
        assert!(!h.sim.lock().unwrap().realtime.contains(&CYCLE_START));
    }

    #[test]
    fn a_laser_job_needs_laser_mode_on() {
        let h = connect(|sim| sim.laser_mode = Some("0"));
        h.wait_for_state(MachineState::Idle);
        let error = h.connection.start_job("G21\n", Tool::Laser).unwrap_err();
        assert!(error.contains("Laser mode is off"), "{error}");
        assert!(h.connection.start_job("G21\n", Tool::Spindle).is_ok());
    }

    #[test]
    fn a_router_job_needs_laser_mode_off() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        let error = h.connection.start_job("G21\n", Tool::Spindle).unwrap_err();
        assert!(error.contains("Laser mode is on"), "{error}");
    }

    #[test]
    fn a_job_is_refused_until_laser_mode_is_known() {
        let h = connect(|sim| sim.laser_mode = None);
        h.wait_for_state(MachineState::Idle);
        let error = h.connection.start_job("G21\n", Tool::Laser).unwrap_err();
        assert!(error.contains("not read the machine's settings"), "{error}");
    }

    #[test]
    fn changing_a_setting_reads_the_settings_again() {
        let h = connect(|sim| sim.laser_mode = Some("0"));
        h.wait_for_state(MachineState::Idle);
        assert!(h.connection.start_job("G21\n", Tool::Laser).is_err());
        h.connection.send_line("$32=1").unwrap();
        h.wait_for(|e| {
            matches!(e, Event::Setting { number: 32, value } if value == "1").then_some(())
        });
        h.wait_for_state(MachineState::Idle);
        assert!(h.connection.start_job("G21\n", Tool::Laser).is_ok());
    }

    #[test]
    fn disconnecting_during_a_job_resets_the_machine() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        h.connection
            .start_job(&program(20_000), Tool::Laser)
            .unwrap();
        wait_for_running(&h);
        let Harness {
            sim, connection, ..
        } = h;
        drop(connection);
        assert!(sim.lock().unwrap().realtime.contains(&SOFT_RESET));
    }

    #[test]
    fn disconnecting_when_idle_does_not_reset() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        let Harness {
            sim, connection, ..
        } = h;
        drop(connection);
        assert!(!sim.lock().unwrap().realtime.contains(&SOFT_RESET));
    }

    #[test]
    fn only_numbered_settings_count_as_setting_changes() {
        assert!(is_setting_change("$32=1"));
        assert!(!is_setting_change("$J=G91 X1 F100"));
        assert!(!is_setting_change("$$"));
        assert!(!is_setting_change("$H"));
    }

    #[test]
    fn dropping_the_connection_reports_a_disconnect() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        let Harness {
            connection, events, ..
        } = h;
        drop(connection);
        assert!(events
            .iter()
            .any(|e| e == Event::Disconnected { reason: None }));
    }
}
