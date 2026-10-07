//! Managed Markdown documents: the safe ASCII path namespace, DOC metadata records, honest state
//! classification, bounded inventory and corpus, exact reads, and the guarded mutations (save,
//! section replace, adopt, remove, relocate) with disclosed partial publication.
//!
//! Every filesystem fact reaches this module through the [`Port`] seam, so the same logic runs on
//! the real store and on a fault-injecting test double. Observation never writes. Mutations assume
//! the caller already holds the single root write lock and never lock again.
use crate::{
    markdown::{self, Outline, Selector, Wire},
    persist::{EffectStatus, ExpectedEffect, PathEffect, effect_status},
    references::{self, ReferenceCheck},
    store::{
        self, Attest, DirListing, EffectKind, EntryKind, Error, OperationId, Publication, Result,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::BTreeMap, ops::Range};

/// Largest managed body, in bytes, that a save accepts and a read treats as supported.
pub const BODY_CAP: usize = 512 * 1024;
/// Largest one-line purpose, in bytes.
pub const PURPOSE_CAP: usize = 240;
/// Largest relative path, in bytes.
pub const PATH_CAP: usize = 160;
/// Directory segments allowed between `docs/` and the file name.
pub const DEPTH_CAP: usize = 3;
/// Managed Markdown files inventoried before the listing reports a cap.
pub const FILE_CAP: usize = 1024;
/// Metadata records inventoried before the listing reports a cap.
pub const DOC_RECORD_CAP: usize = 1024;
/// Bytes one corpus, inventory or reference scan may read in total.
pub const SCAN_BYTES_CAP: usize = store::SCAN_CAP;

/// Validated managed path: `README.md` or `docs/<dirs>/<name>.md`, ASCII only.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocPath(String);

/// Windows device names that are never accepted as a file stem.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Whether `seg` is a valid path segment of the managed grammar.
fn segment_ok(seg: &str) -> bool {
    let b = seg.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !seg.ends_with('.')
}

impl DocPath {
    /// Parse a managed path. Errors: `path` (grammar), `invalid_arguments` (length cap).
    pub fn parse(raw: &str) -> Result<DocPath> {
        let bad = |why: &str| Error::new("path", format!("Not a managed document path: {why}."));
        if raw.len() > PATH_CAP {
            return Err(Error::new("invalid_arguments", "path: exceeds 160 bytes."));
        }
        if !raw.is_ascii() {
            return Err(bad("only ASCII names are supported"));
        }
        let parts: Vec<&str> = raw.split('/').collect();
        if parts == ["README.md"] {
            return Ok(DocPath(raw.into()));
        }
        if parts.first() != Some(&"docs") || parts.len() < 2 || parts.len() > DEPTH_CAP + 2 {
            return Err(bad("use README.md or docs/<up to 3 folders>/<name>.md"));
        }
        if !parts[1..].iter().all(|s| segment_ok(s)) {
            return Err(bad(
                "a segment is empty, too long, dotted or has unsupported characters",
            ));
        }
        let name = parts[parts.len() - 1];
        let stem = name.strip_suffix(".md").filter(|s| !s.is_empty());
        let Some(stem) = stem else {
            return Err(bad("the file name must end in .md"));
        };
        let device = stem.split('.').next().unwrap_or(stem).to_ascii_lowercase();
        if RESERVED.contains(&device.as_str()) {
            return Err(bad("reserved device name"));
        }
        Ok(DocPath(raw.into()))
    }

    /// The path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Directory part for relative link resolution: empty for `README.md`, else `docs` or `docs/a`.
    pub fn dir(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(d, _)| d)
    }
}

/// Reference to one document: by DOC identifier or by managed path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ref {
    /// Canonical `DOC-<digits>` identifier.
    Id(String),
    /// Managed path.
    Path(DocPath),
}

/// Whether `id` is a canonical document identifier: `DOC-` and a positive number with at least
/// three digits and no other leading zero.
pub fn doc_id_ok(id: &str) -> bool {
    id.strip_prefix("DOC-")
        .and_then(|d| d.parse::<u64>().ok().map(|n| (n, d)))
        .is_some_and(|(n, d)| n > 0 && d == format!("{n:03}"))
}

impl Ref {
    /// Parse `DOC-001`, `README.md` or `docs/x.md`. Errors: `invalid_arguments` or `path`.
    pub fn parse(raw: &str) -> Result<Ref> {
        if raw.starts_with("DOC-") {
            return if doc_id_ok(raw) {
                Ok(Ref::Id(raw.into()))
            } else {
                Err(Error::new(
                    "invalid_arguments",
                    "ref: noncanonical DOC identifier.",
                ))
            };
        }
        DocPath::parse(raw).map(Ref::Path)
    }
}

/// Why a native file is not a supported document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// The name is outside the ASCII managed grammar.
    Name,
    /// The bytes are not valid UTF-8.
    NotUtf8,
    /// The file is larger than [`BODY_CAP`].
    TooLarge,
    /// A symbolic link was found where a file or folder is expected.
    Symlink,
    /// The entry is not a regular file.
    NotRegular,
    /// Another sibling differs only by ASCII case.
    Collision,
    /// The record inventory is incomplete, so claims on the path cannot be decided.
    PartialCoverage,
}

/// Document state derived from actual bytes and records at observation time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No file and no active record claims the path.
    Absent,
    /// A supported file that no active record claims; its metadata is unknown.
    Unmanaged,
    /// An active record whose hash and length equal the file.
    Managed,
    /// An active record and a file with different bytes; the cause is not guessed.
    Drifted,
    /// An active record and no file.
    MissingBody,
    /// More than one active record claims the path.
    Conflict,
    /// Observed by identifier: the record is retired (historic metadata).
    Retired,
    /// The native file or the coverage is not supported; the reason is named.
    Unsupported(Unsupported),
}

impl State {
    /// Lowercase label used in acknowledgements (`managed`, `missingbody`, ...).
    pub fn label(&self) -> &'static str {
        match self {
            State::Absent => "absent",
            State::Unmanaged => "unmanaged",
            State::Managed => "managed",
            State::Drifted => "drifted",
            State::MissingBody => "missingbody",
            State::Conflict => "conflict",
            State::Retired => "retired",
            State::Unsupported(_) => "unsupported",
        }
    }
}

/// How a record came to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Created by a managed save.
    Saved,
    /// Created by adopting existing bytes.
    Adopted,
}

/// Record lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordState {
    /// The record describes a live path.
    Active,
    /// The record is historic metadata; its identifier is never reused.
    Retired,
}

/// Closed metadata record `documents/DOC-<n>.yaml`; generated facts only, never body text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// Schema revision; only 1 is understood.
    pub schema_version: u32,
    /// Canonical identifier equal to the file name.
    pub id: String,
    /// Managed path of the described document (the last path when retired).
    pub path: String,
    /// One-line purpose, 1 to [`PURPOSE_CAP`] bytes.
    pub purpose: String,
    /// Creation route.
    pub origin: Origin,
    /// Lifecycle.
    pub state: RecordState,
    /// 1 at creation, plus 1 per managed save, adopt or relocate.
    pub revision: u64,
    /// Generated RFC 3339 creation time.
    pub created_at: String,
    /// Generated RFC 3339 time of the last managed change.
    pub updated_at: String,
    /// Generated retirement time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<String>,
    /// Declared writer of the last managed change, never inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// Hex sha256 of the bytes last published or adopted.
    pub body_sha256: String,
    /// Length in bytes of those bytes.
    pub body_bytes: u64,
}

/// Hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Validate a one-line purpose. Errors: `invalid_arguments` naming `purpose`.
fn purpose_ok(purpose: &str) -> Result<()> {
    if purpose.trim().is_empty()
        || purpose.len() > PURPOSE_CAP
        || purpose.chars().any(|c| c.is_control())
    {
        return Err(Error::new(
            "invalid_arguments",
            "purpose: one line of 1 to 240 bytes without control characters.",
        ));
    }
    Ok(())
}

impl Record {
    /// Check the closed contract of a decoded record against its file name.
    fn validate(&self, name_id: &str) -> std::result::Result<(), String> {
        if self.schema_version != 1 {
            return Err("unknown schema".into());
        }
        if self.id != name_id || !doc_id_ok(&self.id) {
            return Err("identifier differs from the file name".into());
        }
        DocPath::parse(&self.path).map_err(|_| "invalid path".to_string())?;
        purpose_ok(&self.purpose).map_err(|_| "invalid purpose".to_string())?;
        if self.revision == 0
            || self.body_sha256.len() != 64
            || !self.body_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid revision or digest".into());
        }
        for at in [&self.created_at, &self.updated_at] {
            chrono::DateTime::parse_from_rfc3339(at).map_err(|_| "invalid date".to_string())?;
        }
        Ok(())
    }
}

