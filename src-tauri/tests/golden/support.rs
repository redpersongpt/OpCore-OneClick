//! Fixture loading, the per-release runs every golden check reads, and the
//! failure list the checks report into.

use std::fmt;
use std::path::{Path, PathBuf};

use app_lib::contracts::{CompatibilityReport, SupportLevel};
use app_lib::domain::compatibility;
use app_lib::domain::model::{BuildOptions, BuildPlan, HardwareProfile, MacOsVersion};
use app_lib::domain::planner;
use app_lib::domain::profile;
use app_lib::error::AppError;

pub fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

pub struct Fixture {
    /// File stem of `fixtures/profiles/<name>.json`.
    pub name: String,
    pub profile: HardwareProfile,
}

/// Every profile fixture, sorted by name.
pub fn fixtures() -> Vec<Fixture> {
    let dir = tests_dir().join("fixtures").join("profiles");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let profile =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            Fixture {
                name: path.file_stem().unwrap().to_string_lossy().into_owned(),
                profile,
            }
        })
        .collect()
}

/// What the compatibility report says about one release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `MacOsOption::supported`.
    Supported,
    /// Not supported, but the report for this release is `Partial`: an
    /// expert option (root patches, CryptexFixup, telemetrap).
    Expert,
    Unsupported,
    /// The report cannot decide (CPU platform or AMD core count unknown).
    Unknown,
}

impl Verdict {
    pub fn buildable(self) -> bool {
        matches!(self, Verdict::Supported | Verdict::Expert)
    }

    pub fn label(self) -> &'static str {
        match self {
            Verdict::Supported => "supported",
            Verdict::Expert => "expert",
            Verdict::Unsupported => "unsupported",
            Verdict::Unknown => "unknown",
        }
    }
}

pub struct TargetRun {
    pub target: MacOsVersion,
    pub verdict: Verdict,
    pub report: CompatibilityReport,
    pub plan: Result<BuildPlan, AppError>,
}

pub struct FixtureRun {
    pub name: String,
    /// The fixture after `profile::refresh_profile`.
    pub profile: HardwareProfile,
    /// `compatibility::assess(profile, None)`.
    pub overview: CompatibilityReport,
    pub targets: Vec<TargetRun>,
}

impl FixtureRun {
    pub fn target(&self, version: MacOsVersion) -> &TargetRun {
        self.targets
            .iter()
            .find(|t| t.target == version)
            .expect("every release is run")
    }

    pub fn buildable(&self) -> bool {
        self.targets.iter().any(|t| t.plan.is_ok())
    }
}

/// The options the app starts a build with, for `target`.
pub fn options(target: MacOsVersion) -> BuildOptions {
    BuildOptions {
        target,
        ..BuildOptions::default()
    }
}

pub fn run(fixture: &Fixture) -> FixtureRun {
    let profile = profile::refresh_profile(fixture.profile.clone());
    let overview = compatibility::assess(&profile, None);
    let targets = MacOsVersion::ALL
        .into_iter()
        .map(|target| {
            let report = compatibility::assess(&profile, Some(target));
            let verdict = verdict(&report, target);
            let plan = planner::plan(&profile, &options(target));
            TargetRun {
                target,
                verdict,
                report,
                plan,
            }
        })
        .collect();
    FixtureRun {
        name: fixture.name.clone(),
        profile,
        overview,
        targets,
    }
}

fn verdict(report: &CompatibilityReport, target: MacOsVersion) -> Verdict {
    let supported = report
        .versions
        .iter()
        .any(|o| o.version == target && o.supported);
    match report.level {
        SupportLevel::Unknown => Verdict::Unknown,
        _ if supported => Verdict::Supported,
        SupportLevel::Partial => Verdict::Expert,
        _ => Verdict::Unsupported,
    }
}

// ── Failures ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Failure {
    pub fixture: String,
    pub target: Option<MacOsVersion>,
    pub check: &'static str,
    pub detail: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = self.target.map_or("*", MacOsVersion::id);
        write!(
            f,
            "{} @ {target} [{}]: {}",
            self.fixture, self.check, self.detail
        )
    }
}

#[derive(Default)]
pub struct Failures(pub Vec<Failure>);

impl Failures {
    pub fn push(
        &mut self,
        fixture: &str,
        target: Option<MacOsVersion>,
        check: &'static str,
        detail: impl Into<String>,
    ) {
        self.0.push(Failure {
            fixture: fixture.to_string(),
            target,
            check,
            detail: detail.into(),
        });
    }
}

/// One entry of the known-failure list: `fixture` is a fixture name or "*"
/// (a defect every machine shares), `target` a release id or "*".
#[derive(Debug, Clone, Copy)]
pub struct Known {
    pub fixture: &'static str,
    pub target: &'static str,
    pub check: &'static str,
    /// Text the failure detail must contain ("" takes any detail), so an
    /// entry does not hide a different defect behind the same check.
    pub detail: &'static str,
    /// What has to change before the entry can go.
    pub todo: &'static str,
}

impl Known {
    /// Every failure `later` matches is also matched by this entry, so
    /// `later` can never take one when it comes after this entry.
    pub fn shadows(&self, later: &Known) -> bool {
        self.check == later.check
            && (self.fixture == "*" || self.fixture == later.fixture)
            && (self.target == "*" || self.target == later.target)
            && later.detail.contains(self.detail)
    }

    fn matches(&self, failure: &Failure) -> bool {
        (self.fixture == "*" || self.fixture == failure.fixture)
            && self.check == failure.check
            && (self.target == "*" || failure.target.is_some_and(|t| t.id() == self.target))
            && failure.detail.contains(self.detail)
    }
}

/// Fail on every failure the known list does not cover, and on known entries
/// of `checks` that no longer fail (so the list only ever shrinks).
pub fn settle(failures: Failures, known: &[Known], checks: &[&str]) {
    let mut unexpected = Vec::new();
    let mut used = vec![false; known.len()];
    for failure in &failures.0 {
        match known.iter().position(|k| k.matches(failure)) {
            Some(i) => used[i] = true,
            None => unexpected.push(failure.to_string()),
        }
    }
    let stale: Vec<String> = known
        .iter()
        .zip(&used)
        .filter(|(k, used)| !**used && checks.contains(&k.check))
        .map(|(k, _)| format!("{} @ {} [{}]", k.fixture, k.target, k.check))
        .collect();
    for (k, _) in known.iter().zip(&used).filter(|(_, used)| **used) {
        let hits = failures.0.iter().filter(|f| k.matches(f)).count();
        eprintln!(
            "known failure {} @ {} [{}] ({hits}x): TODO {}",
            k.fixture, k.target, k.check, k.todo
        );
    }
    assert!(
        unexpected.is_empty() && stale.is_empty(),
        "{} unexpected failure(s):\n{}\n\n{} known failure entr(y/ies) no longer fail; remove them from KNOWN_FAILURES:\n{}",
        unexpected.len(),
        unexpected.join("\n"),
        stale.len(),
        stale.join("\n"),
    );
}
