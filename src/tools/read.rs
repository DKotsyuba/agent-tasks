//! Semantic bounded retrieval. Coverage, unknown data and real pagination remain explicit.
use super::{
    input::{ContextArgs, SearchArgs, StatusArgs, View},
    work::{Page, module_id},
};
use crate::{
    model::*,
    response::Templates,
    store::{self, Config, Error, Result, Snapshot, Store},
};
use sha2::{Digest, Sha256};

/// New typed page with explicit data and detail coverage; empty does not mean unreadable.
pub(super) fn page(heading: String, version: String) -> Page {
    Page {
        heading,
        lines: Vec::new(),
        rows: Vec::new(),
        version: version.clone(),
        snapshot_version: version,
        allocation_version: None,
        coverage: "complete".into(),
        detail_coverage: "complete".into(),
        next_start: None,
        remaining: 0,
    }
}

/// Require a bounded page and its exact prior snapshot for continuation.
pub(super) fn continuation(
    start: usize,
    limit: usize,
    expected: Option<&str>,
    snapshot: &str,
) -> Result<()> {
    if limit == 0 || limit > 20 {
        return Err(Error::new(
            "invalid_arguments",
            "limit must be between 1 and 20.",
        ));
    }
    if start > 0 && expected.is_none() {
        return Err(Error::new(
            "stale",
            "Continuation needs the snapshot version from the preceding page.",
        ));
    }
    if expected.is_some_and(|v| v != snapshot) {
        return Err(Error::new(
            "stale",
            format!(
                "Read snapshot changed; start again at zero. Current snapshot version: {snapshot}."
            ),
        ));
    }
    Ok(())
}

/// Fit the largest prefix under 8 KiB with exact offsets. Check the full page
/// first because its final footer can be shorter; then grow only until the bound.
pub(super) fn render_page(
    mut value: Page,
    start: usize,
    limit: usize,
    paginate: bool,
    templates: &Templates,
) -> Result<String> {
    let total = value.rows.len();
    if start > total {
        return Err(Error::new(
            "invalid_arguments",
            "Page start is beyond the available items.",
        ));
    }
    let rows = std::mem::take(&mut value.rows);
    let maximum = (total - start).min(limit);
    let original_detail = value.detail_coverage.clone();
    let render = |count: usize, value: &mut Page| {
        value.rows = rows[start..start + count].to_vec();
        value.remaining = total - start - count;
        value.next_start = if paginate && value.remaining > 0 {
            Some(start + count)
        } else {
            None
        };
        value.detail_coverage = if value.remaining > 0 {
            "PARTIAL".into()
        } else {
            original_detail.clone()
        };
        templates.render("core_page", value)
    };
    if let Ok(text) = render(maximum, &mut value) {
        return Ok(text);
    }
    let mut best = None;
    for count in 1..maximum {
        match render(count, &mut value) {
            Ok(text) => best = Some(text),
            Err(_) => break,
        }
    }
    best.ok_or_else(||Error::new("presentation_capacity","One detail row or the context header cannot fit safely; narrow the module, task or view. No read or work is claimed complete."))
}

/// Add bounded warnings, explicitly counting omitted diagnostics without hiding data coverage.
fn warnings(value: &mut Page, warnings: &[String]) {
    for warning in warnings.iter().take(6) {
        value
            .lines
            .push(format!("Warning: {}", store::safe(warning, 240)));
    }
    if warnings.len() > 6 {
        value.lines.push(format!(
            "{} further diagnostics omitted; narrow get_context to named modules.",
            warnings.len() - 6
        ));
    }
}

/// Expose normalization/counter/capacity facts without modifying bytes during reads.
fn diagnostics(value: &mut Page, snapshot: &Snapshot<Module>) {
    if store::encode(&snapshot.value).is_ok_and(|bytes| bytes != snapshot.bytes) {
        value.lines.push(
            "Noncanonical YAML: the next write preserves the exact original before normalization."
                .into(),
        );
    }
    if let Err(e) = snapshot.value.counters() {
        value
            .lines
            .push(format!("Writes blocked: {}", store::safe(&e, 240)));
    }
    if snapshot.bytes.len() > store::RECORD_CAP - store::CLOSING_RESERVE - 32 * 1024 {
        value.lines.push("Capacity warning: normal-edit headroom is low; current human evidence is never silently dropped.".into());
    }
    let m = &snapshot.value;
    if m.reviews
        .last()
        .is_some_and(|r| m.basis().is_ok_and(|b| b != r.basis) && r.epoch == m.review_epoch)
    {
        value.lines.push("Review no longer matches current work: possible native edit. No automatic rollback or repair.".into());
    }
}

