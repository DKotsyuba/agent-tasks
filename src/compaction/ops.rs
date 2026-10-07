//! Proposal lifecycle operations: propose, revise, review, reviewer recovery and withdraw.
//!
//! Every function runs under the caller-held single write lock, never locks, never runs Git and returns
//! a plain [`Outcome`](crate::compaction::ops::Outcome); the producer converts it into the shared acknowledgement. Publication goes only
//! through the adapter, always with a deterministic operation identity.
use super::{
    env::{EffectKindView, Env, Expected, StatusView},
    inventory,
    record::{
        self, Acceptance, CpRecord, CpState, MAX_ACTIVE, MAX_REVISIONS, MAX_TOTAL, REASON_CAP,
        RevisionRecord, SCHEMA,
    },
    review::{self, ReviewIn},
    validate::{self, ProposalIn},
};
use crate::{
    model,
    store::{self, Error, Result, Snapshot},
};
use std::collections::BTreeSet;

/// The result of one compaction operation, independent of presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Proposal identifier.
    pub id: String,
    /// Record version after the call, usable as the next write version.
    pub version: String,
    /// State after the call.
    pub state: CpState,
    /// Current revision number.
    pub revision: u32,
    /// Whether any file was published.
    pub changed: bool,
    /// Identifiers of actions recorded applied.
    pub applied: Vec<String>,
    /// Total actions of the current revision.
    pub total: usize,
    /// Current stop condition, when blocked.
    pub blocked: Option<record::BlockedInfo>,
    /// Plain lines for the acknowledgement notes, at most eight of 200 bytes.
    pub notes: Vec<String>,
    /// Canonical references of affected records, at most 16 of 256 bytes.
    pub refs: Vec<String>,
}

