//! A live connection to a GRBL controller.
//!
//! One worker thread owns the transport. It polls status, sends manual lines
//! and streams jobs, and reports everything that happens through a callback.

use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::jog;
use crate::probe;
use crate::response::{describe_alarm, describe_error, Firmware, MachineState, Response, Status};
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
/// How far ahead of the clock a held jog may send: segments sent × their
/// time, less the time since the hold began. Keeps the queued motion short,
/// so letting go (or losing the connection) stops the machine soon.
const HELD_LEAD: f64 = 0.3;
/// A held jog stops if the interface stops saying it is still held.
const HELD_WATCHDOG: Duration = Duration::from_millis(300);
/// Sent after a jog cancel to find where the replies have got to: GRBL
/// answers `$G` with a `[GC:...]` line and then `ok`, in any state, so the
/// `ok` after that line can be told apart from any for purged jog lines.
const RESYNC_MARKER: &str = "$G";
/// GRBL's acceleration settings, for X, Y and Z in turn, in mm/s².
const ACCELERATION: u32 = 120;
/// GRBL's laser mode setting. With it on, the laser is off whenever the
/// machine is not moving, and power changes do not stop motion.
const LASER_MODE: u32 = 32;
/// Asks for the work offsets: `[G54:...]`, `[G92:...]`, `[TLO:...]` and the
/// rest, then `ok`.
const OFFSETS_QUERY: &str = "$#";
/// Asks which firmware this is; grblHAL answers with `[FIRMWARE:grblHAL]`.
const BUILD_INFO_QUERY: &str = "$I";
/// How long Continue or Probe waits for the fresh offsets it asks for. Less
/// than [`REPLY_TIMEOUT`], so that the caller never hears of a failure while
/// the worker goes ahead anyway.
const READ_TIMEOUT: Duration = Duration::from_millis(1500);
/// How closely the X/Y zero must match its value when the job paused.
const ZERO_TOLERANCE: f64 = 0.001;

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
    /// A part has finished and the job waits, with the machine idle, for
    /// the bit to be changed and Z set again.
    WaitingForBit,
}

/// What a job needs beyond its lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobOptions {
    /// The job moves in machine coordinates (`G53`), which only mean
    /// something once the machine has homed.
    pub needs_homing: bool,
    /// The controller changes bits itself at each `M6`, which only grblHAL
    /// accepts.
    pub atc: bool,
    /// How high above the work zero the bit rises at each change, when it
    /// changes over the work zero.
    pub change_lift: Option<f64>,
    /// Positions the job goes to in machine coordinates, checked against
    /// the machine's travel.
    pub machine_moves: Vec<[f64; 3]>,
    /// How far above the work zero the bit must be at those positions: the
    /// larger of the lift and the job's highest safe height.
    pub change_clearance: Option<f64>,
}

/// Where a job with bit changes has got to.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartsProgress {
    /// The part running, or just finished, counting from 1.
    pub part: usize,
    pub of: usize,
    /// The part has finished and the job waits for a bit change.
    pub waiting: bool,
    /// The machine has said both that the spindle is switched off (`M5`)
    /// and that it reports a speed of 0.
    pub spindle_off: bool,
    /// Z has been set again since the job paused.
    pub z_set: bool,
    /// Z was set with the touch plate, so the clip must come off.
    pub plate_used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeState {
    Running,
    Done,
    Failed,
}

/// The work offsets from a `$#` reply.
#[derive(Debug, Clone, Default, PartialEq)]
struct Offsets {
    g54: Option<[f64; 3]>,
    g92: Option<[f64; 3]>,
    /// The tool length offset; grblHAL may give one value per axis.
    tlo: Option<Vec<f64>>,
}

impl Offsets {
    /// G92 and the tool length offset are both zero, so the work zero is
    /// G54 alone.
    fn only_g54(&self) -> bool {
        let zero = |v: &f64| v.abs() < 1e-6;
        self.g92.is_some_and(|g| g.iter().all(zero))
            && self.tlo.as_ref().is_some_and(|t| t.iter().all(zero))
    }
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
        /// Position from the machine's home, when the controller reports
        /// enough to know it. Only meaningful once the machine has homed.
        machine_position: Option<[f64; 3]>,
        feed: Option<f64>,
        spindle: Option<f64>,
        /// Input pins reported as on, such as `P` for the probe.
        pins: String,
    },
    /// The controller restarted and identified itself.
    Welcome {
        version: String,
        firmware: Firmware,
    },
    /// The firmware, as `$I` reported it.
    Firmware {
        firmware: Firmware,
    },
    /// Where a job with bit changes has got to; `None` once it has ended.
    Parts {
        progress: Option<PartsProgress>,
    },
    /// Setting Z with the touch plate.
    Probe {
        state: ProbeState,
        message: Option<String>,
    },
    /// The probe test passed: the plate touching the bit was seen.
    ProbeTested,
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

/// GRBL's maximum rate settings, for X, Y and Z in turn, in mm/min.
const MAX_RATE: u32 = 110;
/// GRBL's travel settings, for X, Y and Z in turn, in mm.
const TRAVEL: u32 = 130;
/// The longest one press of a held jog may run before it must be pressed
/// again, so that a stuck key or button cannot drive an axis for ever.
const HELD_MAX_TIME: Duration = Duration::from_secs(10);
/// How long to wait for the resync marker's reply, once the machine is
/// idle, before giving up on it.
const RESYNC_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a caller waits for the worker to accept or refuse a move.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the worker answers a request to move, or to start or carry on a job.
struct Reply {
    sender: Sender<Result<(), String>>,
    asked: Instant,
}

impl Reply {
    fn send(&self, answer: Result<(), String>) -> Result<(), mpsc::SendError<Result<(), String>>> {
        self.sender.send(answer)
    }

    /// Whether the caller may have stopped waiting, or soon will. A request
    /// answered this late is refused rather than carried out, so the caller
    /// is never told it failed while it goes ahead.
    fn late(&self) -> bool {
        self.asked.elapsed() >= READ_TIMEOUT
    }
}

const LATE: &str = "The machine did not answer in time. Try again.";

enum Command {
    Line(String),
    Realtime(u8),
    /// Jog lines, sent only if nothing stands in the way of moving.
    Move {
        lines: Vec<String>,
        reply: Reply,
    },
    HeldJogStart {
        direction: [i8; 3],
        feed: f64,
        reply: Reply,
    },
    HeldJogAlive,
    HeldJogStop,
    Reset,
    /// A job's parts: one for a job with no bit changes.
    StartJob {
        parts: Vec<Streamer>,
        options: JobOptions,
        reply: Reply,
    },
    StopJob,
    /// Streams the next part, once the checks on a fresh `$#` read pass.
    ContinueJob {
        plate_cleared: bool,
        reply: Reply,
    },
    ProbeZ {
        probe: ProbeSettings,
        reply: Reply,
    },
    /// Watches for the probe pin, to show that the plate and clip work.
    ProbeTest,
    /// Makes the current position the work zero in X and Y, or in Z.
    Zero {
        xy: bool,
        reply: Reply,
    },
    Disconnect,
}

/// How to set Z with the touch plate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeSettings {
    /// The plate's thickness in millimetres.
    pub plate: f64,
    /// The furthest the bit may go down.
    pub travel: f64,
    pub fast_feed: f64,
    pub slow_feed: f64,
}

/// Whether the spindle or laser is switched on (`M3`/`M4`) or off (`M5`)
/// in GRBL's modal state. In laser mode an `M3` with no motion reports a
/// speed of 0, yet the next move would fire the laser, so the reported speed
/// alone cannot be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpindleMode {
    Off,
    On,
    Unknown,
}

pub struct Connection {
    commands: Sender<Command>,
    state: Arc<Mutex<Option<MachineState>>>,
    /// The machine's `$` settings, as last read.
    settings: Arc<Mutex<HashMap<u32, String>>>,
    worker: Option<JoinHandle<()>>,
}

/// Locks, carrying on even if the worker thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
            spindle: None,
            spindle_mode: SpindleMode::Unknown,
            job: None,
            waiting_job: None,
            reset_pending: None,
            // Not every board restarts and announces itself when the port
            // opens, so the settings, and whether the spindle or laser is
            // switched on, are asked for straight away.
            manual_queue: VecDeque::from(["$$".to_owned(), "$G".to_owned()]),
            manual_in_flight: VecDeque::new(),
            held: None,
            resync: None,
            work_offset: None,
            position: None,
            machine_position: None,
            reported_z: None,
            pins: String::new(),
            firmware: None,
            build_info_grblhal: false,
            seen_home: false,
            homed: false,
            offsets: None,
            reading: Offsets::default(),
            reads_queued: 0,
            reads_done: 0,
            pending: None,
            continuing: None,
            probe_request: None,
            starting: None,
            on_g54: None,
            probing: None,
            probe_watch: false,
            probe_tested: false,
            last_parts: Some(None),
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

    /// Sends a command that the worker accepts or refuses, and waits for its answer.
    fn ask(&self, command: impl FnOnce(Reply) -> Command) -> Result<(), String> {
        let (sender, answer) = mpsc::channel();
        self.send(command(Reply {
            sender,
            asked: Instant::now(),
        }))?;
        match answer.recv_timeout(REPLY_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                // The worker is alive but slow. It may yet accept the move
                // after the caller has been told it failed, so it is
                // cancelled: the cancel follows the move in the queue.
                let _ = self.send(Command::Realtime(JOG_CANCEL));
                Err("The machine did not answer in time, so Kerf has cancelled the move. Try again.".into())
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("The machine is no longer connected.".into())
            }
        }
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

    /// A relative jog of `dx`, `dy`, `dz` millimetres. Refused unless the
    /// laser or spindle is known to be off and nothing else is waiting.
    pub fn jog(&self, dx: f64, dy: f64, dz: f64, feed: f64) -> Result<(), String> {
        check_numbers(&[dx, dy, dz], feed)?;
        let lines = vec![jog::step_line(dx, dy, dz, feed)];
        self.ask(|reply| Command::Move { lines, reply })
    }

    /// Moves back to the work zero in X and Y, at the current height. Refused
    /// as [`jog`](Self::jog) is.
    pub fn go_to_xy_zero(&self, feed: f64) -> Result<(), String> {
        check_numbers(&[], feed)?;
        let lines = vec![jog::go_to_xy_zero_line(feed)];
        self.ask(|reply| Command::Move { lines, reply })
    }

    /// Traces a rectangle in work coordinates, without cutting. Refused as
    /// [`jog`](Self::jog) is.
    pub fn frame(
        &self,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        feed: f64,
    ) -> Result<(), String> {
        check_numbers(&[min_x, min_y, max_x, max_y], feed)?;
        let lines = jog::frame_lines(min_x, min_y, max_x, max_y, feed);
        self.ask(|reply| Command::Move { lines, reply })
    }

    pub fn cancel_jog(&self) -> Result<(), String> {
        self.send(Command::Realtime(JOG_CANCEL))
    }

    /// Starts moving along one axis (`direction` is −1, 0 or 1 for each of
    /// X, Y and Z, with exactly one not 0) at `feed` mm/min, capped at the
    /// axis's maximum rate, for as long as
    /// [`keep_held_jog`](Self::keep_held_jog) keeps arriving. It stops on
    /// [`stop_held_jog`](Self::stop_held_jog), after 300 ms without a
    /// keep-alive, on any error, alarm or change of state, and after 10 s or
    /// the axis's full travel, whichever is first.
    pub fn start_held_jog(&self, direction: [i8; 3], feed: f64) -> Result<(), String> {
        let moving = (0..3).filter(|&i| direction[i] != 0).count();
        if direction.iter().any(|d| !(-1..=1).contains(d)) || moving != 1 {
            return Err("Hold to move goes along one axis at a time.".into());
        }
        check_numbers(&[], feed)?;
        self.ask(|reply| Command::HeldJogStart {
            direction,
            feed,
            reply,
        })
    }

    /// Says the button or key is still held.
    pub fn keep_held_jog(&self) -> Result<(), String> {
        self.send(Command::HeldJogAlive)
    }

    /// Stops a held jog with a jog cancel. Does nothing if none is running.
    pub fn stop_held_jog(&self) -> Result<(), String> {
        self.send(Command::HeldJogStop)
    }

    /// Soft-resets the controller immediately, abandoning any job.
    pub fn reset(&self) -> Result<(), String> {
        self.send(Command::Reset)
    }

    /// Starts streaming `program`. The machine must be idle, with laser mode
    /// set to suit `tool`.
    pub fn start_job(&self, program: &str, tool: Tool) -> Result<(), String> {
        self.start_parts(&[program], tool, &JobOptions::default())
    }

    /// Starts a job made of `parts`, one per bit. Each part after the first
    /// waits until [`continue_job`](Self::continue_job) is called and its
    /// checks pass. One part runs exactly as [`start_job`](Self::start_job).
    pub fn start_parts(
        &self,
        parts: &[&str],
        tool: Tool,
        options: &JobOptions,
    ) -> Result<(), String> {
        match *lock(&self.state) {
            Some(MachineState::Idle) => {}
            Some(state) => {
                return Err(format!(
                    "The machine must be idle to start a job; it is {state:?}."
                ))
            }
            None => return Err("The machine has not reported its state yet.".into()),
        }
        self.check_laser_mode(tool)?;
        if parts.is_empty() {
            return Err("The program is empty.".into());
        }
        let mut streamers = Vec::with_capacity(parts.len());
        for part in parts {
            let streamer = Streamer::new(part)?;
            if streamer.progress().1 == 0 {
                return Err("The program is empty.".into());
            }
            streamers.push(streamer);
        }
        let options = options.clone();
        self.ask(|reply| Command::StartJob {
            parts: streamers,
            options,
            reply,
        })
    }

    /// Streams the next part of a job that waits for a bit change. Refused
    /// unless the spindle is known to be off, Z has been set again, the X/Y
    /// zero is as it was when the job paused, and the touch plate's clip is
    /// off (`plate_cleared`) if the plate was used.
    pub fn continue_job(&self, plate_cleared: bool) -> Result<(), String> {
        self.ask(|reply| Command::ContinueJob {
            plate_cleared,
            reply,
        })
    }

    /// Sets Z with a touch plate under the bit. Allowed when the machine
    /// could be jogged, and only once the probe test has passed.
    pub fn probe_z(&self, probe: ProbeSettings) -> Result<(), String> {
        let ProbeSettings {
            plate,
            travel,
            fast_feed,
            slow_feed,
        } = probe;
        if !(plate.is_finite() && plate > 0.0 && plate <= 50.0) {
            return Err("Enter the touch plate's thickness first (in the Device step).".into());
        }
        if !(travel.is_finite() && travel > 0.0 && travel <= 50.0) {
            return Err("The probing distance must be between 0 and 50 mm.".into());
        }
        if !(fast_feed.is_finite() && fast_feed > 0.0 && fast_feed <= 500.0)
            || !(slow_feed.is_finite() && slow_feed > 0.0 && slow_feed <= fast_feed)
        {
            return Err("The probing speeds must be slow: at most 500 mm/min, and the second no faster than the first.".into());
        }
        self.ask(|reply| Command::ProbeZ { probe, reply })
    }

    /// Starts watching for the probe pin. The test passes once the machine
    /// reports the plate touching the bit.
    pub fn probe_test(&self) -> Result<(), String> {
        self.send(Command::ProbeTest)
    }

    /// Makes the current X and Y the work zero, in the active coordinate
    /// system. Refused while a job waits for a bit change.
    pub fn zero_xy(&self) -> Result<(), String> {
        self.ask(|reply| Command::Zero { xy: true, reply })
    }

    /// Makes the current Z the work zero, in the active coordinate system.
    pub fn zero_z(&self) -> Result<(), String> {
        self.ask(|reply| Command::Zero { xy: false, reply })
    }

    /// Halts motion with a feed hold, then resets once the machine has stopped,
    /// so that the position is not lost. While homing it resets at once.
    pub fn stop_job(&self) -> Result<(), String> {
        self.send(Command::StopJob)
    }

    /// The most recently reported machine state.
    pub fn state(&self) -> Option<MachineState> {
        *lock(&self.state)
    }

    fn check_laser_mode(&self, tool: Tool) -> Result<(), String> {
        match laser_mode_problem(&lock(&self.settings), tool) {
            Some(problem) => Err(problem),
            None => Ok(()),
        }
    }
}

/// Why the machine's laser mode (`$32`) does not suit `tool`, if it does not.
fn laser_mode_problem(settings: &HashMap<u32, String>, tool: Tool) -> Option<String> {
    let Some(value) = settings.get(&LASER_MODE) else {
        return Some("Kerf has not read the machine's settings yet, so it cannot check laser mode ($32). Wait a moment, or send $$ from the console, and try again.".into());
    };
    let on = value.trim().parse::<f64>().is_ok_and(|v| v == 1.0);
    match tool {
        Tool::Laser if !on => Some(format!(
            "Laser mode is off on this machine ($32={value}). Without it the laser stays on whenever the machine pauses, and the head stops at every change of power. Send $32=1 from the console, then start again."
        )),
        Tool::Spindle if on => Some(
            "Laser mode is on ($32=1), so the spindle would stop on every rapid move and restart without waiting to reach speed. For a router, send $32=0 from the console, then start again.".into(),
        ),
        _ => None,
    }
}

