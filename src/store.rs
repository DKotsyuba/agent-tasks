//! Portable bounded file access, optimistic versions and cooperating-writer publication.
use crate::model::{self, Module, Project};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

/// Maximum serialized record, including preserved review history.
pub const RECORD_CAP: usize = 512 * 1024;
/// Headroom retained by nonterminal writes for final review/cancellation.
pub const CLOSING_RESERVE: usize = 32 * 1024;
/// Aggregate reads stop before exceeding this byte budget.
pub const SCAN_CAP: usize = 16 * 1024 * 1024;
/// Allocation/aggregate filename inventory limit.
pub const MODULE_CAP: usize = 512;
/// Process-local exclusive temp suffix; never used as a work identity.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Safe domain failure. Effects are separately retained by the call's execution ledger.
#[derive(Debug)]
pub struct Error {
    /// Stable machine code.
    pub code: &'static str,
    /// Bounded meaningful explanation; no raw serialized source.
    pub message: String,
}
impl Error {
    /// Construct a business/configuration/storage refusal without inventing an effect.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
/// Application result with a compact classified error.
pub type Result<T> = std::result::Result<T, Error>;
/// Convert semantic validation failures without exposing parser internals.
pub fn invalid(message: impl Into<String>) -> Error {
    Error::new("invalid_data", message)
}

/// Capture only the config location; discovery never reads configuration.
#[derive(Clone)]
pub struct Config {
    /// Absolute operator-selected settings path; the registry lives beside it.
    path: Option<PathBuf>,
}
/// Closed settings, separate from the machine-managed project registry.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// Understood settings revision.
    schema_version: u32,
}
/// Named portable roots; names/descriptions remain authoritative in project manifests.
#[derive(serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Aliases {
    /// Understood registry revision.
    pub schema_version: u32,
    /// Independent absolute roots, with no process-global current project.
    pub aliases: BTreeMap<String, PathBuf>,
}
impl Config {
    /// Choose flag, then AGENT_TASKS_CONFIG, then HOME/.agent-tasks/config.toml without I/O.
    pub fn new(flag: Option<PathBuf>) -> Self {
        Self {
            path: flag
                .or_else(|| std::env::var_os("AGENT_TASKS_CONFIG").map(PathBuf::from))
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|h| PathBuf::from(h).join(".agent-tasks/config.toml"))
                }),
        }
    }

    /// Validate optional closed settings and select the sibling registry; never create files.
    /// Missing settings use revision 1. Legacy inline aliases must be moved explicitly.
    pub fn registry_path(&self) -> Result<PathBuf> {
        let path = self
            .path
            .as_ref()
            .filter(|p| p.is_absolute())
            .ok_or_else(|| {
                Error::new(
                    "config",
                    "Select an absolute --config path or AGENT_TASKS_CONFIG.",
                )
            })?;
        if matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("projects.toml" | "projects.lock")
        ) {
            return Err(Error::new(
                "config",
                "Settings and registry paths must be distinct.",
            ));
        }
        if let Some(bytes) = read_path(path, 64 * 1024)? {
            let settings: Settings = toml::from_str(std::str::from_utf8(&bytes)
                .map_err(|_| Error::new("config", "Settings must be UTF-8."))?)
                .map_err(|_| Error::new("config", "Invalid settings. Move [aliases] to sibling projects.toml; config.toml contains schema_version only."))?;
            if settings.schema_version != 1 {
                return Err(Error::new("config", "Unsupported settings schema_version."));
            }
        }
        Ok(path.with_file_name("projects.toml"))
    }

    /// Read at most 64 KiB and 256 aliases with their exact observed bytes.
    /// Missing registry is an empty list; malformed data refuses rather than hiding projects.
    pub fn aliases(&self) -> Result<(Aliases, Option<Vec<u8>>)> {
        let bytes = read_path(&self.registry_path()?, 64 * 1024)?;
        let registry = match &bytes {
            Some(bytes) => toml::from_str::<Aliases>(
                std::str::from_utf8(bytes)
                    .map_err(|_| Error::new("config", "Registry must be UTF-8."))?,
            )
            .map_err(|_| {
                Error::new(
                    "config",
                    "Invalid closed project registry; expected schema_version and [aliases].",
                )
            })?,
            None => Aliases {
                schema_version: 1,
                aliases: BTreeMap::new(),
            },
        };
        if registry.schema_version != 1 || registry.aliases.len() > 256 {
            return Err(Error::new(
                "config",
                "Unsupported registry revision or more than 256 aliases.",
            ));
        }
        for (alias, root) in &registry.aliases {
            model::text(alias, 128).map_err(invalid)?;
            if !root.is_absolute() {
                return Err(Error::new("config", "Registry roots must be absolute."));
            }
        }
        Ok((registry, bytes))
    }

    /// Explicitly prepare the settings directory and missing settings for registration.
    /// Existing settings are never normalized or silently migrated; registry publication is separate.
    pub fn prepare_registry(&self, effects: &mut Vec<String>) -> Result<Store> {
        let path = self.registry_path()?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::new("config", "Registry has no parent."))?;
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|_| Error::new("io", "Cannot prepare settings directory."))?;
            effects.push("Created settings directory.".into());
        }
        let store = Store::from_root(parent)?;
        let settings = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|p| p.to_str())
            .ok_or_else(|| Error::new("config", "Settings filename must be UTF-8."))?;
        if store.bytes(settings)?.is_none() {
            store.publish(settings, b"schema_version = 1\n", None, effects)?;
        }
        Ok(store)
    }

    /// Reload the registry and resolve one alias once for the entire business request.
    pub fn resolve(&self, alias: &str) -> Result<Store> {
        model::text(alias, 128).map_err(invalid)?;
        let (registry, _) = self.aliases()?;
        let root = registry.aliases.get(alias).ok_or_else(|| {
            Error::new(
                "alias",
                "Unknown project alias. Use get_project_list or register_project.",
            )
        })?;
        Store::from_root(root)
    }
}

/// Own one acquired advisory lock; explicit unlock releases it even while a forked child
/// temporarily retains the file descriptor. Never constructed for a failed acquisition.
pub struct LockGuard {
    /// Acquired open-file description; callers cannot unlock another owner's handle.
    file: File,
}

