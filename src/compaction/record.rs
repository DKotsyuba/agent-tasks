//! The closed compaction record model, its capacities, deterministic names and revision hash.
//!
//! A `CpRecord` is one tracked YAML file. It keeps every full prior proposal revision and every review
//! append-only, so no revision, finding or approval is ever lost; a change that would overflow the
//! record is refused before any publication.
use crate::{
    model::{self, Finding, FindingResolution, Verdict},
    store::{self, CLOSING_RESERVE, Error, RECORD_CAP, Result},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Understood record schema revision; unfamiliar revisions are never rewritten.
pub const SCHEMA: u32 = 1;
/// Sources one proposal may name.
pub const MAX_SOURCES: usize = 16;
/// Actions one proposal may contain.
pub const MAX_ACTIONS: usize = 32;
/// Section ledger entries one proposal may contain.
pub const MAX_SECTIONS: usize = 256;
/// Preservation items one proposal may contain.
pub const MAX_PRESERVATION: usize = 64;
/// Revisions one proposal record may retain; further change needs a new proposal.
pub const MAX_REVISIONS: u32 = 8;
/// Reviews one record may retain.
pub const MAX_REVIEWS: usize = 16;
/// Findings, in total, one record may retain.
pub const MAX_FINDINGS: usize = 64;
/// Largest staged candidate blob (the managed Markdown body cap of the document owner).
pub const BLOB_CAP: usize = 524_288;
/// Total staged bytes one revision may carry.
pub const REVISION_BLOB_CAP: usize = 2 * 1024 * 1024;
/// Non-terminal proposals one project may hold.
pub const MAX_ACTIVE: usize = 32;
/// Proposals one project may hold in total.
pub const MAX_TOTAL: usize = 256;
/// Largest title, in bytes.
pub const TITLE_CAP: usize = 128;
/// Largest statement, in bytes.
pub const STATEMENT_CAP: usize = 256;
/// Largest purpose text handed to the document owner, in bytes.
pub const PURPOSE_CAP: usize = 240;
/// Largest reason or summary text, in bytes.
pub const REASON_CAP: usize = 512;
/// Largest managed document path, in bytes.
pub const PATH_CAP: usize = 160;
/// Home directory of every compaction record.
pub const HOME: &str = "compactions";

/// Lifecycle state of one compaction proposal.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CpState {
    /// A revision awaits independent review.
    Proposed,
    /// The pinned reviewer requested changes to the current revision.
    ChangesRequested,
    /// The current revision hash is accepted and may be applied.
    Accepted,
    /// Application started; the proposal is frozen and may have published effects.
    Applying,
    /// Application stopped on a named condition and can resume after it is resolved.
    Blocked,
    /// Every action is applied; a historical fact, not a claim about the current post state.
    Applied,
    /// Withdrawn before any effect was published.
    Withdrawn,
    /// Abandoned after effects were published; applied actions are listed as drift.
    AbandonedPartial,
}
impl CpState {
    /// Terminal states hold no path claims and accept no further operation.
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Applied | Self::Withdrawn | Self::AbandonedPartial
        )
    }
    /// Lowercase label used as the acknowledgement phase.
    pub fn label(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::ChangesRequested => "changes_requested",
            Self::Accepted => "accepted",
            Self::Applying => "applying",
            Self::Blocked => "blocked",
            Self::Applied => "applied",
            Self::Withdrawn => "withdrawn",
            Self::AbandonedPartial => "abandoned_partial",
        }
    }
}

/// Kind of one document action.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Create a new document at an absent path.
    Create,
    /// Replace the whole body of an existing document.
    Replace,
    /// Move a document to an absent destination, preserving its identity.
    Move,
    /// Remove a document; its content must be absorbed elsewhere or dropped with a reason.
    Remove,
}

/// Application state of one action.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    /// Not yet adopted as applied.
    Pending,
    /// Its kind specific post image was verified.
    Applied,
    /// Stopped on a named condition.
    Blocked,
}

/// Address of one section of an exact document body.
///
/// The preamble is its own variant and is never encoded as a heading with ordinal 0; heading ordinals
/// are zero based over real headings of the exact bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum SectionAddr {
    /// Bytes before the first heading.
    Preamble,
    /// One real heading.
    Heading {
        /// Zero based document order ordinal.
        ordinal: usize,
        /// Heading level, 1 to 6.
        level: u8,
        /// One based occurrence among headings with identical text.
        occurrence: usize,
        /// Hex sha256 of the raw heading text.
        text_sha256: String,
    },
}

