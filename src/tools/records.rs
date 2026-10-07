//! Faithful bounded presentation of typed knowledge records, managed Markdown documents and
//! compaction proposals for `get_context`, plus the project-level coverage lines for them.
//!
//! Everything here is a read: no lock beyond the shared read lock the caller holds, no file
//! created, nothing repaired and nothing committed. Domain code is never imported the other way;
//! the domain modules own their models and this file only renders them.
use super::{
    input::{ContextArgs, SearchArgs, SearchKind, StateFilter, View},
    pages::{self, DocumentPage},
    read::{Hit, continuation, hit, page, render_page, scope_version, verdict},
    work::Page,
};
use crate::{
    compaction::{self, ActionState, CpRecord},
    documents::{self, State, StorePort},
    knowledge::{self, Any},
    markdown::{self, Selector},
    persist,
    references::{self, Target},
    response::Templates,
    store::{self, Error, Result, Store},
};

/// Longest single row of record text; longer fields continue on following rows so one row can
/// never exceed the reply budget.
const ROW_BYTES: usize = 1500;

/// Which domain a `get_context` reference belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefKind {
    /// Epic, Module, Atomic and Task references; the existing work routes.
    Work,
    /// Decision, Runbook, Research and procedural Checklist references.
    Knowledge,
    /// A DOC identifier or a managed Markdown path.
    Document,
    /// A compaction proposal reference.
    Compaction,
}

/// Classify a reference once, before any file is read.
pub(super) fn classify(reference: &str) -> RefKind {
    let head = reference.split('/').next().unwrap_or_default();
    let numbered = |prefix: &str| {
        head.strip_prefix(prefix)
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    };
    if numbered("DOC-") || reference == "README.md" || reference.starts_with("docs/") {
        RefKind::Document
    } else if numbered("CP-") {
        RefKind::Compaction
    } else if ["D-", "RB-", "RS-", "CL-"].iter().any(|p| numbered(p)) {
        RefKind::Knowledge
    } else {
        RefKind::Work
    }
}

/// Refuse argument combinations that cannot apply to the referenced kind, naming the field.
///
/// Existing paging parameters keep their meaning; the selector fields apply only to document
/// `view=content`, and a supplied `limit` is refused there because the page size is fixed by the
/// 8192-byte reply budget.
pub(super) fn validate(args: &ContextArgs, kind: RefKind) -> Result<()> {
    let invalid =
        |field: &str, rule: &str| Error::new("invalid_arguments", format!("{field}: {rule}"));
    let allowed = match kind {
        RefKind::Work => matches!(
            args.view,
            View::Summary
                | View::Tasks
                | View::Results
                | View::Checks
                | View::Review
                | View::Log
                | View::Commits
                | View::Integration
        ),
        RefKind::Knowledge => matches!(args.view, View::Summary | View::History | View::References),
        RefKind::Document => matches!(args.view, View::Summary | View::Content | View::References),
        RefKind::Compaction => matches!(
            args.view,
            View::Summary | View::Tasks | View::Review | View::References
        ),
    };
    if !allowed {
        return Err(invalid(
            "view",
            "this view does not apply to the referenced record",
        ));
    }
    let content = kind == RefKind::Document && args.view == View::Content;
    let selectors = [
        ("ordinal", args.ordinal.is_some()),
        ("heading", args.heading.is_some()),
        ("occurrence", args.occurrence.is_some()),
        ("level", args.level.is_some()),
        ("preamble", args.preamble.is_some()),
    ];
    if !content && let Some((field, _)) = selectors.iter().find(|(_, set)| *set) {
        return Err(invalid(field, "applies only to document view=content"));
    }
    if content && args.limit.is_some() {
        return Err(invalid(
            "limit",
            "is not supported for document view=content; the page size is fixed by the reply budget",
        ));
    }
    if args.review_index.is_some() && !matches!(args.view, View::Review) {
        return Err(invalid("review_index", "belongs only to view=review"));
    }
    Ok(())
}

/// Split one field into rows of at most [`ROW_BYTES`], cut at character boundaries.
fn push_field(rows: &mut Vec<String>, label: &str, text: &str) {
    let mut rest = text.trim_end();
    let mut first = true;
    loop {
        let mut end = rest.len().min(ROW_BYTES);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (head, tail) = rest.split_at(end);
        let name = if first {
            label.to_owned()
        } else {
            format!("{label} (continued)")
        };
        rows.push(format!("{name}: {}", head.trim_end()));
        first = false;
        if tail.is_empty() {
            break;
        }
        rest = tail;
    }
}

/// Read one typed knowledge record, a checklist item or a document or proposal and render it.
///
/// Dispatches on the already classified `kind`; `store` is the resolved project and the caller
/// holds its shared read lock.
pub(super) fn context(
    store: &Store,
    args: &ContextArgs,
    kind: RefKind,
    templates: &Templates,
) -> Result<String> {
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| Error::new("invalid_arguments", "Missing reference."))?;
    match kind {
        RefKind::Knowledge => knowledge_context(store, args, reference, templates),
        RefKind::Document => document_context(store, args, reference, templates),
        RefKind::Compaction => compaction_context(store, args, reference, templates),
        RefKind::Work => Err(Error::new("invalid_arguments", "Not a record reference.")),
    }
}

