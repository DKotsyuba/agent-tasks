//! Read-only Git evidence collection; one frozen snapshot per observation, never a tracker.
//!
//! Identity is the canonical Git common directory, so linked worktrees share one repository
//! record while independent clones do not. [`collect`] freezes ref tips, enumerates commits the
//! baseline cannot reach, and reads bounded endpoint patches, integration-target content and
//! per-worktree drafts. Every command is a non-interactive argv call with inherited `GIT_*`
//! environment removed, a bounded output reader, one shared wall-clock deadline and one shared
//! output budget. Incomplete, missing, oversized, racing, secret or binary facts become explicit
//! [`Snapshot::omissions`] entries; the collector never returns a silent empty success, never
//! mutates Git state, never fetches, and never runs a shell.
//!
//! [`Evidence::kind`] taxonomy, stable for analyzer citation:
//! - `commit`: reference is the full commit SHA; text is the bounded original message.
//! - `check`: reference is `<sha>:<label>`; text is `status | detail` parsed from a
//!   `Checks:` section in that commit message. It is an author assertion, not proof of
//!   execution; malformed lines are ignored rather than rejected.
//! - `target_history`: reference is `<integration-ref>@<target-sha>`; text lists the
//!   integration endpoint's recent subjects, making merges, reverts and re-lands visible.
//! - `endpoint_patch`: reference is `<branch-or-detached>@<tip-sha>`; text is the bounded
//!   two-dot content diff `target..tip`. A retained feature branch after a target revert
//!   still shows its feature lines here, so completion never rests on ancestry alone.
//! - `staged` / `unstaged`: per linked worktree draft patches against the index/HEAD.
//! - `untracked`: per linked worktree list of non-ignored new file paths.
//! - `draft_text`: bounded content of one eligible new text file; secrets, binaries,
//!   symlinks and ignored files are never ingested and surface as omissions instead.

use super::error::{Error, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// One cumulative wall-clock budget shared by every command of one collection.
const READ_BUDGET: Duration = Duration::from_secs(20);
/// Aggregate Git output budget; the serialized snapshot stays well below the 1 MiB wire frame.
const TOTAL_CAP: usize = 384 * 1024;
/// Cap for the final serialized snapshot, leaving headroom inside the 1 MiB frame.
const FRAME_LIMIT: usize = 900 * 1024;
/// Maximum retained original commit message bytes, including subject and line endings.
const MESSAGE_CAP: usize = 2048;
/// Maximum retained patch (endpoint or draft) bytes per evidence entry.
const PATCH_CAP: usize = 12 * 1024;
/// Maximum retained integration-target history bytes.
const TARGET_HISTORY_CAP: usize = 4096;
/// Maximum `git status` bytes parsed for one worktree.
const STATUS_CAP: usize = 64 * 1024;
/// Maximum retained untracked path list bytes per worktree.
const UNTRACKED_LIST_CAP: usize = 8 * 1024;
/// Maximum retained bytes per eligible new text file.
const DRAFT_FILE_CAP: usize = 12 * 1024;
/// Maximum changed files expanded after an aggregate patch exceeds its bound.
const DIFF_FILE_LIMIT: usize = 6;
/// Shared per-diff fallback budget; unrelated large files cannot evict every small implementation.
const DIFF_FILE_BYTES: usize = 24 * 1024;
/// Maximum eligible new text files ingested per worktree.
const DRAFT_FILE_LIMIT: usize = 12;
/// Maximum untracked paths listed per worktree before the list is truncated.
const UNTRACKED_PATH_LIMIT: usize = 200;
/// Maximum linked worktrees inspected; excess worktrees are explicit omissions.
const WORKTREE_LIMIT: usize = 12;
/// Maximum local branches enumerated; excess branches are explicit omissions.
const BRANCH_LIMIT: usize = 64;
/// Maximum new commits with retained messages per collection.
const COMMIT_LIMIT: usize = 64;
/// Maximum rev-list rows read per tip; exceeding it truncates that tip's enumeration.
const TIP_WALK_LIMIT: usize = COMMIT_LIMIT + 1;
/// Maximum baseline tip SHAs excluded from the new-commit walk.
const BASELINE_TIP_LIMIT: usize = 96;
/// Maximum omission entries; further losses collapse into one aggregate omission.
const OMISSION_LIMIT: usize = 64;
/// Maximum bounded UTF-8 bytes for identity paths and evidence references.
const TEXT_CAP: usize = 1024;
/// Maximum bytes of a validated integration ref supplied by the planning caller.
const REF_CAP: usize = 256;
/// Git ref patterns that would resolve as options or multi-revision ranges are refused.
const REF_FORBIDDEN: &[char] = &[' ', '\t', '\n', '\r', '\0'];
/// Native Git pathspecs exclude credential containers even when accidentally tracked.
const DIFF_EXCLUSIONS: &[&str] = &[
    ":(exclude,icase,glob)**/*credential*",
    ":(exclude,icase,glob)**/*password*",
    ":(exclude,icase,glob)**/*secret*",
    ":(exclude,icase,glob)**/.env",
    ":(exclude,icase,glob)**/.env.*",
    ":(exclude,icase,glob)**/.netrc",
    ":(exclude,icase,glob)**/.npmrc",
    ":(exclude,icase,glob)**/.pypirc",
    ":(exclude,icase,glob)**/.htpasswd",
    ":(exclude,icase,glob)**/credentials",
    ":(exclude,icase,glob)**/credentials.json",
    ":(exclude,icase,glob)**/secrets.json",
    ":(exclude,icase,glob)**/secrets.toml",
    ":(exclude,icase,glob)**/passwords.txt",
    ":(exclude,icase,glob)**/id_rsa*",
    ":(exclude,icase,glob)**/id_ed25519*",
    ":(exclude,icase,glob)**/id_ecdsa*",
    ":(exclude,icase,glob)**/*.pem",
    ":(exclude,icase,glob)**/*.key",
    ":(exclude,icase,glob)**/*.p12",
    ":(exclude,icase,glob)**/*.pfx",
    ":(exclude,icase,glob)**/*.kdbx",
    ":(exclude,icase,glob)**/*.jks",
    ":(exclude,icase,glob)**/*.keystore",
];

/// Stable repository identity shared by every linked worktree of one common directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    /// Versioned SHA-256 of the canonical common directory for safe storage names.
    pub key: String,
    /// Canonical Git common directory consumed by the Agent Run adapter for worker matching.
    pub common_dir: PathBuf,
    /// Canonical actual declared checkout this identity was resolved from.
    pub root: PathBuf,
    /// Validated integration ref, from the caller or the primary worktree's actual branch.
    pub integration_ref: String,
}

/// One bounded, serializable piece of observed evidence; `kind`/`reference` semantics are fixed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    /// Stable citation id `<kind>:<reference>`; the analyzer may only cite these ids.
    pub id: String,
    /// Fixed taxonomy tag documented in the module docs.
    pub kind: String,
    /// Canonical worktree the entry was observed in (the repository root for commit facts).
    pub worktree: String,
    /// Semantic locator within `kind` (SHA, `<label>@<sha>`, `wt<N>!<locator>`, or a path).
    pub reference: String,
    /// Bounded UTF-8 content; truncation is always paired with an explicit omission.
    pub text: String,
}

/// One frozen observation of a repository, suitable for baseline storage and model input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Repository identity resolved for this observation.
    pub repository: Identity,
    /// Collection time in UTC RFC 3339 seconds; excluded from the semantic digest.
    pub observed_at: String,
    /// Exact initial/current integration commit, independent of optional history excerpts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sha: Option<String>,
    /// Deduplicated full SHAs of local branch heads and linked worktree HEADs, sorted.
    pub tips: Vec<String>,
    /// Bounded evidence entries, sorted by id for determinism.
    pub evidence: Vec<Evidence>,
    /// Explicit incomplete, missing, budget, racing, secret or binary facts.
    pub omissions: Vec<String>,
    /// SHA-256 over semantic content only; clock and absolute worktree names are excluded.
    pub semantic_digest: String,
}

/// Resolves the repository identity: canonical common directory, checkout root, and the
/// integration ref. An explicitly supplied ref must be a concrete existing revision; without
/// one, the primary (non-linked) worktree's actual symbolic branch is used, and a detached or
/// missing primary is an error with code `git_ambiguous` that the planning caller resolves by
/// supplying a ref after Git inspection. Refuses non-repositories and nested/parent paths.
pub fn identify(repository: &Path, integration_ref: Option<&str>) -> Result<Identity> {
    let mut remaining = TOTAL_CAP;
    resolve_identity(
        repository,
        integration_ref,
        Instant::now() + READ_BUDGET,
        &mut remaining,
    )
}

