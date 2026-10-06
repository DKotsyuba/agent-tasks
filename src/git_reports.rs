//! Read bounded reports from explicit local Git commits without mutating Git or deciding completion.
use crate::{
    model::{self, CheckInput, CheckStatus},
    store::{Error, Result},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// Maximum retained original commit message, including its subject and line endings.
const MESSAGE_CAP: usize = 8 * 1024;
/// Maximum combined Git output across repository discovery and eight commit observations.
const TOTAL_CAP: usize = 80 * 1024;
/// One cumulative wall-clock budget for all commands in a report import.
const READ_BUDGET: Duration = Duration::from_secs(5);

/// Observed local commit provenance plus the author's standardized report assertions.
/// Importing this value proves neither execution of checks nor independent acceptance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservedCommit {
    /// Canonical common Git directory, at most 1024 UTF-8 bytes, shared by worktrees for SHA deduplication.
    pub repository: String,
    /// Canonical declared checkout, at most 1024 UTF-8 bytes; no checkout is created or changed.
    pub worktree: String,
    /// Resolved full lowercase forty- or sixty-four-character commit object ID; abbreviated inputs are not retained.
    pub sha: String,
    /// Actual Git author name and email, at most 256 UTF-8 bytes; metadata, not authenticated identity.
    pub author: String,
    /// Actual Git author date normalized to UTC RFC 3339 seconds, not import time.
    pub authored_at: String,
    /// Actual commit subject, at most 256 UTF-8 bytes.
    pub subject: String,
    /// Original observed UTF-8 commit message, at most 8 KiB, including subject and line endings.
    pub message: String,
    /// Meaningful Result section, at most 1024 UTF-8 bytes; it is an author assertion.
    pub summary: String,
    /// Up to eight uniquely labeled reported checks; absence means no checks were reported.
    pub checks: Vec<CheckInput>,
    /// Up to eight reported in-scope gaps, each at most 256 UTF-8 bytes.
    pub gaps: Vec<String>,
    /// Up to eight reported out-of-scope followups, each at most 256 UTF-8 bytes.
    pub followups: Vec<String>,
}

/// Parsed bounded report sections, kept separate from observed Git provenance.
struct ParsedReport {
    /// Required meaningful Result content without the section label.
    summary: String,
    /// Explicit reported status for each unique check label.
    checks: Vec<CheckInput>,
    /// Current reported missing work.
    gaps: Vec<String>,
    /// Reported work outside the current scope.
    followups: Vec<String>,
}

/// Observe commit reports only after verifying declared repository/worktree identity and branch.
/// Absolute local paths, the actual short branch and one to eight hex selectors follow the same
/// restrictions as standalone reads. Preflight and every commit command share ONE five-second
/// deadline and ONE 80-KiB output budget. Any source/report error returns no importable observations;
/// the helper never writes Git/tracker data or decides Task completion.
pub fn read_verified_commits(
    repository: &Path,
    worktree: &Path,
    branch: &str,
    commits: &[String],
) -> Result<Vec<ObservedCommit>> {
    let mut remaining = TOTAL_CAP;
    read_verified_commits_until(
        repository,
        worktree,
        branch,
        commits,
        Instant::now() + READ_BUDGET,
        &mut remaining,
    )
}

/// Coordinate preflight and observations under caller-owned budgets, including short test deadlines.
fn read_verified_commits_until(
    repository: &Path,
    worktree: &Path,
    branch: &str,
    commits: &[String],
    deadline: Instant,
    remaining: &mut usize,
) -> Result<Vec<ObservedCommit>> {
    verify_execution_until(repository, worktree, branch, deadline, remaining)?;
    read_commits_until(worktree, commits, deadline, remaining)
}