/// A cloned/inherited descriptor must not prolong the parent's completed lock lifetime.
#[cfg(test)]
#[test]
#[allow(clippy::unwrap_used, reason = "Disposable lock regression")]
fn lock_release_survives_a_duplicate_descriptor() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::from_root(dir.path()).unwrap();
    let guard = store
        .file_lock("lock", true, &mut Vec::new())
        .unwrap()
        .unwrap();
    let inherited = guard.file.try_clone().unwrap();
    drop(guard);
    let next = store.file_lock("lock", true, &mut Vec::new());
    assert!(
        next.is_ok(),
        "Closed parent must release despite an inherited descriptor"
    );
    drop(inherited);
}

impl Drop for LockGuard {
    /// Release this owned lock before closing the descriptor; close remains the error fallback.
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Root identity held through a request; tools never accept arbitrary relative paths.
#[derive(Clone)]
pub struct Store {
    /// Canonical existing root or canonical parent plus one absent final directory name.
    pub root: PathBuf,
}

/// Exact observed record and its whole-file version; no stale field-level patches.
pub struct Snapshot<T> {
    /// Parsed validated content.
    pub value: T,
    /// Exact original bytes, retained for compare/backup.
    pub bytes: Vec<u8>,
    /// Opaque root-bound version.
    pub version: String,
}

/// Bounded filename coverage; unknown entries do not become nonexistent work.
pub struct Inventory {
    /// Canonical IDs in numeric order.
    pub ids: Vec<String>,
    /// Named coverage warnings, including ignored orphan publication temps.
    pub warnings: Vec<String>,
    /// Whether every eligible filename was inventoried.
    pub complete: bool,
}

/// Aggregate read retaining healthy files and named unreadable rows.
pub struct Scan {
    /// Healthy parsed modules in numeric order.
    pub modules: Vec<Snapshot<Module>>,
    /// Named unreadable/unscanned modules.
    pub unreadable: Vec<String>,
    /// Inventory warnings.
    pub warnings: Vec<String>,
    /// Whether all work records were successfully read.
    pub complete: bool,
    /// Scope-bound snapshot for deterministic continuations.
    pub version: String,
}

/// Generate readable UTC RFC 3339 using the OS wall clock; event IDs define ordering.
pub fn now() -> String {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Resolve an existing directory or one absent final child, never create ancestors.
fn prospective(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(Error::new(
            "root",
            "Root must be absolute without dot path components.",
        ));
    }
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => {
            fs::canonicalize(path).map_err(|_| Error::new("root", "Cannot canonicalize root."))
        }
        Ok(_) => Err(Error::new("root", "Configured root is not a directory.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(Error::new(
                    "root",
                    "Dangling root symlink is not an absent root.",
                ));
            }
            let parent = path
                .parent()
                .ok_or_else(|| Error::new("root", "Root has no parent."))?;
            let parent = fs::canonicalize(parent)
                .map_err(|_| Error::new("root", "Root parent must already exist."))?;
            if !parent.is_dir() {
                return Err(Error::new("root", "Root parent is not a directory."));
            }
            Ok(parent.join(
                path.file_name()
                    .ok_or_else(|| Error::new("root", "Invalid final root name."))?,
            ))
        }
        Err(_) => Err(Error::new("root", "Cannot inspect configured root.")),
    }
}

/// Read cap+one before parsing and reject symlink/nonregular leaves, including dangling links.
fn read_path(path: &Path, cap: usize) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::new("io", "Cannot inspect record.")),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
            return Err(Error::new(
                "file_type",
                "Owned record must be a regular file, not a link.",
            ));
        }
        Ok(m) if m.len() > cap as u64 => {
            return Err(Error::new("capacity", "Record exceeds its read cap."));
        }
        _ => (),
    }
    let file = File::open(path).map_err(|_| Error::new("io", "Cannot open record."))?;
    let mut bytes = Vec::new();
    file.take((cap + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::new("io", "Cannot read record."))?;
    if bytes.len() > cap {
        return Err(Error::new("capacity", "Record grew beyond its read cap."));
    }
    Ok(Some(bytes))
}

/// Reject unsupported YAML syntax before parsing aliases can expand. This is a
/// deliberately conservative subset gate, not a YAML lexer: quoted and block
/// prose are accepted, special anchor/tag/alias tokens in plain text are refused.
fn yaml_subset(bytes: &[u8]) -> Result<()> {
    let source = std::str::from_utf8(bytes).map_err(|_| invalid("YAML must be UTF-8."))?;
    let mut quote = None;
    let mut block_indent = None;
    let mut documents = 0;
    for line in source.lines() {
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        if let Some(base) = block_indent {
            if line.trim().is_empty() || indent > base {
                continue;
            }
            block_indent = None;
        }
        if line.trim() == "---" {
            documents += 1;
            if documents > 1 {
                return Err(invalid("One YAML document is allowed."));
            }
        }
        let mut chars = line.char_indices().peekable();
        let mut previous = ' ';
        let mut escape = false;
        while let Some((_, c)) = chars.next() {
            if let Some(q) = quote {
                if q == '"' && c == '\\' && !escape {
                    escape = true;
                    continue;
                }
                if c == q && !escape {
                    if q == '\'' && chars.peek().is_some_and(|(_, n)| *n == '\'') {
                        chars.next();
                    } else {
                        quote = None;
                    }
                }
                escape = false;
                previous = c;
                continue;
            }
            if c == '#' && previous.is_whitespace() {
                break;
            }
            if (c == '\'' || c == '"') && (previous.is_whitespace() || "[{:,-".contains(previous)) {
                quote = Some(c);
            }
            if matches!(c, '&' | '*' | '!')
                && (previous.is_whitespace() || "[{,:".contains(previous))
            {
                return Err(invalid(
                    "YAML tags, anchors and aliases are unsupported; quote literal prose.",
                ));
            }
            if matches!(c, '|' | '>') && previous.is_whitespace() {
                block_indent = Some(indent);
            }
            previous = c;
        }
    }
    Ok(())
}