/// Reference to a document and, optionally, one section of it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct TargetRef {
    /// Managed document path.
    pub path: String,
    /// Section of that document; `None` addresses the whole document.
    pub section: Option<SectionAddr>,
}

/// Kind of reason for dropping a section.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DropKind {
    /// Equivalent content is retained verbatim elsewhere.
    Duplicate,
    /// The content is no longer true or relevant.
    Obsolete,
    /// Another retained section supersedes it.
    Superseded,
}

/// What happens to one original section.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "fate", rename_all = "snake_case")]
pub enum Disposition {
    /// The exact section bytes appear in the named action's candidate.
    Kept {
        /// Action id whose candidate carries the section.
        action: String,
    },
    /// The section moves with the named Move action.
    Moved {
        /// Move action id.
        action: String,
    },
    /// The content is rewritten into the target.
    Merged {
        /// Where the rewritten content lives.
        target: TargetRef,
    },
    /// The content is dropped with a reason.
    Dropped {
        /// Reason class.
        kind: DropKind,
        /// Human reason, at most 512 bytes.
        reason: String,
    },
}

/// Ledger entry accounting for exactly one original section.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SectionEntry {
    /// `S-001` style identifier assigned by the MCP in source and outline order.
    pub id: String,
    /// Source document path.
    pub path: String,
    /// Address of the section in the observed source bytes.
    pub section: SectionAddr,
    /// Hex sha256 of the exact section bytes.
    pub sha256: String,
    /// What happens to the section.
    pub disposition: Disposition,
}

/// Kind of preserved fact.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreserveKind {
    /// A business requirement.
    Requirement,
    /// Durable rationale.
    Rationale,
    /// A link that must survive.
    Link,
    /// A decision.
    Decision,
    /// A constraint.
    Constraint,
}

/// How a preserved fact reaches its target.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreserveMode {
    /// The exact source section bytes appear contiguous in the target; checked mechanically.
    Verbatim,
    /// Rewritten content; accepted only by reviewer verdict.
    Rewritten,
}

/// One claimed preservation of a requirement, rationale or link.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Preservation {
    /// `P-01` style identifier.
    pub id: String,
    /// Kind of fact.
    pub kind: PreserveKind,
    /// Section ledger id of the source section.
    pub source_section: String,
    /// Short statement, at most 256 bytes.
    pub statement: String,
    /// Where the fact now lives.
    pub target: TargetRef,
    /// Verification mode.
    pub mode: PreserveMode,
}

/// Move basis captured once from the original observation (the document owner's recorded basis).
///
/// It is never recomputed from a partial or new state; a resume passes the stored value.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MoveBasisRec {
    /// Combined body and record observation version.
    pub version: String,
    /// Hex sha256 of the original body.
    pub body_sha256: String,
    /// Hex sha256 of the original DOC record, absent for an unmanaged document.
    pub record_sha256: Option<String>,
    /// Original DOC identifier, absent for an unmanaged document.
    pub id: Option<String>,
}

/// Frozen observation of one source document at proposal time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceObs {
    /// Managed document path.
    pub path: String,
    /// DOC identifier of a managed document.
    pub doc_id: Option<String>,
    /// Combined body and record observation version.
    pub version: String,
    /// Hex sha256 of the exact body bytes.
    pub sha256: String,
    /// Body length in bytes.
    pub len: u64,
    /// Whether an active DOC record claims the document.
    pub managed: bool,
    /// Path of the DOC record file, when managed.
    pub record_path: Option<String>,
    /// Hex sha256 of the DOC record bytes, when managed.
    pub record_sha256: Option<String>,
    /// DOC record revision at proposal time, when managed; later revisions are expected to be plus one.
    pub record_revision: Option<u64>,
    /// DOC record length at proposal time, when managed.
    pub record_len: Option<u64>,
    /// Move basis, captured once for every Move source.
    pub move_basis: Option<MoveBasisRec>,
}

/// Copy of one publication event, kept for convenience; the provenance journal stays authoritative.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PubCopy {
    /// Relative path published.
    pub relative: String,
    /// `created`, `replaced` or `removed`.
    pub kind: String,
    /// Version before, when any.
    pub before: Option<String>,
    /// Version after, when any.
    pub after: Option<String>,
    /// Operation identity the event carried.
    pub operation: Option<String>,
    /// Journal intent identity, when tracked.
    pub intent: Option<String>,
}

