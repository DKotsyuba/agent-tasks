//! Read-only views of compaction records for context and status.
//!
//! Reads take no write lock, never repair, never publish and never commit. Unreadable records are named
//! rather than dropped.
use super::{
    inventory,
    record::{self, ActionState, CpRecord, CpState},
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