/// Relative path of the record file for `id`.
pub fn record_rel(id: &str) -> String {
    format!("documents/{id}.yaml")
}

// ---------------------------------------------------------------------------------------------
// Storage seam
// ---------------------------------------------------------------------------------------------

/// Result of reading one owned file.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    /// No such file.
    Absent,
    /// Exact bytes.
    Bytes(Vec<u8>),
    /// A symbolic link was found in the path.
    Link,
    /// The leaf is not a regular file.
    NotRegular,
    /// The file is larger than the requested cap.
    TooLarge,
}

/// One guarded creation (`observed` absent) or replacement.
pub struct Put<'a> {
    /// Owned relative path.
    pub rel: &'a str,
    /// Exact bytes to publish.
    pub bytes: &'a [u8],
    /// Exact current bytes for a replacement; `None` creates without clobber.
    pub observed: Option<&'a [u8]>,
    /// Largest accepted `bytes.len()`.
    pub cap: usize,
    /// Operation identity carried on the typed event.
    pub op: Option<&'a OperationId>,
    /// Whether the publication must be attested by the private journal (needs `op`).
    pub required: bool,
}

/// One guarded removal.
pub struct Del<'a> {
    /// Owned relative path.
    pub rel: &'a str,
    /// Exact current bytes; removal happens only when they still match.
    pub observed: &'a [u8],
    /// Largest accepted `observed.len()`.
    pub cap: usize,
    /// Operation identity carried on the typed event.
    pub op: Option<&'a OperationId>,
    /// Whether the removal must be attested by the private journal.
    pub required: bool,
}

/// The storage, allocator and oracle calls this module needs. The real store implements it with the
/// publication primitives and the persistence oracle; tests wrap that real implementation with
/// scripted faults and a stand-in allocator.
pub trait Port {
    /// Root- and path-bound version of exact bytes or of absence.
    fn version(&self, rel: &str, bytes: Option<&[u8]>) -> String;
    /// Read one owned file with a cap.
    fn read(&self, rel: &str, cap: usize) -> Result<Read>;
    /// List one owned directory, sorted, with at most `cap` entries.
    fn list(&self, dir: &str, cap: usize) -> Result<DirListing>;
    /// The bounded ID inventory of one record directory.
    fn inventory(&self, dir: &str, prefix: &str) -> Result<store::Inventory>;
    /// Guarded create or replace.
    fn put(&self, put: Put<'_>) -> Result<()>;
    /// Guarded removal.
    fn del(&self, del: Del<'_>) -> Result<()>;
    /// Create missing ancestor directories of `rel` (at most four levels).
    fn ensure_parents(&self, rel: &str) -> Result<()>;
    /// Every typed event this request produced, in order, including events of failed calls.
    fn events(&self) -> Vec<Publication>;
    /// Ask the operation oracle what an operation identity attests.
    fn status(&self, op: &OperationId, expected: &[ExpectedEffect]) -> EffectStatus;
    /// Reserve the next DOC identifier (published before the record; gaps are retained).
    fn reserve_id(&self) -> Result<String>;
    /// Validated records of one structured home, loaded by the owners' loaders.
    fn records(&self, home: &str) -> Result<crate::references::RecordSet>;
    /// Prove that a top-level work, knowledge or compaction identifier (and optional child) exists.
    fn resolve_id(&self, id: &str) -> Result<crate::references::Resolution>;
}

/// [`Port`] over the real request [`store::Store`]: publication primitives with typed events and
/// journal attestation, the persistence oracle and the work and DOC record loaders. Allocation of a
/// DOC identifier and the typed knowledge loaders belong to the knowledge owner and are reported as
/// unavailable until that source exists; nothing here imitates them.
pub struct StorePort<'a> {
    /// The resolved request store; the caller holds its root write lock for mutations.
    pub store: &'a store::Store,
    /// Human effect strings, in order.
    pub fx: RefCell<Vec<String>>,
}

impl<'a> StorePort<'a> {
    /// Wrap one request store.
    pub fn new(store: &'a store::Store) -> Self {
        Self {
            store,
            fx: RefCell::new(Vec::new()),
        }
    }
}

impl Port for StorePort<'_> {
    fn version(&self, rel: &str, bytes: Option<&[u8]>) -> String {
        self.store.version(rel, bytes)
    }

    fn read(&self, rel: &str, cap: usize) -> Result<Read> {
        match self.store.read_exact(rel, cap) {
            Ok(None) => Ok(Read::Absent),
            Ok(Some(o)) => Ok(Read::Bytes(o.bytes)),
            Err(e) if e.code == "capacity" => Ok(Read::TooLarge),
            Err(e) if e.code == "file_type" && e.message.contains("Symlink") => Ok(Read::Link),
            Err(e) if e.code == "file_type" => Ok(Read::NotRegular),
            Err(e) => Err(e),
        }
    }

    fn list(&self, dir: &str, cap: usize) -> Result<DirListing> {
        self.store.list_dir(dir, cap)
    }

    fn inventory(&self, dir: &str, prefix: &str) -> Result<store::Inventory> {
        self.store.kind_inventory(dir, prefix)
    }

    fn put(&self, p: Put<'_>) -> Result<()> {
        let request = store::Publish {
            relative: p.rel,
            bytes: p.bytes,
            observed: p.observed,
            cap: p.cap,
            operation: p.op,
            attest: if p.required {
                Attest::Required
            } else {
                Attest::Optional
            },
        };
        self.store
            .publish_with(request, &mut self.fx.borrow_mut())
            .map(|_| ())
    }

    fn del(&self, d: Del<'_>) -> Result<()> {
        let request = store::Remove {
            relative: d.rel,
            observed: d.observed,
            cap: d.cap,
            operation: d.op,
            attest: if d.required {
                Attest::Required
            } else {
                Attest::Optional
            },
        };
        self.store
            .remove(request, &mut self.fx.borrow_mut())
            .map(|_| ())
    }

    fn ensure_parents(&self, rel: &str) -> Result<()> {
        self.store
            .ensure_parents(rel, &mut self.fx.borrow_mut())
            .map(|_| ())
    }

    fn events(&self) -> Vec<Publication> {
        self.store.publications()
    }

    fn status(&self, op: &OperationId, expected: &[ExpectedEffect]) -> EffectStatus {
        effect_status(self.store, op, expected)
    }

    fn reserve_id(&self) -> Result<String> {
        Err(Error::new(
            "allocator",
            "The knowledge allocator source is not available yet; nothing was reserved.",
        ))
    }

    fn records(&self, home: &str) -> Result<references::RecordSet> {
        references::interim_records(self.store, home)
    }

    fn resolve_id(&self, id: &str) -> Result<references::Resolution> {
        references::interim_resolve(self.store, self, id)
    }
}

// ---------------------------------------------------------------------------------------------
// Records, gaps and states
// ---------------------------------------------------------------------------------------------

/// Why a name or file is reported instead of read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapReason {
    /// The file or record could not be read or validated.
    Unreadable,
    /// The record has an unknown schema.
    UnknownSchema,
    /// The bytes are not UTF-8.
    NotUtf8,
    /// A cap was reached; counts are lower bounds.
    Capped,
    /// The directory holds an entry no owner recognizes.
    UnrecognizedEntry,
    /// A home exists as a file or a link instead of a directory.
    NotADirectory,
    /// The file name is outside the managed grammar.
    Name,
    /// A sibling differs only by ASCII case.
    Collision,
    /// A symbolic link was found.
    Link,
    /// The entry is not a regular file.
    NotRegular,
    /// The file is larger than the body cap.
    TooLarge,
    /// The owner's loader for this home is not available yet.
    LoaderUnavailable,
}

/// One named omission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    /// Quoted name, path or home.
    pub what: String,
    /// Reason.
    pub reason: GapReason,
}

/// One loaded metadata record with its exact bytes.
#[derive(Clone, Debug)]
pub struct RecordEntry {
    /// Relative path of the record file.
    pub rel: String,
    /// Exact file bytes.
    pub bytes: Vec<u8>,
    /// Decoded and validated record.
    pub record: Record,
}

/// All metadata records of a root.
#[derive(Clone, Debug, Default)]
pub struct Records {
    /// Valid records sorted by identifier.
    pub entries: Vec<RecordEntry>,
    /// Named omissions.
    pub gaps: Vec<Gap>,
    /// False when claims cannot be decided.
    pub complete: bool,
}

/// Quote a name for display: escapes, caps and never raw control characters.
pub fn quote(name: &str) -> String {
    store::safe(name, 160)
}

