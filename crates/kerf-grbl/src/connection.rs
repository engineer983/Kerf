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
        /// Position from the machine's home, when the controller reports
        /// enough to know it. Only meaningful once the machine has homed.
        machine_position: Option<[f64; 3]>,
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

/// The worker's answer to a request to move.
type Reply = Sender<Result<(), String>>;

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
    StartJob(Streamer),
    StopJob,
    Disconnect,
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
        let (reply, answer) = mpsc::channel();
        self.send(command(reply))?;
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
        let streamer = Streamer::new(program)?;
        if streamer.progress().1 == 0 {
            return Err("The program is empty.".into());
        }
        self.send(Command::StartJob(streamer))
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
        let settings = lock(&self.settings);
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
            Command::StartJob(streamer) => {
                self.stop_held(Some("Stopped moving: a job was started."))?;
                self.waiting_job = Some(streamer);
            }
            Command::StopJob if self.held.is_some() => {
                // A jog is stopped with a jog cancel, never a feed hold: a hold
                // that arrives once the jog has ended would pause the machine.
                self.stop_held(None)?;
            }
            Command::StopJob => {
                if *lock(&self.state) == Some(MachineState::Home) {
                    // GRBL ignores a feed hold while homing; only a soft reset
                    // stops it. It then raises an alarm (homing aborted).
                    self.port.write_all(&[SOFT_RESET])?;
                    self.after_reset();
                    self.abandon(JobState::Stopped, None);
                } else if self.job.is_some() || self.is_moving() {
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

    /// Why the machine must not be jogged now, if it must not. Jog lines
    /// cannot switch a laser or spindle on, but they carry along one that is
    /// already on, so it must be known to be off.
    fn check_can_move(&self) -> Result<(), String> {
        if self.job.is_some() || self.waiting_job.is_some() {
            return Err("A job is running.".into());
        }
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
                    if job.is_finished() {
                        self.emit_progress(JobState::Finished, None);
                        self.job = None;
                        // A Kerf job ends with M5, but the machine is asked
                        // rather than assumed.
                        self.spindle_mode = SpindleMode::Unknown;
                        self.manual_queue.push_back("$G".to_owned());
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
                self.abandon(
                    JobState::Failed,
                    Some(format!("Alarm {code}: {description}")),
                );
                self.emit(Event::Alarm { code, description });
            }
            Response::Status(status) => self.on_status(status)?,
            Response::Welcome(version) => {
                // The controller has restarted, so everything queued is gone.
                self.after_reset();
                self.abandon(JobState::Stopped, None);
                lock(&self.settings).clear();
                self.emit(Event::Welcome { version });
                self.manual_queue.push_back("$$".to_owned());
                self.manual_queue.push_back("$G".to_owned());
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
        self.emit(Event::Status {
            state: status.state,
            position,
            machine_position,
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
                        let position = if sim.report_work_position {
                            "WPos:4.000,5.000,6.000"
                        } else {
                            "MPos:5.000,6.000,7.000"
                        };
                        sim.say(&format!(
                            "<{state}|{position}{spindle}|WCO:1.000,1.000,1.000>"
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
                        sim.say("Grbl 1.1h ['$' for help]");
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
                                sim.say(&format!("[GC:G0 G54 G17 G21 G90 G94 {mode} M9 T0 F0 S0]"));
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
}