/// Resolves identity under shared budgets, returning the canonical identity or a coded refusal.
fn resolve_identity(
    repository: &Path,
    integration_ref: Option<&str>,
    deadline: Instant,
    remaining: &mut usize,
) -> Result<Identity> {
    if !repository.is_absolute() {
        return Err(Error::new(
            "git_source",
            "Declare an absolute local repository/worktree path.",
        ));
    }
    let root = std::fs::canonicalize(repository)
        .map_err(|_| Error::new("git_source", "Declared repository path is unavailable."))?;
    if !root.is_dir() {
        return Err(Error::new(
            "git_source",
            "Declared repository path must be a directory.",
        ));
    }
    let top = run_git(
        &root,
        &["rev-parse", "--show-toplevel"],
        4096,
        deadline,
        remaining,
    )?;
    if !std::fs::canonicalize(top.trim_end_matches('\n')).is_ok_and(|top| top == root) {
        return Err(Error::new(
            "git_source",
            "Declared path must be the checkout root, not a parent or nested directory.",
        ));
    }
    let common_raw = run_git(
        &root,
        &["rev-parse", "--git-common-dir"],
        4096,
        deadline,
        remaining,
    )?;
    let common_raw = PathBuf::from(common_raw.trim_end_matches('\n'));
    let common = std::fs::canonicalize(if common_raw.is_absolute() {
        common_raw
    } else {
        root.join(&common_raw)
    })
    .map_err(|_| {
        Error::new(
            "git_source",
            "Cannot resolve the canonical Git repository identity.",
        )
    })?;
    let worktrees = worktree_listing(&root, deadline, remaining)?;
    let target_ref = match integration_ref {
        Some(explicit) => {
            validate_ref(explicit)?;
            let resolved = run_git(
                &root,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{explicit}^{{commit}}"),
                ],
                128,
                deadline,
                remaining,
            )
            .map_err(|error| {
                if matches!(error.code.as_str(), "git_unavailable" | "git_source") {
                    Error::new(
                        "git_ref",
                        "Supplied integration ref does not resolve to a commit.",
                    )
                } else {
                    error
                }
            })?;
            if !is_full_sha(resolved.trim_end_matches('\n')) {
                return Err(Error::new(
                    "git_ref",
                    "Supplied integration ref does not resolve to an existing commit.",
                ));
            }
            let symbolic = run_git(
                &root,
                &[
                    "rev-parse",
                    "--symbolic-full-name",
                    "--verify",
                    "--quiet",
                    explicit,
                ],
                REF_CAP + 64,
                deadline,
                remaining,
            )?;
            let symbolic = symbolic.trim();
            if symbolic.starts_with("refs/") && symbolic.lines().count() == 1 {
                symbolic.to_owned()
            } else {
                resolved.trim_end_matches('\n').to_owned()
            }
        }
        None => primary_branch(&root, &common, &worktrees, deadline, remaining)?,
    };
    Ok(Identity {
        key: format!(
            "{:x}",
            Sha256::digest(format!("agent-tasks-observer-v1\0{}", path_text(&common)?))
        ),
        common_dir: common,
        root,
        integration_ref: target_ref,
    })
}

/// Validates a caller-visible ref as pure data: bounded, option-free and traversal-free.
/// Subprocess argv carries no shell, so this closes the only injection surface: arguments Git
/// would parse as options. The `^{commit}` suffix used by resolution stays permitted.
fn validate_ref(reference: &str) -> Result<()> {
    let permitted = |byte: u8| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'/' | b'.' | b'_' | b'-' | b'+' | b'^' | b'{' | b'}' | b'@'
            )
    };
    if reference.is_empty()
        || reference.len() > REF_CAP
        || !reference.bytes().all(permitted)
        || reference.starts_with(['-', '/'])
        || reference.contains("..")
        || reference.contains("//")
        || reference.contains(REF_FORBIDDEN)
    {
        return Err(Error::new(
            "git_ref",
            "Integration ref must be a concrete bounded revision name without options.",
        ));
    }
    Ok(())
}

/// Converts a canonical identity path to bounded UTF-8 metadata without lossy rewriting.
fn path_text(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| Error::new("git_source", "Repository paths must be valid UTF-8."))?;
    if value.len() > TEXT_CAP || value.chars().any(char::is_control) {
        return Err(Error::new(
            "git_source",
            "Repository paths must be bounded and free of control characters.",
        ));
    }
    Ok(value.to_owned())
}

/// Returns this repository's worktrees (primary checkout included) as canonical
/// (path, HEAD) pairs, sorted by path, bounded to [`WORKTREE_LIMIT`].
fn worktree_listing(
    root: &Path,
    deadline: Instant,
    remaining: &mut usize,
) -> Result<Vec<(PathBuf, Option<String>)>> {
    let raw = run_git(
        root,
        &["worktree", "list", "--porcelain"],
        32 * 1024,
        deadline,
        remaining,
    )
    .map_err(|_| {
        Error::new(
            "git_source",
            "Cannot enumerate this repository's worktrees.",
        )
    })?;
    Ok(parse_worktrees(&raw))
}

/// Parses porcelain worktree records into canonical (path, HEAD) pairs; entries without a
/// resolvable path are dropped silently and excess entries stay beyond [`WORKTREE_LIMIT`].
fn parse_worktrees(raw: &str) -> Vec<(PathBuf, Option<String>)> {
    let mut listing: Vec<(PathBuf, Option<String>)> = Vec::new();
    for record in raw.split("\n\n") {
        let mut path = None;
        let mut head = None;
        for line in record.lines() {
            if let Some(value) = line.strip_prefix("worktree ") {
                path = std::fs::canonicalize(value).ok();
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                head = is_full_sha(value.trim()).then(|| value.trim().to_owned());
            }
        }
        if let Some(path) = path.filter(|path| !listing.iter().any(|(seen, _)| seen == path)) {
            listing.push((path, head));
        }
    }
    listing.sort_by(|left, right| left.0.cmp(&right.0));
    listing
}

/// Returns the primary worktree's actual symbolic branch as the default integration ref.
/// The primary checkout is the worktree whose private Git directory is the common directory
/// itself; a detached or missing primary keeps `git_ambiguous` for the caller.
fn primary_branch(
    _root: &Path,
    common: &Path,
    worktrees: &[(PathBuf, Option<String>)],
    deadline: Instant,
    remaining: &mut usize,
) -> Result<String> {
    for (worktree, _) in worktrees {
        let Ok(private) = run_git(
            worktree,
            &["rev-parse", "--absolute-git-dir"],
            4096,
            deadline,
            remaining,
        ) else {
            continue;
        };
        if std::fs::canonicalize(private.trim_end_matches('\n'))
            .ok()
            .as_deref()
            != Some(common)
        {
            continue;
        }
        if let Ok(branch) = run_git(
            worktree,
            &["symbolic-ref", "--quiet", "HEAD"],
            REF_CAP + 64,
            deadline,
            remaining,
        ) {
            let branch = branch.trim_end_matches('\n');
            if !branch.is_empty() {
                validate_ref(branch)?;
                return Ok(branch.to_owned());
            }
        }
        break;
    }
    Err(Error::new(
        "git_ambiguous",
        "Primary checkout has no actual branch; supply a concrete integration ref.",
    ))
}

/// One frozen collection: freezes ref tips, gathers bounded evidence and compares start/end
/// fingerprints so racing repositories become explicit partial observations.
pub fn collect(
    repository: &Path,
    baseline: Option<&Snapshot>,
    integration_ref: Option<&str>,
) -> Result<Snapshot> {
    collect_focused(repository, baseline, integration_ref, &[])
}

/// Collect with active worker workdirs prioritized inside this repository; hints grant no new paths.
/// All normal read, output, race and omission bounds remain in force.
pub fn collect_focused(
    repository: &Path,
    baseline: Option<&Snapshot>,
    integration_ref: Option<&str>,
    workdirs: &[PathBuf],
) -> Result<Snapshot> {
    let mut remaining = TOTAL_CAP;
    collect_until(
        repository,
        baseline,
        integration_ref,
        Instant::now() + READ_BUDGET,
        &mut remaining,
        None,
        workdirs,
    )
}

/// `collect` with injected budgets and an optional probe invoked between the start and end
/// fingerprints. The probe is a test seam only; production callers pass `None`.
fn collect_until(
    repository: &Path,
    baseline: Option<&Snapshot>,
    integration_ref: Option<&str>,
    deadline: Instant,
    remaining: &mut usize,
    probe: Option<&dyn Fn()>,
    workdirs: &[PathBuf],
) -> Result<Snapshot> {
    let identity = resolve_identity(repository, integration_ref, deadline, remaining)?;
    if let Some(base) = baseline
        && base.repository.key != identity.key
    {
        return Err(Error::new(
            "git_source",
            "The baseline snapshot belongs to a different Git repository.",
        ));
    }
    let mut collector = Collector {
        root: identity.root.clone(),
        target_ref: identity.integration_ref.clone(),
        target_sha: String::new(),
        deadline,
        remaining: *remaining,
        omissions: Vec::new(),
        focus: workdirs.to_vec(),
        changed_paths: Vec::new(),
    };
    for round in 0..2 {
        collector.omissions.clear();
        let (tips, evidence, changed) = collector.snapshot_once(baseline, probe)?;
        if !changed || round == 1 {
            if changed {
                collector.omit("Repository refs changed during collection; evidence may be mixed.");
            }
            *remaining = collector.remaining;
            return finalize(
                identity,
                Some(collector.target_sha.clone()),
                tips,
                evidence,
                collector.omissions,
            );
        }
    }
    Err(Error::new(
        "git_internal",
        "Collection retry loop exited without a snapshot.",
    ))
}

/// Frozen tip set, linked checkout heads and named branch endpoints in that order.
type RefState = (
    BTreeSet<String>,
    Vec<(PathBuf, Option<String>)>,
    Vec<(String, String)>,
);

/// One bounded collection's identity facts, deadline, output budget and explicit omissions.
struct Collector {
    /// Canonical declared checkout; every repository-level command runs here.
    root: PathBuf,
    /// Validated integration ref.
    target_ref: String,
    /// Resolved full target SHA once `snapshot_once` has started.
    target_sha: String,
    /// Absolute shared deadline; expiry is a hard error, never a silent success.
    deadline: Instant,
    /// Remaining aggregate Git output bytes for this collection.
    remaining: usize,
    /// Explicit partial-coverage facts accumulated in collection order.
    omissions: Vec<String>,
    /// Canonical active worker paths, used only to order declared Git worktrees.
    focus: Vec<PathBuf>,
    /// Safe changed paths retained in discovery order for current-target content checks.
    changed_paths: Vec<String>,
}

impl Collector {
    /// Records one bounded omission; past the limit further losses collapse into one aggregate entry.
    fn omit(&mut self, text: impl Into<String>) {
        if self.omissions.len() >= OMISSION_LIMIT {
            return;
        }
        if self.omissions.len() == OMISSION_LIMIT - 1 {
            self.omissions
                .push("Further collection omissions were collapsed.".to_owned());
            return;
        }
        self.omissions.push(bounded(&text.into(), 512));
    }

    /// Runs one bounded Git command, turning capacity overrun and command failure into an
    /// omission-backed `Soft` outcome instead of a failed collection; deadlines stay hard errors.
    fn soft(&mut self, directory: &Path, args: &[&str], cap: usize, what: &str) -> Soft {
        match self.git(directory, args, cap) {
            Ok(text) => Soft::Text(text),
            Err(error) if error.code == "git_capacity" => {
                self.omit(format!("{what} output exceeded its bound."));
                Soft::Overrun
            }
            Err(error) => {
                self.omit(format!("{what} was unavailable ({}).", error.code));
                Soft::Failed
            }
        }
    }