impl Outcome {
    /// Build an outcome from a decoded record and its version.
    pub fn of(record: &CpRecord, version: String, changed: bool) -> Self {
        let body = record.revision().ok().map(|r| &r.body);
        let applied = body
            .map(|b| {
                b.actions
                    .iter()
                    .filter(|a| a.state == record::ActionState::Applied)
                    .map(|a| a.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let total = body.map_or(0, |b| b.actions.len());
        let mut refs = vec![record.id.clone()];
        if let Some(b) = body {
            for s in &b.sources {
                if let Some(id) = &s.doc_id
                    && !refs.contains(id)
                {
                    refs.push(id.clone());
                }
            }
            for p in validate::touched_paths(b) {
                if refs.len() < 16 && p.len() <= 256 {
                    refs.push(p);
                }
            }
        }
        refs.truncate(16);
        let mut notes = vec![format!(
            "{}/{} actions applied.",
            applied_count(record),
            total
        )];
        if let Some(b) = record.apply.as_ref().and_then(|a| a.blocked.as_ref()) {
            notes.push(store::safe(
                &format!("Blocked {}: {}", b.kind, b.reason),
                190,
            ));
        }
        Self {
            id: record.id.clone(),
            version,
            state: record.state,
            revision: record.current,
            changed,
            applied,
            total,
            blocked: record.apply.as_ref().and_then(|a| a.blocked.clone()),
            notes,
            refs,
        }
    }
}

/// Number of actions of the current revision recorded applied.
fn applied_count(record: &CpRecord) -> usize {
    record
        .revision()
        .map(|r| {
            r.body
                .actions
                .iter()
                .filter(|a| a.state == record::ActionState::Applied)
                .count()
        })
        .unwrap_or(0)
}

/// Build a stable-code refusal.
fn refuse(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(code, message)
}
/// Build an `invalid_arguments` refusal that names the offending field and its rule.
fn named(field: &str, rule: &str) -> Error {
    Error::new("invalid_arguments", format!("{field}: {rule}"))
}

/// Require a declared actor; the producer maps a missing actor to the same refusal.
pub fn require_actor(actor: &str) -> Result<()> {
    model::text(actor, 128).map_err(|_| {
        refuse(
            "actor_required",
            "actor: a declared actor identity is required",
        )
    })
}

/// Validate a caller idempotency key.
fn request_key(key: &str) -> Result<()> {
    let ok = (8..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(named(
            "request_key",
            "8 to 64 characters of letters, digits, dot, underscore or dash",
        ))
    }
}

/// One loaded record with its exact bytes and version.
pub(crate) type Loaded = Snapshot<CpRecord>;

/// Load one record by identifier and check the caller's expected version.
///
/// Codes: `invalid_arguments` (identifier), `cp_not_found`, `stale`.
pub(crate) fn load(env: &dyn Env, id: &str, version: &str) -> Result<Loaded> {
    record::parse_cp_id(id).map_err(|_| named("cp", "expected a canonical CP identifier"))?;
    let relative = record::record_path(id);
    let bytes = env.store().bytes(&relative)?.ok_or_else(|| {
        refuse(
            "cp_not_found",
            format!("{id}: no such compaction proposal."),
        )
    })?;
    let current = env.store().version(&relative, Some(&bytes));
    if current != version {
        return Err(refuse(
            "stale",
            format!("No work saved. Current version: {current}. Read get_context before retrying."),
        ));
    }
    let value = record::decode_record(&bytes, id)?;
    Ok(Snapshot {
        value,
        bytes,
        version: current,
    })
}

/// Publish a record with a fresh deterministic operation identity and return its new version.
///
/// The write counter increments first so identities never repeat; the capacity rule is enforced before
/// anything is published, and `terminal` writes may use the closing reserve.
pub(crate) fn save_record(
    env: &dyn Env,
    rec: &mut CpRecord,
    observed: Option<&[u8]>,
    terminal: bool,
    effects: &mut Vec<String>,
) -> Result<String> {
    rec.writes += 1;
    rec.updated_at = store::now();
    let bytes = record::encode_record(rec, terminal)?;
    let relative = record::record_path(&rec.id);
    let op = record::record_operation(&rec.id, rec.current, rec.writes);
    env.publish(&relative, &bytes, observed, &op, effects)?;
    Ok(env.store().version(&relative, Some(&bytes)))
}

/// Read every candidate blob of the current revision and verify it against its recorded hash.
pub(crate) fn load_blobs(env: &dyn Env, rec: &CpRecord) -> Result<Vec<(String, Vec<u8>)>> {
    let revision = rec.revision()?;
    let mut out = Vec::new();
    for a in &revision.body.actions {
        let Some(expected) = &a.staged_sha256 else {
            continue;
        };
        let relative = record::stage_path(&rec.id, revision.revision, &a.id);
        let bytes = env.store().bytes(&relative)?.ok_or_else(|| {
            refuse(
                "invalid_data",
                format!("{relative}: staged candidate is missing."),
            )
        })?;
        if &record::sha256_hex(&bytes) != expected {
            return Err(refuse(
                "invalid_data",
                format!("{relative}: staged candidate does not match its recorded hash."),
            ));
        }
        out.push((a.id.clone(), bytes));
    }
    Ok(out)
}

/// Publish one staged blob without clobber, adopting an existing equal blob only when the operation
/// oracle attests it for the exact stage operation. Equal bytes alone prove content, never ownership.
fn stage_blob(
    env: &dyn Env,
    id: &str,
    revision: u32,
    action: &str,
    bytes: &[u8],
    effects: &mut Vec<String>,
) -> Result<()> {
    let relative = record::stage_path(id, revision, action);
    let op = record::stage_operation(id, revision, action);
    match env.store().bytes(&relative)? {
        None => {
            env.ensure_parents(&relative, effects)?;
            env.publish(&relative, bytes, None, &op, effects)
        }
        Some(existing) => {
            let sha = record::sha256_hex(bytes);
            let attested = existing == bytes
                && matches!(
                    env.effect_status(
                        &op,
                        &[Expected { relative: relative.clone(), kind: EffectKindView::Created, after: Some(sha) }]
                    ),
                    StatusView::Attested(r) if r.complete
                );
            if attested {
                Ok(())
            } else {
                Err(refuse(
                    "stage_unowned",
                    format!(
                        "{relative}: a staged file exists that this operation does not own; start a new proposal key."
                    ),
                ))
            }
        }
    }
}

/// Reject a proposal whose paths another live proposal already claims.
fn check_claims(
    scan: &inventory::CpScan,
    own: Option<&str>,
    body: &record::ProposalBody,
) -> Result<()> {
    let mine = validate::touched_paths(body);
    for other in &scan.records {
        let r = &other.value;
        if Some(r.id.as_str()) == own || r.state.terminal() {
            continue;
        }
        if let Ok(rev) = r.revision() {
            let theirs = validate::touched_paths(&rev.body);
            if let Some(path) = mine.intersection(&theirs).next() {
                return Err(refuse("path_claimed", format!("{path}: held by {}.", r.id)));
            }
        }
    }
    Ok(())
}

/// Scan the compaction home and refuse unless every entry was read.
fn complete_scan(env: &dyn Env) -> Result<inventory::CpScan> {
    let scan = inventory::scan(env.store())?;
    if !scan.complete {
        return Err(refuse(
            "inventory",
            "The compaction home has unreadable or foreign entries; inspect before writing.",
        ));
    }
    Ok(scan)
}

/// Create one proposal: validate, reserve its number, stage the candidates and publish the record last.
///
/// `version` is the single knowledge allocation version. The same `request_key` with the same content
/// returns the existing proposal unchanged; with different content it refuses `request_key_conflict`.
/// A failure after the reservation leaves a visible, never recycled gap.
pub fn propose(
    env: &dyn Env,
    actor: &str,
    version: &str,
    key: &str,
    input: &ProposalIn,
    effects: &mut Vec<String>,
) -> Result<Outcome> {
    require_actor(actor)?;
    request_key(key)?;
    let scan = complete_scan(env)?;
    if let Some(existing) = scan.records.iter().find(|s| s.value.request_key == key) {
        let same = validate::build(env, input)
            .ok()
            .and_then(|b| record::content_hash(&input.title, &b.body).ok())
            .zip(
                existing
                    .value
                    .revision()
                    .ok()
                    .map(|r| r.content_hash.clone()),
            )
            .is_some_and(|(a, b)| a == b);
        return if same {
            Ok(Outcome::of(
                &existing.value,
                existing.version.clone(),
                false,
            ))
        } else {
            Err(refuse(
                "request_key_conflict",
                "The request key names a proposal with different content.",
            ))
        };
    }
    if scan.records.len() + scan.orphans.len() >= MAX_TOTAL
        || scan
            .records
            .iter()
            .filter(|s| !s.value.state.terminal())
            .count()
            >= MAX_ACTIVE
    {
        return Err(refuse(
            "capacity",
            "The project holds too many compaction proposals.",
        ));
    }
    let current = env.allocation_version()?;
    if current != version {
        return Err(refuse(
            "stale",
            format!("No work saved. Current version: {current}. Read get_context before retrying."),
        ));
    }
    let built = validate::build(env, input)?;
    check_claims(&scan, None, &built.body)?;
    validate::coverage(env, &built.body, &built.blobs, false)?;
    let hash = record::content_hash(&input.title, &built.body)?;
    let id = env.reserve(version, effects)?;
    let at = store::now();
    let mut rec = CpRecord {
        schema_version: SCHEMA,
        id: id.clone(),
        request_key: key.to_owned(),
        state: CpState::Proposed,
        current: 1,
        revisions: vec![RevisionRecord {
            revision: 1,
            content_hash: hash,
            author: actor.to_owned(),
            at: at.clone(),
            title: input.title.clone(),
            body: built.body,
            staged_dir: record::stage_dir(&id, 1),
        }],
        reviewer: None,
        reviews: Vec::new(),
        accepted_review: None,
        apply: None,
        writes: 0,
        created_at: at.clone(),
        updated_at: at,
    };
    // Capacity is proven before any blob or record is published.
    record::encode_record(&rec, false)?;
    env.ensure_parents(&record::record_path(&id), effects)?;
    for (action, bytes) in &built.blobs {
        stage_blob(env, &id, 1, action, bytes, effects)?;
    }
    let version = save_record(env, &mut rec, None, false, effects)?;
    Ok(Outcome::of(&rec, version, true))
}

/// Append a new full revision to a proposal that has not started applying.
///
/// An identical content hash is UNCHANGED. Every previous revision, review and finding is retained; the
/// acceptance pointer clears and the state returns to `Proposed`. A revision that would overflow the
/// record refuses `capacity` before any publication, leaving the published record untouched.
pub fn revise(
    env: &dyn Env,
    actor: &str,
    version: &str,
    cp: &str,
    input: &ProposalIn,
    effects: &mut Vec<String>,
) -> Result<Outcome> {
    require_actor(actor)?;
    let loaded = load(env, cp, version)?;
    let mut rec = loaded.value;
    match rec.state {
        CpState::Applying | CpState::Blocked => {
            return Err(refuse(
                "apply_started",
                "Application started; the proposal is frozen.",
            ));
        }
        CpState::Applied | CpState::AbandonedPartial => {
            return Err(refuse("not_reviewable", "The proposal is finished."));
        }
        CpState::Withdrawn => {
            return Err(refuse("already_withdrawn", "The proposal was withdrawn."));
        }
        _ => {}
    }
    if rec
        .reviewer
        .as_ref()
        .is_some_and(|b| b.agent_id == actor || b.predecessors.iter().any(|p| p.agent_id == actor))
    {
        return Err(refuse(
            "reviewer_cannot_author",
            "The reviewer cannot author a revision.",
        ));
    }
    let built = validate::build(env, input)?;
    let hash = record::content_hash(&input.title, &built.body)?;
    if hash == rec.revision()?.content_hash {
        return Ok(Outcome::of(&rec, loaded.version, false));
    }
    if rec.current >= MAX_REVISIONS {
        return Err(refuse(
            "capacity",
            "The proposal holds eight revisions; start a new proposal.",
        ));
    }
    let scan = complete_scan(env)?;
    check_claims(&scan, Some(&rec.id), &built.body)?;
    validate::coverage(env, &built.body, &built.blobs, false)?;
    let next = rec.current + 1;
    rec.revisions.push(RevisionRecord {
        revision: next,
        content_hash: hash,
        author: actor.to_owned(),
        at: store::now(),
        title: input.title.clone(),
        body: built.body,
        staged_dir: record::stage_dir(&rec.id, next),
    });
    rec.current = next;
    rec.state = CpState::Proposed;
    rec.accepted_review = None;
    // Prove capacity with the final writes counter before publishing any blob.
    let mut probe = rec.clone();
    probe.writes += 1;
    record::encode_record(&probe, false)?;
    for (action, bytes) in &built.blobs {
        stage_blob(env, &rec.id, next, action, bytes, effects)?;
    }
    let version = save_record(env, &mut rec, Some(&loaded.bytes), false, effects)?;
    Ok(Outcome::of(&rec, version, true))
}

/// Record one independent review of the current revision.
///
/// An acceptance stores freshly computed digests of the full accepted proposal: the current source
/// observations must still equal the frozen ones, incoming coverage must be complete and the overlay is
/// bound by its staged hashes.
pub fn review_cp(
    env: &dyn Env,
    actor: &str,
    version: &str,
    cp: &str,
    input: &ReviewIn,
    effects: &mut Vec<String>,
) -> Result<Outcome> {
    require_actor(actor)?;
    let loaded = load(env, cp, version)?;
    let mut rec = loaded.value;
    let items = review::check_review(&rec, actor, input)?;
    let acceptance = if items.is_some() {
        let body = rec.revision()?.body.clone();
        let blobs = load_blobs(env, &rec)?;
        let current = validate::current_source_lines(env, &body)?;
        if current != validate::frozen_source_lines(&body) {
            return Err(refuse(
                "stale_source",
                "A source or destination observation changed since the proposal.",
            ));
        }
        let cov = validate::coverage(env, &body, &blobs, false)?;
        Some(Acceptance {
            sources_digest: validate::sources_digest(&current),
            incoming_digest: cov.incoming_digest,
            overlay_digest: validate::overlay_digest(&body),
        })
    } else {
        None
    };
    review::append_review(&mut rec, actor, input, items, acceptance, &store::now())?;
    let version = save_record(env, &mut rec, Some(&loaded.bytes), false, effects)?;
    Ok(Outcome::of(&rec, version, true))
}

/// Which step of reviewer replacement is being recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStep {
    /// Report the pinned reviewer lost and unable to continue.
    Lost,
    /// Immerse the replacement reviewer.
    Immersed,
}

/// Arguments of a reviewer recovery step.
#[derive(Clone, Debug, Default)]
pub struct RecoveryIn {
    /// `lost` flag of the loss report.
    pub lost: Option<bool>,
    /// `unrecoverable` flag of the loss report.
    pub unrecoverable: Option<bool>,
    /// Observation of the inability to continue.
    pub observation: Option<String>,
    /// Replacement's understanding of the proposal.
    pub understanding: Option<String>,
    /// Sources the replacement read.
    pub sources: Vec<String>,
    /// Unfinished items inherited.
    pub unfinished: Vec<String>,
    /// Unresolved gaps; any gap blocks review.
    pub gaps: Vec<String>,
}

/// Replace the pinned reviewer, only after observed unrecoverable loss and a fresh immersion.
///
/// This mutates review metadata only; predecessors, reviews and findings stay, and an acceptance already
/// bound to the current hash stays valid.
pub fn recover_reviewer(
    env: &dyn Env,
    actor: &str,
    version: &str,
    cp: &str,
    step: RecoveryStep,
    input: &RecoveryIn,
    effects: &mut Vec<String>,
) -> Result<Outcome> {
    require_actor(actor)?;
    let loaded = load(env, cp, version)?;
    let mut rec = loaded.value;
    let at = store::now();
    match step {
        RecoveryStep::Lost => review::report_lost(
            &mut rec,
            actor,
            input.lost,
            input.unrecoverable,
            input.observation.as_deref(),
            &at,
        )?,
        RecoveryStep::Immersed => review::immerse(
            &mut rec,
            actor,
            input.understanding.as_deref(),
            &input.sources,
            &input.unfinished,
            &input.gaps,
            &at,
        )?,
    }
    let version = save_record(env, &mut rec, Some(&loaded.bytes), false, effects)?;
    Ok(Outcome::of(&rec, version, true))
}

/// Withdraw a proposal before application, or abandon a partially applied one explicitly.
///
/// `Applied` is refused. A proposal that started applying needs `abandon_partial` and ends as
/// `AbandonedPartial`, listing no rollback: Git holds the originals. Claims are released either way.
pub fn withdraw(
    env: &dyn Env,
    actor: &str,
    version: &str,
    cp: &str,
    reason: &str,
    abandon_partial: bool,
    effects: &mut Vec<String>,
) -> Result<Outcome> {
    require_actor(actor)?;
    model::text(reason, REASON_CAP).map_err(|e| named("reason", &e))?;
    let loaded = load(env, cp, version)?;
    let mut rec = loaded.value;
    match rec.state {
        CpState::Applied => {
            return Err(refuse(
                "not_reviewable",
                "An applied proposal cannot be withdrawn.",
            ));
        }
        CpState::Withdrawn | CpState::AbandonedPartial => {
            return Err(refuse(
                "already_withdrawn",
                "The proposal is already withdrawn.",
            ));
        }
        CpState::Applying | CpState::Blocked => {
            if !abandon_partial {
                return Err(refuse(
                    "apply_started",
                    "Effects may exist; withdraw with abandon_partial and a reason to record the partial state.",
                ));
            }
            rec.state = CpState::AbandonedPartial;
        }
        _ => rec.state = CpState::Withdrawn,
    }
    rec.accepted_review = None;
    let version = save_record(env, &mut rec, Some(&loaded.bytes), true, effects)?;
    Ok(Outcome::of(&rec, version, true))
}

/// Every proposal that is not terminal, used by status summaries.
pub fn live_ids(scan: &inventory::CpScan) -> BTreeSet<String> {
    scan.records
        .iter()
        .filter(|s| !s.value.state.terminal())
        .map(|s| s.value.id.clone())
        .collect()
}