fn check_numbers(values: &[f64], feed: f64) -> Result<(), String> {
    if !values.iter().chain([&feed]).all(|v| v.is_finite()) {
        return Err("A coordinate or speed is not a number.".into());
    }
    if feed <= 0.0 {
        return Err("The speed must be greater than zero.".into());
    }
    Ok(())
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
    /// The spindle or laser speed in the last status report, if it gave one.
    spindle: Option<f64>,
    spindle_mode: SpindleMode,
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
    held: Option<HeldJog>,
    /// Set after a jog cancel while lines were unanswered. Nothing more is
    /// sent until the replies are back in step.
    resync: Option<Resync>,
    /// The last `WCO` reported; GRBL only sends it every few reports.
    work_offset: Option<[f64; 3]>,
    /// The last work and machine positions worked out from a status report.
    position: Option<[f64; 3]>,
    machine_position: Option<[f64; 3]>,
    /// The Z the last status report itself gave: machine Z from `MPos`, or
    /// work Z from `WPos`, with no stored offset applied.
    reported_z: Option<ReportedZ>,
    /// Input pins on in the last status report; `P` is the probe.
    pins: String,
    firmware: Option<Firmware>,
    /// A `[FIRMWARE:grblHAL]` line came with the `$I` reply in flight.
    build_info_grblhal: bool,
    /// Homing was seen under way; once the machine is idle after it, it has homed.
    seen_home: bool,
    /// The machine has homed since Kerf connected, with no alarm or reset since.
    homed: bool,
    /// The last complete `$#` reply, and the one coming in.
    offsets: Option<Offsets>,
    reading: Offsets,
    /// `$#` reads asked for, and answered, since connecting. A request that
    /// needs a fresh read waits for the one it asked for.
    reads_queued: u64,
    reads_done: u64,
    /// The parts of a job still to run, while it has more than one.
    pending: Option<PendingParts>,
    /// A Continue waiting for its fresh `$#` read.
    continuing: Option<ReadRequest<bool>>,
    /// A probe waiting for its fresh `$#` read before it starts.
    probe_request: Option<ReadRequest<ProbeSettings>>,
    /// A job with bit changes waiting for its fresh `$#` read before it starts.
    starting: Option<ReadRequest<(Vec<Streamer>, JobOptions)>>,
    /// Whether G54 is the coordinate system in use, from the last `[GC:...]`
    /// report; `None` until one says, or after a typed line may change it.
    on_g54: Option<bool>,
    probing: Option<ProbeRun>,
    /// The probe test is watching for the pin.
    probe_watch: bool,
    /// The probe test has passed since connecting.
    probe_tested: bool,
    /// The last [`Event::Parts`] sent, so that only changes are sent.
    last_parts: Option<Option<PartsProgress>>,
    last_status_query: Instant,
    last_progress: Instant,
    partial: Vec<u8>,
}

/// A jog that runs while the person holds a button or key.
struct HeldJog {
    direction: [i8; 3],
    feed: f64,
    /// Time each segment covers, in seconds.
    dt: f64,
    /// The axis's travel ($130 to $132), if known: one press goes no further.
    max_distance: Option<f64>,
    started: Instant,
    /// Segments sent so far.
    sent: u32,
    /// When the interface last said it is still held.
    alive: Instant,
}

/// The parts of a job after the first, kept apart from `job` and
/// `waiting_job` so that while the job waits for a bit change the machine
/// can be jogged and zeroed under the usual rules.
struct PendingParts {
    parts: VecDeque<Streamer>,
    /// The part running or just finished, counting from 1.
    part: usize,
    of: usize,
    stage: PartStage,
    /// How far the part that just finished got, for reporting.
    finished: (usize, usize),
    /// The offsets read once the part had finished.
    offsets_at_pause: Option<Offsets>,
    z_set: bool,
    plate_used: bool,
    change_lift: Option<f64>,
    machine_moves: Vec<[f64; 3]>,
    change_clearance: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartStage {
    /// A part is being streamed (or waits to start).
    Running,
    /// Its last line has been answered; it is over once a status report
    /// after that says the machine is idle.
    Ending,
    /// Over; waiting for the `$#` read that records the offsets.
    Reading(u64),
    /// Waiting for the bit change.
    Waiting,
}

/// The Z a status report gave, as it gave it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReportedZ {
    Machine(f64),
    Work(f64),
}

/// A request answered once a `$#` read it asked for has come back.
struct ReadRequest<T> {
    ticket: u64,
    reply: Reply,
    value: T,
}

/// Setting Z with the touch plate, one line at a time.
struct ProbeRun {
    settings: ProbeSettings,
    stage: ProbeStage,
    /// The line in flight, whose answer moves the sequence on.
    line: String,
    /// The work offset in Z when probing began; G92 and the tool length
    /// offset were zero, so this turns machine Z into work Z.
    g54_z: f64,
    /// Machine Z of the latest touch, if the line in flight touched.
    touched: Option<f64>,
    /// Work Z of the first, quick touch.
    first_work_z: f64,
}

const PROBE_FAILED: &str = "The probe didn't touch (or was already touching). Unlock, check the clip and the plate, and try again.";
const Z_NOT_SET: &str = "Set Z again for the new bit before continuing: probe it, or lower it onto the material and press Zero Z here.";
const NO_OFFSETS: &str =
    "The machine did not report its work offsets in time, so Kerf has not carried on. Try again.";
const INTERRUPTED: &str = "That was interrupted before the machine answered. Try again.";