    /// Runs one bounded Git command in this repository or one of its worktrees.
    fn git(&mut self, directory: &Path, args: &[&str], cap: usize) -> Result<String> {
        let cap = cap.min(self.remaining.max(1));
        let bytes = capture(directory, args, cap, self.deadline)?;
        self.remaining = self.remaining.checked_sub(bytes.len()).ok_or_else(|| {
            Error::new(
                "git_capacity",
                "Git evidence output exceeded its aggregate budget.",
            )
        })?;
        String::from_utf8(bytes)
            .map_err(|_| Error::new("git_source", "Git output must be valid UTF-8."))
    }

    /// True while the aggregate budget still admits meaningful evidence; the first exhaustion
    /// becomes one explicit omission and later evidence is skipped.
    fn room(&mut self) -> bool {
        if self.remaining >= 4096 {
            return true;
        }
        self.omit("Evidence budget was exhausted; remaining observations were skipped.");
        false
    }

    /// Freezes one attempt: tips, evidence, and whether refs moved between start and end
    /// fingerprints. Racing attempts are retried once by `collect_until`, never silently mixed.
    fn snapshot_once(
        &mut self,
        baseline: Option<&Snapshot>,
        probe: Option<&dyn Fn()>,
    ) -> Result<(Vec<String>, Vec<Evidence>, bool)> {
        self.target_sha = self.require_target_sha()?;
        let (tips, listing, branches) = self.ref_state()?;
        let before = self.fingerprint(&tips, &listing, &branches, &self.target_sha);
        self.changed_paths.clear();
        let draft_allowance = self.remaining;
        let mut drafts = Vec::new();
        self.draft_evidence(&listing, baseline, &mut drafts);
        if let Some(probe) = probe {
            probe();
        }
        let mut evidence: Vec<Evidence> = Vec::new();
        self.target_evidence(baseline, &mut evidence);
        self.endpoint_patches(&branches, &listing, &mut evidence);
        self.target_blobs(&mut evidence);
        if let Some(base) = baseline {
            self.commit_evidence(&tips, base, &mut evidence);
        }
        let draft_bytes = serde_json::to_vec(&drafts)?;
        evidence.extend(drafts);
        let (end_tips, end_listing, end_branches) = self.ref_state()?;
        let end_target = self.require_target_sha()?;
        let mut end_drafts = Vec::new();
        // Validation repeats the same bounded draft allowance, rather than mistaking a spent
        // collection budget for a content race. These bytes never enter the model input twice.
        let mut validation = Collector {
            root: self.root.clone(),
            target_ref: self.target_ref.clone(),
            target_sha: end_target.clone(),
            deadline: self.deadline,
            remaining: draft_allowance,
            omissions: Vec::new(),
            focus: self.focus.clone(),
            changed_paths: Vec::new(),
        };
        validation.draft_evidence(&end_listing, baseline, &mut end_drafts);
        let changed = before
            != self.fingerprint(&end_tips, &end_listing, &end_branches, &end_target)
            || draft_bytes != serde_json::to_vec(&end_drafts)?;
        Ok((tips.into_iter().collect(), evidence, changed))
    }

    /// Freezes the racing-sensitive ref state: worktree listing, local branch heads and
    /// deduplicated tips. The end-of-attempt re-fetch is what detects mid-collection races.
    fn ref_state(&mut self) -> Result<RefState> {
        let listing = self.worktree_listing()?;
        let branches = self.branches()?;
        let mut tips: BTreeSet<String> = branches.iter().map(|(_, sha)| sha.clone()).collect();
        tips.insert(self.target_sha.clone());
        for (_, head) in &listing {
            if let Some(sha) = head {
                tips.insert(sha.clone());
            }
        }
        Ok((tips, listing, branches))
    }

    /// Start/end fingerprint of racing-sensitive facts: target SHA, deduplicated tips, and each
    /// worktree path with its HEAD. Equal fingerprints mean one internally consistent attempt.
    fn fingerprint(
        &self,
        tips: &BTreeSet<String>,
        listing: &[(PathBuf, Option<String>)],
        branches: &[(String, String)],
        target: &str,
    ) -> String {
        let mut lines: Vec<String> = vec![format!("target {target}")];
        lines.extend(
            branches
                .iter()
                .map(|(name, sha)| format!("branch {name} {sha}")),
        );
        lines.extend(tips.iter().map(|sha| format!("tip {sha}")));
        lines.extend(
            listing
                .iter()
                .map(|(path, head)| format!("wt {} {:?}", path.display(), head)),
        );
        lines.join("\n")
    }