/// Bound parsed nesting and require ordinary string-key maps; tags/merges are refused.
fn yaml_tree(value: &serde_yaml_ng::Value, depth: usize) -> Result<()> {
    use serde_yaml_ng::Value;
    if depth > 16 {
        return Err(invalid("YAML nesting exceeds 16 levels."));
    }
    match value {
        Value::Tagged(_) => return Err(invalid("YAML tags are unsupported.")),
        Value::Sequence(s) => {
            for v in s {
                yaml_tree(v, depth + 1)?;
            }
        }
        Value::Mapping(m) => {
            for (k, v) in m {
                if !matches!(k, Value::String(s) if s != "<<") {
                    return Err(invalid(
                        "YAML keys must be ordinary strings; merges are unsupported.",
                    ));
                }
                yaml_tree(v, depth + 1)?;
            }
        }
        _ => (),
    }
    Ok(())
}

/// Decode the closed YAML subset, rejecting duplicates before typed deserialization.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    yaml_subset(bytes)?;
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(bytes).map_err(|_| invalid("Invalid YAML or duplicate keys."))?;
    yaml_tree(&value, 0)?;
    serde_yaml_ng::from_value(value)
        .map_err(|_| invalid("Invalid fields, types or missing required schema data."))
}

/// Encode canonical YAML; callers enforce semantic validation before publication.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_yaml_ng::to_string(value)
        .map(String::into_bytes)
        .map_err(|_| Error::new("encoding", "Cannot encode owned record."))
}

