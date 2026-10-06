//! Build phases and progress reporting. Every report carries the overall
//! fraction for the task bar plus a structured detail (`phase`, `step`,
//! `total`, optional item and byte counts) for the build page.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;

/// Byte-level updates closer together than this are dropped.
const MIN_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Plan,
    OpenCore,
    Resources,
    Assemble,
    Kexts,
    Acpi,
    KernelPatches,
    Config,
    Save,
    Validate,
}

impl Phase {
    pub const ALL: [Phase; 10] = [
        Phase::Plan,
        Phase::OpenCore,
        Phase::Resources,
        Phase::Assemble,
        Phase::Kexts,
        Phase::Acpi,
        Phase::KernelPatches,
        Phase::Config,
        Phase::Save,
        Phase::Validate,
    ];

    /// Stable id sent to the frontend.
    pub fn id(self) -> &'static str {
        match self {
            Phase::Plan => "plan",
            Phase::OpenCore => "opencore",
            Phase::Resources => "resources",
            Phase::Assemble => "assemble",
            Phase::Kexts => "kexts",
            Phase::Acpi => "acpi",
            Phase::KernelPatches => "kernel-patches",
            Phase::Config => "config",
            Phase::Save => "save",
            Phase::Validate => "validate",
        }
    }

    /// 1-based position in [`Phase::ALL`].
    pub fn step(self) -> u32 {
        Phase::ALL.iter().position(|p| *p == self).map_or(0, |i| i as u32 + 1)
    }

    pub fn total() -> u32 {
        Phase::ALL.len() as u32
    }

    /// Share of the overall progress bar: (start, end). Downloads dominate.
    pub fn span(self) -> (f64, f64) {
        match self {
            Phase::Plan => (0.0, 0.03),
            Phase::OpenCore => (0.03, 0.25),
            Phase::Resources => (0.25, 0.37),
            Phase::Assemble => (0.37, 0.40),
            Phase::Kexts => (0.40, 0.76),
            Phase::Acpi => (0.76, 0.80),
            Phase::KernelPatches => (0.80, 0.83),
            Phase::Config => (0.83, 0.90),
            Phase::Save => (0.90, 0.92),
            Phase::Validate => (0.92, 1.0),
        }
    }

    /// Overall fraction for `within` (0..1) of this phase.
    pub fn at(self, within: f64) -> f64 {
        let (start, end) = self.span();
        start + (end - start) * within.clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildProgress {
    pub phase: Phase,
    /// Overall progress 0..1.
    pub fraction: f64,
    pub message: String,
    /// Component being worked on ("Lilu", "SSDT-PLUG.aml").
    pub item: Option<String>,
    /// Position of `item` in the phase (1-based) and the item count.
    pub index: Option<(usize, usize)>,
    /// Bytes downloaded / expected for the current item.
    pub bytes: Option<(u64, Option<u64>)>,
}

impl BuildProgress {
    /// `TaskUpdate.detail` payload.
    pub fn detail(&self) -> serde_json::Value {
        let mut detail = json!({
            "phase": self.phase.id(),
            "step": self.phase.step(),
            "total": Phase::total(),
        });
        if let Some(map) = detail.as_object_mut() {
            if let Some(item) = &self.item {
                map.insert("item".into(), json!(item));
            }
            if let Some((index, count)) = self.index {
                map.insert("index".into(), json!(index));
                map.insert("count".into(), json!(count));
            }
            if let Some((done, total)) = self.bytes {
                map.insert("downloaded".into(), json!(done));
                map.insert("size".into(), json!(total));
            }
        }
        detail
    }
}

pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: BuildProgress);
}

impl<F: Fn(BuildProgress) + Send + Sync> ProgressSink for F {
    fn report(&self, progress: BuildProgress) {
        self(progress)
    }
}

/// Sink that drops everything (tests, headless builds).
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn report(&self, _progress: BuildProgress) {}
}

/// Convenience wrapper that throttles byte-level updates.
pub struct Reporter<'a> {
    sink: &'a dyn ProgressSink,
    last: Mutex<Option<(Instant, Phase, Option<String>)>>,
}

impl<'a> Reporter<'a> {
    pub fn new(sink: &'a dyn ProgressSink) -> Self {
        Self { sink, last: Mutex::new(None) }
    }

