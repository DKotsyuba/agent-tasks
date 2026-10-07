//! Mechanical proposal validation, section and preservation accounting, incoming coverage and the
//! acceptance digests.
//!
//! Everything here is structural: the module proves that every original section is accounted for, that
//! claimed preservations resolve, that incoming references are fully known and resolved and that
//! digests bind exactly the inputs the agreement names. A reviewer owns semantic judgement.
use super::env::{DocFacts, DocState, Env, IncomingRow, OutlineFacts, OverlayFacts, SourceKind};
use super::record::{
    self, Action, ActionKind, ActionState, BLOB_CAP, Disposition, MAX_ACTIONS, MAX_PRESERVATION,
    MAX_SECTIONS, MAX_SOURCES, PURPOSE_CAP, Preservation, PreserveKind, PreserveMode, ProposalBody,
    REASON_CAP, REVISION_BLOB_CAP, STATEMENT_CAP, SectionAddr, SectionEntry, SourceObs, TITLE_CAP,
    TargetRef,
};
use crate::{
    model,
    store::{Error, Result},
};
use std::collections::{BTreeMap, BTreeSet};

/// One source named by the proposer, bound to the observation version the proposer read.
#[derive(Clone, Debug)]
pub struct SourceIn {
    /// Managed document path.
    pub path: String,
    /// Observation version the proposer read; must equal the current observation.
    pub version: String,
}

/// One action as proposed, with its candidate text.
#[derive(Clone, Debug)]
pub struct ActionIn {
    /// `A-01` style identifier.
    pub id: String,
    /// Kind of action.
    pub kind: ActionKind,
    /// Target path; for a Move the destination.
    pub path: String,
    /// Move source path.
    pub from: Option<String>,
    /// Optional caller assertion of the preimage version; must equal the observed one when present.
    pub base_version: Option<String>,
    /// Candidate body text for Create and Replace.
    pub content: Option<String>,
    /// Purpose when a new record will be created.
    pub purpose: Option<String>,
    /// Reason, at most 512 bytes.
    pub reason: String,
    /// Where removed content now lives.
    pub absorbed_into: Vec<TargetRef>,
}

/// One section disposition keyed by source path and section address.
#[derive(Clone, Debug)]
pub struct SectionIn {
    /// Source path.
    pub path: String,
    /// Section address from the source outline.
    pub section: SectionAddr,
    /// Disposition.
    pub disposition: Disposition,
}

/// One claimed preservation keyed by its source section.
#[derive(Clone, Debug)]
pub struct PreservationIn {
    /// `P-01` style identifier.
    pub id: String,
    /// Kind of fact.
    pub kind: PreserveKind,
    /// Source path.
    pub path: String,
    /// Source section.
    pub section: SectionAddr,
    /// Statement, at most 256 bytes.
    pub statement: String,
    /// Target of the preserved fact.
    pub target: TargetRef,
    /// Verification mode.
    pub mode: PreserveMode,
}

/// A complete proposal as supplied by the external actor.
#[derive(Clone, Debug)]
pub struct ProposalIn {
    /// Title, at most 128 bytes.
    pub title: String,
    /// Sources.
    pub sources: Vec<SourceIn>,
    /// Actions.
    pub actions: Vec<ActionIn>,
    /// Section ledger.
    pub sections: Vec<SectionIn>,
    /// Preservation items.
    pub preservation: Vec<PreservationIn>,
}

/// A validated proposal body with the exact candidate bytes to stage.
#[derive(Debug)]
pub struct Built {
    /// Validated body.
    pub body: ProposalBody,
    /// Candidate bytes per action id, for Create and Replace.
    pub blobs: Vec<(String, Vec<u8>)>,
}

/// Build a stable-code refusal.
fn refuse(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(code, message)
}
/// Build an `invalid_arguments` refusal that names the offending field and its rule.
fn named(field: &str, rule: &str) -> Error {
    Error::new("invalid_arguments", format!("{field}: {rule}"))
}

/// Bytes of one section of an exact outline.
fn section_bytes<'a>(outline: &'a OutlineFacts, addr: &SectionAddr) -> Option<&'a [u8]> {
    outline
        .sections
        .iter()
        .find(|s| &s.addr == addr)
        .map(|s| s.bytes.as_slice())
}

