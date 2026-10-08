//! The one private adapter between the compaction domain and the provider modules.
//!
//! `Env` is intentionally thin and expressed in M-005's own decision terms: it carries the facts the
//! domain decides on (document observations, incoming rows, effect status, committed proofs, the
//! current call's publication events) and the few effects it requests (document operations, attested
//! publications, reservations). It reimplements no Store algorithm, no YAML validator, no metadata
//! serializer and no second effect journal. The live implementation forwards to the provider
//! functions; the test implementation is a faithful substitute and is never composition proof.
use super::record::{MoveBasisRec, SectionAddr, TargetRef};
use crate::store::{Result, Store};

/// State of one document observation, reduced to what compaction decides on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocState {
    /// Supported file with an active DOC record whose bytes match it.
    Managed,
    /// Supported file with no active DOC record; metadata is unknown, never invented.
    Unmanaged,
    /// No file and no active record claims the path.
    Absent,
    /// Observed by identifier: the DOC record is retired.
    Retired,
    /// Any other state (drifted, missing body, conflict, unsupported) with its reason.
    Other(String),
}

/// Facts about the DOC record of a managed document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordFacts {
    /// Relative path of the record file, for example `documents/DOC-001.yaml`.
    pub path: String,
    /// DOC identifier.
    pub id: String,
    /// Document path the record binds.
    pub bound_path: String,
    /// Hex sha256 of the body bytes the record last published or adopted.
    pub body_sha256: String,
    /// Record revision, one at creation.
    pub revision: u64,
    /// Hex sha256 of the exact record file bytes.
    pub sha256: String,
    /// Length of the exact record file bytes.
    pub len: u64,
}

/// One observation of one document path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocFacts {
    /// Observed path.
    pub path: String,
    /// Reduced state.
    pub state: DocState,
    /// Combined body and record observation version, usable as an expected version.
    pub version: String,
    /// Exact body bytes when a file exists.
    pub body: Option<Vec<u8>>,
    /// DOC record facts when a record claims the path or was observed by identifier.
    pub record: Option<RecordFacts>,
    /// Whether the DOC record is retired (observed by identifier).
    pub retired: bool,
}
impl DocFacts {
    /// Hex sha256 of the body bytes, when a file exists.
    pub fn body_sha256(&self) -> Option<String> {
        self.body.as_deref().map(super::record::sha256_hex)
    }
    /// Move basis of this original observation, as the document owner builds it.
    pub fn move_basis(&self) -> MoveBasisRec {
        MoveBasisRec {
            version: self.version.clone(),
            body_sha256: self.body_sha256().unwrap_or_default(),
            record_sha256: self.record.as_ref().map(|r| r.sha256.clone()),
            id: self.record.as_ref().map(|r| r.id.clone()),
        }
    }
}

/// One section of an exact body: its address, exact bytes and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionFact {
    /// Section address in the observed bytes.
    pub addr: SectionAddr,
    /// Hex sha256 of the exact section bytes.
    pub sha256: String,
    /// Exact section bytes (the heading line through the section end; the preamble for the preamble).
    pub bytes: Vec<u8>,
}

/// Outline of one exact body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineFacts {
    /// False when the heading cap or another limit made the outline partial.
    pub complete: bool,
    /// Setext heading candidates, which make fragment addresses unprovable.
    pub setext_candidates: usize,
    /// Preamble (only when nonempty) followed by every heading section in document order.
    pub sections: Vec<SectionFact>,
    /// Slugs of every heading, for fragment checks.
    pub slugs: Vec<String>,
}

/// Kind of a referring source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    /// A Markdown document under the managed namespace.
    Markdown,
    /// The root README.
    Readme,
    /// A work record.
    Work,
    /// A typed knowledge record.
    Knowledge,
    /// A DOC metadata record.
    DocMetadata,
    /// The project manifest.
    Project,
}

/// One incoming reference row.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct IncomingRow {
    /// Target document path the row refers to.
    pub target: String,
    /// Kind of source.
    pub kind: SourceKind,
    /// Source identifier or path.
    pub source: String,
    /// How the reference is written.
    pub via: String,
    /// Occurrences.
    pub count: usize,
    /// Fragments the source names.
    pub fragments: Vec<String>,
}

/// Incoming reference coverage of one target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingFacts {
    /// True only when every home was read, every file loaded and nothing is unparsed.
    pub complete: bool,
    /// Named coverage gaps; any gap makes the answer unknown, never zero.
    pub gaps: Vec<String>,
    /// Sorted rows.
    pub rows: Vec<IncomingRow>,
}

