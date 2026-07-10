use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rexymcp_executor::phase::CancelReason;

use crate::jobs::JobRegistry;
use crate::stop;

use rexymcp_executor::agent::CancelHandle;

/// How often the serve-side watcher checks for `.rexymcp/stop`. Stop latency is
/// bounded by this (a human waits at most this long after `rexymcp stop`).
pub const STOP_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Poll `<repo>/.rexymcp/stop` while run `run_id` is live. On first sight of the
/// sentinel, fire **all** runs (`UserStop`) and clear the sentinel, then exit.
/// Also exits (without firing) once `run_id` goes terminal, so the task never
/// outlives its run. `poll` is injectable for tests.
pub async fn watch_stop_sentinel(
    repo_path: PathBuf,
    registry: Arc<JobRegistry>,
    run_id: String,
    poll: Duration,
) {
    loop {
        tokio::time::sleep(poll).await;
        if !registry.is_running(&run_id) {
            return; // run finished on its own — nothing to watch
        }
        if stop::sentinel_present(&repo_path) {
            registry.request_stop_all(CancelReason::UserStop);
            let _ = stop::clear_sentinel(&repo_path);
            return;
        }
    }
}

/// Sentinel watcher for the blocking CLI `run-phase`, which has no `JobRegistry`.
/// Polls `<repo>/.rexymcp/stop`; on first sight, fires this run's single
/// `cancel` handle (`UserStop` is stamped by the caller path) and clears the
/// sentinel, then exits. Unlike `watch_stop_sentinel` it has no run-liveness
/// check — the caller aborts this task once the run finishes.
pub async fn watch_stop_sentinel_single(repo_path: PathBuf, cancel: CancelHandle, poll: Duration) {
    loop {
        tokio::time::sleep(poll).await;
        if stop::sentinel_present(&repo_path) {
            cancel.cancel();
            let _ = stop::clear_sentinel(&repo_path);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobRegistry;
    use rexymcp_executor::agent::CancelSignal;
    use tempfile::TempDir;

    #[tokio::test]
    async fn watcher_fires_stop_all_and_clears_when_sentinel_present() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().to_path_buf();
        let registry = Arc::new(JobRegistry::new());
        let (handle, signal) = CancelSignal::new();
        registry.insert("r1", handle);

        // Write sentinel before the watcher starts
        stop::write_sentinel(&repo).unwrap();

        // Spawn the watcher with a tiny poll
        let watcher = tokio::spawn(watch_stop_sentinel(
            repo.clone(),
            registry.clone(),
            "r1".to_string(),
            Duration::from_millis(1),
        ));

        // Wait for the watcher to finish (should exit quickly)
        let _ = tokio::time::timeout(Duration::from_secs(5), watcher).await;

        // The signal should be cancelled
        assert!(signal.is_cancelled(), "signal should be cancelled");
        // The sentinel should be cleared
        assert!(!stop::sentinel_present(&repo), "sentinel should be cleared");
    }

    #[tokio::test]
    async fn watcher_exits_without_firing_when_run_terminal() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().to_path_buf();
        let registry = Arc::new(JobRegistry::new());
        let (handle, signal) = CancelSignal::new();
        registry.insert("r1", handle);

        // Immediately mark the run as terminal (before the watcher sees the sentinel)
        registry.publish(
            "r1",
            crate::jobs::RunState::Complete(serde_json::json!({"status": "ok"})),
        );

        // Spawn the watcher — it should exit because the run is already terminal
        let watcher = tokio::spawn(watch_stop_sentinel(
            repo.clone(),
            registry.clone(),
            "r1".to_string(),
            Duration::from_millis(1),
        ));

        tokio::time::timeout(Duration::from_secs(5), watcher)
            .await
            .expect("watcher should exit promptly once the run is terminal")
            .expect("watcher task should not panic");

        // Signal should NOT be cancelled (watcher exited before seeing sentinel)
        assert!(
            !signal.is_cancelled(),
            "signal should NOT be cancelled — watcher exited on terminal run"
        );
    }

    #[tokio::test]
    async fn single_watcher_fires_handle_and_clears_when_sentinel_present() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().to_path_buf();
        let (cancel_handle, cancel_signal) = CancelSignal::new();

        // Write sentinel before the watcher starts
        stop::write_sentinel(&repo).unwrap();

        let watcher = tokio::spawn(watch_stop_sentinel_single(
            repo.clone(),
            cancel_handle,
            Duration::from_millis(1),
        ));

        // Await the JoinHandle and assert it completed
        tokio::time::timeout(Duration::from_secs(5), watcher)
            .await
            .expect("watcher should complete within timeout")
            .expect("watcher task should complete successfully");

        // The signal should be cancelled
        assert!(cancel_signal.is_cancelled(), "signal should be cancelled");
        // The sentinel should be cleared
        assert!(!stop::sentinel_present(&repo), "sentinel should be cleared");
    }

    #[tokio::test]
    async fn single_watcher_does_not_fire_without_sentinel() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().to_path_buf();
        let (cancel_handle, cancel_signal) = CancelSignal::new();

        // No sentinel written yet — spawn the watcher
        let watcher = tokio::spawn(watch_stop_sentinel_single(
            repo.clone(),
            cancel_handle,
            Duration::from_millis(1),
        ));

        // Let a few polls pass (10ms on a 1ms poll)
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Signal should NOT be cancelled
        assert!(
            !cancel_signal.is_cancelled(),
            "signal should NOT be cancelled without a sentinel"
        );

        // Abort the watcher (it would loop forever without a sentinel)
        watcher.abort();
    }
}