/// One planned and tracked document action.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Action {
    /// `A-01` style identifier, unique in the proposal.
    pub id: String,
    /// Kind of action.
    pub kind: ActionKind,
    /// Target document path; for a Move the destination.
    pub path: String,
    /// Move source path.
    pub from: Option<String>,
    /// Observation version of the preimage; for Create and Move the absent destination's version.
    pub base_version: Option<String>,
    /// Purpose text, required when a new DOC record will be created.
    pub purpose: Option<String>,
    /// Hex sha256 of the staged candidate, for Create and Replace.
    pub staged_sha256: Option<String>,
    /// Length of the staged candidate, for Create and Replace.
    pub staged_len: Option<u64>,
    /// Why the action is part of the compaction, at most 512 bytes.
    pub reason: String,
    /// Where the content of a removed document now lives.
    pub absorbed_into: Vec<TargetRef>,
    /// Application state.
    pub state: ActionState,
    /// Publication copies recorded when the action was adopted.
    pub publications: Vec<PubCopy>,
}

/// Everything one revision proposes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ProposalBody {
    /// Frozen source observations.
    pub sources: Vec<SourceObs>,
    /// Ordered actions.
    pub actions: Vec<Action>,
    /// Section ledger covering every original section.
    pub sections: Vec<SectionEntry>,
    /// Preservation items.
    pub preservation: Vec<Preservation>,
}

/// One retained proposal revision with its full body.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RevisionRecord {
    /// One based revision number.
    pub revision: u32,
    /// Hash of the revision content, see [`content_hash`].
    pub content_hash: String,
    /// Declared actor that authored the revision.
    pub author: String,
    /// Generated creation time.
    pub at: String,
    /// Title, at most 128 bytes.
    pub title: String,
    /// Full body of this revision, never pruned.
    pub body: ProposalBody,
    /// Staged blob directory, for example `compactions/CP-001/r1`.
    pub staged_dir: String,
}

/// Digests describing the full immutable accepted proposal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    /// Digest of source, base and destination observations.
    pub sources_digest: String,
    /// Digest of incoming reference rows excluding documents touched by the proposal.
    pub incoming_digest: String,
    /// Digest of the full overlay.
    pub overlay_digest: String,
}

/// Immutable review of one revision.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewRecord {
    /// Revision reviewed.
    pub revision: u32,
    /// Revision hash reviewed.
    pub content_hash: String,
    /// Declared reviewer.
    pub reviewer: String,
    /// Verdict.
    pub verdict: Verdict,
    /// Review summary.
    pub summary: String,
    /// Digest of the verified item set, for an acceptance.
    pub items_hash: Option<String>,
    /// Zero based findings of this review.
    pub findings: Vec<Finding>,
    /// Resolutions of earlier findings by zero based review and finding index.
    pub resolved: Vec<FindingResolution>,
    /// Digests stored with an acceptance.
    pub acceptance: Option<Acceptance>,
    /// Generated time.
    pub at: String,
}

/// A reviewer observed lost and unable to continue.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LostReport {
    /// The lost reviewer's declared id.
    pub agent_id: String,
    /// Declared actor that reported the loss.
    pub reported_by: String,
    /// Observation of inability to continue or resume.
    pub observation: String,
    /// Generated time.
    pub at: String,
}

/// Context restoration of a replacement reviewer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Immersion {
    /// Declared replacement id.
    pub agent_id: String,
    /// Understanding of the proposal, at most 512 bytes.
    pub understanding: String,
    /// Sources read.
    pub sources: Vec<String>,
    /// Unfinished items inherited.
    pub unfinished: Vec<String>,
    /// Unresolved gaps; any gap blocks review.
    pub gaps: Vec<String>,
    /// Generated time.
    pub at: String,
}

/// The one persistent independent reviewer of a proposal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewerBinding {
    /// Declared id of the current reviewer.
    pub agent_id: String,
    /// Generated time of the first pin.
    pub pinned_at: String,
    /// Reviewers replaced after observed unrecoverable loss, oldest first.
    pub predecessors: Vec<LostReport>,
    /// Set when the current reviewer was reported lost and no replacement has immersed yet.
    pub lost: Option<LostReport>,
    /// Replacement context restoration, when the current reviewer replaced a predecessor.
    pub immersion: Option<Immersion>,
}