    /// Resolves the integration ref to a full commit SHA, refusing ambiguous or absent targets.
    fn require_target_sha(&mut self) -> Result<String> {
        let target_ref = self.target_ref.clone();
        let resolved = self.git(
            &self.root.clone(),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{target_ref}^{{commit}}"),
            ],
            128,
        )?;
        let sha = resolved.trim_end_matches('\n');
        if !is_full_sha(sha) {
            return Err(Error::new(
                "git_ref",
                "The integration ref does not resolve to an existing commit.",
            ));
        }
        Ok(sha.to_owned())
    }

    /// Parses `git worktree list --porcelain` through the shared parser, surfacing the
    /// worktree-count bound as an explicit omission when the repository exceeds it.
    fn worktree_listing(&mut self) -> Result<Vec<(PathBuf, Option<String>)>> {
        let mut listing = worktree_listing(&self.root.clone(), self.deadline, &mut self.remaining)?;
        listing.sort_by_key(|(path, _)| {
            (
                self.focus.iter().position(|known| known == path).unwrap_or(
                    if path == &self.root {
                        self.focus.len()
                    } else {
                        usize::MAX
                    },
                ),
                path.clone(),
            )
        });
        if listing.len() > WORKTREE_LIMIT {
            self.omit(format!(
                "More than {WORKTREE_LIMIT} worktrees exist; excess worktrees were not inspected."
            ));
        }
        listing.truncate(WORKTREE_LIMIT);
        if listing.is_empty() {
            return Err(Error::new(
                "git_source",
                "This repository reports no inspectable checkout.",
            ));
        }
        Ok(listing)
    }

    /// Enumerates the most recently committed local branch heads before deterministic tie-breaking;
    /// malformed or excess entries become explicit omissions.
    fn branches(&mut self) -> Result<Vec<(String, String)>> {
        let ordered = self.git(
            &self.root.clone(),
            &[
                "for-each-ref",
                "--sort=refname",
                "--sort=-committerdate",
                &format!("--count={}", BRANCH_LIMIT + 1),
                "--format=%(objectname)%00%(refname:short)",
                "refs/heads",
            ],
            32 * 1024,
        );
        let raw = match ordered {
            Ok(raw) => raw,
            Err(error) if error.code == "git_source" => {
                self.omit(
                    "Branch recency was unavailable; unresolved objects remain partial evidence.",
                );
                self.git(
                    &self.root.clone(),
                    &[
                        "for-each-ref",
                        &format!("--count={}", BRANCH_LIMIT + 1),
                        "--format=%(objectname)%00%(refname:short)",
                        "refs/heads",
                    ],
                    32 * 1024,
                )?
            }
            Err(error) => return Err(error),
        };
        let mut branches: Vec<(String, String)> = Vec::new();
        for record in raw.lines() {
            let Some((sha, name)) = record.split_once('\0') else {
                continue;
            };
            if !is_full_sha(sha)
                || name.is_empty()
                || name.len() > REF_CAP
                || name.chars().any(char::is_control)
            {
                self.omit("A local branch entry was malformed and was skipped.");
                continue;
            }
            if branches.len() >= BRANCH_LIMIT {
                self.omit(format!(
                    "More than {BRANCH_LIMIT} local branches exist; excess branches were not inspected."
                ));
                break;
            }
            branches.push((name.to_owned(), sha.to_owned()));
        }
        Ok(branches)
    }

    /// Captures integration-target facts: current code inventory (`target_content`), the
    /// baseline-to-current net change (`target_patch`), recent history subjects
    /// (`target_history`) and the target commit message. History subjects alone are never
    /// integration proof; the inventory and the net patch describe current target code.
    fn target_evidence(&mut self, baseline: Option<&Snapshot>, evidence: &mut Vec<Evidence>) {
        if !self.room() {
            return;
        }
        let target = self.target_sha.clone();
        let reference = format!("{}@{target}", self.target_ref);
        let root = self.root.display().to_string();
        if let Soft::Text(raw) = self.soft(
            &self.root.clone(),
            &["ls-tree", "-r", "--name-only", &target],
            8 * 1024,
            "Target content inventory",
        ) {
            let text = bounded(raw.trim_end(), 8 * 1024);
            if !text.is_empty() {
                push(evidence, "target_content", &root, &reference, &text);
            }
        }
        match self.baseline_target_sha(baseline) {
            Some(base_sha) => {
                let (text, truncated) = self.diff_pair(
                    &self.root.clone(),
                    &[],
                    &[&base_sha, &target],
                    "Target net change",
                );
                if truncated {
                    self.omit(
                        "Target net-change patch has incomplete per-file coverage.".to_owned(),
                    );
                }
                if !text.trim().is_empty() {
                    push(evidence, "target_patch", &root, &reference, &text);
                }
            }
            None if baseline.is_some() => self.omit(
                "Baseline integration target SHA was unavailable; target net changes are unknown."
                    .to_owned(),
            ),
            None => {}
        }
        if let Soft::Text(raw) = self.soft(
            &self.root.clone(),
            &[
                "log",
                "--no-show-signature",
                "--format=%h %s",
                "--max-count=24",
                &target,
            ],
            TARGET_HISTORY_CAP,
            "Integration history",
        ) {
            let text = bounded(raw.trim_end(), TARGET_HISTORY_CAP);
            if !text.is_empty() {
                push(evidence, "target_history", &root, &reference, &text);
            }
        }
        self.commit_message(&target, evidence);
    }

    /// Recovers the baseline snapshot's integration target SHA from its `target_history`
    /// evidence and verifies it still resolves; `None` leaves target net changes explicitly
    /// unknown rather than silently empty.
    fn baseline_target_sha(&mut self, baseline: Option<&Snapshot>) -> Option<String> {
        let prefix = format!("{}@", self.target_ref);
        let baseline = baseline?;
        let sha = baseline.target_sha.clone().or_else(|| {
            baseline.evidence.iter().rev().find_map(|entry| {
                entry
                    .reference
                    .strip_prefix(&prefix)
                    .filter(|_| entry.kind == "target_history")
                    .map(str::to_owned)
            })
        })?;
        if !is_full_sha(&sha) {
            return None;
        }
        let probe = format!("{sha}^{{commit}}");
        let resolved = self.git(
            &self.root.clone(),
            &["rev-parse", "--verify", "--quiet", &probe],
            128,
        );
        resolved
            .ok()
            .filter(|output| is_full_sha(output.trim_end_matches('\n')))
            .map(|_| sha)
    }

    /// Captures candidate changes from each endpoint's merge base, prioritizing active worktrees.
    /// An older branch lacking newer target files is never presented as deleting those files.
    /// Ancestor endpoints contribute no new candidate changes, including retained reverted branches.
    fn endpoint_patches(
        &mut self,
        branches: &[(String, String)],
        listing: &[(PathBuf, Option<String>)],
        evidence: &mut Vec<Evidence>,
    ) {
        if !self.room() {
            return;
        }
        let mut endpoints: Vec<(String, String)> = Vec::new();
        for (_, head) in listing {
            if let Some(sha) = head
                && !endpoints.iter().any(|(_, known)| known == sha)
            {
                let label = branches
                    .iter()
                    .find(|(_, tip)| tip == sha)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| format!("detached@{}", short(sha)));
                endpoints.push((label, sha.clone()));
            }
        }
        endpoints.extend_from_slice(branches);
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (label, sha) in endpoints {
            if sha == self.target_sha || !seen.insert(sha.clone()) || !self.room() {
                continue;
            }
            let merge_base = match self.soft(
                &self.root.clone(),
                &["merge-base", &self.target_sha.clone(), &sha],
                128,
                "Candidate merge base",
            ) {
                Soft::Text(base) if is_full_sha(base.trim()) => base.trim().to_owned(),
                _ => continue,
            };
            if merge_base == sha {
                continue;
            }
            let (text, truncated) = self.diff_pair(
                &self.root.clone(),
                &[],
                &[&merge_base, &sha],
                "Endpoint patch",
            );
            let reference = format!("{label}@{sha}");
            if truncated {
                self.omit(format!(
                    "Endpoint patch {reference} has incomplete per-file coverage."
                ));
            }
            if !text.trim().is_empty() {
                push(
                    evidence,
                    "endpoint_patch",
                    &self.root.clone().display().to_string(),
                    &reference,
                    &text,
                );
            }
        }
    }

    /// Reads a stat summary plus bounded patch for one diff specification; overflow keeps
    /// the summary and is reported as truncated. Returns `(text, patch_truncated)`.
    fn diff_pair(
        &mut self,
        directory: &Path,
        prefix: &[&str],
        range: &[&str],
        what: &str,
    ) -> (String, bool) {
        let sensitive_specs: Vec<String> = DIFF_EXCLUSIONS
            .iter()
            .map(|pattern| pattern.replace(":(exclude,", ":("))
            .collect();
        let mut sensitive_args: Vec<&str> = vec![
            "diff",
            "--name-only",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
        ];
        sensitive_args.extend(prefix);
        sensitive_args.extend(range);
        sensitive_args.push("--");
        sensitive_args.extend(sensitive_specs.iter().map(String::as_str));
        if let Soft::Text(names) = self.soft(
            directory,
            &sensitive_args,
            4096,
            "Credential path inventory",
        ) && !names.is_empty()
        {
            self.omit(format!(
                "{what}: credential-container paths were excluded from evidence."
            ));
        }
        let mut stat_args: Vec<&str> = vec!["diff"];
        stat_args.extend(prefix);
        stat_args.extend(["--stat", "--no-ext-diff", "--no-textconv", "--no-color"]);
        stat_args.extend(range);
        stat_args.extend(["--", "."]);
        stat_args.extend(DIFF_EXCLUSIONS.iter().copied());
        let mut patch_args: Vec<&str> = vec!["diff"];
        patch_args.extend(prefix);
        patch_args.extend(["--patch", "--no-ext-diff", "--no-textconv", "--no-color"]);
        patch_args.extend(range);
        patch_args.extend(["--", "."]);
        patch_args.extend(DIFF_EXCLUSIONS.iter().copied());
        let stat = self.soft(directory, &stat_args, 4096, what);
        let patch = self.soft(directory, &patch_args, PATCH_CAP, what);
        match (stat, patch) {
            (Soft::Text(stat), Soft::Text(patch)) if !patch.trim().is_empty() => {
                (format!("{stat}\n{patch}"), false)
            }
            (Soft::Text(stat), Soft::Overrun) => {
                let chunks = self.file_patches(directory, prefix, range, what);
                (format!("{stat}\n{chunks}"), true)
            }
            (Soft::Text(stat), _) => (stat, false),
            (_, _) => (String::new(), false),
        }
    }

    /// Preserve bounded file patches after aggregate overflow, preferring source over docs/locks.
    /// Literal pathspecs, existing credential exclusions and cumulative budgets remain authoritative.
    fn file_patches(
        &mut self,
        directory: &Path,
        prefix: &[&str],
        range: &[&str],
        what: &str,
    ) -> String {
        let mut args = vec![
            "diff",
            "--name-only",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
        ];
        args.extend(prefix);
        args.extend(range);
        args.extend(["--", "."]);
        args.extend(DIFF_EXCLUSIONS.iter().copied());
        let Soft::Text(raw) = self.soft(directory, &args, 16 * 1024, "Changed file inventory")
        else {
            return String::new();
        };
        let mut paths: Vec<&str> = raw
            .split('\0')
            .filter(|p| safe_relative(p) && !secret_path(p))
            .collect();
        paths.sort_by_key(|path| (source_priority(path), *path));
        if paths.len() > DIFF_FILE_LIMIT {
            self.omit(format!(
                "{what}: per-file coverage includes only {DIFF_FILE_LIMIT} of {} changed files.",
                paths.len()
            ));
        }
        let mut remaining = DIFF_FILE_BYTES;
        let mut chunks = Vec::new();
        for path in paths.into_iter().take(DIFF_FILE_LIMIT) {
            if !self.room() || remaining < 1024 {
                self.omit(format!("{what}: per-file patch budget exhausted."));
                break;
            }
            if !self.changed_paths.iter().any(|p| p == path) {
                self.changed_paths.push(path.to_owned());
            }
            let literal = format!(":(literal){path}");
            let mut args = vec![
                "diff",
                "--patch",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
            ];
            args.extend(prefix);
            args.extend(range);
            args.extend(["--", literal.as_str()]);
            if let Soft::Text(text) = self.soft(
                directory,
                &args,
                PATCH_CAP.min(remaining),
                &format!("{what}: {path}"),
            ) {
                remaining = remaining.saturating_sub(text.len());
                chunks.push(text);
            }
        }
        chunks.join("\n")
    }

    /// Observe actual current target blobs for candidate paths; absence is not proof of deletion.
    /// Oversized, binary or unavailable blobs remain explicit unknown evidence.
    fn target_blobs(&mut self, evidence: &mut Vec<Evidence>) {
        for path in self.changed_paths.clone().into_iter().take(8) {
            if !self.room() {
                break;
            }
            let object = format!("{}:{path}", self.target_sha);
            let literal = format!(":(literal){path}");
            let present = self.git(
                &self.root.clone(),
                &[
                    "ls-tree",
                    "--name-only",
                    &self.target_sha.clone(),
                    "--",
                    &literal,
                ],
                2048,
            );
            if matches!(&present, Ok(names) if names.trim().is_empty()) {
                continue;
            }
            if let Soft::Text(text) = self.soft(
                &self.root.clone(),
                &["show", &object],
                DRAFT_FILE_CAP,
                "Current target file",
            ) && !text.contains('\0')
                && !text.trim().is_empty()
            {
                push(
                    evidence,
                    "target_blob",
                    &self.root.display().to_string(),
                    &format!("{}@{}:{path}", self.target_ref, self.target_sha),
                    &text,
                );
            }
        }
    }

    /// Enumerates, per tip, the commits the baseline cannot reach (deduplicated by full
    /// SHA across tips) and retains bounded messages plus author check assertions.
    fn commit_evidence(
        &mut self,
        tips: &BTreeSet<String>,
        baseline: &Snapshot,
        evidence: &mut Vec<Evidence>,
    ) {
        if !self.room() {
            return;
        }
        let exclusions = self.baseline_exclusions(baseline);
        let mut reachable: BTreeSet<String> = BTreeSet::new();
        for tip in tips {
            if !self.room() {
                break;
            }
            let mut args: Vec<String> = vec![
                "rev-list".to_owned(),
                format!("--max-count={TIP_WALK_LIMIT}"),
                tip.clone(),
            ];
            args.extend(exclusions.iter().map(|sha| format!("^{sha}")));
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            if let Soft::Text(raw) = self.soft(
                &self.root.clone(),
                &refs,
                128 * TIP_WALK_LIMIT,
                "Commit enumeration",
            ) {
                let shas: Vec<&str> = raw.lines().map(str::trim).collect();
                if shas.len() >= TIP_WALK_LIMIT {
                    self.omit(format!(
                        "Commit enumeration from {} reached its per-tip bound.",
                        short(tip)
                    ));
                }
                for sha in shas.into_iter().filter(|sha| is_full_sha(sha)) {
                    reachable.insert(sha.to_owned());
                }
            }
        }
        reachable.remove(self.target_sha.as_str());
        let total = reachable.len();
        for (index, sha) in reachable.into_iter().enumerate() {
            if index >= COMMIT_LIMIT {
                self.omit(format!(
                    "{total} new commits exceed the retained-message bound; older messages were skipped."
                ));
                break;
            }
            if !self.room() {
                break;
            }
            self.commit_message(&sha, evidence);
        }
    }

    /// Resolves baseline tips for rev-list exclusion; missing objects after rewrite or GC
    /// become omissions, and the reappearance of their history stays explicitly disclosed.
    fn baseline_exclusions(&mut self, baseline: &Snapshot) -> Vec<String> {
        let mut exclusions = Vec::new();
        for tip in baseline.tips.iter().take(BASELINE_TIP_LIMIT) {
            if !is_full_sha(tip) {
                self.omit("A baseline tip was malformed and cannot exclude history.");
                continue;
            }
            let probe = format!("{tip}^{{commit}}");
            let verified = self.git(
                &self.root.clone(),
                &["rev-parse", "--verify", "--quiet", &probe],
                128,
            );
            if verified.is_ok() {
                exclusions.push(tip.clone());
            } else {
                self.omit(format!(
                    "Baseline tip {} is missing after rewrite or GC; its history may reappear as new.",
                    short(tip)
                ));
            }
        }
        exclusions
    }

    /// Reads one bounded commit message and pushes `commit` evidence plus any `check`
    /// assertions from its `Checks:` section; malformed metadata becomes an omission.
    fn commit_message(&mut self, sha: &str, evidence: &mut Vec<Evidence>) {
        match self.soft(
            &self.root.clone(),
            &[
                "show",
                "-s",
                "--no-show-signature",
                "--format=%H%x00%B",
                sha,
                "--",
            ],
            MESSAGE_CAP + 512,
            "Commit message",
        ) {
            Soft::Text(raw) => {
                let Some((claimed, message)) = raw.split_once('\0') else {
                    self.omit("Git returned malformed commit metadata; the message was skipped.");
                    return;
                };
                if claimed.trim() != sha || message.contains('\0') {
                    self.omit("Git returned mismatched commit metadata; the message was skipped.");
                    return;
                }
                if message.len() > MESSAGE_CAP {
                    self.omit(format!(
                        "Commit {} message exceeded its bound and was truncated.",
                        short(sha)
                    ));
                }
                let text = bounded(message, MESSAGE_CAP);
                let root = self.root.display().to_string();
                push(evidence, "commit", &root, sha, &text);
                for (label, status, detail) in check_lines(&text) {
                    push(
                        evidence,
                        "check",
                        &root,
                        &format!("{sha}:{label}"),
                        &format!("{status} | {detail}"),
                    );
                }
            }
            Soft::Overrun => {
                self.omit(format!(
                    "Commit {} message exceeded its bound and was skipped.",
                    short(sha)
                ));
            }
            Soft::Failed => {}
        }
    }

    /// Captures per-worktree drafts: staged and unstaged patches, the untracked path list
    /// and bounded contents of eligible new text files. Entries identical to the baseline's
    /// dirty evidence are suppressed so preexisting work never reappears as new activity.
    fn draft_evidence(
        &mut self,
        listing: &[(PathBuf, Option<String>)],
        baseline: Option<&Snapshot>,
        evidence: &mut Vec<Evidence>,
    ) {
        for (ordinal, (path, head)) in listing.iter().enumerate() {
            if head.is_none() {
                continue;
            }
            let worktree = path.display().to_string();
            let tag = format!("wt{}", ordinal + 1);
            self.draft_patch(evidence, path, &tag, "index", true, baseline);
            self.draft_patch(evidence, path, &tag, "worktree", false, baseline);
            if !self.room() {
                continue;
            }
            let untracked = self.untracked_paths(path);
            if untracked.is_empty() {
                continue;
            }
            let listed = untracked.len().min(UNTRACKED_PATH_LIMIT);
            let names = untracked[..listed].join("\n");
            if names.len() > UNTRACKED_LIST_CAP || untracked.len() > listed {
                self.omit("The untracked filename list exceeded its bounded coverage.");
            }
            if untracked.len() > DRAFT_FILE_LIMIT {
                self.omit(format!("Untracked content omitted for {} files beyond the {DRAFT_FILE_LIMIT}-file limit.", untracked.len() - DRAFT_FILE_LIMIT));
            }
            let text = bounded(&names, UNTRACKED_LIST_CAP);
            let reference = format!("{tag}!untracked");
            if !baseline_has(baseline, "untracked", &worktree, &reference, &text) {
                push(evidence, "untracked", &worktree, &reference, &text);
            }
            for relative in untracked.iter().take(DRAFT_FILE_LIMIT) {
                if !self.room() {
                    break;
                }
                if let Some(content) = self.draft_file(path, relative) {
                    let reference = format!("{tag}!{relative}");
                    if !baseline_has(baseline, "draft_text", &worktree, &reference, &content) {
                        push(evidence, "draft_text", &worktree, &reference, &content);
                    }
                }
            }
        }
    }

    /// Reads one bounded draft patch (index or worktree) with its change summary; entries
    /// identical to the baseline observation are suppressed by `baseline_has`.
    fn draft_patch(
        &mut self,
        evidence: &mut Vec<Evidence>,
        path: &Path,
        tag: &str,
        locator: &str,
        staged: bool,
        baseline: Option<&Snapshot>,
    ) {
        if !self.room() {
            return;
        }
        let worktree = path.display().to_string();
        let kind = if staged { "staged" } else { "unstaged" };
        let prefix: &[&str] = if staged { &["--cached"] } else { &[] };
        let (text, truncated) = self.diff_pair(path, prefix, &[], "Draft patch");
        if truncated {
            self.omit(format!(
                "Draft patch {tag}!{locator} has incomplete per-file coverage."
            ));
        }
        if text.trim().is_empty() {
            return;
        }
        let reference = format!("{tag}!{locator}");
        if !baseline_has(baseline, kind, &worktree, &reference, &text) {
            push(evidence, kind, &worktree, &reference, &text);
        }
    }

    /// Lists non-ignored untracked paths from porcelain status; the parse is dropped whole on
    /// overflow (no partial `-z` records) and unsafe or malformed paths become omissions.
    fn untracked_paths(&mut self, path: &Path) -> Vec<String> {
        let mut untracked: Vec<String> = Vec::new();
        if let Soft::Text(raw) = self.soft(
            path,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            STATUS_CAP,
            "Worktree status",
        ) {
            for record in raw.split('\0') {
                let bytes = record.as_bytes();
                if bytes.len() < 4 || bytes[2] != b' ' || bytes[0] != b'?' || bytes[1] != b'?' {
                    continue;
                }
                let relative = &record[3..];
                let safe = relative.len() <= TEXT_CAP
                    && !Path::new(relative).is_absolute()
                    && !relative.split('/').any(|part| part == "..")
                    && !relative.chars().any(char::is_control);
                if !safe {
                    self.omit("An untracked path was unsafe or malformed and was skipped.");
                    continue;
                }
                untracked.push(relative.to_owned());
                if untracked.len() > UNTRACKED_PATH_LIMIT {
                    self.omit(format!(
                        "More than {UNTRACKED_PATH_LIMIT} untracked paths exist; the list was truncated."
                    ));
                    break;
                }
            }
        }
        untracked
    }

    /// Reads one bounded eligible untracked file. Symlinks, credential-named paths, binaries,
    /// non-UTF-8 content are refused. Oversized text retains labeled head/tail excerpts with
    /// explicit omissions; retained file text consumes the same cumulative evidence budget.
    fn draft_file(&mut self, worktree: &Path, relative: &str) -> Option<String> {
        if safe_relative(relative)
            && !secret_path(relative)
            && !self.changed_paths.iter().any(|path| path == relative)
        {
            self.changed_paths.push(relative.to_owned());
        }
        if secret_path(relative) {
            self.omit(format!(
                "Draft file {relative} matches credential patterns and was not read."
            ));
            return None;
        }
        let candidate = worktree.join(relative);
        let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
            return None;
        };
        if metadata.is_symlink() {
            self.omit(format!(
                "Draft file {relative} is a symlink and was not followed."
            ));
            return None;
        }
        if !metadata.is_file() {
            return None;
        }
        let (Ok(canonical), Ok(root)) = (
            std::fs::canonicalize(&candidate),
            std::fs::canonicalize(worktree),
        ) else {
            return None;
        };
        if !canonical.starts_with(root) {
            self.omit(format!(
                "Draft file {relative} escapes its worktree and was not read."
            ));
            return None;
        }
        let limit = DRAFT_FILE_CAP.min(self.remaining.saturating_sub(256));
        if limit < 1024 {
            self.omit("Draft content budget exhausted.");
            return None;
        }
        let Ok(mut file) = std::fs::File::open(&canonical) else {
            return None;
        };
        let mut bytes = Vec::new();
        if file
            .by_ref()
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return None;
        }
        if bytes.len() > limit {
            self.omit(format!("Draft file {relative} exceeded its size bound; only labeled head/tail excerpts are retained."));
            let half = limit / 2;
            let head = &bytes[..half];
            let head = match std::str::from_utf8(head) {
                Ok(text) => text,
                Err(error) if error.error_len().is_none() => {
                    std::str::from_utf8(&head[..error.valid_up_to()]).ok()?
                }
                Err(_) => return None,
            };
            let mut tail = Vec::new();
            if file.seek(SeekFrom::End(-(half as i64))).is_err()
                || file.take(half as u64).read_to_end(&mut tail).is_err()
            {
                return None;
            }
            let tail = tail
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|at| &tail[at + 1..])
                .unwrap_or(&[]);
            if bytes.contains(&0) || tail.contains(&0) {
                return None;
            }
            let tail = std::str::from_utf8(tail).ok()?;
            let excerpt = format!(
                "[Partial draft: file head; middle content omitted]\n{head}\n[Partial draft: file tail]\n{tail}"
            );
            self.remaining = self.remaining.saturating_sub(excerpt.len());
            return Some(excerpt);
        }
        if bytes.contains(&0) {
            self.omit(format!(
                "Draft file {relative} is binary and was not ingested."
            ));
            return None;
        }
        match String::from_utf8(bytes) {
            Ok(text) => {
                self.remaining = self.remaining.saturating_sub(text.len());
                Some(text)
            }
            Err(_) => {
                self.omit(format!(
                    "Draft file {relative} is not valid UTF-8 and was skipped."
                ));
                None
            }
        }
    }
}

