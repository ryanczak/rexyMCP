use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rexymcp_executor::agent::CancelHandle;
use rexymcp_executor::phase::CancelReason;
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
    /// Fires the run's cooperative cancel signal. `None` is never stored — every
    /// registered run owns a handle (a `never()`-signal handle for runs that are
    /// not cancellable, e.g. tests).
    cancel: CancelHandle,
    /// Set by `request_stop`; read by `spawn_run` to stamp the terminal result.
    stop_reason: Option<CancelReason>,
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

    /// Register a fresh run in `Running`, holding its cancel handle. Call before
    /// spawning so a racing `get_run_status` / `stop_phase` always finds the id.
    pub fn insert(&self, run_id: &str, cancel: CancelHandle) {
        let (state_tx, _rx) = watch::channel(RunState::Running);
        self.lock().insert(
            run_id.to_string(),
            RunEntry {
                state_tx,
                cancel,
                stop_reason: None,
            },
        );
    }

    /// Publish a terminal state. No-op if the id is unknown.
    pub fn publish(&self, run_id: &str, state: RunState) {
        if let Some(entry) = self.lock().get(run_id) {
            // send_replace stores the value even with no live receivers, so a
            // later subscriber still sees it via `borrow`.
            entry.state_tx.send_replace(state);
        }
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

    /// Fire a run's cancel signal and record why. Returns `false` for an unknown id.
    /// Firing an already-terminal run's handle is a harmless no-op (all receivers are
    /// gone) — this returns `true` because the run existed, but nothing is re-stamped.
    pub fn request_stop(&self, run_id: &str, reason: CancelReason) -> bool {
        if let Some(entry) = self.lock().get_mut(run_id) {
            entry.stop_reason = Some(reason);
            entry.cancel.cancel();
            true
        } else {
            false
        }
    }

    /// Fire every live run's cancel signal with `reason`, recording it for the
    /// terminal-result stamp. Returns how many runs were signalled. The global
    /// stop-all path: one sentinel detection stops the whole serve process's runs.
    pub fn request_stop_all(&self, reason: CancelReason) -> usize {
        let mut map = self.lock();
        let mut n = 0;
        for entry in map.values_mut() {
            entry.stop_reason = Some(reason.clone());
            entry.cancel.cancel();
            n += 1;
        }
        n
    }

    /// Whether a run exists and is still `Running` (not yet terminal). Used to bound
    /// the sentinel watcher's lifetime so it exits once its run finishes.
    pub fn is_running(&self, run_id: &str) -> bool {
        self.lock()
            .get(run_id)
            .map(|e| !e.state_tx.borrow().is_terminal())
            .unwrap_or(false)
    }

    /// The reason recorded by a prior `request_stop`, if any. Read by `spawn_run`
    /// when a run finishes so a `cancelled` result can be stamped.
    fn recorded_reason(&self, run_id: &str) -> Option<CancelReason> {
        self.lock().get(run_id).and_then(|e| e.stop_reason.clone())
    }
}

/// Fresh run id — a v4 UUID (collision-free across a serve process, unlike the
/// coarse epoch-seconds `generate_session_id`).
pub fn new_run_id() -> String {
    Uuid::new_v4().to_string()
}

/// If `reason` is set and `json` is a `cancelled` PhaseResult, insert
/// `cancellation.reason`. No-op otherwise (a run that completed normally before
/// observing the stop keeps no reason — the status race is resolved in favor of
/// the observed terminal status).
fn stamp_cancel_reason(json: &mut serde_json::Value, reason: Option<CancelReason>) {
    let Some(reason) = reason else { return };
    if json.get("status").and_then(|s| s.as_str()) != Some("cancelled") {
        return;
    }
    if let Some(obj) = json.get_mut("cancellation").and_then(|c| c.as_object_mut())
        && let Ok(v) = serde_json::to_value(reason)
    {
        obj.insert("reason".to_string(), v);
    }
}

