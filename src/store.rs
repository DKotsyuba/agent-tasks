//! Portable bounded file access, optimistic versions and cooperating-writer publication.
mod publish;
#[allow(
    unused_imports,
    reason = "Early primitives handoff: consumers in other Modules land later and this allowance is removed with them"
)]
pub use publish::{
    ABSOLUTE_CAP, Attest, DirEntry, DirListing, Durability, EffectKind, EntryKind, LIST_CAP,
    Observed, OperationId, Publication, Publish, Remove, Tracking, UntrackedReason, not_applicable,
};

use crate::model::{self, Module, Project};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
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
    /// Request state of the Store that acquired the root write lock, so primitives can prove the
    /// cooperative lock is held; `None` for coordination locks other than the root write lock.
    state: Option<std::sync::Arc<RequestState>>,
}

impl LockGuard {
    /// Whether this guard is the root write lock acquired through `store` in this request.
    #[allow(
        dead_code,
        reason = "Early primitives handoff: settlement uses it when it lands and this allowance is removed with it"
    )]
    pub fn is_for(&self, store: &Store) -> bool {
        self.state
            .as_ref()
            .is_some_and(|s| std::sync::Arc::ptr_eq(s, &store.state))
    }
}

/// Request-local state shared by clones of one [`Store`]: the typed event ledger, the root write lock
/// token and the journal intent of this request.
#[derive(Default)]
pub(crate) struct RequestState {
    /// Typed publication events in order, kept even when a later step failed.
    pub(crate) events: std::sync::Mutex<Vec<Publication>>,
    /// True while the root write lock acquired through this Store is held.
    pub(crate) locked: std::sync::atomic::AtomicBool,
    /// Journal intent shared by every tracked publication of this request.
    pub(crate) intent: std::sync::Mutex<Option<String>>,
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
        if let Some(state) = &self.state {
            state.locked.store(false, Ordering::SeqCst);
        }
        let _ = self.file.unlock();
    }
}