/// Load and validate every record under `documents/`, enumerated with the store's bounded ID
/// inventory (its own publication leftovers are warnings, any other entry makes it incomplete).
pub fn load_records(port: &dyn Port) -> Result<Records> {
    let inv = port.inventory("documents", "DOC-")?;
    let mut out = Records {
        complete: inv.complete,
        ..Default::default()
    };
    for warning in inv
        .warnings
        .iter()
        .filter(|w| !w.starts_with("Orphan publication temp"))
    {
        out.gaps.push(Gap {
            what: quote(warning),
            reason: GapReason::UnrecognizedEntry,
        });
    }
    for id in inv.ids.iter().take(DOC_RECORD_CAP) {
        let rel = record_rel(id);
        let name = format!("{id}.yaml");
        let gap = |reason| Gap {
            what: quote(&name),
            reason,
        };
        match port.read(&rel, store::RECORD_CAP)? {
            Read::Bytes(bytes) => match store::decode::<Record>(&bytes) {
                Ok(record) if record.validate(id).is_ok() => {
                    out.entries.push(RecordEntry { rel, bytes, record });
                }
                Ok(record) if record.schema_version != 1 => {
                    out.complete = false;
                    out.gaps.push(gap(GapReason::UnknownSchema));
                }
                _ => {
                    out.complete = false;
                    out.gaps.push(gap(GapReason::Unreadable));
                }
            },
            _ => {
                out.complete = false;
                out.gaps.push(gap(GapReason::Unreadable));
            }
        }
    }
    if inv.ids.len() > DOC_RECORD_CAP {
        out.complete = false;
        out.gaps.push(Gap {
            what: "documents/".into(),
            reason: GapReason::Capped,
        });
    }
    out.entries.sort_by(|a, b| a.record.id.cmp(&b.record.id));
    Ok(out)
}

impl Records {
    /// Active records that claim `path`.
    pub fn claims(&self, path: &DocPath) -> Vec<&RecordEntry> {
        self.entries
            .iter()
            .filter(|e| e.record.state == RecordState::Active && e.record.path == path.as_str())
            .collect()
    }

    /// The record with identifier `id`.
    pub fn by_id(&self, id: &str) -> Option<&RecordEntry> {
        self.entries.iter().find(|e| e.record.id == id)
    }
}

/// What reading one body found.
enum Body {
    /// No file.
    Absent,
    /// Valid UTF-8 bytes.
    Bytes(Vec<u8>),
    /// A named unsupported native file.
    Unsupported(Unsupported),
}

/// Whether every component of `path` exists on disk with exactly this spelling. A case-insensitive
/// filesystem answers reads for a differently cased name, which must never be edited as this path.
fn exact_names(port: &dyn Port, path: &DocPath) -> Result<bool> {
    let mut dir = String::new();
    for part in path.as_str().split('/') {
        let listing = port.list(&dir, FILE_CAP * 2)?;
        if !listing.entries.iter().any(|e| e.name == part) {
            return Ok(false);
        }
        if !dir.is_empty() {
            dir.push('/');
        }
        dir.push_str(part);
    }
    Ok(true)
}

/// Read and vet one body.
fn read_body(port: &dyn Port, path: &DocPath) -> Result<Body> {
    Ok(match port.read(path.as_str(), BODY_CAP)? {
        Read::Absent => Body::Absent,
        Read::Link => Body::Unsupported(Unsupported::Symlink),
        Read::NotRegular => Body::Unsupported(Unsupported::NotRegular),
        Read::TooLarge => Body::Unsupported(Unsupported::TooLarge),
        Read::Bytes(b) if std::str::from_utf8(&b).is_err() => {
            Body::Unsupported(Unsupported::NotUtf8)
        }
        Read::Bytes(b) => Body::Bytes(b),
    })
}

/// State of a path from its claims and its body.
fn classify(records: &Records, path: &DocPath, body: &Body) -> (State, Option<String>) {
    let claims = records.claims(path);
    if let Body::Unsupported(u) = body {
        return (
            State::Unsupported(*u),
            claims.first().map(|c| c.record.id.clone()),
        );
    }
    if claims.len() > 1 {
        return (State::Conflict, None);
    }
    match (body, claims.first()) {
        (Body::Bytes(_), None) => (State::Unmanaged, None),
        (Body::Bytes(b), Some(c)) => {
            let same =
                c.record.body_bytes == b.len() as u64 && c.record.body_sha256 == sha256_hex(b);
            (
                if same { State::Managed } else { State::Drifted },
                Some(c.record.id.clone()),
            )
        }
        (Body::Absent, Some(c)) => (State::MissingBody, Some(c.record.id.clone())),
        (Body::Absent, None) if !records.complete => {
            (State::Unsupported(Unsupported::PartialCoverage), None)
        }
        _ => (State::Absent, None),
    }
}

/// Byte facts of one body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    /// Length in bytes.
    pub bytes: u64,
    /// Hex sha256.
    pub sha256: String,
    /// Starts with a UTF-8 BOM.
    pub bom: bool,
    /// Bare LF terminators.
    pub lf: usize,
    /// CRLF terminators.
    pub crlf: usize,
    /// CR bytes not followed by LF.
    pub lone_cr: usize,
    /// NUL bytes.
    pub nul: usize,
}

/// One observation of a document: state, bytes, record and a combined version.
#[derive(Clone, Debug)]
pub struct Observation {
    /// Managed path (the record's last path for a retired document).
    pub path: DocPath,
    /// DOC identifier when a record is involved.
    pub id: Option<String>,
    /// Derived state.
    pub state: State,
    /// The describing record (active claimant, or the retired record).
    pub record: Option<RecordEntry>,
    /// Exact body bytes when the file is supported.
    pub body: Option<Vec<u8>>,
    /// Byte facts of the body.
    pub facts: Option<Facts>,
    /// Outline of the body.
    pub outline: Option<Outline>,
    /// Version of the body bytes or of their absence.
    pub body_version: String,
    /// Digest of every record file claiming the path, or of their absence.
    pub record_version: String,
    /// sha256 of the record file bytes, `None` without a record.
    pub record_sha256: Option<String>,
    /// Combined body and record version used for guarded writes and read snapshots.
    pub version: String,
    /// Whether the record inventory was complete when observing.
    pub records_complete: bool,
}