/// True when `needle` occurs contiguously in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
}

/// Candidate bytes of the document a target path will hold after the proposal, when it changes.
fn candidate_for<'a>(
    blobs: &'a [(String, Vec<u8>)],
    actions: &[Action],
    path: &str,
) -> Option<&'a [u8]> {
    let action = actions
        .iter()
        .find(|a| a.path == path && matches!(a.kind, ActionKind::Create | ActionKind::Replace))?;
    blobs
        .iter()
        .find(|(id, _)| id == &action.id)
        .map(|(_, b)| b.as_slice())
}

/// Resolve a target to the exact bytes it addresses after the proposal.
///
/// A path changed by the proposal resolves in its candidate; a Move destination resolves in the moved
/// source bytes; any other document resolves in its current observation. A removed document, an
/// unobservable document or a missing section is `preservation_unmapped`.
fn resolve_target(
    env: &dyn Env,
    actions: &[Action],
    blobs: &[(String, Vec<u8>)],
    sources: &BTreeMap<String, DocFacts>,
    target: &TargetRef,
) -> Result<Vec<u8>> {
    let unmapped = |why: &str| refuse("preservation_unmapped", format!("{}: {why}", target.path));
    if actions
        .iter()
        .any(|a| a.kind == ActionKind::Remove && a.path == target.path)
    {
        return Err(unmapped("target document is removed by the proposal"));
    }
    let bytes: Vec<u8> = if let Some(candidate) = candidate_for(blobs, actions, &target.path) {
        candidate.to_vec()
    } else if let Some(moved) = actions
        .iter()
        .find(|a| a.kind == ActionKind::Move && a.path == target.path)
        .and_then(|a| a.from.as_ref())
        .and_then(|from| sources.get(from))
    {
        moved.body.clone().unwrap_or_default()
    } else {
        let facts = match sources.get(&target.path) {
            Some(f) => f.clone(),
            None => env.observe(&target.path)?,
        };
        if !matches!(facts.state, DocState::Managed | DocState::Unmanaged) {
            return Err(unmapped(
                "target document is not a readable managed document",
            ));
        }
        facts.body.unwrap_or_default()
    };
    match &target.section {
        None => Ok(bytes),
        Some(addr) => {
            let outline = env.outline(&bytes)?;
            if !outline.complete || outline.setext_candidates > 0 {
                return Err(unmapped("section addresses of the target cannot be proven"));
            }
            section_bytes(&outline, addr)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| unmapped("section does not exist"))
        }
    }
}

