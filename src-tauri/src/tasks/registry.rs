use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

use crate::contracts::{TaskStatus, TaskUpdate};
use crate::tasks::cancellation::CancellationToken;

/// Finished tasks kept for `task_list` before the oldest are dropped.
const MAX_FINISHED_TASKS: usize = 50;

/// Tracks all active async operations with progress, cancellation, and watchdog.
///
/// Terminal states (completed, failed, cancelled) are final: later calls to
/// `update_progress`, `complete`, `fail` or `cancel` for the same task are
/// ignored, so a watchdog failure or a user cancel can never be turned back
/// into a success.
pub struct TaskRegistry {
    tasks: RwLock<HashMap<String, TaskState>>,
    tokens: RwLock<HashMap<String, CancellationToken>>,
    app: AppHandle,
}

struct TaskState {
    task_id: String,
    kind: String,
    status: TaskStatus,
    progress: Option<f64>,
    message: Option<String>,
    detail: Option<serde_json::Value>,
    /// False while a step that must not be interrupted is running (disk writes).
    cancellable: bool,
    created: Instant,
    last_update: Instant,
}

impl TaskState {
    fn is_running(&self) -> bool {
        matches!(self.status, TaskStatus::Running)
    }

    fn to_update(&self) -> TaskUpdate {
        TaskUpdate {
            task_id: self.task_id.clone(),
            kind: self.kind.clone(),
            status: self.status,
            progress: self.progress,
            message: self.message.clone(),
            detail: self.detail.clone(),
        }
    }
}

/// How long a running task may go without a progress update before the
/// watchdog fails it. Long downloads report progress continuously, so these
/// only catch genuinely hung operations.
fn stall_threshold(kind: &str) -> Duration {
    match kind {
        "usb-flash" => Duration::from_secs(30 * 60),
        "efi-build" => Duration::from_secs(10 * 60),
        "recovery-download" => Duration::from_secs(5 * 60),
        "hardware-scan" => Duration::from_secs(3 * 60),
        _ => Duration::from_secs(5 * 60),
    }
}

