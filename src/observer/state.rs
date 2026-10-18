//! Atomic bounded repository records. One advisory lock protects plan CAS and admission intent.
use super::{
    error::{Error, Result},
    git,
    report::Analysis,
    run,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Maximum atomic state size, including baseline and retained immutable bundles.
const STATE_CAP: u64 = 8 * 1024 * 1024;
/// Counter used only to avoid same-process temporary-name collisions.
static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A stable high-level outcome. IDs survive edits and never appear in normal owner output.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    /// Repository-local monotonic identity; never reused after removal.
    pub id: String,
    /// Owner-agreed high-level result, without execution bookkeeping.
    pub text: String,
}

/// Exactly one active agreed plan for the canonical Git common directory.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Original planning agent's canonical Bridge return address, captured from MCP metadata.
    /// Missing host metadata/older records remain explicitly unknown; revisions preserve it.
    #[serde(default)]
    pub creator_address: Option<String>,
    /// Monotonic CAS revision, including genuinely new plans.
    pub revision: u64,
    /// Agreed purpose, retained verbatim within the documented byte bound.
    pub goal: String,
    /// Flat, nonempty agreed scope.
    pub outcomes: Vec<Outcome>,
    /// Time of this planning interaction, in UTC seconds since Unix epoch.
    pub saved_at: String,
    /// Initial observation retained independently of report eviction.
    pub baseline: git::Snapshot,
    /// IDs added by the latest planning change.
    pub added: Vec<String>,
    /// IDs removed by the latest planning change.
    pub removed: Vec<String>,
}

/// One frozen model input. Observation clocks do not participate in its semantic identity.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    /// Immutable plan revision interpreted by this analysis.
    pub plan: Plan,
    /// Frozen Git evidence, with explicit omissions.
    pub snapshot: git::Snapshot,
    /// Bounded relevant runtime observations; failures are explicit in this value.
    pub developers: Value,
    /// Digest of plan/evidence/route/prompt semantics, excluding collection time.
    pub input_id: String,
}

/// Durable original admission; never reconstruct caller binding during recovery.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    /// Original broker endpoint; recovery never sends unknown admission to another broker.
    pub socket: PathBuf,
    /// Frozen input, also used to validate late results.
    pub bundle: Bundle,
    /// Complete original Agent Run start parameters including schema and binding.
    pub request: Value,
    /// Accepted execution identity, absent only after uncertain admission.
    pub receipt: Option<run::Receipt>,
    /// Confirmed terminal failure; subsequent reads never poll this execution.
    pub terminal: Option<String>,
    /// Explicit owner retry generation, unchanged on unknown-admission replay.
    pub generation: u64,
}

/// Validated result and its original evidence, retained together or expired together.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// Stable Agent Run ID, used for pinned details reads.
    pub analysis_id: String,
    /// Exact immutable input interpreted by this report.
    pub bundle: Bundle,
    /// Strictly validated model rows; Rust owns all totals.
    pub analysis: Analysis,
    /// Exact submitted task, including schema and previous comparison context, retained with
    /// this result. None identifies private pre-qualification records that did not store it.
    #[serde(default)]
    pub model_task: Option<String>,
    /// Local completion observation, not an execution duration claim.
    pub completed_at: String,
}

/// Minimal immutable archive entry; old detailed handles expire on a new plan.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// Original planning agent, preserved when a new plan replaces this one.
    #[serde(default)]
    pub creator_address: Option<String>,
    /// Archived revision.
    pub revision: u64,
    /// Original agreed goal.
    pub goal: String,
    /// Original agreed scope text and stable IDs.
    pub outcomes: Vec<Outcome>,
    /// Last compact summary when available, without old evidence bundles.
    pub last_summary: Option<String>,
}

/// Small per-repository record. At most two successes and one admission are retained.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    /// One original pending/final Bridge publication; unresolved custody is never evicted.
    #[serde(default)]
    pub delivery: Option<super::bridge::Delivery>,
    /// Active owner-agreed plan, absent before first save.
    pub plan: Option<Plan>,
    /// Latest two successful analyses, oldest first; late results never reorder newer inputs.
    pub reports: Vec<Report>,
    /// At most one pending or terminal-failed admission intent.
    pub pending: Option<Pending>,
    /// Bounded immutable minimal plan archives; refuse a 65th archive.
    pub archives: Vec<Archive>,
    /// Last allocated stable outcome number.
    pub next_outcome: u64,
    /// Last explicit analysis attempt generation.
    pub generation: u64,
}