/// Validate a whole proposal against the current observations and build the body to store.
///
/// Observations come from `env`; caller supplied versions must equal them. Nothing is published.
/// Errors carry stable codes: `invalid_arguments` (field named), `capacity`, `source_refused`,
/// `stale_source`, `section_unaccounted`, `section_unknown`, `preservation_unmapped`.
pub fn build(env: &dyn Env, input: &ProposalIn) -> Result<Built> {
    model::text(&input.title, TITLE_CAP).map_err(|e| named("title", &e))?;
    if input.sources.is_empty() || input.sources.len() > MAX_SOURCES {
        return Err(named("sources", "one to 16 sources are required"));
    }
    if input.actions.is_empty() || input.actions.len() > MAX_ACTIONS {
        return Err(named("actions", "one to 32 actions are required"));
    }
    if input.sections.len() > MAX_SECTIONS {
        return Err(refuse(
            "capacity",
            "At most 256 section ledger entries are allowed.",
        ));
    }
    if input.preservation.len() > MAX_PRESERVATION {
        return Err(refuse(
            "capacity",
            "At most 64 preservation items are allowed.",
        ));
    }

    // Sources: exact observations, unique, readable.
    let mut facts: BTreeMap<String, DocFacts> = BTreeMap::new();
    let mut outlines: BTreeMap<String, OutlineFacts> = BTreeMap::new();
    let mut sources = Vec::new();
    for source in &input.sources {
        if facts.contains_key(&source.path) {
            return Err(named("sources", "paths must be unique"));
        }
        let doc = env.observe(&source.path)?;
        if !matches!(doc.state, DocState::Managed | DocState::Unmanaged) {
            return Err(refuse(
                "source_refused",
                format!(
                    "{}: only managed or unmanaged readable documents can be compacted.",
                    source.path
                ),
            ));
        }
        if doc.version != source.version {
            return Err(refuse(
                "stale_source",
                format!("{}: observation changed since it was read.", source.path),
            ));
        }
        let body = doc.body.clone().unwrap_or_default();
        let outline = env.outline(&body)?;
        if !outline.complete || outline.setext_candidates > 0 {
            return Err(refuse(
                "source_refused",
                format!(
                    "{}: section addresses cannot be proven (partial outline or setext headings).",
                    source.path
                ),
            ));
        }
        sources.push(SourceObs {
            path: source.path.clone(),
            doc_id: doc.record.as_ref().map(|r| r.id.clone()),
            version: doc.version.clone(),
            sha256: record::sha256_hex(&body),
            len: body.len() as u64,
            managed: doc.state == DocState::Managed,
            record_path: doc.record.as_ref().map(|r| r.path.clone()),
            record_sha256: doc.record.as_ref().map(|r| r.sha256.clone()),
            record_revision: doc.record.as_ref().map(|r| r.revision),
            record_len: doc.record.as_ref().map(|r| r.len),
            move_basis: None,
        });
        outlines.insert(source.path.clone(), outline);
        facts.insert(source.path.clone(), doc);
    }

    // Actions: shape, one touch per path, staged candidates.
    let mut actions: Vec<Action> = Vec::new();
    let mut blobs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut touched: BTreeSet<String> = BTreeSet::new();
    let mut total = 0usize;
    for a in &input.actions {
        if !record::parse_action_id(&a.id) || actions.iter().any(|x| x.id == a.id) {
            return Err(named("actions", "ids must be unique A-01 to A-32"));
        }
        model::text(&a.reason, REASON_CAP).map_err(|e| named("actions.reason", &e))?;
        let mut touch = vec![a.path.clone()];
        touch.extend(a.from.clone());
        for p in touch {
            if !touched.insert(p) {
                return Err(named("actions", "no two actions may touch one path"));
            }
        }
        let (content, base_version, purpose) = match a.kind {
            ActionKind::Create => {
                let dest = env.observe(&a.path)?;
                if dest.state != DocState::Absent {
                    return Err(refuse(
                        "source_refused",
                        format!("{}: create needs an absent path.", a.path),
                    ));
                }
                if a.from.is_some() || a.content.is_none() {
                    return Err(named("actions", "create takes content and no source"));
                }
                if a.base_version.as_ref().is_some_and(|v| v != &dest.version) {
                    return Err(refuse(
                        "stale_source",
                        format!("{}: destination changed.", a.path),
                    ));
                }
                if a.purpose.is_none() {
                    return Err(named(
                        "actions.purpose",
                        "a new document record needs a purpose",
                    ));
                }
                (a.content.clone(), Some(dest.version), a.purpose.clone())
            }
            ActionKind::Replace | ActionKind::Remove => {
                let doc = facts.get(&a.path).ok_or_else(|| {
                    named("actions", "replace and remove need the path as a source")
                })?;
                if a.from.is_some() {
                    return Err(named("actions", "only a move has a source path"));
                }
                if a.kind == ActionKind::Replace {
                    if a.content.is_none() {
                        return Err(named("actions.content", "replace needs candidate content"));
                    }
                    if doc.state == DocState::Unmanaged && a.purpose.is_none() {
                        return Err(named(
                            "actions.purpose",
                            "an unmanaged document gains a record and needs a purpose",
                        ));
                    }
                } else if a.content.is_some() {
                    return Err(named("actions.content", "remove takes no content"));
                }
                if a.base_version.as_ref().is_some_and(|v| v != &doc.version) {
                    return Err(refuse(
                        "stale_source",
                        format!("{}: observation changed.", a.path),
                    ));
                }
                (
                    a.content.clone(),
                    Some(doc.version.clone()),
                    a.purpose.clone(),
                )
            }
            ActionKind::Move => {
                let from = a
                    .from
                    .clone()
                    .ok_or_else(|| named("actions.from", "move needs a source path"))?;
                if !facts.contains_key(&from) {
                    return Err(named("actions", "move needs its source as a source"));
                }
                if a.content.is_some() {
                    return Err(named("actions.content", "move takes no content"));
                }
                let dest = env.observe(&a.path)?;
                if dest.state != DocState::Absent {
                    return Err(refuse(
                        "source_refused",
                        format!("{}: move needs an absent destination.", a.path),
                    ));
                }
                if a.base_version.as_ref().is_some_and(|v| v != &dest.version) {
                    return Err(refuse(
                        "stale_source",
                        format!("{}: destination changed.", a.path),
                    ));
                }
                (None, Some(dest.version), None)
            }
        };
        if let Some(p) = &purpose {
            model::text(p, PURPOSE_CAP).map_err(|e| named("actions.purpose", &e))?;
        }
        let (staged_sha256, staged_len) = match &content {
            Some(text) => {
                if text.contains('\0') || text.len() > BLOB_CAP {
                    return Err(refuse(
                        "capacity",
                        format!("{}: candidate exceeds the body rules.", a.path),
                    ));
                }
                total += text.len();
                if total > REVISION_BLOB_CAP {
                    return Err(refuse(
                        "capacity",
                        "Staged candidates of one revision exceed 2 MiB.",
                    ));
                }
                let bytes = text.clone().into_bytes();
                let sha = record::sha256_hex(&bytes);
                let len = bytes.len() as u64;
                blobs.push((a.id.clone(), bytes));
                (Some(sha), Some(len))
            }
            None => (None, None),
        };
        actions.push(Action {
            id: a.id.clone(),
            kind: a.kind,
            path: a.path.clone(),
            from: a.from.clone(),
            base_version,
            purpose,
            staged_sha256,
            staged_len,
            reason: a.reason.clone(),
            absorbed_into: a.absorbed_into.clone(),
            state: ActionState::Pending,
            publications: Vec::new(),
        });
    }
    for source in &input.sources {
        let touches = actions
            .iter()
            .filter(|a| a.kind != ActionKind::Create)
            .any(|a| a.path == source.path || a.from.as_deref() == Some(&source.path));
        if !touches {
            return Err(named(
                "sources",
                "every source needs a replace, move or remove action",
            ));
        }
    }
    // Move bases are captured once, from the original observation.
    for s in &mut sources {
        if actions
            .iter()
            .any(|a| a.kind == ActionKind::Move && a.from.as_deref() == Some(&s.path))
            && let Some(doc) = facts.get(&s.path)
        {
            s.move_basis = Some(doc.move_basis());
        }
    }

    // Section ledger: exactly the source outlines, with ids assigned in source then outline order.
    let mut wanted: Vec<(String, SectionAddr, String, String)> = Vec::new();
    for s in &sources {
        if let Some(o) = outlines.get(&s.path) {
            for f in &o.sections {
                wanted.push((
                    s.path.clone(),
                    f.addr.clone(),
                    f.sha256.clone(),
                    String::new(),
                ));
            }
        }
    }
    let mut by_key: BTreeMap<(String, SectionAddr), &Disposition> = BTreeMap::new();
    for entry in &input.sections {
        if by_key
            .insert(
                (entry.path.clone(), entry.section.clone()),
                &entry.disposition,
            )
            .is_some()
        {
            return Err(named("sections", "one disposition per section"));
        }
    }
    let known: BTreeSet<(String, SectionAddr)> = wanted
        .iter()
        .map(|(p, a, _, _)| (p.clone(), a.clone()))
        .collect();
    if let Some(extra) = by_key.keys().find(|k| !known.contains(*k)) {
        return Err(refuse(
            "section_unknown",
            format!("{}: no such section in the observed bytes.", extra.0),
        ));
    }
    let mut sections = Vec::new();
    for (i, (path, addr, sha, _)) in wanted.iter().enumerate() {
        let disposition = by_key.get(&(path.clone(), addr.clone())).ok_or_else(|| {
            refuse(
                "section_unaccounted",
                format!("{path}: a section has no disposition."),
            )
        })?;
        sections.push(SectionEntry {
            id: format!("S-{:03}", i + 1),
            path: path.clone(),
            section: addr.clone(),
            sha256: sha.clone(),
            disposition: (*disposition).clone(),
        });
    }
    if sections.len() > MAX_SECTIONS {
        return Err(refuse(
            "capacity",
            "At most 256 section ledger entries are allowed.",
        ));
    }

    // Preservation accounting.
    let mut preservation = Vec::new();
    for p in &input.preservation {
        if !p.id.starts_with("P-")
            || p.id.len() != 4
            || preservation.iter().any(|x: &Preservation| x.id == p.id)
        {
            return Err(named("preservation", "ids must be unique P-01 to P-64"));
        }
        model::text(&p.statement, STATEMENT_CAP)
            .map_err(|e| named("preservation.statement", &e))?;
        let source = sections
            .iter()
            .find(|s| s.path == p.path && s.section == p.section)
            .ok_or_else(|| {
                refuse(
                    "preservation_unmapped",
                    format!("{}: preserved section is not a source section.", p.path),
                )
            })?;
        let target_bytes = resolve_target(env, &actions, &blobs, &facts, &p.target)?;
        if p.mode == PreserveMode::Verbatim {
            let src = outlines
                .get(&p.path)
                .and_then(|o| section_bytes(o, &p.section))
                .unwrap_or_default();
            if !contains(&target_bytes, src) {
                return Err(refuse(
                    "preservation_unmapped",
                    format!("{}: verbatim preservation not found in the target.", p.id),
                ));
            }
        }
        preservation.push(Preservation {
            id: p.id.clone(),
            kind: p.kind,
            source_section: source.id.clone(),
            statement: p.statement.clone(),
            target: p.target.clone(),
            mode: p.mode,
        });
    }

    // Disposition checks need the finished actions and preservation.
    let preserved: BTreeSet<&str> = preservation
        .iter()
        .map(|p| p.source_section.as_str())
        .collect();
    for entry in &sections {
        let source_bytes = outlines
            .get(&entry.path)
            .and_then(|o| section_bytes(o, &entry.section))
            .unwrap_or_default();
        match &entry.disposition {
            Disposition::Kept { action } => {
                let a = actions
                    .iter()
                    .find(|a| &a.id == action)
                    .ok_or_else(|| named("sections", "kept names an unknown action"))?;
                let bytes = blobs
                    .iter()
                    .find(|(id, _)| id == &a.id)
                    .map(|(_, b)| b.as_slice())
                    .ok_or_else(|| {
                        named("sections", "kept needs an action with candidate content")
                    })?;
                if !contains(bytes, source_bytes) {
                    return Err(refuse(
                        "preservation_unmapped",
                        format!("{}: kept bytes are not in the candidate.", entry.id),
                    ));
                }
            }
            Disposition::Moved { action } => {
                let ok = actions.iter().any(|a| {
                    &a.id == action
                        && a.kind == ActionKind::Move
                        && a.from.as_deref() == Some(&entry.path)
                });
                if !ok {
                    return Err(named(
                        "sections",
                        "moved needs the move action of the same source",
                    ));
                }
            }
            Disposition::Merged { target } => {
                resolve_target(env, &actions, &blobs, &facts, target)?;
            }
            Disposition::Dropped { reason, .. } => {
                model::text(reason, REASON_CAP).map_err(|e| named("sections.reason", &e))?;
                if preserved.contains(entry.id.as_str()) {
                    return Err(refuse(
                        "preservation_unmapped",
                        format!(
                            "{}: a section with a preservation item cannot be dropped.",
                            entry.id
                        ),
                    ));
                }
            }
        }
    }
    for a in &actions {
        if a.kind == ActionKind::Remove {
            let all_dropped = sections
                .iter()
                .filter(|s| s.path == a.path)
                .all(|s| matches!(s.disposition, Disposition::Dropped { .. }));
            if a.absorbed_into.is_empty() && !all_dropped {
                return Err(named(
                    "actions.absorbed_into",
                    "a remove with retained content names where it now lives",
                ));
            }
            for t in &a.absorbed_into {
                resolve_target(env, &actions, &blobs, &facts, t)?;
            }
        }
    }

    Ok(Built {
        body: ProposalBody {
            sources,
            actions,
            sections,
            preservation,
        },
        blobs,
    })
}

