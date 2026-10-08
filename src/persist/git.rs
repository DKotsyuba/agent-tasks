//! Bounded local Git execution for the persistence engine.
//!
//! Every call uses an exact argument vector (paths always follow `--`, literal pathspec mode on),
//! removes inherited Git location and prompt variables, keeps the user's own configuration, hooks and
//! signing, disables lazy fetching and prompts, caps stdout and stderr while draining both so a full
//! pipe cannot stall the child, enforces one deadline and kills the child's process group on expiry.
//! Raw stderr is never returned to callers as text; they classify exit status and fixed markers.
use sha2::{Digest, Sha256};
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
/// Public maximum for settlement and reply preparation. Callers reserve half [`COMMAND_TIME`]
/// for the wire reply; post-write verification stays inside the remaining Git deadline.
pub const SETTLEMENT_TIME: Duration = Duration::from_secs(30);

/// Captured result of one command that ran to completion.
pub struct Output {
    /// Exit code, or `None` when a signal ended the process.
    pub code: Option<i32>,
    /// Captured stdout, at most [`STDOUT_CAP`] bytes.
    pub stdout: Vec<u8>,
    /// Private fingerprint of the complete stdout stream, including bytes beyond the retained cap.
    /// `None` means the reader failed; it never certifies a prefix as a complete snapshot.
    pub stdout_digest: Option<[u8; 32]>,
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

/// Drain `source`, retaining at most `cap` bytes and hashing the entire stream with fixed-size state.
///
/// Returns the retained prefix, its truncation flag and the complete SHA-256; a read failure makes
/// the digest absent. Callers never display the digest or source bytes as diagnostic text. The
/// command's shared deadline bounds capture; excess bytes increase neither retained memory nor hash state.
fn capture(mut source: impl Read, cap: usize) -> (Vec<u8>, bool, Option<[u8; 32]>) {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut cut = false;
    let mut digest = Sha256::new();
    loop {
        match source.read(&mut buffer) {
            Ok(0) => return (kept, cut, Some(digest.finalize().into())),
            Err(_) => return (kept, cut, None),
            Ok(n) => {
                digest.update(&buffer[..n]);
                let room = cap.saturating_sub(kept.len());
                if n > room {
                    cut = true;
                }
                kept.extend_from_slice(&buffer[..n.min(room)]);
            }
        }
    }
}

/// Prefix truncation preserves the complete private digest; a real reader failure supplies no digest.
#[cfg(all(test, unix))]
#[test]
#[allow(
    clippy::unwrap_used,
    reason = "Isolated reader assertions over disposable data"
)]
fn capture_digest_is_complete_only_after_eof() {
    let bytes = vec![b'x'; STDOUT_CAP + 1];
    let (kept, truncated, digest) = capture(std::io::Cursor::new(&bytes), 16);
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    assert_eq!(kept.len(), 16);
    assert!(truncated);
    assert!(
        digest == Some(expected),
        "the fingerprint covers bytes beyond the retained prefix"
    );
    let directory = tempfile::tempdir().unwrap();
    let (_, _, failed) = capture(std::fs::File::open(directory.path()).unwrap(), 16);
    assert!(
        failed.is_none(),
        "a read failure cannot certify a partial stream"
    );
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

/// Like [`run`] with `extra` environment overrides, such as `GIT_INDEX_FILE` for a temporary index.
///
/// The deadline covers process execution and draining both capped output streams. A hook descendant
/// retaining a pipe is killed with the process group when capture times out, even if Git has exited.
/// No stream gets more than five seconds or the remaining command budget, whichever is shorter.
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
    // One command has COMMAND_TIME inside the caller's overall (settlement) deadline.
    let deadline = deadline.min(Instant::now() + COMMAND_TIME);
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
    let ((stdout, cut_out, stdout_digest), (stderr, cut_err, _)) = out_rx
        .recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(5)),
        )
        .and_then(|out| {
            err_rx
                .recv_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(5)),
                )
                .map(|err| (out, err))
        })
        .map_err(|_| {
            kill_group(pid);
            if Instant::now() >= deadline {
                RunError::Timeout
            } else {
                RunError::Io
            }
        })?;
    Ok(Output {
        code: status.code(),
        stdout,
        stdout_digest,
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