/// Read one scope and construct human-oriented facts, conditions and next routes.
pub fn context(config: &Config, args: ContextArgs, templates: &Templates) -> Result<String> {
    let store = config.resolve(&args.project)?;
    let _lock = store.lock(false, &mut Vec::new())?;
    if args.reference.is_none() {
        if !matches!(args.view, View::Summary) || args.review_index.is_some() {
            return Err(Error::new(
                "invalid_arguments",
                "Project context uses summary; select a module/task ref for a detail view.",
            ));
        }
        let project = store.project()?;
        let scan = store.scan(None)?;
        let manifest_version = project
            .as_ref()
            .map(|p| p.version.clone())
            .unwrap_or_else(|| store.version("project.yaml", None));
        let snapshot = scope_version("get_context:project", &manifest_version, &scan.version);
        continuation(args.start, args.limit, args.version.as_deref(), &snapshot)?;
        let mut value = page(
            format!(
                "Project {}{}",
                store::safe(&args.project, 128),
                project
                    .as_ref()
                    .map(|p| format!(" — {}", store::safe(&p.value.title, 256)))
                    .unwrap_or_default()
            ),
            manifest_version,
        );
        value.snapshot_version = snapshot;
        value.allocation_version = store.allocation_version().ok();
        value.coverage = if scan.complete { "complete" } else { "PARTIAL" }.into();
        match project {
            Some(p) => {
                value
                    .lines
                    .push(format!("Purpose: {}", store::safe(&p.value.purpose, 1024)));
                if let Some(remote) = &p.value.remote {
                    value.lines.push(format!(
                        "Repository (reported): {}",
                        store::safe(remote, 256)
                    ));
                }
                if store::encode(&p.value).is_ok_and(|b| b != p.bytes) {
                    value.lines.push(
                        "Manifest is noncanonical; the next edit backs up its exact original."
                            .into(),
                    );
                }
                value.lines.push("Next: plan_work create_module uses allocation_version; edit_project uses manifest Version.".into());
            }
            None => {
                let allocator = store.bytes(".agent-tasks/state.yaml")?;
                let partial = allocator.as_ref().is_some_and(|b| {
                    store::decode::<Allocator>(b)
                        .is_ok_and(|s| matches!(s.schema_version, 1 | 2) && s.next_module == 1)
                }) && scan.modules.is_empty()
                    && scan.complete;
                value.lines.push(if partial {"Empty partial initialization: explicit init_project may resume missing manifest."}else{"Project is not initialized; only explicit plan_work init_project may create it."}.into());
                if !scan.modules.is_empty() {
                    value.coverage = "PARTIAL".into();
                    value.lines.push("Modules exist without a valid project manifest; initialization refuses this inconsistent state.".into());
                }
                value.lines.push("Use returned allocation_version for explicit init_project; reads create nothing.".into());
            }
        }
        match store.bytes(".agent-tasks/state.yaml") {
            Ok(Some(bytes))=>{
                let ids=scan.modules.iter().map(|m|m.value.id.clone()).collect::<Vec<_>>(); let valid=store::decode::<Allocator>(&bytes).is_ok_and(|mut s|s.prepare(&ids).is_ok());
                if !valid {value.lines.push("Module allocation blocked: invalid allocator. Restore retained state; healthy reads remain available.".into());}
            },
            Ok(None)=>value.lines.push("Allocator absent: healthy work remains readable; new module allocation waits for valid state or explicit empty init.".into()),
            Err(e)=>value.lines.push(format!("Allocator unreadable: {}",store::safe(&e.message,200))),
        }
        warnings(&mut value, &scan.warnings);
        for m in scan.modules {
            value.rows.push(module_brief(&store, &m.value));
        }
        for issue in scan.unreadable {
            value
                .rows
                .push(format!("UNREADABLE {}", store::safe(&issue, 240)));
        }
        return render_page(value, args.start, args.limit, true, templates);
    }
    let reference = args
        .reference
        .as_deref()
        .ok_or_else(|| Error::new("invalid_arguments", "Missing reference."))?;
    let snapshot = store.module(module_id(reference)?)?;
    let m = &snapshot.value;
    let index = m.target(reference).map_err(store::invalid)?;
    let selection = format!(
        "get_context:{reference}:{:?}:{:?}",
        args.view, args.review_index
    );
    let (parent, parent_warning) = match store.parent(&m.id) {
        Ok(parent) => (parent, None),
        Err(e) => (None, Some(e.message)),
    };
    let project = store.project()?;
    let parent_version = parent
        .as_ref()
        .map(|p| store.module(&p.id).map(|s| s.version))
        .transpose()?
        .unwrap_or_default();
    let background = format!(
        "{}:{}:{}",
        parent_version,
        parent_warning.as_deref().unwrap_or_default(),
        project
            .as_ref()
            .map(|p| p.version.as_str())
            .unwrap_or_default()
    );
    let read_version = scope_version(&selection, &snapshot.version, &background);
    continuation(
        args.start,
        args.limit,
        args.version.as_deref(),
        &read_version,
    )?;
    let mut value = page(
        format!(
            "{} — {}",
            reference,
            store::safe(
                index.map_or(m.title.as_str(), |i| m.child(i).title.as_str()),
                256
            )
        ),
        snapshot.version.clone(),
    );
    value.snapshot_version = read_version;
    diagnostics(&mut value, &snapshot);
    let child = index.map(|i| m.child(i));
    let reviews = child
        .and_then(|a| a.atomic_workflow.as_ref().map(|w| &w.reviews))
        .unwrap_or(&m.reviews);
    if let Some(w) = &m.workflow {
        value.lines.push(format!(
            "Lifecycle: {}; reported begin: {}; first start: {}.",
            if w.managed {
                "modern"
            } else {
                "legacy declarations; explicit begin opts in"
            },
            w.active,
            w.started_at.as_deref().unwrap_or("not reported")
        ));
        if let Some(e) = &w.execution {
            value.lines.push(format!(
                "Execution (reported): repo={} worktree={} branch={} target={}",
                store::safe(&e.repository, 1024),
                store::safe(&e.worktree, 1024),
                store::safe(&e.branch, 128),
                store::safe(&e.target_branch, 128)
            ));
        }
        if let Some(d) = &w.delivery {
            value.rows.push(format!(
                "Delivery [{}] to {}: {} — {} at {}",
                if m.delivered() { "current" } else { "stale" },
                store::safe(&d.target_branch, 128),
                store::safe(&d.summary, 1024),
                d.actor.as_deref().unwrap_or("unknown"),
                d.at
            ));
        }
        if let Some(roster) = &w.frozen_modules {
            value.lines.push(format!(
                "Frozen Module roster: {}; reopening never unlocks it.",
                roster.join(", ")
            ));
        }
        if let Some(c) = &w.contracts {
            if c.not_required {
                value
                    .lines
                    .push("Contracts: explicitly not required.".into());
            }
            for (direction, items) in [("provides", &c.provides), ("consumes", &c.consumes)] {
                for contract in items {
                    value.rows.push(format!(
                        "{direction} {} [{}]: {}{}",
                        contract.peer,
                        if contract.ready {
                            "ready (reported)"
                        } else {
                            "not ready"
                        },
                        store::safe(&contract.description, 1024),
                        contract
                            .reference
                            .as_ref()
                            .map(|r| format!(" — {}", store::safe(r, 256)))
                            .unwrap_or_default()
                    ));
                }
            }
        }
        for id in w.dependencies.iter().map(|d| &d.reference).chain(
            w.contracts
                .iter()
                .flat_map(|c| c.provides.iter().chain(&c.consumes))
                .map(|c| &c.peer),
        ) {
            if let Err(e) = store.module(id) {
                value.coverage = "PARTIAL".into();
                value.rows.push(format!(
                    "UNREADABLE requirement {id}: {}",
                    store::safe(&e.message, 240)
                ));
            }
        }
        for d in &w.dependencies {
            value.rows.push(format!(
                "Start dependency {} {:?}: {}",
                d.reference,
                d.condition,
                store::safe(&d.reason, 512)
            ));
        }
        if let Some(environment) = &w.environment {
            value.rows.push(format!(
                "Integration environment: {}",
                store::safe(environment, 1024)
            ));
        }
        for scenario in &w.scenarios {
            value.rows.push(format!(
                "Integration scenario: {}",
                store::safe(scenario, 256)
            ));
        }
        if matches!(args.view, View::Summary) {
            for condition in store.readiness(m) {
                value
                    .rows
                    .push(format!("Before begin: {}", store::safe(&condition, 512)));
            }
        }
    } else {
        value.lines.push(
            "Lifecycle: grandfathered legacy; explicit begin opts into modern conditions.".into(),
        );
    }
    if let Some(a) = child
        && a.id.starts_with("A-")
    {
        value.lines.push(format!(
            "Atomic phase: {}. Independent Atomic review is separate from Module review.",
            a.atomic_phase()
        ));
    }

    for id in m
        .modules
        .iter()
        .chain(&m.atomic_members)
        .chain(&m.participants)
    {
        if let Err(e) = store.module(id) {
            value.coverage = "PARTIAL".into();
            value
                .rows
                .push(format!("UNREADABLE {id}: {}", store::safe(&e.message, 240)));
        }
    }
    if let Some(warning) = &parent_warning {
        value.coverage = "PARTIAL".into();
        value.lines.push(format!(
            "Parent ownership unknown: {}. Writes refuse until ownership can be proven.",
            store::safe(warning, 240)
        ));
    }
    if let Some(p) = project {
        value.lines.push(format!(
            "Project background: {} — {}. Check labels are not inherited.",
            store::safe(&p.value.title, 120),
            store::safe(&p.value.purpose, 240)
        ));
    }
    if let Some(parent) = parent {
        value.lines.push(format!("Epic parent {} — {} — {}. Parent criteria are background; this target owns its explicit checks.",parent.id,store::safe(&parent.title,160),store::safe(&parent.outcome,240)));
        for criterion in &parent.criteria {
            value.rows.push(format!(
                "Parent acceptance criterion (background): {}",
                store::safe(criterion, 1024)
            ));
        }
        if parent.state == ModuleState::Canceled {
            value
                .lines
                .push("Parent is canceled: reopen it before child writes.".into());
        }
    } else if !m.id.starts_with("E-") && parent_warning.is_none() {
        value
            .lines
            .push("Ownership: standalone Project work.".into());
    }
    if !m.participants.is_empty() {
        value.lines.push(format!("Integration participants: {}. Evidence generation: {}; new Module reviews also invalidate it.", m.participants.join(", "), if store.acceptance(m).iter().any(|c|c.contains("Integration")) { "stale/missing" } else { "current" }));
    }
    if !matches!(args.view, View::Review) && args.review_index.is_some() {
        return Err(Error::new(
            "invalid_arguments",
            "review_index belongs only to view=review.",
        ));
    }
    match args.view {
        View::Summary => {
            value.lines.push(format!(
                "{} phase: {}; Task counts: {} done, {} open, {} canceled.",
                if m.id.starts_with("E-") {
                    "Epic"
                } else if m.id.starts_with("A-") {
                    "Atomic"
                } else {
                    "Module"
                },
                store.phase(m),
                count(m, TaskState::Done),
                count(m, TaskState::Open),
                count(m, TaskState::Canceled)
            ));
            value
                .lines
                .push(format!("Outcome: {}", store::safe(&m.outcome, 1024)));
            value.lines.push(lead_line(m));
            value.lines.push(format!(
                "Atomic counts (embedded): {} done, {} open, {} canceled.",
                m.atomics
                    .iter()
                    .filter(|a| matches!(a.atomic_phase(), "done" | "accepted"))
                    .count(),
                m.atomics
                    .iter()
                    .filter(|a| a.state == TaskState::Open)
                    .count(),
                m.atomics
                    .iter()
                    .filter(|a| a.state == TaskState::Canceled)
                    .count()
            ));
            for criterion in &m.criteria {
                value.rows.push(format!(
                    "Acceptance criterion: {}",
                    store::safe(criterion, 1024)
                ));
            }
            if m.id.starts_with("E-") {
                let members = m.modules.iter().chain(&m.atomic_members);
                for id in members {
                    value.rows.push(match store.module(id) {
                        Ok(child) => module_brief(&store, &child.value),
                        Err(e) => {
                            value.coverage = "PARTIAL".into();
                            format!("UNREADABLE {id}: {}", store::safe(&e.message, 240))
                        }
                    });
                }
            }
            value.lines.push(format!(
                "Last managed report/change: {}; activity is reported, not live agent state.",
                m.updated_at
            ));
            if let Some(i) = index {
                let t = &m.child(i);
                value.lines.push(task_brief(m, t));
                if let Some(c) = &t.criterion {
                    value
                        .rows
                        .push(format!("Criterion: {}", store::safe(c, 1024)));
                }
                if let Some(c) = &t.cancellation {
                    value
                        .rows
                        .push(format!("Canceled: {}", store::safe(&c.reason, 512)));
                }
            }
            if let Some(b) = &m.blocker {
                value.rows.push(format!(
                    "BLOCKER {} — needed: {} — resolver: {}",
                    store::safe(&b.problem, 512),
                    store::safe(&b.needed_action, 512),
                    b.resolver
                        .as_ref()
                        .map(|r| store::safe(r, 128))
                        .unwrap_or("unknown".into())
                ));
            }
            if let Some(h) = &m.handoff {
                value.rows.push(format!(
                    "Handoff: {} — next: {}",
                    store::safe(&h.stopping_point, 512),
                    store::safe(&h.next_action, 512)
                ));
            }
            let missing = store.acceptance(m);
            value.lines.push(format!("Acceptance conditions remaining: {}. Evidence is reported; no Git/GitHub verification.",missing.len()));
            for condition in missing {
                value
                    .rows
                    .push(format!("Needs: {}", store::safe(&condition, 320)));
            }
            if let Some(review) = reviews.last() {
                value.rows.push(format!(
                    "Latest review: {} by {} — {} (history: {}). Use view=review.",
                    verdict(review.verdict),
                    review
                        .reviewer
                        .as_ref()
                        .map(|s| store::safe(s, 128))
                        .unwrap_or("unknown".into()),
                    store::safe(&review.summary, 180),
                    reviews.len()
                ));
            }
            value.lines.push("Next: view=tasks/results/checks/review/log for detail; record_work result for current evidence; review_module for a whole-module verdict.".into());
        }
        View::Tasks => {
            if index.is_some() {
                return Err(Error::new(
                    "invalid_arguments",
                    "view=tasks requires the module reference.",
                ));
            }
            if m.id.starts_with("E-") {
                for id in m.modules.iter().chain(&m.atomic_members) {
                    value.rows.push(match store.module(id) {
                        Ok(child) => module_brief(&store, &child.value),
                        Err(e) => {
                            value.coverage = "PARTIAL".into();
                            format!("UNREADABLE {id}: {}", store::safe(&e.message, 240))
                        }
                    });
                }
            }
            for t in m.children() {
                value.rows.push(format!(
                    "{}{}{}",
                    task_brief(m, t),
                    t.criterion
                        .as_ref()
                        .map(|s| format!(" Criterion: {}", store::safe(s, 240)))
                        .unwrap_or_default(),
                    t.cancellation
                        .as_ref()
                        .map(|c| format!(" Reason: {}", store::safe(&c.reason, 240)))
                        .unwrap_or_default()
                ));
            }
        }
        View::Results => {
            if let Some(i) = index {
                result_rows(&mut value.rows, reference, &m.child(i).result);
            } else {
                result_rows(&mut value.rows, &m.id, &m.result);
                for t in m.children() {
                    value.rows.push(format!(
                        "{} — {}",
                        task_brief(m, t),
                        t.result
                            .as_ref()
                            .map(|r| format!(
                                "result: {}; get_context ref={}/{} view=results for full evidence.",
                                store::safe(&r.summary, 160),
                                m.id,
                                t.id
                            ))
                            .unwrap_or("result not reported".into())
                    ));
                }
            }
            for reason in m
                .reasons
                .iter()
                .filter(|r| index.is_none() || r.target == reference)
            {
                value.rows.push(format!(
                    "{} {} by {}: {}",
                    reason.at,
                    reason.action,
                    reason
                        .actor
                        .as_ref()
                        .map(|s| store::safe(s, 128))
                        .unwrap_or("unknown".into()),
                    store::safe(&reason.reason, 512)
                ));
            }
        }
        View::Checks => {
            if let Some(i) = index {
                check_rows(
                    &mut value.rows,
                    reference,
                    &m.child(i).required_checks,
                    &m.child(i).checks,
                );
            } else {
                check_rows(&mut value.rows, &m.id, &m.required_checks, &m.checks);
                for t in m.children() {
                    check_rows(
                        &mut value.rows,
                        &format!("{}/{}", m.id, t.id),
                        &t.required_checks,
                        &t.checks,
                    );
                }
            }
        }
        View::Review => {
            if child.is_some_and(|a| a.id.starts_with("T-"))
                || m.id.starts_with("A-") && !m.modern()
            {
                return Err(Error::new(
                    "invalid_arguments",
                    "Tasks/Atomics have no separate review; read their module with view=review.",
                ));
            }
            let selected = args.review_index.or_else(|| reviews.len().checked_sub(1));
            if let Some(i) = selected {
                let r = reviews.get(i).ok_or_else(|| {
                    Error::new(
                        "invalid_arguments",
                        "Review index is outside retained history.",
                    )
                })?;
                value.lines.push(format!(
                    "Review {i} of {}: {} at {} by {}.",
                    reviews.len(),
                    verdict(r.verdict),
                    r.at,
                    r.reviewer
                        .as_ref()
                        .map(|s| store::safe(s, 128))
                        .unwrap_or("unknown".into())
                ));
                value
                    .lines
                    .push(format!("Conclusion: {}", store::safe(&r.summary, 1024)));
                value.lines.push(format!(
                    "Applicability: {}; older reports remain history.",
                    if child.map_or_else(
                        || r.epoch == m.review_epoch
                            && store.work_basis(m).is_ok_and(|b| b == r.basis),
                        |a| a
                            .atomic_workflow
                            .as_ref()
                            .is_some_and(|w| r.epoch == w.review_epoch)
                            && a.atomic_basis().is_ok_and(|b| b == r.basis)
                    ) {
                        "current"
                    } else {
                        "historical/stale"
                    }
                ));
                for f in &r.findings {
                    value.rows.push(format!(
                        "Finding [{}]: {}",
                        if f.must_fix { "must fix" } else { "advisory" },
                        store::safe(&f.text, 256)
                    ));
                }
                for c in &r.check_updates {
                    value.rows.push(format!(
                        "{} {}: {} → {}. Before actor/time: {}; after: {} / {}. Detail: {}",
                        c.target,
                        store::safe(&c.label, 64),
                        c.before
                            .as_ref()
                            .map(|c| check_status(c.status))
                            .unwrap_or("not reported"),
                        check_status(c.after.status),
                        c.before
                            .as_ref()
                            .map(|c| format!(
                                "{} / {}",
                                c.actor
                                    .as_ref()
                                    .map(|s| store::safe(s, 128))
                                    .unwrap_or("unknown".into()),
                                c.reported_at
                            ))
                            .unwrap_or("none".into()),
                        c.after
                            .actor
                            .as_ref()
                            .map(|s| store::safe(s, 128))
                            .unwrap_or("unknown".into()),
                        c.after.reported_at,
                        c.after
                            .detail
                            .as_ref()
                            .map(|s| store::safe(s, 256))
                            .unwrap_or("none".into())
                    ));
                }
            } else {
                value.lines.push("No review recorded.".into());
            }
        }
        View::Commits => {
            for source in &m.imports {
                if index.is_some() && !source.targets.iter().any(|t| t == reference) {
                    continue;
                }
                let c = &source.commit;
                value.rows.push(format!("Commit {} — {} — author {} at {} — targets {} — repository {} — observed checkout {}",c.sha,store::safe(&c.subject,256),store::safe(&c.author,256),c.authored_at,source.targets.join(", "),store::safe(&c.repository,1024),store::safe(&c.worktree,1024)));
                value.rows.push(format!(
                    "Imported Result: {}",
                    store::safe(&c.summary, 1024)
                ));
                for part in message_chunks(&c.message) {
                    value
                        .rows
                        .push(format!("Original message: {}", store::safe(part, 768)));
                }
            }
        }
        View::Log => {
            value.lines.push(format!("Recent generated log: {} retained; {} earlier machine events omitted; retained range {} to {}. Human reports/reviews/reasons are not evicted.",
                m.log.len(),m.omitted_log_entries,m.log.first().map(|e|e.id.as_str()).unwrap_or("none"),m.log.last().map(|e|e.id.as_str()).unwrap_or("none")));
            for e in m
                .log
                .iter()
                .filter(|e| index.is_none() || e.target == reference)
            {
                value.rows.push(format!(
                    "{} {} {}: {} by {}",
                    e.id,
                    e.at,
                    e.target,
                    e.action,
                    e.actor
                        .as_ref()
                        .map(|s| store::safe(s, 128))
                        .unwrap_or("unknown".into())
                ));
            }
        }
    }
    if value.rows.is_empty() {
        let empty = match args.view {
            View::Summary => "No additional acceptance conditions or review-detail rows.",
            View::Review if !m.reviews.is_empty() => {
                "This review has no findings or check updates."
            }
            View::Review => "No review recorded.",
            View::Tasks => "No tasks in this module.",
            View::Checks => "No required or reported checks in this scope.",
            View::Log => "No retained generated events in this scope.",
            View::Results => "No result-detail rows in this scope.",
            View::Commits => "No retained imported commit observations in this scope.",
        };
        value.lines.push(empty.into());
    }
    render_page(value, args.start, args.limit, true, templates)
}

