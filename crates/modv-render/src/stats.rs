//! Frame-time statistics and process resource usage.
//!
//! `/proc` parsing is std-only (no `procfs`/`sysinfo`/`libc`). CPU ticks are
//! reported in USER_HZ units, which is 100 on Linux for the kernel's own
//! accounting — this is a documented assumption, not a detected value.

use std::time::{Duration, Instant};

/// `USER_HZ` on Linux: the clock tick rate the kernel uses for `utime`/`stime`
/// in `/proc/self/stat`. Assumed rather than probed (would need `libc`).
const TICKS_PER_SECOND: u64 = 100;

/// Collects per-frame timings and process CPU/RSS of the current process.
#[derive(Debug)]
pub struct FrameStats {
    start_wall: Instant,
    frames: Vec<Duration>,
    cpu_ticks_start: Option<u64>,
}

/// A snapshot of the collected statistics, produced by [`FrameStats::finish`].
#[derive(Clone, Debug)]
pub struct FrameReport {
    /// Number of frames recorded.
    pub frames: u64,
    /// Wall-clock seconds between `start()` and `finish()`.
    pub wall_seconds: f64,
    /// Frames per wall second.
    pub frames_per_second: f64,
    /// Mean frame time in milliseconds.
    pub mean_ms: f64,
    /// 50th percentile frame time in milliseconds.
    pub p50_ms: f64,
    /// 90th percentile frame time in milliseconds.
    pub p90_ms: f64,
    /// 99th percentile frame time in milliseconds.
    pub p99_ms: f64,
    /// Minimum frame time in milliseconds.
    pub min_ms: f64,
    /// Maximum frame time in milliseconds.
    pub max_ms: f64,
    /// Average CPU usage of the process over the window, in percent
    /// (`100 * cpu_seconds / wall_seconds` on a single core).
    pub cpu_percent: f64,
    /// Peak resident set size (`VmHWM`) of the process in mebibytes.
    pub rss_mb: f64,
}

impl FrameStats {
    /// Start collecting statistics. The CPU tick baseline is read from
    /// `/proc/self/stat`; if the file cannot be read, CPU usage is reported as 0.
    pub fn start() -> Self {
        Self {
            start_wall: Instant::now(),
            frames: Vec::new(),
            cpu_ticks_start: read_cpu_ticks().ok(),
        }
    }

    /// Record one rendered frame with the given frame time.
    pub fn record_frame(&mut self, dt: Duration) {
        self.frames.push(dt);
    }

    /// Stop collecting and compute the report.
    pub fn finish(self) -> FrameReport {
        let wall_seconds = self.start_wall.elapsed().as_secs_f64();
        let frames = self.frames.len() as u64;
        let mut sorted = self.frames.clone();
        sorted.sort();

        let total: Duration = sorted.iter().sum();
        let mean_ms = if sorted.is_empty() {
            0.0
        } else {
            total.as_secs_f64() * 1000.0 / sorted.len() as f64
        };
        let min_ms = sorted.first().map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let max_ms = sorted.last().map_or(0.0, |d| d.as_secs_f64() * 1000.0);

        let cpu_seconds = match (self.cpu_ticks_start, read_cpu_ticks().ok()) {
            (Some(start), Some(end)) => end.saturating_sub(start) as f64 / TICKS_PER_SECOND as f64,
            _ => 0.0,
        };
        let cpu_percent = if wall_seconds > 0.0 {
            100.0 * cpu_seconds / wall_seconds
        } else {
            0.0
        };

        let rss_mb = read_status_kb("VmHWM").map_or(0.0, |kb| kb as f64 / 1024.0);
        let frames_per_second = if wall_seconds > 0.0 {
            frames as f64 / wall_seconds
        } else {
            0.0
        };

        FrameReport {
            frames,
            wall_seconds,
            frames_per_second,
            mean_ms,
            p50_ms: percentile(&sorted, 50.0).as_secs_f64() * 1000.0,
            p90_ms: percentile(&sorted, 90.0).as_secs_f64() * 1000.0,
            p99_ms: percentile(&sorted, 99.0).as_secs_f64() * 1000.0,
            min_ms,
            max_ms,
            cpu_percent,
            rss_mb,
        }
    }
}