/// A number with up to three decimals and no trailing zeros, for messages.
fn trim(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProbeStage {
    Fast,
    BackOff,
    Slow,
    SetZ,
    Lift,
    /// Waiting for the `$#` read that shows Z was set to this.
    Confirm {
        ticket: u64,
        g54_z: f64,
    },
}

struct Resync {
    stage: ResyncStage,
    since: Instant,
    /// `[GC:...]` replies still due for `$G` lines typed before the marker,
    /// which must not be taken for the marker's.
    replies_ahead: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResyncStage {
    /// The marker waits for room in the receive buffer.
    Waiting,
    /// The marker is sent; its `[GC:...]` reply has not come back yet.
    Sent,
    /// The `[GC:...]` reply came back, so the next `ok` is the marker's.
    Answered,
}

impl<F: FnMut(Event)> Worker<F> {
    fn run(mut self) {
        let served = panic::catch_unwind(AssertUnwindSafe(|| self.serve()));
        let reason = match served {
            Ok(Ok(())) => {
                self.shut_down();
                None
            }
            Ok(Err(e)) => Some(e.to_string()),
            Err(_) => {
                // Nothing is watching the machine any more, so stop whatever
                // it is doing: a jog cancel for a jog, then a reset for the rest.
                let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                    let _ = self.port.write_all(&[JOG_CANCEL, SOFT_RESET]);
                    let _ = self.port.flush();
                }));
                Some("Kerf's link to the machine failed unexpectedly, so it reset the machine. Check the machine, then connect again".into())
            }
        };
        *lock(&self.state) = None;
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

            if self
                .held
                .as_ref()
                .is_some_and(|held| held.alive.elapsed() > HELD_WATCHDOG)
            {
                self.stop_held(Some(
                    "Stopped moving: Kerf stopped hearing that the button was held.",
                ))?;
            }

            self.pump()?;
            self.expire_requests();

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
                if self.job.is_some() || self.waiting_job.is_some() || self.part_ending() {
                    self.emit(Event::Line {
                        text: format!("Not sent while a job is running: {line}"),
                    });
                } else if self.checking() {
                    self.emit(Event::Line {
                        text: format!(
                            "Not sent while Kerf is probing or checking the machine: {line}"
                        ),
                    });
                } else {
                    if is_offsets_query(&line) {
                        self.reads_queued += 1;
                    }
                    // A line that may change the work zero in use: ask again.
                    let reread = selects_work_zero(&line);
                    if reread {
                        self.on_g54 = None;
                    }
                    // Counted from when it is queued, so a move asked for
                    // after a typed M3 is refused even before the M3 is sent.
                    if let Some(on) = spindle_words(&line) {
                        self.spindle_mode = if on {
                            SpindleMode::On
                        } else {
                            SpindleMode::Off
                        };
                    }
                    self.manual_queue.push_back(line);
                    if reread {
                        self.manual_queue.push_back(RESYNC_MARKER.to_owned());
                    }
                }
            }
            Command::Realtime(CYCLE_START) if self.reset_pending.is_some() => {
                self.emit(Event::Line {
                    text: "Not resumed: the machine is being stopped.".into(),
                });
            }
            Command::Realtime(JOG_CANCEL) => {
                // Jog lines not yet sent are dropped too, or they would run
                // straight after the cancel.
                self.manual_queue.retain(|line| !line.starts_with("$J="));
                self.held = None;
                self.cancel_jog()?;
            }
            Command::Realtime(byte) => self.port.write_all(&[byte])?,
            Command::Move { lines, reply } => {
                let answer = self.check_can_move();
                if answer.is_ok() {
                    self.manual_queue.extend(lines);
                }
                let _ = reply.send(answer);
            }
            Command::HeldJogStart {
                direction,
                feed,
                reply,
            } => {
                let answer = self.start_held(direction, feed);
                let _ = reply.send(answer);
            }
            Command::HeldJogAlive => {
                if let Some(held) = &mut self.held {
                    held.alive = Instant::now();
                }
            }
            Command::HeldJogStop => self.stop_held(None)?,
            Command::Reset => {
                self.port.write_all(&[SOFT_RESET])?;
                self.after_reset();
                self.abandon(JobState::Stopped, None);
            }
            Command::StartJob {
                parts,
                options,
                reply,
            } => {
                let answer = if reply.late() {
                    Err(LATE.into())
                } else {
                    self.check_can_start(&options)
                };
                let checked = parts.len() > 1
                    || options.atc
                    || options.needs_homing
                    || options.change_lift.is_some();
                match answer {
                    // A job that changes bits starts from a fresh read of
                    // the offsets, checked before anything is sent.
                    Ok(()) if checked => {
                        let ticket = self.queue_offsets_read();
                        self.starting = Some(ReadRequest {
                            ticket,
                            reply,
                            value: (parts, options),
                        });
                    }
                    Ok(()) => {
                        self.begin_job(parts, options)?;
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::ContinueJob {
                plate_cleared,
                reply,
            } => {
                let answer = if reply.late() {
                    Err(LATE.into())
                } else {
                    self.check_can_continue()
                };
                match answer {
                    Ok(()) => {
                        // Answered once a fresh read of the offsets is back.
                        let ticket = self.queue_offsets_read();
                        self.continuing = Some(ReadRequest {
                            ticket,
                            reply,
                            value: plate_cleared,
                        });
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::ProbeZ { probe, reply } => {
                let answer = if reply.late() {
                    Err(LATE.into())
                } else {
                    self.check_can_probe()
                };
                match answer {
                    Ok(()) => {
                        let ticket = self.queue_offsets_read();
                        self.probe_request = Some(ReadRequest {
                            ticket,
                            reply,
                            value: probe,
                        });
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Command::ProbeTest => {
                self.probe_watch = true;
                if self.pins.contains('P') {
                    self.probe_passed();
                }
            }
            Command::Zero { xy, reply } => {
                let answer = if reply.late() {
                    Err(LATE.into())
                } else if self.job.is_some() || self.waiting_job.is_some() || self.part_ending() {
                    Err("A job is running.".into())
                } else if self.checking() {
                    Err("Kerf is probing or checking the machine. Try again in a moment.".into())
                } else if xy && self.pending.is_some() {
                    Err("Zero XY is turned off while the job waits for a bit change: moving the X/Y zero would cut the rest of the job in the wrong place.".into())
                } else {
                    // P0 is the coordinate system in use, which is the one
                    // the job's G54 and the checks at Continue read.
                    self.manual_queue.push_back(
                        if xy {
                            "G10 L20 P0 X0 Y0"
                        } else {
                            "G10 L20 P0 Z0"
                        }
                        .to_owned(),
                    );
                    Ok(())
                };
                let _ = reply.send(answer);
            }
            Command::StopJob if self.held.is_some() => {
                // A jog is stopped with a jog cancel, never a feed hold: a hold
                // that arrives once the jog has ended would pause the machine.
                self.stop_held(None)?;
            }
            Command::StopJob => {
                if matches!(
                    *lock(&self.state),
                    Some(MachineState::Home | MachineState::Tool)
                ) {
                    // GRBL ignores a feed hold while homing; only a soft reset
                    // stops it. It then raises an alarm (homing aborted).
                    // grblHAL waiting at a tool change is stopped the same way.
                    self.port.write_all(&[SOFT_RESET])?;
                    self.after_reset();
                    self.abandon(JobState::Stopped, None);
                } else if self.job.is_some() || self.is_moving() {
                    // Also when no job is streaming: the machine may still be
                    // working through the last lines of one, or a jog.
                    self.port.write_all(&[FEED_HOLD])?;
                    self.reset_pending.get_or_insert_with(Instant::now);
                } else if self.waiting_job.is_some() || self.pending.is_some() || self.checking() {
                    // Waiting for a bit change, or about to probe: nothing
                    // is moving, so ending the job is enough.
                    self.abandon(JobState::Stopped, None);
                }
            }
            Command::Disconnect => {}
        }
        Ok(())
    }

    /// Why the machine must not be jogged now, if it must not. Jog lines
    /// cannot switch a laser or spindle on, but they carry along one that is
    /// already on, so it must be known to be off.
    fn check_can_move(&self) -> Result<(), String> {
        if self.job.is_some() || self.waiting_job.is_some() || self.part_ending() {
            return Err("A job is running.".into());
        }
        if self.checking() {
            return Err("Kerf is probing or checking the machine. Try again in a moment.".into());
        }
        self.check_machine_free()
    }

    /// A part has finished but the job is not yet waiting for the bit
    /// change: the machine has not yet been seen idle, or the offsets read.
    fn part_ending(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|p| matches!(p.stage, PartStage::Ending | PartStage::Reading(_)))
    }

    /// The checks before moving that do not depend on a job: nothing being
    /// stopped, the spindle known to be off, and nothing else waiting.
    fn check_machine_free(&self) -> Result<(), String> {
        if self.reset_pending.is_some() {
            return Err("The machine is being stopped.".into());
        }
        match self.spindle {
            None => return Err("The machine has not said whether the laser or spindle is off, so Kerf will not move it. Wait a moment and try again.".into()),
            Some(speed) if speed > 0.0 => return Err(format!(
                "The laser or spindle is on (S{speed}). Send M5 from the console before moving the machine."
            )),
            Some(_) => {}
        }
        match self.spindle_mode {
            SpindleMode::Off => {}
            SpindleMode::On => return Err("The laser or spindle is switched on (M3 or M4), even if it is not running yet: it would come on as soon as the machine moved. Send M5 from the console before moving the machine.".into()),
            SpindleMode::Unknown => return Err("Kerf does not know yet whether the laser or spindle is switched off. Send M5 from the console, then try again.".into()),
        }
        // An M3, M4 or M5 not yet answered has not taken effect, and a `$G`
        // reply sent before it ran does not show it.
        if self
            .manual_queue
            .iter()
            .chain(&self.manual_in_flight)
            .any(|l| spindle_words(l).is_some())
        {
            return Err(
                "The machine is still busy with the last M3, M4 or M5. Try again in a moment."
                    .into(),
            );
        }
        // Moves asked for while earlier ones wait would run back to back.
        if self.held.is_some() || self.resync.is_some() || !self.manual_queue.is_empty() {
            return Err(
                "The machine is still busy with earlier commands. Try again in a moment.".into(),
            );
        }
        Ok(())
    }

    fn start_held(&mut self, direction: [i8; 3], feed: f64) -> Result<(), String> {
        self.check_can_move()?;
        if *lock(&self.state) != Some(MachineState::Idle) || !self.manual_in_flight.is_empty() {
            return Err("The machine must be idle to start moving.".into());
        }
        let axis = direction
            .iter()
            .position(|&d| d != 0)
            .ok_or("Hold to move goes along one axis at a time.")? as u32;
        let (accel, max_rate, travel) = {
            let settings = lock(&self.settings);
            let read = |number: u32| {
                settings
                    .get(&number)
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .filter(|v| v.is_finite() && *v > 0.0)
            };
            (
                read(ACCELERATION + axis),
                read(MAX_RATE + axis),
                read(TRAVEL + axis),
            )
        };
        let Some(accel) = accel else {
            return Err("Kerf has not read the machine's acceleration ($120 to $122) yet, so it cannot move while held. Wait a moment, or send $$ from the console, and try again.".into());
        };
        // Never faster than the machine's own limit for the axis.
        let feed = max_rate.map_or(feed, |max| feed.min(max));
        let now = Instant::now();
        self.held = Some(HeldJog {
            direction,
            feed,
            dt: jog::segment_time(feed, accel),
            max_distance: travel,
            started: now,
            sent: 0,
            alive: now,
        });
        Ok(())
    }

    /// A `$n` setting as a number, if it has been read.
    fn setting(&self, number: u32) -> Option<f64> {
        lock(&self.settings)
            .get(&number)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    }

    /// Why a job must not start now, if it must not. Only the checks the
    /// worker can make; the caller has checked the state and laser mode.
    fn check_can_start(&mut self, options: &JobOptions) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("A job is waiting for a bit change. Continue or stop it first.".into());
        }
        if self.job.is_some() || self.waiting_job.is_some() {
            return Err("A job is already running.".into());
        }
        if self.checking() {
            return Err("Kerf is probing or checking the machine. Try again in a moment.".into());
        }
        if options.atc {
            match self.firmware {
                Some(Firmware::GrblHal) => {}
                Some(Firmware::Grbl) => return Err("This job changes bits with an automatic tool changer (M6), which needs grblHAL. This machine runs Grbl, which refuses M6.".into()),
                None => {
                    let asked = self
                        .manual_queue
                        .iter()
                        .chain(&self.manual_in_flight)
                        .any(|l| l.eq_ignore_ascii_case(BUILD_INFO_QUERY));
                    if !asked {
                        self.manual_queue.push_back(BUILD_INFO_QUERY.to_owned());
                    }
                    return Err("Kerf is checking which firmware the machine runs. Try again in a moment.".into());
                }
            }
        }
        if options.needs_homing && !self.homed {
            return Err("This job goes to a fixed place on the machine to change bits, which needs the machine to have homed since Kerf connected. Home it, then start again.".into());
        }
        if !options.machine_moves.is_empty() {
            let travel = [
                self.setting(TRAVEL),
                self.setting(TRAVEL + 1),
                self.setting(TRAVEL + 2),
            ];
            let [Some(tx), Some(ty), Some(tz)] = travel else {
                return Err("Kerf has not read the machine's travel ($130 to $132) yet, so it cannot check the bit change place. Wait a moment, or send $$ from the console, and try again.".into());
            };
            for [x, y, z] in &options.machine_moves {
                // After homing, GRBL's machine coordinates run from 0 at the
                // home corner to minus the travel.
                let within = |v: f64, travel: f64| v <= 0.0 && v >= -travel;
                let inside = within(*x, tx) && within(*y, ty) && within(*z, tz);
                if !inside {
                    return Err(format!(
                        "The bit change place (X{x} Y{y} Z{z} from home) is outside the machine's travel ({tx} × {ty} × {tz} mm). Change it in the Device step and generate again."
                    ));
                }
            }
        }
        Ok(())
    }

    /// Kerf is waiting on the machine for a probe, a Continue or a Start.
    fn checking(&self) -> bool {
        self.probing.is_some()
            || self.probe_request.is_some()
            || self.continuing.is_some()
            || self.starting.is_some()
    }

    /// Why the work zero in use is not one Kerf can rely on, if it is not.
    /// Every part asserts G54, and the checks read G54, so it must be G54.
    fn check_on_g54(&self) -> Result<(), String> {
        match self.on_g54 {
            Some(true) => Ok(()),
            Some(false) => Err("The machine is using a work zero other than G54. Kerf sets and checks G54, so send G54 from the console, then set zero and try again.".into()),
            None => Err("Kerf has not yet read which work zero the machine is using. Send $G from the console, or wait a moment, and try again.".into()),
        }
    }

    /// The checks on the fresh offsets before a job with bit changes
    /// starts, then it starts.
    fn finish_start(&mut self, parts: Vec<Streamer>, options: JobOptions) -> Result<(), String> {
        self.check_can_start(&options)?;
        let offsets = self.offsets.clone().unwrap_or_default();
        if !offsets.only_g54() {
            return Err("A G92 offset or a tool length offset is set, so Kerf could not check the zero at the bit changes. Clear it (send G92.1 and G49 from the console), set zero, then start again.".into());
        }
        let Some(g54) = offsets.g54 else {
            return Err(NO_OFFSETS.into());
        };
        self.check_on_g54()?;
        self.check_lift(options.change_lift, Some(g54[2]))?;
        check_clearance(&options.machine_moves, options.change_clearance, g54[2])?;
        self.begin_job(parts, options)
            .map_err(|e| format!("Kerf could not start the job: {e}"))
    }

    /// Starts a job that has passed its checks.
    fn begin_job(&mut self, parts: Vec<Streamer>, options: JobOptions) -> std::io::Result<()> {
        self.stop_held(Some("Stopped moving: a job was started."))?;
        let of = parts.len();
        let mut parts = VecDeque::from(parts);
        self.waiting_job = parts.pop_front();
        if of > 1 {
            self.pending = Some(PendingParts {
                parts,
                part: 1,
                of,
                stage: PartStage::Running,
                finished: (0, 0),
                offsets_at_pause: None,
                z_set: false,
                plate_used: false,
                change_lift: options.change_lift,
                machine_moves: options.machine_moves,
                change_clearance: options.change_clearance,
            });
            self.emit_parts();
        }
        Ok(())
    }

    /// Whether the bit can rise `lift` above the work zero, whose machine Z
    /// is `zero_z`, without passing the top of Z. Only checked once the
    /// machine has homed and its Z travel is known; otherwise the person is
    /// asked to check by hand.
    fn check_lift(&self, lift: Option<f64>, zero_z: Option<f64>) -> Result<(), String> {
        let (Some(lift), Some(zero_z)) = (lift, zero_z) else {
            return Ok(());
        };
        if !self.homed || self.setting(TRAVEL + 2).is_none() {
            return Ok(());
        }
        // After homing, machine Z0 is the top of Z.
        if zero_z + lift > 1e-6 {
            let room = (-zero_z).max(0.0);
            return Err(format!(
                "The bit can't rise {} mm above the material here; there's only {} mm left. Lower \"Raise for changing bits\" to {} or less and generate again.",
                trim(lift),
                trim(room),
                room.floor()
            ));
        }
        Ok(())
    }

    /// Why the next part must not start now, before its fresh offsets read.
    fn check_can_continue(&self) -> Result<(), String> {
        let Some(pending) = &self.pending else {
            return Err("No job is waiting for a bit change.".into());
        };
        if pending.stage != PartStage::Waiting {
            return Err("The part has not finished yet.".into());
        }
        if self.checking() {
            return Err("Kerf is probing or checking the machine. Try again in a moment.".into());
        }
        if *lock(&self.state) != Some(MachineState::Idle) {
            return Err("The machine must be idle to carry on.".into());
        }
        self.check_machine_free()?;
        if !self.manual_in_flight.is_empty() {
            return Err(
                "The machine is still busy with earlier commands. Try again in a moment.".into(),
            );
        }
        if !pending.z_set {
            return Err(Z_NOT_SET.into());
        }
        Ok(())
    }

    /// The checks on the fresh offsets, then the next part is started.
    fn finish_continue(&mut self, plate_cleared: bool) -> Result<(), String> {
        self.check_can_continue()?;
        let offsets = self.offsets.clone().unwrap_or_default();
        let pending = self.pending.as_ref().expect("checked above");
        let pause = pending.offsets_at_pause.clone().unwrap_or_default();
        let (Some(now), Some(then)) = (offsets.g54, pause.g54) else {
            return Err("The machine did not report its work offsets, so Kerf cannot check the X/Y zero. Stop the job.".into());
        };
        let xy_kept = (now[0] - then[0]).abs() <= ZERO_TOLERANCE
            && (now[1] - then[1]).abs() <= ZERO_TOLERANCE;
        if !xy_kept || !offsets.only_g54() {
            return Err("The X/Y zero (or an offset) has changed since the job paused, so the next part would be cut in the wrong place. Stop the job.".into());
        }
        if pending.plate_used {
            if self.pins.contains('P') {
                return Err("The probe still reads as touching. Take the clip off the bit and move the plate away.".into());
            }
            if !plate_cleared {
                return Err(
                    "Take the clip off the bit and move the plate away, then tick the box.".into(),
                );
            }
        }
        if let Some(problem) = laser_mode_problem(&lock(&self.settings), Tool::Spindle) {
            return Err(problem);
        }
        self.check_on_g54()?;
        self.check_lift(pending.change_lift, Some(now[2]))?;
        check_clearance(&pending.machine_moves, pending.change_clearance, now[2])?;

        let pending = self.pending.as_mut().expect("checked above");
        pending.part += 1;
        pending.stage = PartStage::Running;
        pending.z_set = false;
        pending.plate_used = false;
        pending.offsets_at_pause = None;
        self.waiting_job = pending.parts.pop_front();
        self.emit_parts();
        Ok(())
    }

    /// Why probing must not start now, before its fresh offsets read.
    fn check_can_probe(&self) -> Result<(), String> {
        self.check_can_move()?;
        if *lock(&self.state) != Some(MachineState::Idle) {
            return Err("The machine must be idle to probe.".into());
        }
        if !self.manual_in_flight.is_empty() {
            return Err(
                "The machine is still busy with earlier commands. Try again in a moment.".into(),
            );
        }
        if !self.probe_tested {
            return Err("Test the probe first: touch the plate to the bit and check that the light comes on.".into());
        }
        self.check_on_g54()?;
        if self.pins.contains('P') {
            return Err("The probe already reads as touching. Check the clip and the plate are apart, then try again.".into());
        }
        Ok(())
    }

    /// Sends the first probing line, once the fresh offsets show nothing
    /// that would make the new Z wrong.
    fn start_probe(&mut self, settings: ProbeSettings) -> Result<(), String> {
        self.check_can_probe()?;
        let offsets = self.offsets.clone().unwrap_or_default();
        if !offsets.only_g54() {
            return Err("A G92 offset or a tool length offset is set, which would make the new Z wrong. Clear it (send G92.1 and G49 from the console), then try again.".into());
        }
        let Some(g54) = offsets.g54 else {
            return Err("The machine did not report its work offsets. Try again.".into());
        };
        // Work Z from what the last report itself said and the fresh G54,
        // never from a work offset the reports may not have repeated lately.
        // G92 and the tool length offset are zero, so the work zero is G54.
        let (work_z, machine_z) = match self.reported_z {
            Some(ReportedZ::Machine(z)) => (z - g54[2], z),
            Some(ReportedZ::Work(z)) => (z, z + g54[2]),
            None => {
                return Err(
                    "Kerf does not know where the bit is yet. Wait a moment and try again.".into(),
                )
            }
        };
        let work = [0.0, 0.0, work_z];
        let mut travel = settings.travel;
        // Never further than the bottom of Z, when that is known.
        if let (true, Some(z_travel)) = (self.homed, self.setting(TRAVEL + 2)) {
            travel = travel.min(machine_z + z_travel);
        }
        if travel < 0.5 {
            return Err("There is no room left below the bit to probe.".into());
        }
        let line = probe::fast_line(work[2], travel, settings.fast_feed);
        self.manual_queue.push_back(line.clone());
        self.probing = Some(ProbeRun {
            settings,
            stage: ProbeStage::Fast,
            line,
            g54_z: g54[2],
            touched: None,
            first_work_z: 0.0,
        });
        // The clip is on from now, whatever happens.
        if let Some(pending) = &mut self.pending {
            pending.plate_used = true;
        }
        self.emit(Event::Probe {
            state: ProbeState::Running,
            message: None,
        });
        self.emit_parts();
        Ok(())
    }

    /// The probing line in flight was answered: send the next, if the
    /// touches it needed were reported.
    fn probe_answered(&mut self) {
        let Some(run) = &mut self.probing else {
            return;
        };
        let ProbeSettings {
            plate, slow_feed, ..
        } = run.settings;
        let next = match run.stage {
            ProbeStage::Fast => run.touched.take().map(|machine_z| {
                run.first_work_z = machine_z - run.g54_z;
                run.stage = ProbeStage::BackOff;
                probe::back_off_line(run.first_work_z)
            }),
            ProbeStage::BackOff => {
                run.stage = ProbeStage::Slow;
                Some(probe::slow_line(run.first_work_z, slow_feed))
            }
            // Only now, after a reported touch, is Z set.
            ProbeStage::Slow => run.touched.take().map(|machine_z| {
                run.stage = ProbeStage::SetZ;
                run.g54_z = machine_z - plate;
                probe::set_z_line(machine_z, plate)
            }),
            ProbeStage::SetZ => {
                run.stage = ProbeStage::Lift;
                Some(probe::lift_line(plate))
            }
            ProbeStage::Lift => {
                run.stage = ProbeStage::Confirm {
                    ticket: self.reads_queued + 1,
                    g54_z: run.g54_z,
                };
                run.line.clear();
                self.queue_offsets_read();
                return;
            }
            ProbeStage::Confirm { .. } => return,
        };
        match next {
            Some(next) => {
                run.line = next.clone();
                self.manual_queue.push_back(next);
            }
            None => self.probe_failed(PROBE_FAILED),
        }
    }

    /// Ends probing without setting Z (or without confirming it), and says why.
    fn probe_failed(&mut self, message: &str) {
        if self.probing.take().is_some() {
            self.emit(Event::Probe {
                state: ProbeState::Failed,
                message: Some(message.to_owned()),
            });
        }
    }

    fn probe_passed(&mut self) {
        self.probe_watch = false;
        self.probe_tested = true;
        self.emit(Event::ProbeTested);
    }

    /// Queues a `$#` read and returns its ticket.
    fn queue_offsets_read(&mut self) -> u64 {
        self.manual_queue.push_back(OFFSETS_QUERY.to_owned());
        self.reads_queued += 1;
        self.reads_queued
    }

    /// After lines were dropped unsent or unanswered: counts the `$#` reads
    /// still to come, and refuses requests whose read is gone.
    fn recount_reads(&mut self) {
        let left = self
            .manual_queue
            .iter()
            .chain(&self.manual_in_flight)
            .filter(|l| is_offsets_query(l))
            .count() as u64;
        self.reads_queued = self.reads_done + left;
        let queued = self.reads_queued;
        if self.continuing.as_ref().is_some_and(|r| r.ticket > queued) {
            let request = self.continuing.take().expect("checked");
            let _ = request.reply.send(Err(INTERRUPTED.into()));
        }
        if self
            .probe_request
            .as_ref()
            .is_some_and(|r| r.ticket > queued)
        {
            let request = self.probe_request.take().expect("checked");
            let _ = request.reply.send(Err(INTERRUPTED.into()));
        }
        if self.starting.as_ref().is_some_and(|r| r.ticket > queued) {
            let request = self.starting.take().expect("checked");
            let _ = request.reply.send(Err(INTERRUPTED.into()));
        }
        if self.probing.as_ref().is_some_and(
            |r| matches!(r.stage, ProbeStage::Confirm { ticket, .. } if ticket > queued),
        ) {
            self.probe_failed(PROBE_FAILED);
        }
    }

    /// A `$#` read has been answered (or refused, if `ok` is false).
    fn finish_read(&mut self, ok: bool) {
        self.reads_done += 1;
        let reading = std::mem::take(&mut self.reading);
        if ok {
            self.offsets = Some(reading);
        }
        let done = self.reads_done;
        let offsets = self.offsets.clone().filter(|_| ok);
        let mut waiting_now = None;
        let mut read_again = false;
        if let Some(pending) = &mut self.pending {
            match pending.stage {
                PartStage::Reading(ticket) if done >= ticket => {
                    match offsets.filter(|o| o.g54.is_some()) {
                        Some(offsets) => {
                            pending.offsets_at_pause = Some(offsets);
                            pending.stage = PartStage::Waiting;
                            waiting_now = Some(pending.finished);
                        }
                        None => read_again = true,
                    }
                }
                PartStage::Waiting => {
                    // Z is set again once G54 Z differs from the pause.
                    let then = pending.offsets_at_pause.as_ref().and_then(|o| o.g54);
                    let now = offsets.and_then(|o| o.g54);
                    if let (Some(then), Some(now)) = (then, now) {
                        if (then[2] - now[2]).abs() > 1e-4 {
                            pending.z_set = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if read_again {
            let ticket = self.queue_offsets_read();
            if let Some(pending) = &mut self.pending {
                pending.stage = PartStage::Reading(ticket);
            }
        }
        if let Some((acknowledged, total)) = waiting_now {
            self.emit(Event::Job {
                state: JobState::WaitingForBit,
                acknowledged,
                total,
                message: None,
            });
        }
        if self.continuing.as_ref().is_some_and(|r| r.ticket <= done) {
            let request = self.continuing.take().expect("checked");
            let answer = if !ok {
                Err(NO_OFFSETS.into())
            } else if request.reply.late() {
                Err(LATE.into())
            } else {
                self.finish_continue(request.value)
            };
            let _ = request.reply.send(answer);
        }
        if self.starting.as_ref().is_some_and(|r| r.ticket <= done) {
            let request = self.starting.take().expect("checked");
            let (parts, options) = request.value;
            let answer = if !ok {
                Err(NO_OFFSETS.into())
            } else if request.reply.late() {
                Err(LATE.into())
            } else {
                self.finish_start(parts, options)
            };
            let _ = request.reply.send(answer);
        }
        if self
            .probe_request
            .as_ref()
            .is_some_and(|r| r.ticket <= done)
        {
            let request = self.probe_request.take().expect("checked");
            let answer = if !ok {
                Err(NO_OFFSETS.into())
            } else if request.reply.late() {
                Err(LATE.into())
            } else {
                self.start_probe(request.value)
            };
            let _ = request.reply.send(answer);
        }
        if let Some(ProbeRun {
            stage: ProbeStage::Confirm { ticket, g54_z },
            ..
        }) = self.probing
        {
            if done >= ticket {
                let set = self
                    .offsets
                    .as_ref()
                    .and_then(|o| o.g54)
                    .is_some_and(|g| ok && (g[2] - g54_z).abs() <= ZERO_TOLERANCE);
                if set {
                    self.probing = None;
                    if let Some(pending) = &mut self.pending {
                        pending.z_set = true;
                        pending.plate_used = true;
                    }
                    self.emit(Event::Probe {
                        state: ProbeState::Done,
                        message: None,
                    });
                } else {
                    self.probe_failed("Kerf could not confirm the new Z. Set Z again.");
                }
            }
        }
        self.emit_parts();
    }

    /// Fails requests that waited for a read when the deadline has passed.
    fn expire_requests(&mut self) {
        if self.continuing.as_ref().is_some_and(|r| r.reply.late()) {
            let request = self.continuing.take().expect("checked");
            let _ = request.reply.send(Err(NO_OFFSETS.into()));
        }
        if self.probe_request.as_ref().is_some_and(|r| r.reply.late()) {
            let request = self.probe_request.take().expect("checked");
            let _ = request.reply.send(Err(NO_OFFSETS.into()));
        }
        if self.starting.as_ref().is_some_and(|r| r.reply.late()) {
            let request = self.starting.take().expect("checked");
            let _ = request.reply.send(Err(NO_OFFSETS.into()));
        }
    }

    fn parts_progress(&self) -> Option<PartsProgress> {
        self.pending.as_ref().map(|p| PartsProgress {
            part: p.part,
            of: p.of,
            waiting: p.stage == PartStage::Waiting,
            spindle_off: self.spindle_mode == SpindleMode::Off && self.spindle == Some(0.0),
            z_set: p.z_set,
            plate_used: p.plate_used,
        })
    }

    /// Sends [`Event::Parts`] if it has changed since last sent.
    fn emit_parts(&mut self) {
        let now = self.parts_progress();
        if self.last_parts.as_ref() != Some(&now) {
            self.last_parts = Some(now.clone());
            self.emit(Event::Parts { progress: now });
        }
    }

    /// Whether the last report says the machine is, or may be, in motion or
    /// with its spindle or laser on.
    fn is_moving(&self) -> bool {
        !matches!(
            *lock(&self.state),
            None | Some(MachineState::Idle | MachineState::Alarm | MachineState::Sleep)
        )
    }

    /// Run when the connection is closed on purpose. Anything still running is
    /// reset, since once the port is closed nothing more can stop it.
    fn shut_down(&mut self) {
        let held = self.held.take().is_some();
        if held {
            let _ = self.port.write_all(&[JOG_CANCEL]);
        }
        if held || self.job.is_some() || self.reset_pending.is_some() || self.is_moving() {
            let _ = self.port.write_all(&[SOFT_RESET]);
            let _ = self.port.flush();
        }
        self.abandon(JobState::Stopped, None);
    }

    /// Forgets what a soft reset clears. The machine runs its start-up lines
    /// again, so whether the spindle is on is asked once it has restarted.
    fn after_reset(&mut self) {
        self.reset_pending = None;
        self.held = None;
        self.resync = None;
        self.spindle_mode = SpindleMode::Unknown;
        // A reset while moving loses the position, so homing counts no more.
        self.homed = false;
        self.seen_home = false;
    }

    /// Stops a held jog, if one is running, and says why when there is a
    /// reason worth telling.
    fn stop_held(&mut self, reason: Option<&str>) -> std::io::Result<()> {
        if self.held.take().is_some() {
            self.cancel_jog()?;
            if let Some(text) = reason {
                self.emit(Event::Line { text: text.into() });
            }
        }
        Ok(())
    }

    /// Sends a jog cancel, which empties GRBL's queued jog moves. Jog lines
    /// still unanswered may then be answered or not, so the replies are
    /// brought back in step before anything more is sent.
    fn cancel_jog(&mut self) -> std::io::Result<()> {
        self.port.write_all(&[JOG_CANCEL])?;
        if !self.manual_in_flight.is_empty() && self.resync.is_none() {
            self.resync = Some(Resync {
                stage: ResyncStage::Waiting,
                since: Instant::now(),
                replies_ahead: 0,
            });
        }
        Ok(())
    }

    /// Ends a resync: nothing is in flight any more. A jog line that was on
    /// its way when the first cancel went out may have arrived once GRBL was
    /// idle again, which GRBL takes as a fresh jog (it ignores a cancel
    /// outside a jog, and keeps what it has received), so it is cancelled
    /// once more before anything else is sent.
    fn finish_resync(&mut self) -> std::io::Result<()> {
        self.manual_in_flight.clear();
        self.resync = None;
        self.recount_reads();
        self.port.write_all(&[JOG_CANCEL])
    }

    /// Bytes of manual lines sent but not yet answered.
    fn in_flight_bytes(&self) -> usize {
        self.manual_in_flight.iter().map(|l| l.len() + 1).sum()
    }

    /// Writes whatever the controller currently has room for.
    fn pump(&mut self) -> std::io::Result<()> {
        if self.job.is_none()
            && self.waiting_job.is_some()
            && self.held.is_none()
            && self.resync.is_none()
            && self.manual_queue.is_empty()
            && self.manual_in_flight.is_empty()
        {
            self.job = self.waiting_job.take();
            // A job switches the spindle or laser on and off itself.
            self.spindle_mode = SpindleMode::On;
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

        if let Some(resync) = &mut self.resync {
            if resync.stage == ResyncStage::Waiting
                // The marker and its newline must fit.
                && self.manual_in_flight.iter().map(|l| l.len() + 1).sum::<usize>()
                    + RESYNC_MARKER.len()
                    < RX_BUFFER_BYTES
            {
                resync.replies_ahead = self
                    .manual_in_flight
                    .iter()
                    .filter(|l| is_modes_query(l))
                    .count();
                self.port.write_all(RESYNC_MARKER.as_bytes())?;
                self.port.write_all(b"\n")?;
                self.manual_in_flight.push_back(RESYNC_MARKER.to_owned());
                resync.stage = ResyncStage::Sent;
            }
            return Ok(());
        }

        if self.held.is_some() {
            let mut limit = false;
            if let Some(held) = &mut self.held {
                // Send while the lead allows, no more than two unanswered,
                // always leaving room for the marker that a cancel will need.
                let elapsed = held.started.elapsed().as_secs_f64();
                let segment = jog::held_segment(held.direction, held.feed, held.dt);
                let length = held.feed / 60.0 * held.dt;
                let mut in_flight: usize = self.manual_in_flight.iter().map(|l| l.len() + 1).sum();
                while f64::from(held.sent + 1) * held.dt - elapsed <= HELD_LEAD
                && self.manual_in_flight.len() < 2
                // Each with its newline.
                && in_flight + segment.len() + 1 + RESYNC_MARKER.len() < RX_BUFFER_BYTES
                {
                    let too_far = held
                        .max_distance
                        .is_some_and(|max| f64::from(held.sent + 1) * length > max);
                    if too_far || held.started.elapsed() >= HELD_MAX_TIME {
                        limit = true;
                        break;
                    }
                    self.port.write_all(segment.as_bytes())?;
                    self.port.write_all(b"\n")?;
                    self.manual_in_flight.push_back(segment.clone());
                    in_flight += segment.len() + 1;
                    held.sent += 1;
                }
            }
            if limit {
                self.stop_held(Some("Stopped moving: one press moves for at most 10 seconds, or the length of the axis. Press again to carry on."))?;
            }
            // Lines typed meanwhile wait until the hold has ended.
            return Ok(());
        }

        while let Some(line) = self.manual_queue.front() {
            let in_flight = self.in_flight_bytes();
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
                    let more_parts = self.pending.as_ref().is_some_and(|p| !p.parts.is_empty());
                    if job.is_finished() && more_parts {
                        // The part's last line, a dwell, is only answered
                        // once motion has stopped. It is over once a status
                        // report after this says the machine is idle.
                        let (acknowledged, total) = job.progress();
                        self.job = None;
                        if let Some(pending) = &mut self.pending {
                            pending.stage = PartStage::Ending;
                            pending.finished = (acknowledged, total);
                        }
                        self.emit(Event::Job {
                            state: JobState::Running,
                            acknowledged,
                            total,
                            message: None,
                        });
                        // The part ends with M5, but the machine is asked.
                        self.spindle_mode = SpindleMode::Unknown;
                        self.manual_queue.push_back("$G".to_owned());
                        self.emit_parts();
                    } else if job.is_finished() {
                        self.emit_progress(JobState::Finished, None);
                        self.job = None;
                        // A Kerf job ends with M5, but the machine is asked
                        // rather than assumed.
                        self.spindle_mode = SpindleMode::Unknown;
                        self.manual_queue.push_back("$G".to_owned());
                        if self.pending.take().is_some() {
                            self.emit_parts();
                        }
                    } else if self.last_progress.elapsed() >= PROGRESS_INTERVAL {
                        self.emit_progress(JobState::Running, None);
                    }
                } else if self
                    .resync
                    .as_ref()
                    .is_some_and(|r| r.stage == ResyncStage::Answered)
                {
                    // The marker's own `ok`: every line before it has been
                    // answered or dropped, so none is still in flight.
                    self.finish_resync()?;
                } else if let Some(line) = self.manual_in_flight.pop_front() {
                    // A changed setting, such as `$32=1`, is read back so the
                    // checks before a job see it.
                    if is_setting_change(&line) {
                        self.manual_queue.push_back("$$".to_owned());
                    }
                    self.manual_answered(&line);
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
                    if is_offsets_query(&line) {
                        self.finish_read(false);
                    }
                    if self.probing.as_ref().is_some_and(|r| r.line == line) {
                        self.probe_failed(PROBE_FAILED);
                    }
                    let jog = line.starts_with("$J=");
                    // A refused M5 may have left the spindle on: ask.
                    if spindle_words(&line).is_some() {
                        self.spindle_mode = SpindleMode::Unknown;
                        self.manual_queue.push_back("$G".to_owned());
                    }
                    // Once a hold has ended, its other segments may be refused
                    // too (at a soft limit, say); the first error is enough.
                    if !(jog && self.resync.is_some()) {
                        self.emit(Event::CommandError {
                            line,
                            code,
                            description: describe_error(code).to_owned(),
                        });
                    }
                    if jog {
                        self.stop_held(None)?;
                    }
                }
            }
            Response::Alarm(code) => {
                self.stop_held(None)?;
                let description = describe_alarm(code).to_owned();
                self.homed = false;
                self.seen_home = false;
                // Only Kerf's own probe going wrong keeps a job that waits
                // for a bit change: nothing has been set, and the person
                // can unlock and try again. Any other alarm ends the job.
                if self.probing.is_some() && matches!(code, 4 | 5) {
                    self.probe_failed(PROBE_FAILED);
                    self.manual_queue.clear();
                    self.manual_in_flight.clear();
                    self.recount_reads();
                } else {
                    self.abandon(
                        JobState::Failed,
                        Some(format!("Alarm {code}: {description}")),
                    );
                }
                self.emit(Event::Alarm { code, description });
            }
            Response::Status(status) => self.on_status(status)?,
            Response::Welcome(firmware, version) => {
                // The controller has restarted, so everything queued is gone.
                self.after_reset();
                self.abandon(JobState::Stopped, None);
                lock(&self.settings).clear();
                self.firmware = Some(firmware);
                self.emit(Event::Welcome { version, firmware });
                self.manual_queue.push_back("$$".to_owned());
                self.manual_queue.push_back("$G".to_owned());
            }
            Response::Offset(name, values) => {
                let xyz = (values.len() >= 3).then(|| [values[0], values[1], values[2]]);
                match name.as_str() {
                    "G54" => self.reading.g54 = xyz,
                    "G92" => self.reading.g92 = xyz,
                    "TLO" => self.reading.tlo = Some(values),
                    _ => {}
                }
                self.emit(Event::Line {
                    text: line.trim().to_owned(),
                });
            }
            Response::Probe { position, ok } => {
                // The `[PRB:...]` line in a `$#` reply is the last probe's,
                // not news; only one answering a probing line counts.
                let answering = self.probing.as_ref().is_some_and(|r| {
                    matches!(r.stage, ProbeStage::Fast | ProbeStage::Slow)
                        && self.manual_in_flight.front() == Some(&r.line)
                });
                if answering {
                    if ok {
                        if let Some(run) = &mut self.probing {
                            run.touched = Some(position[2]);
                        }
                    } else {
                        self.probe_failed(PROBE_FAILED);
                    }
                }
                self.emit(Event::Line {
                    text: line.trim().to_owned(),
                });
            }
            Response::Other(text) if text.starts_with("[FIRMWARE:") => {
                if text.to_ascii_lowercase().contains("grblhal") {
                    self.build_info_grblhal = true;
                }
                self.emit(Event::Line { text });
            }
            Response::Setting(number, value) => {
                lock(&self.settings).insert(number, value.clone());
                self.emit(Event::Setting { number, value });
            }
            Response::Other(text) if text.starts_with("[GC:") => {
                // The reply is for the first `$G` in flight. If an M3, M4 or M5
                // comes after it, the reply was made before that line ran,
                // and the mode set when the line was queued stands.
                let query = self
                    .manual_in_flight
                    .iter()
                    .position(|l| is_modes_query(l))
                    .map_or(0, |at| at + 1);
                let later_spindle_word = self
                    .manual_in_flight
                    .iter()
                    .skip(query)
                    .chain(&self.manual_queue)
                    .any(|l| spindle_words(l).is_some());
                if !later_spindle_word {
                    self.spindle_mode = modes_spindle(&text);
                }
                let later_work_zero = self
                    .manual_in_flight
                    .iter()
                    .skip(query)
                    .chain(&self.manual_queue)
                    .any(|l| selects_work_zero(l));
                if !later_work_zero {
                    self.on_g54 = modes_on_g54(&text);
                }
                self.emit_parts();
                let mut marker = false;
                if let Some(resync) = &mut self.resync {
                    if resync.stage == ResyncStage::Sent {
                        if resync.replies_ahead > 0 {
                            // The reply to a `$G` typed before the marker.
                            resync.replies_ahead -= 1;
                        } else {
                            resync.stage = ResyncStage::Answered;
                            marker = true;
                        }
                    }
                }
                if !marker {
                    self.emit(Event::Line { text });
                }
            }
            Response::Other(text) => self.emit(Event::Line { text }),
        }
        Ok(())
    }

    /// A manual line was answered with `ok`.
    fn manual_answered(&mut self, line: &str) {
        if is_offsets_query(line) {
            self.finish_read(true);
        }
        if line.eq_ignore_ascii_case(BUILD_INFO_QUERY) {
            let firmware = if self.build_info_grblhal || self.firmware == Some(Firmware::GrblHal) {
                Firmware::GrblHal
            } else {
                Firmware::Grbl
            };
            self.build_info_grblhal = false;
            self.firmware = Some(firmware);
            self.emit(Event::Firmware { firmware });
        }
        if self.probing.as_ref().is_some_and(|r| r.line == line) {
            self.probe_answered();
        } else if self.probing.is_none()
            && self
                .pending
                .as_ref()
                .is_some_and(|p| p.stage == PartStage::Waiting)
            && changes_offsets(line)
        {
            // Read again, to see whether Z has been set or X/Y moved.
            self.queue_offsets_read();
        }
    }

    fn on_status(&mut self, status: Status) -> std::io::Result<()> {
        if status.work_offset.is_some() {
            self.work_offset = status.work_offset;
        }
        // Until an offset is reported it is taken as zero for the work
        // position, as before; the machine position waits for a real one.
        let offset = self.work_offset.unwrap_or([0.0; 3]);
        let position = status.work_position.or_else(|| {
            status
                .machine_position
                .map(|m| [0, 1, 2].map(|i| m[i] - offset[i]))
        });
        let machine_position = status.machine_position.or_else(|| {
            let (w, o) = (status.work_position?, self.work_offset?);
            Some([0, 1, 2].map(|i| w[i] + o[i]))
        });
        *lock(&self.state) = Some(status.state);
        self.spindle = status.spindle;
        self.position = position;
        self.machine_position = machine_position;
        self.reported_z = match (status.machine_position, status.work_position) {
            (Some(m), _) => Some(ReportedZ::Machine(m[2])),
            (None, Some(w)) => Some(ReportedZ::Work(w[2])),
            _ => None,
        };
        self.pins = status.pins.clone();
        self.emit(Event::Status {
            state: status.state,
            position,
            machine_position,
            feed: status.feed,
            spindle: status.spindle,
            pins: status.pins,
        });
        match status.state {
            MachineState::Home => self.seen_home = true,
            MachineState::Idle if self.seen_home => {
                self.seen_home = false;
                self.homed = true;
            }
            _ => {}
        }
        if self.probe_watch && self.pins.contains('P') {
            self.probe_passed();
        }
        // The first status after a part's last `ok` that says idle: the
        // part is over, and the offsets are read before the job waits.
        if status.state == MachineState::Idle
            && self
                .pending
                .as_ref()
                .is_some_and(|p| p.stage == PartStage::Ending)
        {
            let ticket = self.queue_offsets_read();
            if let Some(pending) = &mut self.pending {
                pending.stage = PartStage::Reading(ticket);
            }
        }
        self.emit_parts();
        // `Hold:1` means still slowing down; only `Hold:0` is at rest.
        let at_rest = match status.state {
            MachineState::Idle => true,
            MachineState::Hold => status.sub_state == Some(0),
            _ => false,
        };
        if self.reset_pending.is_some() && at_rest {
            self.finish_stop()?;
        }
        if self.held.is_some() {
            if !matches!(status.state, MachineState::Idle | MachineState::Jog) {
                self.stop_held(Some(&format!(
                    "Stopped moving: the machine reported {:?}.",
                    status.state
                )))?;
            } else if status.spindle != Some(0.0) {
                self.stop_held(Some(
                    "Stopped moving: the laser or spindle is no longer reported as off.",
                ))?;
            }
        }
        // A marker that is never answered would block everything after it.
        if status.state == MachineState::Idle
            && self
                .resync
                .as_ref()
                .is_some_and(|r| r.since.elapsed() >= RESYNC_TIMEOUT)
        {
            self.emit(Event::Line {
                text: "The machine did not answer after a move was stopped; Kerf has carried on without waiting.".into(),
            });
            self.finish_resync()?;
        }
        Ok(())
    }

    fn finish_stop(&mut self) -> std::io::Result<()> {
        self.port.write_all(&[SOFT_RESET])?;
        self.after_reset();
        self.abandon(JobState::Stopped, None);
        Ok(())
    }

    /// Forgets all queued work, reporting the end of a job if one was active.
    /// A pending reset is kept.
    fn abandon(&mut self, state: JobState, message: Option<String>) {
        if self.job.is_some() || self.waiting_job.is_some() || self.pending.is_some() {
            self.emit_progress(state, message);
        }
        self.job = None;
        self.waiting_job = None;
        // The parts still to run go too: Start needs a new job.
        self.pending = None;
        self.probe_failed("Probing was stopped before Z was set.");
        self.probe_watch = false;
        self.manual_queue.clear();
        self.manual_in_flight.clear();
        self.reading = Offsets::default();
        self.recount_reads();
        self.emit_parts();
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

/// Whether `line` switches the spindle or laser on (`Some(true)`, M3 or M4)
/// or off (`Some(false)`, M5, or M2/M30 at a program's end), going by the
/// last such word on it. `None` if it has none. `$` lines are left alone:
/// a jog cannot carry M words, and `$N0=M3` only stores a start-up line.
fn spindle_words(line: &str) -> Option<bool> {
    if line.trim_start().starts_with('$') {
        return None;
    }
    let mut code = String::new();
    let mut in_comment = false;
    for c in line.chars() {
        match c {
            '(' => in_comment = true,
            ')' => in_comment = false,
            ';' if !in_comment => break,
            c if !in_comment && !c.is_whitespace() => code.push(c.to_ascii_uppercase()),
            _ => {}
        }
    }
    let mut found = None;
    let mut rest = code.as_str();
    while let Some(at) = rest.find('M') {
        rest = &rest[at + 1..];
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        match digits.parse::<f64>() {
            Ok(n) if n == 3.0 || n == 4.0 => found = Some(true),
            Ok(n) if n == 5.0 || n == 2.0 || n == 30.0 => found = Some(false),
            _ => {}
        }
    }
    found
}

/// The spindle state in a `$G` reply, such as `[GC:G0 G54 ... M5 M9 ...]`.
fn modes_spindle(report: &str) -> SpindleMode {
    let words = report
        .trim_start_matches("[GC:")
        .trim_end_matches(']')
        .split_whitespace();
    let mut mode = SpindleMode::Unknown;
    for word in words {
        match word {
            "M3" | "M4" => mode = SpindleMode::On,
            "M5" => mode = SpindleMode::Off,
            _ => {}
        }
    }
    mode
}

/// A `$G` line, which GRBL answers with `[GC:...]`.
fn is_modes_query(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case(RESYNC_MARKER)
}

/// Why a change at a fixed place would take the bit too close to the work,
/// whose zero is at machine Z `zero_z`: each place's Z must be at least
/// `clearance` above it.
fn check_clearance(moves: &[[f64; 3]], clearance: Option<f64>, zero_z: f64) -> Result<(), String> {
    let Some(clearance) = clearance else {
        return Ok(());
    };
    for [_, _, z] in moves {
        if *z < zero_z + clearance - 1e-6 {
            return Err(format!(
                "The place to change bits (Z{} from home) is only {} mm above the material here; it must be at least {} mm above it, or the bit could hit the work on the way. Set a higher place in the Device step and generate again.",
                trim(*z),
                trim(z - zero_z),
                trim(clearance)
            ));
        }
    }
    Ok(())
}

/// Whether a `$G` report says G54 is in use: `Some(false)` for G55 to G59.
fn modes_on_g54(report: &str) -> Option<bool> {
    let words = report
        .trim_start_matches("[GC:")
        .trim_end_matches(']')
        .split_whitespace();
    let mut found = None;
    for word in words {
        match word {
            "G54" => found = Some(true),
            "G55" | "G56" | "G57" | "G58" | "G59" | "G59.1" | "G59.2" | "G59.3" => {
                found = Some(false)
            }
            _ => {}
        }
    }
    found
}

/// Whether a typed line may change the coordinate system in use (G54 to G59).
fn selects_work_zero(line: &str) -> bool {
    if line.trim_start().starts_with('$') {
        return false;
    }
    let code: String = line
        .split(';')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let mut rest = code.as_str();
    while let Some(at) = rest.find('G') {
        rest = &rest[at + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.trim_start_matches('0').parse::<u32>() {
            if (54..=59).contains(&n) {
                return true;
            }
        }
    }
    false
}

/// A `$#` line, which GRBL answers with the work offsets.
fn is_offsets_query(line: &str) -> bool {
    line.trim() == OFFSETS_QUERY
}

/// Whether a line may change a work offset: `G10` or `G92` (not `G100`).
fn changes_offsets(line: &str) -> bool {
    if line.trim_start().starts_with('$') {
        return false;
    }
    let code: String = line
        .split(';')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let mut rest = code.as_str();
    while let Some(at) = rest.find('G') {
        rest = &rest[at + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if matches!(digits.trim_start_matches('0'), "10" | "92") {
            return true;
        }
    }
    false
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
        /// The machine is homing; only a soft reset stops it.
        homing: bool,
        /// The spindle or laser speed reported in `FS:`.
        spindle: u32,
        /// Report `WPos` instead of `MPos`, as GRBL does with `$10=0`.
        report_work_position: bool,
        /// Leave out the acceleration settings (`$120` to `$122`) from `$$`.
        no_acceleration: bool,
        /// When the jog lines run so far will have finished moving.
        jog_ends: Option<Instant>,
        /// Hold lines in the buffer without running them, as GRBL does when
        /// its planner is full. A jog cancel clears it.
        stall: bool,
        /// Answer the jog lines a jog cancel throws away with `ok`. GRBL's
        /// guide does not say whether it does, so both are tested.
        answer_purged: bool,
        /// Answer every jog line with this `error:` code.
        jog_error: Option<u32>,
        /// Lines and real-time bytes other than `?`, in the order received.
        wire: Vec<String>,
        /// When each jog line arrived.
        jog_arrivals: Vec<Instant>,
        /// Each line and real-time byte other than `?` as it arrived, with
        /// how many lines had been run by then.
        arrivals: Vec<(String, usize)>,
        /// Whether M3 or M4 is in force, as `$G` reports it.
        spindle_on: bool,
        /// Behave as GRBL does on a jog cancel: ignore it unless jogging, and
        /// keep whatever is in the receive buffer.
        keeps_receive_buffer: bool,
        /// Jogs cut short by a jog cancel.
        jogs_cancelled: u32,
        /// Never answer `$G` at all.
        mute_modes_query: bool,
        /// Answer `$G` with `ok` alone, leaving out its `[GC:...]` report.
        modes_query_without_report: bool,
        /// Leave `FS:` out of status reports.
        no_spindle_report: bool,
        /// More `$n=value` lines for `$$` to report.
        extra_settings: Vec<(u32, &'static str)>,
        /// Panic on the next read, as a bug in the worker might.
        panic_on_read: bool,
        /// Make the next read take this long, as a busy computer might.
        slow_read: Option<Duration>,
        /// Behave as grblHAL: its banner after a reset, and `$I` says so.
        hal: bool,
        /// Where the machine is, and its offsets. `None` takes the usual
        /// test values: machine 5, 6, 7 and G54 1, 1, 1.
        mpos: Option<[f64; 3]>,
        g54: Option<[f64; 3]>,
        g92: [f64; 3],
        tlo: f64,
        /// The probe pin reads as touching.
        probe_touching: bool,
        /// Machine Z of the top of a touch plate under the bit, if one is there.
        plate_at: Option<f64>,
        /// A failed probe reports `[PRB:...:0]` instead of raising an alarm.
        probe_misses_quietly: bool,
        /// Status reports to say `Run` after each dwell, as when the planner
        /// is slow to empty.
        run_after_dwell: u32,
        busy_reports: u32,
        /// Report the `Tool` state, as grblHAL does at a tool change.
        tool_state: bool,
        /// The coordinate system `$G` reports, if not G54.
        work_zero: Option<&'static str>,
        /// A `WCO` for status reports to give in place of the real one, as
        /// when GRBL has not repeated it since an offset changed.
        stale_wco: Option<[f64; 3]>,
    }

    impl Sim {
        fn mpos(&self) -> [f64; 3] {
            self.mpos.unwrap_or([5.0, 6.0, 7.0])
        }

        fn g54(&self) -> [f64; 3] {
            self.g54.unwrap_or([1.0, 1.0, 1.0])
        }

        /// The work coordinate offset: G54, G92 and the tool length offset.
        fn wco(&self) -> [f64; 3] {
            let g54 = self.g54();
            [
                g54[0] + self.g92[0],
                g54[1] + self.g92[1],
                g54[2] + self.g92[2] + self.tlo,
            ]
        }

        /// Runs the parts of a line that move the simulated machine or set
        /// its offsets. Returns false if the line raised an alarm instead of
        /// being answered.
        fn run_motion(&mut self, line: &str) -> bool {
            let words = |line: &str| -> Vec<(char, f64)> {
                let mut found = Vec::new();
                let mut chars = line.chars().peekable();
                while let Some(c) = chars.next() {
                    if c.is_ascii_alphabetic() {
                        let mut number = String::new();
                        while let Some(&d) = chars.peek() {
                            if d.is_ascii_digit() || d == '.' || d == '-' {
                                number.push(d);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        if let Ok(value) = number.parse() {
                            found.push((c.to_ascii_uppercase(), value));
                        }
                    }
                }
                found
            };
            let found = words(line);
            let word = |letter: char| found.iter().find(|(l, _)| *l == letter).map(|(_, v)| *v);
            let has_g = |n: f64| {
                found
                    .iter()
                    .any(|(l, v)| *l == 'G' && (*v - n).abs() < 1e-9)
            };
            if line.starts_with('$') {
                return true;
            }
            if has_g(10.0) {
                let mut g54 = self.g54();
                let mpos = self.mpos();
                let l20 = word('L') == Some(20.0);
                for (axis, letter) in ['X', 'Y', 'Z'].into_iter().enumerate() {
                    if let Some(value) = word(letter) {
                        g54[axis] = if l20 {
                            let extra = self.g92[axis] + if axis == 2 { self.tlo } else { 0.0 };
                            mpos[axis] - extra - value
                        } else {
                            value
                        };
                    }
                }
                self.g54 = Some(g54);
                return true;
            }
            if has_g(92.0) {
                let mpos = self.mpos();
                let g54 = self.g54();
                for (axis, letter) in ['X', 'Y', 'Z'].into_iter().enumerate() {
                    if let Some(value) = word(letter) {
                        self.g92[axis] = mpos[axis] - g54[axis] - value;
                    }
                }
                return true;
            }
            if let (true, Some(target)) = (has_g(38.2), word('Z')) {
                let mut mpos = self.mpos();
                if self.probe_touching {
                    self.alarm = true;
                    self.say("ALARM:4");
                    return false;
                }
                let target = target + self.wco()[2];
                match self.plate_at {
                    Some(plate) if plate <= mpos[2] && plate >= target => {
                        mpos[2] = plate;
                        self.mpos = Some(mpos);
                        self.say(&format!(
                            "[PRB:{:.3},{:.3},{:.3}:1]",
                            mpos[0], mpos[1], mpos[2]
                        ));
                    }
                    _ => {
                        mpos[2] = target;
                        self.mpos = Some(mpos);
                        if self.probe_misses_quietly {
                            self.say(&format!(
                                "[PRB:{:.3},{:.3},{:.3}:0]",
                                mpos[0], mpos[1], mpos[2]
                            ));
                        } else {
                            self.alarm = true;
                            self.say("ALARM:5");
                            return false;
                        }
                    }
                }
                return true;
            }
            if has_g(4.0) {
                self.busy_reports = self.run_after_dwell;
            }
            if (has_g(0.0) || has_g(1.0)) && !has_g(53.0) {
                if let Some(z) = word('Z') {
                    let mut mpos = self.mpos();
                    mpos[2] = z + self.wco()[2];
                    self.mpos = Some(mpos);
                }
            }
            true
        }
    }

    /// How long the Sim takes over each jog line it runs.
    const SIM_JOG_LINE_TIME: Duration = Duration::from_millis(20);

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
                        } else if sim.homing {
                            "Home"
                        } else if sim.tool_state {
                            "Tool"
                        } else if sim.busy_reports > 0 {
                            sim.busy_reports -= 1;
                            "Run"
                        } else if sim.held && sim.settling > 0 {
                            sim.settling -= 1;
                            "Hold:1"
                        } else if sim.held {
                            "Hold:0"
                        } else if sim.jog_ends.is_some_and(|end| end > Instant::now())
                            || sim.buffered.iter().any(|l| l.starts_with("$J="))
                        {
                            "Jog"
                        } else if sim.buffered.is_empty() {
                            "Idle"
                        } else {
                            "Run"
                        };
                        let spindle = if sim.no_spindle_report {
                            String::new()
                        } else {
                            format!("|FS:0,{}", sim.spindle)
                        };
                        let (mpos, wco) = (sim.mpos(), sim.wco());
                        let position = if sim.report_work_position {
                            format!(
                                "WPos:{:.3},{:.3},{:.3}",
                                mpos[0] - wco[0],
                                mpos[1] - wco[1],
                                mpos[2] - wco[2]
                            )
                        } else {
                            format!("MPos:{:.3},{:.3},{:.3}", mpos[0], mpos[1], mpos[2])
                        };
                        let pins = if sim.probe_touching { "|Pn:P" } else { "" };
                        let shown = sim.stale_wco.unwrap_or(wco);
                        sim.say(&format!(
                            "<{state}|{position}{spindle}{pins}|WCO:{:.3},{:.3},{:.3}>",
                            shown[0], shown[1], shown[2]
                        ));
                        sim.realtime.push(byte);
                    }
                    FEED_HOLD => {
                        if !sim.held {
                            sim.settling = sim.settle_reports;
                        }
                        sim.held = true;
                        sim.realtime.push(byte);
                        sim.wire.push("<feed hold>".into());
                    }
                    CYCLE_START => {
                        sim.held = false;
                        sim.realtime.push(byte);
                    }
                    JOG_CANCEL if sim.keeps_receive_buffer => {
                        // Stops the jog in motion, if any; lines still in the
                        // receive buffer stay there and run afterwards.
                        if sim.jog_ends.is_some_and(|end| end > Instant::now()) {
                            sim.jogs_cancelled += 1;
                        }
                        sim.jog_ends = None;
                        sim.stall = false;
                        sim.realtime.push(byte);
                        sim.wire.push("<jog cancel>".into());
                        let run = sim.executed.len();
                        sim.arrivals.push(("<jog cancel>".into(), run));
                    }
                    JOG_CANCEL => {
                        // Throws away the jog lines not yet run, and stops.
                        if sim.jog_ends.is_some_and(|end| end > Instant::now()) {
                            sim.jogs_cancelled += 1;
                        }
                        let run = sim.executed.len();
                        sim.arrivals.push(("<jog cancel>".into(), run));
                        let purged = sim.buffered.iter().filter(|l| l.starts_with("$J=")).count();
                        sim.buffered.retain(|l| !l.starts_with("$J="));
                        if sim.answer_purged {
                            for _ in 0..purged {
                                sim.say("ok");
                            }
                        }
                        sim.jog_ends = None;
                        sim.stall = false;
                        sim.realtime.push(byte);
                        sim.wire.push("<jog cancel>".into());
                    }
                    SOFT_RESET => {
                        sim.wire.push("<reset>".into());
                        sim.jog_ends = None;
                        sim.spindle_on = false;
                        sim.reset_while_moving |= sim.held && sim.settling > 0;
                        sim.settling = 0;
                        sim.buffered.clear();
                        sim.partial.clear();
                        sim.held = false;
                        sim.realtime.push(byte);
                        if std::mem::take(&mut sim.homing) {
                            // As GRBL does: homing aborted, and locked.
                            sim.alarm = true;
                            sim.say("ALARM:6");
                        }
                        sim.busy_reports = 0;
                        sim.tool_state = false;
                        if sim.hal {
                            sim.say("GrblHAL 1.1f ['$' or '$HELP' for help]");
                        } else {
                            sim.say("Grbl 1.1h ['$' for help]");
                        }
                    }
                    b'\n' => {
                        let line = std::mem::take(&mut sim.partial);
                        if line.starts_with("$J=") {
                            sim.jog_arrivals.push(Instant::now());
                        }
                        sim.wire.push(line.clone());
                        let run = sim.executed.len();
                        sim.arrivals.push((line.clone(), run));
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
            let slow = self.0.lock().unwrap().slow_read.take();
            if let Some(time) = slow {
                std::thread::sleep(time);
            }
            if std::mem::take(&mut self.0.lock().unwrap().panic_on_read) {
                // Outside the lock, so the Sim can still record what follows.
                panic!("a simulated fault in the worker");
            }
            {
                let mut sim = self.0.lock().unwrap();
                // Execute one buffered line per read, like a busy controller.
                if !sim.held && !sim.stall {
                    if let Some(line) = sim.buffered.pop_front() {
                        let jog = line.starts_with("$J=");
                        if line == "$G" && sim.mute_modes_query {
                            // No reply at all.
                        } else if sim.reject.as_deref() == Some(line.as_str()) {
                            sim.say("error:20");
                        } else if let (true, Some(code)) = (jog, sim.jog_error) {
                            sim.say(&format!("error:{code}"));
                        } else {
                            if jog {
                                let from =
                                    sim.jog_ends.unwrap_or(Instant::now()).max(Instant::now());
                                sim.jog_ends = Some(from + SIM_JOG_LINE_TIME);
                            }
                            if line == "$$" {
                                sim.say("$130=750.000");
                                if !sim.no_acceleration {
                                    sim.say("$120=500.000");
                                    sim.say("$121=500.000");
                                    sim.say("$122=100.000");
                                }
                                if let Some(mode) = sim.laser_mode {
                                    sim.say(&format!("$32={mode}"));
                                }
                                for (number, value) in sim.extra_settings.clone() {
                                    sim.say(&format!("${number}={value}"));
                                }
                            }
                            if let Some(on) = spindle_words(&line) {
                                sim.spindle_on = on;
                            }
                            if line == "$G" && !sim.modes_query_without_report {
                                let mode = if sim.spindle_on { "M3" } else { "M5" };
                                let zero = sim.work_zero.unwrap_or("G54");
                                sim.say(&format!(
                                    "[GC:G0 {zero} G17 G21 G90 G94 {mode} M9 T0 F0 S0]"
                                ));
                            }
                            if let Some(mode) = line.strip_prefix("$32=") {
                                sim.laser_mode = Some(if mode == "1" { "1" } else { "0" });
                            }
                            if line == "$#" {
                                let (g54, g92, tlo) = (sim.g54(), sim.g92, sim.tlo);
                                sim.say(&format!(
                                    "[G54:{:.3},{:.3},{:.3}]",
                                    g54[0], g54[1], g54[2]
                                ));
                                sim.say("[G55:0.000,0.000,0.000]");
                                sim.say("[G28:0.000,0.000,0.000]");
                                sim.say(&format!(
                                    "[G92:{:.3},{:.3},{:.3}]",
                                    g92[0], g92[1], g92[2]
                                ));
                                sim.say(&format!("[TLO:{tlo:.3}]"));
                                sim.say("[PRB:0.000,0.000,0.000:0]");
                            }
                            if line == "$I" {
                                sim.say("[VER:1.1h.20190825:]");
                                sim.say("[OPT:V,15,128]");
                                if sim.hal {
                                    sim.say("[FIRMWARE:grblHAL]");
                                }
                            }
                            if sim.run_motion(&line) {
                                sim.say("ok");
                            }
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
    fn status_reports_machine_position() {
        for report_work_position in [false, true] {
            let h = connect(|sim| sim.report_work_position = report_work_position);
            let positions = h.wait_for(|e| match e {
                Event::Status {
                    position,
                    machine_position,
                    ..
                } => Some((*position, *machine_position)),
                _ => None,
            });
            assert_eq!(
                positions,
                (Some([4.0, 5.0, 6.0]), Some([5.0, 6.0, 7.0])),
                "reporting WPos: {report_work_position}"
            );
        }
    }

    #[test]
    fn streams_a_job_to_completion_without_overflow() {
        let h = connect(|_| {});
        h.wait_for_state(MachineState::Idle);
        let text = program(400);
        h.connection.start_job(&text, Tool::Laser).unwrap();

        let (state, acknowledged, total, _) = h.wait_for_job_end();
        assert_eq!((state, acknowledged, total), (JobState::Finished, 400, 400));
        // The `$G` after the job is sent just after it is reported finished.
        wait_until_executed(&h, 403);
        let sim = h.sim.lock().unwrap();
        assert!(!sim.overflowed);
        // The settings query sent on connecting runs first.
        assert_eq!(sim.executed[..2], ["$$", "$G"]);
        assert_eq!(sim.executed[2..402], text.lines().collect::<Vec<_>>());
        // Afterwards Kerf asks whether the job left the spindle on.
        assert_eq!(sim.executed[402..], ["$G"]);
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
        // Only the queries sent after the restart ran; no more of the job.
        let after: Vec<_> = sim.executed[executed_at_stop..].iter().collect();
        assert!(
            after.iter().all(|line| *line == "$$" || *line == "$G"),
            "{after:?}"
        );
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
        h.wait_for(|e| {
            matches!(e, Event::Welcome { version, .. } if version == "1.1h").then_some(())
        });
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
    fn stop_while_homing_resets_immediately() {
        // A long settle would hold up a reset that waited for `Hold:0`.
        let h = connect(|sim| {
            sim.homing = true;
            sim.settle_reports = 1_000;
        });
        h.wait_for_state(MachineState::Home);
        let asked = Instant::now();
        h.connection.stop_job().unwrap();
        let code = h.wait_for(|e| match e {
            Event::Alarm { code, .. } => Some(*code),
            _ => None,
        });
        assert_eq!(code, 6);
        assert!(asked.elapsed() < STOP_TIMEOUT / 3, "{:?}", asked.elapsed());
        let sim = h.sim.lock().unwrap();
        // A soft reset only: GRBL ignores a feed hold while homing.
        assert_eq!(
            sim.realtime
                .iter()
                .filter(|&&b| b != STATUS_QUERY)
                .collect::<Vec<_>>(),
            [&SOFT_RESET]
        );
    }

    /// Waits until the machine has run every line it was sent.
    fn wait_until_executed(h: &Harness, count: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let sim = h.sim.lock().unwrap();
                if sim.executed.len() >= count {
                    return sim.executed.clone();
                }
            }
            assert!(Instant::now() < deadline, "timed out waiting for lines");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn jogs_go_to_zero_and_framing_send_exactly_these_lines() {
        let h = connect_ready(|_| {});
        when_free(|| h.connection.jog(0.0, 1.0, 0.0, 1000.0)).unwrap();
        when_free(|| h.connection.go_to_xy_zero(3000.0)).unwrap();
        when_free(|| h.connection.frame(0.0, 0.0, 20.0, 10.0, 3000.0)).unwrap();
        let executed = wait_until_executed(&h, 9);
        assert_eq!(
            executed[2..],
            [
                "$J=G91 G21 Y1.000 F1000",
                "$J=G90 G21 X0.000 Y0.000 F3000",
                "$J=G90 G21 X0.000 Y0.000 F3000",
                "$J=G90 G21 X20.000 Y0.000 F3000",
                "$J=G90 G21 X20.000 Y10.000 F3000",
                "$J=G90 G21 X0.000 Y10.000 F3000",
                "$J=G90 G21 X0.000 Y0.000 F3000",
            ]
        );
    }

    #[test]
    fn moves_are_refused_while_the_laser_or_spindle_is_on() {
        let h = connect_ready(|sim| sim.spindle = 100);
        let refusals = [
            h.connection.jog(1.0, 0.0, 0.0, 1000.0),
            h.connection.go_to_xy_zero(1000.0),
            h.connection.frame(0.0, 0.0, 1.0, 1.0, 1000.0),
        ];
        for refusal in refusals {
            let message = refusal.unwrap_err();
            assert!(
                message.contains("laser or spindle is on (S100)"),
                "{message}"
            );
            assert!(message.contains("Send M5"), "{message}");
        }
        // A typed line is the person's own, and is not checked.
        h.connection.send_line("M5").unwrap();
        h.sim.lock().unwrap().spindle = 0;
        h.wait_for(|e| {
            matches!(e, Event::Status { spindle: Some(s), .. } if *s == 0.0).then_some(())
        });
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
        let executed = wait_until_executed(&h, 4);
        assert_eq!(executed[2..], ["M5", "$J=G91 G21 X1.000 F1000"]);
    }

    #[test]
    fn moves_need_a_real_speed() {
        let h = connect(|_| {});
        assert!(h.connection.jog(f64::NAN, 0.0, 0.0, 1000.0).is_err());
        assert!(h.connection.jog(1.0, 0.0, 0.0, 0.0).is_err());
        assert!(h.connection.go_to_xy_zero(f64::INFINITY).is_err());
        assert!(h.connection.frame(0.0, 0.0, 1.0, 1.0, -5.0).is_err());
    }

    /// 1000 mm/min at 500 mm/s² makes 0.05 s segments of 0.833 mm.
    const HELD_FEED: f64 = 1000.0;
    const SEGMENT: &str = "$J=G91 G21 X0.833 F1000";

    /// Connects and waits until a held jog could start: settings read, the
    /// spindle's state read with `$G`, and the machine idle with a spindle
    /// speed reported.
    fn connect_for_holding(setup: impl FnOnce(&mut Sim)) -> Harness {
        let h = connect(setup);
        h.wait_for(|e| matches!(e, Event::Setting { number: 122, .. }).then_some(()));
        h.wait_for(|e| matches!(e, Event::Line { text } if text.starts_with("[GC:")).then_some(()));
        h.wait_for_state(MachineState::Idle);
        h
    }

    /// Connects and waits for the settings and the spindle's state.
    fn connect_ready(setup: impl FnOnce(&mut Sim)) -> Harness {
        connect_for_holding(setup)
    }

    /// Tries `action` until it is not refused for being busy: a move is
    /// refused while earlier lines still wait to be sent.
    fn when_free(mut action: impl FnMut() -> Result<(), String>) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match action() {
                Err(e) if e.contains("still busy") && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                result => return result,
            }
        }
    }

    impl Harness {
        fn wire(&self) -> Vec<String> {
            self.sim.lock().unwrap().wire.clone()
        }

        /// Says the jog is still held, every 50 ms, for `time`.
        fn keep_alive_for(&self, time: Duration) {
            let until = Instant::now() + time;
            while Instant::now() < until {
                self.connection.keep_held_jog().unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        /// Waits for the Sim to receive a jog cancel.
        fn wait_for_jog_cancel(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self.sim.lock().unwrap().realtime.contains(&JOG_CANCEL) {
                assert!(Instant::now() < deadline, "no jog cancel was sent");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    /// What reached the machine after the last jog cancel.
    fn after_cancel(wire: &[String]) -> &[String] {
        let at = wire
            .iter()
            .rposition(|l| l == "<jog cancel>")
            .expect("a jog cancel");
        &wire[at + 1..]
    }

    fn jog_lines(wire: &[String]) -> Vec<&String> {
        wire.iter().filter(|l| l.starts_with("$J=")).collect()
    }

    #[test]
    fn held_jog_sends_segments_only_while_kept_alive() {
        let h = connect_for_holding(|_| {});
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(500));
        let while_held = jog_lines(&h.wire()).len();
        // 0.5 s plus 0.3 s of lead, at 0.05 s a segment: about 16.
        assert!(while_held >= 10, "only {while_held} segments");
        h.connection.stop_held_jog().unwrap();
        h.wait_for_jog_cancel();
        std::thread::sleep(Duration::from_millis(300));
        let wire = h.wire();
        assert!(jog_lines(&wire).iter().all(|l| *l == SEGMENT), "{wire:?}");
        assert!(jog_lines(after_cancel(&wire)).is_empty(), "{wire:?}");
    }

    #[test]
    fn held_jog_stops_with_jog_cancel_when_keep_alive_stops() {
        let h = connect_for_holding(|_| {});
        let started = Instant::now();
        h.connection.start_held_jog([0, -1, 0], HELD_FEED).unwrap();
        // No keep-alive at all: the watchdog must end it.
        h.wait_for(|e| {
            matches!(e, Event::Line { text } if text.contains("stopped hearing")).then_some(())
        });
        let waited = started.elapsed();
        assert!(
            waited >= HELD_WATCHDOG && waited < Duration::from_secs(1),
            "{waited:?}"
        );
        std::thread::sleep(Duration::from_millis(300));
        let wire = h.wire();
        assert!(jog_lines(&wire)
            .iter()
            .all(|l| *l == "$J=G91 G21 Y-0.833 F1000"));
        assert!(!jog_lines(&wire).is_empty());
        assert!(jog_lines(after_cancel(&wire)).is_empty(), "{wire:?}");
        assert!(!h.sim.lock().unwrap().realtime.contains(&FEED_HOLD));
    }

    #[test]
    fn held_jog_stop_sends_jog_cancel_not_feed_hold() {
        // Both ways of stopping: letting go, and the Stop button.
        for use_stop_button in [false, true] {
            let h = connect_for_holding(|_| {});
            h.connection.start_held_jog([0, 0, 1], 200.0).unwrap();
            h.keep_alive_for(Duration::from_millis(200));
            if use_stop_button {
                h.connection.stop_job().unwrap();
            } else {
                h.connection.stop_held_jog().unwrap();
            }
            h.wait_for_jog_cancel();
            std::thread::sleep(Duration::from_millis(300));
            let sim = h.sim.lock().unwrap();
            assert!(
                !sim.realtime.contains(&FEED_HOLD),
                "Stop button: {use_stop_button}"
            );
            assert!(
                !sim.realtime.contains(&SOFT_RESET),
                "Stop button: {use_stop_button}"
            );
            assert!(jog_lines(after_cancel(&sim.wire)).is_empty());
        }
    }

    #[test]
    fn held_jog_lead_never_exceeds_budget() {
        let h = connect_for_holding(|_| {});
        let started = Instant::now();
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(1000));
        h.connection.stop_held_jog().unwrap();
        h.wait_for_jog_cancel();
        let sim = h.sim.lock().unwrap();
        let dt = jog::segment_time(HELD_FEED, 500.0);
        assert_eq!(dt, 0.05);
        assert!(sim.jog_arrivals.len() >= 20, "{}", sim.jog_arrivals.len());
        for (i, arrived) in sim.jog_arrivals.iter().enumerate() {
            let lead = (i + 1) as f64 * dt - (*arrived - started).as_secs_f64();
            assert!(lead <= HELD_LEAD + 1e-9, "segment {i} was {lead} s ahead");
        }
        assert!(!sim.overflowed);
    }

    #[test]
    fn held_jog_refused_unless_idle() {
        let h = connect(|sim| sim.alarm = true);
        h.wait_for(|e| matches!(e, Event::Setting { number: 122, .. }).then_some(()));
        h.wait_for(|e| matches!(e, Event::Line { text } if text.starts_with("[GC:")).then_some(()));
        h.wait_for_state(MachineState::Alarm);
        let error = h
            .connection
            .start_held_jog([1, 0, 0], HELD_FEED)
            .unwrap_err();
        assert!(error.contains("must be idle"), "{error}");
        std::thread::sleep(Duration::from_millis(100));
        assert!(jog_lines(&h.wire()).is_empty());
    }

    #[test]
    fn held_jog_refused_during_job() {
        let h = connect_for_holding(|_| {});
        h.connection
            .start_job(&program(20_000), Tool::Laser)
            .unwrap();
        wait_for_running(&h);
        let error = h
            .connection
            .start_held_jog([1, 0, 0], HELD_FEED)
            .unwrap_err();
        assert!(error.contains("job is running"), "{error}");
        h.keep_alive_for(Duration::from_millis(200));
        assert!(jog_lines(&h.wire()).is_empty());
        h.connection.reset().unwrap();
    }

    #[test]
    fn held_jog_refused_with_spindle_on() {
        let h = connect_for_holding(|sim| sim.spindle = 100);
        h.wait_for(|e| {
            matches!(e, Event::Status { spindle: Some(s), .. } if *s == 100.0).then_some(())
        });
        let error = h
            .connection
            .start_held_jog([1, 0, 0], HELD_FEED)
            .unwrap_err();
        assert!(error.contains("laser or spindle is on (S100)"), "{error}");
        std::thread::sleep(Duration::from_millis(100));
        assert!(jog_lines(&h.wire()).is_empty());
    }

    #[test]
    fn held_jog_refused_until_acceleration_is_known() {
        let h = connect(|sim| sim.no_acceleration = true);
        h.wait_for(|e| matches!(e, Event::Setting { number: 32, .. }).then_some(()));
        h.wait_for(|e| matches!(e, Event::Line { text } if text.starts_with("[GC:")).then_some(()));
        h.wait_for_state(MachineState::Idle);
        let error = h
            .connection
            .start_held_jog([1, 0, 0], HELD_FEED)
            .unwrap_err();
        assert!(error.contains("acceleration"), "{error}");
    }

    #[test]
    fn held_jog_stops_when_the_spindle_comes_on() {
        let h = connect_for_holding(|_| {});
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(100));
        h.sim.lock().unwrap().spindle = 50;
        h.keep_alive_for(Duration::from_millis(500));
        let wire = h.wire();
        assert!(jog_lines(after_cancel(&wire)).is_empty(), "{wire:?}");
    }

    #[test]
    fn held_jog_ends_on_error_or_alarm() {
        // A refused segment, as at a soft limit.
        let h = connect_for_holding(|sim| sim.jog_error = Some(15));
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        let code = h.wait_for(|e| match e {
            Event::CommandError { code, .. } => Some(*code),
            _ => None,
        });
        assert_eq!(code, 15);
        // Still "held", but it must not carry on.
        h.keep_alive_for(Duration::from_millis(400));
        let wire = h.wire();
        assert!(jog_lines(after_cancel(&wire)).is_empty(), "{wire:?}");
        let more_errors = h
            .events
            .try_iter()
            .filter(|e| matches!(e, Event::CommandError { .. }))
            .count();
        assert_eq!(more_errors, 0, "one error is reported, not one per segment");

        // An alarm, such as a limit switch.
        let h = connect_for_holding(|_| {});
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(100));
        {
            let mut sim = h.sim.lock().unwrap();
            sim.alarm = true;
            sim.say("ALARM:1");
        }
        h.wait_for(|e| matches!(e, Event::Alarm { code: 1, .. }).then_some(()));
        h.keep_alive_for(Duration::from_millis(400));
        let wire = h.wire();
        assert!(jog_lines(after_cancel(&wire)).is_empty(), "{wire:?}");
    }

    #[test]
    fn held_jog_rejects_two_axes() {
        let h = connect_for_holding(|_| {});
        for direction in [[1, 1, 0], [0, -1, 1], [0, 0, 0], [2, 0, 0]] {
            let error = h
                .connection
                .start_held_jog(direction, HELD_FEED)
                .unwrap_err();
            assert!(error.contains("one axis"), "{direction:?}: {error}");
        }
        assert!(h.connection.start_held_jog([1, 0, 0], f64::NAN).is_err());
        assert!(h.connection.start_held_jog([1, 0, 0], 0.0).is_err());
    }

    #[test]
    fn manual_lines_work_after_cancel_whether_purged_lines_are_acknowledged_or_not() {
        for answer_purged in [false, true] {
            // The machine holds the segments unanswered, so some are still
            // in flight when the cancel goes out.
            let h = connect_for_holding(|sim| sim.answer_purged = answer_purged);
            h.sim.lock().unwrap().stall = true;
            h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
            h.keep_alive_for(Duration::from_millis(150));
            // Never more than two unanswered.
            assert_eq!(h.sim.lock().unwrap().buffered.len(), 2);
            h.connection.stop_held_jog().unwrap();
            h.connection.send_line("G4 P0").unwrap();
            // A job only starts once nothing is in flight, so this finishing
            // shows the count is back in step.
            let deadline = Instant::now() + Duration::from_secs(10);
            while h.connection.start_job(&program(50), Tool::Laser).is_err() {
                assert!(Instant::now() < deadline, "the machine never went idle");
                std::thread::sleep(Duration::from_millis(20));
            }
            let (state, acknowledged, total, _) = h.wait_for_job_end();
            assert_eq!(
                (state, acknowledged, total),
                (JobState::Finished, 50, 50),
                "purged lines answered: {answer_purged}"
            );
            let wire = h.wire();
            let first = wire.iter().position(|l| l == "<jog cancel>").unwrap();
            let after = &wire[first + 1..];
            // The marker, then a second cancel once it is answered, then the rest.
            assert_eq!(after[..3], ["$G", "<jog cancel>", "G4 P0"], "{after:?}");
            assert!(jog_lines(after).is_empty());
            assert!(!h.sim.lock().unwrap().overflowed);
        }
    }

    #[test]
    fn disconnecting_mid_hold_cancels_and_resets() {
        let h = connect_for_holding(|_| {});
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(100));
        let Harness {
            sim, connection, ..
        } = h;
        drop(connection);
        let sim = sim.lock().unwrap();
        let wire = after_cancel(&sim.wire);
        assert_eq!(wire, ["<reset>"]);
    }

    /// Every kind of move, each expected to be refused.
    fn try_every_move(h: &Harness) -> Vec<Result<(), String>> {
        vec![
            h.connection.jog(1.0, 0.0, 0.0, 1000.0),
            h.connection.frame(0.0, 0.0, 1.0, 1.0, 1000.0),
            h.connection.go_to_xy_zero(1000.0),
            h.connection.start_held_jog([1, 0, 0], HELD_FEED),
        ]
    }

    #[test]
    fn moves_refused_after_typed_m3_in_laser_mode() {
        // In laser mode GRBL reports a speed of 0 until the machine moves,
        // so only the M3 itself shows the laser would fire.
        let h = connect_ready(|_| {});
        h.connection.send_line("M3 S1000").unwrap();
        for refusal in try_every_move(&h) {
            let message = refusal.unwrap_err();
            assert!(message.contains("M3 or M4"), "{message}");
            assert!(message.contains("Send M5"), "{message}");
        }
        wait_until_executed(&h, 3);
        std::thread::sleep(Duration::from_millis(100));
        for refusal in try_every_move(&h) {
            assert!(refusal.unwrap_err().contains("M3 or M4"));
        }
        assert!(jog_lines(&h.wire()).is_empty(), "{:?}", h.wire());

        // M5 switches it off again, and moving is allowed.
        h.connection.send_line("M5").unwrap();
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
        let executed = wait_until_executed(&h, 5);
        assert_eq!(executed[2..], ["M3 S1000", "M5", "$J=G91 G21 X1.000 F1000"]);
    }

    #[test]
    fn spindle_words_are_read_from_typed_lines() {
        assert_eq!(spindle_words("M3 S1000"), Some(true));
        assert_eq!(spindle_words("g1 x10 m4 s200"), Some(true));
        assert_eq!(spindle_words("M03"), Some(true));
        assert_eq!(spindle_words("M3 S100 M5"), Some(false));
        assert_eq!(spindle_words("M30"), Some(false));
        assert_eq!(spindle_words("M2"), Some(false));
        assert_eq!(spindle_words("M8"), None);
        assert_eq!(spindle_words("G0 X1 (M3 in a comment)"), None);
        assert_eq!(spindle_words("G0 X1 ; M3"), None);
        assert_eq!(spindle_words("$N0=M3 S100"), None);
        assert_eq!(
            modes_spindle("[GC:G0 G54 G17 G21 G90 G94 M3 M9 T0 F0 S0]"),
            SpindleMode::On
        );
        assert_eq!(
            modes_spindle("[GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0]"),
            SpindleMode::Off
        );
    }

    #[test]
    fn moves_refused_while_the_machine_reports_m3() {
        // Switched on before Kerf connected: the `$G` asked on connecting shows it.
        let h = connect_ready(|sim| sim.spindle_on = true);
        for refusal in try_every_move(&h) {
            assert!(refusal.unwrap_err().contains("M3 or M4"));
        }
        // A reset switches the spindle off; `$G` is asked again to know it.
        h.connection.reset().unwrap();
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        h.wait_for(|e| matches!(e, Event::Line { text } if text.contains(" M5 ")).then_some(()));
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
    }

    #[test]
    fn moves_refused_until_the_spindle_state_is_known() {
        // No speed in the status reports.
        let h = connect_ready(|sim| sim.no_spindle_report = true);
        for refusal in try_every_move(&h) {
            assert!(refusal.unwrap_err().contains("has not said"));
        }
        // No report from `$G`, so M3 or M5 is unknown until M5 is sent.
        let h = connect(|sim| sim.modes_query_without_report = true);
        h.wait_for(|e| matches!(e, Event::Setting { number: 122, .. }).then_some(()));
        h.wait_for_state(MachineState::Idle);
        for refusal in try_every_move(&h) {
            assert!(refusal.unwrap_err().contains("does not know yet"));
        }
        h.connection.send_line("M5").unwrap();
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
        assert!(jog_lines(&wait_until_executed(&h, 4))
            .iter()
            .all(|l| *l == "$J=G91 G21 X1.000 F1000"));
    }

    #[test]
    fn moves_refused_while_replies_are_out_of_step_then_recover() {
        // The marker is never answered, so the resync waits for its timeout.
        let h = connect_for_holding(|_| {});
        {
            let mut sim = h.sim.lock().unwrap();
            sim.stall = true;
            sim.mute_modes_query = true;
        }
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(100));
        h.connection.stop_held_jog().unwrap();
        h.wait_for_jog_cancel();
        std::thread::sleep(Duration::from_millis(50));
        let error = h.connection.jog(1.0, 0.0, 0.0, 1000.0).unwrap_err();
        assert!(error.contains("still busy"), "{error}");
        // Clicks do not pile up: nothing was queued by the refused one.
        h.wait_for(|e| {
            matches!(e, Event::Line { text } if text.contains("did not answer")).then_some(())
        });
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !h.wire().contains(&"$J=G91 G21 X1.000 F1000".to_owned()) {
            assert!(Instant::now() < deadline, "{:?}", h.wire());
            std::thread::sleep(Duration::from_millis(5));
        }
        let wire = h.wire();
        let first = wire.iter().position(|l| l == "<jog cancel>").unwrap();
        assert_eq!(
            wire[first + 1..],
            ["$G", "<jog cancel>", "$J=G91 G21 X1.000 F1000"],
        );
    }

    #[test]
    fn held_jog_stop_cancels_again_when_grbl_keeps_its_receive_buffer() {
        // Real GRBL ignores a cancel outside a jog and keeps its receive
        // buffer, so segments still there run as a fresh jog afterwards.
        let h = connect_for_holding(|sim| sim.keeps_receive_buffer = true);
        h.sim.lock().unwrap().stall = true;
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(100));
        h.connection.stop_held_jog().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.sim.lock().unwrap().jogs_cancelled == 0 {
            assert!(Instant::now() < deadline, "{:?}", h.wire());
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(100));
        let sim = h.sim.lock().unwrap();
        // The two segments in the buffer ran after the first cancel, then the
        // marker, then the second cancel stopped them.
        let first = sim.wire.iter().position(|l| l == "<jog cancel>").unwrap();
        assert_eq!(sim.wire[first + 1..], ["$G", "<jog cancel>"]);
        assert_eq!(
            &sim.executed[sim.executed.len() - 3..],
            [SEGMENT, SEGMENT, "$G"]
        );
        assert!(sim.jog_ends.is_none());
    }

    #[test]
    fn held_jog_feed_is_capped_at_the_axis_maximum() {
        let h = connect_for_holding(|sim| sim.extra_settings = vec![(110, "600.000")]);
        h.connection.start_held_jog([1, 0, 0], 3000.0).unwrap();
        h.keep_alive_for(Duration::from_millis(200));
        h.connection.stop_held_jog().unwrap();
        h.wait_for_jog_cancel();
        let wire = h.wire();
        // 600 mm/min for 0.05 s is 0.5 mm.
        let jogs = jog_lines(&wire);
        assert!(!jogs.is_empty());
        assert!(
            jogs.iter().all(|l| *l == "$J=G91 G21 X0.500 F600"),
            "{jogs:?}"
        );
    }

    #[test]
    fn held_jog_ends_after_the_axis_travel() {
        // 4 mm of travel holds four 0.833 mm segments; a fifth would pass it.
        let h = connect_for_holding(|sim| sim.extra_settings = vec![(130, "4.000")]);
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(300));
        h.wait_for(|e| {
            matches!(e, Event::Line { text } if text.contains("Press again")).then_some(())
        });
        h.keep_alive_for(Duration::from_millis(300));
        let wire = h.wire();
        assert_eq!(jog_lines(&wire).len(), 4, "{wire:?}");
        assert!(jog_lines(after_cancel(&wire)).is_empty());
        // A fresh press is allowed.
        h.wait_for_state(MachineState::Idle);
        when_free(|| h.connection.start_held_jog([-1, 0, 0], HELD_FEED)).unwrap();
        h.connection.stop_held_jog().unwrap();
    }

    #[test]
    fn a_typed_modes_query_ahead_of_the_marker_is_not_taken_for_it() {
        let h = connect_ready(|_| {});
        h.sim.lock().unwrap().stall = true;
        h.connection.send_line("$G").unwrap();
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.sim.lock().unwrap().buffered.len() < 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        // In flight: the typed $G, then a jog. The cancel adds the marker.
        h.connection.cancel_jog().unwrap();
        h.connection.send_line("G4 P0").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !h.wire().contains(&"G4 P0".to_owned()) {
            assert!(Instant::now() < deadline, "{:?}", h.wire());
            std::thread::sleep(Duration::from_millis(2));
        }
        let sim = h.sim.lock().unwrap();
        let modes_queries_run =
            |run: usize| sim.executed[..run].iter().filter(|l| *l == "$G").count();
        // The second cancel and the next line wait for the marker's reply:
        // the one asked on connecting, the typed one, and the marker.
        for (line, run) in sim.arrivals.iter().rev().take(2) {
            assert_eq!(modes_queries_run(*run), 3, "{line} arrived too soon");
        }
    }

    #[test]
    fn a_fault_in_the_worker_stops_the_machine_and_is_reported() {
        let h = connect_for_holding(|_| {});
        h.connection.start_held_jog([1, 0, 0], HELD_FEED).unwrap();
        h.keep_alive_for(Duration::from_millis(100));
        h.sim.lock().unwrap().panic_on_read = true;
        let reason = h.wait_for(|e| match e {
            Event::Disconnected { reason } => Some(reason.clone()),
            _ => None,
        });
        assert!(reason.unwrap().contains("reset the machine"));
        let sim = h.sim.lock().unwrap();
        assert_eq!(sim.wire[sim.wire.len() - 2..], ["<jog cancel>", "<reset>"]);
    }

    #[test]
    fn a_modes_reply_sent_before_a_typed_m3_does_not_clear_it() {
        let h = connect_ready(|_| {});
        // Both in flight together: the `$G` reply, made before the M3 ran,
        // says M5.
        h.sim.lock().unwrap().stall = true;
        h.connection.send_line("$G").unwrap();
        h.connection.send_line("M3 S1000").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.sim.lock().unwrap().buffered.len() < 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        h.sim.lock().unwrap().stall = false;
        wait_until_executed(&h, 4);
        std::thread::sleep(Duration::from_millis(100));
        for refusal in [
            h.connection.jog(1.0, 0.0, 0.0, 1000.0),
            h.connection.frame(0.0, 0.0, 1.0, 1.0, 1000.0),
        ] {
            let message = refusal.unwrap_err();
            assert!(message.contains("M3 or M4"), "{message}");
        }
        assert!(jog_lines(&h.wire()).is_empty(), "{:?}", h.wire());
    }

    #[test]
    fn moves_wait_for_a_typed_m5_to_be_answered() {
        let h = connect_ready(|_| {});
        h.connection.send_line("M3 S1000").unwrap();
        wait_until_executed(&h, 3);
        h.sim.lock().unwrap().stall = true;
        h.connection.send_line("M5").unwrap();
        // Sent, but not yet run: the laser may still be on.
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.sim.lock().unwrap().buffered.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let message = h.connection.jog(1.0, 0.0, 0.0, 1000.0).unwrap_err();
        assert!(message.contains("last M3, M4 or M5"), "{message}");
        h.sim.lock().unwrap().stall = false;
        when_free(|| h.connection.jog(1.0, 0.0, 0.0, 1000.0)).unwrap();
    }

    #[test]
    fn a_move_that_times_out_is_cancelled() {
        let h = connect_ready(|_| {});
        h.sim.lock().unwrap().slow_read = Some(REPLY_TIMEOUT + Duration::from_millis(500));
        std::thread::sleep(Duration::from_millis(50));
        let message = h.connection.jog(1.0, 0.0, 0.0, 1000.0).unwrap_err();
        assert!(message.contains("did not answer in time"), "{message}");
        // The worker wakes, takes the move, then the cancel: nothing moves.
        std::thread::sleep(Duration::from_millis(1000));
        let wire = h.wire();
        assert!(jog_lines(&wire).is_empty(), "{wire:?}");
        assert!(wire.contains(&"<jog cancel>".to_owned()), "{wire:?}");
    }

    #[test]
    fn a_jog_cancel_drops_jog_lines_not_yet_sent() {
        let h = connect_ready(|_| {});
        h.sim.lock().unwrap().stall = true;
        // Five framing lines do not all fit in the 127-byte buffer.
        when_free(|| h.connection.frame(0.0, 0.0, 1.0, 1.0, 1000.0)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.sim.lock().unwrap().buffered.len() < 4 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        h.connection.cancel_jog().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let wire = h.wire();
        assert_eq!(jog_lines(&wire).len(), 4, "{wire:?}");
        assert!(jog_lines(after_cancel(&wire)).is_empty(), "{wire:?}");
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

    // Jobs with bit changes, and setting Z with a touch plate.

    /// The first part of a two-bit job, as `kerf-gcode` writes it: the
    /// change block at the end turns the spindle off, waits for it to stop,
    /// raises the bit, moves over the work zero and ends on a short dwell.
    const PART_1: &str = "; Generated by Kerf\nG17 G21 G90 G94 G54\nG0 Z5\nM3 S10000\nG4 P2\nG1 Z-1 F100\nG1 X1 F300\nG0 Z5\n; Bit change 1 of 1: fit T2 (1 mm tapered ball nose), then set Z again\nM5\nG4 P5\nG0 Z20\nG0 X0 Y0\nG4 P0.1\n";
    const PART_2: &str = "G17 G21 G90 G94 G54\nG0 Z5\nM3 S10000\nG4 P2\nG1 Z-1 F100\nG1 X2 F300\nG0 Z5\nM5\nG0 X0 Y0\n";
    const SECOND_PART_CUT: &str = "G1 X2 F300";

    fn code_lines(text: &str) -> Vec<&str> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with(';'))
            .collect()
    }

    /// Connected to a router: laser mode off, settings and spindle state read.
    fn connect_router(setup: impl FnOnce(&mut Sim)) -> Harness {
        connect_ready(|sim| {
            sim.laser_mode = Some("0");
            setup(sim);
        })
    }

    fn two_parts(lift: Option<f64>) -> JobOptions {
        JobOptions {
            change_lift: lift,
            ..JobOptions::default()
        }
    }

    impl Harness {
        fn executed(&self) -> Vec<String> {
            self.sim.lock().unwrap().executed.clone()
        }

        /// Starts the two-part job and waits until it waits for the bit.
        fn run_to_change(&self) {
            self.connection
                .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
                .unwrap();
            self.wait_for_bit();
        }

        fn wait_for_bit(&self) {
            self.wait_for(|e| {
                matches!(
                    e,
                    Event::Job {
                        state: JobState::WaitingForBit,
                        ..
                    }
                )
                .then_some(())
            });
        }

        fn wait_for_parts(&self, mut pick: impl FnMut(&PartsProgress) -> bool) {
            self.wait_for(|e| match e {
                Event::Parts {
                    progress: Some(progress),
                } if pick(progress) => Some(()),
                _ => None,
            });
        }

        fn wait_for_probe(&self) -> (ProbeState, Option<String>) {
            self.wait_for(|e| match e {
                Event::Probe { state, message } if *state != ProbeState::Running => {
                    Some((*state, message.clone()))
                }
                _ => None,
            })
        }

        /// Passes the probe test: the plate touches the bit, then is taken away.
        fn pass_probe_test(&self) {
            self.connection.probe_test().unwrap();
            self.sim.lock().unwrap().probe_touching = true;
            self.wait_for(|e| matches!(e, Event::ProbeTested).then_some(()));
            self.sim.lock().unwrap().probe_touching = false;
            self.wait_for(|e| {
                matches!(e, Event::Status { pins, state: MachineState::Idle, .. } if pins.is_empty())
                    .then_some(())
            });
        }

        fn wait_for_idle_and_quiet(&self) {
            self.wait_for_state(MachineState::Idle);
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    const PLATE: ProbeSettings = ProbeSettings {
        plate: 12.7,
        travel: probe::DEFAULT_TRAVEL,
        fast_feed: probe::DEFAULT_FAST_FEED,
        slow_feed: probe::DEFAULT_SLOW_FEED,
    };

    #[test]
    fn grblhal_banner_is_a_restart() {
        let h = connect(|sim| sim.hal = true);
        h.wait_for_state(MachineState::Idle);
        h.connection
            .start_job(&program(20_000), Tool::Laser)
            .unwrap();
        wait_for_running(&h);
        // The machine's own reset button: GRBL forgets everything and says
        // hello again, with grblHAL's banner.
        {
            let mut sim = h.sim.lock().unwrap();
            sim.buffered.clear();
            sim.say("GrblHAL 1.1f ['$' or '$HELP' for help]");
        }
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Stopped);
        let firmware = h.wait_for(|e| match e {
            Event::Welcome { firmware, version } if version == "1.1f" => Some(*firmware),
            _ => None,
        });
        assert_eq!(firmware, Firmware::GrblHal);
        // The settings and modes are read again, and nothing more of the job is sent.
        h.wait_for(|e| matches!(e, Event::Setting { number: 130, .. }).then_some(()));
        std::thread::sleep(Duration::from_millis(200));
        // Lines already on their way when the banner came may still arrive,
        // but once Kerf has seen it, nothing of the job is sent.
        let wire = h.wire();
        let reread = wire
            .iter()
            .rposition(|l| l == "$$")
            .expect("settings read again");
        assert!(
            !wire[reread..].iter().any(|l| l.starts_with("G1")),
            "{:?}",
            &wire[reread..]
        );
        assert!(wire[reread..].contains(&"$G".to_owned()));
    }

    #[test]
    fn grblhal_restart_while_waiting_ends_the_job() {
        let h = connect_router(|sim| sim.hal = true);
        h.run_to_change();
        h.sim
            .lock()
            .unwrap()
            .say("GrblHAL 1.1f ['$' or '$HELP' for help]");
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Stopped);
        h.wait_for(|e| matches!(e, Event::Parts { progress: None }).then_some(()));
        let error = h.connection.continue_job(false).unwrap_err();
        assert!(error.contains("No job is waiting"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn waits_between_parts_until_continued() {
        let h = connect_router(|_| {});
        h.run_to_change();
        std::thread::sleep(Duration::from_millis(300));
        // The offsets are read before anything of the job is sent.
        let mut expected: Vec<&str> = vec!["$$", "$G", "$#"];
        expected.extend(code_lines(PART_1));
        // The spindle's state and the offsets are read once the part is over.
        expected.extend(["$G", "$#"]);
        assert_eq!(h.executed(), expected);

        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.waiting && p.z_set && p.spindle_off);
        when_free(|| h.connection.continue_job(false)).unwrap();
        let (state, acknowledged, total, _) = h.wait_for_job_end();
        assert_eq!((state, acknowledged, total), (JobState::Finished, 9, 9));
        h.wait_for(|e| matches!(e, Event::Parts { progress: None }).then_some(()));
        std::thread::sleep(Duration::from_millis(100));
        // Zero Z here, the read it causes, the fresh read for Continue, then part 2.
        expected.extend(["G10 L20 P0 Z0", "$#", "$#"]);
        expected.extend(code_lines(PART_2));
        expected.push("$G");
        assert_eq!(h.executed(), expected);
    }

    #[test]
    fn waiting_card_not_shown_until_a_fresh_idle_after_last_ok() {
        // The machine says it is still running for a while after each dwell.
        let h = connect_router(|sim| sim.run_after_dwell = 4);
        h.connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
            .unwrap();
        let mut last_ok_seen = false;
        let mut running_after = 0;
        let mut last_state = None;
        h.wait_for(|e| match e {
            Event::Job {
                state: JobState::Running,
                acknowledged,
                total,
                ..
            } if acknowledged == total && *total > 0 => {
                last_ok_seen = true;
                None
            }
            Event::Status { state, .. } => {
                if last_ok_seen && *state == MachineState::Run {
                    running_after += 1;
                }
                last_state = Some(*state);
                None
            }
            Event::Parts {
                progress: Some(PartsProgress { waiting: true, .. }),
            } => Some(()),
            Event::Job {
                state: JobState::WaitingForBit,
                ..
            } => {
                assert!(last_ok_seen, "waited before the last line was answered");
                assert!(running_after >= 1, "no busy report was waited out");
                assert_eq!(last_state, Some(MachineState::Idle));
                None
            }
            _ => None,
        });
        assert!(running_after >= 1);
    }

    #[test]
    fn continue_is_refused_until_z_is_set() {
        let h = connect_router(|_| {});
        h.run_to_change();
        h.wait_for_idle_and_quiet();
        let error = h.connection.continue_job(false).unwrap_err();
        assert!(error.contains("Set Z again"), "{error}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn continue_is_refused_while_the_spindle_is_on() {
        let h = connect_router(|_| {});
        h.run_to_change();
        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.z_set);
        h.sim.lock().unwrap().spindle = 100;
        h.wait_for(|e| {
            matches!(e, Event::Status { spindle: Some(s), .. } if *s == 100.0).then_some(())
        });
        let error = h.connection.continue_job(false).unwrap_err();
        assert!(error.contains("S100"), "{error}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn continue_refused_after_typed_m3_while_waiting() {
        let h = connect_router(|_| {});
        h.run_to_change();
        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.z_set);
        h.connection.send_line("M3 S1000").unwrap();
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("M3 or M4"), "{error}");
        wait_until_executed(&h, h.executed().len().max(1));
        std::thread::sleep(Duration::from_millis(100));
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("M3 or M4"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
        // Switched off again, it may carry on.
        h.connection.send_line("M5").unwrap();
        when_free(|| h.connection.continue_job(false)).unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Finished);
    }

    #[test]
    fn continue_is_refused_if_xy_zero_changed() {
        let h = connect_router(|_| {});
        h.run_to_change();
        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.z_set);
        h.connection.send_line("G10 L20 P0 X0").unwrap();
        wait_until_executed(&h, h.executed().len() + 2);
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("X/Y zero"), "{error}");
        assert!(error.contains("Stop the job"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn continue_is_refused_with_g92_set() {
        let h = connect_router(|_| {});
        h.run_to_change();
        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.z_set);
        h.connection.send_line("G92 Z3").unwrap();
        wait_until_executed(&h, h.executed().len() + 2);
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("(or an offset)"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn jogging_and_zero_z_work_while_waiting() {
        let h = connect_router(|_| {});
        h.run_to_change();
        when_free(|| h.connection.jog(0.0, 0.0, 5.0, 200.0)).unwrap();
        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.z_set);
        let executed = h.executed();
        let at = executed
            .iter()
            .position(|l| l == "$J=G91 G21 Z5.000 F200")
            .expect("the jog ran");
        assert_eq!(executed[at + 1], "G10 L20 P0 Z0");
    }

    #[test]
    fn zero_xy_is_refused_while_waiting() {
        let h = connect_router(|_| {});
        // Before a job, it is Kerf's own P0 line.
        when_free(|| h.connection.zero_xy()).unwrap();
        wait_until_executed(&h, 3);
        assert_eq!(h.executed()[2], "G10 L20 P0 X0 Y0");
        h.run_to_change();
        let error = h.connection.zero_xy().unwrap_err();
        assert!(error.contains("Zero XY"), "{error}");
        std::thread::sleep(Duration::from_millis(200));
        let wire = h.wire();
        assert_eq!(
            wire.iter().filter(|l| *l == "G10 L20 P0 X0 Y0").count(),
            1,
            "{wire:?}"
        );
    }

    #[test]
    fn start_refused_while_waiting_for_bit() {
        let h = connect_router(|_| {});
        h.run_to_change();
        h.wait_for_idle_and_quiet();
        for refusal in [
            h.connection.start_job(PART_2, Tool::Spindle),
            h.connection
                .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0))),
        ] {
            let error = refusal.unwrap_err();
            assert!(error.contains("waiting for a bit change"), "{error}");
        }
    }

    #[test]
    fn stop_while_waiting_ends_the_job() {
        let h = connect_router(|_| {});
        h.run_to_change();
        h.wait_for_idle_and_quiet();
        h.connection.stop_job().unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Stopped);
        h.wait_for(|e| matches!(e, Event::Parts { progress: None }).then_some(()));
        let error = h.connection.continue_job(false).unwrap_err();
        assert!(error.contains("No job is waiting"), "{error}");
        // Start needs a new job, which it accepts.
        h.connection.start_job(PART_2, Tool::Spindle).unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Finished);
        assert_eq!(
            h.executed()
                .iter()
                .filter(|l| *l == SECOND_PART_CUT)
                .count(),
            1
        );
    }

    #[test]
    fn reset_while_waiting_clears_parts() {
        let h = connect_router(|_| {});
        h.run_to_change();
        when_free(|| h.connection.zero_z()).unwrap();
        h.wait_for_parts(|p| p.z_set);
        h.connection.reset().unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Stopped);
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        let error = h.connection.continue_job(false).unwrap_err();
        assert!(error.contains("No job is waiting"), "{error}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn probe_lines_are_exact_and_sent_one_at_a_time() {
        // Work Z 6 (machine 7, G54 Z 1); the plate's top is at machine Z 0.
        let h = connect_router(|sim| sim.plate_at = Some(0.0));
        h.pass_probe_test();
        let before = h.executed().len();
        h.connection.probe_z(PLATE).unwrap();
        assert_eq!(h.wait_for_probe(), (ProbeState::Done, None));
        let sim = h.sim.lock().unwrap();
        let lines = [
            "G90 G38.2 Z-4.000 F100",
            "G90 G0 Z1.000",
            "G90 G38.2 Z-2.000 F25",
            "G10 L2 P0 Z-12.700",
            "G90 G0 Z17.700",
        ];
        let mut expected = vec!["$#"];
        expected.extend(lines);
        expected.push("$#");
        assert_eq!(sim.executed[before..], expected);
        // Each line arrived only once the one before it had run.
        for pair in lines.windows(2) {
            let ran = sim.executed.iter().position(|l| l == pair[0]).unwrap();
            let (_, run_by_then) = sim
                .arrivals
                .iter()
                .find(|(l, _)| l == pair[1])
                .expect("sent");
            assert!(
                *run_by_then > ran,
                "{} was sent before {} ran",
                pair[1],
                pair[0]
            );
        }
        assert_eq!(sim.g54(), [1.0, 1.0, -12.7]);
    }

    #[test]
    fn probe_start_height_ignores_a_stale_work_offset() {
        // Work Z is 6 (machine 7, G54 Z 1), but the status reports still
        // give an old offset. With MPos or with WPos only ($10=0), the first
        // line goes down 10 mm from work Z 6, from the fresh G54.
        for work_position in [false, true] {
            let h = connect_router(|sim| {
                sim.plate_at = Some(0.0);
                sim.report_work_position = work_position;
                sim.stale_wco = Some([1.0, 1.0, 9.0]);
            });
            h.pass_probe_test();
            h.connection.probe_z(PLATE).unwrap();
            assert_eq!(h.wait_for_probe(), (ProbeState::Done, None));
            let wire = h.wire();
            let first = wire.iter().find(|l| l.contains("G38.2")).unwrap();
            assert_eq!(
                first, "G90 G38.2 Z-4.000 F100",
                "WPos only: {work_position}"
            );
            assert!(wire.contains(&"G10 L2 P0 Z-12.700".to_owned()), "{wire:?}");
        }
    }

    #[test]
    fn g10_not_sent_after_failed_probe() {
        for quietly in [false, true] {
            // No plate under the bit: it goes the whole 10 mm and touches nothing.
            let h = connect_router(|sim| sim.probe_misses_quietly = quietly);
            h.pass_probe_test();
            h.connection.probe_z(PLATE).unwrap();
            let (state, message) = h.wait_for_probe();
            assert_eq!(state, ProbeState::Failed, "quietly: {quietly}");
            assert!(message.unwrap().contains("didn't touch"));
            std::thread::sleep(Duration::from_millis(200));
            let wire = h.wire();
            assert!(!wire.iter().any(|l| l.starts_with("G10")), "{wire:?}");
            assert!(!wire.contains(&"G90 G0 Z-2.000".to_owned()), "{wire:?}");
            assert_eq!(
                wire.iter().filter(|l| l.contains("G38.2")).count(),
                1,
                "{wire:?}"
            );
        }
    }

    #[test]
    fn probe_is_refused_when_already_touching() {
        let h = connect_router(|sim| sim.plate_at = Some(0.0));
        let error = h.connection.probe_z(PLATE).unwrap_err();
        assert!(error.contains("Test the probe first"), "{error}");
        h.pass_probe_test();
        h.sim.lock().unwrap().probe_touching = true;
        h.wait_for(|e| matches!(e, Event::Status { pins, .. } if pins == "P").then_some(()));
        let error = h.connection.probe_z(PLATE).unwrap_err();
        assert!(error.contains("already reads as touching"), "{error}");
        // A plate thickness is needed, and no default is assumed.
        let error = h
            .connection
            .probe_z(ProbeSettings {
                plate: 0.0,
                ..PLATE
            })
            .unwrap_err();
        assert!(error.contains("thickness"), "{error}");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!h.wire().iter().any(|l| l.contains("G38")));
    }

    #[test]
    fn probe_alarm_keeps_parts_other_alarm_ends_job() {
        let h = connect_router(|_| {});
        h.run_to_change();
        h.pass_probe_test();
        // No plate: the probe runs out of travel and GRBL raises alarm 5.
        h.connection.probe_z(PLATE).unwrap();
        assert_eq!(h.wait_for_probe().0, ProbeState::Failed);
        h.wait_for(|e| matches!(e, Event::Alarm { code: 5, .. }).then_some(()));
        h.sim.lock().unwrap().alarm = false;
        h.wait_for_idle_and_quiet();
        // Still waiting for the bit, with Z not set.
        let error = h.connection.continue_job(true).unwrap_err();
        assert!(error.contains("Set Z again"), "{error}");
        // Any other alarm ends the job.
        {
            let mut sim = h.sim.lock().unwrap();
            sim.alarm = true;
            sim.say("ALARM:1");
        }
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Failed);
        let error = h.connection.continue_job(true).unwrap_err();
        assert!(error.contains("No job is waiting"), "{error}");
    }

    #[test]
    fn continue_refused_while_probe_pin_touching() {
        // After part 1 the bit is at work Z 20 (machine 21); the plate's top
        // is at machine Z 15.
        let h = connect_router(|sim| sim.plate_at = Some(15.0));
        h.run_to_change();
        h.pass_probe_test();
        when_free(|| h.connection.probe_z(PLATE)).unwrap();
        assert_eq!(h.wait_for_probe(), (ProbeState::Done, None));
        h.wait_for_parts(|p| p.z_set && p.plate_used);
        assert!(h.wire().contains(&"G10 L2 P0 Z2.300".to_owned()));
        // The clip is still on.
        h.sim.lock().unwrap().probe_touching = true;
        h.wait_for(|e| matches!(e, Event::Status { pins, .. } if pins == "P").then_some(()));
        let error = h.connection.continue_job(true).unwrap_err();
        assert!(error.contains("still reads as touching"), "{error}");
        h.sim.lock().unwrap().probe_touching = false;
        h.wait_for(|e| {
            matches!(e, Event::Status { pins, state: MachineState::Idle, .. } if pins.is_empty())
                .then_some(())
        });
        // Off, but the box is not ticked.
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("tick the box"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
        when_free(|| h.connection.continue_job(true)).unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Finished);
    }

    /// Homes the simulated machine and waits until Kerf has seen it idle after.
    fn home(h: &Harness) {
        h.sim.lock().unwrap().homing = true;
        h.wait_for_state(MachineState::Home);
        h.sim.lock().unwrap().homing = false;
        h.wait_for_state(MachineState::Idle);
    }

    #[test]
    fn machine_moves_need_homing_this_session() {
        let h = connect_router(|sim| sim.extra_settings = vec![(131, "500.000"), (132, "80.000")]);
        let options = JobOptions {
            needs_homing: true,
            machine_moves: vec![[-10.0, -10.0, -5.0]],
            ..JobOptions::default()
        };
        let error = h
            .connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &options)
            .unwrap_err();
        assert!(error.contains("homed"), "{error}");
        home(&h);
        let outside = JobOptions {
            machine_moves: vec![[-800.0, -10.0, -5.0]],
            ..options.clone()
        };
        let error = h
            .connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &outside)
            .unwrap_err();
        assert!(error.contains("outside the machine's travel"), "{error}");
        h.connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &options)
            .unwrap();
        h.wait_for_bit();
        // A reset forgets the homing: a new job must home again.
        h.connection.reset().unwrap();
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        h.wait_for_idle_and_quiet();
        let error = h
            .connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &options)
            .unwrap_err();
        assert!(error.contains("homed"), "{error}");
    }

    #[test]
    fn lift_refused_when_it_would_pass_machine_z0() {
        let h = connect_router(|sim| {
            sim.extra_settings = vec![(131, "500.000"), (132, "80.000")];
            // The material's top is 12 mm below the top of Z.
            sim.g54 = Some([0.0, 0.0, -12.0]);
            sim.mpos = Some([0.0, 0.0, -12.0]);
        });
        // Not homed: Kerf cannot tell, so the job may start (the person is warned).
        h.connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
            .unwrap();
        h.wait_for_bit();
        h.connection.stop_job().unwrap();
        h.wait_for_job_end();
        home(&h);
        h.wait_for(|e| matches!(e, Event::Status { .. }).then_some(()));
        let error = h
            .connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
            .unwrap_err();
        assert!(error.contains("rise 20 mm"), "{error}");
        assert!(error.contains("only 12 mm left"), "{error}");
        assert!(error.contains("to 12 or less"), "{error}");

        // Deeper down there is room; then Z is set higher at the change.
        h.sim.lock().unwrap().g54 = Some([0.0, 0.0, -40.0]);
        h.wait_for(|e| {
            matches!(
                e,
                Event::Status {
                    machine_position: Some(_),
                    ..
                }
            )
            .then_some(())
        });
        std::thread::sleep(Duration::from_millis(500));
        h.connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
            .unwrap();
        h.wait_for_bit();
        h.connection.send_line("G10 L2 P0 Z-5").unwrap();
        h.wait_for_parts(|p| p.z_set);
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("only 5 mm left"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn atc_job_refused_on_plain_grbl() {
        let atc = JobOptions {
            atc: true,
            ..JobOptions::default()
        };
        let h = connect_router(|_| {});
        let error = h
            .connection
            .start_parts(&[PART_2], Tool::Spindle, &atc)
            .unwrap_err();
        assert!(error.contains("checking which firmware"), "{error}");
        let firmware = h.wait_for(|e| match e {
            Event::Firmware { firmware } => Some(*firmware),
            _ => None,
        });
        assert_eq!(firmware, Firmware::Grbl);
        let error = h
            .connection
            .start_parts(&[PART_2], Tool::Spindle, &atc)
            .unwrap_err();
        assert!(error.contains("needs grblHAL"), "{error}");

        let h = connect_router(|sim| sim.hal = true);
        assert!(h
            .connection
            .start_parts(&[PART_2], Tool::Spindle, &atc)
            .is_err());
        let firmware = h.wait_for(|e| match e {
            Event::Firmware { firmware } => Some(*firmware),
            _ => None,
        });
        assert_eq!(firmware, Firmware::GrblHal);
        when_free(|| h.connection.start_parts(&[PART_2], Tool::Spindle, &atc)).unwrap();
        let (state, ..) = h.wait_for_job_end();
        assert_eq!(state, JobState::Finished);
    }

    #[test]
    fn stop_in_tool_state_resets() {
        let h = connect(|sim| {
            sim.tool_state = true;
            sim.settle_reports = 1_000;
        });
        h.wait_for_state(MachineState::Tool);
        let asked = Instant::now();
        h.connection.stop_job().unwrap();
        h.wait_for(|e| matches!(e, Event::Welcome { .. }).then_some(()));
        assert!(asked.elapsed() < STOP_TIMEOUT / 3, "{:?}", asked.elapsed());
        let sim = h.sim.lock().unwrap();
        assert_eq!(
            sim.realtime
                .iter()
                .filter(|&&b| b != STATUS_QUERY)
                .collect::<Vec<_>>(),
            [&SOFT_RESET]
        );
    }

    #[test]
    fn machine_change_place_below_the_work_is_refused() {
        // The material's top (G54 Z) is 30 mm below the top of Z; the job
        // must be 20 mm above it to go to the change place.
        let h = connect_router(|sim| {
            sim.extra_settings = vec![(131, "500.000"), (132, "80.000")];
            sim.g54 = Some([-100.0, -100.0, -30.0]);
            sim.mpos = Some([-100.0, -100.0, -10.0]);
        });
        home(&h);
        let at = |z: f64| JobOptions {
            needs_homing: true,
            machine_moves: vec![[-10.0, -10.0, z]],
            change_clearance: Some(20.0),
            ..JobOptions::default()
        };
        let error = h
            .connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &at(-15.0))
            .unwrap_err();
        assert!(error.contains("only 15 mm above the material"), "{error}");
        assert!(error.contains("at least 20 mm"), "{error}");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!h.wire().iter().any(|l| l.starts_with("G17")));
        // High enough: it starts. Then Z is set higher at the change.
        h.connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &at(-5.0))
            .unwrap();
        h.wait_for_bit();
        h.connection.send_line("G10 L2 P0 Z-20").unwrap();
        h.wait_for_parts(|p| p.z_set);
        let error = when_free(|| h.connection.continue_job(false)).unwrap_err();
        assert!(error.contains("only 15 mm above the material"), "{error}");
        assert!(!h.executed().contains(&SECOND_PART_CUT.to_owned()));
    }

    #[test]
    fn change_place_xy_must_be_on_the_machine() {
        let h = connect_router(|sim| sim.extra_settings = vec![(131, "500.000"), (132, "80.000")]);
        home(&h);
        for place in [
            [10.0, -10.0, -5.0],
            [-10.0, 10.0, -5.0],
            [-10.0, -600.0, -5.0],
        ] {
            let options = JobOptions {
                needs_homing: true,
                machine_moves: vec![place],
                ..JobOptions::default()
            };
            let error = h
                .connection
                .start_parts(&[PART_1, PART_2], Tool::Spindle, &options)
                .unwrap_err();
            assert!(
                error.contains("outside the machine's travel"),
                "{place:?}: {error}"
            );
        }
    }

    #[test]
    fn multi_part_start_and_probe_refused_unless_g54() {
        let h = connect_router(|sim| {
            sim.work_zero = Some("G55");
            sim.plate_at = Some(0.0);
        });
        let error = h
            .connection
            .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
            .unwrap_err();
        assert!(error.contains("other than G54"), "{error}");
        h.pass_probe_test();
        let error = when_free(|| h.connection.probe_z(PLATE)).unwrap_err();
        assert!(error.contains("other than G54"), "{error}");
        std::thread::sleep(Duration::from_millis(100));
        let wire = h.wire();
        assert!(
            !wire
                .iter()
                .any(|l| l.contains("G38") || l.starts_with("G17")),
            "{wire:?}"
        );
        // A typed G54 is read back, and then both are allowed.
        h.sim.lock().unwrap().work_zero = None;
        h.connection.send_line("G54").unwrap();
        h.wait_for(|e| matches!(e, Event::Line { text } if text.contains(" G54 ")).then_some(()));
        when_free(|| h.connection.probe_z(PLATE)).unwrap();
        assert_eq!(h.wait_for_probe(), (ProbeState::Done, None));
    }

    #[test]
    fn multi_part_start_refused_with_g92_or_tool_offset_set() {
        for (g92, tlo) in [([0.0, 0.0, 3.0], 0.0), ([0.0; 3], 1.5)] {
            let h = connect_router(|sim| {
                sim.g92 = g92;
                sim.tlo = tlo;
            });
            let error = h
                .connection
                .start_parts(&[PART_1, PART_2], Tool::Spindle, &two_parts(Some(20.0)))
                .unwrap_err();
            assert!(
                error.contains("G92 offset or a tool length offset"),
                "{error}"
            );
            std::thread::sleep(Duration::from_millis(100));
            assert!(!h.wire().iter().any(|l| l.starts_with("G17")));
        }
    }

    #[test]
    fn offset_lines_are_recognised() {
        assert!(changes_offsets("G10 L20 P0 Z0"));
        assert!(changes_offsets("g10l2p1x0"));
        assert!(changes_offsets("G92 X0"));
        assert!(changes_offsets("G0 X1 G092 Y0"));
        assert!(!changes_offsets("G1 X10"));
        assert!(!changes_offsets("G100"));
        assert!(!changes_offsets("$J=G91 X1 F100"));
        assert!(!changes_offsets("G0 X1 ; G10"));
        assert!(selects_work_zero("G55"));
        assert!(selects_work_zero("g0 g54 x1"));
        assert!(!selects_work_zero("G540"));
        assert!(!selects_work_zero("$G"));
        assert_eq!(
            modes_on_g54("[GC:G0 G54 G17 G21 G90 G94 M5 M9 T0 F0 S0]"),
            Some(true)
        );
        assert_eq!(
            modes_on_g54("[GC:G0 G56 G17 G21 G90 G94 M5 M9 T0 F0 S0]"),
            Some(false)
        );
    }
}
