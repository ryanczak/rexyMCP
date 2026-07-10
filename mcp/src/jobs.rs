use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use uuid::Uuid;

/// Bounded long-poll window for `get_run_status`. A poll that finds the run
/// still in flight returns `Running` after at most this long, so the caller
/// re-polls rather than blocking indefinitely.
pub const RUN_STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(15);

/// Terminal-or-running state of a spawned `execute_phase` run.
#[derive(Debug, Clone)]
pub enum RunState {
    /// Still executing.
    Running,
    /// Finished; holds the serialized (capped) `PhaseResult` JSON.
    Complete(serde_json::Value),
    /// Errored at the infrastructure level (config load / scope / IO).
    Failed(String),
}

impl RunState {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, RunState::Running)
    }
}

/// Per-run control block held in the registry.
struct RunEntry {
    state_tx: watch::Sender<RunState>,
}

/// In-memory registry of spawned `execute_phase` runs, keyed by `run_id`.
/// Lives for the serve-process lifetime on `RexyMcpServer.runs`.
#[derive(Default)]
pub struct JobRegistry {
    runs: Mutex<HashMap<String, RunEntry>>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh run in `Running`. Call before spawning so a racing
    /// `get_run_status` can always find the id.
    pub fn insert(&self, run_id: &str) {
        let (state_tx, _rx) = watch::channel(RunState::Running);
        self.lock()
            .insert(run_id.to_string(), RunEntry { state_tx });
    }

    /// Publish a terminal state. No-op if the id is unknown.
    pub fn publish(&self, run_id: &str, state: RunState) {
        if let Some(entry) = self.lock().get(run_id) {
            // send_replace stores the value even with no live receivers, so a
            // later subscriber still sees it via `borrow`.
            entry.state_tx.send_replace(state);
        }
    }

    /// Non-blocking snapshot. `None` = unknown id.
    #[allow(dead_code)]
    pub fn snapshot(&self, run_id: &str) -> Option<RunState> {
        self.lock().get(run_id).map(|e| e.state_tx.borrow().clone())
    }

    fn subscribe(&self, run_id: &str) -> Option<watch::Receiver<RunState>> {
        self.lock().get(run_id).map(|e| e.state_tx.subscribe())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunEntry>> {
        self.runs.lock().expect("jobs registry mutex poisoned")
    }

    /// Bounded long-poll: resolve as soon as the run is terminal, or return the
    /// current (still-`Running`) state after `timeout`. `None` = unknown id.
    pub async fn await_terminal(&self, run_id: &str, timeout: Duration) -> Option<RunState> {
        let mut rx = self.subscribe(run_id)?;
        {
            let cur = rx.borrow_and_update();
            if cur.is_terminal() {
                return Some(cur.clone());
            }
        }
        match tokio::time::timeout(timeout, rx.wait_for(|s| s.is_terminal())).await {
            Ok(Ok(guard)) => Some(guard.clone()),
            // sender dropped without ever going terminal — report as running.
            Ok(Err(_)) => Some(RunState::Running),
            // timed out — still running.
            Err(_) => Some(RunState::Running),
        }
    }
}

/// Fresh run id — a v4 UUID (collision-free across a serve process, unlike the
/// coarse epoch-seconds `generate_session_id`).
pub fn new_run_id() -> String {
    Uuid::new_v4().to_string()
}

/// Spawn `work` as run `run_id`, publishing its terminal state when it
/// finishes. Registers the run (`Running`) **synchronously** before returning,
/// so a `get_run_status` issued immediately after always finds it.
pub fn spawn_run<F>(registry: Arc<JobRegistry>, run_id: String, work: F)
where
    F: std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'static,
{
    registry.insert(&run_id);
    tokio::spawn(async move {
        let state = match work.await {
            Ok(json) => RunState::Complete(json),
            Err(e) => RunState::Failed(e),
        };
        registry.publish(&run_id, state);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_run_ids_are_unique() {
        let id1 = new_run_id();
        let id2 = new_run_id();
        assert_ne!(id1, id2, "run ids should differ");
        assert_eq!(
            id1.split('-').count(),
            5,
            "UUID should have four hyphens (5 segments)"
        );
        assert_eq!(
            id2.split('-').count(),
            5,
            "UUID should have four hyphens (5 segments)"
        );
    }

    #[test]
    fn snapshot_unknown_id_is_none() {
        let registry = JobRegistry::new();
        assert!(registry.snapshot("nonexistent").is_none());
    }

    #[test]
    fn insert_then_snapshot_is_running() {
        let registry = JobRegistry::new();
        registry.insert("r1");
        assert!(matches!(registry.snapshot("r1"), Some(RunState::Running)));
    }

    #[test]
    fn publish_sets_terminal_snapshot() {
        let registry = JobRegistry::new();
        registry.insert("r1");
        registry.publish("r1", RunState::Complete(json!({"status": "ok"})));
        assert!(matches!(
            registry.snapshot("r1"),
            Some(RunState::Complete(_))
        ));
    }

    #[tokio::test]
    async fn await_terminal_returns_immediately_when_already_terminal() {
        let registry = JobRegistry::new();
        registry.insert("r1");
        registry.publish("r1", RunState::Complete(json!({"status": "ok"})));
        let result = registry.await_terminal("r1", Duration::from_secs(60)).await;
        assert!(matches!(result, Some(RunState::Complete(_))));
    }

    #[tokio::test]
    async fn await_terminal_wakes_on_racing_publish() {
        let registry = Arc::new(JobRegistry::new());
        registry.insert("r1");

        let reg_clone = registry.clone();
        let waiter = tokio::spawn(async move {
            reg_clone
                .await_terminal("r1", Duration::from_secs(60))
                .await
        });

        registry.publish("r1", RunState::Complete(json!({"status": "complete"})));
        let result = waiter.await.unwrap();
        assert!(matches!(result, Some(RunState::Complete(_))));
    }

    #[tokio::test]
    async fn await_terminal_times_out_to_running() {
        let registry = JobRegistry::new();
        registry.insert("r1");
        let result = registry
            .await_terminal("r1", Duration::from_millis(1))
            .await;
        assert!(matches!(result, Some(RunState::Running)));
    }

    #[tokio::test]
    async fn await_terminal_unknown_id_is_none() {
        let registry = JobRegistry::new();
        let result = registry
            .await_terminal("nonexistent", Duration::from_secs(1))
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn spawn_run_publishes_complete() {
        let registry = Arc::new(JobRegistry::new());
        let run_id = new_run_id();
        spawn_run(registry.clone(), run_id.clone(), async {
            Ok(json!({"status": "complete"}))
        });
        let result = registry
            .await_terminal(&run_id, Duration::from_secs(60))
            .await;
        assert!(matches!(result, Some(RunState::Complete(_))));
    }

    #[tokio::test]
    async fn spawn_run_publishes_failed() {
        let registry = Arc::new(JobRegistry::new());
        let run_id = new_run_id();
        spawn_run(registry.clone(), run_id.clone(), async {
            Err("boom".into())
        });
        let result = registry
            .await_terminal(&run_id, Duration::from_secs(60))
            .await;
        assert!(matches!(result, Some(RunState::Failed(_))));
    }
}
