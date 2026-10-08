//! Read-only views of compaction records for context and status.
//!
//! Reads take no write lock, never repair, never publish and never commit. Unreadable records are named
//! rather than dropped.
use super::{
    inventory,
    record::{self, Action, ActionKind, ActionState, BLOB_CAP, CpRecord, CpState, RevisionRecord},
};
use crate::store::{Error, Result, Snapshot, Store};

/// Maximum rows one summary returns.
pub const SUMMARY_ROWS: usize = 20;

/// One bounded row of the proposal summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpSummaryRow {
    /// Proposal identifier.
    pub id: String,
    /// Lifecycle state.
    pub state: CpState,
    /// Current revision number.
    pub revision: u32,
    /// Pinned reviewer, when one is set.
    pub reviewer: Option<String>,
    /// Actions recorded applied.
    pub applied: usize,
    /// Actions of the current revision.
    pub total: usize,
    /// Stop kind when blocked.
    pub blocked: Option<String>,
}

/// Bounded summary of every proposal with honest coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpSummaries {
    /// At most [`SUMMARY_ROWS`] rows, live proposals first by number.
    pub rows: Vec<CpSummaryRow>,
    /// Total readable proposals.
    pub total: usize,
    /// Readable proposals not shown.
    pub omitted: usize,
    /// Identifiers with a staged directory but no record.
    pub orphans: Vec<String>,
    /// Named unreadable records.
    pub unreadable: Vec<String>,
    /// False when any entry was unreadable, foreign or capped.
    pub complete: bool,
}

/// Read one proposal by identifier: exact record, bytes and version.
///
/// Codes: `invalid_arguments` for a noncanonical identifier, `cp_not_found`, `invalid_data` for a
/// record that cannot be decoded.
pub fn read_cp(store: &Store, id: &str) -> Result<Snapshot<CpRecord>> {
    record::parse_cp_id(id).map_err(|_| {
        Error::new(
            "invalid_arguments",
            "cp: expected a canonical CP identifier",
        )
    })?;
    let relative = record::record_path(id);
    let bytes = store.bytes(&relative)?.ok_or_else(|| {
        Error::new(
            "cp_not_found",
            format!("{id}: no such compaction proposal."),
        )
    })?;
    let value = record::decode_record(&bytes, id)?;
    let version = store.version(&relative, Some(&bytes));
    Ok(Snapshot {
        value,
        bytes,
        version,
    })
}

/// Summarize proposals for project context and status, bounded to twenty rows.
pub fn summaries(store: &Store, limit: usize) -> Result<CpSummaries> {
    let scan = inventory::scan(store)?;
    let mut rows: Vec<CpSummaryRow> = scan
        .records
        .iter()
        .map(|s| {
            let r = &s.value;
            let body = r.revision().ok().map(|x| &x.body);
            CpSummaryRow {
                id: r.id.clone(),
                state: r.state,
                revision: r.current,
                reviewer: r.reviewer.as_ref().map(|b| b.agent_id.clone()),
                applied: body
                    .map(|b| {
                        b.actions
                            .iter()
                            .filter(|a| a.state == ActionState::Applied)
                            .count()
                    })
                    .unwrap_or(0),
                total: body.map(|b| b.actions.len()).unwrap_or(0),
                blocked: r
                    .apply
                    .as_ref()
                    .and_then(|a| a.blocked.as_ref())
                    .map(|b| b.kind.clone()),
            }
        })
        .collect();
    rows.sort_by_key(|r| (r.state.terminal(), r.id.clone()));
    let total = rows.len();
    let cap = limit.clamp(1, SUMMARY_ROWS);
    rows.truncate(cap);
    Ok(CpSummaries {
        omitted: total - rows.len(),
        total,
        rows,
        orphans: scan.orphans,
        unreadable: scan.unreadable,
        complete: scan.complete,
    })
}