/// Paths a proposal touches: sources, action paths and move sources.
pub fn touched_paths(body: &ProposalBody) -> BTreeSet<String> {
    let mut paths: BTreeSet<String> = body.sources.iter().map(|s| s.path.clone()).collect();
    for a in &body.actions {
        paths.insert(a.path.clone());
        paths.extend(a.from.clone());
    }
    paths
}

/// Whether a proposal contains any action that needs complete incoming coverage.
pub fn needs_coverage(body: &ProposalBody) -> bool {
    body.actions.iter().any(|a| a.kind != ActionKind::Create)
}

/// The overlay a proposal (or its pending remainder) describes.
///
/// Create and Replace are `put` with the exact candidate bytes, Remove is `remove`, Move is `moves`.
pub fn overlay(
    body: &ProposalBody,
    blobs: &[(String, Vec<u8>)],
    pending_only: bool,
) -> OverlayFacts {
    let mut out = OverlayFacts::default();
    for a in &body.actions {
        if pending_only && a.state == ActionState::Applied {
            continue;
        }
        match a.kind {
            ActionKind::Create | ActionKind::Replace => {
                if let Some((_, bytes)) = blobs.iter().find(|(id, _)| id == &a.id) {
                    out.put.push((a.path.clone(), bytes.clone()));
                }
            }
            ActionKind::Remove => out.remove.push(a.path.clone()),
            ActionKind::Move => {
                if let Some(from) = &a.from {
                    out.moves.push((from.clone(), a.path.clone()));
                }
            }
        }
    }
    out
}