/// Bounded rows rendered as one pinned page of `snapshot_basis`-bound continuation.
fn paged(
    mut value: Page,
    args: &ContextArgs,
    selection: &str,
    basis: &str,
    templates: &Templates,
) -> Result<String> {
    let snapshot = scope_version(selection, &value.version, basis);
    continuation(args.start, args.rows(), args.version.as_deref(), &snapshot)?;
    value.snapshot_version = snapshot;
    render_page(value, args.start, args.rows(), true, templates)
}

// ---------------------------------------------------------------------------------------------
// Typed knowledge
// ---------------------------------------------------------------------------------------------

/// Render a Decision, Runbook, Research record or checklist (or one checklist item).
fn knowledge_context(
    store: &Store,
    args: &ContextArgs,
    reference: &str,
    templates: &Templates,
) -> Result<String> {
    let (id, item) = match reference.split_once('/') {
        Some((id, item)) => (id, Some(item)),
        None => (reference, None),
    };
    let snapshot = knowledge::load(store, id)?;
    let record = &snapshot.value;
    let mut value = page(
        format!("{reference} — {}", store::safe(record.title(), 200)),
        snapshot.version.clone(),
    );
    value.lines.push(format!(
        "Kind: {:?}; state: {}; revision: {}.",
        record.kind(),
        record.state_label(),
        record.revision()
    ));
    if let Some(by) = record.superseded_by() {
        value.lines.push(format!(
            "SUPERSEDED by {by}. This record stays readable but is no longer the current statement; open get_context ref={by}."
        ));
    }
    if matches!(args.view, View::Summary) {
        incoming_attention(store, record.id(), record.current(), &mut value);
    }
    let selection = format!("get_context:{reference}:{:?}", args.view);
    match args.view {
        View::Summary => match item {
            Some(item) => item_rows(record, item, &mut value)?,
            None => record_rows(record, &mut value),
        },
        View::History => history_rows(record, &mut value),
        View::References => reference_rows(
            store,
            Target::Knowledge(id.to_owned()),
            record.references(),
            &mut value,
        ),
        _ => unreachable!("validated before dispatch"),
    }
    value.lines.push(format!(
        "Next: get_context ref={reference} view=history|references; write with knowledge_work using version {}.",
        snapshot.version
    ));
    paged(value, args, &selection, &snapshot.version, templates)
}

/// One bounded attention line about incoming references to this record.
fn incoming_attention(store: &Store, id: &str, current: bool, value: &mut Page) {
    let port = StorePort::new(store);
    match references::incoming(&port, &Target::Knowledge(id.to_owned())) {
        Ok(result) => {
            let sources: usize = result.rows.iter().map(|r| r.count).sum();
            if !result.coverage.complete {
                value.coverage = "PARTIAL".into();
            }
            value.lines.push(format!(
                "Incoming references: {sources} from {} source(s); coverage {}.{}",
                result.rows.len(),
                if result.coverage.complete {
                    "complete"
                } else {
                    "PARTIAL, counts are lower bounds"
                },
                if !current && sources > 0 {
                    " Attention: this superseded record is still referenced; see view=references."
                } else {
                    ""
                }
            ));
        }
        Err(e) => {
            value.coverage = "PARTIAL".into();
            value.lines.push(format!(
                "Incoming references unknown: {}",
                store::safe(&e.message, 160)
            ));
        }
    }
}