/// Percentile of a pre-sorted duration slice, using linear interpolation
/// between closest ranks (numpy "linear" method). Empty input yields zero.
pub fn percentile(sorted: &[Duration], pct: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let pct = pct.clamp(0.0, 100.0);
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = pct / 100.0 * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let weight = rank - lower as f64;
    let low = sorted[lower].as_secs_f64();
    let high = sorted[upper].as_secs_f64();
    Duration::from_secs_f64(low + (high - low) * weight)
}

/// Sum of `utime` + `stime` in USER_HZ ticks for the current process.
fn read_cpu_ticks() -> std::io::Result<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    parse_proc_stat_ticks(&stat).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed /proc/self/stat")
    })
}

/// Parse `utime` + `stime` ticks from the contents of `/proc/self/stat`.
///
/// The `comm` field (field 2) is wrapped in parentheses and may contain
/// spaces, so parsing starts after the trailing `')'`: the first token after
/// it is the state (field 3), and `utime` (field 14) / `stime` (field 15)
/// land at indices 11 and 12 of the remaining whitespace-separated tokens.
fn parse_proc_stat_ticks(content: &str) -> Option<u64> {
    let close = content.rfind(')')?;
    let rest = content.get(close + 1..)?.trim_start();
    let mut fields = rest.split_whitespace();
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(utime + stime)
}

/// Read a `Key: value kB` entry from `/proc/self/status` (e.g. `VmHWM`).
fn read_status_kb(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_status_kb(&status, key)
}

/// Parse a `Key: value kB` line from the contents of `/proc/self/status`.
/// Lines that do not parse are skipped rather than aborting the search.
fn parse_status_kb(content: &str, key: &str) -> Option<u64> {
    for line in content.lines() {
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(value) = rest.trim().strip_suffix("kB") else {
            continue;
        };
        if let Ok(parsed) = value.trim().parse() {
            return Some(parsed);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ms_close(expected: f64, actual: Duration) {
        let actual_ms = actual.as_secs_f64() * 1000.0;
        assert!(
            (expected - actual_ms).abs() < 0.001,
            "expected {expected} ms, got {actual_ms} ms"
        );
    }

    #[test]
    fn percentile_of_one_to_hundred() {
        let sorted: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        // numpy "linear": rank = pct/100 * 99.
        assert_ms_close(1.0, percentile(&sorted, 0.0));
        assert_ms_close(50.5, percentile(&sorted, 50.0));
        assert_ms_close(90.1, percentile(&sorted, 90.0));
        assert_ms_close(99.01, percentile(&sorted, 99.0));
        assert_ms_close(100.0, percentile(&sorted, 100.0));
    }

    #[test]
    fn percentile_empty_and_singleton() {
        assert_eq!(percentile(&[], 50.0), Duration::ZERO);
        assert_eq!(
            percentile(&[Duration::from_millis(7)], 99.0),
            Duration::from_millis(7)
        );
    }

    #[test]
    fn parse_proc_stat_ticks_ignores_spaces_in_comm() {
        // comm "modv worker" contains a space; fields after ')' start at state.
        let mut stat = String::from("1234 (modv worker) S");
        // ppid pgrp session tty_nr tpgid flags minflt cminflt majflt cmajflt + utime stime
        for tick in [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 42, 8] {
            stat.push_str(&format!(" {tick}"));
        }
        assert_eq!(parse_proc_stat_ticks(&stat), Some(50));
    }

    #[test]
    fn parse_status_kb_reads_vmhwm() {
        let status = "Name:\tmodv-engine\nVmRSS:\t   12345 kB\nVmHWM:\t   23456 kB\n";
        assert_eq!(parse_status_kb(status, "VmHWM"), Some(23456));
        assert_eq!(parse_status_kb(status, "VmRSS"), Some(12345));
        assert_eq!(parse_status_kb(status, "Nonexistent"), None);
    }

    #[test]
    fn proc_self_stat_is_readable() {
        match std::fs::read_to_string("/proc/self/stat") {
            Ok(stat) => assert!(parse_proc_stat_ticks(&stat).is_some()),
            Err(_) => eprintln!("skipping: /proc/self/stat not readable on this host"),
        }
    }
}