/// Digest of the full overlay, computed from the stored staged hashes without reading bytes.
pub fn overlay_digest(body: &ProposalBody) -> String {
    let mut lines: Vec<String> = Vec::new();
    for a in &body.actions {
        lines.push(match a.kind {
            ActionKind::Create | ActionKind::Replace => {
                format!(
                    "put {} {}",
                    a.path,
                    a.staged_sha256.clone().unwrap_or_default()
                )
            }
            ActionKind::Remove => format!("remove {}", a.path),
            ActionKind::Move => format!("move {} {}", a.from.clone().unwrap_or_default(), a.path),
        });
    }
    lines.sort();
    let parts: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
    record::digest_parts("agent-tasks/cp-overlay/v1", &parts)
}

/// One `(path, version, body digest)` observation line of the sources digest.
pub type SourceLine = (String, String, String);

/// The lines the frozen proposal observed: sources, plus the absent destinations of Create and Move.
pub fn frozen_source_lines(body: &ProposalBody) -> Vec<SourceLine> {
    let mut lines: Vec<SourceLine> = body
        .sources
        .iter()
        .map(|s| (s.path.clone(), s.version.clone(), s.sha256.clone()))
        .collect();
    for a in &body.actions {
        if matches!(a.kind, ActionKind::Create | ActionKind::Move) {
            lines.push((
                a.path.clone(),
                a.base_version.clone().unwrap_or_default(),
                String::new(),
            ));
        }
    }
    lines.sort();
    lines
}