/// Read one to eight hexadecimal commit selectors from an absolute, existing checkout root.
/// Selectors contain 7–64 hex digits and must resolve unambiguously to commits in this checkout.
/// All Git commands are read-only argv calls with inherited Git overrides removed, bounded output
/// before parsing and one five-second deadline. Any invalid source/report refuses the whole read;
/// this function writes no tracker state and never marks a Task done. Duplicate resolved SHAs are
/// returned only once in first-requested order. Original messages remain bounded UTF-8 evidence.
#[cfg(test)]
pub fn read_commits(repository: &Path, commits: &[String]) -> Result<Vec<ObservedCommit>> {
    let mut remaining = TOTAL_CAP;
    read_commits_until(
        repository,
        commits,
        Instant::now() + READ_BUDGET,
        &mut remaining,
    )
}

/// Read commits with an existing caller deadline and output budget; never restart either budget.
fn read_commits_until(
    repository: &Path,
    commits: &[String],
    deadline: Instant,
    remaining: &mut usize,
) -> Result<Vec<ObservedCommit>> {
    if commits.is_empty() || commits.len() > 8 {
        return Err(Error::new(
            "invalid_arguments",
            "Supply one to eight commit hashes.",
        ));
    }
    for selector in commits {
        if !(7..=64).contains(&selector.len()) || !selector.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::new(
                "invalid_arguments",
                "Commit selectors must be 7–64 hexadecimal digits, without revisions or options.",
            ));
        }
    }
    if !repository.is_absolute() {
        return Err(Error::new(
            "git_source",
            "Declare an absolute local repository/worktree root.",
        ));
    }
    let root = std::fs::canonicalize(repository)
        .map_err(|_| Error::new("git_source", "Declared repository/worktree is unavailable."))?;
    if !root.is_dir() {
        return Err(Error::new(
            "git_source",
            "Declared repository/worktree must be a directory.",
        ));
    }
    let top = git(
        &root,
        &["rev-parse", "--show-toplevel"],
        4096,
        deadline,
        remaining,
    )?;
    let top = std::fs::canonicalize(top.trim_end_matches('\n'))
        .map_err(|_| Error::new("git_source", "Cannot resolve the checkout root."))?;
    if top != root {
        return Err(Error::new(
            "git_source",
            "Declared path must be the checkout root, not an unrelated parent or nested directory.",
        ));
    }
    let common = git(
        &root,
        &["rev-parse", "--git-common-dir"],
        4096,
        deadline,
        remaining,
    )?;
    let common = Path::new(common.trim_end_matches('\n'));
    let common = std::fs::canonicalize(if common.is_absolute() {
        common.to_path_buf()
    } else {
        root.join(common)
    })
    .map_err(|_| {
        Error::new(
            "git_source",
            "Cannot resolve the common Git repository identity.",
        )
    })?;
    let repository = path_text(&common)?;
    let worktree = path_text(&root)?;
    let mut seen = BTreeSet::new();
    let mut observed = Vec::new();
    for selector in commits {
        let object_selector = format!("--disambiguate={}", selector.to_ascii_lowercase());
        let matches = git(
            &root,
            &["rev-parse", &object_selector],
            256,
            deadline,
            remaining,
        )?;
        let matches: Vec<_> = matches.lines().collect();
        if matches.len() != 1 {
            return Err(Error::new(
                "git_source",
                "Commit hash must identify exactly one local object; use its full SHA.",
            ));
        }
        let sha = matches[0];
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::new(
                "git_source",
                "The source must resolve to a full SHA-1 or SHA-256 commit object ID.",
            ));
        }
        let kind = git(&root, &["cat-file", "-t", sha], 32, deadline, remaining)?;
        if kind.trim_end_matches('\n') != "commit" {
            return Err(Error::new(
                "git_source",
                "Commit selectors must identify commit objects, not tags, trees or blobs.",
            ));
        }
        if !seen.insert(sha.to_owned()) {
            continue;
        }
        let value = git(
            &root,
            &[
                "show",
                "--no-patch",
                "--no-show-signature",
                "--no-ext-diff",
                "--encoding=none",
                "--format=format:%H%x00%an <%ae>%x00%aI%x00%s%x00%B",
                sha,
                "--",
            ],
            MESSAGE_CAP + 2048,
            deadline,
            remaining,
        )?;
        let parts: Vec<_> = value.splitn(5, '\0').collect();
        if parts.len() != 5 || parts[0] != sha || parts[4].contains('\0') {
            return Err(Error::new(
                "git_source",
                "Git returned malformed commit metadata.",
            ));
        }
        model::text(parts[1], 256).map_err(report_error)?;
        model::text(parts[3], 256).map_err(report_error)?;
        model::text(parts[4], MESSAGE_CAP).map_err(report_error)?;
        let authored_at = chrono::DateTime::parse_from_rfc3339(parts[2])
            .map_err(|_| Error::new("git_source", "Git returned an invalid author date."))?
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let report = parse_report(parts[4])?;
        observed.push(ObservedCommit {
            repository: repository.clone(),
            worktree: worktree.clone(),
            sha: sha.into(),
            author: parts[1].into(),
            authored_at,
            subject: parts[3].into(),
            message: parts[4].into(),
            summary: report.summary,
            checks: report.checks,
            gaps: report.gaps,
            followups: report.followups,
        });
    }
    Ok(observed)
}