/// Current content rows of a record, one field per row, long fields split.
fn record_rows(record: &Any, value: &mut Page) {
    let rows = &mut value.rows;
    match record {
        Any::Decision(r) => {
            let c = &r.content;
            push_field(rows, "Question", &c.question);
            push_field(rows, "Decision", &c.decision);
            push_field(rows, "Rationale", &c.rationale);
            for a in &c.alternatives {
                push_field(
                    rows,
                    "Alternative",
                    &format!("{} — rejected because {}", a.option, a.rejected_because),
                );
            }
            for q in &c.open_questions {
                push_field(
                    rows,
                    if q.needs_owner {
                        "Open question (owner)"
                    } else {
                        "Open question"
                    },
                    &q.text,
                );
            }
            if let Some(detail) = &c.detail {
                push_field(rows, "Detail", detail);
            }
        }
        Any::Research(r) => {
            let c = &r.content;
            push_field(rows, "Question", &c.question);
            for x in &c.conclusions {
                push_field(rows, &format!("Conclusion [{:?}]", x.basis), &x.statement);
            }
            for e in &c.evidence {
                push_field(
                    rows,
                    &format!("Evidence [{:?}]", e.basis),
                    &format!("{} — source {}", e.claim, e.source),
                );
            }
            for l in &c.limitations {
                push_field(rows, "Limitation", l);
            }
            push_field(rows, "Applicability", &c.applicability);
            if let Some(detail) = &c.detail {
                push_field(rows, "Detail", detail);
            }
        }
        Any::Runbook(r) => {
            let c = &r.content;
            push_field(rows, "Purpose", &c.purpose);
            for p in &c.prerequisites {
                push_field(rows, "Prerequisite", p);
            }
            for i in &c.inputs {
                push_field(
                    rows,
                    &format!(
                        "Input {}{}",
                        i.name,
                        if i.required { " (required)" } else { "" }
                    ),
                    &i.description,
                );
            }
            for (n, s) in c.steps.iter().enumerate() {
                push_field(rows, &format!("Step {} {}", n + 1, s.title), &s.description);
                if let Some(command) = &s.command {
                    push_field(rows, "  Command (never executed by a read)", command);
                }
                push_field(rows, "  Expected", &s.expected);
                if let Some(recovery) = &s.recovery {
                    push_field(rows, "  Recovery", recovery);
                }
            }
            for p in &c.pitfalls {
                push_field(rows, "Pitfall", p);
            }
            if let Some(detail) = &c.detail {
                push_field(rows, "Detail", detail);
            }
            rows.push(format!(
                "Uses: {} recorded; use evidence is tied to the revision it names. Open view=history.",
                r.uses.len()
            ));
        }
        Any::Checklist(c) => {
            push_field(rows, "Purpose", &c.content.purpose);
            for i in &c.content.items {
                let state = match i.state {
                    knowledge::ItemState::Open => "open",
                    knowledge::ItemState::Done => "done",
                    knowledge::ItemState::Canceled => "canceled",
                };
                push_field(rows, &format!("{} [{state}]", i.id), &i.text);
                if let Some(r) = &i.resolution {
                    push_field(rows, "  Resolution", &r.text);
                }
            }
            rows.push(format!(
                "Checklist {}: {} open, {} done, {} canceled. Work TODO views reuse canonical Tasks; this list never copies them.",
                match c.state {
                    knowledge::ChecklistState::Open => "open",
                    knowledge::ChecklistState::Completed => "completed",
                    knowledge::ChecklistState::Canceled => "canceled",
                },
                c.content.items.iter().filter(|i| i.state == knowledge::ItemState::Open).count(),
                c.content.items.iter().filter(|i| i.state == knowledge::ItemState::Done).count(),
                c.content.items.iter().filter(|i| i.state == knowledge::ItemState::Canceled).count(),
            ));
        }
    }
}

/// One checklist item with its resolution, or an honest absence.
fn item_rows(record: &Any, item: &str, value: &mut Page) -> Result<()> {
    let Any::Checklist(c) = record else {
        return Err(Error::new(
            "invalid_arguments",
            "ref: only a checklist has items.",
        ));
    };
    let found = c
        .content
        .items
        .iter()
        .find(|i| i.id == item)
        .ok_or_else(|| Error::new("not_found", format!("{item}: no such checklist item.")))?;
    push_field(
        &mut value.rows,
        &format!("{} [{:?}]", found.id, found.state),
        &found.text,
    );
    if let Some(r) = &found.resolution {
        push_field(&mut value.rows, "Resolution", &r.text);
        value.rows.push(format!(
            "Resolved at {} by {}.",
            r.at,
            r.by.as_deref().unwrap_or("unknown")
        ));
    }
    Ok(())
}

/// Retained revisions, Runbook uses and checklist events, with omitted counts named.
fn history_rows(record: &Any, value: &mut Page) {
    let rows = &mut value.rows;
    match record {
        Any::Decision(r) => {
            for h in r.history.iter().rev() {
                rows.push(format!(
                    "Revision {} at {} by {}: {}",
                    h.revision,
                    h.at,
                    h.by.as_deref().unwrap_or("unknown"),
                    store::safe(&h.content.decision, 300)
                ));
            }
            evicted(rows, r.evicted.len());
        }
        Any::Research(r) => {
            for h in r.history.iter().rev() {
                rows.push(format!(
                    "Revision {} at {} by {}: {}",
                    h.revision,
                    h.at,
                    h.by.as_deref().unwrap_or("unknown"),
                    store::safe(&h.content.question, 300)
                ));
            }
            evicted(rows, r.evicted.len());
        }
        Any::Runbook(r) => {
            for h in r.history.iter().rev() {
                rows.push(format!(
                    "Revision {} at {} by {}: {}",
                    h.revision,
                    h.at,
                    h.by.as_deref().unwrap_or("unknown"),
                    store::safe(&h.content.purpose, 300)
                ));
            }
            evicted(rows, r.evicted.len());
            for u in r.uses.iter().rev() {
                rows.push(format!(
                    "Use {} at {} revision {} {:?} in {}{}{}",
                    u.id,
                    u.at,
                    u.revision,
                    u.outcome,
                    store::safe(&u.environment, 100),
                    if u.stale_revision {
                        " [older revision]"
                    } else {
                        ""
                    },
                    if u.superseded_record {
                        " [superseded record]"
                    } else {
                        ""
                    },
                ));
            }
            if !r.evicted_uses.is_empty() {
                rows.push(format!(
                    "{} older use(s) are retained only as committed locators.",
                    r.evicted_uses.len()
                ));
            }
        }
        Any::Checklist(c) => {
            for e in c.events.iter().rev() {
                rows.push(format!(
                    "{} at {} by {}{}",
                    e.action,
                    e.at,
                    e.by.as_deref().unwrap_or("unknown"),
                    e.note
                        .as_deref()
                        .map(|n| format!(": {}", store::safe(n, 200)))
                        .unwrap_or_default()
                ));
            }
        }
    }
    if rows.is_empty() {
        rows.push("No retained history: the record has never been revised.".into());
    }
}