/// The same lines observed now, for comparison with [`frozen_source_lines`].
pub fn current_source_lines(env: &dyn Env, body: &ProposalBody) -> Result<Vec<SourceLine>> {
    let mut lines: Vec<SourceLine> = Vec::new();
    for s in &body.sources {
        let doc = env.observe(&s.path)?;
        lines.push((
            s.path.clone(),
            doc.version.clone(),
            doc.body_sha256().unwrap_or_default(),
        ));
    }
    for a in &body.actions {
        if matches!(a.kind, ActionKind::Create | ActionKind::Move) {
            lines.push((a.path.clone(), env.observe(&a.path)?.version, String::new()));
        }
    }
    lines.sort();
    Ok(lines)
}

/// Digest of source observation lines.
pub fn sources_digest(lines: &[SourceLine]) -> String {
    let flat: Vec<String> = lines
        .iter()
        .map(|(p, v, s)| format!("{p}\n{v}\n{s}"))
        .collect();
    let parts: Vec<&[u8]> = flat.iter().map(|l| l.as_bytes()).collect();
    record::digest_parts("agent-tasks/cp-sources/v1", &parts)
}

/// Result of an incoming coverage and integrity run.
#[derive(Debug)]
pub struct Coverage {
    /// Digest of the rows excluding documents touched by the proposal.
    pub incoming_digest: String,
    /// Rows considered, for display.
    pub rows: usize,
}