/// Verify that declared local repository/worktree identities share one Git common directory and branch.
/// Both paths must be absolute existing directories, the worktree must be an actual checkout root,
/// and `branch` is its exact short symbolic branch name (detached HEAD refuses). Read-only Git calls
/// share one five-second deadline and bounded output; mismatches return safe source errors without
/// modifying Git or trusting inherited Git overrides. Core performs this check only on explicit import.
#[cfg(test)]
pub fn verify_execution(repository: &Path, worktree: &Path, branch: &str) -> Result<()> {
    let mut remaining = TOTAL_CAP;
    verify_execution_until(
        repository,
        worktree,
        branch,
        Instant::now() + READ_BUDGET,
        &mut remaining,
    )
}

/// Verify execution with the import caller's existing deadline/output budget; no reset is permitted.
fn verify_execution_until(
    repository: &Path,
    worktree: &Path,
    branch: &str,
    deadline: Instant,
    remaining: &mut usize,
) -> Result<()> {
    model::text(branch, 256).map_err(report_error)?;
    if branch.contains(['\n', '\r']) || !repository.is_absolute() || !worktree.is_absolute() {
        return Err(Error::new(
            "git_source",
            "Declare absolute local execution paths and an exact short branch name.",
        ));
    }
    let repository = std::fs::canonicalize(repository)
        .map_err(|_| Error::new("git_source", "Declared repository is unavailable."))?;
    let worktree = std::fs::canonicalize(worktree)
        .map_err(|_| Error::new("git_source", "Declared worktree is unavailable."))?;
    if !repository.is_dir() || !worktree.is_dir() {
        return Err(Error::new(
            "git_source",
            "Declared repository/worktree must be directories.",
        ));
    }
    let top = git(
        &worktree,
        &["rev-parse", "--show-toplevel"],
        4096,
        deadline,
        remaining,
    )?;
    if std::fs::canonicalize(top.trim_end_matches('\n'))
        .ok()
        .as_ref()
        != Some(&worktree)
    {
        return Err(Error::new(
            "git_source",
            "Declared worktree must be the checkout root.",
        ));
    }
    let common = |root: &Path, remaining: &mut usize| -> Result<std::path::PathBuf> {
        let value = git(
            root,
            &["rev-parse", "--git-common-dir"],
            4096,
            deadline,
            remaining,
        )?;
        let value = Path::new(value.trim_end_matches('\n'));
        std::fs::canonicalize(if value.is_absolute() {
            value.to_path_buf()
        } else {
            root.join(value)
        })
        .map_err(|_| {
            Error::new(
                "git_source",
                "Cannot resolve declared Git repository identity.",
            )
        })
    };
    if common(&repository, remaining)? != common(&worktree, remaining)? {
        return Err(Error::new(
            "git_source",
            "Declared repository and worktree belong to different Git repositories.",
        ));
    }
    let actual = git(
        &worktree,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        1024,
        deadline,
        remaining,
    )?;
    if actual.trim_end_matches('\n') != branch {
        return Err(Error::new(
            "git_source",
            "Declared branch does not match the actual worktree branch.",
        ));
    }
    Ok(())
}