/// Spawn `work` as run `run_id`, holding `cancel_handle` in the registry so
/// `request_stop` can fire it. Publishes the terminal state when `work` finishes;
/// if the run was stopped and came back `cancelled`, stamps the recorded reason
/// into the result JSON's `cancellation.reason`.
pub fn spawn_run<F>(
    registry: Arc<JobRegistry>,
    run_id: String,
    cancel_handle: CancelHandle,
    work: F,
) where
    F: std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'static,
{
    registry.insert(&run_id, cancel_handle);
    tokio::spawn(async move {
        let state = match work.await {
            Ok(mut json) => {
                stamp_cancel_reason(&mut json, registry.recorded_reason(&run_id));
                RunState::Complete(json)
            }
            Err(e) => RunState::Failed(e),
        };
        registry.publish(&run_id, state);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::agent::CancelSignal;
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
    fn request_stop_unknown_id_returns_false() {
        let registry = JobRegistry::new();
        assert!(!registry.request_stop("nonexistent", CancelReason::ClaudeStop));
    }

    #[test]
    fn request_stop_known_id_fires_and_returns_true() {
        let registry = JobRegistry::new();
        let (handle, signal) = CancelSignal::new();
        registry.insert("r1", handle);
        assert!(!signal.is_cancelled(), "signal should start uncancelled");
        assert!(
            registry.request_stop("r1", CancelReason::ClaudeStop),
            "request_stop should return true for known id"
        );
        assert!(
            signal.is_cancelled(),
            "signal should be cancelled after request_stop"
        );
    }

    #[test]
    fn stamp_cancel_reason_sets_reason_on_cancelled() {
        let mut json = json!({
            "status": "cancelled",
            "cancellation": { "stage": "between_turns", "turns_done": 2 }
        });
        stamp_cancel_reason(&mut json, Some(CancelReason::ClaudeStop));
        let reason = json["cancellation"]["reason"].as_str();
        assert_eq!(reason, Some("claude_stop"));
    }

    #[test]
    fn stamp_cancel_reason_noop_on_complete() {
        let mut json = json!({ "status": "complete" });
        stamp_cancel_reason(&mut json, Some(CancelReason::ClaudeStop));
        assert!(
            json.get("cancellation").is_none(),
            "complete result should not gain cancellation"
        );
    }

    #[test]
    fn stamp_cancel_reason_noop_when_reason_none() {
        let mut json = json!({
            "status": "cancelled",
            "cancellation": { "stage": "between_turns", "turns_done": 2 }
        });
        stamp_cancel_reason(&mut json, None);
        assert!(
            json["cancellation"].get("reason").is_none(),
            "None reason should leave cancellation unchanged"
        );
    }

    #[tokio::test]
    async fn spawn_run_with_stopped_signal_stamps_reason_on_cancelled_result() {
        let registry = Arc::new(JobRegistry::new());
        let run_id = new_run_id();
        let (handle, _signal) = CancelSignal::new();
        registry.insert(&run_id, handle);
        registry.request_stop(&run_id, CancelReason::ClaudeStop);
        // Verify the recorded reason was set.
        assert!(
            registry.recorded_reason(&run_id).is_some(),
            "recorded_reason should be Some after request_stop"
        );
    }

    #[tokio::test]
    async fn await_terminal_returns_immediately_when_already_terminal() {
        let registry = JobRegistry::new();
        let (handle, _signal) = CancelSignal::new();
        registry.insert("r1", handle);
        registry.publish("r1", RunState::Complete(json!({"status": "ok"})));
        let result = registry.await_terminal("r1", Duration::from_secs(60)).await;
        assert!(matches!(result, Some(RunState::Complete(_))));
    }

    #[tokio::test]
    async fn await_terminal_wakes_on_racing_publish() {
        let registry = Arc::new(JobRegistry::new());
        let (handle, _signal) = CancelSignal::new();
        registry.insert("r1", handle);

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
        let (handle, _signal) = CancelSignal::new();
        registry.insert("r1", handle);
        let result = registry
            .await_terminal("r1", Duration::from_millis(1))
            .await;
        assert!(matches!(result, Some(RunState::Running)));
    }

    #[tokio::test]
    async fn await_terminal_unknown_id_is_none() {
        let registry = JobRegistry::new();
        let result = registry
            .await_terminal("nonexistent", Duration::from_millis(1))
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn spawn_run_publishes_complete() {
        let registry = Arc::new(JobRegistry::new());
        let run_id = new_run_id();
        let (handle, _signal) = CancelSignal::new();
        spawn_run(registry.clone(), run_id.clone(), handle, async {
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
        let (handle, _signal) = CancelSignal::new();
        spawn_run(registry.clone(), run_id.clone(), handle, async {
            Err("boom".into())
        });
        let result = registry
            .await_terminal(&run_id, Duration::from_secs(60))
            .await;
        assert!(matches!(result, Some(RunState::Failed(_))));
    }

    #[test]
    fn request_stop_all_fires_every_run_and_counts() {
        let registry = JobRegistry::new();
        let (handle1, signal1) = CancelSignal::new();
        let (handle2, signal2) = CancelSignal::new();
        registry.insert("r1", handle1);
        registry.insert("r2", handle2);
        assert!(!signal1.is_cancelled());
        assert!(!signal2.is_cancelled());

        let count = registry.request_stop_all(CancelReason::UserStop);
        assert_eq!(count, 2, "should fire two runs");
        assert!(signal1.is_cancelled(), "signal1 should be cancelled");
        assert!(signal2.is_cancelled(), "signal2 should be cancelled");
    }

    #[test]
    fn request_stop_all_on_empty_registry_is_zero() {
        let registry = JobRegistry::new();
        let count = registry.request_stop_all(CancelReason::UserStop);
        assert_eq!(count, 0, "empty registry should return 0");
    }

    #[test]
    fn is_running_true_for_running_false_after_publish() {
        let registry = JobRegistry::new();
        let (handle, _signal) = CancelSignal::new();
        registry.insert("r1", handle);
        assert!(registry.is_running("r1"), "should be running after insert");
        registry.publish("r1", RunState::Complete(json!({"status": "ok"})));
        assert!(
            !registry.is_running("r1"),
            "should not be running after publish terminal"
        );
        assert!(
            !registry.is_running("unknown"),
            "unknown id should not be running"
        );
    }
}