impl Store {
    /// Resolve an absolute existing root or one missing final child without creating it.
    pub fn from_root(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(Error::new(
                "path",
                "Documentation directory must be absolute.",
            ));
        }
        Ok(Self {
            root: prospective(root)?,
        })
    }

    /// Validate each owned path component, refusing links and non-directory ancestors.
    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        if prospective(&self.root)? != self.root {
            return Err(Error::new(
                "root_changed",
                "Root identity changed; resolve the alias again.",
            ));
        }
        let mut path = self.root.clone();
        let parts: Vec<_> = Path::new(relative).components().collect();
        for (i, part) in parts.iter().enumerate() {
            let Component::Normal(part) = part else {
                return Err(Error::new("path", "Invalid owned relative path."));
            };
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(Error::new("file_type", "Symlink in owned path is refused."));
                }
                Ok(m) if i + 1 < parts.len() && !m.is_dir() => {
                    return Err(Error::new("file_type", "Owned parent is not a directory."));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err(Error::new("io", "Cannot inspect owned path.")),
                _ => (),
            }
        }
        Ok(path)
    }

    /// Read exact owned bytes with no filesystem effects.
    pub fn bytes(&self, relative: &str) -> Result<Option<Vec<u8>>> {
        read_path(&self.path(relative)?, RECORD_CAP)
    }

    /// Root/target-bound digest over exact observed bytes or an explicit absence marker.
    pub fn version(&self, relative: &str, bytes: Option<&[u8]>) -> String {
        let mut digest = Sha256::new();
        for value in [
            b"agent-tasks/file/v1".as_slice(),
            self.root.as_os_str().as_encoded_bytes(),
            relative.as_bytes(),
            if bytes.is_some() {
                b"present"
            } else {
                b"absent"
            },
            bytes.unwrap_or_default(),
        ] {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
        }
        format!("{:x}", digest.finalize())
    }

    /// Refuse stale writes before publication; callers get a fresh context rather than replay.
    pub fn expect(&self, relative: &str, expected: &str) -> Result<()> {
        let current = self.bytes(relative)?;
        let version = self.version(relative, current.as_deref());
        if version != expected {
            return Err(Error::new(
                "stale",
                format!(
                    "No work saved. Current version: {version}. Read get_context before retrying."
                ),
            ));
        }
        Ok(())
    }

    /// Read a manifest and validate its closed semantic contract.
    pub fn project(&self) -> Result<Option<Snapshot<Project>>> {
        self.snapshot("project.yaml", Project::validate)
    }

    /// Read one module, not an aggregate; corrupt siblings cannot hide healthy context.
    pub fn module(&self, id: &str) -> Result<Snapshot<Module>> {
        let relative = model::work_path(id).map_err(invalid)?;
        let mut snapshot = self
            .snapshot(&relative, Module::validate)?
            .ok_or_else(|| Error::new("not_found", "Module does not exist."))?;
        if snapshot.value.id != id {
            return Err(invalid("Module ID does not match its filename."));
        }
        snapshot.version = self.work_version(&snapshot.value, &snapshot.bytes)?;
        Ok(snapshot)
    }

    /// Bind Epic/Atomic writes and read snapshots to direct and transitive integration observations.
    /// Missing/malformed dependencies remain digest inputs, while unknown work blocks acceptance.
    /// Reads are bounded by the shared aggregate byte cap; no file is migrated or repaired.
    pub fn work_version(&self, value: &Module, bytes: &[u8]) -> Result<String> {
        let own = self.version(&model::work_path(&value.id).map_err(invalid)?, Some(bytes));
        if value.modules.is_empty()
            && value.atomic_members.is_empty()
            && value.participants.is_empty()
            && value.workflow.as_ref().is_none_or(|w| {
                w.dependencies.is_empty()
                    && w.contracts
                        .as_ref()
                        .is_none_or(|c| c.provides.is_empty() && c.consumes.is_empty())
            })
        {
            return Ok(own);
        }
        let mut digest = Sha256::new();
        digest.update(own.as_bytes());
        let mut remaining = SCAN_CAP;
        let mut visited = std::collections::BTreeSet::new();
        for id in value
            .modules
            .iter()
            .chain(&value.atomic_members)
            .chain(&value.participants)
        {
            self.dependency_digest(id, &mut digest, &mut remaining, &mut visited)?;
        }
        if let Some(w) = &value.workflow {
            for id in w.dependencies.iter().map(|d| &d.reference).chain(
                w.contracts
                    .iter()
                    .flat_map(|c| c.provides.iter().chain(&c.consumes))
                    .map(|c| &c.peer),
            ) {
                self.dependency_digest(id, &mut digest, &mut remaining, &mut visited)?;
            }
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    /// Add a canonical dependency and standalone Atomic participants to a bounded digest.
    /// Kind restrictions prevent cycles: Epic -> Module/Atomic and Atomic -> Module only.
    fn dependency_digest(
        &self,
        id: &str,
        digest: &mut Sha256,
        remaining: &mut usize,
        visited: &mut std::collections::BTreeSet<String>,
    ) -> Result<()> {
        if !visited.insert(id.into()) {
            return Ok(());
        }
        let relative = model::work_path(id).map_err(invalid)?;
        digest.update(id.as_bytes());
        let path = match self.path(&relative) {
            Ok(path) => path,
            Err(e) => {
                digest.update(e.code.as_bytes());
                digest.update(e.message.as_bytes());
                return Ok(());
            }
        };
        match read_path(&path, RECORD_CAP.min(*remaining)) {
            Ok(bytes) => {
                digest.update(self.version(&relative, bytes.as_deref()).as_bytes());
                if let Some(bytes) = bytes {
                    *remaining = remaining.saturating_sub(bytes.len());
                    if id.starts_with("A-") || id.starts_with("E-") || id.starts_with("M-") {
                        match decode::<Module>(&bytes).and_then(|m| {
                            m.validate().map_err(invalid)?;
                            Ok(m)
                        }) {
                            Ok(m) => {
                                for related in m
                                    .participants
                                    .iter()
                                    .chain(&m.modules)
                                    .chain(&m.atomic_members)
                                {
                                    self.dependency_digest(related, digest, remaining, visited)?;
                                }
                                if let Some(w) = &m.workflow {
                                    for related in
                                        w.dependencies.iter().map(|d| &d.reference).chain(
                                            w.contracts
                                                .iter()
                                                .flat_map(|c| c.provides.iter().chain(&c.consumes))
                                                .map(|c| &c.peer),
                                        )
                                    {
                                        self.dependency_digest(
                                            related, digest, remaining, visited,
                                        )?;
                                    }
                                }
                            }
                            Err(e) => digest.update(e.code.as_bytes()),
                        }
                    }
                }
            }

            Err(e) => {
                digest.update(e.code.as_bytes());
                digest.update(e.message.as_bytes());
            }
        }
        Ok(())
    }

    /// Capture participating Module semantic state and generation; no Git commit is invented.
    pub fn participant_basis(&self, participants: &[String]) -> Result<BTreeMap<String, String>> {
        let mut result = BTreeMap::new();
        for id in participants {
            model::number(id, "M-").map_err(invalid)?;
            let m = self.module(id)?.value;
            let mut digest = Sha256::new();
            digest.update(m.basis().map_err(invalid)?.as_bytes());
            digest.update(m.review_epoch.to_le_bytes());
            if let Some(w) = &m.workflow {
                digest.update([u8::from(m.delivered())]);
                if let Some(d) = &w.delivery {
                    digest.update(d.target_branch.as_bytes());
                    digest.update(d.basis.as_bytes());
                }
            }
            result.insert(id.clone(), format!("{:x}", digest.finalize()));
        }
        Ok(result)
    }

    /// Compute the acceptance basis, extending Epic intent with current member evidence and generations.
    pub fn work_basis(&self, m: &Module) -> Result<String> {
        let own = m.basis().map_err(invalid)?;
        if !m.id.starts_with("E-") {
            return Ok(own);
        }
        let mut digest = Sha256::new();
        digest.update(own.as_bytes());
        for id in m.modules.iter().chain(&m.atomic_members) {
            let child = self.module(id)?.value;
            digest.update(id.as_bytes());
            digest.update(child.basis().map_err(invalid)?.as_bytes());
            digest.update(child.review_epoch.to_le_bytes());
            digest.update(self.phase(&child).as_bytes());
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    /// Remaining local and referenced acceptance requirements; unreadable work remains a named condition.
    pub fn acceptance(&self, m: &Module) -> Vec<String> {
        let mut missing = m.acceptance();
        if m.id.starts_with("E-") {
            if let Err(e) = self.membership(m) {
                missing.push(format!("Membership: {}", e.message));
            }
            for id in m.modules.iter().chain(&m.atomic_members) {
                match self.module(id) {
                    Ok(child) if child.value.state == model::ModuleState::Canceled => (),
                    Ok(child) => {
                        let phase = self.phase(&child.value);
                        let expected = if id.starts_with("M-") || child.value.modern() || m.modern()
                        {
                            "accepted"
                        } else {
                            "done"
                        };
                        if phase != expected {
                            missing
                                .push(format!("{id}: requires current {expected}; now {phase}."));
                        }
                    }
                    Err(e) => missing.push(format!("{id}: unknown member work ({})", e.message)),
                }
            }
        }
        if m.id.starts_with("E-") && m.modern() {
            let active = m
                .modules
                .iter()
                .filter(|id| {
                    self.module(id)
                        .is_ok_and(|s| s.value.state != model::ModuleState::Canceled)
                })
                .collect::<Vec<_>>();
            if !active.is_empty() {
                let complete = m.atomic_members.iter().any(|id| {
                    self.module(id).is_ok_and(|s| {
                        s.value.modern()
                            && !s.value.participants.is_empty()
                            && s.value.participants.len() == active.len()
                            && active
                                .iter()
                                .all(|member| s.value.participants.contains(member))
                            && s.value
                                .workflow
                                .as_ref()
                                .is_some_and(|w| w.environment.is_some() && !w.scenarios.is_empty())
                            && self.phase(&s.value) == "accepted"
                    })
                });
                if !complete {
                    missing.push("Independently accept a current integration Atomic covering every noncanceled frozen Module, with environment/scenarios.".into());
                }
            }
        }

        if m.modern() {
            if let Err(e) = self.links(m) {
                missing.push(format!("Requirements unknown/invalid: {}", e.message));
            }
            if m.id.starts_with("E-")
                && m.workflow
                    .as_ref()
                    .is_some_and(|w| w.frozen_modules.is_none())
            {
                missing.push("Report Epic begin to freeze its Module roster.".into());
            }
        }
        if m.id.starts_with("A-") && !m.participants.is_empty() {
            if m.modern() {
                for id in &m.participants {
                    match self.module(id) {
                        Ok(participant)
                            if self.phase(&participant.value) == "accepted"
                                && participant.value.delivered() => {}
                        Ok(_) => missing.push(format!(
                            "{id}: integration requires current accepted/delivered Module."
                        )),
                        Err(e) => missing.push(format!(
                            "{id}: unknown integration participant ({})",
                            e.message
                        )),
                    }
                }
            }
            match self.participant_basis(&m.participants) {
                Ok(basis) if basis == m.participant_basis => (),
                Ok(_) => missing.push("Integration evidence is stale; report a fresh result against current participants.".into()),
                Err(e) => missing.push(format!("Integration participant unreadable: {}", e.message)),
            }
        }
        missing
    }

    /// Derive the current phase without modifying history or treating missing children as completed.
    pub fn phase(&self, m: &Module) -> &'static str {
        if m.state == model::ModuleState::Canceled {
            return "canceled";
        }
        if m.id.starts_with("A-") {
            if !m.modern() {
                return if m.completed {
                    if self.acceptance(m).is_empty() {
                        "done"
                    } else {
                        "stale completion"
                    }
                } else {
                    m.phase()
                };
            }
            if let Some(r) = m.reviews.last() {
                if r.epoch == m.review_epoch && m.basis().is_ok_and(|b| b == r.basis) {
                    if r.verdict == model::Verdict::ChangesRequested {
                        return "changes requested";
                    }
                    return if self.acceptance(m).is_empty() {
                        "accepted"
                    } else {
                        "stale approval"
                    };
                }
                if r.verdict == model::Verdict::Accepted {
                    return "stale approval";
                }
            }
            if m.completed {
                return if self.acceptance(m).is_empty() {
                    "ready"
                } else {
                    "working"
                };
            }
            return m.phase();
        }
        if m.id.starts_with("E-") {
            if let Some(r) = m.reviews.last() {
                let applicable =
                    r.epoch == m.review_epoch && self.work_basis(m).is_ok_and(|b| b == r.basis);
                if applicable {
                    return if r.verdict == model::Verdict::Accepted && m.acceptance().is_empty() {
                        "accepted"
                    } else if r.verdict == model::Verdict::ChangesRequested {
                        "changes requested"
                    } else {
                        "stale approval"
                    };
                }
                if r.verdict == model::Verdict::Accepted {
                    return "stale approval";
                }
            }
            return if self.acceptance(m).is_empty() {
                "ready"
            } else if m.result.is_some() {
                "working"
            } else {
                "planned"
            };
        }
        m.phase()
    }

    /// Validate sole Epic membership against all Epic authorities and referenced canonical records.
    /// No cross-file publication occurs: candidate membership is validated before its one-file write.
    pub fn membership(&self, candidate: &Module) -> Result<()> {
        if !candidate.id.starts_with("E-") {
            return Ok(());
        }
        let inventory = self.kind_inventory("epics", "E-")?;
        if !inventory.complete {
            return Err(invalid("Epic ownership inventory is incomplete."));
        }
        if let Some(roster) = candidate
            .workflow
            .as_ref()
            .and_then(|w| w.frozen_modules.as_ref())
            && roster != &candidate.modules
        {
            return Err(invalid("Epic Module roster is frozen permanently."));
        }
        for id in candidate.modules.iter().chain(&candidate.atomic_members) {
            self.module(id)?;
        }
        for id in inventory.ids {
            if id == candidate.id {
                continue;
            }
            let other = self.module(&id)?.value;
            if candidate
                .modules
                .iter()
                .any(|id| other.modules.contains(id))
                || candidate
                    .atomic_members
                    .iter()
                    .any(|id| other.atomic_members.contains(id))
            {
                return Err(invalid("A member already belongs to another Epic."));
            }
        }
        Ok(())
    }

    /// Validate declared peers and waits against healthy records; only blocking edges participate in cycle checks.
    pub fn links(&self, candidate: &Module) -> Result<()> {
        candidate.validate().map_err(invalid)?;
        if let Some(w) = &candidate.workflow {
            for peer in w
                .contracts
                .iter()
                .flat_map(|c| c.provides.iter().chain(&c.consumes))
                .map(|c| &c.peer)
            {
                self.module(peer)?;
            }
            for dependency in &w.dependencies {
                self.module(&dependency.reference)?;
            }
        }
        if (candidate.id.starts_with("E-")
            && candidate.modules.is_empty()
            && candidate.atomic_members.is_empty())
            || !candidate.id.starts_with("E-")
                && candidate
                    .workflow
                    .as_ref()
                    .is_none_or(|w| w.dependencies.is_empty())
        {
            return Ok(());
        }
        let scan = self.scan(None)?;
        if !scan.complete {
            return Err(invalid(
                "Blocking dependency graph has unreadable or incomplete work.",
            ));
        }
        let mut graph = BTreeMap::<String, Vec<String>>::new();
        for m in scan
            .modules
            .iter()
            .map(|s| &s.value)
            .filter(|m| m.id != candidate.id)
            .chain(std::iter::once(candidate))
        {
            let edges = m
                .modules
                .iter()
                .chain(&m.atomic_members)
                .chain(&m.participants)
                .cloned()
                .chain(
                    m.workflow
                        .iter()
                        .flat_map(|w| w.dependencies.iter().map(|d| d.reference.clone())),
                )
                .collect();
            graph.insert(m.id.clone(), edges);
        }
        let mut visiting = std::collections::BTreeSet::new();
        let mut done = std::collections::BTreeSet::new();
        for id in graph.keys() {
            blocking_cycle(id, &graph, &mut visiting, &mut done)?;
        }
        Ok(())
    }

    /// Return actual reported-start requirements; contract data flow never becomes a hidden wait.
    pub fn readiness(&self, m: &Module) -> Vec<String> {
        let mut missing = Vec::new();
        if m.state == model::ModuleState::Canceled {
            missing.push("Reopen canceled work.".into());
        }
        if let Err(e) = self.links(m) {
            missing.push(e.message);
            return missing;
        }
        match self.parent(&m.id) {
            Ok(Some(parent))
                if parent.state == model::ModuleState::Canceled
                    || parent.workflow.as_ref().is_some_and(|w| !w.active) =>
            {
                missing.push(format!("Parent {} must be begun/open.", parent.id))
            }
            Err(e) => missing.push(format!("Parent ownership unknown: {}", e.message)),
            _ => (),
        }
        if let Some(w) = &m.workflow {
            if m.id.starts_with("M-") {
                if m.lead.is_none() {
                    missing.push("Declare a known Module lead.".into());
                }
                if m.criteria.is_empty() {
                    missing.push("Declare Module criteria.".into());
                }
                if w.execution.is_none() {
                    missing
                        .push("Declare repository/worktree/branch/target_branch execution.".into());
                }
                match &w.contracts {
                    None => {
                        missing.push("Declare provides/consumes or explicit not_required.".into())
                    }
                    Some(c) => {
                        for contract in c.provides.iter().chain(&c.consumes) {
                            if !contract.ready {
                                missing
                                    .push(format!("Contract with {} is not ready.", contract.peer));
                            }
                        }
                    }
                }
                for dependency in &w.dependencies {
                    match self.module(&dependency.reference) {
                        Ok(target) => {
                            let met = match dependency.condition {
                                model::DependencyCondition::Accepted => {
                                    self.phase(&target.value) == "accepted"
                                }
                                model::DependencyCondition::Delivered => {
                                    self.phase(&target.value) == "accepted"
                                        && target.value.delivered()
                                }
                            };
                            if !met {
                                missing.push(format!(
                                    "Wait for {} {:?}: {}",
                                    dependency.reference, dependency.condition, dependency.reason
                                ));
                            }
                        }
                        Err(e) => missing.push(format!(
                            "{}: unknown prerequisite ({})",
                            dependency.reference, e.message
                        )),
                    }
                }
            }
            if m.id.starts_with("A-") {
                if m.lead.is_none() {
                    missing.push("Declare an Atomic executor.".into());
                }
                if !m.participants.is_empty() {
                    if w.environment.is_none() || w.scenarios.is_empty() {
                        missing.push("Declare real integration environment and scenarios.".into());
                    }
                    for id in &m.participants {
                        match self.module(id) {
                            Ok(target)
                                if self.phase(&target.value) == "accepted"
                                    && target.value.delivered() => {}
                            Ok(_) => missing.push(format!(
                                "{id}: requires accepted and delivered before integration begin."
                            )),
                            Err(e) => missing
                                .push(format!("{id}: unreadable participant ({})", e.message)),
                        }
                    }
                }
            }
        }
        missing
    }

    /// Find an authoritative Epic parent; malformed ownership refuses rather than guessing standalone.
    pub fn parent(&self, id: &str) -> Result<Option<Module>> {
        let inventory = self.kind_inventory("epics", "E-")?;
        if !inventory.complete {
            return Err(invalid("Epic ownership inventory is incomplete."));
        }
        let mut owner = None;
        for epic in inventory.ids {
            let m = self.module(&epic)?.value;
            if m.modules
                .iter()
                .chain(&m.atomic_members)
                .any(|member| member == id)
            {
                if owner.is_some() {
                    return Err(invalid("Duplicate Epic ownership."));
                }
                owner = Some(m);
            }
        }
        Ok(owner)
    }

    /// Refuse child writes while their authoritative Epic parent is canceled; no cascade occurs.
    pub fn open_parent(&self, id: &str) -> Result<()> {
        if let Some(parent) = self.parent(id)?
            && parent.state == model::ModuleState::Canceled
        {
            return Err(Error::new(
                "canceled_parent",
                format!("Reopen parent {} before changing its child.", parent.id),
            ));
        }
        Ok(())
    }

    /// Parse observed bytes and attach their exact version without normalizing on read.
    fn snapshot<T: DeserializeOwned>(
        &self,
        relative: &str,
        validate: impl FnOnce(&T) -> std::result::Result<(), String>,
    ) -> Result<Option<Snapshot<T>>> {
        let Some(bytes) = self.bytes(relative)? else {
            return Ok(None);
        };
        let value = decode(&bytes)?;
        validate(&value).map_err(invalid)?;
        Ok(Some(Snapshot {
            value,
            version: self.version(relative, Some(&bytes)),
            bytes,
        }))
    }

    /// Enumerate a bounded immediate module inventory; no recursive repository scan.
    pub fn inventory(&self) -> Result<Inventory> {
        let mut combined = Inventory {
            ids: Vec::new(),
            warnings: Vec::new(),
            complete: true,
        };
        for (directory, prefix) in [("epics", "E-"), ("modules", "M-"), ("atomics", "A-")] {
            let part = self.kind_inventory(directory, prefix)?;
            combined.ids.extend(part.ids);
            combined.warnings.extend(part.warnings);
            combined.complete &= part.complete;
        }
        if combined.ids.len() > MODULE_CAP {
            combined.complete = false;
            combined
                .warnings
                .push("Combined work inventory limit reached; counts are lower bounds.".into());
            combined.ids.truncate(MODULE_CAP);
        }
        combined.warnings.sort();
        Ok(combined)
    }

    /// Enumerate one bounded kind directory; absence is empty and unknown entries remain warnings.
    fn kind_inventory(&self, directory: &str, prefix: &str) -> Result<Inventory> {
        let path = self.path(directory)?;
        if !path.exists() {
            return Ok(Inventory {
                ids: Vec::new(),
                warnings: Vec::new(),
                complete: true,
            });
        }
        if !path.is_dir() {
            return Err(Error::new("file_type", "modules must be a directory."));
        }
        let mut inventory = Inventory {
            ids: Vec::new(),
            warnings: Vec::new(),
            complete: true,
        };
        let entries = fs::read_dir(path).map_err(|_| Error::new("io", "Cannot list modules."))?;
        for (index, entry) in entries.enumerate() {
            if index >= MODULE_CAP {
                inventory.complete = false;
                inventory
                    .warnings
                    .push("Module inventory limit reached; counts are lower bounds.".into());
                break;
            }
            let entry = entry.map_err(|_| Error::new("io", "Cannot inspect module entry."))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if own_temp(&name) {
                inventory.warnings.push(format!(
                    "Orphan publication temp ignored: {}",
                    safe(&name, 160)
                ));
                continue;
            }
            let id = name
                .strip_suffix(".yaml")
                .filter(|s| model::number(s, prefix).is_ok());
            let regular = entry
                .file_type()
                .is_ok_and(|t| t.is_file() && !t.is_symlink());
            if let Some(id) = id {
                inventory.ids.push(id.into());
                if !regular {
                    inventory.complete = false;
                    inventory
                        .warnings
                        .push(format!("{id}: nonregular module entry."));
                }
            } else {
                inventory.complete = false;
                inventory
                    .warnings
                    .push(format!("Unrecognized module entry: {}", safe(&name, 160)));
            }
        }
        inventory
            .ids
            .sort_by_key(|id| model::number(id, prefix).unwrap_or(0));
        inventory.warnings.sort();
        Ok(inventory)
    }

    /// Creation precondition independent of report contents; own directories/lock do not affect it.
    pub fn allocation_version(&self) -> Result<String> {
        let inventory = self.inventory()?;
        let project = self.bytes("project.yaml")?;
        let state = self.bytes(".agent-tasks/state.yaml")?;
        let mut digest = Sha256::new();
        for part in [
            self.version("project.yaml", project.as_deref()),
            self.version(".agent-tasks/state.yaml", state.as_deref()),
        ]
        .iter()
        .chain(inventory.ids.iter())
        .chain(inventory.warnings.iter())
        {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        digest.update([u8::from(inventory.complete)]);
        Ok(format!("{:x}", digest.finalize()))
    }

    /// Return healthy records, named omissions and a snapshot over exactly scanned data.
    /// ponytail: bounded O(n) file scan; add an index only after measured scan cost matters.
    pub fn scan(&self, module: Option<&str>) -> Result<Scan> {
        let inventory = if let Some(id) = module {
            model::work_number(id).map_err(invalid)?;
            Inventory {
                ids: vec![id.into()],
                warnings: Vec::new(),
                complete: true,
            }
        } else {
            self.inventory()?
        };
        let mut scan = Scan {
            modules: Vec::new(),
            unreadable: Vec::new(),
            warnings: inventory.warnings,
            complete: inventory.complete,
            version: String::new(),
        };
        let mut total = 0usize;
        let mut digest = Sha256::new();
        digest.update(self.root.as_os_str().as_encoded_bytes());
        for id in inventory.ids {
            let relative = model::work_path(&id).map_err(invalid)?;
            let available = SCAN_CAP.saturating_sub(total);
            let path = match self.path(&relative) {
                Ok(path) => path,
                Err(e) => {
                    scan.complete = false;
                    scan.unreadable.push(format!("{id}: {}", e.message));
                    digest.update(id.as_bytes());
                    digest.update(e.code.as_bytes());
                    continue;
                }
            };
            if fs::symlink_metadata(&path).is_ok_and(|m| {
                m.is_file() && m.len() <= RECORD_CAP as u64 && m.len() > available as u64
            }) {
                scan.complete = false;
                scan.unreadable
                    .push(format!("{id}: aggregate byte budget reached."));
                digest.update(id.as_bytes());
                digest.update(b"unscanned");
                continue;
            }
            let raw = read_path(&path, RECORD_CAP.min(available));
            match raw {
                Ok(Some(bytes)) if total.saturating_add(bytes.len()) <= SCAN_CAP => {
                    total += bytes.len();
                    digest.update(id.as_bytes());
                    digest.update(self.version(&relative, Some(&bytes)).as_bytes());
                    match decode::<Module>(&bytes).and_then(|value| {
                        value.validate().map_err(invalid)?;
                        if value.id != id {
                            return Err(invalid("ID differs from filename."));
                        }
                        Ok(value)
                    }) {
                        Ok(value) => scan.modules.push(Snapshot {
                            value,
                            version: self.version(&relative, Some(&bytes)),
                            bytes,
                        }),
                        Err(e) => {
                            scan.complete = false;
                            scan.unreadable.push(format!("{id}: {}", e.message));
                        }
                    }
                }
                Ok(Some(_)) => {
                    scan.complete = false;
                    scan.unreadable
                        .push(format!("{id}: aggregate byte budget reached."));
                    digest.update(id.as_bytes());
                    digest.update(b"unscanned");
                }
                Ok(None) => {
                    scan.complete = false;
                    scan.unreadable.push(format!("{id}: missing record."));
                    digest.update(id.as_bytes());
                    digest.update(b"missing");
                }
                Err(e) => {
                    scan.complete = false;
                    scan.unreadable.push(format!("{id}: {}", e.message));
                    digest.update(id.as_bytes());
                    digest.update(e.code.as_bytes());
                }
            }
        }
        if module.is_none() {
            let mut owners = BTreeMap::new();
            for m in &scan.modules {
                for id in m.value.modules.iter().chain(&m.value.atomic_members) {
                    if !scan.modules.iter().any(|s| s.value.id == *id) {
                        scan.complete = false;
                        scan.warnings.push(format!(
                            "{}: missing or unreadable member {id}.",
                            m.value.id
                        ));
                    }
                    if let Some(previous) = owners.insert(id, &m.value.id) {
                        scan.complete = false;
                        scan.warnings.push(format!(
                            "{id}: duplicate ownership by {previous} and {}.",
                            m.value.id
                        ));
                    }
                }
                for id in &m.value.participants {
                    if !scan.modules.iter().any(|s| s.value.id == *id) {
                        scan.complete = false;
                        scan.warnings.push(format!(
                            "{}: missing or unreadable participant {id}.",
                            m.value.id
                        ));
                    }
                }
            }
        }
        for warning in scan.warnings.iter().chain(&scan.unreadable) {
            digest.update(warning.as_bytes());
        }
        scan.version = format!("{:x}", digest.finalize());
        Ok(scan)
    }

    /// Explicit init prepares only the final root and owned directories; all effects are disclosed.
    pub fn prepare(&self, effects: &mut Vec<String>) -> Result<()> {
        if !self.root.exists() {
            fs::create_dir(&self.root)
                .map_err(|_| Error::new("io", "Cannot create the configured final root."))?;
            effects.push("Created configured root directory.".into());
        }
        for relative in ["modules", "epics", "atomics", ".agent-tasks"] {
            let path = self.path(relative)?;
            if !path.exists() {
                fs::create_dir(&path)
                    .map_err(|_| Error::new("io", "Cannot prepare owned directory."))?;
                effects.push(format!("Created {relative}/."));
            }
            if !path.is_dir() {
                return Err(Error::new(
                    "file_type",
                    format!("{relative} is not a directory."),
                ));
            }
        }
        Ok(())
    }

    /// Acquire a fresh advisory handle. Reads never create a lock, writes return busy immediately.
    pub fn lock(&self, write: bool, effects: &mut Vec<String>) -> Result<Option<LockGuard>> {
        self.file_lock(".agent-tasks/write.lock", write, effects)
    }

    /// Lock one validated relative coordination file; readers never create it.
    /// Writers create mode-0600 only in an existing parent; contention refuses immediately.
    pub fn file_lock(
        &self,
        relative: &str,
        write: bool,
        effects: &mut Vec<String>,
    ) -> Result<Option<LockGuard>> {
        let path = self.path(relative)?;
        let file = if write {
            if !path.parent().is_some_and(Path::is_dir) {
                return Err(Error::new(
                    "not_initialized",
                    "Initialize the configured project explicitly first.",
                ));
            }
            let exists = path.exists();
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let f = options
                .open(path)
                .map_err(|_| Error::new("io", "Cannot open writer lock."))?;
            if !exists {
                effects.push(format!("Created {relative}."));
            }
            f
        } else {
            if !path.exists() {
                return Ok(None);
            }
            File::open(path).map_err(|_| Error::new("io", "Cannot open existing read lock."))?
        };
        let result = if write {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        result.map_err(|_| {
            Error::new(
                "busy",
                "Another cooperating call owns the store lock; retry later.",
            )
        })?;
        Ok(Some(LockGuard { file }))
    }

    /// Preserve exact noncanonical bytes before a guarded canonical replacement.
    pub fn save<T: Serialize + DeserializeOwned>(
        &self,
        relative: &str,
        value: &T,
        observed: Option<&[u8]>,
        terminal: bool,
        effects: &mut Vec<String>,
    ) -> Result<String> {
        let bytes = encode(value)?;
        let cap = if terminal {
            RECORD_CAP
        } else {
            RECORD_CAP - CLOSING_RESERVE
        };
        if bytes.len() > cap {
            return Err(Error::new(
                "capacity",
                "Record capacity reached; preserve review history and reduce current detail or start coherent continuation work.",
            ));
        }
        if let Some(old) = observed {
            let old_value: T = decode(old)?;
            if encode(&old_value)? != old {
                let dir = self.path(".agent-tasks/backups")?;
                if !dir.exists() {
                    fs::create_dir(&dir).map_err(|_| {
                        Error::new("backup", "Cannot create normalization backup directory.")
                    })?;
                    effects.push("Created .agent-tasks/backups/.".into());
                }
                if !dir.is_dir() {
                    return Err(Error::new("backup", "Backup location is not a directory."));
                }
                let name = Path::new(relative)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| Error::new("path", "Invalid backup record name."))?;
                let backup = format!(
                    ".agent-tasks/backups/{name}-{}.yaml",
                    self.version(relative, Some(old))
                );
                match self.bytes(&backup)? {
                    Some(existing) if existing == old => (),
                    Some(_) => {
                        return Err(Error::new(
                            "backup",
                            "Version-bound backup conflicts; original record untouched.",
                        ));
                    }
                    None => self.publish(&backup, old, None, effects)?,
                }
                effects.push(format!(
                    "Normalized source; exact original retained at {backup}."
                ));
            }
        }
        self.publish(relative, &bytes, observed, effects)?;
        Ok(self.version(relative, Some(&bytes)))
    }

    /// Publish fully synced bytes. New records use no-clobber hard links; replacements
    /// recheck exact observed bytes before rename. Post-publication sync failure is
    /// reported with a visible-effect ledger, never retried or claimed as no effect.
    pub(crate) fn publish(
        &self,
        relative: &str,
        bytes: &[u8],
        observed: Option<&[u8]>,
        effects: &mut Vec<String>,
    ) -> Result<()> {
        let path = self.path(relative)?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::new("path", "Record has no parent."))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::new("path", "Invalid record name."))?;
        let temp = parent.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| Error::new("io", "Cannot exclusively create publication temp."))?;
        let publication = (|| {
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| {
                    Error::new("io", "Cannot write/sync candidate; work not published.")
                })?;
            if self.bytes(relative)?.as_deref() != observed {
                return Err(Error::new(
                    "stale",
                    "Observed bytes changed; work not published. Read context before retrying.",
                ));
            }
            self.path(relative)?;
            if observed.is_some() {
                fs::rename(&temp, &path)
                    .map_err(|_| Error::new("io", "Atomic replacement failed."))?;
            } else {
                fs::hard_link(&temp, &path).map_err(|_| {
                    Error::new(
                        "publication",
                        "No-clobber publication failed; no overwrite fallback.",
                    )
                })?;
            }
            effects.push(format!("Published {relative}."));
            sync_parent(parent).map_err(|_| Error::new("durability_unknown",format!("{relative} is visibly published; directory sync failed. Inspect current context; do not blindly replay.")))?;
            Ok(())
        })();
        drop(file);
        let _ = fs::remove_file(&temp);
        publication
    }
}