/// Convert a canonical source path to bounded UTF-8 metadata; do not lossy-normalize identity.
fn path_text(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| Error::new("git_source", "Source paths must be UTF-8."))?;
    model::text(value, 1024).map_err(report_error)?;
    if value.chars().any(char::is_control) {
        return Err(Error::new(
            "git_source",
            "Source paths must not contain control characters.",
        ));
    }
    Ok(value.into())
}

/// Run one read-only Git command with disabled replacement objects, pager, prompts and Git overrides.
/// Failed commands expose a fixed refusal, never arbitrary Git stderr or rejected commit content.
fn git(
    root: &Path,
    args: &[&str],
    cap: usize,
    deadline: Instant,
    remaining: &mut usize,
) -> Result<String> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args([
            "--no-pager",
            "--no-replace-objects",
            "-c",
            "core.quotePath=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|name| name.starts_with("GIT_")) {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    let bytes = capture(command, cap.min(*remaining), deadline)?;
    *remaining = remaining.checked_sub(bytes.len()).ok_or_else(|| {
        Error::new(
            "git_capacity",
            "Git report output exceeded its aggregate budget.",
        )
    })?;
    String::from_utf8(bytes)
        .map_err(|_| Error::new("git_source", "Git report output must be valid UTF-8."))
}

/// Capture a subprocess with a cap enforced by its reader before decoding and a caller deadline.
/// A full pipe cannot deadlock the process waiter; timeout/capacity errors kill and reap the child.
/// No stderr/stdin reaches the subprocess caller. Nonzero exit is a read-source refusal.
fn capture(mut command: Command, cap: usize, deadline: Instant) -> Result<Vec<u8>> {
    if Instant::now() >= deadline {
        return Err(Error::new(
            "git_timeout",
            "Local Git report reading exceeded five seconds.",
        ));
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::new("git_unavailable", "Cannot start local Git report reading."))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::new("git_source", "Cannot capture local Git output."));
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take((cap + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        })
        .map_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
            Error::new(
                "git_unavailable",
                "Cannot start bounded local Git output reading.",
            )
        })?;
    let mut output = None;
    loop {
        if output.is_none() {
            match receiver.try_recv() {
                Ok(Ok(bytes)) if bytes.len() <= cap => output = Some(bytes),
                Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::new(
                        "git_capacity",
                        "Local Git output could not be read within its bounded capacity.",
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(Error::new(
                        "git_source",
                        "Local Git could not read the declared checkout/commit; inspect the source before importing.",
                    ));
                }
                if let Some(output) = output {
                    return Ok(output);
                }
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::new(
                    "git_source",
                    "Cannot observe local Git report reading.",
                ));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new(
                "git_timeout",
                "Local Git report reading exceeded five seconds.",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Convert owned report validation into a fixed category while preserving only safe semantic bounds.
fn report_error(message: String) -> Error {
    Error::new("git_report", message)
}

/// Parse Result/Checks/Gaps/Followups sections from a bounded original message.
/// Result accepts multiline prose. Checks use `status | label | optional detail`; gaps/followups
/// use one item per nonempty line (optional `- ` bullets). Each section occurs at most once.
/// Unknown check statuses, duplicate labels, missing Result or exceeded domain bounds refuse.
fn parse_report(message: &str) -> Result<ParsedReport> {
    model::text(message, MESSAGE_CAP).map_err(report_error)?;
    let mut report = ParsedReport {
        summary: String::new(),
        checks: Vec::new(),
        gaps: Vec::new(),
        followups: Vec::new(),
    };
    let mut section = None;
    let mut sections = BTreeSet::new();
    let mut labels = BTreeSet::new();
    for original in message.lines() {
        let mut line = original.trim();
        for header in ["Result:", "Checks:", "Gaps:", "Followups:"] {
            if let Some(rest) = line.strip_prefix(header) {
                if !sections.insert(header) {
                    return Err(Error::new(
                        "git_report",
                        "Each report section may occur only once.",
                    ));
                }
                section = Some(header);
                line = rest.trim();
                break;
            }
        }
        if line.is_empty() {
            continue;
        }
        match section {
            Some("Result:") => {
                if !report.summary.is_empty() {
                    report.summary.push('\n');
                }
                report.summary.push_str(line);
                if report.summary.len() > 1024 {
                    return Err(Error::new("git_report", "Result exceeds 1024 UTF-8 bytes."));
                }
            }
            Some("Checks:") => {
                let parts: Vec<_> = line.splitn(3, '|').map(str::trim).collect();
                if parts.len() < 2 {
                    return Err(Error::new(
                        "git_report",
                        "Checks use status | label | optional detail.",
                    ));
                }
                let status = match parts[0] {
                    "passed" => CheckStatus::Passed,
                    "failed" => CheckStatus::Failed,
                    "not_run" => CheckStatus::NotRun,
                    "not_applicable" => CheckStatus::NotApplicable,
                    _ => return Err(Error::new("git_report", "Unknown reported check status.")),
                };
                model::text(parts[1], 64).map_err(report_error)?;
                if !labels.insert(parts[1].to_owned()) || report.checks.len() >= 8 {
                    return Err(Error::new(
                        "git_report",
                        "Checks require at most eight distinct labels.",
                    ));
                }
                let detail = parts
                    .get(2)
                    .filter(|value| !value.is_empty())
                    .map(|value| (*value).to_owned());
                model::optional(&detail, 256).map_err(report_error)?;
                report.checks.push(CheckInput {
                    label: parts[1].into(),
                    status,
                    detail,
                });
            }
            Some("Gaps:") | Some("Followups:") => {
                let item = line.strip_prefix("- ").unwrap_or(line);
                model::text(item, 256).map_err(report_error)?;
                let items = if section == Some("Gaps:") {
                    &mut report.gaps
                } else {
                    &mut report.followups
                };
                if items.len() >= 8 {
                    return Err(Error::new(
                        "git_report",
                        "At most eight gaps/followups are allowed.",
                    ));
                }
                items.push(item.into());
            }
            _ => {}
        }
    }
    model::text(&report.summary, 1024).map_err(|_| {
        Error::new(
            "git_report",
            "Commit must contain a meaningful Result section.",
        )
    })?;
    Ok(report)
}

/// Actual disposable Git fixtures exercise metadata, parser refusals and bounded read-only execution.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated Git fixture assertions"
)]
mod tests {
    use super::*;

    /// Run fixture-only Git with user/system configuration disabled, refusing setup failures.
    fn fixture_git(root: &Path, args: &[&str]) -> String {
        let mut command = Command::new("git");
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| name.starts_with("GIT_")) {
                command.env_remove(name);
            }
        }
        let output = command
            .current_dir(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Report Author")
            .env("GIT_AUTHOR_EMAIL", "report@example.invalid")
            .env("GIT_COMMITTER_NAME", "Report Committer")
            .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
            .env("GIT_AUTHOR_DATE", "2026-10-06T10:00:00+02:00")
            .env("GIT_COMMITTER_DATE", "2026-10-06T10:00:00+02:00")
            .output()
            .unwrap();
        assert!(output.status.success(), "Fixture Git failed");
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end_matches('\n')
            .into()
    }

    /// Write a supplied commit/tag object only into this disposable fixture, returning its full SHA.
    /// Git overrides/global configuration are disabled; callers supply bounded fixed test bytes.
    fn fixture_object(root: &Path, kind: &str, bytes: &[u8]) -> String {
        use std::io::Write;
        let mut command = Command::new("git");
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_some_and(|name| name.starts_with("GIT_")) {
                command.env_remove(name);
            }
        }
        let mut child = command
            .current_dir(root)
            .args(["hash-object", "-t", kind, "-w", "--stdin"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().into()
    }

    /// Initialize one disposable checkout and commit the supplied original report message.
    fn fixture(message: &str) -> (tempfile::TempDir, String) {
        let temp = tempfile::tempdir_in("/private/tmp").unwrap();
        fixture_git(
            temp.path(),
            &["-c", "init.defaultBranch=main", "init", "--quiet"],
        );
        fixture_git(
            temp.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "--cleanup=verbatim",
                "-m",
                message,
            ],
        );
        let sha = fixture_git(temp.path(), &["rev-parse", "HEAD"]);
        (temp, sha)
    }

    /// Read two reports and deduplicate abbreviated/full selectors without changing source tree/index.
    #[test]
    fn actual_git_reports_preserve_metadata_and_do_not_mutate_source() {
        let message = "feat: report delivery\n\nResult:\nFirst delivery recorded\nChecks:\npassed | native | Checked manually\nGaps:\n- Remaining scenario\nFollowups:\n- Future scenario\n";
        let (temp, first) = fixture(message);
        fixture_git(
            temp.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "--cleanup=verbatim",
                "-m",
                "fix: next delivery\n\nResult: Second delivery recorded\nChecks:\nnot_run | later",
            ],
        );
        let second = fixture_git(temp.path(), &["rev-parse", "HEAD"]);
        std::fs::write(temp.path().join("untracked-evidence"), "preserve").unwrap();
        let before = fixture_git(temp.path(), &["status", "--porcelain=v1"]);
        let index = std::fs::read(temp.path().join(".git/index")).ok();
        let observed = read_commits(
            temp.path(),
            &[first[..12].into(), first.clone(), second.clone()],
        )
        .unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].sha, first);
        assert_eq!(observed[1].sha, second);
        assert_eq!(observed[0].author, "Report Author <report@example.invalid>");
        assert_eq!(observed[0].authored_at, "2026-10-06T08:00:00Z");
        assert_eq!(observed[0].message, message);
        assert_eq!(
            observed[0].checks[0].detail.as_deref(),
            Some("Checked manually")
        );
        assert_eq!(observed[0].gaps, ["Remaining scenario"]);
        assert_eq!(observed[0].followups, ["Future scenario"]);
        assert_eq!(
            fixture_git(temp.path(), &["status", "--porcelain=v1"]),
            before
        );
        assert_eq!(std::fs::read(temp.path().join(".git/index")).ok(), index);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("untracked-evidence")).unwrap(),
            "preserve"
        );
    }

    /// Invalid selectors, noncommit objects, missing commits and nonrepositories remain safe refusals.
    #[test]
    fn refuses_invalid_sources_and_oversized_or_malformed_reports() {
        let (temp, sha) = fixture("feat: initial\n\nResult: Meaningful report");
        for selector in [
            "--help",
            "HEAD",
            "HEAD~1",
            "abcdef",
            "0000000000000000000000000000000000000000",
        ] {
            assert!(read_commits(temp.path(), &[selector.into()]).is_err());
        }
        let fake_hash = if sha.starts_with("1111111") {
            "2222222"
        } else {
            "1111111"
        };
        fixture_git(
            temp.path(),
            &["update-ref", &format!("refs/heads/{fake_hash}"), &sha],
        );
        assert!(read_commits(temp.path(), &[fake_hash.into()]).is_err());
        let tree = fixture_git(temp.path(), &["rev-parse", "HEAD^{tree}"]);
        assert!(read_commits(temp.path(), &[tree]).is_err());
        let tag = format!(
            "object {sha}\ntype commit\ntag fixture-tag\ntagger Report Author <report@example.invalid> 0 +0000\n\nFixture tag\n"
        );
        let tag = fixture_object(temp.path(), "tag", tag.as_bytes());
        assert_eq!(
            read_commits(temp.path(), &[tag]).unwrap_err().code,
            "git_source"
        );
        let empty = tempfile::tempdir_in("/private/tmp").unwrap();
        assert!(read_commits(empty.path(), &[sha]).is_err());
        for message in [
            "No result",
            "Result: report\nChecks:\nunknown | check",
            "Result: report\nChecks:\npassed | same\nfailed | same",
            "Result: one\nResult: two",
        ] {
            let (temp, sha) = fixture(message);
            assert!(read_commits(temp.path(), &[sha]).is_err());
        }
        let (temp, sha) = fixture(&format!(
            "feat: large\n\nResult: {}",
            "x".repeat(MESSAGE_CAP + 2048)
        ));
        assert!(read_commits(temp.path(), &[sha]).is_err());
        assert!(parse_report(&format!("Result: {}", "x".repeat(1025))).is_err());
        assert!(parse_report(&format!("Result: report\nGaps:\n{}", "item\n".repeat(9))).is_err());
    }

    /// Import preflight rejects unrelated repositories and wrong declared branches without Git writes.
    #[test]
    fn declared_execution_must_match_git_repository_and_branch() {
        let (temp, _) = fixture("feat: source\n\nResult: Source report");
        let (other, _) = fixture("feat: other\n\nResult: Other report");
        verify_execution(temp.path(), temp.path(), "main").unwrap();
        assert_eq!(
            verify_execution(temp.path(), temp.path(), "wrong-branch")
                .unwrap_err()
                .code,
            "git_source"
        );
        let mismatch = verify_execution(other.path(), temp.path(), "main").unwrap_err();
        assert!(mismatch.message.contains("different Git repositories"));
    }

    /// Native SHA-256 repositories resolve abbreviated selectors to full 64-character source IDs.
    #[test]
    fn supports_native_sha256_commit_ids() {
        let temp = tempfile::tempdir_in("/private/tmp").unwrap();
        fixture_git(temp.path(), &["init", "--quiet", "--object-format=sha256"]);
        fixture_git(
            temp.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "-m",
                "feat: SHA256 source\n\nResult: SHA256 report imported",
            ],
        );
        let sha = fixture_git(temp.path(), &["rev-parse", "HEAD"]);
        assert_eq!(sha.len(), 64);
        let report = read_commits(temp.path(), &[sha[..16].into(), sha.clone()]).unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].sha, sha);
        assert_eq!(
            report[0].repository,
            std::fs::canonicalize(temp.path().join(".git"))
                .unwrap()
                .to_str()
                .unwrap()
        );
    }

    /// Preserve the UTF-8 trust boundary even when Git contains a syntactically valid binary message.
    #[test]
    fn rejects_non_utf8_commit_messages_without_echoing_source() {
        let (temp, _) = fixture("feat: initial\n\nResult: Existing report");
        let tree = fixture_git(temp.path(), &["rev-parse", "HEAD^{tree}"]);
        let mut object = format!("tree {tree}\nauthor Report Author <report@example.invalid> 0 +0000\ncommitter Report Author <report@example.invalid> 0 +0000\n\nfeat: invalid UTF8\n\nResult: ").into_bytes();
        object.push(0xff);
        let sha = fixture_object(temp.path(), "commit", &object);
        let error = read_commits(temp.path(), &[sha]).unwrap_err();
        assert_eq!(error.code, "git_source");
        assert_eq!(error.message, "Git report output must be valid UTF-8.");
    }

    /// Missing promisor objects refuse locally without invoking the configured local fetch helper.
    #[cfg(unix)]
    #[test]
    fn unavailable_promisor_object_never_runs_lazy_fetch() {
        use std::os::unix::fs::PermissionsExt;
        let (temp, sha) = fixture("feat: promised report\n\nResult: Promised report");
        let marker = temp.path().join("fetch-was-invoked");
        let upload = temp.path().join("upload-pack-fixture");
        std::fs::write(
            &upload,
            format!("#!/bin/sh\n: > '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&upload, std::fs::Permissions::from_mode(0o700)).unwrap();
        fixture_git(
            temp.path(),
            &["config", "core.repositoryFormatVersion", "1"],
        );
        fixture_git(
            temp.path(),
            &["config", "extensions.partialClone", "origin"],
        );
        fixture_git(temp.path(), &["config", "remote.origin.promisor", "true"]);
        fixture_git(
            temp.path(),
            &[
                "config",
                "remote.origin.url",
                temp.path().join("missing-local-remote").to_str().unwrap(),
            ],
        );
        fixture_git(
            temp.path(),
            &[
                "config",
                "remote.origin.uploadpack",
                upload.to_str().unwrap(),
            ],
        );
        std::fs::remove_file(
            temp.path()
                .join(format!(".git/objects/{}/{}", &sha[..2], &sha[2..])),
        )
        .unwrap();
        assert_eq!(
            read_commits(temp.path(), &[sha]).unwrap_err().code,
            "git_source"
        );
        assert!(!marker.exists());
    }

    /// Oversized actual author metadata is rejected before it can reach core persistence.
    #[test]
    fn oversized_actual_author_is_a_safe_report_refusal() {
        let (temp, _) = fixture("feat: first\n\nResult: First report");
        let author = format!(
            "{} <author@example.invalid>",
            "private-author-canary".repeat(20)
        );
        fixture_git(
            temp.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "--author",
                &author,
                "-m",
                "feat: long author\n\nResult: Meaningful report",
            ],
        );
        let sha = fixture_git(temp.path(), &["rev-parse", "HEAD"]);
        let error = read_commits(temp.path(), &[sha]).unwrap_err();
        assert_eq!(error.code, "git_report");
        assert!(!error.message.contains("private-author-canary"));
    }

    /// Execution preflight and commit reads consume the same deadline and total output budget.
    #[test]
    fn verified_import_shares_one_budget_across_preflight_and_commit_reads() {
        let (temp, sha) = fixture("feat: bounded source\n\nResult: Shared import budget");
        let reports =
            read_verified_commits(temp.path(), temp.path(), "main", std::slice::from_ref(&sha))
                .unwrap();
        assert_eq!(reports[0].sha, sha);
        let start = Instant::now();
        let mut remaining = TOTAL_CAP;
        let error = read_verified_commits_until(
            temp.path(),
            temp.path(),
            "main",
            std::slice::from_ref(&sha),
            start + Duration::from_millis(1),
            &mut remaining,
        )
        .unwrap_err();
        assert_eq!(error.code, "git_timeout");
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut remaining = 1;
        let error = read_verified_commits_until(
            temp.path(),
            temp.path(),
            "main",
            &[sha],
            Instant::now() + READ_BUDGET,
            &mut remaining,
        )
        .unwrap_err();
        assert_eq!(error.code, "git_capacity");
    }

    /// The bounded runner kills/reaps a slow child and refuses capacity overflow before parsing.
    #[test]
    fn capture_enforces_deadline_and_output_capacity() {
        let start = Instant::now();
        let mut command = Command::new("/bin/sleep");
        command.arg("2");
        assert_eq!(
            capture(command, 32, start + Duration::from_millis(25))
                .unwrap_err()
                .code,
            "git_timeout"
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut command = Command::new("git");
        command.arg("--version");
        assert_eq!(
            capture(command, 1, Instant::now() + READ_BUDGET)
                .unwrap_err()
                .code,
            "git_capacity"
        );
    }
}
