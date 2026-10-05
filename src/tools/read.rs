//! Semantic bounded retrieval. Coverage, unknown data and real pagination remain explicit.
use super::{
    input::{ContextArgs, SearchArgs, StatusArgs, View},
    work::{Page, module_id},
};
use crate::{
    model::*,
    response::Templates,
    store::{self, Config, Error, Result, Snapshot},
};
use sha2::{Digest, Sha256};

/// New typed page with explicit data and detail coverage; empty does not mean unreadable.
fn page(heading: String, version: String) -> Page {
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
fn continuation(start: usize, limit: usize, expected: Option<&str>, snapshot: &str) -> Result<()> {
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
fn render_page(
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
                        .is_ok_and(|s| s.schema_version == SCHEMA && s.next_module == 1)
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
                let valid=store::decode::<Allocator>(&bytes).is_ok_and(|s|s.schema_version==SCHEMA && s.next_module>scan.modules.iter().filter_map(|m|number(&m.value.id,"M-").ok()).max().unwrap_or(0) && s.next_module<u64::MAX);
                if !valid {value.lines.push("Module allocation blocked: invalid allocator. Restore retained state; healthy reads remain available.".into());}
            },
            Ok(None)=>value.lines.push("Allocator absent: healthy work remains readable; new module allocation waits for valid state or explicit empty init.".into()),
            Err(e)=>value.lines.push(format!("Allocator unreadable: {}",store::safe(&e.message,200))),
        }
        warnings(&mut value, &scan.warnings);
        for m in scan.modules {
            value.rows.push(module_brief(&m.value));
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
    let read_version = scope_version(&selection, &snapshot.version, "");
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
                index.map_or(m.title.as_str(), |i| m.tasks[i].title.as_str()),
                256
            )
        ),
        snapshot.version.clone(),
    );
    value.snapshot_version = read_version;
    diagnostics(&mut value, &snapshot);
    if !matches!(args.view, View::Review) && args.review_index.is_some() {
        return Err(Error::new(
            "invalid_arguments",
            "review_index belongs only to view=review.",
        ));
    }
    match args.view {
        View::Summary => {
            value.lines.push(format!(
                "Module phase: {}; Task counts: {} done, {} open, {} canceled.",
                m.phase(),
                count(m, TaskState::Done),
                count(m, TaskState::Open),
                count(m, TaskState::Canceled)
            ));
            value
                .lines
                .push(format!("Outcome: {}", store::safe(&m.outcome, 1024)));
            value.lines.push(lead_line(m));
            value.lines.push(format!(
                "Last managed report/change: {}; activity is reported, not live agent state.",
                m.updated_at
            ));
            if let Some(i) = index {
                let t = &m.tasks[i];
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
            let missing = m.acceptance();
            value.lines.push(format!("Acceptance conditions remaining: {}. Evidence is reported; no Git/GitHub verification.",missing.len()));
            for condition in missing {
                value
                    .rows
                    .push(format!("Needs: {}", store::safe(&condition, 320)));
            }
            if let Some(review) = m.reviews.last() {
                value.rows.push(format!(
                    "Latest review: {} by {} — {} (history: {}). Use view=review.",
                    verdict(review.verdict),
                    review
                        .reviewer
                        .as_ref()
                        .map(|s| store::safe(s, 128))
                        .unwrap_or("unknown".into()),
                    store::safe(&review.summary, 180),
                    m.reviews.len()
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
            for t in &m.tasks {
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
                result_rows(&mut value.rows, reference, &m.tasks[i].result);
            } else {
                result_rows(&mut value.rows, &m.id, &m.result);
                for t in &m.tasks {
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
                    &m.tasks[i].required_checks,
                    &m.tasks[i].checks,
                );
            } else {
                check_rows(&mut value.rows, &m.id, &m.required_checks, &m.checks);
                for t in &m.tasks {
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
            if index.is_some() {
                return Err(Error::new(
                    "invalid_arguments",
                    "Tasks have no separate review; read their module with view=review.",
                ));
            }
            let selected = args.review_index.or_else(|| m.reviews.len().checked_sub(1));
            if let Some(i) = selected {
                let r = m.reviews.get(i).ok_or_else(|| {
                    Error::new(
                        "invalid_arguments",
                        "Review index is outside retained history.",
                    )
                })?;
                value.lines.push(format!(
                    "Review {i} of {}: {} at {} by {}.",
                    m.reviews.len(),
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
                    if r.epoch == m.review_epoch && m.basis().is_ok_and(|b| b == r.basis) {
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
        value.lines.push("No entries in this view.".into());
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
/// Human-oriented module row with explicit current counts and reported activity.
fn module_brief(m: &Module) -> String {
    format!(
        "{} {} — {} — {}/{} tasks done; {} open; {} canceled; {} — last reported: {}",
        m.id,
        store::safe(&m.title, 120),
        m.phase(),
        count(m, TaskState::Done),
        m.tasks.len(),
        count(m, TaskState::Open),
        count(m, TaskState::Canceled),
        lead_line(m),
        m.updated_at
    )
}
/// Compact task row retains reference/state and routes detailed evidence separately.
fn task_brief(m: &Module, t: &Task) -> String {
    format!(
        "{}/{} {} — {} — checks: {} reported / {} required",
        m.id,
        t.id,
        store::safe(&t.title, 120),
        match t.state {
            TaskState::Open => "open",
            TaskState::Done => "done",
            TaskState::Canceled => "canceled",
        },
        t.checks.len(),
        t.required_checks.len()
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
    let modules = scan.modules.len();
    let done = scan
        .modules
        .iter()
        .filter(|m| m.value.phase() == "accepted")
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
        value.rows.push(module_brief(m));
        value.rows.push(format!(
            "{} expected: {} — current summary: {}",
            m.id,
            store::safe(&m.outcome, 140),
            m.result
                .as_ref()
                .map(|r| store::safe(&r.summary, 160))
                .unwrap_or_else(|| if m.tasks.iter().any(|t| t.result.is_some()) {
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
        for t in m.tasks.iter().take(task_limit) {
            value.rows.push(format!(
                "{} — {}",
                task_brief(m, t),
                t.result
                    .as_ref()
                    .map(|r| store::safe(&r.summary, 120))
                    .unwrap_or("result not reported".into())
            ));
        }
        if m.tasks.len() > task_limit {
            value.detail_coverage = "PARTIAL".into();
            value.rows.push(format!("{}: {} task details omitted; counts include them. Use module={} or get_context view=tasks.",m.id,m.tasks.len()-task_limit,m.id));
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
        let n = number(&m.id, "M-").map_err(store::invalid)?;
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
        for t in &m.tasks {
            let mut fields = vec![
                ("title", t.title.clone()),
                ("criterion", t.criterion.clone().unwrap_or_default()),
                ("required_checks", t.required_checks.join(" ")),
            ];
            if let Some(c) = &t.cancellation {
                fields.push(("cancellation", c.reason.clone()));
            }
            evidence_fields(&mut fields, &t.result, &t.checks);
            hit(
                &mut hits,
                format!("{}/{}", m.id, t.id),
                &t.title,
                (n, number(&t.id, "T-").map_err(store::invalid)?),
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
    value.lines.push("Read a match with get_context ref=<reference>; select results/checks/review for its full evidence.".into());
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