/// Bind tool/ref/view/query/review selection and data snapshot, separately from editable file versions.
fn scope_version(selection: &str, version: &str, scan: &str) -> String {
    let mut digest = Sha256::new();
    for value in ["agent-tasks/view/v1", selection, version, scan] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Count task lifecycle over healthy tracked work only.
fn count(module: &Module, state: TaskState) -> usize {
    module.tasks.iter().filter(|t| t.state == state).count()
}
/// Present a reported lead and handle without an agent-liveness claim.
fn lead_line(m: &Module) -> String {
    m.lead
        .as_ref()
        .map(|l| {
            format!(
                "Lead: {}{}",
                store::safe(&l.name, 128),
                l.handle
                    .as_ref()
                    .map(|h| format!("; handle (reported): {}", store::safe(h, 256)))
                    .unwrap_or_default()
            )
        })
        .unwrap_or("Lead: unknown".into())
}
/// Human-oriented kind-specific row with current progress, ownership and reported actor.
fn module_brief(store: &Store, m: &Module) -> String {
    let owner = match store.parent(&m.id) {
        Ok(Some(p)) => format!("Epic-owned {}", p.id),
        Ok(None) => "standalone".into(),
        Err(_) => "ownership unknown".into(),
    };
    if m.id.starts_with("E-") {
        let accepted = m
            .modules
            .iter()
            .filter(|id| {
                store
                    .module(id)
                    .is_ok_and(|c| store.phase(&c.value) == "accepted")
            })
            .count();
        let done = m
            .atomic_members
            .iter()
            .filter(|id| {
                store
                    .module(id)
                    .is_ok_and(|c| matches!(store.phase(&c.value), "done" | "accepted"))
            })
            .count();
        return format!(
            "{} {} — {} — Modules {accepted}/{} accepted; Atomics {done}/{} done; {} — last reported: {}",
            m.id,
            store::safe(&m.title, 120),
            store.phase(m),
            m.modules.len(),
            m.atomic_members.len(),
            lead_line(m),
            m.updated_at
        );
    }
    if m.id.starts_with("A-") {
        return format!(
            "{} {} — {} — {}; {owner}; participants: {} — last reported: {}",
            m.id,
            store::safe(&m.title, 120),
            store.phase(m),
            lead_line(m),
            m.participants.join(", "),
            m.updated_at
        );
    }
    let obligations = m
        .workflow
        .as_ref()
        .map(|w| {
            format!(
                "; provides {}; consumes {}; waits {}",
                w.contracts
                    .as_ref()
                    .map(|c| c
                        .provides
                        .iter()
                        .map(|c| format!("{}: {}", c.peer, store::safe(&c.description, 80)))
                        .collect::<Vec<_>>()
                        .join("; "))
                    .unwrap_or_else(|| "unknown".into()),
                w.contracts
                    .as_ref()
                    .map(|c| c
                        .consumes
                        .iter()
                        .map(|c| format!("{}: {}", c.peer, store::safe(&c.description, 80)))
                        .collect::<Vec<_>>()
                        .join("; "))
                    .unwrap_or_else(|| "unknown".into()),
                w.dependencies
                    .iter()
                    .map(|d| d.reference.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .unwrap_or_default();
    format!(
        "{} {} — {} — {}/{} tasks done; {} open; {} canceled; Atomics {}/{} done, {} open, {} canceled; {} — {owner}; last reported: {}{}",
        m.id,
        store::safe(&m.title, 120),
        store.phase(m),
        count(m, TaskState::Done),
        m.tasks.len(),
        count(m, TaskState::Open),
        count(m, TaskState::Canceled),
        m.atomics
            .iter()
            .filter(|a| a.state == TaskState::Done)
            .count(),
        m.atomics.len(),
        m.atomics
            .iter()
            .filter(|a| a.state == TaskState::Open)
            .count(),
        m.atomics
            .iter()
            .filter(|a| a.state == TaskState::Canceled)
            .count(),
        lead_line(m),
        m.updated_at,
        obligations
    )
}
/// Compact embedded work row preserves identity/state/checks and declared Atomic executor.
fn task_brief(m: &Module, t: &Task) -> String {
    let executor = t
        .executor
        .as_ref()
        .map(|e| {
            format!(
                "; executor: {}{}",
                store::safe(&e.name, 128),
                e.handle
                    .as_ref()
                    .map(|h| format!("; handle (reported): {}", store::safe(h, 256)))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_default();
    format!(
        "{}/{} {} — {} — checks: {} reported / {} required{}",
        m.id,
        t.id,
        store::safe(&t.title, 120),
        match t.state {
            TaskState::Open => "open",
            TaskState::Done if t.id.starts_with("A-") => t.atomic_phase(),
            TaskState::Done => "done",
            TaskState::Canceled => "canceled",
        },
        t.checks.len(),
        t.required_checks.len(),
        executor
    )
}
/// Expand a target's current substance into real pageable rows.
fn result_rows(rows: &mut Vec<String>, target: &str, report: &Option<Report>) {
    if let Some(r) = report {
        rows.push(format!(
            "{target} result: {} — author {} at {}",
            store::safe(&r.summary, 1024),
            r.actor
                .as_ref()
                .map(|s| store::safe(s, 128))
                .unwrap_or("unknown".into()),
            r.reported_at
        ));
        for (label, values) in [
            ("Gap", &r.gaps),
            ("Followup", &r.followups),
            ("Artifact (reported)", &r.artifacts),
        ] {
            for value in values {
                rows.push(format!("{target} {label}: {}", store::safe(value, 256)));
            }
        }
    } else {
        rows.push(format!("{target}: no target-owned result; module context may derive progress from task reports."));
    }
}
/// Expand required omissions and current evidence without converting absence into pass.
fn check_rows(rows: &mut Vec<String>, target: &str, required: &[String], checks: &[Check]) {
    for label in required {
        if !checks.iter().any(|c| &c.label == label) {
            rows.push(format!(
                "{target} required {}: NOT REPORTED",
                store::safe(label, 64)
            ));
        }
    }
    for c in checks {
        rows.push(format!(
            "{target} {} [{}]: {} — {} — reporter {} at {}",
            store::safe(&c.label, 64),
            if required.contains(&c.label) {
                "required"
            } else {
                "reported"
            },
            check_status(c.status),
            c.detail
                .as_ref()
                .map(|s| store::safe(s, 256))
                .unwrap_or("no detail".into()),
            c.actor
                .as_ref()
                .map(|s| store::safe(s, 128))
                .unwrap_or("unknown".into()),
            c.reported_at
        ));
    }
}
/// Stable English check label for compact text.
fn check_status(value: CheckStatus) -> &'static str {
    match value {
        CheckStatus::Passed => "passed",
        CheckStatus::Failed => "failed",
        CheckStatus::NotRun => "not_run",
        CheckStatus::NotApplicable => "not_applicable",
    }
}
/// Stable English verdict without raw enums or serialization.
fn verdict(value: Verdict) -> &'static str {
    match value {
        Verdict::Accepted => "accepted",
        Verdict::ChangesRequested => "changes requested",
    }
}

/// Return one owner-ready status: current tracked work and declared actors, no runtime polling.
pub fn status(config: &Config, args: StatusArgs, templates: &Templates) -> Result<String> {
    let store = config.resolve(&args.project)?;
    let _lock = store.lock(false, &mut Vec::new())?;
    let project = store.project()?.ok_or_else(|| {
        Error::new(
            "not_initialized",
            "Project has no manifest; inspect get_context before explicit initialization.",
        )
    })?;
    let scan = store.scan(args.module.as_deref())?;
    let mut value = page(
        format!(
            "Project status — {}",
            store::safe(&project.value.title, 256)
        ),
        String::new(),
    );
    value.coverage = if scan.complete { "complete" } else { "PARTIAL" }.into();
    value.lines.push(
        args.module
            .as_ref()
            .map(|id| format!("Scope: module {id}; project-wide totals are excluded."))
            .unwrap_or("Scope: all tracked project work within the disclosed read bounds.".into()),
    );
    let modules = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("M-"))
        .count();
    let done = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("M-") && store.phase(&m.value) == "accepted")
        .count();
    let tasks: usize = scan.modules.iter().map(|m| m.value.tasks.len()).sum();
    let counts = |s| {
        scan.modules
            .iter()
            .map(|m| count(&m.value, s))
            .sum::<usize>()
    };
    value.lines.push(format!("Modules: {done} accepted / {modules} readable. Tasks: {} done / {tasks} readable; {} open; {} canceled.{}",
        counts(TaskState::Done),counts(TaskState::Open),counts(TaskState::Canceled),if scan.complete {""}else{" Counts are lower bounds; unreadable work is unknown."}));
    let epics = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("E-"))
        .count();
    let accepted_epics = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("E-") && store.phase(&m.value) == "accepted")
        .count();
    let root_atomics = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("A-"))
        .count();
    let embedded_atomics: usize = scan.modules.iter().map(|m| m.value.atomics.len()).sum();
    let done_atomics = scan
        .modules
        .iter()
        .filter(|m| {
            m.value.id.starts_with("A-") && matches!(store.phase(&m.value), "done" | "accepted")
        })
        .count()
        + scan
            .modules
            .iter()
            .map(|m| {
                m.value
                    .atomics
                    .iter()
                    .filter(|a| matches!(a.atomic_phase(), "done" | "accepted"))
                    .count()
            })
            .sum::<usize>();
    let local_atomics = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("A-") && m.value.completed)
        .count()
        + scan
            .modules
            .iter()
            .map(|m| {
                m.value
                    .atomics
                    .iter()
                    .filter(|a| a.state == TaskState::Done)
                    .count()
            })
            .sum::<usize>();
    value.lines.push(format!("Atomic local outcomes: {local_atomics} done; final current closure: {done_atomics}. Local completion alone is not modern Atomic acceptance."));
    let canceled_atomics = scan
        .modules
        .iter()
        .filter(|m| m.value.id.starts_with("A-") && m.value.state == ModuleState::Canceled)
        .count()
        + scan
            .modules
            .iter()
            .map(|m| {
                m.value
                    .atomics
                    .iter()
                    .filter(|a| a.state == TaskState::Canceled)
                    .count()
            })
            .sum::<usize>();
    value.lines.push(format!("Epics: {accepted_epics} accepted / {epics} readable. Atomics: {done_atomics} done / {} readable; {canceled_atomics} canceled; {} unfinished/stale. Each item counted once.",root_atomics+embedded_atomics,root_atomics+embedded_atomics-done_atomics-canceled_atomics));
    value.lines.push(format!(
        "Purpose: {}",
        store::safe(&project.value.purpose, 240)
    ));
    value.lines.push("Last activity is reported, not live agent presence. Artifacts/checks are agent reports, not independently queried external facts.".into());
    value.lines.push("More detail: get_context ref=M-001 view=tasks/results/checks/review. Narrow this status with module=M-001.".into());
    warnings(&mut value, &scan.warnings);
    warnings(&mut value, &scan.unreadable);
    for snapshot in &scan.modules {
        let m = &snapshot.value;
        value.rows.push(module_brief(&store, m));
        value.rows.push(format!(
            "{} expected: {} — current summary: {}",
            m.id,
            store::safe(&m.outcome, 140),
            m.result
                .as_ref()
                .map(|r| store::safe(&r.summary, 160))
                .unwrap_or_else(|| if m.children().any(|t| t.result.is_some()) {
                    "derived from task reports below".into()
                } else {
                    "not reported".into()
                })
        ));
        if let Some(b) = &m.blocker {
            value.rows.push(format!(
                "{} BLOCKER {} — needs {} — resolver {}",
                m.id,
                store::safe(&b.problem, 192),
                store::safe(&b.needed_action, 192),
                b.resolver
                    .as_ref()
                    .map(|s| store::safe(s, 128))
                    .unwrap_or("unknown".into())
            ));
        }
        if let Some(r) = m.reviews.last() {
            value.rows.push(format!(
                "{} latest review: {} by {} — {} ({} findings).",
                m.id,
                verdict(r.verdict),
                r.reviewer
                    .as_ref()
                    .map(|s| store::safe(s, 128))
                    .unwrap_or("unknown".into()),
                store::safe(&r.summary, 140),
                r.findings.len()
            ));
        }
        let task_limit = if args.module.is_some() { MAX_TASKS } else { 4 };
        for t in m.children().take(task_limit) {
            value.rows.push(format!(
                "{} — {}",
                task_brief(m, t),
                t.result
                    .as_ref()
                    .map(|r| store::safe(&r.summary, 120))
                    .unwrap_or("result not reported".into())
            ));
        }
        if m.tasks.len() + m.atomics.len() > task_limit {
            value.detail_coverage = "PARTIAL".into();
            value.rows.push(format!("{}: {} task details omitted; counts include them. Use module={} or get_context view=tasks.",m.id,m.tasks.len() + m.atomics.len() - task_limit,m.id));
        }
        if let Err(e) = m.counters() {
            value
                .rows
                .push(format!("{} writes blocked: {}", m.id, store::safe(&e, 180)));
        }
    }
    for issue in scan.unreadable {
        value
            .rows
            .push(format!("UNREADABLE {}", store::safe(&issue, 240)));
    }
    render_page(value, 0, usize::MAX, false, templates)
}

