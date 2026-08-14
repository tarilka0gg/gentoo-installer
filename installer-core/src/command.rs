//! `CommandRunner` trait + real/fake implementations — the seam that makes every phase
//! testable on a non-Gentoo machine with no root and no real disks (spec §10). Every
//! module that used to shell out via `tokio::process::Command` directly now takes
//! `&dyn CommandRunner` and goes through this instead; tests assert exact argv against
//! `FakeCommandRunner` rather than mocking individual functions.

use std::collections::HashMap;
use std::sync::Mutex;

#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, cmd: &str, args: &[&str]) -> crate::Result<String>;

    /// Convenience for callers that don't care about stdout.
    async fn run_status(&self, cmd: &str, args: &[&str]) -> crate::Result<()> {
        self.run(cmd, args).await.map(|_| ())
    }
}

pub struct RealCommandRunner;

#[async_trait::async_trait]
impl CommandRunner for RealCommandRunner {
    async fn run(&self, cmd: &str, args: &[&str]) -> crate::Result<String> {
        let output = tokio::process::Command::new(cmd)
            .args(args)
            .output()
            .await
            .map_err(|e| crate::Error::Command {
                cmd: format!("{cmd} {}", args.join(" ")),
                detail: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(crate::Error::Command {
                cmd: format!("{cmd} {}", args.join(" ")),
                detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Records every invocation (for argv assertions) and returns a canned response per
/// command name (for parsers that need real-shaped output, e.g. `blkid`/`lsblk`).
/// Responses default to `""` — most commands in this codebase are run for their side
/// effect and only checked for success, per `CommandRunner::run_status`.
#[derive(Default)]
pub struct FakeCommandRunner {
    calls: Mutex<Vec<(String, Vec<String>)>>,
    responses: Mutex<HashMap<String, String>>,
    failures: Mutex<HashMap<String, String>>,
}

impl FakeCommandRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next (and all subsequent) calls to `cmd` return `output` instead of "".
    pub fn respond(&self, cmd: &str, output: impl Into<String>) {
        self.responses.lock().unwrap().insert(cmd.to_string(), output.into());
    }

    /// Makes calls to `cmd` fail with the given error message instead of succeeding.
    pub fn fail(&self, cmd: &str, error: impl Into<String>) {
        self.failures.lock().unwrap().insert(cmd.to_string(), error.into());
    }

    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    /// Asserts the exact argv of the call at `index` — the pattern spec §10 wants for
    /// every phase test.
    pub fn assert_call(&self, index: usize, cmd: &str, args: &[&str]) {
        let calls = self.calls();
        let (actual_cmd, actual_args) = calls
            .get(index)
            .unwrap_or_else(|| panic!("expected a call at index {index}, only {} recorded", calls.len()));
        assert_eq!(actual_cmd, cmd, "command mismatch at call {index}");
        assert_eq!(actual_args, args, "argv mismatch at call {index}");
    }
}

#[async_trait::async_trait]
impl CommandRunner for FakeCommandRunner {
    async fn run(&self, cmd: &str, args: &[&str]) -> crate::Result<String> {
        self.calls
            .lock()
            .unwrap()
            .push((cmd.to_string(), args.iter().map(|s| s.to_string()).collect()));

        if let Some(error) = self.failures.lock().unwrap().get(cmd) {
            return Err(crate::Error::Command {
                cmd: format!("{cmd} {}", args.join(" ")),
                detail: error.clone(),
            });
        }

        Ok(self.responses.lock().unwrap().get(cmd).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_records_calls_in_order() {
        let runner = FakeCommandRunner::new();
        runner.run("parted", &["--script", "/dev/sda", "mklabel", "gpt"]).await.unwrap();
        runner.run("mkfs.vfat", &["-F32", "/dev/sda1"]).await.unwrap();

        runner.assert_call(0, "parted", &["--script", "/dev/sda", "mklabel", "gpt"]);
        runner.assert_call(1, "mkfs.vfat", &["-F32", "/dev/sda1"]);
    }

    #[tokio::test]
    async fn fake_returns_canned_response() {
        let runner = FakeCommandRunner::new();
        runner.respond("blkid", "ABCD-1234\n");
        let out = runner.run("blkid", &["-s", "UUID", "-o", "value", "/dev/sda1"]).await.unwrap();
        assert_eq!(out.trim(), "ABCD-1234");
    }

    #[tokio::test]
    async fn fake_can_simulate_failure() {
        let runner = FakeCommandRunner::new();
        runner.fail("mkfs.btrfs", "device or resource busy");
        let err = runner.run("mkfs.btrfs", &["/dev/sda3"]).await.unwrap_err();
        assert!(err.to_string().contains("device or resource busy"));
    }
}