/// Compute incoming coverage and the integrity preview for a proposal's remaining overlay.
///
/// `blobs` are the candidate bytes of every Create and Replace action. For a proposal needing coverage
/// every gap, dangling reference and unresolved record referrer refuses; a create-only proposal needs
/// none. The coverage version and counts are never inputs to the digest.
pub fn coverage(
    env: &dyn Env,
    body: &ProposalBody,
    blobs: &[(String, Vec<u8>)],
    pending_only: bool,
) -> Result<Coverage> {
    let touched = touched_paths(body);
    let touched_ids: BTreeSet<String> = body
        .sources
        .iter()
        .flat_map(|s| s.doc_id.clone().into_iter().chain(s.record_path.clone()))
        .collect();
    let mut targets: Vec<String> = body.sources.iter().map(|s| s.path.clone()).collect();
    targets.extend(
        body.actions
            .iter()
            .filter(|a| a.kind == ActionKind::Move)
            .map(|a| a.path.clone()),
    );
    targets.sort();
    targets.dedup();
    let mut rows: Vec<IncomingRow> = Vec::new();
    let mut gaps: Vec<String> = Vec::new();
    for t in &targets {
        let inc = env.incoming(t)?;
        if !inc.complete {
            gaps.extend(inc.gaps.clone());
            if inc.gaps.is_empty() {
                gaps.push(format!("{t}: coverage is incomplete."));
            }
        }
        rows.extend(inc.rows);
    }
    if needs_coverage(body) && !gaps.is_empty() {
        gaps.sort();
        gaps.dedup();
        return Err(refuse(
            "coverage_incomplete",
            format!("Incoming coverage is unknown: {}", gaps.join("; ")),
        ));
    }
    // Record and project referrers cannot be rewritten: the target must stay and keep named fragments.
    for row in &rows {
        let rewritable = matches!(row.kind, SourceKind::Markdown | SourceKind::Readme);
        let own_doc = touched.contains(&row.source) || touched_ids.contains(&row.source);
        if rewritable || own_doc {
            continue;
        }
        let removed = body.actions.iter().any(|a| {
            (a.kind == ActionKind::Remove && a.path == row.target)
                || (a.kind == ActionKind::Move && a.from.as_deref() == Some(&row.target))
        });
        if removed {
            return Err(refuse(
                "incoming_unresolved",
                format!(
                    "{} is referenced by {} and cannot be moved or removed.",
                    row.target, row.source
                ),
            ));
        }
        if let Some(candidate) = candidate_for(blobs, &body.actions, &row.target) {
            let outline = env.outline(candidate)?;
            if let Some(f) = row.fragments.iter().find(|f| !outline.slugs.contains(f)) {
                return Err(refuse(
                    "incoming_unresolved",
                    format!(
                        "{} no longer has the section {f} that {} names.",
                        row.target, row.source
                    ),
                ));
            }
        }
    }
    if needs_coverage(body) {
        let integrity = env.integrity(&overlay(body, blobs, pending_only))?;
        if !integrity.complete {
            return Err(refuse(
                "coverage_incomplete",
                format!(
                    "Integrity coverage is unknown: {}",
                    integrity.gaps.join("; ")
                ),
            ));
        }
        if !integrity.introduced.is_empty() {
            return Err(refuse(
                "incoming_unresolved",
                format!(
                    "The proposal would leave dangling references: {}",
                    integrity.introduced.join("; ")
                ),
            ));
        }
    }
    let mut digestible: Vec<&IncomingRow> = rows
        .iter()
        .filter(|r| {
            !touched.contains(&r.source)
                && !(r.kind == SourceKind::DocMetadata && touched_ids.contains(&r.source))
        })
        .collect();
    digestible.sort();
    digestible.dedup();
    let lines: Vec<String> = digestible
        .iter()
        .map(|r| {
            format!(
                "{}\n{:?}\n{}\n{}\n{}\n{}",
                r.target,
                r.kind,
                r.source,
                r.via,
                r.count,
                r.fragments.join(",")
            )
        })
        .collect();
    let parts: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
    Ok(Coverage {
        incoming_digest: record::digest_parts("agent-tasks/cp-incoming/v1", &parts),
        rows: rows.len(),
    })
}
