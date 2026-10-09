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

/// Commands whose output is data for the caller, not something a person watching the install wants to read.
const QUIET: &[&str] = &[
    "blkid",
    "lsblk",
    "lspci",
    "findmnt",
    "udevadm",
    "id",
    "cat",
    "uname",
    "getent",
    "sh",
    "sha256sum",
    "sha512sum",
    "nproc",
    "os-prober",
    "cpuid2cpuflags",
    "printf",
];
/// A single command may stream at most this many lines into the log (a runaway build must not drown the page).
const MAX_STREAMED_LINES: usize = 3000;

/// Like [`RealCommandRunner`], but every line a command prints also goes to the event stream as a log line, so a long step
/// (an emerge, a mkfs) is visible while it runs instead of only when it ends. Output is still returned to the caller.
pub struct StreamingCommandRunner {
    tx: crate::event::EventTx,
}

impl StreamingCommandRunner {
    pub fn new(tx: crate::event::EventTx) -> Self {
        Self { tx }
    }
}

/// Drops terminal escapes (emerge colours its output) and keeps the last `\r` segment (progress meters redraw one line).
pub fn clean_line(raw: &str) -> String {
    let last = raw
        .rsplit('\r')
        .find(|s| !s.trim().is_empty())
        .unwrap_or("");
    let mut out = String::with_capacity(last.len());
    let mut chars = last.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if !c.is_control() || c == '\t' {
            out.push(c);
        }
    }
    out.trim_end().to_string()
}

async fn pump<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    tx: crate::event::EventTx,
    stream: bool,
    level: crate::event::Level,
) -> Vec<u8> {
    use tokio::io::AsyncBufReadExt;
    let mut reader = tokio::io::BufReader::new(reader);
    let (mut all, mut line, mut sent) = (Vec::new(), Vec::new(), 0usize);
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        all.extend_from_slice(&line);
        if stream && sent < MAX_STREAMED_LINES {
            let text = clean_line(&String::from_utf8_lossy(&line));
            if !text.is_empty() {
                sent += 1;
                let _ = tx.send(crate::event::Event::Log {
                    line: text,
                    level: level.clone(),
                });
            }
        }
    }
    all
}

#[async_trait::async_trait]
impl CommandRunner for StreamingCommandRunner {
    async fn run(&self, cmd: &str, args: &[&str]) -> crate::Result<String> {
        use crate::event::{Event, Level};
        use std::process::Stdio;
        let shown = format!("{cmd} {}", args.join(" "));
        let stream = !QUIET.contains(&cmd);
        if stream {
            let short: String = shown.chars().take(160).collect();
            let _ = self.tx.send(Event::Log {
                line: format!("$ {short}"),
                level: Level::Info,
            });
        }
        let err = |detail: String| crate::Error::Command {
            cmd: shown.clone(),
            detail,
        };
        let mut child = tokio::process::Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| err(e.to_string()))?;
        let out = pump(
            child.stdout.take().expect("piped"),
            self.tx.clone(),
            stream,
            Level::Info,
        );
        let errs = pump(
            child.stderr.take().expect("piped"),
            self.tx.clone(),
            stream,
            Level::Info,
        );
        let (out, errs) = tokio::join!(out, errs);
        let status = child.wait().await.map_err(|e| err(e.to_string()))?;
        if !status.success() {
            return Err(err(String::from_utf8_lossy(&errs).trim().to_string()));
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
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
        self.responses
            .lock()
            .unwrap()
            .insert(cmd.to_string(), output.into());
    }

    /// Makes calls to `cmd` fail with the given error message instead of succeeding.
    pub fn fail(&self, cmd: &str, error: impl Into<String>) {
        self.failures
            .lock()
            .unwrap()
            .insert(cmd.to_string(), error.into());
    }

    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    /// Asserts the exact argv of the call at `index` — the pattern spec §10 wants for
    /// every phase test.
    pub fn assert_call(&self, index: usize, cmd: &str, args: &[&str]) {
        let calls = self.calls();
        let (actual_cmd, actual_args) = calls.get(index).unwrap_or_else(|| {
            panic!(
                "expected a call at index {index}, only {} recorded",
                calls.len()
            )
        });
        assert_eq!(actual_cmd, cmd, "command mismatch at call {index}");
        assert_eq!(actual_args, args, "argv mismatch at call {index}");
    }
}

#[async_trait::async_trait]
impl CommandRunner for FakeCommandRunner {
    async fn run(&self, cmd: &str, args: &[&str]) -> crate::Result<String> {
        self.calls.lock().unwrap().push((
            cmd.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));

        if let Some(error) = self.failures.lock().unwrap().get(cmd) {
            return Err(crate::Error::Command {
                cmd: format!("{cmd} {}", args.join(" ")),
                detail: error.clone(),
            });
        }

        Ok(self
            .responses
            .lock()
            .unwrap()
            .get(cmd)
            .cloned()
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use crate::event::Event;

    #[test]
    fn colours_and_carriage_returns_are_cleaned() {
        assert_eq!(
            clean_line("\x1b[32;01m>>>\x1b[39;49;00m Emerging (1 of 3)\n"),
            ">>> Emerging (1 of 3)"
        );
        assert_eq!(clean_line("10%\r50%\r100%\n"), "100%");
    }

    #[tokio::test]
    async fn output_is_streamed_and_still_returned() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = StreamingCommandRunner::new(tx);
        let out = runner
            .run("sh", &["-c", "printf 'a\\nb\\n'"])
            .await
            .unwrap();
        assert_eq!(out, "a\nb\n");
        assert!(
            rx.try_recv().is_err(),
            "sh is a data command: nothing is streamed"
        );
        let out = runner.run("seq", &["1", "2"]).await.unwrap();
        assert_eq!(out, "1\n2\n");
        let mut lines = Vec::new();
        while let Ok(Event::Log { line, .. }) = rx.try_recv() {
            lines.push(line);
        }
        assert_eq!(lines, ["$ seq 1 2", "1", "2"]);
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_stderr() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let e = StreamingCommandRunner::new(tx)
            .run("sh", &["-c", "echo boom >&2; exit 3"])
            .await
            .unwrap_err();
        assert!(e.to_string().contains("boom"), "{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_records_calls_in_order() {
        let runner = FakeCommandRunner::new();
        runner
            .run("parted", &["--script", "/dev/sda", "mklabel", "gpt"])
            .await
            .unwrap();
        runner
            .run("mkfs.vfat", &["-F32", "/dev/sda1"])
            .await
            .unwrap();

        runner.assert_call(0, "parted", &["--script", "/dev/sda", "mklabel", "gpt"]);
        runner.assert_call(1, "mkfs.vfat", &["-F32", "/dev/sda1"]);
    }

    #[tokio::test]
    async fn fake_returns_canned_response() {
        let runner = FakeCommandRunner::new();
        runner.respond("blkid", "ABCD-1234\n");
        let out = runner
            .run("blkid", &["-s", "UUID", "-o", "value", "/dev/sda1"])
            .await
            .unwrap();
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