/// Search hit projection retaining matched field names, score and numeric order.
struct Hit {
    /// Owned reference or Project.
    reference: String,
    /// Human title.
    title: String,
    /// Count of semantic fields containing terms.
    score: usize,
    /// Stable numeric module/task ordering, never filename lexicographic order.
    order: (u64, u64),
    /// Matching field labels.
    fields: Vec<String>,
    /// Bounded relevant excerpt.
    excerpt: String,
}

/// Add one semantic target when every term matches at least one of its fields.
fn hit(
    hits: &mut Vec<Hit>,
    reference: String,
    title: &str,
    order: (u64, u64),
    fields: Vec<(&str, String)>,
    terms: &[String],
) {
    let lower: Vec<_> = fields.iter().map(|(_, v)| v.to_lowercase()).collect();
    if !terms
        .iter()
        .all(|term| lower.iter().any(|field| field.contains(term)))
    {
        return;
    }
    let matched: Vec<_> = fields
        .iter()
        .zip(lower)
        .filter(|(_, v)| terms.iter().any(|t| v.contains(t)))
        .map(|((label, value), _)| (label, value))
        .collect();
    let Some((_, excerpt)) = matched.first() else {
        return;
    };
    hits.push(Hit {
        reference,
        title: title.into(),
        score: matched.len(),
        order,
        fields: matched.iter().map(|(label, _)| (**label).into()).collect(),
        excerpt: store::safe(excerpt, 220),
    });
}

