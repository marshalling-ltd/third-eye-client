//! Records how long each step of starting the live stream takes, measured from
//! the moment the user connects: ffmpeg spawned, first byte on its stdout,
//! first JPEG decoded, first frame handed to the UI. Each step is recorded
//! once per connection, so a restart gets fresh numbers.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A step in stream startup, in the order they happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The ffmpeg child process was spawned.
    Spawned,
    /// The first bytes arrived on ffmpeg's stdout.
    FirstByte,
    /// The first JPEG was decoded into a frame.
    FirstJpeg,
    /// The first frame was handed to the UI.
    FirstShown,
}

impl Stage {
    const ALL: [Stage; 4] = [
        Stage::Spawned,
        Stage::FirstByte,
        Stage::FirstJpeg,
        Stage::FirstShown,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// Short label used in the log line.
    pub fn label(self) -> &'static str {
        match self {
            Stage::Spawned => "ffmpeg spawned",
            Stage::FirstByte => "first byte",
            Stage::FirstJpeg => "first JPEG",
            Stage::FirstShown => "first frame shown",
        }
    }
}

/// Timestamps for one stream start. Share it between threads in an `Arc`.
#[derive(Debug)]
pub struct StartupTimings {
    started: Instant,
    /// Microseconds since `started`, plus one so that zero means "not yet".
    marks: [AtomicU64; 4],
}

impl Default for StartupTimings {
    fn default() -> Self {
        Self::new()
    }
}

impl StartupTimings {
    /// Starts the clock now (the connect click).
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            marks: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    /// Records `stage` now. Only the first call per stage counts; returns
    /// whether this call was that first one.
    pub fn mark(&self, stage: Stage) -> bool {
        let micros = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX - 1);
        self.marks[stage.index()]
            .compare_exchange(0, micros + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Time from the connect click to `stage`, if it has happened.
    pub fn elapsed(&self, stage: Stage) -> Option<Duration> {
        match self.marks[stage.index()].load(Ordering::Acquire) {
            0 => None,
            value => Some(Duration::from_micros(value - 1)),
        }
    }

    /// One line with every recorded stage, e.g.
    /// `stream startup: ffmpeg spawned 12 ms, first byte 840 ms, ...`.
    pub fn summary(&self) -> String {
        let mut line = String::from("stream startup:");
        let mut first = true;
        for stage in Stage::ALL {
            if let Some(at) = self.elapsed(stage) {
                let _ = write!(
                    line,
                    "{} {} {} ms",
                    if first { "" } else { "," },
                    stage.label(),
                    at.as_millis()
                );
                first = false;
            }
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_unset_until_marked() {
        let timings = StartupTimings::new();
        assert!(timings.elapsed(Stage::FirstByte).is_none());
        assert_eq!(timings.summary(), "stream startup:");
    }

    #[test]
    fn only_the_first_mark_counts() {
        let timings = StartupTimings::new();
        assert!(timings.mark(Stage::Spawned));
        let first = timings.elapsed(Stage::Spawned).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(!timings.mark(Stage::Spawned));
        assert_eq!(timings.elapsed(Stage::Spawned), Some(first));
    }

    #[test]
    fn summary_lists_marked_stages_in_order() {
        let timings = StartupTimings::new();
        timings.mark(Stage::FirstJpeg);
        timings.mark(Stage::Spawned);
        let summary = timings.summary();
        let spawned = summary.find("ffmpeg spawned").unwrap();
        let jpeg = summary.find("first JPEG").unwrap();
        assert!(spawned < jpeg, "{summary}");
        assert!(!summary.contains("first byte"));
        assert!(summary.contains(", first JPEG"));
    }

    #[test]
    fn stages_have_distinct_labels() {
        let labels: Vec<_> = Stage::ALL.iter().map(|s| s.label()).collect();
        for (i, a) in labels.iter().enumerate() {
            assert!(!labels[i + 1..].contains(a));
        }
    }
}