/// Combined version from its two parts.
fn combine(path: &DocPath, body_version: &str, record_version: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        "agent-tasks/doc/v1",
        path.as_str(),
        body_version,
        record_version,
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// Record-version part for a path with no claims.
fn absent_record_version(path: &DocPath) -> String {
    sha256_hex(format!("absent-records:{}", path.as_str()).as_bytes())
}

/// Version of an absent managed path, computable from the path alone; equals the observation
/// version of an absent path whose record inventory is complete.
pub fn absent_version(port: &dyn Port, path: &DocPath) -> String {
    combine(
        path,
        &port.version(path.as_str(), None),
        &absent_record_version(path),
    )
}

/// Facts from an outline-supported body.
fn facts_of(bytes: &[u8], o: &Outline) -> Facts {
    Facts {
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        bom: o.bom,
        lf: o.lf,
        crlf: o.crlf,
        lone_cr: o.lone_cr,
        nul: o.nul,
    }
}

/// Observe a document. Never writes. Errors: `not_found` (unknown DOC identifier), `io`,
/// `file_type`, `invalid_arguments`.
pub fn observe(port: &dyn Port, target: &Ref) -> Result<Observation> {
    let records = load_records(port)?;
    let (path, retired) = match target {
        Ref::Path(p) => (p.clone(), None),
        Ref::Id(id) => {
            let entry = records.by_id(id).ok_or_else(|| {
                Error::new("not_found", "No document record has that identifier.")
            })?;
            let path = DocPath::parse(&entry.record.path)?;
            (
                path,
                (entry.record.state == RecordState::Retired).then(|| entry.clone()),
            )
        }
    };
    if let Some(entry) = retired {
        let body_version = port.version(path.as_str(), None);
        let record_version = port.version(&entry.rel, Some(&entry.bytes));
        let version = combine(&path, &body_version, &record_version);
        return Ok(Observation {
            path,
            id: Some(entry.record.id.clone()),
            state: State::Retired,
            record_sha256: Some(sha256_hex(&entry.bytes)),
            record: Some(entry),
            body: None,
            facts: None,
            outline: None,
            body_version,
            record_version,
            version,
            records_complete: records.complete,
        });
    }
    let mut body = read_body(port, &path)?;
    if matches!(body, Body::Bytes(_)) && !exact_names(port, &path)? {
        // A case-insensitive filesystem answered a differently cased name: never edit that file.
        body = Body::Unsupported(Unsupported::Collision);
    }
    let (state, id) = classify(&records, &path, &body);
    let claims = records.claims(&path);
    let (bytes, body_version) = match &body {
        Body::Bytes(b) => (Some(b.clone()), port.version(path.as_str(), Some(b))),
        _ => (None, port.version(path.as_str(), None)),
    };
    let record_version = if claims.is_empty() {
        absent_record_version(&path)
    } else {
        let mut h = Sha256::new();
        for c in &claims {
            h.update(c.record.id.as_bytes());
            h.update(port.version(&c.rel, Some(&c.bytes)).as_bytes());
        }
        format!("{:x}", h.finalize())
    };
    let outline = bytes.as_deref().map(markdown::outline).transpose()?;
    let facts = bytes
        .as_deref()
        .zip(outline.as_ref())
        .map(|(b, o)| facts_of(b, o));
    let record = (claims.len() == 1).then(|| claims[0].clone());
    let version = combine(&path, &body_version, &record_version);
    Ok(Observation {
        path,
        id,
        state,
        record_sha256: record.as_ref().map(|r| sha256_hex(&r.bytes)),
        record,
        body: bytes,
        facts,
        outline,
        body_version,
        record_version,
        version,
        records_complete: records.complete,
    })
}

// ---------------------------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------------------------

/// Snapshot value bound to the observation, the resolved selection, the encoding and the dialect.
/// It never contains an offset.
fn read_snapshot(version: &str, key: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        "agent-tasks/doc-read/v1",
        version,
        key,
        markdown::ENCODING,
        markdown::DIALECT,
    ] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// One exact page of a document or section.
#[derive(Clone, Debug)]
pub struct ReadPage {
    /// The encoded page.
    pub page: markdown::Page,
    /// The resolved selection.
    pub range: Range<usize>,
    /// Read snapshot to pass back for continuation.
    pub snapshot: String,
    /// Observation version of the document read.
    pub observation_version: String,
    /// State of the document read.
    pub state: State,
}

/// Read one page of the selection. `snapshot` must be supplied whenever `at` is beyond the start of
/// the selection. Errors: `not_found` (no body), `stale`, `ambiguous_section`, `encoding`,
/// `presentation_capacity`, `invalid_arguments`. Never writes.
pub fn read(
    obs: &Observation,
    selector: &Selector,
    at: Option<usize>,
    snapshot: Option<&str>,
    budget: usize,
) -> Result<ReadPage> {
    let (body, outline) = match (&obs.body, &obs.outline) {
        (Some(b), Some(o)) => (b, o),
        _ => {
            return Err(Error::new(
                "not_found",
                "The document has no readable body.",
            ));
        }
    };
    let resolved = markdown::resolve_selection(outline, body.len(), selector)?;
    let current = read_snapshot(&obs.version, &resolved.key);
    if let Some(given) = snapshot
        && given != current
    {
        return Err(Error::new(
            "stale",
            format!(
                "The document or selection changed; start again at zero. Current snapshot version: {current}."
            ),
        ));
    }
    let start = at.unwrap_or(resolved.range.start);
    if start > resolved.range.start && snapshot.is_none() {
        return Err(Error::new(
            "stale",
            "Continuation needs the snapshot version from the preceding page.",
        ));
    }
    let page = markdown::page(body, resolved.range.clone(), start, budget)?;
    Ok(ReadPage {
        page,
        range: resolved.range,
        snapshot: current,
        observation_version: obs.version.clone(),
        state: obs.state,
    })
}

// ---------------------------------------------------------------------------------------------
// Inventory and corpus
// ---------------------------------------------------------------------------------------------

/// One native Markdown file found while walking the namespace.
pub struct Found {
    /// Validated managed path.
    pub path: DocPath,
    /// Body read result.
    body: Body,
}

impl Found {
    /// The exact body bytes when the file is a supported document.
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.body {
            Body::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

/// Everything one walk of the namespace found.
pub struct Walk {
    /// Supported-name files in path order.
    pub files: Vec<Found>,
    /// Named omissions.
    pub gaps: Vec<Gap>,
    /// False when any gap makes counts lower bounds.
    pub complete: bool,
    /// Files whose bytes were read.
    pub files_read: usize,
    /// Bytes read.
    pub bytes_read: u64,
}

/// Whether a native name looks like Markdown (so an unsupported name is a named gap).
fn markdown_like(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".md")
}

/// Walk `README.md` and the `docs/` tree within the caps.
pub fn walk(port: &dyn Port) -> Result<Walk> {
    let mut w = Walk {
        files: Vec::new(),
        gaps: Vec::new(),
        complete: true,
        files_read: 0,
        bytes_read: 0,
    };
    let mut pending: Vec<String> = vec!["README.md".into()];
    let mut dirs = vec![("docs".to_string(), 0usize)];
    while let Some((dir, depth)) = dirs.pop() {
        let listing = port.list(&dir, FILE_CAP * 2)?;
        if !listing.complete {
            w.complete = false;
            w.gaps.push(Gap {
                what: format!("{dir}/"),
                reason: GapReason::Capped,
            });
        }
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        for e in &listing.entries {
            let collides = names
                .iter()
                .any(|o| *o != e.name && o.eq_ignore_ascii_case(&e.name));
            let rel = format!("{dir}/{}", e.name);
            if e.name.starts_with('.') && (store::own_temp_name(&e.name) || !markdown_like(&e.name))
            {
                continue;
            }
            match e.kind {
                EntryKind::Symlink => {
                    w.complete = false;
                    w.gaps.push(Gap {
                        what: quote(&rel),
                        reason: GapReason::Link,
                    });
                }
                EntryKind::Directory => {
                    if depth < DEPTH_CAP && segment_ok(&e.name) && !collides {
                        dirs.push((rel, depth + 1));
                    } else {
                        w.complete = false;
                        let reason = if collides {
                            GapReason::Collision
                        } else {
                            GapReason::Name
                        };
                        w.gaps.push(Gap {
                            what: quote(&rel),
                            reason,
                        });
                    }
                }
                EntryKind::File if markdown_like(&e.name) => {
                    if collides {
                        w.complete = false;
                        w.gaps.push(Gap {
                            what: quote(&rel),
                            reason: GapReason::Collision,
                        });
                    } else {
                        pending.push(rel);
                    }
                }
                EntryKind::Other if markdown_like(&e.name) => {
                    w.complete = false;
                    w.gaps.push(Gap {
                        what: quote(&rel),
                        reason: GapReason::NotRegular,
                    });
                }
                _ => {}
            }
        }
    }
    pending.sort();
    for rel in pending {
        if w.files.len() >= FILE_CAP {
            w.complete = false;
            w.gaps.push(Gap {
                what: "docs/".into(),
                reason: GapReason::Capped,
            });
            break;
        }
        let Ok(path) = DocPath::parse(&rel) else {
            w.complete = false;
            w.gaps.push(Gap {
                what: quote(&rel),
                reason: GapReason::Name,
            });
            continue;
        };
        if w.bytes_read as usize >= SCAN_BYTES_CAP {
            w.complete = false;
            w.gaps.push(Gap {
                what: quote(&rel),
                reason: GapReason::Capped,
            });
            continue;
        }
        let body = read_body(port, &path)?;
        match &body {
            Body::Absent => continue,
            Body::Bytes(b) => {
                w.files_read += 1;
                w.bytes_read += b.len() as u64;
            }
            Body::Unsupported(u) => {
                w.complete = false;
                let reason = match u {
                    Unsupported::NotUtf8 => GapReason::NotUtf8,
                    Unsupported::TooLarge => GapReason::TooLarge,
                    Unsupported::Symlink => GapReason::Link,
                    _ => GapReason::NotRegular,
                };
                w.gaps.push(Gap {
                    what: quote(path.as_str()),
                    reason,
                });
            }
        }
        w.files.push(Found { path, body });
    }
    Ok(w)
}

/// One inventory row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Managed path (the last path for a retired record).
    pub path: String,
    /// DOC identifier when a record is involved.
    pub id: Option<String>,
    /// State.
    pub state: State,
    /// Body length when known.
    pub bytes: Option<u64>,
}

/// Bounded listing of documents and records.
#[derive(Clone, Debug)]
pub struct Inventory {
    /// Rows sorted by path then identifier.
    pub rows: Vec<Row>,
    /// Named omissions.
    pub gaps: Vec<Gap>,
    /// False when counts are lower bounds.
    pub complete: bool,
    /// Files read.
    pub files_read: usize,
    /// Bytes read.
    pub bytes_read: u64,
    /// Digest of every row, gap and file version.
    pub version: String,
}

/// Digest of rows and gaps for continuation.
fn listing_version(
    port: &dyn Port,
    rows: &[Row],
    gaps: &[Gap],
    walk: &Walk,
    records: &Records,
) -> String {
    let mut h = Sha256::new();
    for r in rows {
        h.update(format!("{}|{:?}|{:?}|{:?}\n", r.path, r.id, r.state, r.bytes).as_bytes());
    }
    for g in gaps {
        h.update(format!("gap|{}|{:?}\n", g.what, g.reason).as_bytes());
    }
    for f in &walk.files {
        if let Body::Bytes(b) = &f.body {
            h.update(port.version(f.path.as_str(), Some(b)).as_bytes());
        }
    }
    for e in &records.entries {
        h.update(port.version(&e.rel, Some(&e.bytes)).as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// Rows, gaps and states for one walk and its records.
fn rows_of(walk: &Walk, records: &Records) -> Vec<Row> {
    let mut rows = Vec::new();
    for f in &walk.files {
        let (state, id) = classify(records, &f.path, &f.body);
        let bytes = match &f.body {
            Body::Bytes(b) => Some(b.len() as u64),
            _ => None,
        };
        rows.push(Row {
            path: f.path.as_str().into(),
            id,
            state,
            bytes,
        });
    }
    for e in &records.entries {
        let on_disk = walk.files.iter().any(|f| f.path.as_str() == e.record.path);
        if e.record.state == RecordState::Retired {
            rows.push(Row {
                path: e.record.path.clone(),
                id: Some(e.record.id.clone()),
                state: State::Retired,
                bytes: None,
            });
        } else if !on_disk {
            rows.push(Row {
                path: e.record.path.clone(),
                id: Some(e.record.id.clone()),
                state: State::MissingBody,
                bytes: None,
            });
        }
    }
    rows.sort_by(|a, b| (&a.path, &a.id).cmp(&(&b.path, &b.id)));
    rows
}

/// Bounded sorted enumeration of managed files and records. An absent `docs/` is a complete empty
/// result. Never writes.
pub fn inventory(port: &dyn Port) -> Result<Inventory> {
    let walk = walk(port)?;
    let records = load_records(port)?;
    let rows = rows_of(&walk, &records);
    let mut gaps = walk.gaps.clone();
    gaps.extend(records.gaps.iter().cloned());
    let version = listing_version(port, &rows, &gaps, &walk, &records);
    Ok(Inventory {
        complete: walk.complete && records.complete,
        rows,
        gaps,
        files_read: walk.files_read,
        bytes_read: walk.bytes_read,
        version,
    })
}

/// One searchable document.
#[derive(Clone, Debug)]
pub struct CorpusDoc {
    /// DOC identifier when managed, else the path.
    pub reference: String,
    /// Managed path.
    pub path: DocPath,
    /// DOC identifier when a record exists.
    pub id: Option<String>,
    /// State.
    pub state: State,
    /// Purpose line when a record exists.
    pub purpose: Option<String>,
    /// Headings of the body.
    pub headings: Vec<markdown::Heading>,
    /// Exact UTF-8 body for lexical matching.
    pub text: String,
}

/// Searchable documents with honest coverage.
#[derive(Clone, Debug)]
pub struct Corpus {
    /// Supported documents in path order.
    pub docs: Vec<CorpusDoc>,
    /// Named omissions.
    pub gaps: Vec<Gap>,
    /// False when counts are lower bounds.
    pub complete: bool,
    /// Files read.
    pub files_read: usize,
    /// Bytes read.
    pub bytes_read: u64,
    /// Digest for continuation.
    pub version: String,
}

/// All supported documents within [`SCAN_BYTES_CAP`]; unsupported files become gaps.
pub fn corpus(port: &dyn Port) -> Result<Corpus> {
    let walk = walk(port)?;
    let records = load_records(port)?;
    let mut docs = Vec::new();
    for f in &walk.files {
        let Body::Bytes(b) = &f.body else { continue };
        let (state, id) = classify(&records, &f.path, &f.body);
        let o = markdown::outline(b)?;
        let purpose = id
            .as_deref()
            .and_then(|i| records.by_id(i))
            .map(|e| e.record.purpose.clone());
        docs.push(CorpusDoc {
            reference: id.clone().unwrap_or_else(|| f.path.as_str().into()),
            path: f.path.clone(),
            id,
            state,
            purpose,
            headings: o.headings,
            text: String::from_utf8_lossy(b).into_owned(),
        });
    }
    let rows = rows_of(&walk, &records);
    let mut gaps = walk.gaps.clone();
    gaps.extend(records.gaps.iter().cloned());
    let version = listing_version(port, &rows, &gaps, &walk, &records);
    Ok(Corpus {
        docs,
        complete: walk.complete && records.complete,
        gaps,
        files_read: walk.files_read,
        bytes_read: walk.bytes_read,
        version,
    })
}

// ---------------------------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------------------------

/// Everything a mutation needs from its caller: the storage seam and the optional operation
/// identity. The caller holds the root write lock for the whole call; mutations never lock.
pub struct Scope<'a> {
    /// Storage, allocator and oracle seam.
    pub port: &'a dyn Port,
    /// Deterministic operation identity; `Some` makes every publication required-attested.
    pub operation: Option<&'a OperationId>,
}

impl Scope<'_> {
    /// Whether publications must be attested.
    fn required(&self) -> bool {
        self.operation.is_some()
    }
}

/// How the body changes.
pub enum Edit<'a> {
    /// Replace the whole body with these exact bytes.
    Body(&'a [u8]),
    /// Replace only the body of one section (or the preamble), keeping its heading line.
    Section {
        /// Section to replace.
        selector: &'a Selector,
        /// Exact replacement bytes.
        body: &'a [u8],
    },
}

/// One guarded save request.
pub struct Save<'a> {
    /// Target document.
    pub target: &'a Ref,
    /// One-line purpose; required when a record is created.
    pub purpose: Option<&'a str>,
    /// Body change.
    pub edit: Edit<'a>,
    /// Observation version the caller read.
    pub expected: &'a str,
    /// Declared writer.
    pub actor: Option<&'a str>,
}

/// Non-fatal observations about a mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    /// The result mixes LF and CRLF where the original did not.
    MixedLineEndings,
    /// The document has setext underline lines that are not headings.
    SetextIgnored,
    /// An interrupted move had already completed; nothing was published.
    AlreadyApplied,
}