/// Root identity held through a request; tools never accept arbitrary relative paths.
#[derive(Clone)]
pub struct Store {
    /// Canonical existing root or canonical parent plus one absent final directory name.
    pub root: PathBuf,
    /// Request-local ledger and lock token shared by clones of this Store.
    pub(crate) state: std::sync::Arc<RequestState>,
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

/// Refuse anchors, aliases and tags before the parser could expand them, and count explicit
/// document starts.
///
/// This is a small node-context scanner, not a YAML lexer. A property indicator (`&`, `*`, `!`)
/// is refused only where a node may begin: at a line start in block context, after a block
/// sequence or explicit-key indicator, after a mapping separator, after `[`, `{`, `,`, `:` or an
/// explicit-key `?` in flow context (with or without a following space, as the parser allows),
/// and after a document marker. The same characters inside a
/// plain scalar (`valid_reference(store,&str)`), a quoted scalar, a comment or a literal/folded
/// block scalar are ordinary text. A block scalar ends at the first non-blank line that is not
/// indented past the column of the node that owns it, so mapping siblings after it are scanned
/// again. Only SPACE and TAB separate tokens or begin comments; Unicode spaces such as U+00A0 are
/// scalar text.
///
/// Deliberately strict subset, so no later line can be masked by a fake continuation:
/// - every line must close its quotes and flow brackets (`single-line` rule); native multi-line
///   quoted scalars and flow collections are refused, and the canonical writer, which emits
///   block and literal collections with single-line quoted scalars, never produces them;
/// - a continuation line of a multi-line plain scalar is scanned as a fresh node, so one that
///   begins with an indicator is refused rather than trusted;
/// - the only line breaks are LF and CRLF, and a byte order mark is accepted once at the start of
///   the document; lone CR, NEL, LS, PS and a mid-document BOM are refused because the parser and
///   `str::lines` would disagree about where lines start.
///
/// `encode` verifies its own output through this same gate.
///
/// Document count: only explicit `---` starts are counted here. An implicit first document
/// followed by a later `---` passes this scanner and is refused by the parser stage of
/// `read_tree`, before any typed value is built; a rejected later document cannot expand aliases
/// because every later line is scanned like any other.
///
/// Errors: non-UTF-8 input, forbidden line breaks, more than one explicit document start, a
/// property indicator at a node start, a line that leaves a quote or flow collection open, or a malformed block
/// scalar header.
fn yaml_subset(bytes: &[u8]) -> Result<()> {
    let source = std::str::from_utf8(bytes).map_err(|_| invalid("YAML must be UTF-8."))?;
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    if source.contains(['\u{feff}', '\u{85}', '\u{2028}', '\u{2029}'])
        || source.split("\r\n").any(|part| part.contains('\r'))
    {
        return Err(invalid(
            "YAML line breaks must be LF or CRLF and a byte order mark may only start the document.",
        ));
    }
    let refused =
        || invalid("YAML tags, anchors and aliases are unsupported; quote literal prose.");
    let blank = |c: char| c == ' ' || c == '\t';
    let separated = |next: Option<&(usize, char)>| next.is_none_or(|(_, c)| blank(*c));
    let mut block: Option<usize> = None;
    let mut documents = 0;
    for line in source.lines() {
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        if let Some(base) = block {
            if line.chars().all(blank) || indent > base {
                continue;
            }
            block = None;
        }
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut at = 0;
        let mut quote = None;
        let mut flow = 0usize;
        let mut start = true;
        let mut entry_start = true;
        let mut entry = indent;
        let mut parent = indent;
        if line.starts_with("...") && line[3..].chars().all(blank) {
            continue;
        }
        if line.starts_with("---") && line[3..].chars().next().is_none_or(blank) {
            documents += 1;
            if documents > 1 {
                return Err(invalid("One YAML document is allowed."));
            }
            at = 3;
            entry_start = false;
        }
        while at < chars.len() {
            let (col, c) = chars[at];
            let next = chars.get(at + 1);
            if let Some(q) = quote {
                if q == '"' && c == '\\' {
                    at += 2;
                    continue;
                }
                if c == q {
                    if q == '\'' && next.is_some_and(|(_, n)| *n == '\'') {
                        at += 2;
                        continue;
                    }
                    quote = None;
                    start = false;
                }
                at += 1;
                continue;
            }
            if blank(c) {
                at += 1;
                continue;
            }
            if c == '#' && (at == 0 || blank(chars[at - 1].1)) {
                break;
            }
            let opened = start;
            if opened && matches!(c, '&' | '*' | '!') {
                return Err(refused());
            }
            if flow > 0 {
                match c {
                    ',' | ':' => start = true,
                    '?' if opened || separated(next) => start = true,
                    '[' | '{' => {
                        flow += 1;
                        start = true;
                    }
                    ']' | '}' => {
                        flow -= 1;
                        start = false;
                    }
                    '\'' | '"' if opened => {
                        quote = Some(c);
                        start = false;
                    }
                    _ => start = false,
                }
            } else if opened {
                let owner = entry_start.then_some(col);
                match c {
                    '-' | '?' if separated(next) => {
                        parent = col;
                        entry_start = true;
                    }
                    ':' if separated(next) => parent = entry,
                    '|' | '>' => {
                        let header = line[col + 1..]
                            .trim_start_matches(|h: char| {
                                h.is_ascii_digit() || h == '+' || h == '-'
                            })
                            .trim_start_matches(blank);
                        if !header.is_empty() && !header.starts_with('#') {
                            return Err(invalid("Invalid block scalar header."));
                        }
                        block = Some(parent);
                        break;
                    }
                    _ => {
                        match c {
                            '\'' | '"' => quote = Some(c),
                            '[' | '{' => flow += 1,
                            _ => (),
                        }
                        start = matches!(c, '[' | '{');
                        if let Some(owner) = owner {
                            entry = owner;
                            entry_start = false;
                        }
                    }
                }
            } else if c == ':' && separated(next) {
                start = true;
                parent = entry;
            }
            at += 1;
        }
        if quote.is_some() || flow > 0 {
            return Err(invalid(
                "YAML quoted scalars and flow collections must stay on a single-line; use a block scalar.",
            ));
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

/// Run the exact read gate up to a generic tree: subset preflight, duplicate-key-free parse and
/// the closed tree rules. `decode` continues from this tree and `encode` verifies its own output
/// with it, so the writer and reader can never disagree about what is acceptable.
fn read_tree(bytes: &[u8]) -> Result<serde_yaml_ng::Value> {
    yaml_subset(bytes)?;
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(bytes).map_err(|_| invalid("Invalid YAML or duplicate keys."))?;
    yaml_tree(&value, 0)?;
    Ok(value)
}

/// Decode the closed YAML subset, rejecting duplicates before typed deserialization.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_yaml_ng::from_value(read_tree(bytes)?)
        .map_err(|_| invalid("Invalid fields, types or missing required schema data."))
}

/// Encode canonical YAML and prove the bytes pass the exact read gate before returning them.
///
/// Errors: `encoding` when serialization fails; `encoding_unreadable` when the canonical bytes
/// would be refused by `decode`'s preflight or parser. Nothing is published by this function, and
/// callers must not publish bytes it refused. Callers still enforce semantic validation.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_yaml_ng::to_string(value)
        .map(String::into_bytes)
        .map_err(|_| Error::new("encoding", "Cannot encode owned record."))?;
    read_tree(&bytes).map_err(|_| {
        Error::new(
            "encoding_unreadable",
            "Canonical record bytes would not pass the read gate; nothing was published.",
        )
    })?;
    Ok(bytes)
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
            state: Default::default(),
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
        if m.core().is_some() && m.id.starts_with("M-") {
            return semantic_digest(
                &serde_json::json!({"own":own,"contracts":self.contract_basis(m)?}),
            );
        }
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
                        let expected = if child.value.core().is_some() && id.starts_with("M-") {
                            "ready for integration"
                        } else if id.starts_with("M-") || child.value.modern() || m.modern() {
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
        if m.id.starts_with("E-") && m.modern() && m.core().is_none() {
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

        if let Some(core) = m.core() {
            if m.id.starts_with("M-") {
                missing.extend(self.core_review_gaps(m));
            }
            if m.id.starts_with("E-") {
                if m.workflow
                    .as_ref()
                    .is_none_or(|w| w.frozen_modules.is_none())
                {
                    missing.push("Freeze negotiated roster after lead agreement.".into());
                }
                for (index, text) in m.criteria.iter().enumerate() {
                    let scope = core
                        .criterion_scopes
                        .iter()
                        .find(|s| s.index == index && s.text == *text);
                    match scope {
                        None => missing.push(format!(
                            "Criterion {index}: exact affected Module scope missing."
                        )),
                        Some(scope) => {
                            if !core.criterion_verifications.iter().rev().any(|v| {
                                v.scope == *scope
                                    && self
                                        .criterion_basis(scope, v.integration_ref.as_deref())
                                        .is_ok_and(|b| b == v.basis)
                                    && v.checks
                                        .iter()
                                        .all(|c| c.status == model::CheckStatus::Passed)
                                    && !v.checks.is_empty()
                            }) {
                                missing.push(format!("Criterion {index}: CURRENT actual affected-set/E2E verification missing."));
                            }
                        }
                    }
                }
                let mut boundaries = std::collections::BTreeSet::new();
                for id in &m.modules {
                    if let Ok(module) = self.module(id) {
                        for cid in self.contract_ids(&module.value) {
                            if let Ok(f) = self.contract_facts(&cid)
                                && f.parties.iter().filter(|p| m.modules.contains(p)).count() >= 2
                            {
                                boundaries.insert(cid);
                            }
                        }
                    }
                }
                for cid in boundaries {
                    let covered = self.contract_facts(&cid).is_ok_and(|f| {
                        f.parties
                            .iter()
                            .filter(|p| **p != f.provider && m.modules.contains(p))
                            .all(|consumer| {
                                m.atomic_members.iter().any(|id| {
                                    self.module(id).is_ok_and(|a| {
                                        a.value.participants.contains(&f.provider)
                                            && a.value.participants.contains(consumer)
                                            && self.phase(&a.value) == "accepted"
                                    })
                                })
                            })
                    });
                    if !covered {
                        missing.push(format!(
                            "{cid}: current actual connected integration coverage missing."
                        ));
                    }
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
            if m.core().is_some()
                && m.result
                    .as_ref()
                    .and_then(|r| r.candidate.as_ref())
                    .is_none()
            {
                missing.push(
                    "Report the definite actual assembly candidate before integration acceptance."
                        .into(),
                );
            }
            if m.modern() && m.core().is_none() {
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
            if m.core().is_some() {
                match self.coverage_basis(&m.participants) {
                    Ok(b) if m.participant_basis.get("core") == Some(&b) => (),
                    Ok(_) => {
                        missing.push("Integration candidate/contract coverage is stale.".into())
                    }
                    Err(e) => missing.push(e.message),
                }
            } else {
                match self.participant_basis(&m.participants) {
                Ok(basis) if basis == m.participant_basis => (),
                Ok(_) => missing.push("Integration evidence is stale; report a fresh result against current participants.".into()),
                Err(e) => missing.push(format!("Integration participant unreadable: {}", e.message)),
            }
            }
        }
        missing
    }

    /// Derive the current phase without modifying history or treating missing children as completed.
    pub fn phase(&self, m: &Module) -> &'static str {
        if m.state == model::ModuleState::Canceled {
            return "canceled";
        }
        if m.id.starts_with("M-") && m.core().is_some() {
            if let Some(r) = m.reviews.last() {
                let current =
                    r.epoch == m.review_epoch && self.work_basis(m).is_ok_and(|b| b == r.basis);
                if current
                    && r.verdict == model::Verdict::Accepted
                    && self.core_review_gaps(m).is_empty()
                {
                    return "ready for integration";
                }
                if current && r.verdict == model::Verdict::ChangesRequested {
                    return "changes requested";
                }
                if r.verdict == model::Verdict::Accepted {
                    return "stale approval";
                }
            }
            return if m.workflow.as_ref().is_some_and(|w| w.active) {
                "working"
            } else {
                "planning"
            };
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
                if let Some(core) = m.core() {
                    if let Err(e) = core.actor(
                        model::AgentRole::Lead,
                        &core
                            .binding(model::AgentRole::Lead)
                            .map(|b| b.current.identity.agent_id.clone()),
                    ) {
                        missing.push(e);
                    }
                    if !core
                        .planning
                        .as_ref()
                        .is_some_and(|p| m.plan_basis().is_ok_and(|b| b == p.basis))
                    {
                        missing
                            .push("Current bound lead discovery/planning missing or stale.".into());
                    }
                    missing.extend(self.agreement_gaps(m));
                    if let Ok(Some(parent)) = self.parent(&m.id)
                        && parent
                            .workflow
                            .as_ref()
                            .is_none_or(|w| w.frozen_modules.is_none())
                    {
                        missing.push(
                            "Freeze Epic after relevant lead agreement before coding.".into(),
                        );
                    }
                    if let Some(execution) = &w.execution
                        && let Ok(scan) = self.scan(None)
                    {
                        for peer in scan.modules.iter().map(|s| &s.value).filter(|p| {
                            p.id != m.id
                                && p.id.starts_with("M-")
                                && p.core().is_some()
                                && p.workflow.as_ref().is_some_and(|w| w.active)
                        }) {
                            if peer
                                .workflow
                                .as_ref()
                                .and_then(|w| w.execution.as_ref())
                                .is_some_and(|e| e.worktree == execution.worktree)
                            {
                                missing.push(format!("{} shares the writable Module checkout; isolate before implementation.",peer.id));
                            }
                        }
                    }
                }
                if m.core().is_none() && m.lead.is_none() {
                    missing.push("Declare a known Module lead.".into());
                }
                if m.criteria.is_empty() {
                    missing.push("Declare Module criteria.".into());
                }
                if w.execution.is_none() {
                    missing
                        .push("Declare repository/worktree/branch/target_branch execution.".into());
                }
                if m.core().is_none() {
                    match &w.contracts {
                        None => missing
                            .push("Declare provides/consumes or explicit not_required.".into()),
                        Some(c) => {
                            for contract in c.provides.iter().chain(&c.consumes) {
                                if !contract.ready {
                                    missing.push(format!(
                                        "Contract with {} is not ready.",
                                        contract.peer
                                    ));
                                }
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
                                        || self.module_ready(&target.value)
                                }
                                model::DependencyCondition::Delivered => {
                                    (self.phase(&target.value) == "accepted"
                                        || self.module_ready(&target.value))
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
                if let Some(core) = m.core().filter(|_| !m.participants.is_empty()) {
                    if let Err(e) = core.actor(
                        model::AgentRole::Integrator,
                        &core
                            .binding(model::AgentRole::Integrator)
                            .map(|b| b.current.identity.agent_id.clone()),
                    ) {
                        missing.push(e);
                    }
                    if !m.participants.is_empty() {
                        if !self.connected(&m.participants).unwrap_or(false) {
                            missing.push("Integration needs >=2 connected ready Modules.".into());
                        }
                        if let Err(e) = self.coverage_basis(&m.participants) {
                            missing.push(e.message);
                        }
                        if w.execution.is_none() {
                            missing.push("Declare a separate actual integration checkout.".into());
                        }
                        if let Some(e) = &w.execution {
                            for id in &m.participants {
                                if self.module(id).is_ok_and(|p| {
                                    p.value
                                        .workflow
                                        .as_ref()
                                        .and_then(|w| w.execution.as_ref())
                                        .is_some_and(|m| m.worktree == e.worktree)
                                }) {
                                    missing.push(format!(
                                        "{id}: integration checkout must be distinct."
                                    ));
                                }
                            }
                        }
                        if let (Some(environment), true) = (&w.environment, !w.scenarios.is_empty())
                        {
                            if let Ok(Some(existing)) = self.covered_integration(
                                &m.participants,
                                environment,
                                &w.scenarios,
                                Some(&m.id),
                            ) {
                                missing.push(format!("Current identical coverage already exists at {existing}; do not duplicate a job."));
                            }
                            match self.pending_integration(&m.participants,environment,&w.scenarios,Some(&m.id)){
                                Ok(Some(existing))=>missing.push(format!("Equivalent active/pending integration claim {existing}; inspect its outcome before another dispatch.")),
                                Err(e)=>missing.push(e.message),Ok(None)=>(),
                            }
                        }
                    }
                }
                if (m.core().is_none() || m.participants.is_empty()) && m.lead.is_none() {
                    missing.push("Declare an Atomic executor.".into());
                }
                if !m.participants.is_empty() {
                    if w.environment.is_none() || w.scenarios.is_empty() {
                        missing.push("Declare real integration environment and scenarios.".into());
                    }
                    if m.core().is_none() {
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

    /// The bounded one-directory ID inventory, public so knowledge, document and compaction code reuse it.
    ///
    /// Lists `directory` without recursion. An entry is an ID when it is a regular file named
    /// `<prefix><canonical digits>.yaml`; an ID-named link or non-regular entry sets `complete=false`.
    /// Entries beyond [`MODULE_CAP`] set `complete=false`. Any other name, every subdirectory
    /// included, sets `complete=false` with a warning; nothing is deleted or silently ignored. Own
    /// publication leftovers are warnings only: for `modules`, `epics` and `atomics` the rule is the
    /// original one (canonical `M-` stems only), and for every other directory a leftover is own when
    /// [`own_temp_name`] holds and its stem is `<prefix><digits>.yaml` for the requested prefix.
    pub fn kind_inventory(&self, directory: &str, prefix: &str) -> Result<Inventory> {
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
            if own_inventory_temp(directory, prefix, &name) {
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
        let state = (write && relative == ".agent-tasks/write.lock").then(|| {
            self.state.locked.store(true, Ordering::SeqCst);
            self.state.clone()
        });
        Ok(Some(LockGuard { file, state }))
    }

    /// Preserve exact noncanonical bytes before a guarded canonical replacement.
    ///
    /// The canonical bytes must pass the exact read gate and decode back to `T` before anything is
    /// created, backed up or replaced; otherwise `encoding_unreadable` is returned with no effect.
    pub fn save<T: Serialize + DeserializeOwned>(
        &self,
        relative: &str,
        value: &T,
        observed: Option<&[u8]>,
        terminal: bool,
        effects: &mut Vec<String>,
    ) -> Result<String> {
        let bytes = encode(value)?;
        decode::<T>(&bytes).map_err(|_| {
            Error::new(
                "encoding_unreadable",
                "Canonical record would not read back as its own type; nothing was published.",
            )
        })?;
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
                    self.record_directory(".agent-tasks/backups");
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
        self.publish_inner(
            Publish {
                relative,
                bytes,
                observed,
                cap: RECORD_CAP,
                operation: None,
                attest: Attest::Optional,
            },
            effects,
        )
        .map(|_| ())
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

/// Whether `name` is this store's own publication leftover: `.<name>.tmp-<pid>-<seq>` or
/// `.<name>.rm-<pid>-<seq>` for a nonempty file name without separators; foreign dotfiles stay foreign.
pub fn own_temp_name(name: &str) -> bool {
    temp_stem(name).is_some()
}

/// The file name a leftover was made for, when `name` is one of our temp or detach siblings.
fn temp_stem(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('.')?;
    [".tmp-", ".rm-"].iter().find_map(|marker| {
        let (stem, suffix) = rest.rsplit_once(marker)?;
        let (pid, seq) = suffix.split_once('-')?;
        (!stem.is_empty()
            && !stem.contains(['/', '\0'])
            && pid.parse::<u32>().is_ok()
            && seq.parse::<u64>().is_ok())
        .then_some(stem)
    })
}

/// Leftover recognition for one inventory: the original `M-` rule for the work directories and the
/// prefix-specific stem rule for every other directory.
fn own_inventory_temp(directory: &str, prefix: &str, name: &str) -> bool {
    if matches!(directory, "modules" | "epics" | "atomics") {
        return own_temp(name);
    }
    temp_stem(name)
        .and_then(|stem| stem.strip_suffix(".yaml"))
        .is_some_and(|id| model::number(id, prefix).is_ok())
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

/// Canonical provider-owned boundary facts resolved from reciprocal Module obligations.
pub struct ContractFacts {
    /// Stable canonical ID.
    pub id: String,
    /// Exact positive canonical revision.
    pub revision: u64,
    /// Module owning the definition.
    pub provider: String,
    /// Participating Modules, including provider.
    pub parties: Vec<String>,
    /// Practical exact canonical/party semantic digest.
    pub snapshot: String,
    /// Explicit unresolved reciprocal/revision/artifact/confirmation gaps.
    pub gaps: Vec<String>,
}
impl Store {
    /// Resolve one canonical boundary without inferring a wait or treating ready=true as agreement.
    pub fn contract_facts(&self, id: &str) -> Result<ContractFacts> {
        model::text(id, 64).map_err(invalid)?;
        let scan = self.scan(None)?;
        if !scan.complete {
            return Err(invalid(
                "Contract scope has unreadable/incomplete Module facts.",
            ));
        }
        let providers = scan
            .modules
            .iter()
            .filter(|m| m.value.id.starts_with("M-"))
            .flat_map(|m| {
                m.value
                    .workflow
                    .iter()
                    .flat_map(|w| w.contracts.iter())
                    .flat_map(move |c| {
                        c.provides
                            .iter()
                            .filter(move |e| e.id.as_deref() == Some(id))
                            .map(move |e| (&m.value, e))
                    })
            })
            .collect::<Vec<_>>();
        let provider_ids = providers
            .iter()
            .map(|(m, _)| m.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if provider_ids.len() != 1 {
            return Err(invalid(format!(
                "Contract {id}: requires one canonical provider; found {}.",
                provider_ids.len()
            )));
        }
        let (provider, first) = providers
            .first()
            .ok_or_else(|| invalid("Missing canonical provider."))?;
        let revision = first
            .revision
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("Canonical contract revision is missing."))?;
        let mut parties = std::collections::BTreeSet::from([provider.id.clone()]);
        let mut declarations = Vec::new();
        let mut gaps = Vec::new();
        for (m, e) in &providers {
            if e.revision != Some(revision)
                || e.reference != first.reference
                || e.description != first.description
            {
                gaps.push(format!("{id}: canonical provider definitions disagree."));
            }
            parties.insert(e.peer.clone());
            declarations.push(serde_json::json!({"module":m.id,"direction":"provides","peer":e.peer,"id":e.id,"revision":e.revision,"reference":e.reference,"description":e.description}));
            match scan.modules.iter().find(|m|m.value.id==e.peer).and_then(|m|m.value.workflow.as_ref()).and_then(|w|w.contracts.as_ref()).and_then(|c|c.consumes.iter().find(|v|v.peer==provider.id&&v.id.as_deref()==Some(id))) {
                Some(c) if c.revision==Some(revision)&&c.reference==first.reference=>declarations.push(serde_json::json!({"module":e.peer,"direction":"consumes","peer":c.peer,"id":c.id,"revision":c.revision,"reference":c.reference,"description":c.description})),
                _=>gaps.push(format!("{}: confirm reciprocal consumes {id} revision {revision} and canonical artifact.",e.peer)),
            }
        }
        for consumer in &scan.modules {
            if let Some(c) = consumer
                .value
                .workflow
                .as_ref()
                .and_then(|w| w.contracts.as_ref())
            {
                for e in c.consumes.iter().filter(|e| e.id.as_deref() == Some(id)) {
                    if e.peer != provider.id
                        || !providers.iter().any(|(_, p)| p.peer == consumer.value.id)
                    {
                        gaps.push(format!(
                            "{}: unpaired or foreign consumes {id}.",
                            consumer.value.id
                        ));
                    }
                }
            }
        }
        declarations.sort_by_key(|v| v.to_string());
        let snapshot = semantic_digest(
            &serde_json::json!({"id":id,"revision":revision,"provider":provider.id,"declarations":declarations}),
        )?;
        Ok(ContractFacts {
            id: id.into(),
            revision,
            provider: provider.id.clone(),
            parties: parties.into_iter().collect(),
            snapshot,
            gaps,
        })
    }
    /// Relevant canonical facts, returning named failures rather than guessing external readiness.
    pub fn contract_ids(&self, m: &Module) -> Vec<String> {
        m.workflow
            .iter()
            .flat_map(|w| w.contracts.iter())
            .flat_map(|c| c.provides.iter().chain(&c.consumes))
            .filter_map(|e| e.id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    /// Check all exact affected-party confirmations; old lead history remains an agreed Module fact.
    pub fn agreement_gaps(&self, m: &Module) -> Vec<String> {
        let mut gaps = Vec::new();
        if m.core().is_some()
            && m.workflow
                .as_ref()
                .and_then(|w| w.contracts.as_ref())
                .is_none_or(|c| !c.not_required && c.provides.is_empty() && c.consumes.is_empty())
        {
            gaps.push("Explicitly declare contracts or not_required=true; unknown obligations cannot be treated as none.".into());
        }
        if m.workflow
            .as_ref()
            .and_then(|w| w.contracts.as_ref())
            .is_some_and(|c| {
                c.provides
                    .iter()
                    .chain(&c.consumes)
                    .any(|e| e.id.is_none() || e.revision.is_none_or(|r| r == 0))
            })
        {
            gaps.push(
                "Declare canonical id and positive revision for every affecting boundary.".into(),
            );
        }
        for id in self.contract_ids(m) {
            match self.contract_facts(&id) {
                Err(e) => gaps.push(e.message),
                Ok(f) => {
                    gaps.extend(f.gaps);
                    for party in &f.parties {
                        match self.module(party) {
                            Ok(p) => {
                                let agreed = p.value.core().is_some_and(|c| {
                                    c.agreements.iter().rev().any(|a| {
                                        a.contract_id == id
                                            && a.revision == f.revision
                                            && a.snapshot == f.snapshot
                                    })
                                });
                                if !agreed {
                                    gaps.push(format!("{party}: actual lead agreement missing for {id} revision {}.",f.revision));
                                }
                            }
                            Err(e) => {
                                gaps.push(format!("{party}: agreement unknown ({})", e.message))
                            }
                        }
                    }
                }
            }
        }
        gaps
    }
    /// Snapshot only affecting canonical obligations, excluding unrelated peer metadata.
    pub fn contract_basis(&self, m: &Module) -> Result<serde_json::Value> {
        let mut values = Vec::new();
        for id in self.contract_ids(m) {
            let f = self.contract_facts(&id)?;
            values.push(serde_json::json!({"id":f.id,"revision":f.revision,"snapshot":f.snapshot}));
        }
        Ok(serde_json::json!(values))
    }
    /// Current exact accepted candidate-set/related-contract coverage, excluding role/contact/delivery bookkeeping.
    pub fn coverage_basis(&self, participants: &[String]) -> Result<String> {
        let mut modules = Vec::new();
        let mut contracts = std::collections::BTreeSet::new();
        for id in participants {
            let m = self.module(id)?.value;
            if !self.module_ready(&m) {
                return Err(invalid(format!(
                    "{id}: current positive candidate review is not ready."
                )));
            }
            let candidate = m
                .result
                .as_ref()
                .and_then(|r| r.candidate.as_ref())
                .ok_or_else(|| invalid(format!("{id}: definite candidate missing.")))?;
            modules.push(serde_json::json!({"id":id,"candidate":candidate,"implementation_epoch":m.core().map(|c|c.implementation_epoch)}));
            for cid in self.contract_ids(&m) {
                let f = self.contract_facts(&cid)?;
                if f.parties
                    .iter()
                    .filter(|p| participants.contains(p))
                    .count()
                    >= 2
                {
                    contracts.insert(cid);
                }
            }
        }
        modules.sort_by_key(|v| v.to_string());
        let mut boundaries = Vec::new();
        for id in contracts {
            let f = self.contract_facts(&id)?;
            boundaries
                .push(serde_json::json!({"id":id,"revision":f.revision,"snapshot":f.snapshot}));
        }
        semantic_digest(&serde_json::json!({"modules":modules,"contracts":boundaries}))
    }
    /// Current intrinsic Module review readiness; final target-branch delivery is independent in core mode.
    pub fn module_ready(&self, m: &Module) -> bool {
        if m.state == model::ModuleState::Canceled {
            return false;
        }
        if m.core().is_none() {
            return self.phase(m) == "accepted";
        }
        m.reviews.last().is_some_and(|r| {
            r.verdict == model::Verdict::Accepted
                && r.epoch == m.review_epoch
                && self.work_basis(m).is_ok_and(|b| b == r.basis)
        }) && self.core_review_gaps(m).is_empty()
    }
    /// Candidate/planning/agreement/control facts required for a core Module positive review.
    pub fn core_review_gaps(&self, m: &Module) -> Vec<String> {
        let mut missing = Vec::new();
        let Some(core) = m.core() else {
            return missing;
        };
        if !m.id.starts_with("M-") {
            return missing;
        }
        let candidate = m.result.as_ref().and_then(|r| r.candidate.as_ref());
        if candidate.is_none() {
            missing.push("Report the definite restored Module candidate.".into());
        }
        if !core
            .planning
            .as_ref()
            .is_some_and(|p| m.plan_basis().is_ok_and(|b| b == p.basis))
        {
            missing.push("Current bound lead discovery/planning is missing or stale.".into());
        }
        missing.extend(self.agreement_gaps(m));
        let ids = self.contract_ids(m);
        let scopes = if ids.is_empty() {
            vec!["local".to_owned()]
        } else {
            ids
        };
        for id in scopes {
            let revision = if id == "local" {
                1
            } else {
                match self.contract_facts(&id) {
                    Ok(f) => f.revision,
                    Err(e) => {
                        missing.push(e.message);
                        continue;
                    }
                }
            };
            let intent_basis = m.plan_basis().ok();
            let valid = core.boundary_evidence.iter().rev().any(|e| {
                e.contract_id == id
                    && e.revision == revision
                    && e.intent_basis.as_ref() == intent_basis.as_ref()
                    && Some(&e.candidate) == candidate
                    && e.correct.status == model::CheckStatus::Passed
                    && e.failed.status == model::CheckStatus::Failed
                    && e.restored.status == model::CheckStatus::Passed
                    && m.workflow
                        .as_ref()
                        .and_then(|w| w.execution.as_ref())
                        .is_some_and(|w| w.worktree == e.worktree)
            });
            if !valid {
                missing.push(format!(
                    "{id}: current correct-pass/mutant-fail/restored-pass observations missing."
                ));
            }
        }
        missing
    }
    /// Current connected ready components of at least two; unrelated unfinished Modules create no barrier.
    pub fn ready_sets(&self, epic: Option<&Module>) -> Result<Vec<Vec<String>>> {
        let scan = self.scan(None)?;
        if !scan.complete {
            return Err(invalid(
                "Ready-set scope has unreadable/partial work; inspect named coverage.",
            ));
        }
        let eligible = scan
            .modules
            .iter()
            .filter(|m| {
                m.value.id.starts_with("M-")
                    && epic.is_none_or(|e| e.modules.contains(&m.value.id))
                    && self.module_ready(&m.value)
            })
            .map(|m| m.value.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut graph = BTreeMap::<String, std::collections::BTreeSet<String>>::new();
        for id in &eligible {
            graph.insert(id.clone(), std::collections::BTreeSet::new());
            let m = self.module(id)?.value;
            for cid in self.contract_ids(&m) {
                let f = self.contract_facts(&cid)?;
                for peer in f.parties.iter().filter(|p| {
                    *p != id && eligible.contains(*p) && (f.provider == *id || f.provider == **p)
                }) {
                    graph.entry(id.clone()).or_default().insert(peer.clone());
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut sets = Vec::new();
        for id in &eligible {
            if seen.contains(id) {
                continue;
            }
            let mut stack = vec![id.clone()];
            let mut component = Vec::new();
            while let Some(n) = stack.pop() {
                if !seen.insert(n.clone()) {
                    continue;
                }
                component.push(n.clone());
                stack.extend(graph.get(&n).into_iter().flatten().cloned());
            }
            component.sort();
            if component.len() >= 2 {
                sets.push(component);
            }
        }
        Ok(sets)
    }
    /// Test connectivity of an explicit ready subset; a job may assemble AB before later ABC.
    pub fn connected(&self, participants: &[String]) -> Result<bool> {
        if participants.len() < 2 {
            return Ok(false);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![participants[0].clone()];
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let m = self.module(&id)?.value;
            for cid in self.contract_ids(&m) {
                let f = self.contract_facts(&cid)?;
                stack.extend(
                    f.parties
                        .iter()
                        .filter(|p| {
                            participants.contains(p)
                                && !seen.contains(*p)
                                && (f.provider == id || f.provider == ***p)
                        })
                        .cloned(),
                );
            }
        }
        Ok(seen.len() == participants.len())
    }
    /// Existing accepted integration matching unchanged candidate/contract/scenario/environment inputs.
    pub fn covered_integration(
        &self,
        participants: &[String],
        environment: &str,
        scenarios: &[String],
        exclude: Option<&str>,
    ) -> Result<Option<String>> {
        let basis = self.coverage_basis(participants)?;
        let scan = self.scan(None)?;
        if !scan.complete {
            return Err(invalid("Integration inventory is partial."));
        }
        for s in scan.modules {
            let a = s.value;
            if exclude == Some(a.id.as_str()) || !a.id.starts_with("A-") || a.core().is_none() {
                continue;
            }
            let same = a
                .participants
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == participants
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>();
            if same
                && a.workflow.as_ref().is_some_and(|w| {
                    w.environment.as_deref() == Some(environment) && w.scenarios == scenarios
                })
                && a.participant_basis.get("core") == Some(&basis)
                && self.phase(&a) == "accepted"
            {
                return Ok(Some(a.id));
            }
        }
        Ok(None)
    }
    /// Pending current-key assembly claims are not verified coverage; inspect them before another dispatch.
    /// Root locking serializes begin claims; canceled or changed-input claims do not block a fresh key.
    pub fn pending_integration(
        &self,
        participants: &[String],
        environment: &str,
        scenarios: &[String],
        exclude: Option<&str>,
    ) -> Result<Option<String>> {
        let key = self.coverage_basis(participants)?;
        let scan = self.scan(None)?;
        if !scan.complete {
            return Err(invalid(
                "Pending integration inventory is partial; inspect before dispatch.",
            ));
        }
        for s in scan.modules {
            let a = s.value;
            if exclude == Some(a.id.as_str())
                || !a.id.starts_with("A-")
                || a.state != model::ModuleState::Open
                || a.core().is_none()
            {
                continue;
            }
            let same = a
                .participants
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                == participants
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>();
            if same
                && a.workflow.as_ref().is_some_and(|w| {
                    w.active
                        && w.environment.as_deref() == Some(environment)
                        && w.scenarios == scenarios
                })
                && a.participant_basis.get("core") == Some(&key)
            {
                return Ok(Some(a.id));
            }
        }
        Ok(None)
    }
    /// Exact current business verification applicability; AB+BC is never inferred as an ABC E2E report.
    pub fn criterion_basis(
        &self,
        scope: &model::CriterionScope,
        integration: Option<&str>,
    ) -> Result<String> {
        let coverage = self.coverage_basis(&scope.modules)?;
        let related = if let Some(id) = integration {
            let a = self.module(id)?.value;
            if self.phase(&a) != "accepted"
                || !scope.modules.iter().all(|m| a.participants.contains(m))
            {
                return Err(invalid(
                    "Criterion needs ONE current accepted composition covering its affected set.",
                ));
            }
            Some(
                serde_json::json!({"ref":id,"basis":a.basis().map_err(invalid)?,"coverage":a.participant_basis}),
            )
        } else {
            None
        };
        semantic_digest(
            &serde_json::json!({"scope":scope,"coverage":coverage,"integration":related}),
        )
    }
}
/// Encode one local semantic applicability digest; no certificate or external observation is manufactured.
fn semantic_digest(value: &serde_json::Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(value).map_err(|_| invalid("Cannot encode semantic coverage."))?
        )
    ))
}

/// Preflight-level regressions: every assertion calls the subset scanner itself and checks its own
/// refusal message, so a later parser or tree error can never be mistaken for scanner protection.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Explicit isolated scanner assertions"
)]
mod yaml_preflight_tests {
    use super::{read_tree, yaml_subset};

    /// Fixed refusal text for property indicators at a node start.
    const PROPERTY: &str = "YAML tags, anchors and aliases are unsupported";
    /// Fixed refusal text for quote or flow state left open at a line end.
    const MULTILINE: &str = "single-line";
    /// Fixed refusal text for line breaks other than LF and CRLF, and for stray byte order marks.
    const BREAKS: &str = "line breaks";

    /// Assert the scanner alone refuses every `(input, expected)` pair with a message containing
    /// `expected`; every wrong outcome is listed, not only the first.
    fn assert_refused<'a>(cases: impl IntoIterator<Item = (&'a str, &'a str)>) {
        let wrong: Vec<String> = cases
            .into_iter()
            .filter_map(|(input, expected)| match yaml_subset(input.as_bytes()) {
                Ok(()) => Some(format!("{input:?} accepted")),
                Err(e) if !e.message.contains(expected) => {
                    Some(format!("{input:?} refused with {:?}", e.message))
                }
                Err(_) => None,
            })
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Assert the scanner alone refuses one input with a message containing `expected`.
    fn refused(input: &str, expected: &str) {
        assert_refused([(input, expected)]);
    }

    /// The five raw inputs from the source review, plus byte order mark and later-document cases.
    /// Each is valid for the real parser, so the scanner is the only thing standing in front of
    /// alias expansion; the test proves that by parsing the raw bytes first.
    #[test]
    fn reviewed_attack_inputs_are_refused_by_the_preflight() {
        assert_refused([
            ("a: x\n  [y\nb:\n- &anc v\n- *anc\n", MULTILINE),
            ("[? &x a : 1, ? *x : 2]\n", PROPERTY),
            ("a:\r  &x [1]\rb:\r  *x\r", BREAKS),
            ("a: x\n  'y\nb: &anc v\nc: *anc\nd: it's\n", MULTILINE),
            ("a\u{a0}#: &x [1]\nb\u{a0}#: *x\n", PROPERTY),
            ("\u{feff}&r a: 1\n", PROPERTY),
            ("a: 1\n\u{feff}  &r b: 2\nc: *r\n", BREAKS),
            ("a: |\n  x\n\u{85}  &y z: 1\n", BREAKS),
            ("a: x\u{2028}  &y z: 1\n", BREAKS),
            ("a: x\u{2029}b: &y 1\n", BREAKS),
            ("[?&x a : 1, ?*x : 2]\n", PROPERTY),
            ("{?!t a: 1}\n", PROPERTY),
        ]);
        for raw in [
            "a: x\n  [y\nb:\n- &anc v\n- *anc\n",
            "[? &x a : 1, ? *x : 2]\n",
            "a\u{a0}#: &x [1]\nb\u{a0}#: *x\n",
        ] {
            assert!(
                serde_yaml_ng::from_slice::<serde_yaml_ng::Value>(raw.as_bytes()).is_ok(),
                "{raw:?} must be accepted by the raw parser for this proof"
            );
        }
    }

    /// Inside a flow collection the real parser takes `?` as an explicit-key marker even without
    /// a following space, so a property right after it starts a node. The raw parser is checked
    /// first; the scanner has to be the one that refuses what the parser accepts.
    #[test]
    fn flow_question_mark_key_without_space_is_a_node_start() {
        for raw in ["[?&x a : 1, ?*x : 2]\n", "{?!t a: 1}\n"] {
            let parsed = serde_yaml_ng::from_slice::<serde_yaml_ng::Value>(raw.as_bytes());
            assert!(parsed.is_ok(), "{raw:?} raw parser: {parsed:?}");
            assert_refused([(raw, PROPERTY)]);
        }
    }

    /// Property indicators at every node start are refused by the scanner, with defined anchors.
    #[test]
    fn property_indicators_at_node_starts_are_refused() {
        assert_refused(
            [
                "a: &x 1\nb: *x\n",
                "a: !!str x\n",
                "a: !Tag x\n",
                "a: ! x\n",
                "a:\t&x 1\n",
                "- &x one\n- *x\n",
                "- - &x a\n",
                "a:\n  - &x b\n",
                "a:\n  &x b\n",
                "&x a: 1\nb: *x\n",
                "!t a: 1\n",
                "? &x k\n: v\n",
                "k: [&x a, *x]\n",
                "k: [a, !t b]\n",
                "k: {a: &x 1}\n",
                "k: {a: 1, b: !t x}\n",
                "k: [? &x a : 1]\n",
                "--- &x\na: 1\n",
                "--- !t\na: 1\n",
                "a: |\n  text\nb: &x 1\nc: *x\n",
                "- a: |\n    x\n  b: &x 1\n  c: *x\n",
                "a: >-\n  text\n&y z: 1\nw: *y\n",
                "a: \u{a0}b #c\nd: &x 1\n",
            ]
            .map(|bad| (bad, PROPERTY)),
        );
    }

    /// Multiple documents, merges, complex keys and duplicate keys keep failing, each with its own
    /// preflight or tree-level refusal rather than a coincidental parse error.
    #[test]
    fn other_forbidden_shapes_keep_their_own_refusals() {
        refused("--- \na: 1\n--- \na: 2\n", "One YAML document");
        for (input, expected) in [
            ("<<: {a: 1}\n", "ordinary strings"),
            ("a: {<<: {x: 1}}\n", "ordinary strings"),
            ("? [a, b]\n: x\n", "ordinary strings"),
            ("[a]: x\n", "ordinary strings"),
            ("a: 1\na: 2\n", "duplicate keys"),
            ("a: 1\n---\na: 2\n", "duplicate keys"),
        ] {
            let error = read_tree(input.as_bytes()).unwrap_err();
            assert!(
                error.message.contains(expected),
                "{input:?}: {}",
                error.message
            );
        }
    }

    /// Text that is merely punctuation stays readable in every position the writer can emit.
    #[test]
    fn literal_text_controls_pass_the_preflight() {
        for good in [
            "a: valid_reference(store,&str)\n",
            "a: 'x &y *z !w, &v'\n",
            "a: \"x &y *z !w, &v \\\" &q\"\n",
            "a: [b, \"&c\", 'd, *e']\n",
            "a: {b: \"&c\", d: 'e, !f'}\n",
            "a: |\n  &x text\n  *y !z\nb: ok\n",
            "a: >-\n  &x folded\n  *y\nb: ok\n",
            "- a: |\n    &x text\n  b: ok\n",
            "- |\n  &x text\n- ok\n",
            "# &x comment\na: b # *y !z\n",
            "a: b&c\nd: e*f\ng: h!i\n",
            "a: x - &y\n",
            "a\u{a0}#b: c\u{a0}&d\n",
            "a: b\r\nc: d&x\r\n",
            "\u{feff}a: 1\n",
        ] {
            yaml_subset(good.as_bytes()).unwrap_or_else(|e| panic!("{good:?}: {}", e.message));
        }
    }

    /// Native multi-line quoted scalars and flow collections are a deliberate unsupported shape:
    /// they are refused as a whole, and the canonical writer emits only single-line forms.
    #[test]
    fn multiline_quotes_and_flow_are_refused_by_design() {
        assert_refused(
            [
                "a: \"multi\n  line\"\n",
                "a: 'multi\n  line'\n",
                "a: [1,\n  2]\n",
                "a: {b: 1,\n  c: 2}\n",
                "a: \"escaped \\\n  break\"\n",
            ]
            .map(|bad| (bad, MULTILINE)),
        );
    }
}