/// The post state overlay handed to the integrity preview.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OverlayFacts {
    /// Paths created or whole-body replaced, with the exact new bytes.
    pub put: Vec<(String, Vec<u8>)>,
    /// Paths removed.
    pub remove: Vec<String>,
    /// Moves, `(from, to)`.
    pub moves: Vec<(String, String)>,
}

/// Result of the integrity preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrityFacts {
    /// False means unknown coverage; destructive application must refuse.
    pub complete: bool,
    /// Named gaps.
    pub gaps: Vec<String>,
    /// References the overlay would introduce as dangling.
    pub introduced: Vec<String>,
}

/// One document operation requested from the document owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocOp {
    /// Whole body save at a path (create or replace).
    Save {
        /// Target path.
        path: String,
        /// Exact new body bytes.
        body: Vec<u8>,
        /// Purpose when a record will be created.
        purpose: Option<String>,
        /// Observation version expected at the path.
        expected: String,
    },
    /// Identity bearing move with the stored original basis.
    Relocate {
        /// Source path.
        from: String,
        /// Absent destination.
        to: String,
        /// Expected source observation version.
        expected: String,
        /// Expected destination observation version.
        expected_to: String,
        /// Basis captured once at proposal time.
        basis: MoveBasisRec,
    },
    /// Remove a document, retiring its record when one exists.
    Remove {
        /// Path to remove.
        path: String,
        /// Observation version expected at the path.
        expected: String,
    },
    /// Adopt an attested body whose record the interrupted call did not publish or update.
    ///
    /// Allowed only for a Create, a Replace of an Unmanaged document, and, by revision 8 of the
    /// contract, the interrupted Replace of a Managed document whose body was attested for the action's
    /// own operation identity; the verified post image (same DOC identifier, base revision plus one,
    /// recorded hash) must hold afterwards. Equal bytes alone are never ownership.
    Adopt {
        /// Path whose current bytes are recorded.
        path: String,
        /// Purpose for the new record.
        purpose: Option<String>,
        /// Observation version expected at the path.
        expected: String,
    },
}

/// Kind of an effect row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKindView {
    /// A file was created.
    Created,
    /// A file was replaced.
    Replaced,
    /// A file was removed.
    Removed,
}

/// Git state of one attested effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitView {
    /// No journal entry; never certifiable.
    Untracked,
    /// Published, recorded, not committed; the intent identity.
    Pending(String),
    /// Held after a partial, failed or unsettled call; the intent identity.
    Held(String),
    /// Committed in a reachable commit.
    Committed(String),
    /// An attempt may or may not have landed.
    Unknown(String),
}

/// One attested effect of an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRow {
    /// Relative path.
    pub relative: String,
    /// Kind.
    pub kind: EffectKindView,
    /// Hex sha256 of the bytes before, when any.
    pub before: Option<String>,
    /// Hex sha256 of the bytes after, when any.
    pub after: Option<String>,
    /// Whether it matched a caller assertion.
    pub asserted: bool,
    /// Digest the effect was superseded into by a later intent, `Some("-")` for a removal.
    pub superseded_into: Option<String>,
    /// Git state.
    pub git: GitView,
}

/// The complete attested set of one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptView {
    /// Every attested effect, asserted or generated.
    pub rows: Vec<EffectRow>,
    /// False when the journal or history could not be fully read.
    pub complete: bool,
}

/// One caller assertion about a computable body effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    /// Relative path.
    pub relative: String,
    /// Kind.
    pub kind: EffectKindView,
    /// Expected after digest, `None` for a removal.
    pub after: Option<String>,
}

/// Answer of the complete operation oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusView {
    /// Every assertion is attested and nothing is unproven.
    Attested(ReceiptView),
    /// Some assertions are attested; the listed paths were never published.
    Partial {
        /// What is attested.
        attested: ReceiptView,
        /// Missing assertion paths.
        missing: Vec<String>,
    },
    /// Nothing attested and nothing equal to the intended result.
    NotPublished,
    /// Unproven rows, equal bytes without attestation or incomplete history.
    Unknown {
        /// Attested part, for display.
        attested: Option<ReceiptView>,
        /// Reason text.
        reason: String,
    },
    /// An assertion path is recorded for this operation with a different kind or digest.
    Foreign {
        /// Reason text.
        reason: String,
    },
}

/// Tracking state of one event of the current call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackingView {
    /// A journal entry exists for the effect.
    Tracked,
    /// The optional write proceeded without an entry.
    Untracked,
    /// A directory, ignored backup, lock or temp effect: not an owned file.
    NotApplicable,
}