/// Outcome of one mutation.
#[derive(Debug)]
pub struct Receipt {
    /// DOC identifier after the call.
    pub id: Option<String>,
    /// Managed path after the call.
    pub path: DocPath,
    /// Whether any byte or record changed.
    pub changed: bool,
    /// State before.
    pub state_before: State,
    /// State after.
    pub state_after: State,
    /// Observation version before.
    pub version_before: String,
    /// Observation version after: by DOC identifier when one exists, else by path.
    pub version_after: String,
    /// Record revision after.
    pub revision: Option<u64>,
    /// Body length after.
    pub bytes: u64,
    /// Body sha256 after.
    pub sha256: String,
    /// Typed events this call produced, in order.
    pub publications: Vec<Publication>,
    /// Non-fatal observations.
    pub warnings: Vec<Warning>,
    /// Reference attention.
    pub references: ReferenceCheck,
}

/// Turn a failed step into the contract error class (section 7.1).
fn fail(scope: &Scope<'_>, n0: usize, step: &str, last: bool, hint: &str, e: Error) -> Error {
    let events = scope.port.events();
    if events.len() <= n0 || (last && e.code == "durability_unknown") {
        return e;
    }
    let list: Vec<String> = events[n0..]
        .iter()
        .filter(|ev| ev.kind != EffectKind::DirectoryExisting)
        .take(8)
        .map(|ev| {
            let k = match ev.kind {
                EffectKind::Created => "created",
                EffectKind::Replaced => "replaced",
                EffectKind::Removed => "removed",
                EffectKind::DirectoryCreated | EffectKind::DirectoryExisting => "folder",
            };
            format!("{k} {}", ev.relative)
        })
        .collect();
    Error::new(
        "partial_publication",
        format!(
            "Partially saved. Failed step: {step} ({}). Completed: {}. Nothing was rolled back or retried. Recovery: {hint}",
            e.code,
            list.join("; ")
        ),
    )
}

/// The stale refusal with the current version.
fn stale(current: &str) -> Error {
    Error::new(
        "stale",
        format!("No work saved. Current version: {current}. Read the document before retrying."),
    )
}