    /// Start of a phase.
    pub fn phase(&self, phase: Phase, message: impl Into<String>) {
        self.send(BuildProgress {
            phase,
            fraction: phase.at(0.0),
            message: message.into(),
            item: None,
            index: None,
            bytes: None,
        });
    }

    /// Item `index` (0-based) of `count` within a phase.
    pub fn item(&self, phase: Phase, index: usize, count: usize, item: &str, message: impl Into<String>) {
        let within = if count == 0 { 0.0 } else { index as f64 / count as f64 };
        self.send(BuildProgress {
            phase,
            fraction: phase.at(within),
            message: message.into(),
            item: Some(item.to_string()),
            index: Some((index + 1, count)),
            bytes: None,
        });
    }

    /// Download progress of item `index` (0-based) of `count`.
    pub fn bytes(&self, phase: Phase, index: usize, count: usize, item: &str, done: u64, total: Option<u64>) {
        let finished = total.is_some_and(|t| done >= t);
        {
            let Ok(mut last) = self.last.lock() else { return };
            if let Some((at, p, i)) = last.as_ref() {
                let same = *p == phase && i.as_deref() == Some(item);
                if same && !finished && at.elapsed() < MIN_INTERVAL {
                    return;
                }
            }
            *last = Some((Instant::now(), phase, Some(item.to_string())));
        }
        let part = match total {
            Some(t) if t > 0 => (done as f64 / t as f64).min(1.0),
            _ => 0.0,
        };
        let count = count.max(1);
        let within = (index as f64 + part) / count as f64;
        let message = match total {
            Some(t) if t > 0 => format!("Downloading {item} ({} of {})", mib(done), mib(t)),
            _ => format!("Downloading {item} ({})", mib(done)),
        };
        self.sink.report(BuildProgress {
            phase,
            fraction: phase.at(within),
            message,
            item: Some(item.to_string()),
            index: Some((index + 1, count)),
            bytes: Some((done, total)),
        });
    }

    fn send(&self, progress: BuildProgress) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some((Instant::now(), progress.phase, progress.item.clone()));
        }
        self.sink.report(progress);
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_cover_the_bar_in_order() {
        let mut end = 0.0;
        for (i, phase) in Phase::ALL.iter().enumerate() {
            let (s, e) = phase.span();
            assert!((s - end).abs() < 1e-9, "{phase:?} starts at {s}, previous ended at {end}");
            assert!(e > s);
            assert_eq!(phase.step(), i as u32 + 1);
            end = e;
        }
        assert!((end - 1.0).abs() < 1e-9);
        assert_eq!(Phase::total(), 10);
    }

    #[test]
    fn detail_has_phase_step_total() {
        let p = BuildProgress {
            phase: Phase::Kexts,
            fraction: 0.5,
            message: String::new(),
            item: Some("Lilu".into()),
            index: Some((1, 4)),
            bytes: Some((10, Some(20))),
        };
        let d = p.detail();
        assert_eq!(d["phase"], "kexts");
        assert_eq!(d["step"], 5);
        assert_eq!(d["total"], 10);
        assert_eq!(d["item"], "Lilu");
        assert_eq!(d["index"], 1);
        assert_eq!(d["count"], 4);
        assert_eq!(d["downloaded"], 10);
        assert_eq!(d["size"], 20);
    }

    #[test]
    fn byte_updates_are_throttled() {
        let seen = Mutex::new(Vec::new());
        let sink = |p: BuildProgress| seen.lock().unwrap().push(p);
        let reporter = Reporter::new(&sink);
        reporter.phase(Phase::OpenCore, "start");
        for done in 0..100 {
            reporter.bytes(Phase::OpenCore, 0, 1, "OpenCore", done, Some(100));
        }
        reporter.bytes(Phase::OpenCore, 0, 1, "OpenCore", 100, Some(100));
        let seen = seen.into_inner().unwrap();
        // Phase start, the first byte update, and the completed transfer.
        assert!(seen.len() <= 4, "{} updates", seen.len());
        let last = seen.last().unwrap();
        assert!((last.fraction - Phase::OpenCore.at(1.0)).abs() < 1e-9);
        assert!(seen.windows(2).all(|w| w[0].fraction <= w[1].fraction));
    }
}