/// One publication event of the current call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventView {
    /// Relative path.
    pub relative: String,
    /// Operation identity the event carried.
    pub operation: Option<String>,
    /// Journal intent identity.
    pub intent: Option<String>,
    /// Tracking.
    pub tracking: TrackingView,
    /// False when the parent sync is uncertain.
    pub durable: bool,
}

/// One pending intent reported by the persistence engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingIntent {
    /// Intent identity.
    pub intent: String,
    /// Phase label such as `held`, `published`, `unknown` or `drifted`.
    pub phase: String,
    /// Relative paths of the intent.
    pub paths: Vec<String>,
}

/// One original to prove committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalItem {
    /// Relative path.
    pub relative: String,
    /// Hex sha256 of the exact bytes.
    pub sha256: String,
    /// Length of the bytes.
    pub len: u64,
    /// Commit to prove it in, instead of the latest commit touching the path.
    pub at: Option<String>,
}

/// A proven committed original.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorView {
    /// Relative path.
    pub relative: String,
    /// Commit holding the exact bytes.
    pub commit: String,
    /// Encoded locator, at most 256 bytes.
    pub locator: String,
}

/// The adapter between the compaction domain and its providers.
///
/// All methods run under the caller-held single write lock of the dispatcher and never lock again.
pub trait Env {
    /// The request-local store, for the record, staged blobs and read-only listings.
    fn store(&self) -> &Store;

    /// The single knowledge allocation version observed by every writer.
    fn allocation_version(&self) -> Result<String>;
    /// Reserve the next compaction identifier at the expected allocation version.
    fn reserve(&self, expected: &str, effects: &mut Vec<String>) -> Result<String>;
    /// Whether the six home allocator and current DOC identity invariants validate a DOC identifier.
    fn allocation_valid(&self, doc_id: &str) -> Result<bool>;
    /// Create the missing owned parent directories of a path, at most four levels, each disclosed.
    fn ensure_parents(&self, relative: &str, effects: &mut Vec<String>) -> Result<()>;

    /// Observe one document path.
    fn observe(&self, path: &str) -> Result<DocFacts>;
    /// Observe a document by DOC identifier, including a retired record.
    fn observe_id(&self, id: &str) -> Result<DocFacts>;
    /// Outline exact bytes (pure).
    fn outline(&self, bytes: &[u8]) -> Result<OutlineFacts>;
    /// Incoming reference coverage of one target document.
    fn incoming(&self, path: &str) -> Result<IncomingFacts>;
    /// Integrity preview of an overlay.
    fn integrity(&self, overlay: &OverlayFacts) -> Result<IntegrityFacts>;
    /// Execute one document operation in its own scope with the given operation identity.
    fn doc_op(
        &self,
        operation: &str,
        actor: &str,
        op: &DocOp,
        effects: &mut Vec<String>,
    ) -> Result<()>;

    /// Publish bytes with required attestation and a deterministic operation identity.
    ///
    /// `observed` `None` creates without clobber; `Some` replaces only when the current bytes equal it.
    fn publish(
        &self,
        relative: &str,
        bytes: &[u8],
        observed: Option<&[u8]>,
        operation: &str,
        effects: &mut Vec<String>,
    ) -> Result<()>;
    /// The complete operation oracle for one operation.
    fn effect_status(&self, operation: &str, expected: &[Expected]) -> StatusView;
    /// Prove every item exists byte identical in a reachable commit; `not_committed` names the missing.
    fn verify_committed(&self, items: &[OriginalItem]) -> Result<Vec<LocatorView>>;
    /// Pending intents of the persistence engine, at most the first sixteen the engine lists.
    fn pending(&self) -> Vec<PendingIntent>;
    /// Why [`Env::pending`] cannot be trusted to show every pending intent with its paths, or `None`
    /// when it can.
    ///
    /// The list is bounded and an intent whose evidence is unreadable shows no paths, so a barrier that
    /// reads it alone could miss a held or unknown intent on a path the proposal needs. A caller must
    /// refuse, never proceed, while this names a gap. The default is for substitutes that list everything.
    fn pending_gap(&self) -> Option<String> {
        None
    }
    /// Every publication event the current request has produced so far.
    fn call_events(&self) -> Vec<EventView>;
}

/// Convenience: a target reference for a whole document.
pub fn whole(path: &str) -> TargetRef {
    TargetRef {
        path: path.to_owned(),
        section: None,
    }
}
