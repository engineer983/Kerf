//! Streaming a program with GRBL's character-counting protocol.
//!
//! GRBL has a 127-byte serial receive buffer. Rather than wait for an `ok`
//! after every line, the sender keeps count of the bytes GRBL has not yet
//! acknowledged and sends the next line whenever it is certain to fit. This
//! keeps the planner fed, which matters for laser engraving with many short moves.
//!
//! [`Streamer`] is only the bookkeeping; it does no I/O, so it can be tested
//! without a machine.

use std::collections::VecDeque;

/// Size of GRBL's serial receive buffer in bytes.
pub const RX_BUFFER_BYTES: usize = 127;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    /// Index into the cleaned program, starting at 0.
    pub line_index: usize,
    pub line: String,
    pub code: u32,
}

#[derive(Debug)]
pub struct Streamer {
    lines: Vec<String>,
    next: usize,
    /// Byte lengths, including the newline, of lines sent but not yet acknowledged.
    in_flight: VecDeque<usize>,
    acknowledged: usize,
    error: Option<StreamError>,
}

impl Streamer {
    /// Strips comments and blank lines; GRBL would only discard them, and they
    /// would use up buffer space on the way.
    pub fn new(program: &str) -> Result<Self, String> {
        let mut lines = Vec::new();
        for (number, raw) in program.lines().enumerate() {
            let line = strip_comments(raw);
            if line.is_empty() {
                continue;
            }
            if line.len() + 1 > RX_BUFFER_BYTES {
                return Err(format!(
                    "Line {} is {} characters long; GRBL accepts at most {}.",
                    number + 1,
                    line.len(),
                    RX_BUFFER_BYTES - 1
                ));
            }
            lines.push(line);
        }
        Ok(Self {
            lines,
            next: 0,
            in_flight: VecDeque::new(),
            acknowledged: 0,
            error: None,
        })
    }

    /// The next line to write, without its newline, if GRBL has room for it.
    /// Returns `None` when the buffer is full, the program is finished, or an
    /// error has stopped the stream.
    pub fn next_line(&mut self) -> Option<&str> {
        if self.error.is_some() {
            return None;
        }
        let line = self.lines.get(self.next)?;
        let bytes = line.len() + 1;
        if self.in_flight.iter().sum::<usize>() + bytes > RX_BUFFER_BYTES {
            return None;
        }
        self.in_flight.push_back(bytes);
        self.next += 1;
        Some(line)
    }

    /// Call for every `ok` from GRBL.
    pub fn on_ok(&mut self) {
        if self.in_flight.pop_front().is_some() {
            self.acknowledged += 1;
        }
    }

    /// Call for every `error:n` from GRBL. The stream stops: lines already in
    /// GRBL's buffer will still run, but nothing further is sent.
    pub fn on_error(&mut self, code: u32) {
        if self.in_flight.pop_front().is_some() {
            let line_index = self.acknowledged;
            self.acknowledged += 1;
            self.error.get_or_insert(StreamError {
                line_index,
                line: self.lines[line_index].clone(),
                code,
            });
        }
    }

    pub fn error(&self) -> Option<&StreamError> {
        self.error.as_ref()
    }

    /// Lines acknowledged so far, and the total.
    pub fn progress(&self) -> (usize, usize) {
        (self.acknowledged, self.lines.len())
    }

    /// True once every line has been sent and acknowledged. GRBL may still be
    /// executing moves from its planner at this point.
    pub fn is_finished(&self) -> bool {
        self.error.is_none() && self.acknowledged == self.lines.len()
    }
}