/// Check that creating `path` would not collide with a sibling differing only by ASCII case or a
/// file blocking a folder level.
fn check_collision(port: &dyn Port, path: &DocPath) -> Result<()> {
    let parts: Vec<&str> = path.as_str().split('/').collect();
    let mut dir = String::new();
    for (i, part) in parts.iter().enumerate() {
        let listing = port.list(&dir, FILE_CAP * 2)?;
        let last = i + 1 == parts.len();
        for e in &listing.entries {
            if e.name != *part && e.name.eq_ignore_ascii_case(part) {
                return Err(Error::new(
                    "collision",
                    "A sibling differs only by letter case; nothing was saved.",
                ));
            }
            if e.name == *part {
                let wants_dir = !last;
                let is_dir = e.kind == EntryKind::Directory;
                if e.kind == EntryKind::Symlink || (wants_dir && !is_dir) || (!wants_dir && is_dir)
                {
                    return Err(Error::new(
                        "file_type",
                        "A file, folder or link blocks the path; nothing was saved.",
                    ));
                }
            }
        }
        if !dir.is_empty() {
            dir.push('/');
        }
        dir.push_str(part);
    }
    Ok(())
}

/// Updated record bytes for a changed body or purpose.
#[allow(clippy::too_many_arguments)]
fn record_bytes(
    base: Option<&Record>,
    id: &str,
    path: &DocPath,
    purpose: &str,
    origin: Origin,
    body: &[u8],
    actor: Option<&str>,
) -> Result<Vec<u8>> {
    let now = store::now();
    let record = Record {
        schema_version: 1,
        id: id.into(),
        path: path.as_str().into(),
        purpose: purpose.into(),
        origin: base.map_or(origin, |b| b.origin),
        state: RecordState::Active,
        revision: base.map_or(1, |b| b.revision + 1),
        created_at: base.map_or_else(|| now.clone(), |b| b.created_at.clone()),
        updated_at: now,
        retired_at: None,
        actor: actor
            .map(str::to_owned)
            .or_else(|| base.and_then(|b| b.actor.clone())),
        body_sha256: sha256_hex(body),
        body_bytes: body.len() as u64,
    };
    let bytes = store::encode(&record)?;
    if store::decode::<Record>(&bytes).ok().as_ref() != Some(&record) {
        return Err(Error::new(
            "invalid_arguments",
            "purpose: the text cannot be stored faithfully.",
        ));
    }
    Ok(bytes)
}

/// Mixed-line-ending warning when the result mixes LF and CRLF and the original did not.
fn line_warning(before: &Outline, after: &Outline, warnings: &mut Vec<Warning>) {
    let mixed = |o: &Outline| o.lf > 0 && o.crlf > 0;
    if mixed(after) && !mixed(before) {
        warnings.push(Warning::MixedLineEndings);
    }
    if after.setext_candidates > 0 {
        warnings.push(Warning::SetextIgnored);
    }
}

/// Observe by DOC identifier when one exists, else by path.
fn observe_after(port: &dyn Port, id: Option<&str>, path: &DocPath) -> Result<Observation> {
    match id {
        Some(i) => observe(port, &Ref::Id(i.to_owned())),
        None => observe(port, &Ref::Path(path.clone())),
    }
}

/// Refuse states a mutation never edits.
fn refuse_state(state: State) -> Result<()> {
    match state {
        State::Unsupported(Unsupported::Collision) => Err(Error::new(
            "collision",
            "A name that differs only by letter case already exists; nothing was saved.",
        )),
        State::Unsupported(u) => Err(Error::new(
            "unsupported",
            format!("The document is not supported ({u:?}); it is never edited."),
        )),
        State::Conflict => Err(Error::new(
            "conflict",
            "More than one active record claims the path.",
        )),
        State::Retired => Err(Error::new(
            "conflict",
            "The document record is retired; save at its path to create a new document.",
        )),
        _ => Ok(()),
    }
}

/// Save a new or existing document (whole body or one section). Guarded by `req.expected`; creates
/// a record with a fresh DOC identifier for an absent or unmanaged path. Effects run in order:
/// reservation, folders, body, record; the first failure stops with the section 7.1 error class.
pub fn save(scope: &mut Scope<'_>, req: Save<'_>) -> Result<Receipt> {
    let port = scope.port;
    let n0 = port.events().len();
    let obs = observe(port, req.target)?;
    if obs.version != req.expected {
        return Err(stale(&obs.version));
    }
    refuse_state(obs.state)?;
    let creating = matches!(obs.state, State::Absent);
    let new_bytes: Vec<u8> = match &req.edit {
        Edit::Body(b) => b.to_vec(),
        Edit::Section { selector, body } => {
            let (bytes, outline) = match (&obs.body, &obs.outline) {
                (Some(b), Some(o)) => (b, o),
                _ => return Err(Error::new("not_found", "The document has no body to edit.")),
            };
            let resolved = markdown::resolve_selection(outline, bytes.len(), selector)?;
            let target = match (selector, resolved.ordinal) {
                (Selector::Preamble, _) => Selector::Preamble,
                (_, Some(n)) => Selector::Ordinal(n),
                _ => {
                    return Err(Error::new(
                        "invalid_arguments",
                        "section: name a heading or the preamble; the whole document is replaced with a body edit.",
                    ));
                }
            };
            markdown::replace_body(bytes, outline, &target, body)?
        }
    };
    std::str::from_utf8(&new_bytes)
        .map_err(|_| Error::new("encoding", "body: must be valid UTF-8."))?;
    if new_bytes.contains(&0) {
        return Err(Error::new(
            "invalid_arguments",
            "body: must not contain NUL.",
        ));
    }
    if new_bytes.len() > BODY_CAP {
        return Err(Error::new(
            "capacity",
            "The body exceeds 524288 bytes; nothing was saved.",
        ));
    }
    if let Some(p) = req.purpose {
        purpose_ok(p)?;
    }
    let existing = obs.record.as_ref();
    let purpose = match (req.purpose, existing) {
        (Some(p), _) => p.to_owned(),
        (None, Some(r)) => r.record.purpose.clone(),
        (None, None) => {
            return Err(Error::new(
                "invalid_arguments",
                "purpose: required when a document record is created.",
            ));
        }
    };
    let before_outline = obs.outline.clone();
    let after_outline = markdown::outline(&new_bytes)?;
    let body_changed = obs.body.as_deref() != Some(new_bytes.as_slice());
    let record_changed = match existing {
        None => true,
        Some(r) => {
            r.record.body_sha256 != sha256_hex(&new_bytes)
                || r.record.body_bytes != new_bytes.len() as u64
                || r.record.purpose != purpose
        }
    };
    let path = obs.path.clone();
    let mut warnings = Vec::new();
    if let Some(b) = &before_outline {
        line_warning(b, &after_outline, &mut warnings);
    }
    if !body_changed && !record_changed {
        return Ok(Receipt {
            id: obs.id.clone(),
            path,
            changed: false,
            state_before: obs.state,
            state_after: obs.state,
            version_before: obs.version.clone(),
            version_after: obs.version.clone(),
            revision: existing.map(|r| r.record.revision),
            bytes: new_bytes.len() as u64,
            sha256: sha256_hex(&new_bytes),
            publications: Vec::new(),
            warnings,
            references: ReferenceCheck::Skipped,
        });
    }
    if body_changed && obs.body.is_none() {
        check_collision(port, &path)?;
    }
    let overlay_put = [(path.clone(), new_bytes.clone())];
    let references = references::preview(
        port,
        &references::Overlay {
            put: &overlay_put,
            remove: &[],
            moves: &[],
        },
        obs.outline.as_ref(),
        Some(&after_outline),
    );
    let op = scope.operation;
    let required = scope.required();
    let mut id = existing.map(|r| r.record.id.clone());
    if id.is_none() {
        id = Some(port.reserve_id()?);
        if port.events().len() > n0 {
            // The reservation is published; a later failure is partial.
        }
    }
    let id = id.unwrap_or_default();
    let record_rel = record_rel(&id);
    let origin = Origin::Saved;
    let rec_bytes = record_bytes(
        existing.map(|r| &r.record),
        &id,
        &path,
        &purpose,
        origin,
        &new_bytes,
        req.actor,
    )?;
    let hint = if creating {
        "the reserved identifier stays a visible gap; run adopt on the path if a body was written, then save again with a fresh version"
    } else {
        "run adopt on the path to record the current bytes, then save again with a fresh version"
    };
    if body_changed {
        if obs.body.is_none() {
            port.ensure_parents(path.as_str())
                .map_err(|e| fail(scope, n0, "create folders", false, hint, e))?;
            port.put(Put {
                rel: path.as_str(),
                bytes: &new_bytes,
                observed: None,
                cap: BODY_CAP,
                op,
                required,
            })
            .map_err(|e| fail(scope, n0, "publish body", false, hint, e))?;
        } else {
            let old = obs.body.as_deref().unwrap_or_default();
            port.put(Put {
                rel: path.as_str(),
                bytes: &new_bytes,
                observed: Some(old),
                cap: BODY_CAP,
                op,
                required,
            })
            .map_err(|e| fail(scope, n0, "replace body", false, hint, e))?;
        }
    }
    match existing {
        None => {
            port.ensure_parents(&record_rel)
                .map_err(|e| fail(scope, n0, "create record folder", false, hint, e))?;
            port.put(Put {
                rel: &record_rel,
                bytes: &rec_bytes,
                observed: None,
                cap: store::RECORD_CAP,
                op,
                required,
            })
            .map_err(|e| fail(scope, n0, "publish record", true, hint, e))?;
        }
        Some(r) => {
            port.put(Put {
                rel: &record_rel,
                bytes: &rec_bytes,
                observed: Some(&r.bytes),
                cap: store::RECORD_CAP,
                op,
                required,
            })
            .map_err(|e| fail(scope, n0, "replace record", true, hint, e))?;
        }
    }
    let after = observe_after(port, Some(&id), &path)?;
    Ok(Receipt {
        id: Some(id),
        path,
        changed: true,
        state_before: obs.state,
        state_after: after.state,
        version_before: obs.version.clone(),
        version_after: after.version,
        revision: after.record.as_ref().map(|r| r.record.revision),
        bytes: new_bytes.len() as u64,
        sha256: sha256_hex(&new_bytes),
        publications: port.events()[n0..].to_vec(),
        warnings,
        references,
    })
}