/// Phase of an apply run.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    /// Preflight passed; no document effect yet.
    Started,
    /// Create, Move and Replace actions run.
    Writing,
    /// Verification before removals.
    Verifying,
    /// Remove actions run.
    Removing,
    /// All actions applied.
    Done,
}

/// A committed original proven before any effect, matched to its frozen identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoredOriginal {
    /// Relative path of the proven file (body or DOC record).
    pub relative: String,
    /// Hex sha256 of the exact proven bytes.
    pub sha256: String,
    /// Length of the proven bytes.
    pub len: u64,
    /// Encoded committed locator, at most 256 bytes.
    pub locator: String,
    /// Commit the locator names.
    pub commit: String,
}

/// Why an apply stopped.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BlockedInfo {
    /// Stable error code.
    pub kind: String,
    /// Action that stopped, when one did.
    pub action: Option<String>,
    /// Bounded explanation.
    pub reason: String,
}

/// Progress of an apply run; a convenience view, never the authority on effects.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplyProgress {
    /// Attempts started.
    pub attempts: u32,
    /// Current phase.
    pub phase: ApplyPhase,
    /// Originals proven committed before the first effect.
    pub originals: Vec<StoredOriginal>,
    /// Current stop condition.
    pub blocked: Option<BlockedInfo>,
}

/// One compaction proposal record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CpRecord {
    /// Owned schema revision.
    pub schema_version: u32,
    /// `CP-001` style identifier, never recycled.
    pub id: String,
    /// Caller idempotency key of the proposal.
    pub request_key: String,
    /// Lifecycle state.
    pub state: CpState,
    /// Current revision number, 1 to 8.
    pub current: u32,
    /// Every revision with its full body, oldest first, append only.
    pub revisions: Vec<RevisionRecord>,
    /// The pinned independent reviewer.
    pub reviewer: Option<ReviewerBinding>,
    /// Every review, append only and immutable.
    pub reviews: Vec<ReviewRecord>,
    /// Index of the accepting review; force only while that review's hash equals the current hash.
    pub accepted_review: Option<usize>,
    /// Application progress.
    pub apply: Option<ApplyProgress>,
    /// Record publication counter, part of record operation identities.
    pub writes: u32,
    /// Generated creation time.
    pub created_at: String,
    /// Generated last change time.
    pub updated_at: String,
}

impl CpRecord {
    /// The current revision record.
    ///
    /// Returns an error for a record whose `current` number names no retained revision.
    pub fn revision(&self) -> Result<&RevisionRecord> {
        self.revisions
            .iter()
            .find(|r| r.revision == self.current)
            .ok_or_else(|| store::invalid("Record has no current revision."))
    }
    /// Mutable access to the current revision.
    pub fn revision_mut(&mut self) -> Result<&mut RevisionRecord> {
        let current = self.current;
        self.revisions
            .iter_mut()
            .find(|r| r.revision == current)
            .ok_or_else(|| store::invalid("Record has no current revision."))
    }
    /// Every declared author of any revision, deduplicated.
    pub fn authors(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for r in &self.revisions {
            if !seen.contains(&r.author.as_str()) {
                seen.push(r.author.as_str());
            }
        }
        seen
    }
    /// Whether the accepting review still binds the current revision hash.
    pub fn acceptance_current(&self) -> Option<&ReviewRecord> {
        let hash = &self.revision().ok()?.content_hash;
        let review = self.reviews.get(self.accepted_review?)?;
        (review.verdict == Verdict::Accepted && &review.content_hash == hash).then_some(review)
    }
}