/// Removes `;` and `( )` comments and surrounding whitespace.
fn strip_comments(line: &str) -> String {
    let line = line.split(';').next().unwrap_or("");
    let mut out = String::with_capacity(line.len());
    let mut depth = 0u32;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for GRBL's receive buffer and fails the test if it overflows.
    struct MockGrbl {
        buffered: VecDeque<String>,
        bytes: usize,
        received: Vec<String>,
    }

    impl MockGrbl {
        fn new() -> Self {
            Self {
                buffered: VecDeque::new(),
                bytes: 0,
                received: Vec::new(),
            }
        }

        fn receive(&mut self, line: &str) {
            self.bytes += line.len() + 1;
            assert!(
                self.bytes <= RX_BUFFER_BYTES,
                "receive buffer overflowed at {line:?}"
            );
            self.buffered.push_back(line.to_owned());
        }

        /// Processes one buffered line, freeing its space.
        fn process(&mut self) -> Option<String> {
            let line = self.buffered.pop_front()?;
            self.bytes -= line.len() + 1;
            self.received.push(line.clone());
            Some(line)
        }
    }

    fn program(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("G1 X{}.{:03} Y{} S500 F3000\n", i, i * 7 % 1000, i * 3))
            .collect()
    }

    #[test]
    fn comments_and_blank_lines_are_dropped() {
        let mut s = Streamer::new("; header\nG21 ; mm\n\n(setup) G90\n  \nM5\n").unwrap();
        assert_eq!(s.progress(), (0, 3));
        assert_eq!(s.next_line(), Some("G21"));
        assert_eq!(s.next_line(), Some("G90"));
        assert_eq!(s.next_line(), Some("M5"));
        assert_eq!(s.next_line(), None);
    }

    #[test]
    fn never_overflows_the_receive_buffer() {
        let text = program(500);
        let mut streamer = Streamer::new(&text).unwrap();
        let mut grbl = MockGrbl::new();
        let mut most_in_flight = 0;

        while !streamer.is_finished() {
            while let Some(line) = streamer.next_line() {
                grbl.receive(line);
            }
            most_in_flight = most_in_flight.max(grbl.buffered.len());
            grbl.process()
                .expect("the sender stalled with an empty buffer");
            streamer.on_ok();
        }

        assert_eq!(grbl.received.len(), 500);
        assert_eq!(grbl.received, text.lines().collect::<Vec<_>>());
        // Several lines were queued at once, so this is not plain send-and-wait.
        assert!(
            most_in_flight > 2,
            "only {most_in_flight} lines were ever in flight"
        );
        assert_eq!(streamer.progress(), (500, 500));
    }

    #[test]
    fn an_error_stops_the_stream_and_names_the_line() {
        let mut streamer = Streamer::new("G21\nG90\nBAD\nG1 X1\nG1 X2\n").unwrap();
        while streamer.next_line().is_some() {}
        streamer.on_ok();
        streamer.on_ok();
        streamer.on_error(20);
        streamer.on_ok();

        let error = streamer.error().unwrap();
        assert_eq!(
            (error.line_index, error.line.as_str(), error.code),
            (2, "BAD", 20)
        );
        assert_eq!(streamer.next_line(), None);
        assert!(!streamer.is_finished());
    }

    #[test]
    fn nothing_more_is_sent_after_an_error() {
        let text = program(50);
        let mut streamer = Streamer::new(&text).unwrap();
        let mut sent = 0;
        while streamer.next_line().is_some() {
            sent += 1;
        }
        streamer.on_error(9);
        assert!(streamer.next_line().is_none());
        assert!(sent < 50);
    }

    #[test]
    fn overlong_lines_are_refused_up_front() {
        let long = format!("G1 X{}", "1".repeat(130));
        let message = Streamer::new(&format!("G21\n{long}\n")).unwrap_err();
        assert!(message.contains("Line 2"));
    }

    #[test]
    fn stray_acknowledgements_are_ignored() {
        let mut streamer = Streamer::new("G21\n").unwrap();
        streamer.on_ok();
        assert_eq!(streamer.progress(), (0, 1));
        assert_eq!(streamer.next_line(), Some("G21"));
        streamer.on_ok();
        assert!(streamer.is_finished());
    }
}