/// Record the current bytes of an `Unmanaged` or `Drifted` document without rewriting the body.
/// Creates a record with a fresh identifier for an unmanaged document; a drifted document keeps its
/// identifier and gains a revision. Purpose is required unless a record exists.
pub fn adopt(
    scope: &mut Scope<'_>,
    target: &Ref,
    purpose: Option<&str>,
    expected: &str,
    actor: Option<&str>,
) -> Result<Receipt> {
    let port = scope.port;
    let n0 = port.events().len();
    let obs = observe(port, target)?;
    if obs.version != expected {
        return Err(stale(&obs.version));
    }
    refuse_state(obs.state)?;
    if !matches!(obs.state, State::Unmanaged | State::Drifted) {
        return Err(Error::new(
            "conflict",
            "Only an unmanaged or drifted document can be adopted.",
        ));
    }
    if let Some(p) = purpose {
        purpose_ok(p)?;
    }
    let body = obs.body.clone().unwrap_or_default();
    let existing = obs.record.as_ref();
    let purpose = match (purpose, existing) {
        (Some(p), _) => p.to_owned(),
        (None, Some(r)) => r.record.purpose.clone(),
        (None, None) => {
            return Err(Error::new(
                "invalid_arguments",
                "purpose: required when a document record is created.",
            ));
        }
    };
    let id = match existing {
        Some(r) => r.record.id.clone(),
        None => port.reserve_id()?,
    };
    let rel = record_rel(&id);
    let bytes = record_bytes(
        existing.map(|r| &r.record),
        &id,
        &obs.path,
        &purpose,
        Origin::Adopted,
        &body,
        actor,
    )?;
    let hint = "repeat adopt with a fresh version";
    match existing {
        None => {
            port.ensure_parents(&rel)
                .map_err(|e| fail(scope, n0, "create record folder", false, hint, e))?;
            port.put(Put {
                rel: &rel,
                bytes: &bytes,
                observed: None,
                cap: store::RECORD_CAP,
                op: scope.operation,
                required: scope.required(),
            })
            .map_err(|e| fail(scope, n0, "publish record", true, hint, e))?;
        }
        Some(r) => {
            port.put(Put {
                rel: &rel,
                bytes: &bytes,
                observed: Some(&r.bytes),
                cap: store::RECORD_CAP,
                op: scope.operation,
                required: scope.required(),
            })
            .map_err(|e| fail(scope, n0, "replace record", true, hint, e))?;
        }
    }
    let after = observe_after(port, Some(&id), &obs.path)?;
    Ok(Receipt {
        id: Some(id),
        path: obs.path.clone(),
        changed: true,
        state_before: obs.state,
        state_after: after.state,
        version_before: obs.version.clone(),
        version_after: after.version,
        revision: after.record.as_ref().map(|r| r.record.revision),
        bytes: body.len() as u64,
        sha256: sha256_hex(&body),
        publications: port.events()[n0..].to_vec(),
        warnings: Vec::new(),
        references: ReferenceCheck::Skipped,
    })
}

/// Remove the file and retire its record (when one exists). The identifier and the record stay as
/// historic metadata; the path becomes free. Order: guarded body removal, then record retirement.
pub fn remove(
    scope: &mut Scope<'_>,
    target: &Ref,
    expected: &str,
    actor: Option<&str>,
) -> Result<Receipt> {
    let port = scope.port;
    let n0 = port.events().len();
    let obs = observe(port, target)?;
    if obs.version != expected {
        return Err(stale(&obs.version));
    }
    refuse_state(obs.state)?;
    if obs.state == State::Absent {
        return Err(Error::new(
            "not_found",
            "There is nothing to remove at that path.",
        ));
    }
    let overlay_remove = [obs.path.clone()];
    let references = references::preview(
        port,
        &references::Overlay {
            put: &[],
            remove: &overlay_remove,
            moves: &[],
        },
        obs.outline.as_ref(),
        None,
    );
    let hint = "repeat remove with a fresh version";
    if let Some(body) = &obs.body {
        port.del(Del {
            rel: obs.path.as_str(),
            observed: body,
            cap: BODY_CAP,
            op: scope.operation,
            required: scope.required(),
        })
        .map_err(|e| fail(scope, n0, "remove body", obs.record.is_none(), hint, e))?;
    }
    let mut revision = None;
    if let Some(r) = &obs.record {
        let mut record = r.record.clone();
        record.state = RecordState::Retired;
        record.retired_at = Some(store::now());
        record.updated_at = record.retired_at.clone().unwrap_or_default();
        record.revision += 1;
        if actor.is_some() {
            record.actor = actor.map(str::to_owned);
        }
        revision = Some(record.revision);
        let bytes = store::encode(&record)?;
        port.put(Put {
            rel: &r.rel,
            bytes: &bytes,
            observed: Some(&r.bytes),
            cap: store::RECORD_CAP,
            op: scope.operation,
            required: scope.required(),
        })
        .map_err(|e| fail(scope, n0, "retire record", true, hint, e))?;
    }
    let after = observe_after(port, obs.id.as_deref(), &obs.path)?;
    Ok(Receipt {
        id: obs.id.clone(),
        path: obs.path.clone(),
        changed: true,
        state_before: obs.state,
        state_after: after.state,
        version_before: obs.version.clone(),
        version_after: after.version,
        revision,
        bytes: 0,
        sha256: String::new(),
        publications: port.events()[n0..].to_vec(),
        warnings: Vec::new(),
        references,
    })
}

/// What the caller observed about a move source when it planned the move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveBasis {
    /// Observation version of the source.
    pub version: String,
    /// sha256 of the source body bytes actually present.
    pub body_sha256: String,
    /// sha256 of the record file bytes; `None` when the source has no record.
    pub record_sha256: Option<String>,
    /// DOC identifier when a record exists.
    pub id: Option<String>,
}

impl Observation {
    /// The recorded move observation of this document. Errors: `unsupported` for any state other
    /// than managed, drifted or unmanaged; `not_found` for an absent path.
    pub fn move_basis(&self) -> Result<MoveBasis> {
        match self.state {
            State::Absent => Err(Error::new(
                "not_found",
                "There is nothing to move at that path.",
            )),
            State::Managed | State::Drifted | State::Unmanaged => Ok(MoveBasis {
                version: self.version.clone(),
                body_sha256: self
                    .facts
                    .as_ref()
                    .map(|f| f.sha256.clone())
                    .unwrap_or_default(),
                record_sha256: self.record_sha256.clone(),
                id: self.id.clone(),
            }),
            _ => Err(Error::new(
                "unsupported",
                "Only a managed, drifted or unmanaged document can be moved.",
            )),
        }
    }
}