/// Outcome of one soft Git command: complete text, an overrun whose loss is already an
/// omission, or a failure whose stable code is already an omission.
enum Soft {
    /// Complete bounded UTF-8 output.
    Text(String),
    /// Output exceeded its per-command bound; no text is retained.
    Overrun,
    /// The command failed or was unavailable; the omission carries the stable code.
    Failed,
}

/// Accept one literal repository-relative path without control characters or parent traversal.
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= TEXT_CAP
        && !Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        && !path.chars().any(char::is_control)
}

/// Prefer implementation source, then tests, then other files during bounded fallback collection.
fn source_priority(path: &str) -> u8 {
    match Path::new(path).extension().and_then(|v| v.to_str()) {
        Some("rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "go" | "java" | "c" | "h" | "cpp") => {
            u8::from(path.contains("test"))
        }
        _ => 2,
    }
}

/// Appends one evidence entry with the stable `<kind>:<reference>` citation id.
fn push(evidence: &mut Vec<Evidence>, kind: &str, worktree: &str, reference: &str, text: &str) {
    evidence.push(Evidence {
        id: format!("{kind}:{reference}"),
        kind: kind.to_owned(),
        worktree: worktree.to_owned(),
        reference: reference.to_owned(),
        text: text.to_owned(),
    });
}

