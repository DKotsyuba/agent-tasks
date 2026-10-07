//! Bounded local Git execution for the persistence engine.
//!
//! Every call uses an exact argument vector (paths always follow `--`, literal pathspec mode on),
//! removes inherited Git location and prompt variables, keeps the user's own configuration, hooks and
//! signing, disables lazy fetching and prompts, caps stdout and stderr while draining both so a full
//! pipe cannot stall the child, enforces one deadline and kills the child's process group on expiry.
//! Raw stderr is never returned to callers as text; they classify exit status and fixed markers.
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// Largest stdout kept from one command.
pub const STDOUT_CAP: usize = 1024 * 1024;
/// Largest stderr kept from one command.
pub const STDERR_CAP: usize = 64 * 1024;
/// Deadline for one ordinary command.
pub const COMMAND_TIME: Duration = Duration::from_secs(10);
/// Deadline for one whole settlement.
pub const SETTLEMENT_TIME: Duration = Duration::from_secs(30);

/// Captured result of one command that ran to completion.
pub struct Output {
    /// Exit code, or `None` when a signal ended the process.
    pub code: Option<i32>,
    /// Captured stdout, at most [`STDOUT_CAP`] bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr, at most [`STDERR_CAP`] bytes; only for fixed-marker classification.
    pub stderr: Vec<u8>,
    /// Whether either stream exceeded its cap and was cut.
    pub truncated: bool,
}

impl Output {
    /// Whether the process exited with status zero.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Stdout as UTF-8 text with surrounding whitespace removed; invalid bytes are replaced.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_owned()
    }
}

/// Why a command produced no usable result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    /// Git could not be started.
    Unavailable,
    /// The deadline passed; the process group was killed and the outcome of any write is unknown.
    Timeout,
    /// Waiting for or reading the process failed.
    Io,
}

#[cfg(test)]
std::thread_local! {
    /// Test-only environment applied to every command on this thread (isolated Git configuration).
    static TEST_ENVIRONMENT: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Install the test-only environment for commands run on this thread.
#[cfg(test)]
pub fn set_test_environment(variables: Vec<(String, String)>) {
    TEST_ENVIRONMENT.with(|env| *env.borrow_mut() = variables);
}

/// Drain `source` into a buffer of at most `cap` bytes, discarding the rest; returns bytes and a cut flag.
fn capture(mut source: impl Read, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut cut = false;
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                if n > room {
                    cut = true;
                }
                kept.extend_from_slice(&buffer[..n.min(room)]);
            }
        }
    }
    (kept, cut)
}

/// Run `git` with `args` inside `root`, optionally feeding `stdin`, until `deadline`.
///
/// The environment keeps the user's configuration and hooks. Inherited `GIT_*` variables are removed,
/// prompts, editors and lazy fetching are disabled, and `LC_ALL=C` fixes message markers.
///
/// # Errors
/// [`RunError::Unavailable`] when Git cannot start, [`RunError::Timeout`] after killing the process
/// group at the deadline, and [`RunError::Io`] for pipe or wait failures.
pub fn run(
    root: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    deadline: Instant,
) -> Result<Output, RunError> {
    run_env(root, args, stdin, deadline, &[])
}

/// Like [`run`] with extra environment variables, such as `GIT_INDEX_FILE` for a temporary index.
///
/// # Errors
/// The same as [`run`].
pub fn run_env(
    root: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    deadline: Instant,
    extra: &[(&str, &str)],
) -> Result<Output, RunError> {
    if Instant::now() >= deadline {
        return Err(RunError::Timeout);
    }
    let mut command = Command::new("git");
    command.current_dir(root).arg("--no-pager");
    // `check-ignore` takes plain paths and rejects pathspec magic outright.
    if args.first() != Some(&"check-ignore") {
        command.arg("--literal-pathspecs");
    }
    command
        .arg("--no-replace-objects")
        .args(["-c", "core.quotePath=false", "-c", "core.fsmonitor=false"])
        .args(args);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|n| n.starts_with("GIT_")) {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_EDITOR", ":")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in extra {
        command.env(name, value);
    }
    #[cfg(test)]
    TEST_ENVIRONMENT.with(|env| {
        for (name, value) in env.borrow().iter() {
            command.env(name, value);
        }
    });
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|_| RunError::Unavailable)?;
    let pid = child.id();
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let owned = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&owned);
        });
    }
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(RunError::Io);
    };
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = out_tx.send(capture(out, STDOUT_CAP));
    });
    std::thread::spawn(move || {
        let _ = err_tx.send(capture(err, STDERR_CAP));
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                kill_group(pid);
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Io);
            }
        }
    };
    let (stdout, cut_out) = out_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| RunError::Io)?;
    let (stderr, cut_err) = err_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| RunError::Io)?;
    Ok(Output {
        code: status.code(),
        stdout,
        stderr,
        truncated: cut_out || cut_err,
    })
}

/// Kill the whole process group of a timed-out command so hook children do not outlive it.
#[cfg(unix)]
fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Without process groups only the direct child is killed by the caller.
#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

/// Run a read-only command and return trimmed stdout when it succeeds.
pub fn read_text(root: &Path, args: &[&str], deadline: Instant) -> Option<String> {
    run(root, args, None, deadline)
        .ok()
        .filter(Output::success)
        .map(|o| o.text())
}