/// Repository lock owner. Drop releases the advisory lock, including on errors.
pub struct Locked {
    /// Exclusive lock held throughout synchronous bounded collection/admission.
    _lock: File,
    /// Atomic data destination.
    path: PathBuf,
    /// Validated current bounded record.
    pub state: State,
}

impl Locked {
    /// Open `home/observer-v1/key`; reject unsafe keys/symlinks and busy repositories.
    /// Creates only observer storage, never legacy data or repository source files.
    pub fn open(home: &Path, key: &str) -> Result<Self> {
        Self::open_mode(home, key, false)
    }

    /// Wait for the short repository transaction from a dedicated continuation process only.
    pub fn wait(home: &Path, key: &str) -> Result<Self> {
        Self::open_mode(home, key, true)
    }

    /// Share path validation and record loading; MCP callers refuse busy instead of blocking.
    fn open_mode(home: &Path, key: &str, wait: bool) -> Result<Self> {
        if !home.is_absolute()
            || key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::new(
                "storage_identity",
                "Observer home must be absolute and repository identity safe.",
            ));
        }
        let directory = home.join("observer-v1").join(key);
        safe_directory(&directory)?;
        let lock_path = directory.join("write.lock");
        reject_symlink(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        if wait {
            lock.lock_exclusive()
        } else {
            lock.try_lock_exclusive()
        }
        .map_err(|_| {
            Error::new(
                "busy",
                "Another observer request holds this repository; retry explicitly.",
            )
        })?;
        let path = directory.join("state.json");
        reject_symlink(&path)?;
        let state = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(STATE_CAP + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > STATE_CAP {
                    return Err(Error::new(
                        "state_limit",
                        "Observer record exceeds its storage bound.",
                    ));
                }
                let state: State = serde_json::from_slice(&bytes)?;
                if state.reports.len() > 2 || state.archives.len() > 64 {
                    return Err(Error::new(
                        "invalid_state",
                        "Observer retention invariants are invalid.",
                    ));
                }
                state
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            _lock: lock,
            path,
            state,
        })
    }

    /// Claim the continuation lease while the repository lock prevents an exit/admission race.
    /// None means an existing worker owns it; the successful file is transferred to child stdin.
    pub fn completion_lease(&self) -> Result<Option<File>> {
        let path = self.path.with_file_name("completion.lock");
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(file)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Verify the inherited regular file refers to this repository's exact lease inode.
    pub fn matches_completion_lease(&self, file: &File) -> Result<bool> {
        use std::os::unix::fs::MetadataExt;
        let expected = fs::symlink_metadata(self.path.with_file_name("completion.lock"))?;
        let actual = file.metadata()?;
        Ok(expected.is_file()
            && actual.is_file()
            && expected.dev() == actual.dev()
            && expected.ino() == actual.ino())
    }

    /// Publish the complete record with fsync/rename under the held lock.
    /// An error after rename reports uncertain durability; admission must not precede a confirmed save.
    pub fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.state)?;
        if bytes.len() as u64 > STATE_CAP {
            return Err(Error::new(
                "state_limit",
                "Observer record exceeds its storage bound; nothing published.",
            ));
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary =
            self.path
                .with_file_name(format!("state.{}.{}.tmp", std::process::id(), sequence));
        let mut published = false;
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            published = true;
            File::open(self.path.parent().ok_or_else(|| {
                Error::new("storage_identity", "Missing observer storage parent.")
            })?)?
            .sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            if published {
                return Err(Error::new(
                    "state_outcome_unknown",
                    "State was replaced but durability is unconfirmed; read current state before retrying.",
                ));
            }
        }
        result
    }
}

/// Reject existing symlinks; missing paths are permitted for explicit creation.
fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(Error::new(
            "storage_symlink",
            "Observer storage cannot traverse symlinks.",
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Create missing directory components without following an existing symlink.
fn safe_directory(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(Error::new(
                "storage_identity",
                "Observer home cannot contain parent traversal.",
            ));
        }
        current.push(component);
        reject_symlink(&current)?;
        if !current.exists() {
            fs::create_dir(&current)?;
        }
    }
    Ok(())
}

/// SHA-256 of serialized semantic data; serialization failure is never a cache hit.
pub fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

/// Observation label in UTC Unix seconds; clock errors are explicit unknown labels.
pub fn now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or_else(|_| "unknown".to_owned(), |d| d.as_secs().to_string())
}