/// Strips the `wt<N>!` worktree ordinal from an evidence reference locator.
fn semantic_locator(reference: &str) -> &str {
    reference
        .split_once('!')
        .map(|(_, rest)| rest)
        .unwrap_or(reference)
}

/// True when the baseline snapshot already carries this exact dirty observation. Worktree
/// ordinals may shift between snapshots, so only the locator after `!` participates.
fn baseline_has(
    baseline: Option<&Snapshot>,
    kind: &str,
    worktree: &str,
    reference: &str,
    text: &str,
) -> bool {
    baseline.is_some_and(|base| {
        base.evidence.iter().any(|entry| {
            entry.kind == kind
                && entry.worktree == worktree
                && semantic_locator(&entry.reference) == semantic_locator(reference)
                && entry.text == text
        })
    })
}

/// True when a relative draft path's file name looks like a credential container:
/// environment files, private keys, certificates, or credential/secret/password names.
fn secret_path(relative: &str) -> bool {
    let path = Path::new(relative);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name.as_str(),
            ".netrc" | ".htpasswd" | ".npmrc" | ".pypirc" | "credentials"
        )
        || matches!(
            extension.as_str(),
            "pem" | "key" | "p12" | "pfx" | "kdbx" | "jks" | "keystore"
        )
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || ["credential", "password", "secret"]
            .iter()
            .any(|needle| name.contains(needle))
}

/// Extracts `Checks:` section rows (`status | label | detail`) from a commit message.
/// This is tolerant evidence extraction, not report validation: malformed, excess or
/// unknown-status rows are ignored, capped at eight per commit.
fn check_lines(message: &str) -> Vec<(String, String, String)> {
    let mut checks: Vec<(String, String, String)> = Vec::new();
    let mut section = false;
    for line in message.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "Checks:" {
            section = true;
            continue;
        }
        if !section {
            continue;
        }
        if line.ends_with(':') && !line.contains('|') {
            section = false;
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '|').map(str::trim).collect();
        if parts.len() < 2
            || !matches!(parts[0], "passed" | "failed" | "not_run" | "not_applicable")
        {
            continue;
        }
        let (label, detail) = (parts[1], parts.get(2).copied().unwrap_or(""));
        if label.is_empty() || label.len() > 64 || label.chars().any(char::is_control) {
            continue;
        }
        if checks.len() >= 8 {
            break;
        }
        checks.push((label.to_owned(), parts[0].to_owned(), detail.to_owned()));
    }
    checks
}

/// Runs one read-only Git command with pager, replace objects, optional locks, fsmonitor,
/// prompts, lazy fetch and all inherited `GIT_*` overrides disabled. Failed commands expose
/// fixed refusals; Git stderr and repository content never become diagnostics.
fn run_git(
    root: &Path,
    args: &[&str],
    cap: usize,
    deadline: Instant,
    remaining: &mut usize,
) -> Result<String> {
    let bytes = capture(root, args, cap.min((*remaining).max(1)), deadline)?;
    *remaining = remaining
        .checked_sub(bytes.len())
        .ok_or_else(|| Error::new("git_capacity", "Git output exceeded its aggregate budget."))?;
    String::from_utf8(bytes)
        .map_err(|_| Error::new("git_source", "Git output must be valid UTF-8."))
}