impl TaskRegistry {
    pub fn new(app: AppHandle) -> Arc<Self> {
        let registry = Arc::new(Self {
            tasks: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
            app,
        });

        // Spawn watchdog using tauri's async runtime (not bare tokio::spawn,
        // because setup() may run before the tokio reactor is active).
        let watchdog = Arc::clone(&registry);
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                watchdog.check_stalled().await;
            }
        });

        registry
    }

    /// Create a new tracked task and return its cancellation token.
    pub async fn create(&self, kind: &str) -> (String, CancellationToken) {
        let task_id = uuid::Uuid::new_v4().to_string();
        let token = CancellationToken::new();
        let now = Instant::now();

        let state = TaskState {
            task_id: task_id.clone(),
            kind: kind.to_string(),
            status: TaskStatus::Running,
            progress: Some(0.0),
            message: None,
            detail: None,
            cancellable: true,
            created: now,
            last_update: now,
        };

        {
            let mut tasks = self.tasks.write().await;
            prune_finished(&mut tasks);
            tasks.insert(task_id.clone(), state);
        }
        self.tokens.write().await.insert(task_id.clone(), token.clone());

        self.emit_update(&task_id).await;
        (task_id, token)
    }

    /// Update task progress (0.0 - 1.0) with optional message.
    pub async fn update_progress(&self, task_id: &str, progress: f64, message: Option<String>) {
        self.update(task_id, |state| {
            state.progress = Some(progress.clamp(0.0, 1.0));
            state.message = message;
        })
        .await;
    }

    /// Update progress together with a structured detail payload (e.g. the
    /// current build phase) that the frontend can render without parsing text.
    pub async fn update_detail(
        &self,
        task_id: &str,
        progress: f64,
        message: Option<String>,
        detail: serde_json::Value,
    ) {
        self.update(task_id, |state| {
            state.progress = Some(progress.clamp(0.0, 1.0));
            state.message = message;
            state.detail = Some(detail);
        })
        .await;
    }

    /// Mark whether the running step may be cancelled. Disk writes turn this
    /// off so a cancel request cannot leave a half-written disk marked as
    /// cancelled while the work continues.
    pub async fn set_cancellable(&self, task_id: &str, cancellable: bool) {
        self.update(task_id, |state| state.cancellable = cancellable).await;
    }

    /// Mark task as completed.
    pub async fn complete(&self, task_id: &str) {
        self.finish(task_id, TaskStatus::Completed, None).await;
    }

    /// Mark task as failed.
    pub async fn fail(&self, task_id: &str, error: &str) {
        self.finish(task_id, TaskStatus::Failed, Some(error.to_string())).await;
    }

    /// Cancel a running task. Returns false when the task is unknown, already
    /// finished, or currently in a step that cannot be interrupted.
    pub async fn cancel(&self, task_id: &str) -> bool {
        {
            let tasks = self.tasks.read().await;
            match tasks.get(task_id) {
                Some(state) if state.is_running() && state.cancellable => {}
                _ => return false,
            }
        }
        if let Some(token) = self.tokens.read().await.get(task_id) {
            token.cancel();
        }
        self.finish(task_id, TaskStatus::Cancelled, Some("Cancelled by user".into())).await;
        true
    }

    /// List all current tasks, oldest first.
    pub async fn list(&self) -> Vec<TaskUpdate> {
        let tasks = self.tasks.read().await;
        let mut states: Vec<&TaskState> = tasks.values().collect();
        states.sort_by_key(|s| s.created);
        states.into_iter().map(TaskState::to_update).collect()
    }

    /// Apply a change to a running task and emit the update. No-op for
    /// finished or unknown tasks.
    async fn update(&self, task_id: &str, change: impl FnOnce(&mut TaskState)) {
        let changed = {
            let mut tasks = self.tasks.write().await;
            match tasks.get_mut(task_id) {
                Some(state) if state.is_running() => {
                    change(state);
                    state.last_update = Instant::now();
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit_update(task_id).await;
        }
    }

    async fn finish(&self, task_id: &str, status: TaskStatus, message: Option<String>) {
        let changed = {
            let mut tasks = self.tasks.write().await;
            match tasks.get_mut(task_id) {
                Some(state) if state.is_running() => {
                    if status == TaskStatus::Completed {
                        state.progress = Some(1.0);
                    }
                    if message.is_some() {
                        state.message = message;
                    }
                    state.status = status;
                    state.last_update = Instant::now();
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit_update(task_id).await;
            self.tokens.write().await.remove(task_id);
        }
    }

    /// Watchdog: fail tasks that stopped reporting progress. The token is
    /// cancelled first so the worker stops instead of finishing later.
    async fn check_stalled(&self) {
        let now = Instant::now();
        let stalled: Vec<String> = {
            let tasks = self.tasks.read().await;
            tasks
                .values()
                .filter(|s| s.is_running() && s.cancellable)
                .filter(|s| now.duration_since(s.last_update) > stall_threshold(&s.kind))
                .map(|s| s.task_id.clone())
                .collect()
        };

        for task_id in stalled {
            log::warn!("Task {task_id} stalled, marking as failed");
            if let Some(token) = self.tokens.read().await.get(&task_id) {
                token.cancel();
            }
            self.finish(
                &task_id,
                TaskStatus::Failed,
                Some("The operation stopped responding (no progress for too long).".into()),
            )
            .await;
        }
    }

    /// Emit a task:update event to the frontend.
    async fn emit_update(&self, task_id: &str) {
        let update = {
            let tasks = self.tasks.read().await;
            tasks.get(task_id).map(TaskState::to_update)
        };
        if let Some(update) = update {
            let _ = self.app.emit("task:update", &update);
        }
    }
}

fn prune_finished(tasks: &mut HashMap<String, TaskState>) {
    let mut finished: Vec<(Instant, String)> = tasks
        .values()
        .filter(|s| !s.is_running())
        .map(|s| (s.last_update, s.task_id.clone()))
        .collect();
    if finished.len() <= MAX_FINISHED_TASKS {
        return;
    }
    finished.sort();
    let excess = finished.len() - MAX_FINISHED_TASKS;
    for (_, id) in finished.into_iter().take(excess) {
        tasks.remove(&id);
    }
}