/// Current result/check text contributes search evidence; technical metadata and hashes do not.
fn evidence_fields(
    fields: &mut Vec<(&'static str, String)>,
    result: &Option<Report>,
    checks: &[Check],
) {
    if let Some(r) = result {
        fields.push(("result", r.summary.clone()));
        fields.push(("gaps", r.gaps.join(" ")));
        fields.push(("followups", r.followups.join(" ")));
        fields.push(("artifacts", r.artifacts.join(" ")));
    }
    for c in checks {
        fields.push((
            "check",
            format!("{} {}", c.label, c.detail.as_deref().unwrap_or_default()),
        ));
    }
}

/// Bounded Unicode lowercase all-term search; exact scope snapshot protects continuation.
pub fn search(config: &Config, args: SearchArgs, templates: &Templates) -> Result<String> {
    text(&args.query, 256).map_err(store::invalid)?;
    let terms: Vec<_> = args
        .query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    if terms.is_empty() || terms.len() > 8 {
        return Err(Error::new(
            "invalid_arguments",
            "Search accepts one to eight whitespace-separated words.",
        ));
    }
    let store = config.resolve(&args.project)?;
    let _lock = store.lock(false, &mut Vec::new())?;
    let project = store.project()?.ok_or_else(|| {
        Error::new(
            "not_initialized",
            "Initialize the project before work search.",
        )
    })?;
    let scan = store.scan(args.module.as_deref())?;
    let selection = format!("search:{:?}:{terms:?}", args.module);
    let snapshot = scope_version(&selection, &project.version, &scan.version);
    continuation(args.start, args.limit, args.version.as_deref(), &snapshot)?;
    let mut hits = Vec::new();
    if args.module.is_none() {
        hit(
            &mut hits,
            "Project".into(),
            &project.value.title,
            (0, 0),
            vec![
                ("title", project.value.title.clone()),
                ("purpose", project.value.purpose.clone()),
            ],
            &terms,
        );
    }
    for snapshot in &scan.modules {
        let m = &snapshot.value;
        let n = work_number(&m.id).map(|(_, n)| n).map_err(store::invalid)?;
        let mut fields = vec![
            ("title", m.title.clone()),
            ("outcome", m.outcome.clone()),
            ("required_checks", m.required_checks.join(" ")),
        ];
        if let Some(l) = &m.lead {
            fields.push((
                "lead",
                format!("{} {}", l.name, l.handle.as_deref().unwrap_or_default()),
            ));
        }
        if let Some(b) = &m.blocker {
            fields.push(("blocker", format!("{} {}", b.problem, b.needed_action)));
        }
        if let Some(h) = &m.handoff {
            fields.push(("handoff", format!("{} {}", h.stopping_point, h.next_action)));
        }
        if let Some(c) = &m.cancellation {
            fields.push(("cancellation", c.reason.clone()));
        }
        fields.push(("criteria", m.criteria.join(" ")));
        fields.push((
            "members",
            m.modules
                .iter()
                .chain(&m.atomic_members)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        ));
        fields.push(("participants", m.participants.join(" ")));
        if let Some(w) = &m.workflow {
            if let Some(e) = &w.execution {
                fields.push((
                    "execution",
                    format!(
                        "{} {} {} {}",
                        e.repository, e.worktree, e.branch, e.target_branch
                    ),
                ));
            }
            if let Some(c) = &w.contracts {
                for contract in &c.provides {
                    fields.push((
                        "provides",
                        format!(
                            "{} {} {}",
                            contract.peer,
                            contract.description,
                            contract.reference.as_deref().unwrap_or_default()
                        ),
                    ));
                }
                for contract in &c.consumes {
                    fields.push((
                        "consumes",
                        format!(
                            "{} {} {}",
                            contract.peer,
                            contract.description,
                            contract.reference.as_deref().unwrap_or_default()
                        ),
                    ));
                }
            }
            for d in &w.dependencies {
                fields.push((
                    "dependency",
                    format!("{} {:?} {}", d.reference, d.condition, d.reason),
                ));
            }
            if let Some(d) = &w.delivery {
                fields.push(("delivery", format!("{} {}", d.target_branch, d.summary)));
            }
            if let Some(e) = &w.environment {
                fields.push(("environment", e.clone()));
            }
            fields.push(("scenarios", w.scenarios.join(" ")));
        }
        evidence_fields(&mut fields, &m.result, &m.checks);
        for r in &m.reviews {
            fields.push(("review", r.summary.clone()));
            for f in &r.findings {
                fields.push(("finding", f.text.clone()));
            }
        }
        for r in &m.reasons {
            fields.push(("reason", r.reason.clone()));
        }
        hit(&mut hits, m.id.clone(), &m.title, (n, 0), fields, &terms);
        for t in m.children() {
            let mut fields = vec![
                ("title", t.title.clone()),
                ("criterion", t.criterion.clone().unwrap_or_default()),
                ("required_checks", t.required_checks.join(" ")),
            ];
            if let Some(c) = &t.cancellation {
                fields.push(("cancellation", c.reason.clone()));
            }
            if let Some(executor) = &t.executor {
                fields.push((
                    "executor",
                    format!(
                        "{} {}",
                        executor.name,
                        executor.handle.as_deref().unwrap_or_default()
                    ),
                ));
            }
            if let Some(w) = &t.atomic_workflow {
                for review in &w.reviews {
                    fields.push(("review", review.summary.clone()));
                }
            }
            evidence_fields(&mut fields, &t.result, &t.checks);
            hit(
                &mut hits,
                format!("{}/{}", m.id, t.id),
                &t.title,
                (
                    n,
                    number(&t.id, if t.id.starts_with("A-") { "A-" } else { "T-" })
                        .map_err(store::invalid)?,
                ),
                fields,
                &terms,
            );
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.order.cmp(&b.order)));
    let mut value = page(
        format!("Work search — {}", store::safe(&args.query, 256)),
        snapshot,
    );
    value.coverage = if scan.complete { "complete" } else { "PARTIAL" }.into();
    value.lines.push(format!("{} matches in readable tracked work. All terms must match; ranked by matching fields, then numeric reference.",hits.len()));
    value.lines.push(
        args.module
            .as_ref()
            .map(|id| format!("Scope: module {id}."))
            .unwrap_or("Scope: tracked project work.".into()),
    );
    value.lines.push("Open Module/Task hits with get_context ref=<reference>. For a Project hit, call get_context with project only and omit ref. Select results/checks/review for full evidence.".into());
    warnings(&mut value, &scan.warnings);
    warnings(&mut value, &scan.unreadable);
    for h in hits {
        value.rows.push(format!(
            "{} {} — fields: {}{} — excerpt: {}",
            h.reference,
            store::safe(&h.title, 100),
            h.fields
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            if h.fields.len() > 3 {
                format!(" (+{} fields)", h.fields.len() - 3)
            } else {
                String::new()
            },
            h.excerpt
        ));
    }
    render_page(value, args.start, args.limit, true, templates)
}

/// Split retained human commit text at UTF-8 boundaries so exact source can be paged under response budgets.
fn message_chunks(message: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    while start < message.len() {
        let mut end = (start + 768).min(message.len());
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&message[start..end]);
        start = end;
    }
    parts
}