/// Move a document to an absent path, keeping its DOC identity. Order: S1 create the body at `to`,
/// S2 replace the record (same identifier, new path, revision plus 1), S3 remove the body at `from`.
/// With an operation identity an interrupted move resumes from the storage oracle (section 7.2 of
/// the contract); without one the call is strictly the normal start.
pub fn relocate(
    scope: &mut Scope<'_>,
    from: &Ref,
    to: &DocPath,
    basis: &MoveBasis,
    expected_to: &str,
    actor: Option<&str>,
) -> Result<Receipt> {
    let port = scope.port;
    let n0 = port.events().len();
    let src = observe(port, from)?;
    let dst = observe(port, &Ref::Path(to.clone()))?;
    let normal = src.version == basis.version
        && dst.version == expected_to
        && dst.version == absent_version(port, to);
    let hint = "see the relocation windows in the contract";
    if normal {
        refuse_state(src.state)?;
        let _ = src.move_basis()?;
        if src.path == *to {
            return Err(Error::new(
                "invalid_arguments",
                "to: the destination equals the source.",
            ));
        }
        check_collision(port, to)?;
        let body = src.body.clone().unwrap_or_default();
        let op = scope.operation;
        let required = scope.required();
        port.ensure_parents(to.as_str())
            .map_err(|e| fail(scope, n0, "create folders", false, hint, e))?;
        port.put(Put {
            rel: to.as_str(),
            bytes: &body,
            observed: None,
            cap: BODY_CAP,
            op,
            required,
        })
        .map_err(|e| fail(scope, n0, "create destination body", false, hint, e))?;
        return finish_move(scope, n0, &src, to, &body, actor, true, hint);
    }
    let Some(op) = scope.operation else {
        return Err(stale(if src.version != basis.version {
            &src.version
        } else {
            &dst.version
        }));
    };
    // Identity-bearing resume: only the storage oracle can prove an earlier effect was ours.
    let expect = [ExpectedEffect {
        relative: to.as_str().into(),
        kind: EffectKind::Created,
        after_sha256: Some(basis.body_sha256.clone()),
    }];
    let status = port.status(op, &expect);
    let unproven = |why: &str| {
        Error::new(
            "resume_unproven",
            format!(
                "The interrupted move cannot be proven as this operation's own work ({why}); nothing was changed. Inspect the paths and use ordinary remove and relocate."
            ),
        )
    };
    let attested = match status {
        EffectStatus::Attested(r) | EffectStatus::Partial { attested: r, .. } if r.complete => r,
        _ => return Err(unproven("the operation oracle did not attest it")),
    };
    let mut created = false;
    let mut record_row: Option<&PathEffect> = None;
    let mut removed_from = false;
    for row in &attested.paths {
        match row.kind {
            EffectKind::DirectoryCreated | EffectKind::DirectoryExisting => {}
            EffectKind::Created
                if row.relative == to.as_str()
                    && row.after_sha256.as_deref() == Some(&basis.body_sha256) =>
            {
                created = true
            }
            EffectKind::Replaced
                if row.relative.starts_with("documents/DOC-")
                    && basis.record_sha256.is_some()
                    && row.before_sha256 == basis.record_sha256
                    && basis
                        .id
                        .as_ref()
                        .is_none_or(|i| row.relative == record_rel(i)) =>
            {
                record_row = Some(row)
            }
            EffectKind::Removed if row.before_sha256.as_deref() == Some(&basis.body_sha256) => {
                removed_from = true
            }
            _ => {
                return Err(unproven(
                    "an unexpected effect is attested for this operation",
                ));
            }
        }
    }
    if !created {
        return Err(unproven("the destination copy is not attested"));
    }
    let to_obs_ok =
        dst.body.as_deref().map(sha256_hex).as_deref() == Some(basis.body_sha256.as_str());
    if !to_obs_ok {
        return Err(stale(&dst.version));
    }
    match (record_row, removed_from) {
        (None, false) => {
            // W1: the source must still be exactly what was planned and the destination unclaimed.
            if src.version != basis.version
                || dst.state != State::Unmanaged
                || expected_to != absent_version(port, to)
            {
                return Err(stale(&src.version));
            }
            let body = src.body.clone().unwrap_or_default();
            finish_move(scope, n0, &src, to, &body, actor, false, hint)
        }
        (Some(row), false) => {
            // W2: the record already moved; only the unclaimed leftover at `from` remains.
            let rec = dst
                .record
                .as_ref()
                .ok_or_else(|| unproven("the destination has no moved record"))?;
            let leftover_path = match from {
                Ref::Path(p) => p.clone(),
                Ref::Id(_) => return Err(unproven("name the original path of the source")),
            };
            if row.after_sha256.as_deref() != Some(&sha256_hex(&rec.bytes))
                || rec.record.path != to.as_str()
                || dst.state != State::Managed
            {
                return Err(unproven(
                    "the moved record does not match the attested effect",
                ));
            }
            let leftover = observe(port, &Ref::Path(leftover_path.clone()))?;
            let bytes = leftover
                .body
                .clone()
                .filter(|b| sha256_hex(b) == basis.body_sha256);
            let (Some(bytes), State::Unmanaged) = (bytes, leftover.state) else {
                return Err(stale(&leftover.version));
            };
            port.del(Del {
                rel: leftover_path.as_str(),
                observed: &bytes,
                cap: BODY_CAP,
                op: Some(op),
                required: true,
            })
            .map_err(|e| fail(scope, n0, "remove source body", true, hint, e))?;
            move_receipt(port, n0, &src, &dst, to, true)
        }
        (_, true) => {
            let leftover = match from {
                Ref::Path(p) => observe(port, &Ref::Path(p.clone()))?.state,
                Ref::Id(_) => State::Absent,
            };
            if leftover != State::Absent
                || (basis.record_sha256.is_some() && dst.state != State::Managed)
            {
                return Err(unproven(
                    "the completed move does not match the current files",
                ));
            }
            let mut r = move_receipt(port, n0, &src, &dst, to, false)?;
            r.warnings.push(Warning::AlreadyApplied);
            Ok(r)
        }
    }
}

/// Steps S2 and S3 of a move, after the destination body exists.
#[allow(clippy::too_many_arguments)]
fn finish_move(
    scope: &mut Scope<'_>,
    n0: usize,
    src: &Observation,
    to: &DocPath,
    body: &[u8],
    actor: Option<&str>,
    first_run: bool,
    hint: &str,
) -> Result<Receipt> {
    let port = scope.port;
    let (op, required) = (scope.operation, scope.required());
    let mut record_done = false;
    if let Some(r) = &src.record {
        let mut record = r.record.clone();
        record.path = to.as_str().into();
        record.revision += 1;
        record.updated_at = store::now();
        if actor.is_some() {
            record.actor = actor.map(str::to_owned);
        }
        let bytes = store::encode(&record)?;
        port.put(Put {
            rel: &r.rel,
            bytes: &bytes,
            observed: Some(&r.bytes),
            cap: store::RECORD_CAP,
            op,
            required,
        })
        .map_err(|e| fail(scope, n0, "replace record", false, hint, e))?;
        record_done = true;
    }
    let _ = (first_run, record_done);
    let from_body = src.body.clone().unwrap_or_default();
    debug_assert_eq!(from_body, body);
    port.del(Del {
        rel: src.path.as_str(),
        observed: &from_body,
        cap: BODY_CAP,
        op,
        required,
    })
    .map_err(|e| fail(scope, n0, "remove source body", true, hint, e))?;
    let dst = observe_after(port, src.id.as_deref(), to)?;
    move_receipt(port, n0, src, &dst, to, true)
}

/// Receipt of a move from the final observation.
fn move_receipt(
    port: &dyn Port,
    n0: usize,
    src: &Observation,
    dst: &Observation,
    to: &DocPath,
    changed: bool,
) -> Result<Receipt> {
    let after = observe_after(port, src.id.as_deref().or(dst.id.as_deref()), to)?;
    Ok(Receipt {
        id: after.id.clone(),
        path: to.clone(),
        changed,
        state_before: src.state,
        state_after: after.state,
        version_before: src.version.clone(),
        version_after: after.version.clone(),
        revision: after.record.as_ref().map(|r| r.record.revision),
        bytes: after.facts.as_ref().map_or(0, |f| f.bytes),
        sha256: after
            .facts
            .as_ref()
            .map(|f| f.sha256.clone())
            .unwrap_or_default(),
        publications: port.events()[n0..].to_vec(),
        warnings: Vec::new(),
        references: ReferenceCheck::Skipped,
    })
}

/// Wire form label for acknowledgements and framing lines.
pub fn wire_label(wire: Wire) -> &'static str {
    match wire {
        Wire::Raw => "raw",
        Wire::Escaped => "escaped",
    }
}

/// Group rows by state label, for counts in project context.
pub fn state_counts(rows: &[Row]) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for r in rows {
        *out.entry(r.state.label()).or_insert(0) += 1;
    }
    out
}

/// Scripted storage double and the behavior tests of this module.
#[cfg(test)]
pub(crate) mod tests;
