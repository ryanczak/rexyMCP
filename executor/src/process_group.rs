//! Unix process-group isolation for spawned shell commands. Children run in
//! their own process group (`process_group(0)`), so a timeout or a dropped
//! future can signal the whole tree (`kill -9 -- -pgid`) instead of orphaning
//! grandchildren like test binaries under `cargo test`.

use tokio::process::Command;

pub(crate) fn configure_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(not(unix))]
    let _ = cmd;
}

pub(crate) fn kill_process_group(pgid: u32) {
    if pgid == 0 {
        // `kill -0` targets the caller's own process group — signalling it
        // would kill the server / test runner itself.
        return;
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", "--", &format!("-{pgid}")])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(unix))]
    let _ = pgid;
}

pub(crate) struct ProcessGroupGuard {
    pgid: Option<u32>,
}

impl ProcessGroupGuard {
    pub(crate) fn new(pgid: Option<u32>) -> Self {
        Self {
            pgid: pgid.filter(|&id| id > 0),
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.pgid = None;
    }

    pub(crate) fn kill_now(&mut self) {
        if let Some(id) = self.pgid.take() {
            kill_process_group(id);
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.kill_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_process_group_zero_is_noop() {
        kill_process_group(0);
        drop(ProcessGroupGuard::new(Some(0)));
    }

    #[cfg(unix)]
    #[test]
    fn disarmed_guard_does_not_kill_process_group_on_drop() {
        use std::io::Read;
        use std::process::Stdio;
        use std::time::{Duration, Instant};

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut guard = ProcessGroupGuard::new(Some(pid));
        guard.disarm();
        drop(guard);

        std::thread::sleep(Duration::from_millis(100));
        let state = process_state(pid);
        assert!(
            state.as_deref().is_some_and(|s| !s.starts_with('Z')),
            "disarmed guard must not kill the child; state = {state:?}"
        );

        // Explicit cleanup so the test does not leak a 30s sleeper.
        let _ = libc_kill(pid);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let mut buf = String::new();
            if child
                .stdout
                .as_mut()
                .unwrap()
                .read_to_string(&mut buf)
                .is_ok()
                || child.try_wait().is_ok_and(|s| s.is_some())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    fn process_state(pid: u32) -> Option<String> {
        let out = std::process::Command::new("ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    #[cfg(unix)]
    fn libc_kill(pid: u32) -> std::io::Result<()> {
        // `kill` shell-out is fine here; SIGTERM first is unnecessary — the
        // child is ours and about to be reaped.
        let status = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("kill failed"))
        }
    }
}