/// Name evicted history honestly.
fn evicted(rows: &mut Vec<String>, count: usize) {
    if count > 0 {
        rows.push(format!(
            "{count} older revision(s) are retained only as committed locators."
        ));
    }
}

/// Outgoing references and bounded incoming sources with their coverage.
fn reference_rows(store: &Store, target: Target, outgoing: Vec<String>, value: &mut Page) {
    for r in outgoing {
        value
            .rows
            .push(format!("Outgoing: {}", store::safe(&r, 256)));
    }
    let port = StorePort::new(store);
    match references::incoming(&port, &target) {
        Ok(result) => {
            for row in &result.rows {
                value.rows.push(format!(
                    "Incoming: {:?} {} via {:?} x{}{}",
                    row.source.kind,
                    store::safe(&row.source.id_or_path, 160),
                    row.via,
                    row.count,
                    if row.fragments.is_empty() {
                        String::new()
                    } else {
                        format!(" fragments {}", store::safe(&row.fragments.join(","), 120))
                    }
                ));
            }
            if result.rows.is_empty() {
                value.rows.push("Incoming: none found.".into());
            }
            if !result.coverage.complete {
                value.coverage = "PARTIAL".into();
                value.rows.push(format!(
                    "Incoming coverage PARTIAL: {} gap(s), {} unparsed; lower bounds.",
                    result.coverage.gaps.len(),
                    result.coverage.unparsed
                ));
            }
        }
        Err(e) => {
            value.coverage = "PARTIAL".into();
            value.rows.push(format!(
                "Incoming unknown: {}",
                store::safe(&e.message, 200)
            ));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Managed documents
// ---------------------------------------------------------------------------------------------

/// Render a document summary (state, facts, outline), exact framed content pages or references.
fn document_context(
    store: &Store,
    args: &ContextArgs,
    reference: &str,
    templates: &Templates,
) -> Result<String> {
    let port = StorePort::new(store);
    let target = documents::Ref::parse(reference)?;
    let obs = documents::observe(&port, &target)?;
    let coverage = if obs.records_complete {
        "complete"
    } else {
        "PARTIAL"
    };
    match args.view {
        View::Content => {
            let selector = Selector::from_parts(
                args.preamble.unwrap_or(false),
                args.ordinal,
                args.heading.clone(),
                args.level,
                args.occurrence,
            )?;
            let mut view = DocumentPage {
                heading: format!(
                    "{} — {}",
                    store::safe(obs.path.as_str(), 160),
                    selection_label(&selector)
                ),
                coverage: coverage.into(),
                state: obs.state.label().into(),
                version: obs.version.clone(),
                snapshot: "0".repeat(64),
                wire: "raw",
                start: 0,
                end: 0,
                range_end: 0,
                encoded_len: 0,
                text: String::new(),
                next: None,
                remaining: 0,
            };
            let budget = pages::payload_budget(templates, &view)?;
            let at = (args.start > 0).then_some(args.start);
            let read = documents::read(&obs, &selector, at, args.version.as_deref(), budget)?;
            view.snapshot = read.snapshot;
            view.wire = documents::wire_label(read.page.wire);
            view.start = read.page.start;
            view.end = read.page.end;
            view.range_end = read.page.span_end;
            view.encoded_len = read.page.text.len();
            view.text = read.page.text;
            view.next = read.page.more.then_some(read.page.end);
            view.remaining = read.page.span_end - read.page.end;
            pages::render(templates, &view)
        }
        View::References => {
            let mut value = page(format!("{reference} — references"), obs.version.clone());
            value.coverage = coverage.into();
            let outgoing = references::outgoing(&port, &obs);
            match outgoing {
                Ok(result) => {
                    for o in &result.rows {
                        value.rows.push(format!(
                            "Outgoing: line {} {:?} {:?}",
                            o.link.line,
                            o.link.target.canonical(),
                            o.resolution
                        ));
                    }
                    if !result.coverage.complete {
                        value.coverage = "PARTIAL".into();
                    }
                }
                Err(e) => {
                    value.coverage = "PARTIAL".into();
                    value.rows.push(format!(
                        "Outgoing unknown: {}",
                        store::safe(&e.message, 200)
                    ));
                }
            }
            let target = match &obs.id {
                Some(id) => Target::Knowledge(id.clone()),
                None => Target::Doc {
                    path: obs.path.clone(),
                    fragment: None,
                },
            };
            reference_rows(store, target, Vec::new(), &mut value);
            paged(
                value,
                args,
                &format!("get_context:{reference}:references"),
                &obs.version,
                templates,
            )
        }
        _ => {
            let mut value = page(
                format!("{} — document", store::safe(obs.path.as_str(), 160)),
                obs.version.clone(),
            );
            value.coverage = coverage.into();
            document_summary(&obs, &mut value);
            paged(
                value,
                args,
                &format!("get_context:{reference}:summary"),
                &obs.version,
                templates,
            )
        }
    }
}

/// Human label of a selected document part.
fn selection_label(selector: &Selector) -> String {
    match selector {
        Selector::Whole => "whole document".into(),
        Selector::Preamble => "preamble".into(),
        Selector::Ordinal(n) => format!("section ordinal {n}"),
        Selector::Heading { text, .. } => format!("section {}", store::safe(text, 80)),
    }
}

/// State, facts, record metadata and the heading outline of one observed document.
fn document_summary(obs: &documents::Observation, value: &mut Page) {
    value.lines.push(format!(
        "State: {}{}.",
        obs.state.label(),
        match obs.state {
            State::Absent => " (no file and no active record claim this path)",
            State::Unmanaged => " (readable as found; metadata unknown until explicit adoption)",
            State::Drifted => " (the file differs from its record; cause unknown)",
            State::MissingBody => " (an active record has no file)",
            State::Conflict => " (several records claim this path)",
            State::Retired => " (the record is retired)",
            State::Unsupported(_) => " (listed but not readable as supported prose)",
            State::Managed => "",
        }
    ));
    if let Some(id) = &obs.id {
        value.lines.push(format!("Identity: {id}."));
    }
    if let Some(record) = &obs.record {
        value.lines.push(format!(
            "Record: revision {} purpose {}.",
            record.record.revision,
            store::safe(&record.record.purpose, 200)
        ));
    }
    if let Some(f) = &obs.facts {
        value.lines.push(format!(
            "Facts: {} bytes, sha256 {}; BOM {}; LF {}, CRLF {}, lone CR {}, NUL {}.",
            f.bytes, f.sha256, f.bom, f.lf, f.crlf, f.lone_cr, f.nul
        ));
    }
    if obs.state == State::Absent {
        value.lines.push(format!(
            "Create it with document_work save using version {} (an absent path has a version too).",
            obs.version
        ));
    } else {
        value.lines.push(format!(
            "Write with document_work using version {}; read exact bytes with view=content (heading, occurrence, level, ordinal or preamble select a part).",
            obs.version
        ));
    }
    if !obs.records_complete {
        value.coverage = "PARTIAL".into();
        value.lines.push(
            "Record inventory is incomplete: claims on this path cannot be fully decided.".into(),
        );
    }
    if let Some(outline) = &obs.outline {
        if outline.setext_candidates > 0 {
            value.lines.push(format!("{} setext-like heading candidate(s) are ordinary content; heading fragments stay partial.", outline.setext_candidates));
        }
        if !outline.complete {
            value.coverage = "PARTIAL".into();
        }
        for h in &outline.headings {
            value.rows.push(format!(
                "Section ordinal {} level {} occurrence {} bytes {}-{}: {}",
                h.ordinal,
                h.level,
                h.occurrence,
                h.start,
                h.end,
                store::safe(&h.text, 120)
            ));
        }
        if outline.headings.is_empty() {
            value
                .rows
                .push("No headings; the whole document is the preamble.".into());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Compaction proposals
// ---------------------------------------------------------------------------------------------

/// Render one compaction proposal: summary, actions and sections, reviews or references.
fn compaction_context(
    store: &Store,
    args: &ContextArgs,
    reference: &str,
    templates: &Templates,
) -> Result<String> {
    let snapshot = compaction::read_cp(store, reference)?;
    let record = &snapshot.value;
    let mut value = page(
        format!("{reference} — compaction proposal"),
        snapshot.version.clone(),
    );
    value.lines.push(format!(
        "State: {}; revision {} of {}; reviewer {}.",
        record.state.label(),
        record.current,
        record.revisions.len(),
        record
            .reviewer
            .as_ref()
            .map_or("unbound", |r| r.agent_id.as_str())
    ));
    let revision = record.revision()?;
    value.lines.push(format!(
        "Title: {}; content hash {}; authored by {} at {}.",
        store::safe(&revision.title, 160),
        revision.content_hash,
        store::safe(&revision.author, 100),
        revision.at
    ));
    if let Some(apply) = &record.apply {
        value.lines.push(format!(
            "Apply: phase {:?}, attempts {}{}.",
            apply.phase,
            apply.attempts,
            apply
                .blocked
                .as_ref()
                .map(|b| format!(", BLOCKED {} — {}", b.kind, store::safe(&b.reason, 200)))
                .unwrap_or_default()
        ));
    }
    match args.view {
        View::Tasks => cp_actions(record, &mut value),
        View::Review => cp_reviews(record, args.review_index, &mut value)?,
        View::References => {
            for s in &revision.body.sources {
                value.rows.push(format!(
                    "Source: {} version {}",
                    store::safe(&s.path, 160),
                    s.version
                ));
            }
            for a in &revision.body.actions {
                value.rows.push(format!(
                    "Target: {:?} {}",
                    a.kind,
                    store::safe(&a.path, 160)
                ));
            }
        }
        _ => {
            let applied = revision
                .body
                .actions
                .iter()
                .filter(|a| a.state == ActionState::Applied)
                .count();
            value.rows.push(format!(
                "Actions {applied}/{} applied; {} section(s) accounted; {} preservation item(s); {} review(s).",
                revision.body.actions.len(),
                revision.body.sections.len(),
                revision.body.preservation.len(),
                record.reviews.len()
            ));
            value.rows.push("Open view=tasks for actions and sections, view=review for the reviews, view=references for sources and targets.".into());
        }
    }
    let selection = format!(
        "get_context:{reference}:{:?}:{:?}",
        args.view, args.review_index
    );
    paged(value, args, &selection, &snapshot.version, templates)
}

/// Actions and section dispositions of the current revision.
fn cp_actions(record: &CpRecord, value: &mut Page) {
    let Ok(revision) = record.revision() else {
        return;
    };
    for a in &revision.body.actions {
        value.rows.push(format!(
            "Action {} {:?} {} [{:?}]: {}",
            a.id,
            a.kind,
            store::safe(&a.path, 160),
            a.state,
            store::safe(&a.reason, 200)
        ));
    }
    for s in &revision.body.sections {
        value.rows.push(format!(
            "Section {} {} {:?}",
            s.id,
            store::safe(&s.path, 120),
            s.disposition
        ));
    }
}

/// One retained review with its findings; the latest when no index is given.
fn cp_reviews(record: &CpRecord, index: Option<usize>, value: &mut Page) -> Result<()> {
    if record.reviews.is_empty() {
        value.rows.push("No review has been recorded.".into());
        return Ok(());
    }
    let at = index.unwrap_or(record.reviews.len() - 1);
    let review = record.reviews.get(at).ok_or_else(|| {
        Error::new(
            "invalid_arguments",
            "review_index: no such retained review (zero-based).",
        )
    })?;
    value.rows.push(format!(
        "Review {at} of {} revision {} by {}: {} — {}",
        record.reviews.len(),
        review.revision,
        store::safe(&review.reviewer, 100),
        verdict(review.verdict),
        store::safe(&review.summary, 300)
    ));
    for (n, f) in review.findings.iter().enumerate() {
        value
            .rows
            .push(format!("Finding {n}: {}", store::safe(&f.text, 400)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Project context coverage
// ---------------------------------------------------------------------------------------------

/// Add typed knowledge, document, compaction and Git pending coverage to project context.
///
/// Every figure names its coverage; an incomplete inventory is PARTIAL with lower bounds, never a
/// silent zero. Pending Git facts come from the persistence provider's read-only summary.
pub(super) fn project_lines(store: &Store, value: &mut Page) {
    match knowledge::observe_allocation(store) {
        Ok(a) => {
            value
                .lines
                .push(format!("Knowledge allocation version: {}", a.version));
            if !a.complete {
                value.coverage = "PARTIAL".into();
                value.lines.push("Knowledge allocation: PARTIAL, a home or the allocator file is not fully understood; creation refuses until restored.".into());
            }
        }
        Err(e) => {
            value.coverage = "PARTIAL".into();
            value.lines.push(format!(
                "Knowledge allocation unknown: {}",
                store::safe(&e.message, 160)
            ));
        }
    }
    match knowledge::scan(store, None) {
        Ok(scan) => {
            let count = |k: knowledge::Kind, current: bool| {
                scan.records
                    .iter()
                    .filter(|s| s.value.kind() == k && s.value.current() == current)
                    .count()
            };
            value.lines.push(format!(
                "Knowledge: Decisions {} current/{} superseded; Runbooks {} current/{} superseded; Research {} current/{} superseded; Checklists {} open/{} closed{}.",
                count(knowledge::Kind::Decision, true), count(knowledge::Kind::Decision, false),
                count(knowledge::Kind::Runbook, true), count(knowledge::Kind::Runbook, false),
                count(knowledge::Kind::Research, true), count(knowledge::Kind::Research, false),
                count(knowledge::Kind::Checklist, true), count(knowledge::Kind::Checklist, false),
                if scan.complete { "; coverage complete" } else { "; coverage PARTIAL, lower bounds" }
            ));
            if !scan.complete {
                value.coverage = "PARTIAL".into();
                for issue in scan.unreadable.iter().take(3) {
                    value
                        .lines
                        .push(format!("Knowledge UNREADABLE {}", store::safe(issue, 200)));
                }
            }
        }
        Err(e) => {
            value.coverage = "PARTIAL".into();
            value.lines.push(format!(
                "Knowledge unknown: {}",
                store::safe(&e.message, 160)
            ));
        }
    }
    let port = StorePort::new(store);
    match documents::inventory(&port) {
        Ok(inv) => {
            let counts = documents::state_counts(&inv.rows)
                .iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            value.lines.push(format!(
                "Documents: {} listed ({}); coverage {}.",
                inv.rows.len(),
                if counts.is_empty() {
                    "none".into()
                } else {
                    counts
                },
                if inv.complete {
                    "complete"
                } else {
                    "PARTIAL, lower bounds"
                }
            ));
            if !inv.complete {
                value.coverage = "PARTIAL".into();
                for gap in inv.gaps.iter().take(3) {
                    value
                        .lines
                        .push(format!("Documents gap {} {:?}", gap.what, gap.reason));
                }
            }
        }
        Err(e) => {
            value.coverage = "PARTIAL".into();
            value.lines.push(format!(
                "Documents unknown: {}",
                store::safe(&e.message, 160)
            ));
        }
    }
    match compaction::summaries(store, 20) {
        Ok(s) => {
            let live = s.rows.iter().filter(|r| !r.state.terminal()).count();
            value.lines.push(format!(
                "Compaction proposals: {} readable, {live} non-terminal shown{}{}.",
                s.total,
                if s.complete {
                    "; coverage complete"
                } else {
                    "; coverage PARTIAL"
                },
                if s.omitted > 0 {
                    format!("; {} not shown", s.omitted)
                } else {
                    String::new()
                }
            ));
            if !s.complete {
                value.coverage = "PARTIAL".into();
            }
        }
        Err(e) => {
            value.coverage = "PARTIAL".into();
            value.lines.push(format!(
                "Compaction unknown: {}",
                store::safe(&e.message, 160)
            ));
        }
    }
    let pending = persist::pending(store);
    value.lines.push(format!(
        "Git persistence: {} pending intent(s), {} path(s), {} unknown, {} drifted; pending version {}{}.",
        pending.facts.intents,
        pending.facts.paths,
        pending.facts.unknown,
        pending.facts.drifted,
        pending.version,
        if pending.complete { "" } else { "; coverage PARTIAL" }
    ));
    if !pending.complete {
        value.coverage = "PARTIAL".into();
    }
}

// ---------------------------------------------------------------------------------------------
// Lexical search sources
// ---------------------------------------------------------------------------------------------

/// Which sources one search covers: `(work, knowledge, document)`.
///
/// Omitted `kinds` means all three, except that a supplied `module` scopes the search to work
/// only, so existing scoped calls behave as before. A `module` combined with an explicit
/// knowledge or document kind is refused naming `module`, because a module scope cannot apply to
/// those sources.
pub(super) fn search_kinds(args: &SearchArgs) -> Result<(bool, bool, bool)> {
    let kinds = match (&args.kinds, &args.module) {
        (Some(k), _) if k.is_empty() || k.len() > 3 => {
            return Err(Error::new(
                "invalid_arguments",
                "kinds: give one to three distinct kinds.",
            ));
        }
        (Some(k), _) => k.clone(),
        (None, Some(_)) => vec![SearchKind::Work],
        (None, None) => vec![
            SearchKind::Work,
            SearchKind::Knowledge,
            SearchKind::Document,
        ],
    };
    for (n, k) in kinds.iter().enumerate() {
        if kinds[..n].contains(k) {
            return Err(Error::new(
                "invalid_arguments",
                "kinds: kinds must be distinct.",
            ));
        }
    }
    let (work, knowledge, document) = (
        kinds.contains(&SearchKind::Work),
        kinds.contains(&SearchKind::Knowledge),
        kinds.contains(&SearchKind::Document),
    );
    if args.module.is_some() && (knowledge || document) {
        return Err(Error::new(
            "invalid_arguments",
            "module: a module scope applies to work only; drop the knowledge and document kinds.",
        ));
    }
    Ok((work, knowledge, document))
}

/// The knowledge and document sources one search read, each kept with its own failure.
pub(super) struct Sources {
    /// Typed record scan, when knowledge is searched.
    pub(super) knowledge: Option<Result<knowledge::Scan>>,
    /// Markdown corpus, when documents are searched.
    pub(super) corpus: Option<Result<documents::Corpus>>,
}

impl Sources {
    /// Read the requested sources; a failing source is named later, never turned into zero hits.
    pub(super) fn gather(store: &Store, knowledge: bool, document: bool) -> Self {
        Sources {
            knowledge: knowledge.then(|| knowledge::scan(store, None)),
            corpus: document.then(|| documents::corpus(&StorePort::new(store))),
        }
    }

    /// Digest of every source version, so continuation stops when any searched source changed.
    pub(super) fn basis(&self) -> String {
        let part = |name: &str, version: Option<&str>| {
            format!("{name}={}", version.unwrap_or("unavailable"))
        };
        format!(
            "{};{}",
            part(
                "knowledge",
                self.knowledge
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map(|s| s.version.as_str())
            ),
            part(
                "documents",
                self.corpus
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map(|c| c.version.as_str())
            ),
        )
    }

    /// One coverage line per searched source; a partial or failed source marks the reply PARTIAL.
    pub(super) fn coverage_lines(&self, value: &mut Page) {
        if let Some(result) = &self.knowledge {
            match result {
                Ok(scan) => {
                    value.lines.push(format!(
                        "Coverage knowledge: {} readable record(s), {}.",
                        scan.records.len(),
                        if scan.complete {
                            "complete"
                        } else {
                            "PARTIAL (lower bounds)"
                        }
                    ));
                    if !scan.complete {
                        value.coverage = "PARTIAL".into();
                        for issue in scan.unreadable.iter().take(2) {
                            value.lines.push(format!(
                                "Warning: knowledge unreadable {}",
                                store::safe(issue, 200)
                            ));
                        }
                    }
                }
                Err(e) => {
                    value.coverage = "PARTIAL".into();
                    value.lines.push(format!(
                        "Coverage knowledge: unknown, {}",
                        store::safe(&e.message, 200)
                    ));
                }
            }
        }
        if let Some(result) = &self.corpus {
            match result {
                Ok(corpus) => {
                    value.lines.push(format!(
                        "Coverage documents: {} file(s) read, {} byte(s), {}.",
                        corpus.files_read,
                        corpus.bytes_read,
                        if corpus.complete {
                            "complete"
                        } else {
                            "PARTIAL (lower bounds)"
                        }
                    ));
                    if !corpus.complete {
                        value.coverage = "PARTIAL".into();
                        for gap in corpus.gaps.iter().take(3) {
                            value.lines.push(format!(
                                "Warning: documents gap {} {:?}",
                                gap.what, gap.reason
                            ));
                        }
                    }
                }
                Err(e) => {
                    value.coverage = "PARTIAL".into();
                    value.lines.push(format!(
                        "Coverage documents: unknown, {}",
                        store::safe(&e.message, 200)
                    ));
                }
            }
        }
    }

    /// Typed knowledge hits over the stable per-kind field labels; currentness is explicit.
    pub(super) fn knowledge_hits(
        &self,
        terms: &[String],
        filter: StateFilter,
        hits: &mut Vec<Hit>,
    ) {
        let Some(Ok(scan)) = &self.knowledge else {
            return;
        };
        for (index, record) in scan.records.iter().enumerate() {
            let any = &record.value;
            let current = any.current();
            if !keeps(filter, current) {
                continue;
            }
            let fields = any.search_text();
            let before = hits.len();
            hit(
                hits,
                any.id().to_owned(),
                any.title(),
                (u64::MAX / 2, index as u64),
                fields,
                terms,
            );
            if hits.len() > before
                && let Some(h) = hits.last_mut()
            {
                h.kind = "knowledge";
                h.state = any.state_label().to_owned();
                h.current = current;
            }
        }
    }

    /// Markdown hits: path, purpose, headings and body, each located at its section.
    pub(super) fn document_hits(&self, terms: &[String], filter: StateFilter, hits: &mut Vec<Hit>) {
        let Some(Ok(corpus)) = &self.corpus else {
            return;
        };
        for (index, doc) in corpus.docs.iter().enumerate() {
            let current = !matches!(doc.state, State::Retired);
            if !keeps(filter, current) {
                continue;
            }
            let heading_text = doc
                .headings
                .iter()
                .map(|h| h.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let fields = [
                ("path", doc.path.as_str()),
                ("purpose", doc.purpose.as_deref().unwrap_or_default()),
                ("headings", heading_text.as_str()),
                ("body", doc.text.as_str()),
            ];
            let lower: Vec<String> = fields.iter().map(|(_, v)| v.to_lowercase()).collect();
            if !terms
                .iter()
                .all(|t| lower.iter().any(|f| f.contains(t.as_str())))
            {
                continue;
            }
            let matched: Vec<&str> = fields
                .iter()
                .zip(&lower)
                .filter(|(_, v)| terms.iter().any(|t| v.contains(t.as_str())))
                .map(|((label, _), _)| *label)
                .collect();
            let (excerpt, route) = locate(doc, terms);
            hits.push(Hit {
                reference: doc.reference.clone(),
                title: doc.path.as_str().to_owned(),
                score: matched.len(),
                order: (u64::MAX - 1, index as u64),
                fields: matched.iter().map(|m| (*m).to_owned()).collect(),
                excerpt: store::safe(&excerpt, 220),
                kind: "document",
                state: doc.state.label().to_owned(),
                current,
                route,
            });
        }
    }
}

/// Whether a hit's currentness passes the filter.
fn keeps(filter: StateFilter, current: bool) -> bool {
    match filter {
        StateFilter::Any => true,
        StateFilter::Current => current,
        StateFilter::Superseded => !current,
    }
}

/// Excerpt and exact read route of a document hit.
///
/// The first body line holding any term gives the excerpt and, through its byte offset, the
/// innermost heading; the route reads exactly that section (or the preamble) and names the
/// occurrence so duplicate headings resolve. A match only in the path, purpose or headings routes
/// to the whole document.
fn locate(doc: &documents::CorpusDoc, terms: &[String]) -> (String, String) {
    let whole = format!("get_context ref={}", doc.reference);
    let mut offset = 0;
    for line in doc.text.split_inclusive('\n') {
        let lower = line.to_lowercase();
        if terms.iter().any(|t| lower.contains(t.as_str())) {
            let excerpt = line.trim().to_owned();
            let route = match markdown::heading_at(&doc.headings, offset)
                .and_then(|i| doc.headings.get(i))
            {
                Some(h) => format!(
                    "get_context ref={} heading={} occurrence={} level={} view=content",
                    doc.reference,
                    serde_json::to_string(&h.text).unwrap_or_default(),
                    h.occurrence,
                    h.level
                ),
                None => format!(
                    "get_context ref={} preamble=true view=content",
                    doc.reference
                ),
            };
            return (excerpt, route);
        }
        offset += line.len();
    }
    let first = doc
        .purpose
        .clone()
        .unwrap_or_else(|| doc.path.as_str().to_owned());
    (first, whole)
}

/// Context, search and page qualification over disposable roots.
#[cfg(test)]
mod tests;