/// Canonical record path, for example `compactions/CP-001.yaml`.
pub fn record_path(id: &str) -> String {
    format!("{HOME}/{id}.yaml")
}
/// Staged blob directory of one revision, for example `compactions/CP-001/r1`.
pub fn stage_dir(id: &str, revision: u32) -> String {
    format!("{HOME}/{id}/r{revision}")
}
/// Staged blob path of one action in one revision.
pub fn stage_path(id: &str, revision: u32, action: &str) -> String {
    format!("{}/{action}.md", stage_dir(id, revision))
}
/// Deterministic operation identity of one document action; never carries an attempt nonce.
pub fn action_operation(id: &str, revision: u32, action: &str) -> String {
    format!("cp:{id}:r{revision}:{action}")
}
/// Deterministic operation identity of one staged blob publication.
pub fn stage_operation(id: &str, revision: u32, action: &str) -> String {
    format!("cp:{id}:r{revision}:stage:{action}")
}
/// Deterministic operation identity of one record publication, from the stored write counter.
pub fn record_operation(id: &str, revision: u32, write: u32) -> String {
    format!("cp:{id}:r{revision}:rec:{write}")
}
/// Validate a canonical `CP-NNN` identifier and return its number.
///
/// This is the single seam over the typed identifier grammar of the knowledge owner; it parses only and
/// never touches the allocator.
pub fn parse_cp_id(id: &str) -> std::result::Result<u64, String> {
    model::number(id, "CP-")
}
/// Validate a canonical action identifier `A-01` to `A-32`.
pub fn parse_action_id(id: &str) -> bool {
    id.len() >= 4
        && id.starts_with("A-")
        && id[2..].bytes().all(|b| b.is_ascii_digit())
        && id[2..]
            .parse::<u32>()
            .is_ok_and(|n| (1..=MAX_ACTIONS as u32).contains(&n))
        && id[2..].len() == 2
}

/// Length-prefixed digest helper shared by every hash of this module.
pub fn digest_parts(domain: &str, parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    format!("{:x}", digest.finalize())
}
pub use crate::persist::journal::sha256_hex;

/// Hash of one revision's content: title, sources, actions, ledger and preservation items.
///
/// State, reviews, authors and times are excluded, so a revise with identical content is unchanged.
/// The encoding is domain separated JSON of the closed structs, whose field order is fixed.
pub fn content_hash(title: &str, body: &ProposalBody) -> Result<String> {
    /// Domain separated hash basis of one revision.
    #[derive(Serialize)]
    struct Basis<'a> {
        title: &'a str,
        sources: Vec<(&'a str, &'a str, &'a str)>,
        actions: Vec<ActionBasis<'a>>,
        sections: &'a [SectionEntry],
        preservation: &'a [Preservation],
    }
    /// Hash basis of one action, excluding its state and publication copies.
    #[derive(Serialize)]
    struct ActionBasis<'a> {
        kind: ActionKind,
        path: &'a str,
        from: &'a Option<String>,
        base_version: &'a Option<String>,
        purpose: &'a Option<String>,
        staged_sha256: &'a Option<String>,
        reason: &'a str,
        absorbed_into: &'a [TargetRef],
    }
    let basis = Basis {
        title,
        sources: body
            .sources
            .iter()
            .map(|s| (s.path.as_str(), s.version.as_str(), s.sha256.as_str()))
            .collect(),
        actions: body
            .actions
            .iter()
            .map(|a| ActionBasis {
                kind: a.kind,
                path: &a.path,
                from: &a.from,
                base_version: &a.base_version,
                purpose: &a.purpose,
                staged_sha256: &a.staged_sha256,
                reason: &a.reason,
                absorbed_into: &a.absorbed_into,
            })
            .collect(),
        sections: &body.sections,
        preservation: &body.preservation,
    };
    let bytes =
        serde_json::to_vec(&basis).map_err(|_| store::invalid("Cannot hash the proposal."))?;
    Ok(digest_parts("agent-tasks/cp/v3", &[&bytes]))
}

/// Encode a record canonically and enforce the capacity rule before any publication.
///
/// A nonterminal write must fit `RECORD_CAP - CLOSING_RESERVE`; terminal writes may use the reserve.
/// Overflow is refused with code `capacity` and leaves the published record untouched.
pub fn encode_record(record: &CpRecord, terminal: bool) -> Result<Vec<u8>> {
    let bytes = store::encode(record)?;
    let cap = if terminal {
        RECORD_CAP
    } else {
        RECORD_CAP - CLOSING_RESERVE
    };
    if bytes.len() > cap {
        return Err(Error::new(
            "capacity",
            "Compaction record capacity reached; every revision and review is retained, so start a new proposal.",
        ));
    }
    Ok(bytes)
}

/// Decode and validate one compaction record, requiring its name to match its content.
pub fn decode_record(bytes: &[u8], id: &str) -> Result<CpRecord> {
    let record: CpRecord = store::decode(bytes)?;
    if record.schema_version != SCHEMA {
        return Err(store::invalid("Unfamiliar compaction record schema."));
    }
    if record.id != id {
        return Err(store::invalid(
            "Compaction record identifier differs from its file name.",
        ));
    }
    record.revision()?;
    Ok(record)
}