/// Spawns one bounded Git command; the cap is enforced by the reader before decoding and the
/// shared deadline bounds the child. Overrun, timeout and failure kill and reap the child;
/// no stderr or stdin reaches the caller and no shell is involved.
fn capture(directory: &Path, args: &[&str], cap: usize, deadline: Instant) -> Result<Vec<u8>> {
    if Instant::now() >= deadline {
        return Err(Error::new(
            "git_timeout",
            "Git evidence collection exceeded its deadline.",
        ));
    }
    let mut command = Command::new("git");
    command
        .current_dir(directory)
        .args([
            "--no-pager",
            "--no-replace-objects",
            "--no-optional-locks",
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
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::new("git_unavailable", "Cannot start local Git."))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::new("git_source", "Cannot capture Git output."));
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::Builder::new()
        .name("observer-git-read".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take((cap + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::new(
            "git_unavailable",
            "Cannot start bounded Git output reading.",
        ));
    }
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
                        "Git output could not be read within its bounded capacity.",
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
                        "Local Git refused a read-only evidence command.",
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
                    "Cannot observe Git output reading.",
                ));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new(
                "git_timeout",
                "Git evidence collection exceeded its deadline.",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// True for a full forty- or sixty-four-character hexadecimal commit object id.
fn is_full_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Abbreviates a full SHA for omission prose; never used as a selector.
fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// Truncates at a UTF-8 char boundary, never splitting a character.
fn bounded(text: &str, maximum: usize) -> String {
    let mut end = text.len().min(maximum);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Sorts evidence by citation id, enforces the serialized frame bound with explicit dropping,
/// stamps the observation time, and computes the semantic digest. Evidence removal past the
/// bound is always an explicit omission, never silent truncation.
fn finalize(
    identity: Identity,
    target_sha: Option<String>,
    tips: Vec<String>,
    evidence: Vec<Evidence>,
    omissions: Vec<String>,
) -> Result<Snapshot> {
    let mut evidence = evidence;
    evidence.sort_by(|left, right| left.id.cmp(&right.id));
    let mut snapshot = Snapshot {
        repository: identity,
        observed_at: observed_at(),
        target_sha,
        tips,
        evidence,
        omissions,
        semantic_digest: String::new(),
    };
    loop {
        let size = serde_json::to_vec(&snapshot)
            .map(|bytes| bytes.len())
            .map_err(|_| Error::new("invalid_json", "Snapshot serialization failed."))?;
        if size <= FRAME_LIMIT {
            break;
        }
        let before = snapshot.evidence.len();
        snapshot.evidence.retain(|entry| entry.kind != "draft_text");
        if snapshot.evidence.len() == before {
            snapshot
                .evidence
                .retain(|entry| entry.kind != "endpoint_patch");
        }
        if snapshot.evidence.len() == before {
            return Err(Error::new(
                "git_capacity",
                "Evidence exceeds its serialized frame bound.",
            ));
        }
        if snapshot.omissions.len() < OMISSION_LIMIT {
            snapshot
                .omissions
                .push("Evidence was dropped to fit its serialized frame bound.".to_owned());
        }
    }
    snapshot.semantic_digest = semantic_digest(&snapshot)?;
    Ok(snapshot)
}

/// SHA-256 over semantic content only: integration ref, deduplicated tips, every evidence
/// entry with worktree paths normalized to stable tokens, and omissions. Clocks and absolute
/// worktree names are excluded, so unchanged repositories keep an unchanged digest.
fn semantic_digest(snapshot: &Snapshot) -> Result<String> {
    let root = snapshot.repository.root.display().to_string();
    let mut worktrees: Vec<&str> = snapshot
        .evidence
        .iter()
        .map(|entry| entry.worktree.as_str())
        .filter(|path| *path != root)
        .collect();
    worktrees.sort_unstable();
    worktrees.dedup();
    let token = |path: &str| -> String {
        if path == root {
            "root".to_owned()
        } else {
            format!(
                "wt{}",
                worktrees
                    .iter()
                    .position(|known| *known == path)
                    .unwrap_or(0)
                    + 1
            )
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(b"observer-evidence-v1\n");
    hasher.update(snapshot.repository.integration_ref.as_bytes());
    if let Some(target) = &snapshot.target_sha {
        hasher.update(target.as_bytes());
    }
    hasher.update(b"\n");
    for tip in &snapshot.tips {
        hasher.update(tip.as_bytes());
        hasher.update(b"\n");
    }
    for entry in &snapshot.evidence {
        let worktree_token = token(&entry.worktree);
        for part in [
            entry.kind.as_str(),
            worktree_token.as_str(),
            entry.reference.as_str(),
            entry.text.as_str(),
        ] {
            hasher.update(part.as_bytes());
            hasher.update(b"\n");
        }
    }
    for omission in &snapshot.omissions {
        hasher.update(omission.as_bytes());
        hasher.update(b"\n");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Disposable Git fixtures exercise identity, evidence, omissions, budgets and races read-only.
/// Format an observation clock without enabling Chrono's optional system-clock dependency.
/// Clock/range failures remain an explicit unknown label and never affect semantic identity.
fn observed_at() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| {
            i64::try_from(d.as_secs()).ok().and_then(|seconds| {
                chrono::DateTime::<Utc>::from_timestamp(seconds, d.subsec_nanos())
            })
        })
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated Git fixture assertions"
)]
mod tests {
    use super::*;

    /// Runs fixture-only Git with user/system configuration disabled, refusing setup failures.
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
            .env("GIT_AUTHOR_NAME", "Fixture Author")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture Committer")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }

    /// Initializes one disposable checkout with a deterministic empty initial commit on main.
    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        fixture_git(
            temp.path(),
            &["-c", "init.defaultBranch=main", "init", "--quiet"],
        );
        commit(temp.path(), "feat: initial");
        temp
    }

    /// Commits everything in the fixture with the supplied original message, returning the SHA.
    fn commit(root: &Path, message: &str) -> String {
        fixture_git(root, &["add", "--all"]);
        fixture_git(
            root,
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
        fixture_git(root, &["rev-parse", "HEAD"])
    }

    /// An oversized aggregate diff still preserves the changed implementation in smaller files.
    #[test]
    fn large_diff_retains_per_file_implementation() {
        let temp = fixture();
        let baseline = collect(temp.path(), None, None).unwrap();
        std::fs::create_dir_all(temp.path().join("docs")).unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(
            temp.path().join("docs/large.md"),
            "Unrelated historical explanation.\n".repeat(1000),
        )
        .unwrap();
        std::fs::write(
            temp.path().join("src/feature.rs"),
            "pub fn locally_ready() -> bool { true }\n",
        )
        .unwrap();
        commit(
            temp.path(),
            "Implement a small feature alongside a large document",
        );
        let observed = collect(temp.path(), Some(&baseline), None).unwrap();
        assert!(
            observed
                .evidence
                .iter()
                .any(|e| e.kind == "target_patch" && e.text.contains("pub fn locally_ready")),
            "large aggregate output must not discard all code"
        );
    }

    /// An older candidate cannot make a newer target-only file look like a candidate deletion.
    #[test]
    fn candidate_diff_does_not_delete_newer_target_files() {
        let temp = fixture();
        let baseline = collect(temp.path(), None, None).unwrap();
        fixture_git(temp.path(), &["checkout", "-b", "older-candidate"]);
        std::fs::write(
            temp.path().join("candidate.rs"),
            "pub fn candidate_ready() {}\n",
        )
        .unwrap();
        commit(temp.path(), "Implement candidate behavior");
        fixture_git(temp.path(), &["checkout", "main"]);
        std::fs::write(
            temp.path().join("new_contract.rs"),
            "pub fn newer_target_contract() {}\n",
        )
        .unwrap();
        commit(temp.path(), "Add a newer target contract");
        let observed = collect(temp.path(), Some(&baseline), None).unwrap();
        let candidate = observed
            .evidence
            .iter()
            .find(|e| e.kind == "endpoint_patch" && e.reference.starts_with("older-candidate@"))
            .unwrap();
        assert!(candidate.text.contains("candidate_ready"));
        assert!(
            !candidate.text.contains("-pub fn newer_target_contract"),
            "older target absence is not a candidate deletion"
        );
    }

    /// A modest new source file larger than the old 4 KiB limit remains observable as draft code.
    #[test]
    fn medium_untracked_source_is_not_discarded_whole() {
        let temp = fixture();
        let baseline = collect(temp.path(), None, None).unwrap();
        std::fs::write(
            temp.path().join("environment.rs"),
            format!(
                "{}pub fn local_environment() {{}}\n",
                "// Relevant environment implementation.\n".repeat(180)
            ),
        )
        .unwrap();
        let observed = collect(temp.path(), Some(&baseline), None).unwrap();
        assert!(
            observed
                .evidence
                .iter()
                .any(|e| e.kind == "draft_text" && e.text.contains("local_environment")),
            "bounded source must not disappear because a coarse file limit was hit"
        );
    }

    /// Large new files preserve bounded local-code excerpts instead of disappearing entirely.
    #[test]
    fn oversized_draft_keeps_labeled_tail_evidence() {
        let temp = fixture();
        let baseline = collect(temp.path(), None, None).unwrap();
        std::fs::write(
            temp.path().join("large_environment.rs"),
            format!(
                "{}pub fn locally_observable_tail() {{}}\n",
                "// Relevant environment input.\n".repeat(900)
            ),
        )
        .unwrap();
        let observed = collect(temp.path(), Some(&baseline), None).unwrap();
        let content = observed
            .evidence
            .iter()
            .find(|row| row.kind == "draft_text")
            .unwrap();
        assert!(content.text.contains("locally_observable_tail"));
        assert!(content.text.contains("middle content omitted"));
        assert!(
            observed
                .omissions
                .iter()
                .any(|text| text.contains("size bound"))
        );
    }

    /// Identity pins the canonical common directory/checkout and validates integration refs.
    #[test]
    fn identify_pins_common_dir_and_validates_integration_refs() {
        let temp = fixture();
        let identity = identify(temp.path(), None).unwrap();
        assert_eq!(identity.integration_ref, "refs/heads/main");
        assert_eq!(identity.root, std::fs::canonicalize(temp.path()).unwrap());
        assert!(identity.common_dir.ends_with(".git"));
        assert_eq!(identity.key.len(), 64);
        assert!(identity.key.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(identity.key, identity.common_dir.display().to_string());
        assert_eq!(
            identify(temp.path(), Some("main")).unwrap().integration_ref,
            "refs/heads/main"
        );
        assert_eq!(
            identify(temp.path(), Some("missing-ref")).unwrap_err().code,
            "git_ref"
        );
        assert_eq!(
            identify(temp.path(), Some("-oX")).unwrap_err().code,
            "git_ref"
        );
        let empty = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        assert_eq!(identify(empty.path(), None).unwrap_err().code, "git_source");
        assert_eq!(
            identify(Path::new("relative/path"), None).unwrap_err().code,
            "git_source"
        );
    }

    /// Preexisting staged/unstaged/untracked drafts are captured; secrets, binaries, symlinks
    /// and ignored files are never ingested; repeated collection is digest-stable.
    #[test]
    fn initial_snapshot_keeps_preexisting_drafts_and_refuses_secrets() {
        let temp = fixture();
        std::fs::write(temp.path().join("tracked.txt"), "v1\n").unwrap();
        commit(temp.path(), "feat: tracked base");
        std::fs::write(temp.path().join("tracked.txt"), "v2\n").unwrap();
        std::fs::write(temp.path().join("staged.txt"), "staged line\n").unwrap();
        fixture_git(temp.path(), &["add", "staged.txt"]);
        std::fs::write(temp.path().join("notes.txt"), "draft notes\n").unwrap();
        std::fs::write(temp.path().join("credentials.pem"), "PRIVATE KEY\n").unwrap();
        std::fs::write(temp.path().join("blob.bin"), [0u8, 1u8, 2u8]).unwrap();
        std::fs::write(temp.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(temp.path().join("ignored.txt"), "SECRET-IGNORED\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hostname", temp.path().join("outside.lnk")).unwrap();
        let linked = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let linked_path = linked.path().to_str().unwrap().to_owned();
        fixture_git(
            temp.path(),
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                &linked_path,
                "HEAD",
            ],
        );

        let snapshot = collect(temp.path(), None, None).unwrap();
        assert_eq!(snapshot.tips.len(), 1, "branch and detached heads dedup");
        let facts = snapshot.omissions.join("\n");
        assert!(facts.contains("credentials.pem"), "{facts}");
        assert!(facts.contains("blob.bin"), "{facts}");
        assert!(facts.contains("outside.lnk"), "{facts}");
        let kinds: Vec<_> = snapshot.evidence.iter().map(|e| e.kind.as_str()).collect();
        for kind in [
            "commit",
            "target_content",
            "target_history",
            "staged",
            "unstaged",
            "untracked",
            "draft_text",
        ] {
            assert!(kinds.contains(&kind), "missing {kind} in {kinds:?}");
        }
        let untracked = snapshot
            .evidence
            .iter()
            .find(|e| e.kind == "untracked")
            .unwrap();
        assert!(untracked.text.contains("notes.txt"));
        assert!(!untracked.text.contains("ignored.txt"));
        assert!(
            snapshot
                .evidence
                .iter()
                .any(|e| e.kind == "draft_text" && e.text.contains("draft notes"))
        );
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("PRIVATE KEY"));
        assert!(!serialized.contains("SECRET-IGNORED"));
        #[cfg(unix)]
        assert!(!serialized.contains("hostname"));
        let reference = snapshot.tips[0].clone();
        assert_eq!(
            reference,
            fixture_git(temp.path(), &["rev-parse", "HEAD"]),
            "tips are full SHAs"
        );
        let again = collect(temp.path(), None, None).unwrap();
        assert_eq!(again.semantic_digest, snapshot.semantic_digest);
        assert_eq!(again.tips, snapshot.tips);
    }

    /// Baseline reachability excludes old commits, SHA-dedups shared commits, retains author
    /// check assertions, and refuses a foreign baseline.
    #[test]
    fn baseline_reachability_excludes_old_commits_and_dedups_shas() {
        let temp = fixture();
        let base = commit(temp.path(), "feat: base\n\nChecks:\npassed | unit | ok\n");
        let snapshot = collect(temp.path(), None, None).unwrap();
        fixture_git(temp.path(), &["checkout", "-b", "feature"]);
        let shared = commit(
            temp.path(),
            "feat: shared work\n\nChecks:\npassed | unit | ok\n",
        );
        fixture_git(temp.path(), &["checkout", "main"]);
        let main_tip = commit(temp.path(), "fix: main only");
        let next = collect(temp.path(), Some(&snapshot), None).unwrap();
        let commits: Vec<&str> = next
            .evidence
            .iter()
            .filter(|e| e.kind == "commit")
            .map(|e| e.reference.as_str())
            .collect();
        assert!(commits.contains(&shared.as_str()));
        assert!(commits.contains(&main_tip.as_str()));
        assert!(!commits.contains(&base.as_str()));
        assert_eq!(commits.iter().filter(|sha| **sha == shared).count(), 1);
        assert!(next.evidence.iter().any(|e| e.kind == "check"
            && e.reference == format!("{shared}:unit")
            && e.text == "passed | ok"));
        let foreign_repo = fixture();
        let foreign = collect(foreign_repo.path(), None, None).unwrap();
        assert_eq!(
            collect(temp.path(), Some(&foreign), None).unwrap_err().code,
            "git_source"
        );
        fixture_git(temp.path(), &["branch", "-f", "feature", "main"]);
        let dedup = collect(temp.path(), Some(&snapshot), None).unwrap();
        assert_eq!(dedup.tips.len(), 1, "identical branch tips collapse");
    }

    /// A merged-then-reverted target is distinguishable from a never-merged retained branch:
    /// endpoint patches carry branch content, net target changes and revert history explicitly.
    #[test]
    fn retained_feature_branch_and_target_revert_stay_distinguishable() {
        let temp = fixture();
        commit(temp.path(), "feat: base");
        let baseline = collect(temp.path(), None, None).unwrap();
        fixture_git(temp.path(), &["checkout", "-b", "feature"]);
        std::fs::write(temp.path().join("feature.txt"), "feature line\n").unwrap();
        commit(temp.path(), "feat: feature work");
        fixture_git(temp.path(), &["checkout", "main"]);
        fixture_git(temp.path(), &["merge", "--no-ff", "--no-edit", "feature"]);
        let merged = collect(temp.path(), Some(&baseline), None).unwrap();
        assert!(
            !merged
                .evidence
                .iter()
                .any(|e| e.kind == "endpoint_patch" && e.reference.starts_with("feature@"))
        );
        let net = merged
            .evidence
            .iter()
            .find(|e| e.kind == "target_patch")
            .unwrap();
        assert!(net.text.contains("feature.txt"));
        fixture_git(temp.path(), &["revert", "--no-edit", "-m", "1", "HEAD"]);
        let reverted = collect(temp.path(), Some(&baseline), None).unwrap();
        let history = reverted
            .evidence
            .iter()
            .find(|e| e.kind == "target_history")
            .unwrap();
        assert!(history.text.contains("Revert"));
        assert!(
            !reverted.evidence.iter().any(|e| e.kind == "target_patch"),
            "reverted net target change is empty against the baseline"
        );
        assert!(
            !reverted
                .evidence
                .iter()
                .any(|e| e.kind == "endpoint_patch" && e.reference.starts_with("feature@")),
            "an ancestor branch cannot resurrect reverted work"
        );
        assert_ne!(merged.semantic_digest, reverted.semantic_digest);
    }

    /// A tip whose object is missing after rewrite/GC yields explicit partial evidence while
    /// the remaining repository is still observed.
    #[test]
    fn missing_tip_object_becomes_explicit_partial_evidence() {
        let temp = fixture();
        let _base = commit(temp.path(), "feat: base");
        fixture_git(temp.path(), &["checkout", "-b", "side"]);
        let side_tip = commit(temp.path(), "feat: side work");
        fixture_git(temp.path(), &["checkout", "main"]);
        let snapshot = collect(temp.path(), None, None).unwrap();
        std::fs::remove_file(temp.path().join(format!(
            ".git/objects/{}/{}",
            &side_tip[..2],
            &side_tip[2..]
        )))
        .unwrap();
        let partial = collect(temp.path(), Some(&snapshot), None).unwrap();
        assert!(
            partial
                .omissions
                .iter()
                .any(|omission| omission.contains("Commit enumeration"))
        );
        assert!(
            partial.evidence.iter().any(|e| e.kind == "commit"),
            "readable target commit facts remain"
        );
    }

    /// An exhausted aggregate budget returns an explicit partial snapshot, never an error
    /// and never a silent empty success.
    #[test]
    fn exhausted_budget_is_explicit_partial_evidence() {
        let temp = fixture();
        commit(temp.path(), "feat: base");
        let mut remaining = 1024;
        let snapshot = collect_until(
            temp.path(),
            None,
            None,
            Instant::now() + READ_BUDGET,
            &mut remaining,
            None,
            &[],
        )
        .unwrap();
        assert!(
            snapshot
                .omissions
                .iter()
                .any(|omission| omission.contains("budget"))
        );
    }

    /// Racing refs are retried once; a persistently moving repository stays explicitly partial.
    #[test]
    fn racing_refs_retry_then_stay_explicitly_partial() {
        let temp = fixture();
        commit(temp.path(), "feat: base");
        let counter = std::cell::Cell::new(0u32);
        let mut remaining = TOTAL_CAP;
        let once = collect_until(
            temp.path(),
            None,
            None,
            Instant::now() + READ_BUDGET,
            &mut remaining,
            Some(&|| {
                let round = counter.get();
                counter.set(round + 1);
                if round == 0 {
                    fixture_git(temp.path(), &["branch", "raced"]);
                }
            }),
            &[],
        )
        .unwrap();
        assert_eq!(
            counter.get(),
            2,
            "the collector retried after the ref mapping changed"
        );
        assert_eq!(
            once.tips.len(),
            1,
            "two refs to one commit remain deduplicated"
        );
        assert!(
            !once
                .omissions
                .iter()
                .any(|omission| omission.contains("changed during collection"))
        );

        let counter = std::cell::Cell::new(0u32);
        let mut remaining = TOTAL_CAP;
        let racing = collect_until(
            temp.path(),
            None,
            None,
            Instant::now() + READ_BUDGET,
            &mut remaining,
            Some(&|| {
                let round = counter.get();
                counter.set(round + 1);
                fixture_git(temp.path(), &["branch", &format!("raced-{round}")]);
            }),
            &[],
        )
        .unwrap();
        assert!(
            racing
                .omissions
                .iter()
                .any(|omission| omission.contains("changed during collection"))
        );
    }

    /// Credentials remain excluded in both staged and committed patches, with visible loss.
    #[test]
    fn tracked_credentials_are_not_model_evidence() {
        let temp = fixture();
        for name in [".env", "aws_credentials.ini", "production-secrets.yaml"] {
            std::fs::write(temp.path().join(name), "ORIGINAL_PRIVATE_VALUE\n").unwrap();
        }
        commit(temp.path(), "Initial configuration");
        let baseline = collect(temp.path(), None, None).unwrap();
        for name in [".env", "aws_credentials.ini", "production-secrets.yaml"] {
            std::fs::write(temp.path().join(name), "DO_NOT_SEND_TRACKED_CREDENTIAL\n").unwrap();
        }
        std::fs::write(temp.path().join("app.rs"), "fn visible() {}\n").unwrap();
        fixture_git(temp.path(), &["add", "--all"]);
        let staged = collect(temp.path(), Some(&baseline), None).unwrap();
        let encoded = serde_json::to_string(&staged).unwrap();
        assert!(!encoded.contains("DO_NOT_SEND_TRACKED_CREDENTIAL"));
        assert!(encoded.contains("visible"));
        assert!(staged.omissions.iter().any(|s| s.contains("credential")));
        commit(temp.path(), "Update configuration and application");
        let committed = collect(temp.path(), Some(&baseline), None).unwrap();
        assert!(
            !serde_json::to_string(&committed)
                .unwrap()
                .contains("DO_NOT_SEND_TRACKED_CREDENTIAL")
        );
        assert!(committed.omissions.iter().any(|s| s.contains("credential")));
    }

    /// A saved HEAD resolves to one shared branch; relative expressions pin one object.
    #[test]
    fn integration_ref_is_independent_of_query_checkout() {
        let temp = fixture();
        let saved = identify(temp.path(), Some("HEAD")).unwrap();
        assert_eq!(saved.integration_ref, "refs/heads/main");
        std::fs::write(temp.path().join("app.rs"), "fn main_version() {}\n").unwrap();
        commit(temp.path(), "Second main version");
        let expression = identify(temp.path(), Some("HEAD^")).unwrap();
        assert!(is_full_sha(&expression.integration_ref));
        let worker = temp.path().join("worker");
        fixture_git(
            temp.path(),
            &["worktree", "add", "-b", "worker", worker.to_str().unwrap()],
        );
        std::fs::write(worker.join("app.rs"), "fn worker_version() {}\n").unwrap();
        commit(&worker, "Independent worker version");
        let other = identify(&worker, Some(&saved.integration_ref)).unwrap();
        assert_eq!(other.key, saved.key);
        assert_eq!(other.integration_ref, saved.integration_ref);
        let snapshot = collect(&worker, None, Some(&saved.integration_ref)).unwrap();
        assert_eq!(
            snapshot.target_sha.unwrap(),
            fixture_git(temp.path(), &["rev-parse", "refs/heads/main"])
        );
        assert_eq!(
            identify(&worker, Some(&expression.integration_ref))
                .unwrap()
                .integration_ref,
            expression.integration_ref
        );
    }

    /// Missing optional history excerpts do not erase the explicitly pinned starting commit.
    #[test]
    fn baseline_target_pin_survives_history_omission() {
        let temp = fixture();
        let mut baseline = collect(temp.path(), None, None).unwrap();
        baseline.evidence.retain(|e| e.kind != "target_history");
        std::fs::write(temp.path().join("app.rs"), "fn added_after_start() {}\n").unwrap();
        commit(temp.path(), "Add actual implementation");
        let current = collect(temp.path(), Some(&baseline), None).unwrap();
        assert!(
            current
                .evidence
                .iter()
                .any(|e| e.kind == "target_patch" && e.text.contains("added_after_start"))
        );
    }

    /// File-count and filename-byte ceilings are visible instead of looking like full coverage.
    #[test]
    fn untracked_content_and_list_limits_are_explicit() {
        let temp = fixture();
        for i in 0..60 {
            std::fs::write(
                temp.path().join(format!("{}-{i}.rs", "n".repeat(180))),
                "fn draft() {}\n",
            )
            .unwrap();
        }
        let snapshot = collect(temp.path(), None, None).unwrap();
        assert!(
            snapshot
                .omissions
                .iter()
                .any(|s| s.contains("Untracked content omitted"))
        );
        assert!(
            snapshot
                .omissions
                .iter()
                .any(|s| s.contains("filename list"))
        );
    }

    /// The tolerant check parser keeps well-formed rows and skips malformed ones.
    #[test]
    fn check_parsing_is_tolerant_evidence_extraction() {
        let checks = check_lines(
            "Checks:\npassed | unit | ok\nbroken row\nfailed | build\nOther:\npassed | late\n",
        );
        assert_eq!(checks.len(), 2);
        assert_eq!(
            checks[0],
            ("unit".to_owned(), "passed".to_owned(), "ok".to_owned())
        );
        assert_eq!(
            checks[1],
            ("build".to_owned(), "failed".to_owned(), String::new())
        );
        assert!(secret_path(".env"));
        assert!(secret_path("certs/server.pem"));
        assert!(secret_path("id_ed25519.pub"));
        assert!(!secret_path("src/main.rs"));
        assert!(is_full_sha(&"a".repeat(40)));
        assert!(!is_full_sha(&"g".repeat(40)));
        assert_eq!(bounded("héllo", 4), "hél");
        assert_eq!(bounded("héllo", 3), "hé");
        assert_eq!(bounded("héllo", 2), "h");
    }
}