#[cfg(test)]
std::thread_local! {
    /// One-shot calling-thread fault, never a shipping option or a cross-test global flag.
    static FAIL_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Inject one post-publication sync failure in the current test thread only.
#[cfg(test)]
pub fn fail_next_directory_sync() {
    FAIL_DIRECTORY_SYNC.with(|flag| flag.set(true));
}

/// Sync directory publication; a post-publication failure leaves a visible-effect receipt.
fn sync_parent(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_DIRECTORY_SYNC.with(|flag| flag.replace(false)) {
        return Err(std::io::Error::other(
            "Injected post-publication directory sync failure",
        ));
    }
    File::open(path)?.sync_all()
}

/// Recognize only our target-specific publication temps; foreign dotfiles remain foreign.
fn own_temp(name: &str) -> bool {
    let Some((base, suffix)) = name
        .strip_prefix('.')
        .and_then(|s| s.split_once(".yaml.tmp-"))
    else {
        return false;
    };
    model::number(base, "M-").is_ok()
        && suffix
            .split_once('-')
            .is_some_and(|(pid, n)| pid.parse::<u32>().is_ok() && n.parse::<u64>().is_ok())
}

/// Quote bounded user text for one-line presentation, escaping newlines and structural punctuation.
pub fn safe(value: &str, cap: usize) -> String {
    let mut end = value.len().min(cap);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = String::new();
    result.push('"');
    for c in value[..end].chars() {
        for c in c.escape_default() {
            result.push(c);
        }
    }
    if end < value.len() {
        result.push('…');
    }
    result.push('"');
    result
}

/// Detect only blocking dependency cycles, including owning container closure edges; contract links are excluded.
fn blocking_cycle(
    id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    visiting: &mut std::collections::BTreeSet<String>,
    done: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    if done.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.into()) {
        return Err(invalid(format!("Blocking dependency cycle at {id}.")));
    }
    for next in graph.get(id).into_iter().flatten() {
        if !graph.contains_key(next) {
            return Err(invalid(format!("Dangling blocking reference {next}.")));
        }
        blocking_cycle(next, graph, visiting, done)?;
    }
    visiting.remove(id);
    done.insert(id.into());
    Ok(())
}
