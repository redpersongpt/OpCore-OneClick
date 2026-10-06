//! Build pipeline behaviour that needs no network: failed or cancelled
//! builds leave nothing behind in `builds/`.

use std::path::PathBuf;
use std::sync::Mutex;

use app_lib::build::progress::BuildProgress;
use app_lib::build::{self, BuildEnv};
use app_lib::domain::model::{BuildOptions, HardwareProfile, MacOsVersion};
use app_lib::domain::planner::empty_plan;
use app_lib::services::http::Downloader;
use app_lib::tasks::cancellation::CancellationToken;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("oneclick-pipeline-offline-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn cancelled_build_leaves_no_directory() {
    let tmp = TempDir::new();
    let builds = tmp.0.join("builds");
    std::fs::create_dir_all(builds.join(".staging-20200101-000000-00000000/EFI/OC")).unwrap();
    let downloader = Downloader::new(tmp.0.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let seen = Mutex::new(Vec::<BuildProgress>::new());
    let sink = |p: BuildProgress| seen.lock().unwrap().push(p);
    let env = BuildEnv {
        builds_dir: &builds,
        work_dir: &tmp.0.join("work"),
        downloader: &downloader,
        cancel: &cancel,
        progress: &sink,
        keep_builds: 5,
    };
    let mut plan = empty_plan(MacOsVersion::Sequoia);
    plan.smbios.model = "iMac19,1".into();
    let err = build::run(&env, &HardwareProfile::default(), &BuildOptions::default(), plan).await.unwrap_err();
    assert_eq!(err.code, "TASK_CANCELLED");
    assert!(entries(&builds).is_empty(), "{:?}", entries(&builds));
}

#[tokio::test]
async fn incomplete_plan_is_refused_before_anything_is_written() {
    let tmp = TempDir::new();
    let builds = tmp.0.join("builds");
    let downloader = Downloader::new(tmp.0.join("cache")).unwrap();
    let cancel = CancellationToken::new();
    let env = BuildEnv {
        builds_dir: &builds,
        work_dir: &tmp.0.join("work"),
        downloader: &downloader,
        cancel: &cancel,
        progress: &build::progress::NoProgress,
        keep_builds: 5,
    };
    let plan = empty_plan(MacOsVersion::Sequoia);
    let err = build::run(&env, &HardwareProfile::default(), &BuildOptions::default(), plan).await.unwrap_err();
    assert_eq!(err.code, "PLAN_INCOMPLETE");
    assert!(entries(&builds).is_empty());
}