/// Return one retained revision of a proposal by its one based number.
///
/// Pure selection over the already decoded record: nothing is read, locked, written or repaired, and
/// no other revision is ever substituted.
///
/// Codes: `invalid_arguments` naming `revision` when the number is `0` or is not retained; the message
/// states the retained range `1 to <current>`.
pub fn retained_revision(record: &CpRecord, revision: u32) -> Result<&RevisionRecord> {
    record
        .revisions
        .iter()
        .find(|r| r.revision == revision && revision != 0)
        .ok_or_else(|| {
            Error::new(
                "invalid_arguments",
                format!(
                    "revision: expected a retained revision, 1 to {}.",
                    record.current
                ),
            )
        })
}

/// The exact, hash and length verified staged candidate of one action of one retained revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedCandidate {
    /// One based revision number the candidate belongs to.
    pub revision: u32,
    /// Canonical action identifier, for example `A-01`.
    pub action: String,
    /// Kind of the action: `Create` or `Replace`, the only kinds that stage a candidate.
    pub kind: ActionKind,
    /// Target document path of the action.
    pub path: String,
    /// Recorded sha256 digest, verified against `bytes`.
    pub sha256: String,
    /// Exact staged bytes; their length equals the recorded staged length.
    pub bytes: Vec<u8>,
}

/// Read and verify the staged candidate of one action against its recorded hash and length.
///
/// This is the single path and integrity rule for staged blobs: both [`read_staged`] and the apply and
/// review preparation (`ops::load_blobs`) call it. It reads exactly
/// [`record::stage_path`], takes no lock and never falls back to a live document or another revision.
///
/// Returns `Ok(None)` when the action stages no candidate (a Move or Remove).
///
/// Codes: `invalid_data` with the relative path for a missing, oversize, wrongly sized or hash
/// mismatching file; store failures propagate unchanged.
pub(crate) fn verified_blob(
    store: &Store,
    id: &str,
    revision: u32,
    action: &Action,
) -> Result<Option<Vec<u8>>> {
    let Some(expected) = &action.staged_sha256 else {
        return Ok(None);
    };
    let relative = record::stage_path(id, revision, &action.id);
    let mismatch = || {
        Error::new(
            "invalid_data",
            format!("{relative}: staged candidate does not match its recorded hash."),
        )
    };
    let bytes = match store.bytes(&relative) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return Err(Error::new(
                "invalid_data",
                format!("{relative}: staged candidate is missing."),
            ));
        }
        Err(e) if e.code == "capacity" => return Err(mismatch()),
        Err(e) => return Err(e),
    };
    if bytes.len() > BLOB_CAP
        || action.staged_len != Some(bytes.len() as u64)
        || &record::sha256_hex(&bytes) != expected
    {
        return Err(mismatch());
    }
    Ok(Some(bytes))
}

/// Read the exact verified staged candidate of one action of one retained revision.
///
/// Read only: no lock, no write, no repair and no fall back to the live document or to the current
/// revision. Revisions are append only and staged directories never reused, so one
/// `(record, revision, action)` always yields the same bytes.
///
/// Codes: `invalid_arguments` naming `revision` (see [`retained_revision`]) or `action` (not a
/// canonical `A-01` to `A-32` identifier, absent from that revision, or a Move or Remove that stages
/// nothing); `invalid_data` for a missing, wrongly sized or hash mismatching blob.
pub fn read_staged(
    store: &Store,
    record: &CpRecord,
    revision: u32,
    action: &str,
) -> Result<StagedCandidate> {
    let retained = retained_revision(record, revision)?;
    let found = record::parse_action_id(action)
        .then(|| retained.body.actions.iter().find(|a| a.id == action))
        .flatten()
        .ok_or_else(|| {
            Error::new(
                "invalid_arguments",
                format!("action: expected an action of revision {revision}."),
            )
        })?;
    let (Some(sha256), Some(bytes)) = (
        found.staged_sha256.clone(),
        verified_blob(store, &record.id, retained.revision, found)?,
    ) else {
        return Err(Error::new(
            "invalid_arguments",
            format!(
                "action: {action} is a {:?} and stages no candidate.",
                found.kind
            ),
        ));
    };
    Ok(StagedCandidate {
        revision: retained.revision,
        action: found.id.clone(),
        kind: found.kind,
        path: found.path.clone(),
        sha256,
        bytes,
    })
}
