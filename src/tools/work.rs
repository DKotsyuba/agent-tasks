//! Purpose-based work tools: guarded planning, current evidence, whole-record review and bounded retrieval.
use super::input::{self, Common, Completion, Plan, ReviewArgs, ReviewWorkArgs, TaskInput, Work};
use super::{compaction_ops, document_ops, knowledge_ops, recovery_ops};
use crate::{
    model::*,
    persist,
    response::Templates,
    store::{self, Config, Error, LockGuard, Result, Snapshot, Store},
};
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;
use serde_json::Value;

/// Confirmed mutation receipt, captured before presentation. It never replays an effect.
#[derive(Clone, Serialize)]
pub struct Ack {
    /// Target or project reference.
    pub target: String,
    /// Current file or creation precondition version.
    pub version: String,
    /// Derived current phase, never externally verified delivery.
    pub phase: String,
    /// Owning record label including its own noun (`Module phase`, `Decision state`); embedded
    /// children always report their Module phase. The template prints it before the colon.
    pub phase_label: &'static str,
    /// Whether the call changed any business record.
    pub changed: bool,
    /// Producer notes (applied actions, blocked kind, warnings). Producers supply at most eight
    /// plain lines of 200 bytes each; the presenter sanitizes control characters, cuts longer
    /// lines and adds an explicit omitted count for any excess, never silently.
    pub notes: Vec<String>,
    /// Affected canonical references. Producers supply at most sixteen of 256 bytes each; the
    /// presenter drops any reference over the bound rather than truncating an identity.
    /// Advisory labels, never publication proof.
    pub refs: Vec<String>,
}

/// Most producer note lines shown in one reply; excess is counted, not hidden.
const MAX_NOTES: usize = 8;
/// Most bytes of one producer note line.
const MAX_NOTE_BYTES: usize = 200;
/// Most affected references one acknowledgement carries.
const MAX_REFS: usize = 16;
/// Most bytes of one affected reference.
const MAX_REF_BYTES: usize = 256;

/// Closed kind of tool whose acknowledgement is presented; selects the layout and the route.
///
/// The presenter chooses from this kind and never from a target string or a template name that a
/// producer or a user supplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolKind {
    /// `plan_work`, `record_work`, `review_work` and `review_module`.
    Work,
    /// `knowledge_work`.
    Knowledge,
    /// `document_work`.
    Document,
    /// `compaction_work`.
    Compaction,
    /// `git_recovery`, the one tool whose target is a display label, not a canonical reference.
    GitRecovery,
}

/// Whether `target` is a canonical reference that `get_context` can open: a work, knowledge,
/// document or compaction ID with an optional child (`M-001/T-001`, `CL-001/I-001`) or a
/// managed Markdown path (`README.md` or `docs/...md`, ASCII, no `..`).
pub(super) fn is_canonical_ref(target: &str) -> bool {
    let numbered = |part: &str, prefixes: &[&str]| {
        prefixes.iter().any(|prefix| {
            part.strip_prefix(prefix).is_some_and(|digits| {
                digits.len() >= 3 && digits.bytes().all(|b| b.is_ascii_digit())
            })
        })
    };
    let mut parts = target.split('/');
    let head = parts.next().unwrap_or_default();
    if numbered(
        head,
        &["E-", "M-", "A-", "D-", "RB-", "RS-", "CL-", "DOC-", "CP-"],
    ) {
        return parts.all(|child| numbered(child, &["T-", "A-", "I-"])) && target.len() <= 64;
    }
    target.len() <= MAX_REF_BYTES
        && target.is_ascii()
        && (target == "README.md" || target.starts_with("docs/"))
        && target.ends_with(".md")
        && !target.contains("..")
        && !target.chars().any(|c| c.is_control() || c == '\\')
}

/// One plain line of producer text: control characters become spaces, ends are trimmed and the
/// text is cut at a character boundary within `cap` bytes.
fn plain_line(text: &str, cap: usize) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = line.trim();
    let mut end = line.len().min(cap);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].trim_end().to_owned()
}

impl Ack {
    /// Bound producer text before presentation.
    ///
    /// Notes keep their first eight lines, sanitized and cut to 200 bytes; excess becomes one
    /// explicit `N notes omitted; <route>` line. References over 256 bytes or containing control
    /// characters are dropped and only the first sixteen are kept. `route` is the reply's own
    /// read route, so the omitted line never invents one.
    fn bounded(mut self, route: &str) -> Ack {
        let omitted = self.notes.len().saturating_sub(MAX_NOTES);
        self.notes = self
            .notes
            .iter()
            .take(MAX_NOTES)
            .map(|note| plain_line(note, MAX_NOTE_BYTES))
            .filter(|note| !note.is_empty())
            .collect();
        if omitted > 0 {
            self.notes.push(format!("{omitted} notes omitted; {route}"));
        }
        self.refs
            .retain(|r| r.len() <= MAX_REF_BYTES && !r.chars().any(char::is_control));
        self.refs.truncate(MAX_REFS);
        self
    }
}
/// Typed compact semantic projection; raw storage structs never enter templates.
#[derive(Clone, Serialize)]
pub struct Page {
    /// Safe human-facing heading.
    pub heading: String,
    /// Bounded scope/evidence/conditions lines.
    pub lines: Vec<String>,
    /// Pageable semantic rows, never serialized YAML.
    pub rows: Vec<String>,
    /// Editable file version; empty on owner-facing status.
    pub version: String,
    /// Exact view/scope snapshot for pagination, distinct from a manifest edit.
    pub snapshot_version: String,
    /// Exact creation precondition, only on project context.
    pub allocation_version: Option<String>,
    /// Complete or PARTIAL data coverage.
    pub coverage: String,
    /// Whether all readable detail fits; complete counts are retained separately.
    pub detail_coverage: String,
    /// Next actual row offset, absent at the end.
    pub next_start: Option<usize>,
    /// Omitted rows from this snapshot.
    pub remaining: usize,
}
/// Error projection with independently recorded effects, including post-write failure.
#[derive(Serialize)]
struct Failure<'a> {
    /// Classified stable code.
    code: &'a str,
    /// Bounded quoted explanation.
    message: String,
    /// Immutable execution ledger, retained even if normal rendering fails.
    effects: &'a [String],
    /// Plain Git receipt lines of this call, present when a failure happened after publishing.
    git: &'a [String],
    /// Distinguish setup-only effects from visibly published work.
    published: bool,
    /// Tool-specific inspection route; registration may not have published an alias yet.
    recovery: &'a str,
}
/// Mutation template input; the receipt remains separate from the effects ledger.
#[derive(Serialize)]
struct Saved<'a> {
    /// Confirmed outcome.
    ack: &'a Ack,
    /// Exact observed side effects.
    effects: &'a [String],
    /// Plain Git receipt lines of the current call, rendered after the producer notes.
    git: &'a [String],
    /// The single read route this reply recommends, built from the tool kind and target.
    next: String,
}

/// What one mutating call published and what Git said about it.
///
/// Kept outside the result so a failure after publication still reports both: `published` comes
/// from the typed publications of the request, never from effect strings, and `git` holds the
/// persistence receipt lines of this call only.
#[derive(Default)]
struct Ledger {
    /// The request published at least one typed effect (a directory counts).
    published: bool,
    /// Plain receipt lines of the current call; earlier commits appear as their own line.
    git: Vec<String>,
}

/// Closed text layouts preserve execution effects, scope labels and exact continuation tokens.
pub fn templates() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "project_registered",
            "{% if changed %}REGISTERED{% else %}UNCHANGED{% endif %} {{ project }} — {{ name }}\nVersion: {{ version }}\n{% for effect in effects %}{{ effect }}\n{% endfor %}Next: get_context(project=\"{{ project }}\") for orientation, or project_status for progress.\n",
        ),
        (
            "core_page",
            "{{ heading }}\nData coverage: {{ coverage }}; detail coverage: {{ detail_coverage }}\n{% if version %}Version: {{ version }}\nSnapshot version: {{ snapshot_version }}\n{% endif %}{% if allocation_version %}Allocation version: {{ allocation_version }}\n{% endif %}{% for line in lines %}{{ line }}\n{% endfor %}{% for row in rows %}{{ row }}\n{% endfor %}{% if next_start != none %}Next: start={{ next_start }}; version={{ snapshot_version }}; remaining={{ remaining }}. Keep the same tool and selection.\n{% elif remaining %}{{ remaining }} detail rows omitted; narrow project_status with module=M-001.\n{% endif %}",
        ),
        (
            "core_ack",
            "{% if ack.changed %}SAVED{% else %}UNCHANGED{% endif %} {{ ack.target }}\n{% if ack.target == \"Project\" %}Project state:{% else %}{{ ack.phase_label }}:{% endif %} {{ ack.phase }}\nVersion: {{ ack.version }}\n{% for effect in effects %}{{ effect }}\n{% endfor %}{% for note in ack.notes %}{{ note }}\n{% endfor %}{% for line in git %}{{ line }}\n{% endfor %}Next: {{ next }} Use project_status for the complete tracked overview. Do not replay a lost reply blindly.\n",
        ),
        (
            "recovery_ack",
            "{% if ack.changed %}SAVED{% else %}UNCHANGED{% endif %} {{ ack.target }}\n{{ ack.phase_label }}: {{ ack.phase }}\nVersion: {{ ack.version }}\n{% for effect in effects %}{{ effect }}\n{% endfor %}{% for note in ack.notes %}{{ note }}\n{% endfor %}{% for line in git %}{{ line }}\n{% endfor %}Next: {{ next }} Do not replay a lost reply blindly.\n",
        ),
        (
            "core_error",
            "ERROR {{ code }}: {{ message }}\n{% if published %}Visible publication occurred; outcome/durability may be partial.\n{% else %}No business publication confirmed by this call.\n{% endif %}{% for effect in effects %}{{ effect }}\n{% endfor %}{% for line in git %}{{ line }}\n{% endfor %}Next: {{ recovery }}\n",
        ),
    ]
}

/// Route only the known business tools. Expected failures return isError=true; a
/// saved changes_requested verdict returns false. Unknown tools remain protocol errors.
pub fn call(
    name: &str,
    args: Value,
    config: &Config,
    templates: &Templates,
) -> Option<CallToolResult> {
    if ![
        "register_project",
        "get_project_list",
        "get_context",
        "project_status",
        "search",
        "plan_work",
        "record_work",
        "review_module",
        "review_work",
        "knowledge_work",
        "document_work",
        "compaction_work",
        "git_recovery",
    ]
    .contains(&name)
    {
        return None;
    }
    let mut effects = Vec::new();
    let mut ledger = Ledger::default();
    let result = (|| -> Result<String> {
        match name {
            "register_project" => {
                super::projects::register(config, decode_args(name, args)?, templates, &mut effects)
            }
            "get_project_list" => {
                super::projects::list(config, decode_args(name, args)?, templates)
            }
            "get_context" => {
                let args = decode_args(name, args)?;
                super::read::context(config, args, templates)
            }
            "project_status" => super::read::status(config, decode_args(name, args)?, templates),
            "search" => super::read::search(config, decode_args(name, args)?, templates),
            "plan_work" => {
                let (common, operation) =
                    input::mutation::<Plan>(args, false).map_err(tool_arguments(name))?;
                validate_common(&common)?;
                let prepare = matches!(operation, Plan::InitProject { .. });
                let ack = mutation_scope(
                    config,
                    &common.project,
                    persist::EventClass::Work,
                    prepare,
                    &mut effects,
                    &mut ledger,
                    |s, g, e| plan(s, g, &common, operation, e),
                )?;
                Ok(render_ack(
                    templates,
                    ToolKind::Work,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "record_work" => {
                let (common, operation) =
                    input::mutation::<Work>(args, true).map_err(tool_arguments(name))?;
                validate_common(&common)?;
                let ack = mutation_scope(
                    config,
                    &common.project,
                    persist::EventClass::Work,
                    false,
                    &mut effects,
                    &mut ledger,
                    |s, g, e| record(s, g, &common, operation, e),
                )?;
                Ok(render_ack(
                    templates,
                    ToolKind::Work,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "review_module" => {
                let args: ReviewArgs = decode_args(name, args)?;
                number(&args.module, "M-").map_err(arguments)?;
                let ack = review_call(config, args, &mut effects, &mut ledger)?;
                Ok(render_ack(
                    templates,
                    ToolKind::Work,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "review_work" => {
                let a: ReviewWorkArgs = decode_args(name, args)?;
                let args = ReviewArgs {
                    project: a.project,
                    module: a.reference,
                    version: a.version,
                    verdict: a.verdict,
                    summary: a.summary,
                    findings: a.findings,
                    checks: a.checks,
                    actor: a.actor,
                    changed_scope: a.changed_scope,
                    resolved_findings: a.resolved_findings,
                };
                let ack = review_call(config, args, &mut effects, &mut ledger)?;
                Ok(render_ack(
                    templates,
                    ToolKind::Work,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "knowledge_work" => {
                let (common, operation) =
                    input::mutation::<knowledge_ops::KnowledgeOp>(args, false)
                        .map_err(tool_arguments(name))?;
                validate_common(&common)?;
                let ack = mutation_scope(
                    config,
                    &common.project,
                    persist::EventClass::Knowledge,
                    false,
                    &mut effects,
                    &mut ledger,
                    |s, g, e| knowledge_ops::execute_locked(s, g, &common, operation, e),
                )?;
                Ok(render_ack(
                    templates,
                    ToolKind::Knowledge,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "document_work" => {
                let (common, operation) = input::mutation::<document_ops::DocumentOp>(args, false)
                    .map_err(tool_arguments(name))?;
                validate_common(&common)?;
                let ack = mutation_scope(
                    config,
                    &common.project,
                    persist::EventClass::Document,
                    false,
                    &mut effects,
                    &mut ledger,
                    |s, g, e| document_ops::execute_locked(s, g, &common, operation, e),
                )?;
                Ok(render_ack(
                    templates,
                    ToolKind::Document,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "compaction_work" => {
                let (common, operation) =
                    input::mutation::<compaction_ops::Compaction>(args, false)
                        .map_err(tool_arguments(name))?;
                validate_common(&common)?;
                let class = operation.event_class();
                let ack = mutation_scope(
                    config,
                    &common.project,
                    class,
                    false,
                    &mut effects,
                    &mut ledger,
                    |s, g, e| compaction_ops::execute_locked(s, g, &common, operation, e),
                )?;
                Ok(render_ack(
                    templates,
                    ToolKind::Compaction,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            "git_recovery" => {
                let (common, operation) = input::mutation::<recovery_ops::RecoveryOp>(args, false)
                    .map_err(tool_arguments(name))?;
                validate_common(&common)?;
                let ack = mutation_scope(
                    config,
                    &common.project,
                    recovery_ops::event_class(),
                    false,
                    &mut effects,
                    &mut ledger,
                    |s, g, e| recovery_ops::execute_locked(s, g, &common, operation, e),
                )?;
                Ok(render_ack(
                    templates,
                    ToolKind::GitRecovery,
                    &ack,
                    &effects,
                    &ledger.git,
                ))
            }
            _ => Err(arguments("Unknown business tool.")),
        }
    })();
    let (text, error) = match result {
        Ok(text) => (text, false),
        Err(e) => {
            let failure = Failure {
                code: e.code,
                message: store::safe(&e.message, 600),
                effects: &effects,
                git: &ledger.git,
                published: ledger.published || effects.iter().any(|e| e.starts_with("Published ")),
                recovery: match name {
                    "register_project" => {
                        "Inspect the requested documentation root and get_project_list; the alias may be unpublished. Resolve the cause before repeating the same intent."
                    }
                    "get_project_list" => {
                        "Inspect settings and projects.toml; reads do not repair or migrate them."
                    }
                    _ => {
                        "Use get_context to inspect current work/version; repair the condition before retrying."
                    }
                },
            };
            let text = templates
                .render("core_error", &failure)
                .unwrap_or_else(|_| {
                    let head = format!(
                        "ERROR {}. Presentation degraded.\n{}",
                        e.code,
                        plain_line(&failure.message, 512)
                    );
                    degraded(
                        &head,
                        &effects,
                        &ledger.git,
                        "Inspect current state before retrying.",
                    )
                });
            (text, true)
        }
    };
    let mut reply = CallToolResult::success(vec![ContentBlock::text(text)]);
    reply.is_error = Some(error);
    Some(reply)
}

/// The one place a mutating call resolves its project, takes the single root write lock and
/// settles Git before releasing it.
///
/// The alias is resolved, `prepare_first` creates the state directory before locking (only
/// `init_project` needs it), the write lock is taken exactly once, and `body` runs while the
/// guard is held. Settlement happens under that same guard, from the actual typed publications of
/// this request and never from the `changed` flag alone:
///
/// - `Ok` with `changed = true` settles as `Success`.
/// - `Ok` with `changed = false` and no eligible file publication is a real no-op: nothing is
///   settled and no Git line is added.
/// - `Ok` with `changed = false` but an eligible file publication settles as `Partial`, so the
///   effect is tracked and reported, never committed as a success and never dropped.
/// - `Err` after any publication settles as `Partial`; the business error is returned unchanged.
/// - `Err` before any publication is not settled.
///
/// The provider's plain receipt lines are appended to `git`; settlement never changes the
/// business result and a Git failure never fails a saved change. The guard drops when the scope
/// returns, so nothing inside `body` may lock again.
///
/// # Errors
/// Alias, preparation and lock failures (`busy` when another writer owns the lock) and every
/// error `body` returns, unchanged.
fn mutation_scope(
    config: &Config,
    project: &str,
    class: persist::EventClass,
    prepare_first: bool,
    effects: &mut Vec<String>,
    ledger: &mut Ledger,
    body: impl FnOnce(&Store, &LockGuard, &mut Vec<String>) -> Result<Ack>,
) -> Result<Ack> {
    let store = config.resolve(project)?;
    if prepare_first {
        store.prepare(effects)?;
    }
    let guard = store
        .lock(true, effects)?
        .ok_or_else(|| Error::new("io", "Cannot take the writer lock."))?;
    let mut result = body(&store, &guard, effects);
    let events = store.publications();
    ledger.published = !events.is_empty();
    let file_effect = events.iter().any(|p| {
        matches!(
            p.kind,
            store::EffectKind::Created | store::EffectKind::Replaced | store::EffectKind::Removed
        ) && p.tracking != store::Tracking::NotApplicable
    });
    let outcome = match &result {
        Ok(ack) if ack.changed => Some(persist::EventOutcome::Success),
        Ok(_) if file_effect => Some(persist::EventOutcome::Partial),
        Ok(_) => None,
        Err(_) if !events.is_empty() => Some(persist::EventOutcome::Partial),
        Err(_) => None,
    };
    // Recovery settles itself through its own receipt; ordinary settlement never re-enters the
    // engine that just acted (or failed).
    if class != persist::EventClass::Recovery
        && let Some(outcome) = outcome
    {
        let refs = match &result {
            Ok(ack) if is_canonical_ref(&ack.target) => vec![ack.target.clone()],
            _ => Vec::new(),
        };
        let event = persist::Event {
            class,
            refs,
            operation: None,
            outcome,
        };
        ledger
            .git
            .extend(persist::settle(&store, &guard, &event, persist::production_policy()).lines());
        if let Ok(ack) = &mut result
            && !ack.changed
        {
            ack.notes.push(
                "Files were published although the result is unchanged; see the Git line.".into(),
            );
        }
    }
    result
}

/// Validate and run one whole-record review inside the shared mutation scope.
///
/// Request validation happens before the alias is resolved or any lock is taken, exactly as the
/// review always did.
fn review_call(
    config: &Config,
    args: ReviewArgs,
    effects: &mut Vec<String>,
    ledger: &mut Ledger,
) -> Result<Ack> {
    validate_common(&Common {
        project: args.project.clone(),
        version: args.version.clone(),
        actor: args.actor.clone(),
        reference: None,
    })?;
    let project = args.project.clone();
    mutation_scope(
        config,
        &project,
        persist::EventClass::Work,
        false,
        effects,
        ledger,
        |s, g, e| review(s, g, args, e),
    )
}

/// Closed serde argument decoding for a tool without an operation tag.
///
/// The failure names `tool` and, for an unknown or missing key, that field; parser source values
/// never enter diagnostics.
pub fn decode_args<T: serde::de::DeserializeOwned>(tool: &str, args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|e| {
        arguments(format!(
            "{tool}: {}",
            input::shape_error(&e, input::Scope::Tool)
        ))
    })
}
/// Prefix an operation decoding failure with the tool that was called.
fn tool_arguments(tool: &str) -> impl Fn(String) -> Error + '_ {
    move |message| arguments(format!("{tool}: {message}"))
}
/// Classify semantic/structural arguments without dumping raw inputs.
fn arguments(message: impl Into<String>) -> Error {
    Error::new("invalid_arguments", message)
}

/// Validate generated-version preconditions and declared identity at the request boundary.
fn validate_common(common: &Common) -> Result<()> {
    input::field("project", text(&common.project, 128))?;
    if common.version.len() != 64 || !common.version.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(arguments(
            "Use the exact 64-character version returned by get_context.",
        ));
    }
    input::field("actor", optional(&common.actor, 256))
}

/// Render once after capturing the immutable receipt; a presentation failure still names the saved target/version.
pub(super) fn render_ack(
    templates: &Templates,
    kind: ToolKind,
    ack: &Ack,
    effects: &[String],
    git: &[String],
) -> String {
    let next = if kind == ToolKind::GitRecovery {
        "get_context with project only; omit ref, to read the pending version and facts.".to_owned()
    } else if ack.target == "Project" {
        "get_context with project only; omit ref.".to_owned()
    } else if is_canonical_ref(&ack.target) {
        format!("get_context with ref={}.", ack.target)
    } else {
        "the target is not a canonical reference; use get_context with project only and omit ref."
            .to_owned()
    };
    let layout = if kind == ToolKind::GitRecovery {
        "recovery_ack"
    } else {
        "core_ack"
    };
    let ack = ack.clone().bounded(&next);
    templates
        .render(
            layout,
            &Saved {
                ack: &ack,
                effects,
                git,
                next,
            },
        )
        .unwrap_or_else(|_| {
            let head = format!(
                "{} {}. Version: {}. Presentation degraded.",
                if ack.changed { "SAVED" } else { "UNCHANGED" },
                ack.target,
                ack.version
            );
            degraded(&head, effects, git, "Inspect get_context; do not replay.")
        })
}

/// Last-resort reply when a template cannot render: the head line, every Git receipt line (a
/// pending commit or an attention flag must never disappear), then as many effect lines as the
/// reply limit allows with an explicit omission count.
fn degraded(head: &str, effects: &[String], git: &[String], tail: &str) -> String {
    let mut text = format!("{head}\n");
    for line in git {
        text.push_str(&plain_line(line, 200));
        text.push('\n');
    }
    let reserve = tail.len() + 80;
    let mut shown = 0;
    for effect in effects {
        let line = plain_line(effect, 400);
        if text.len() + line.len() + 1 + reserve > super::pages::REPLY_LIMIT {
            break;
        }
        text.push_str(&line);
        text.push('\n');
        shown += 1;
    }
    if shown < effects.len() {
        text.push_str(&format!(
            "{} effect line(s) omitted.\n",
            effects.len() - shown
        ));
    }
    text.push_str(tail);
    text.push('\n');
    text
}

/// Label the record kind of a canonical target; the label carries its own noun.
///
/// `E-`, `A-` and `M-` keep the historical phase wording, so existing replies stay byte
/// identical. Knowledge, document and compaction targets report a state. A target that is not a
/// recognised canonical reference falls back to the Module label, exactly as before; callers
/// never build a read route from a label.
fn target_label(target: &str) -> &'static str {
    if target.starts_with("E-") {
        "Epic phase"
    } else if target.starts_with("A-") {
        "Atomic phase"
    } else if target.starts_with("RB-") {
        "Runbook state"
    } else if target.starts_with("RS-") {
        "Research state"
    } else if target.starts_with("CL-") {
        "Checklist state"
    } else if target.starts_with("CP-") {
        "Compaction state"
    } else if target == "Git recovery" {
        "Git recovery state"
    } else if target.starts_with("DOC-") || target == "README.md" || target.starts_with("docs/") {
        "Document state"
    } else if target.starts_with("D-") {
        "Decision state"
    } else {
        "Module phase"
    }
}

/// Build a target receipt without deriving external success.
///
/// `target` is the canonical reference usable with `get_context`; `version` the new record or
/// observation version usable for the next write; `phase` a short lowercase state; `changed`
/// whether the business record changed. Notes and refs start empty and are filled by the caller.
pub(super) fn ack(
    target: impl Into<String>,
    version: String,
    phase: impl Into<String>,
    changed: bool,
) -> Ack {
    let target = target.into();
    Ack {
        phase_label: target_label(&target),
        target,
        version,
        phase: phase.into(),
        changed,
        notes: Vec::new(),
        refs: Vec::new(),
    }
}

/// Allocate initial task facts; no commit, dates or counters come from the caller.
fn make_task(input: TaskInput, n: u64, at: &str) -> Task {
    Task {
        id: format!("T-{n:03}"),
        title: input.title,
        criterion: input.criterion,
        executor: None,
        atomic_workflow: None,
        started_at: None,
        required_checks: input.required_checks,
        state: TaskState::Open,
        result: None,
        checks: Vec::new(),
        cancellation: None,
        cancellation_history: Vec::new(),
        created_at: at.into(),
        updated_at: at.into(),
    }
}

/// Resolve the module portion and reject arbitrary paths or alternate numeric identities.
pub fn module_id(reference: &str) -> Result<&str> {
    let id = reference
        .split('/')
        .next()
        .ok_or_else(|| arguments("Invalid reference."))?;
    work_number(id).map_err(arguments)?;
    Ok(id)
}

/// Read a complete current module, enforce byte version and intact counters before any change.
fn current(store: &Store, id: &str, version: &str) -> Result<Snapshot<Module>> {
    store.open_parent(id)?;
    let snapshot = store.module(id)?;
    if snapshot.version != version {
        return Err(Error::new(
            "stale",
            format!(
                "No work saved. {} is {} with current version {}. Read get_context before retrying.",
                id,
                store.phase(&snapshot.value),
                snapshot.version
            ),
        ));
    }
    snapshot.value.counters().map_err(store::invalid)?;
    Ok(snapshot)
}

/// Check the bounded list fields of a record before whole-record validation so a rejected list
/// names its field and limit (`criteria`, `required_checks`) and never echoes a value.
///
/// Covers the record's own criteria and required checks and every Task or Atomic's required
/// checks. The record validator stays the authority; this only adds the field name.
fn check_lists(value: &Module) -> Result<()> {
    input::field("criteria", strings(&value.criteria, 1024, true))?;
    input::field("required_checks", strings(&value.required_checks, 64, true))?;
    for child in value.children() {
        input::field("required_checks", strings(&child.required_checks, 64, true))?;
    }
    Ok(())
}

/// Save one module, advancing review epoch for semantic changes or explicit reopen.
/// Only the fixed canceled action may consume terminal closing headroom here;
/// accepted review uses its dedicated path. A no-op never appends a log/date.
fn save_module(
    store: &Store,
    before: &Snapshot<Module>,
    mut value: Module,
    target: &str,
    action: &str,
    actor: &Option<String>,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    if value == before.value {
        return Ok(ack(
            target,
            before.version.clone(),
            store.phase(&value),
            false,
        ));
    }
    for i in 0..value.tasks.len() + value.atomics.len() {
        if value.child(i).atomic_workflow.is_some() && action != "Atomic review recorded" {
            let Some(old) = before
                .value
                .children()
                .find(|old| old.id == value.child(i).id)
            else {
                continue;
            };
            if action == "reopened" && target == format!("{}/{}", value.id, value.child(i).id)
                || value.child(i).atomic_basis().map_err(arguments)?
                    != old.atomic_basis().map_err(arguments)?
            {
                let a = value
                    .child_mut(i)
                    .atomic_workflow
                    .as_mut()
                    .ok_or_else(|| arguments("Missing Atomic lifecycle."))?;
                a.review_epoch = a
                    .review_epoch
                    .checked_add(1)
                    .ok_or_else(|| arguments("Atomic review epoch exhausted."))?;
            }
        }
    }
    if value.core().is_some()
        && (action == "reopened"
            || value.result.as_ref().and_then(|r| r.candidate.as_ref())
                != before
                    .value
                    .result
                    .as_ref()
                    .and_then(|r| r.candidate.as_ref()))
    {
        let c = value.core_mut().map_err(arguments)?;
        c.implementation_epoch = c
            .implementation_epoch
            .checked_add(1)
            .ok_or_else(|| arguments("Implementation generation exhausted."))?;
    }
    let semantic_change =
        value.basis().map_err(store::invalid)? != before.value.basis().map_err(store::invalid)?;
    if value.id.starts_with("A-")
        && semantic_change
        && !matches!(
            action,
            "result reported" | "local outcome completed" | "commits imported"
        )
    {
        value.completed = false;
    }
    if value.modern() && value.id.starts_with("M-") && (action == "reopened" || semantic_change) {
        value.workflow_mut().delivery = None;
    }
    if action == "reopened" || semantic_change {
        value.review_epoch = value
            .review_epoch
            .checked_add(1)
            .ok_or_else(|| store::invalid("Review epoch exhausted."))?;
    }
    value
        .event(target, action, &store::now(), actor)
        .map_err(store::invalid)?;
    check_lists(&value)?;
    value.validate().map_err(store::invalid)?;
    store.save(
        &work_path(&value.id).map_err(arguments)?,
        &value,
        Some(&before.bytes),
        action == "canceled" || action == "delivery reported",
        effects,
    )?;
    let version = store.work_version(&value, &store::encode(&value)?)?;
    Ok(ack(target, version, store.phase(&value), true))
}

/// Initialize missing manifest/allocator records under the caller's root lock.
/// Reject existing manifests, foreign modules and inconsistent partial allocators; retain every effect.
pub(super) fn initialize(
    store: &Store,
    project: Project,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    if store.project()?.is_some() {
        return Err(Error::new(
            "already_initialized",
            "Project already exists; use edit_project with its manifest version.",
        ));
    }
    let inventory = store.inventory()?;
    require_inventory(&inventory)?;
    if !inventory.ids.is_empty() {
        return Err(Error::new(
            "partial_init",
            "Modules without a manifest are not an empty initialization.",
        ));
    }
    project.validate().map_err(arguments)?;
    let old = store.bytes(".agent-tasks/state.yaml")?;
    let allocator = Allocator {
        schema_version: 2,
        next_module: 1,
        next_epic: Some(1),
        next_atomic: Some(1),
    };
    if let Some(bytes) = &old {
        let mut existing: Allocator = store::decode(bytes)?;
        existing.prepare(&inventory.ids).map_err(store::invalid)?;
        if existing != allocator {
            return Err(Error::new(
                "partial_init",
                "Existing allocator is inconsistent with empty initialization; restore a retained copy.",
            ));
        }
    }
    let expected_state = old.clone().unwrap_or(store::encode(&allocator)?);
    if old.is_none() {
        store.save(".agent-tasks/state.yaml", &allocator, None, false, effects)?;
    }
    let after_inventory = store.inventory()?;
    require_inventory(&after_inventory)?;
    if store.bytes("project.yaml")?.is_some()
        || store.bytes(".agent-tasks/state.yaml")?.as_deref() != Some(expected_state.as_slice())
        || !after_inventory.ids.is_empty()
    {
        return Err(Error::new(
            "partial_init",
            "Initialization changed after allocator publication; inspect context before resuming.",
        ));
    }
    let version = store.save("project.yaml", &project, None, false, effects)?;
    Ok(ack("Project", version, "initialized", true))
}

/// Execute the closed plan operation under one root lock; creations reserve numbers before publication.
fn plan(
    store: &Store,
    _guard: &LockGuard,
    common: &Common,
    operation: Plan,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    match operation {
        Plan::FreezeEpic { epic } => {
            let before = current(store, &epic, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            freeze_epic(store, &mut value)?;
            save_module(
                store,
                &before,
                value,
                &epic,
                "roster frozen",
                &common.actor,
                effects,
            )
        }
        Plan::InitProject {
            title,
            purpose,
            remote,
        } => {
            expect_allocation(store, &common.version)?;
            let at = store::now();
            initialize(
                store,
                Project {
                    schema_version: SCHEMA,
                    title,
                    purpose,
                    remote,
                    created_at: at.clone(),
                    updated_at: at,
                },
                effects,
            )
        }
        Plan::EditProject {
            title,
            purpose,
            remote,
        } => {
            let mut snapshot = store
                .project()?
                .ok_or_else(|| Error::new("not_initialized", "Initialize the project first."))?;
            store.expect("project.yaml", &common.version)?;
            let before = snapshot.value.clone();
            title
                .required(&mut snapshot.value.title)
                .map_err(arguments)?;
            purpose
                .required(&mut snapshot.value.purpose)
                .map_err(arguments)?;
            remote.optional(&mut snapshot.value.remote);
            snapshot.value.validate().map_err(arguments)?;
            if before == snapshot.value {
                return Ok(ack("Project", snapshot.version, "unchanged", false));
            }
            snapshot.value.updated_at = store::now();
            let version = store.save(
                "project.yaml",
                &snapshot.value,
                Some(&snapshot.bytes),
                false,
                effects,
            )?;
            Ok(ack("Project", version, "updated", true))
        }
        Plan::CreateModule {
            title,
            outcome,
            lead,
            required_checks,
            tasks,
            criteria,
            execution,
            contracts,
            dependencies,
        } => {
            if !tasks.is_empty() {
                return Err(arguments(
                    "Create the provisional Module goal, bind its actual lead, then let that lead discover/add Tasks.",
                ));
            }
            if tasks.len() > MAX_TASKS {
                return Err(arguments("At most 32 initial tasks."));
            }
            let at = store::now();
            let mut value = new_record("M-001", title, outcome, lead, required_checks, &at);
            value.criteria = criteria;
            let w = value.workflow_mut();
            w.execution = execution;
            w.contracts = contracts;
            w.dependencies = dependencies;
            value.next_task = Some(tasks.len() as u64 + 1);
            value.tasks = tasks
                .into_iter()
                .enumerate()
                .map(|(i, t)| make_task(t, i as u64 + 1, &at))
                .collect();
            create_record(store, common, value, effects)
        }
        Plan::CreateEpic {
            title,
            outcome,
            criteria,
            lead,
            required_checks,
        } => {
            let mut value = new_record(
                "E-001",
                title,
                outcome,
                lead,
                required_checks,
                &store::now(),
            );
            value.criteria = criteria;
            create_record(store, common, value, effects)
        }
        Plan::CreateAtomic {
            title,
            outcome,
            executor,
            required_checks,
            participants,
            execution,
            environment,
            scenarios,
        } => {
            let mut value = new_record(
                "A-001",
                title,
                outcome,
                executor,
                required_checks,
                &store::now(),
            );
            value.participants = participants;
            let w = value.workflow_mut();
            w.execution = execution;
            w.environment = environment;
            w.scenarios = scenarios;
            value.validate().map_err(arguments)?;
            store.participant_basis(&value.participants)?;
            create_record(store, common, value, effects)
        }
        Plan::EditEpic {
            epic,
            title,
            outcome,
            criteria,
            lead,
            required_checks,
            modules,
            atomics,
            criterion_scopes,
        } => {
            number(&epic, "E-").map_err(arguments)?;
            let before = current(store, &epic, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            title.required(&mut value.title).map_err(arguments)?;
            outcome.required(&mut value.outcome).map_err(arguments)?;
            criteria.required(&mut value.criteria).map_err(arguments)?;
            lead.optional(&mut value.lead);
            required_checks
                .required(&mut value.required_checks)
                .map_err(arguments)?;
            modules.required(&mut value.modules).map_err(arguments)?;
            atomics
                .required(&mut value.atomic_members)
                .map_err(arguments)?;
            if !matches!(criterion_scopes, input::Patch::Absent) {
                criterion_scopes
                    .required(&mut value.core_mut().map_err(arguments)?.criterion_scopes)
                    .map_err(arguments)?;
            }
            check_lists(&value)?;
            value.validate().map_err(arguments)?;
            store.membership(&value)?;
            let adds_edges = value
                .modules
                .iter()
                .any(|id| !before.value.modules.contains(id))
                || value
                    .atomic_members
                    .iter()
                    .any(|id| !before.value.atomic_members.contains(id));
            if adds_edges {
                store.links(&value)?;
            }
            for id in value.modules.iter().chain(&value.atomic_members) {
                if !before.value.modules.contains(id) && !before.value.atomic_members.contains(id) {
                    store.open_parent(id)?;
                }
            }
            save_module(
                store,
                &before,
                value,
                &epic,
                "Epic plan edited",
                &common.actor,
                effects,
            )
        }
        Plan::AddAtomic {
            module,
            title,
            outcome,
            executor,
            required_checks,
        } => {
            number(&module, "M-").map_err(arguments)?;
            let before = current(store, &module, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            if value.atomics.len() >= MAX_TASKS {
                return Err(arguments("At most 32 Module Atomics."));
            }
            let n = value
                .next_atomic
                .ok_or_else(|| store::invalid("Missing Atomic counter."))?;
            let mut atomic = make_task(
                TaskInput {
                    title,
                    criterion: Some(outcome),
                    required_checks,
                },
                n,
                &store::now(),
            );
            atomic.id = format!("A-{n:03}");
            atomic.executor = executor;
            atomic.atomic_workflow = Some(AtomicWorkflow::new());
            let target = format!("{module}/{}", atomic.id);
            value.next_atomic = Some(
                n.checked_add(1)
                    .ok_or_else(|| store::invalid("Atomic counter exhausted."))?,
            );
            value.atomics.push(atomic);
            save_module(
                store,
                &before,
                value,
                &target,
                "Atomic added",
                &common.actor,
                effects,
            )
        }
        Plan::EditAtomic {
            reference,
            title,
            outcome,
            executor,
            required_checks,
            participants,
            execution,
            environment,
            scenarios,
        } => {
            let before = current(store, module_id(&reference)?, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            let index = value.target(&reference).map_err(arguments)?;
            if let Some(i) = index {
                if !value.child(i).id.starts_with("A-") {
                    return Err(arguments("edit_atomic requires an Atomic."));
                }
                if !matches!(execution, input::Patch::Absent)
                    || !matches!(environment, input::Patch::Absent)
                    || !matches!(scenarios, input::Patch::Absent)
                {
                    return Err(arguments(
                        "Execution/integration belong to standalone Atomics; embedded work inherits its Module.",
                    ));
                }
                if !matches!(participants, input::Patch::Absent) {
                    return Err(arguments(
                        "Integration participants belong to standalone Atomics.",
                    ));
                }
                let atomic = value.child_mut(i);
                if atomic.state == TaskState::Canceled {
                    return Err(arguments("Reopen canceled Atomic first."));
                }
                title.required(&mut atomic.title).map_err(arguments)?;
                let mut expected = atomic
                    .criterion
                    .clone()
                    .ok_or_else(|| arguments("Atomic outcome missing."))?;
                outcome.required(&mut expected).map_err(arguments)?;
                atomic.criterion = Some(expected);
                executor.optional(&mut atomic.executor);
                required_checks
                    .required(&mut atomic.required_checks)
                    .map_err(arguments)?;
                if atomic != before.value.child(i) {
                    atomic.updated_at = store::now();
                }
            } else {
                number(&value.id, "A-").map_err(arguments)?;
                title.required(&mut value.title).map_err(arguments)?;
                outcome.required(&mut value.outcome).map_err(arguments)?;
                executor.optional(&mut value.lead);
                required_checks
                    .required(&mut value.required_checks)
                    .map_err(arguments)?;
                participants
                    .required(&mut value.participants)
                    .map_err(arguments)?;
                if !matches!(execution, input::Patch::Absent)
                    || !matches!(environment, input::Patch::Absent)
                    || !matches!(scenarios, input::Patch::Absent)
                {
                    let w = value.workflow_mut();
                    execution.optional(&mut w.execution);
                    environment.optional(&mut w.environment);
                    scenarios.required(&mut w.scenarios).map_err(arguments)?;
                }
                value.validate().map_err(arguments)?;
                store.participant_basis(&value.participants)?;
                if value.basis().map_err(arguments)? != before.value.basis().map_err(arguments)? {
                    value.completed = false;
                    value.participant_basis.clear();
                }
            }
            save_module(
                store,
                &before,
                value,
                &reference,
                "Atomic plan edited",
                &common.actor,
                effects,
            )
        }
        Plan::EditModule {
            module,
            title,
            outcome,
            lead,
            required_checks,
            criteria,
            execution,
            contracts,
            dependencies,
        } => {
            number(&module, "M-").map_err(arguments)?;
            let before = current(store, &module, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            title.required(&mut value.title).map_err(arguments)?;
            outcome.required(&mut value.outcome).map_err(arguments)?;
            if value
                .core()
                .is_some_and(|c| c.binding(AgentRole::Lead).is_some())
                && !matches!(lead, input::Patch::Absent)
            {
                return Err(arguments(
                    "Keep bound observed lead identity; edit_module lead cannot replace/clear it.",
                ));
            }
            lead.optional(&mut value.lead);
            required_checks
                .required(&mut value.required_checks)
                .map_err(arguments)?;
            criteria.required(&mut value.criteria).map_err(arguments)?;
            check_lists(&value)?;
            if !matches!(execution, input::Patch::Absent)
                || !matches!(contracts, input::Patch::Absent)
                || !matches!(dependencies, input::Patch::Absent)
                || (!value.criteria.is_empty() && value.workflow.is_none())
            {
                let w = value.workflow_mut();
                execution.optional(&mut w.execution);
                contracts.optional(&mut w.contracts);
                dependencies
                    .required(&mut w.dependencies)
                    .map_err(arguments)?;
            }
            contract_edit(store, &before.value, &mut value)?;
            store.links(&value)?;
            save_module(
                store,
                &before,
                value,
                &module,
                "plan edited",
                &common.actor,
                effects,
            )
        }
        Plan::AddTask {
            module,
            title,
            criterion,
            required_checks,
        } => {
            let before = current(store, &module, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            core_actor(&value, AgentRole::Lead, &common.actor)?;
            if value.tasks.len() >= MAX_TASKS {
                return Err(Error::new(
                    "capacity",
                    "Module already has 32 tasks; use a coherent new module.",
                ));
            }
            let n = value
                .next_task
                .ok_or_else(|| store::invalid("Missing task counter."))?;
            let task = make_task(
                TaskInput {
                    title,
                    criterion,
                    required_checks,
                },
                n,
                &store::now(),
            );
            let target = format!("{module}/{}", task.id);
            value.next_task = Some(
                n.checked_add(1)
                    .ok_or_else(|| store::invalid("Task counter exhausted."))?,
            );
            value.tasks.push(task);
            save_module(
                store,
                &before,
                value,
                &target,
                "task added",
                &common.actor,
                effects,
            )
        }
        Plan::EditTask {
            reference,
            title,
            criterion,
            required_checks,
        } => {
            let before = current(store, module_id(&reference)?, &common.version)?;
            let mut value = before.value.clone();
            open_module(&value)?;
            core_actor(&value, AgentRole::Lead, &common.actor)?;
            let i = value
                .target(&reference)
                .map_err(arguments)?
                .ok_or_else(|| arguments("edit_task requires an owned task reference."))?;
            if !reference
                .split_once('/')
                .is_some_and(|(_, id)| id.starts_with("T-"))
            {
                return Err(arguments("edit_task requires a Task, not an Atomic."));
            }
            let task = value.child_mut(i);
            if task.state == TaskState::Canceled {
                return Err(Error::new(
                    "canceled",
                    "Reopen this task explicitly before editing it.",
                ));
            }
            title.required(&mut task.title).map_err(arguments)?;
            criterion.optional(&mut task.criterion);
            required_checks
                .required(&mut task.required_checks)
                .map_err(arguments)?;
            if task != before.value.child(i) {
                task.updated_at = store::now();
            }
            save_module(
                store,
                &before,
                value,
                &reference,
                "task plan edited",
                &common.actor,
                effects,
            )
        }
    }
}

/// Construct the existing lightweight evidence envelope for one typed top-level work reference.
fn new_record(
    id: &str,
    title: String,
    outcome: String,
    lead: Option<Lead>,
    required_checks: Vec<String>,
    at: &str,
) -> Module {
    Module {
        schema_version: SCHEMA,
        id: id.into(),
        title,
        outcome,
        lead,
        required_checks,
        state: ModuleState::Open,
        next_task: Some(1),
        next_atomic: Some(1),
        next_log: Some(1),
        omitted_log_entries: 0,
        review_epoch: 0,
        tasks: Vec::new(),
        atomics: Vec::new(),
        criteria: Vec::new(),
        modules: Vec::new(),
        atomic_members: Vec::new(),
        participants: Vec::new(),
        participant_basis: std::collections::BTreeMap::new(),
        completed: false,
        workflow: Some({
            let mut w = Workflow::new();
            w.core = Some(CoreWorkflow::default());
            w
        }),
        imports: Vec::new(),
        result: None,
        checks: Vec::new(),
        blocker: None,
        handoff: None,
        cancellation: None,
        cancellation_history: Vec::new(),
        reviews: Vec::new(),
        reasons: Vec::new(),
        log: Vec::new(),
        created_at: at.into(),
        updated_at: at.into(),
    }
}

/// Reserve the next kind-specific number durably, then publish one independent no-clobber file.
/// A failed publication retains its reservation/effects; membership is always a separate Epic write.
fn create_record(
    store: &Store,
    common: &Common,
    mut value: Module,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    expect_allocation(store, &common.version)?;
    check_lists(&value)?;
    let project = store
        .project()?
        .ok_or_else(|| Error::new("not_initialized", "Initialize Project first."))?;
    let inventory = store.inventory()?;
    require_inventory(&inventory)?;
    if inventory.ids.len() >= store::MODULE_CAP {
        return Err(Error::new("capacity", "Work inventory at capacity."));
    }
    let bytes = store
        .bytes(".agent-tasks/state.yaml")?
        .ok_or_else(|| Error::new("allocator", "Missing allocator; restore retained state."))?;
    let mut state: Allocator = store::decode(&bytes)?;
    state.prepare(&inventory.ids).map_err(store::invalid)?;
    let prefix = work_number(&value.id).map_err(arguments)?.0;
    let counter = match prefix {
        "E-" => state.next_epic.as_mut(),
        "A-" => state.next_atomic.as_mut(),
        _ => Some(&mut state.next_module),
    }
    .ok_or_else(|| store::invalid("Missing counter."))?;
    value.id = format!("{prefix}{counter:03}");
    *counter += 1;
    let id = value.id.clone();
    store.links(&value)?;
    value
        .event(&id, "created", &store::now(), &common.actor)
        .map_err(arguments)?;
    value.validate().map_err(arguments)?;
    if store::encode(&value)?.len() > store::RECORD_CAP - store::CLOSING_RESERVE {
        return Err(Error::new("capacity", "Initial record exceeds capacity."));
    }
    effects.push(format!(
        "Creation target {id}; attachment is a separate Epic write."
    ));
    store.prepare(effects)?;
    store.save(
        ".agent-tasks/state.yaml",
        &state,
        Some(&bytes),
        false,
        effects,
    )?;
    let after = store.inventory()?;
    if store.bytes("project.yaml")?.as_deref() != Some(&project.bytes)
        || after.ids != inventory.ids
        || after.warnings != inventory.warnings
        || !after.complete
        || store.bytes(".agent-tasks/state.yaml")?.as_deref()
            != Some(store::encode(&state)?.as_slice())
    {
        return Err(Error::new(
            "allocation_changed",
            format!(
                "Scope changed after reservation of {id}; reserved number remains a gap. Inspect context."
            ),
        ));
    }
    store.save(
        &work_path(&id).map_err(arguments)?,
        &value,
        None,
        false,
        effects,
    )?;
    let version = store.work_version(&value, &store::encode(&value)?)?;
    Ok(ack(id, version, store.phase(&value), true))
}

/// Compare only the creation snapshot under lock; not an idempotency certificate.
fn expect_allocation(store: &Store, expected: &str) -> Result<()> {
    let current = store.allocation_version()?;
    if current != expected {
        return Err(Error::new(
            "stale",
            format!(
                "No new work allocated. Current allocation version: {current}. Read project context before retrying."
            ),
        ));
    }
    Ok(())
}

/// Unknown/nonregular/over-limit inventories refuse allocation without blocking healthy reads.
fn require_inventory(inventory: &store::Inventory) -> Result<()> {
    if !inventory.complete {
        return Err(Error::new(
            "inventory",
            "Incomplete or foreign module inventory; inspect project context before allocating.",
        ));
    }
    Ok(())
}
/// Canceled parents accept only explicit reopen and never cascade child changes.
fn open_module(value: &Module) -> Result<()> {
    if value.state == ModuleState::Canceled {
        return Err(Error::new(
            "canceled",
            "Reopen the module explicitly before changing it or its tasks.",
        ));
    }
    Ok(())
}

/// Apply reported current substance/lifecycle under the whole-module version.
fn record(
    store: &Store,
    _guard: &LockGuard,
    common: &Common,
    operation: Work,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let reference = common
        .reference
        .as_deref()
        .ok_or_else(|| arguments("Missing target."))?;
    let before = current(store, module_id(reference)?, &common.version)?;
    let mut value = before.value.clone();
    let index = value.target(reference).map_err(arguments)?;
    let reopening = matches!(operation, Work::Reopen { .. });
    if !reopening || index.is_some() {
        open_module(&value)?;
    }
    if index.is_some_and(|i| value.child_mut(i).state == TaskState::Canceled) && !reopening {
        return Err(Error::new(
            "canceled",
            "Reopen this canceled task before changing it.",
        ));
    }
    let at = store::now();
    let action = match operation {
        Work::AdoptCore {} => {
            module_only(index)?;
            if value.core().is_some() {
                return Ok(ack(reference, before.version, store.phase(&value), false));
            }
            let w = value.workflow_mut();
            w.managed = true;
            w.active = false;
            w.core = Some(CoreWorkflow::default());
            "core adopted"
        }
        Work::BindAgent {
            role,
            harness,
            agent_id,
            communication_ref,
            resume_ref,
            launch_ref,
        } => {
            module_only(index)?;
            role_owner(&value, role)?;
            let identity = AgentIdentity {
                harness,
                agent_id,
                communication_ref,
                resume_ref,
                launch_ref,
            };
            input::field("harness", text(&identity.harness, 64))?;
            input::field("agent_id", text(&identity.agent_id, 256))?;
            input::field("communication_ref", text(&identity.communication_ref, 256))?;
            input::field("resume_ref", optional(&identity.resume_ref, 256))?;
            input::field("launch_ref", text(&identity.launch_ref, 256))?;
            let core = value.core_mut().map_err(arguments)?;
            if let Some(binding) = core.bindings.iter_mut().find(|b| b.role == role) {
                let same = binding.current.identity.harness == identity.harness
                    && binding.current.identity.agent_id == identity.agent_id;
                if same {
                    binding.current.identity = identity;
                } else {
                    if binding.current.loss.is_none() {
                        return Err(Error::new(
                            "agent_replacement_refused",
                            "Keep the same assigned agent unless both irrecoverable loss and inability to continue/resume were observed.",
                        ));
                    }
                    let previous = std::mem::replace(
                        &mut binding.current,
                        AgentTerm {
                            identity,
                            loss: None,
                            needs_immersion: true,
                            immersion: None,
                        },
                    );
                    binding.history.push(previous);
                }
            } else {
                core.bindings.push(AgentBinding {
                    role,
                    current: AgentTerm {
                        identity,
                        loss: None,
                        needs_immersion: false,
                        immersion: None,
                    },
                    history: Vec::new(),
                });
            }
            "agent bound"
        }
        Work::RecoverAgent {
            role,
            stage,
            reason,
            observation,
            lost,
            unrecoverable,
            understanding,
            sources,
            unfinished,
            gaps,
        } => {
            module_only(index)?;
            role_owner(&value, role)?;
            let binding = value
                .core_mut()
                .map_err(arguments)?
                .bindings
                .iter_mut()
                .find(|b| b.role == role)
                .ok_or_else(|| arguments("Bind the observed actor before recovery."))?;
            match stage {
                input::RecoveryStage::Lost => {
                    if lost != Some(true) || unrecoverable != Some(true) {
                        return Err(Error::new(
                            "agent_loss_unproven",
                            "Report BOTH lost and unable to continue/unrecoverable; temporary unavailability is insufficient.",
                        ));
                    }
                    let reason = reason.ok_or_else(|| arguments("Loss reason required."))?;
                    let observation = observation.ok_or_else(|| {
                        arguments("Observed inability to continue/resume required.")
                    })?;
                    input::field("reason", text(&reason, 512))?;
                    input::field("observation", text(&observation, 1024))?;
                    binding.current.loss = Some(AgentLoss {
                        reason,
                        observation,
                        at: at.clone(),
                    });
                }
                input::RecoveryStage::Immersed => {
                    if common.actor.as_deref() != Some(binding.current.identity.agent_id.as_str())
                        || !binding.current.needs_immersion
                    {
                        return Err(arguments(
                            "Only the current replacement records its new immersion.",
                        ));
                    }
                    let understanding = understanding
                        .ok_or_else(|| arguments("Recovered understanding required."))?;
                    input::field("understanding", text(&understanding, 1024))?;
                    input::field("sources", strings(&sources, 256, false))?;
                    input::field("unfinished", strings(&unfinished, 256, false))?;
                    input::field("gaps", strings(&gaps, 256, false))?;
                    let complete = gaps.is_empty();
                    binding.current.immersion = Some(Immersion {
                        understanding,
                        sources,
                        unfinished,
                        gaps,
                        actor: binding.current.identity.agent_id.clone(),
                        at: at.clone(),
                    });
                    binding.current.needs_immersion = !complete;
                }
            }
            "agent recovery recorded"
        }
        Work::Planning {
            responsibility,
            scope,
            exclusions,
            read_refs,
            uncertainties,
        } => {
            module_only(index)?;
            number(&value.id, "M-").map_err(arguments)?;
            core_actor(&value, AgentRole::Lead, &common.actor)?;
            input::field("responsibility", text(&responsibility, 1024))?;
            input::field("scope", text(&scope, 1024))?;
            input::field("exclusions", strings(&exclusions, 256, false))?;
            input::field("read_refs", strings(&read_refs, 256, false))?;
            input::field("uncertainties", strings(&uncertainties, 256, false))?;
            let basis = value.plan_basis().map_err(arguments)?;
            value.core_mut().map_err(arguments)?.planning = Some(Planning {
                responsibility,
                scope,
                exclusions,
                read_refs,
                uncertainties,
                basis,
                actor: common
                    .actor
                    .clone()
                    .ok_or_else(|| arguments("Actual lead actor required."))?,
                at: at.clone(),
            });
            "lead planning recorded"
        }
        Work::AgreeContract {
            contract_id,
            revision,
            summary,
        } => {
            module_only(index)?;
            number(&value.id, "M-").map_err(arguments)?;
            core_actor(&value, AgentRole::Lead, &common.actor)?;
            input::field("summary", text(&summary, 1024))?;
            let facts = store.contract_facts(&contract_id)?;
            if facts.revision != revision
                || !facts.parties.contains(&value.id)
                || !facts.gaps.is_empty()
            {
                return Err(Error::new(
                    "contract_agreement_refused",
                    format!(
                        "Exact reciprocal {contract_id} revision and artifact must resolve before agreement: {}",
                        facts.gaps.join(" ")
                    ),
                ));
            }
            let core = value.core_mut().map_err(arguments)?;
            if !core.agreements.iter().any(|a| {
                a.contract_id == contract_id
                    && a.revision == revision
                    && a.snapshot == facts.snapshot
            }) {
                core.agreements.push(Agreement {
                    contract_id,
                    revision,
                    summary,
                    snapshot: facts.snapshot,
                    actor: common
                        .actor
                        .clone()
                        .ok_or_else(|| arguments("Actual lead actor required."))?,
                    at: at.clone(),
                });
            }
            "contract agreed"
        }
        Work::BoundaryEvidence {
            contract_id,
            revision,
            candidate,
            conditions,
            correct,
            mutation,
            failed,
            restored,
            artifacts,
        } => {
            module_only(index)?;
            number(&value.id, "M-").map_err(arguments)?;
            core_actor(&value, AgentRole::Lead, &common.actor)?;
            running(store, &value, index)?;
            input::field("candidate", text(&candidate, 256))?;
            input::field("conditions", text(&conditions, 1024))?;
            input::field("mutation", text(&mutation, 1024))?;
            input::field("artifacts", strings(&artifacts, 256, false))?;
            for observation in [&correct, &failed, &restored] {
                input::field("detail", text(&observation.detail, 512))?;
                input::field("artifact", optional(&observation.artifact, 256))?;
            }
            if correct.status != CheckStatus::Passed
                || failed.status != CheckStatus::Failed
                || restored.status != CheckStatus::Passed
            {
                return Err(Error::new(
                    "boundary_control",
                    "Record actual correct-pass, intended mutant-fail and restored-pass observations.",
                ));
            }
            if contract_id == "local" {
                if !store.contract_ids(&value).is_empty() || revision != 1 {
                    return Err(arguments(
                        "local scope applies only to a Module without declared cross-boundary contracts.",
                    ));
                }
            } else {
                let f = store.contract_facts(&contract_id)?;
                if f.revision != revision || !f.parties.contains(&value.id) {
                    return Err(arguments(
                        "Evidence must match an affecting current contract revision.",
                    ));
                }
            }
            let worktree = value
                .workflow
                .as_ref()
                .and_then(|w| w.execution.as_ref())
                .map(|e| e.worktree.clone())
                .ok_or_else(|| arguments("Declare the isolated Module checkout."))?;
            let intent_basis = Some(value.plan_basis().map_err(arguments)?);
            value
                .core_mut()
                .map_err(arguments)?
                .boundary_evidence
                .push(BoundaryEvidence {
                    contract_id,
                    revision,
                    candidate,
                    intent_basis,
                    conditions,
                    correct,
                    mutation,
                    failed,
                    restored,
                    artifacts,
                    worktree,
                    actor: common
                        .actor
                        .clone()
                        .ok_or_else(|| arguments("Actual lead actor required."))?,
                    at: at.clone(),
                });
            "boundary control recorded"
        }
        Work::VerifyCriterion {
            index: criterion_index,
            text: criterion_text,
            modules,
            candidate,
            environment,
            scenarios,
            summary,
            checks,
            artifacts,
            integration_ref,
        } => {
            module_only(index)?;
            number(&value.id, "E-").map_err(arguments)?;
            let scope = CriterionScope {
                index: criterion_index,
                text: criterion_text,
                modules,
            };
            if !value
                .core()
                .is_some_and(|c| c.criterion_scopes.contains(&scope))
            {
                return Err(arguments(
                    "Exact zero-based criterion text and declared affected Module set required.",
                ));
            }
            input::field("candidate", text(&candidate, 256))?;
            input::field("environment", text(&environment, 1024))?;
            input::field("summary", text(&summary, 1024))?;
            input::field("scenarios", strings(&scenarios, 256, true))?;
            input::field("artifacts", strings(&artifacts, 256, false))?;
            if scenarios.is_empty()
                || checks.is_empty()
                || checks.iter().any(|c| c.status != CheckStatus::Passed)
            {
                return Err(arguments(
                    "Actual business scenarios and passed checks required.",
                ));
            }
            if let Some(id) = &integration_ref {
                let a = store.module(id)?.value;
                if a.result.as_ref().and_then(|r| r.candidate.as_ref()) != Some(&candidate)
                    || a.workflow.as_ref().is_none_or(|w| {
                        w.environment.as_ref() != Some(&environment) || w.scenarios != scenarios
                    })
                {
                    return Err(arguments(
                        "Criterion reuse must match the ONE actual composition candidate/environment/scenarios.",
                    ));
                }
            }
            let basis = store.criterion_basis(&scope, integration_ref.as_deref())?;
            value
                .core_mut()
                .map_err(arguments)?
                .criterion_verifications
                .push(CriterionVerification {
                    scope,
                    candidate,
                    environment,
                    scenarios,
                    summary,
                    checks: checks
                        .into_iter()
                        .map(|c| Check::from_input(c, &at, &common.actor))
                        .collect(),
                    artifacts,
                    integration_ref,
                    basis,
                    actor: common.actor.clone(),
                    at: at.clone(),
                });
            "business criterion verified"
        }
        Work::Begin {} => {
            core_actor(
                &value,
                if value.id.starts_with("M-") {
                    AgentRole::Lead
                } else {
                    AgentRole::Integrator
                },
                &common.actor,
            )?;
            if let Some(i) = index {
                if value.modern() && !value.workflow.as_ref().is_some_and(|w| w.active) {
                    return Err(arguments("Begin the Module before child execution."));
                }
                if value.child(i).id.starts_with("A-") {
                    let a = value.child_mut(i);
                    let w = a.atomic_workflow.get_or_insert_with(AtomicWorkflow::new);
                    if w.active {
                        return Ok(ack(reference, before.version, store.phase(&value), false));
                    }
                    w.active = true;
                    w.started_at.get_or_insert(at.clone());
                    a.started_at.get_or_insert(at.clone());
                } else {
                    value.child_mut(i).started_at.get_or_insert(at.clone());
                }
            } else {
                let w = value.workflow_mut();
                w.managed = true;
                let missing = store.readiness(&value);
                if !missing.is_empty() {
                    return Err(Error::new(
                        "not_ready",
                        missing.into_iter().take(4).collect::<Vec<_>>().join(" "),
                    ));
                }
                if value.workflow.as_ref().is_some_and(|w| w.active) {
                    return Ok(ack(reference, before.version, store.phase(&value), false));
                }
                if value.core().is_some()
                    && value.id.starts_with("A-")
                    && !value.participants.is_empty()
                {
                    value.participant_basis = std::collections::BTreeMap::from([(
                        "core".into(),
                        store.coverage_basis(&value.participants)?,
                    )]);
                }
                let roster = value.modules.clone();
                let epic = value.id.starts_with("E-");
                let w = value.workflow_mut();
                w.active = true;
                w.started_at.get_or_insert(at.clone());
                if epic && w.core.is_none() {
                    w.frozen_modules.get_or_insert(roster);
                }
                if value.id.starts_with("M-") {
                    for a in &mut value.atomics {
                        a.atomic_workflow.get_or_insert_with(AtomicWorkflow::new);
                    }
                }
            }
            "begun"
        }
        Work::Complete {} => {
            running(store, &value, index)?;
            complete_local(&mut value, index, &common.actor)?;
            "local outcome completed"
        }
        Work::Deliver {
            target_branch,
            summary,
            artifact,
        } => {
            module_only(index)?;
            number(&value.id, "M-").map_err(arguments)?;
            if value.core().is_none() {
                running(store, &value, index)?;
            } else if !value.workflow.as_ref().is_some_and(|w| w.active) {
                return Err(arguments(
                    "Deliver only the current begun/reviewed candidate.",
                ));
            }
            input::field("target_branch", text(&target_branch, 128))?;
            input::field("summary", text(&summary, 1024))?;
            input::field("artifact", optional(&artifact, 256))?;
            if !value.modern() {
                return Err(arguments(
                    "Begin this Module to opt into reported delivery.",
                ));
            }
            let basis = value.basis().map_err(arguments)?;
            let reviewed_basis = store.work_basis(&value)?;
            let reviewed = value.reviews.last().is_some_and(|r| {
                r.verdict == Verdict::Accepted
                    && r.epoch == value.review_epoch
                    && r.basis == reviewed_basis
            });
            if !reviewed || !store.acceptance(&value).is_empty() {
                return Err(Error::new(
                    "acceptance",
                    "Independently review the current complete Module before delivery.",
                ));
            }
            if value
                .workflow
                .as_ref()
                .and_then(|w| w.execution.as_ref())
                .is_none_or(|e| e.target_branch != target_branch)
            {
                return Err(arguments(
                    "Delivery target must match execution.target_branch.",
                ));
            }
            value.workflow_mut().delivery = Some(Delivery {
                target_branch,
                summary,
                artifact,
                basis,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "delivery reported"
        }
        Work::ImportCommits { commits, state } => {
            core_actor(
                &value,
                if value.id.starts_with("M-") {
                    AgentRole::Lead
                } else {
                    AgentRole::Integrator
                },
                &common.actor,
            )?;
            running(store, &value, index)?;
            if value.core().is_some()
                && value.id.starts_with("A-")
                && !value.participants.is_empty()
            {
                let current = store.coverage_basis(&value.participants)?;
                if value
                    .participant_basis
                    .get("core")
                    .is_some_and(|claimed| claimed != &current)
                {
                    return Err(Error::new(
                        "integration_inputs_changed",
                        "The begun candidate/contract set changed; explicitly reopen/recheck before importing a late result.",
                    ));
                }
            }
            let imported =
                import_commits(&mut value, index, reference, &commits, &common.actor, &at)?;
            if imported && value.id.starts_with("A-") {
                value.participant_basis =
                    if value.core().is_some() && !value.participants.is_empty() {
                        std::collections::BTreeMap::from([(
                            "core".into(),
                            store.coverage_basis(&value.participants)?,
                        )])
                    } else {
                        store.participant_basis(&value.participants)?
                    };
            }
            if let Some(state) = state {
                set_completion(&mut value, index, state, &common.actor)?;
            }
            "commits imported"
        }
        Work::Result {
            summary,
            state,
            checks,
            gaps,
            followups,
            artifacts,
            candidate,
            changed_scope,
        } => {
            // Name the offending result field; the model validator alone states only the rule.
            input::field("summary", text(&summary, 1024))?;
            input::field("candidate", optional(&candidate, 256))?;
            input::field("changed_scope", strings(&changed_scope, 256, false))?;
            input::field("gaps", strings(&gaps, 256, false))?;
            input::field("followups", strings(&followups, 256, false))?;
            input::field("artifacts", strings(&artifacts, 256, false))?;
            running(store, &value, index)?;
            if value.core().is_some()
                && value.id.starts_with("A-")
                && !value.participants.is_empty()
            {
                let current = store.coverage_basis(&value.participants)?;
                if value
                    .participant_basis
                    .get("core")
                    .is_some_and(|claimed| claimed != &current)
                {
                    return Err(Error::new(
                        "integration_inputs_changed",
                        "The begun candidate/contract set changed; explicitly reopen/recheck instead of rebinding late completion.",
                    ));
                }
            }
            if value.core().is_some() {
                core_actor(
                    &value,
                    if value.id.starts_with("M-") {
                        AgentRole::Lead
                    } else if value.id.starts_with("A-") {
                        AgentRole::Integrator
                    } else {
                        AgentRole::Lead
                    },
                    &common.actor,
                )
                .or_else(|e| {
                    if value.id.starts_with("E-") {
                        Ok(())
                    } else {
                        Err(e)
                    }
                })?;
            }
            let report = Report {
                summary,
                candidate,
                changed_scope,
                gaps,
                followups,
                artifacts,
                reported_at: at.clone(),
                actor: common.actor.clone(),
            };
            let current_checks = checks
                .into_iter()
                .map(|c| Check::from_input(c, &at, &common.actor))
                .collect();
            if let Some(i) = index {
                let task = value.child_mut(i);
                task.result = Some(report);
                task.checks = current_checks;
                if let Some(state) = state {
                    set_completion(&mut value, index, state, &common.actor)?;
                }
            } else {
                if state.is_some() && !value.id.starts_with("A-") {
                    return Err(arguments(
                        "state belongs only to Task/Atomic results; Modules/Epics use independent review.",
                    ));
                }
                value.result = Some(report);
                value.checks = current_checks;
                if value.id.starts_with("A-") {
                    if let Some(state) = state {
                        set_completion(&mut value, index, state, &common.actor)?;
                    }
                    value.participant_basis =
                        if value.core().is_some() && !value.participants.is_empty() {
                            std::collections::BTreeMap::from([(
                                "core".into(),
                                store.coverage_basis(&value.participants)?,
                            )])
                        } else {
                            store.participant_basis(&value.participants)?
                        };
                    if value.completed && !value.modern() {
                        let missing = store.acceptance(&value);
                        if !missing.is_empty() {
                            return Err(Error::new(
                                "acceptance",
                                missing.into_iter().take(3).collect::<Vec<_>>().join(" "),
                            ));
                        }
                    }
                }
            }
            "result reported"
        }
        Work::Blocker {
            problem,
            needed_action,
            resolver,
        } => {
            module_only(index)?;
            value.blocker = Some(Blocker {
                problem,
                needed_action,
                resolver,
            });
            "blocker set"
        }
        Work::Handoff {
            stopping_point,
            next_action,
        } => {
            module_only(index)?;
            value.handoff = Some(Handoff {
                stopping_point,
                next_action,
            });
            "handoff set"
        }
        Work::ClearBlocker { reason } => {
            module_only(index)?;
            input::field("reason", text(&reason, 512))?;
            if value.blocker.take().is_none() {
                return Ok(ack(reference, before.version, store.phase(&value), false));
            }
            value.reasons.push(Reason {
                target: reference.into(),
                action: "blocker cleared".into(),
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "blocker cleared"
        }
        Work::ClearHandoff { reason } => {
            module_only(index)?;
            input::field("reason", text(&reason, 512))?;
            if value.handoff.take().is_none() {
                return Ok(ack(reference, before.version, store.phase(&value), false));
            }
            value.reasons.push(Reason {
                target: reference.into(),
                action: "handoff cleared".into(),
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "handoff cleared"
        }
        Work::Cancel { reason } => {
            input::field("reason", text(&reason, 512))?;
            let cancellation = Cancellation {
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            };
            if let Some(i) = index {
                value.child_mut(i).state = TaskState::Canceled;
                value.child_mut(i).cancellation = Some(cancellation);
            } else {
                if value.children().any(|t| t.state == TaskState::Open) {
                    return Err(Error::new(
                        "open_children",
                        "Complete or cancel children explicitly before canceling their module.",
                    ));
                }
                if value.id.starts_with("E-") {
                    store.membership(&value)?;
                    for id in value.modules.iter().chain(&value.atomic_members) {
                        let child = store.module(id)?.value;
                        if !matches!(
                            store.phase(&child),
                            "accepted" | "ready for integration" | "done" | "canceled"
                        ) {
                            return Err(Error::new(
                                "open_children",
                                format!("Resolve {id} before canceling Epic; no cascade."),
                            ));
                        }
                    }
                }
                value.state = ModuleState::Canceled;
                value.cancellation = Some(cancellation);
            }
            "canceled"
        }
        Work::Reopen { reason } => {
            input::field("reason", text(&reason, 512))?;
            if let Some(i) = index {
                let task = value.child_mut(i);
                task.state = TaskState::Open;
                if let Some(w) = &mut task.atomic_workflow {
                    w.active = false;
                }
                if let Some(c) = task.cancellation.take() {
                    task.cancellation_history.push(c);
                }
            } else {
                value.state = ModuleState::Open;
                value.completed = false;
                if let Some(w) = &mut value.workflow {
                    w.active = false;
                    w.delivery = None;
                }
                if let Some(c) = value.cancellation.take() {
                    value.cancellation_history.push(c);
                }
            }
            value.reasons.push(Reason {
                target: reference.into(),
                action: "reopened".into(),
                reason,
                at: at.clone(),
                actor: common.actor.clone(),
            });
            "reopened"
        }
    };
    if let Some(i) = index
        && value.child(i) != before.value.child(i)
    {
        value.child_mut(i).updated_at = at;
    }
    save_module(
        store,
        &before,
        value,
        reference,
        action,
        &common.actor,
        effects,
    )
}

/// Require a current reported lifecycle before semantic outcome writes, without launching anything.
fn running(store: &Store, m: &Module, index: Option<usize>) -> Result<()> {
    if let Some(core) = m.core().filter(|_| {
        m.id.starts_with("M-") || (m.id.starts_with("A-") && !m.participants.is_empty())
    }) {
        let role = if m.id.starts_with("M-") {
            AgentRole::Lead
        } else {
            AgentRole::Integrator
        };
        let actor = core
            .binding(role)
            .map(|b| b.current.identity.agent_id.clone());
        core.actor(role, &actor).map_err(arguments)?;
    }
    if m.modern() {
        if !m.workflow.as_ref().is_some_and(|w| w.active) {
            return Err(Error::new(
                "not_started",
                "Report begin before recording execution outcomes.",
            ));
        }
        if let Some(parent) = store.parent(&m.id)?
            && parent.modern()
            && !parent.workflow.as_ref().is_some_and(|w| w.active)
        {
            return Err(Error::new(
                "not_started",
                "Begin the Epic parent before child outcomes.",
            ));
        }
    }
    if let Some(i) = index
        && m.child(i)
            .atomic_workflow
            .as_ref()
            .is_some_and(|w| !w.active)
    {
        return Err(Error::new(
            "not_started",
            "Report Atomic begin before its outcome.",
        ));
    }
    Ok(())
}

/// Apply an explicit local state decision; imported commits and successful tests never invoke it implicitly.
fn set_completion(
    m: &mut Module,
    index: Option<usize>,
    state: Completion,
    actor: &Option<String>,
) -> Result<()> {
    if matches!(state, Completion::Done) {
        return complete_local(m, index, actor);
    }
    if let Some(i) = index {
        if m.modern() && m.child(i).state == TaskState::Done {
            return Err(arguments("Use reasoned reopen to undo local completion."));
        }
        m.child_mut(i).state = TaskState::Open;
    } else if m.id.starts_with("A-") {
        if m.modern() && m.completed {
            return Err(arguments("Use reasoned reopen to undo local completion."));
        }
        m.completed = false;
    } else {
        return Err(arguments(
            "Only Task/Atomic results have local completion state.",
        ));
    }
    Ok(())
}

/// Mark a meaningful local result done; modern Task decisions must be declared by their Module lead.
fn complete_local(m: &mut Module, index: Option<usize>, actor: &Option<String>) -> Result<()> {
    if m.id.starts_with("A-") && !m.participants.is_empty() {
        core_actor(m, AgentRole::Integrator, actor)?;
    }
    if let Some(i) = index {
        if m.child(i).result.is_none() {
            return Err(arguments(
                "Record a meaningful result before local completion.",
            ));
        }
        if m.core().is_some() && m.child(i).id.starts_with("T-") {
            core_actor(m, AgentRole::Lead, actor)?;
        }
        if m.core().is_none()
            && m.modern()
            && m.child(i).id.starts_with("T-")
            && !actor.as_ref().is_some_and(|a| {
                m.lead
                    .as_ref()
                    .is_some_and(|l| l.name == *a || l.handle.as_ref() == Some(a))
            })
        {
            return Err(Error::new(
                "lead_required",
                "The declared Module lead decides Task done after tests or manual verification.",
            ));
        }
        m.child_mut(i).state = TaskState::Done;
    } else {
        number(&m.id, "A-").map_err(arguments)?;
        if m.result.is_none() {
            return Err(arguments(
                "Record a meaningful Atomic result before local completion.",
            ));
        }
        m.completed = true;
    }
    Ok(())
}

/// Import a complete explicit commit set after all reads succeed; owner history deduplicates immutable sources.
fn import_commits(
    m: &mut Module,
    index: Option<usize>,
    reference: &str,
    commits: &[String],
    actor: &Option<String>,
    at: &str,
) -> Result<bool> {
    let e = m
        .workflow
        .as_ref()
        .and_then(|w| w.execution.as_ref())
        .ok_or_else(|| {
            arguments("Declare repository/worktree/branch execution before importing commits.")
        })?;
    let observed = crate::git_reports::read_verified_commits(
        std::path::Path::new(&e.repository),
        std::path::Path::new(&e.worktree),
        &e.branch,
        commits,
    )?;
    let changed = observed.iter().any(|c| {
        !m.imports.iter().any(|i| {
            i.commit.repository == c.repository
                && i.commit.sha == c.sha
                && i.targets.iter().any(|t| t == reference)
        })
    });
    if !changed {
        return Ok(false);
    }
    let summary = observed
        .iter()
        .map(|c| c.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    input::field("summary", text(&summary, 1024))?;
    let mut checks = std::collections::BTreeMap::new();
    let mut gaps = Vec::new();
    let mut followups = Vec::new();
    for c in &observed {
        for check in &c.checks {
            checks.insert(check.label.clone(), check.clone());
        }
        gaps.extend(c.gaps.clone());
        followups.extend(c.followups.clone());
    }
    input::field("gaps", strings(&gaps, 256, false))?;
    input::field("followups", strings(&followups, 256, false))?;
    if checks.len() > 8 {
        return Err(arguments(
            "Combined imported check set exceeds eight labels; import a bounded complete report.",
        ));
    }
    let artifacts = observed
        .iter()
        .map(|c| format!("commit:{}", c.sha))
        .collect::<Vec<_>>();
    let report = Report {
        summary,
        candidate: if m.core().is_some() {
            observed.last().map(|c| c.sha.clone())
        } else {
            None
        },
        changed_scope: Vec::new(),
        gaps,
        followups,
        artifacts,
        reported_at: at.into(),
        actor: actor.clone(),
    };
    let checks = checks
        .into_values()
        .map(|c| Check::from_input(c, at, actor))
        .collect();
    for commit in observed {
        if let Some(existing) = m
            .imports
            .iter_mut()
            .find(|i| i.commit.repository == commit.repository && i.commit.sha == commit.sha)
        {
            if !existing.targets.iter().any(|t| t == reference) {
                existing.targets.push(reference.into());
            }
        } else {
            m.imports.push(ImportedCommit {
                commit,
                targets: vec![reference.into()],
            });
        }
    }
    if let Some(i) = index {
        m.child_mut(i).result = Some(report);
        m.child_mut(i).checks = checks;
    } else {
        m.result = Some(report);
        m.checks = checks;
    }
    m.validate().map_err(arguments)?;
    Ok(true)
}

/// Independently review one embedded Atomic and publish only its owning Module file.
fn review_atomic(
    store: &Store,
    before: &Snapshot<Module>,
    args: ReviewArgs,
    index: usize,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let mut value = before.value.clone();
    running(store, &value, Some(index))?;
    let a = value.child(index);
    if a.state == TaskState::Canceled {
        return Err(arguments("Reopen canceled Atomic before review."));
    }
    if a.atomic_workflow.is_none() {
        return Err(arguments("Report Atomic begin before independent review."));
    }
    if args.actor.as_ref().is_some_and(|actor| {
        a.executor
            .iter()
            .chain(value.lead.iter())
            .any(|l| l.name == *actor || l.handle.as_ref() == Some(actor))
            || a.result.as_ref().and_then(|r| r.actor.as_ref()) == Some(actor)
    }) {
        return Err(Error::new(
            "self_review",
            "Known Atomic executor/author cannot review their own result.",
        ));
    }
    input::field("summary", text(&args.summary, 1024))?;
    if args.findings.len() > 8
        || args.checks.len() > 8
        || args.verdict == Verdict::Accepted && args.findings.iter().any(|f| f.must_fix)
    {
        return Err(arguments("Invalid Atomic review findings/check limits."));
    }
    let at = store::now();
    let mut updates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for input in args.checks {
        if input.target != args.module || !seen.insert(input.label.clone()) {
            return Err(arguments(
                "Atomic review updates must target only this Atomic, with unique labels.",
            ));
        }
        let after = Check::from_input(
            CheckInput {
                label: input.label.clone(),
                status: input.status,
                detail: input.detail,
            },
            &at,
            &args.actor,
        );
        let checks = &mut value.child_mut(index).checks;
        let previous = checks.iter().position(|c| c.label == input.label);
        let original = previous.map(|i| checks[i].clone());
        if let Some(i) = previous {
            checks[i] = after.clone();
        } else {
            checks.push(after.clone());
        }
        updates.push(CheckUpdate {
            target: input.target,
            label: input.label,
            before: original,
            after,
        });
    }
    if args.verdict == Verdict::Accepted {
        let missing = value.child(index).atomic_acceptance();
        if !missing.is_empty() {
            return Err(Error::new("acceptance", missing.join(" ")));
        }
    }
    let basis = value.child(index).atomic_basis().map_err(arguments)?;
    let core_policy = value.core().is_some();
    let w = value
        .child_mut(index)
        .atomic_workflow
        .as_mut()
        .ok_or_else(|| arguments("Missing Atomic lifecycle."))?;
    w.review_epoch = w
        .review_epoch
        .checked_add(1)
        .ok_or_else(|| arguments("Atomic review generation exhausted."))?;
    w.reviews.push(Review {
        verdict: args.verdict,
        summary: args.summary,
        findings: args.findings,
        basis,
        epoch: w.review_epoch,
        at,
        reviewer: args.actor.clone(),
        check_updates: updates,
        candidate: None,
        contracts: std::collections::BTreeMap::new(),
        changed_scope: args.changed_scope,
        resolved_findings: args.resolved_findings,
        core_policy,
    });
    save_module(
        store,
        before,
        value,
        &args.module,
        "Atomic review recorded",
        &args.actor,
        effects,
    )
}

/// Validate persistent role ownership; Tasks do not acquire separate role/review ceremonies.
fn role_owner(m: &Module, role: AgentRole) -> Result<()> {
    if m.core().is_none() {
        return Err(arguments("Explicitly adopt_core before new coordination."));
    }
    let allowed = match role {
        AgentRole::Lead => m.id.starts_with("M-"),
        AgentRole::Reviewer => m.id.starts_with("M-") || m.id.starts_with("A-"),
        AgentRole::Integrator => m.id.starts_with("A-"),
    };
    if !allowed {
        return Err(arguments(
            "Persistent role does not belong to this owner kind.",
        ));
    }
    Ok(())
}
/// Require the actual assigned usable ID for new scoped actions; do not authenticate a guessed label.
fn core_actor(m: &Module, role: AgentRole, actor: &Option<String>) -> Result<()> {
    if let Some(core) = m.core().filter(|_| {
        m.id.starts_with("M-") || (m.id.starts_with("A-") && !m.participants.is_empty())
    }) {
        core.actor(role, actor)
            .map_err(|e| Error::new("agent_binding", e))?;
    }
    Ok(())
}
/// Validate significant canonical revisions before publication while preserving staged reciprocal updates.
/// Provider definitions/artifacts belong to the canonical ID across consumers and provider transfers;
/// consumer-specific obligations retain their own peer history. Removal never recycles either revision.
fn contract_edit(store: &Store, before: &Module, after: &mut Module) -> Result<()> {
    if after.core().is_none() {
        return Ok(());
    }
    if before.workflow.as_ref().and_then(|w| w.contracts.as_ref())
        == after.workflow.as_ref().and_then(|w| w.contracts.as_ref())
    {
        return Ok(());
    }
    let inventory = store.scan(None)?;
    if !inventory.complete {
        return Err(arguments(
            "Canonical revision history is unknown in incomplete inventory.",
        ));
    }
    if let Some(c) = after.workflow.as_ref().and_then(|w| w.contracts.as_ref()) {
        for (provides, new) in c
            .provides
            .iter()
            .map(|e| (true, e))
            .chain(c.consumes.iter().map(|e| (false, e)))
        {
            let id = new
                .id
                .as_ref()
                .ok_or_else(|| arguments("Core boundary needs canonical id."))?;
            text(id, 64).map_err(arguments)?;
            if id == "local" || new.revision.is_none_or(|r| r == 0) {
                return Err(arguments(
                    "Declare positive revision; local is reserved for non-cross-boundary controls.",
                ));
            }
            let revision = new.revision.unwrap_or_default();
            for record in &inventory.modules {
                let owner = &record.value;
                let Some(core) = owner.core() else {
                    continue;
                };
                let current = owner.workflow.as_ref().and_then(|w| w.contracts.as_ref());
                let floor = core
                    .agreements
                    .iter()
                    .filter(|a| a.contract_id == *id)
                    .map(|a| a.revision)
                    .chain(
                        core.boundary_evidence
                            .iter()
                            .filter(|e| e.contract_id == *id)
                            .map(|e| e.revision),
                    )
                    .chain(
                        current
                            .into_iter()
                            .chain(core.contract_history.iter())
                            .flat_map(|c| c.provides.iter().chain(&c.consumes))
                            .filter(|e| e.id.as_ref() == Some(id))
                            .filter_map(|e| e.revision),
                    )
                    .max()
                    .unwrap_or_default();
                if revision < floor {
                    return Err(arguments(
                        "Canonical revision cannot decrease below retained boundary history.",
                    ));
                }
                if provides {
                    for old in current
                        .into_iter()
                        .chain(core.contract_history.iter())
                        .flat_map(|c| c.provides.iter())
                        .filter(|e| e.id == new.id)
                    {
                        if new.revision <= old.revision
                            && (new.description != old.description
                                || new.reference != old.reference)
                        {
                            return Err(arguments(
                                "Affecting provider definition/artifact changes require a greater retained canonical revision, independent of consumer or provider.",
                            ));
                        }
                    }
                }
            }
            for old in before
                .workflow
                .as_ref()
                .and_then(|w| w.contracts.as_ref())
                .into_iter()
                .chain(
                    before
                        .core()
                        .into_iter()
                        .flat_map(|c| c.contract_history.iter()),
                )
                .flat_map(|c| {
                    if provides {
                        c.provides.iter()
                    } else {
                        c.consumes.iter()
                    }
                })
                .filter(|e| e.id == new.id && e.peer == new.peer)
            {
                if new.revision <= old.revision
                    && (new.description != old.description || new.reference != old.reference)
                {
                    return Err(arguments(
                        "Affecting boundary changes require a greater retained canonical revision.",
                    ));
                }
            }
            if let Some(old) = before
                .workflow
                .as_ref()
                .and_then(|w| w.contracts.as_ref())
                .and_then(|c| {
                    (if provides { &c.provides } else { &c.consumes })
                        .iter()
                        .find(|e| e.id == new.id && e.peer == new.peer)
                })
            {
                let affecting =
                    old.description != new.description || old.reference != new.reference;
                if new.revision < old.revision || affecting && new.revision <= old.revision {
                    return Err(arguments(
                        "Affecting boundary changes require a greater canonical revision.",
                    ));
                }
            }
        }
    }
    let old = before.workflow.as_ref().and_then(|w| w.contracts.as_ref());
    let new = after.workflow.as_ref().and_then(|w| w.contracts.as_ref());
    if old != new
        && let Some(old) = old.filter(|c| {
            c.provides
                .iter()
                .chain(&c.consumes)
                .any(|e| e.id.is_some() && e.revision.is_some())
        })
    {
        let history = &mut after.core_mut().map_err(arguments)?.contract_history;
        if !history.contains(old) {
            if history.len() >= 16 {
                return Err(arguments("Retained contract history capacity exhausted."));
            }
            let mut retained = old.clone();
            retained
                .provides
                .retain(|e| e.id.is_some() && e.revision.is_some());
            retained
                .consumes
                .retain(|e| e.id.is_some() && e.revision.is_some());
            history.push(retained);
        }
    }
    Ok(())
}
/// Freeze prepared negotiated roster without demanding acyclic completion prerequisites already satisfied.
fn freeze_epic(store: &Store, value: &mut Module) -> Result<()> {
    number(&value.id, "E-").map_err(arguments)?;
    value.core_mut().map_err(arguments)?;
    store.membership(value)?;
    store.links(value)?;
    let mut checkouts = std::collections::BTreeSet::new();
    for id in &value.modules {
        let m = store.module(id)?.value;
        if m.state == ModuleState::Canceled {
            continue;
        }
        let c = m
            .core()
            .ok_or_else(|| arguments(format!("{id}: explicitly adopt core planning.")))?;
        let actor = c
            .binding(AgentRole::Lead)
            .map(|b| b.current.identity.agent_id.clone());
        c.actor(AgentRole::Lead, &actor).map_err(arguments)?;
        if !c
            .planning
            .as_ref()
            .is_some_and(|p| m.plan_basis().is_ok_and(|b| b == p.basis))
        {
            return Err(arguments(format!(
                "{id}: current bound lead discovery required."
            )));
        }
        let gaps = store.agreement_gaps(&m);
        if !gaps.is_empty() {
            return Err(Error::new("contract_not_agreed", gaps.join(" ")));
        }
        let execution = m
            .workflow
            .as_ref()
            .and_then(|w| w.execution.as_ref())
            .ok_or_else(|| arguments(format!("{id}: prepared separate checkout required.")))?;
        if !checkouts.insert(execution.worktree.clone()) {
            return Err(arguments(
                "Module implementation checkouts must be distinct.",
            ));
        }
    }
    let core = value.core().ok_or_else(|| arguments("Missing core."))?;
    if core.criterion_scopes.len() != value.criteria.len()
        || value.criteria.iter().enumerate().any(|(i, t)| {
            !core
                .criterion_scopes
                .iter()
                .any(|s| s.index == i && s.text == *t)
        })
    {
        return Err(arguments(
            "Declare every zero-based business criterion's exact affected Module set.",
        ));
    }
    let roster = value.modules.clone();
    value.workflow_mut().frozen_modules.get_or_insert(roster);
    Ok(())
}

/// Refuse module-only blocker/handoff writes to tasks rather than silently rerouting them.
fn module_only(index: Option<usize>) -> Result<()> {
    if index.is_some() {
        return Err(arguments("This operation requires a module reference."));
    }
    Ok(())
}

/// Verify stable finding resolutions and changed-scope continuation without discarding predecessor history.
fn core_review_guard(m: &Module, args: &mut ReviewArgs, target: Option<usize>) -> Result<()> {
    let Some(core) = m.core() else {
        return Ok(());
    };
    if m.id.starts_with("M-") || (m.id.starts_with("A-") && !m.participants.is_empty()) {
        core.actor(AgentRole::Reviewer, &args.actor)
            .map_err(arguments)?;
        let reviewer = &core
            .binding(AgentRole::Reviewer)
            .ok_or_else(|| arguments("Bind reviewer."))?
            .current
            .identity;
        for role in [AgentRole::Lead, AgentRole::Integrator] {
            if core.binding(role).is_some_and(|b| {
                b.current.identity.harness == reviewer.harness
                    && b.current.identity.agent_id == reviewer.agent_id
            }) {
                return Err(Error::new(
                    "self_review",
                    "The bound author/integrator cannot independently review its own candidate.",
                ));
            }
        }
    }
    if target.is_none()
        && (m.id.starts_with("M-") || (m.id.starts_with("A-") && !m.participants.is_empty()))
        && m.result
            .as_ref()
            .and_then(|r| r.candidate.as_ref())
            .is_none()
    {
        return Err(arguments(
            "Submit a definite candidate before any core Module/integration review.",
        ));
    }
    let reviews = target.map_or(m.reviews.as_slice(), |i| {
        m.child(i)
            .atomic_workflow
            .as_ref()
            .map_or(&[], |w| w.reviews.as_slice())
    });
    let report = target.map_or(m.result.as_ref(), |i| m.child(i).result.as_ref());
    input::field("changed_scope", strings(&args.changed_scope, 256, false))?;
    if args.resolved_findings.len() > 8 {
        return Err(arguments(
            "At most eight stable finding resolutions per round.",
        ));
    }
    let mut resolving = std::collections::BTreeSet::new();
    for resolution in &args.resolved_findings {
        input::field("summary", text(&resolution.summary, 512))?;
        let finding = reviews
            .get(resolution.review_index)
            .and_then(|r| r.findings.get(resolution.finding_index))
            .ok_or_else(|| arguments("Unknown retained zero-based finding reference."))?;
        if !finding.must_fix
            || !resolving.insert((resolution.review_index, resolution.finding_index))
        {
            return Err(arguments("Resolve each required prior finding once."));
        }
    }
    if reviews
        .iter()
        .any(|r| r.core_policy || r.candidate.is_some())
    {
        if args.changed_scope.is_empty() {
            args.changed_scope = report.map(|r| r.changed_scope.clone()).unwrap_or_default();
        }
        if args.changed_scope.is_empty() {
            return Err(arguments(
                "Follow-up review needs changed scope or a concrete verification gap; retain the same reviewer.",
            ));
        }
    }
    if args.verdict == Verdict::Accepted {
        let mut resolved = std::collections::BTreeSet::new();
        for review in reviews {
            for r in &review.resolved_findings {
                resolved.insert((r.review_index, r.finding_index));
            }
        }
        resolved.extend(resolving);
        for (ri, review) in reviews
            .iter()
            .enumerate()
            .filter(|(_, r)| r.core_policy || r.candidate.is_some())
        {
            for (fi, finding) in review.findings.iter().enumerate() {
                if finding.must_fix && !resolved.contains(&(ri, fi)) {
                    return Err(Error::new(
                        "review_findings",
                        format!(
                            "Retained finding review_index={ri} finding_index={fi} still requires independent resolution."
                        ),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Independently review the whole module, applying new check reports before basis/epoch capture.
fn review(
    store: &Store,
    _guard: &LockGuard,
    mut args: ReviewArgs,
    effects: &mut Vec<String>,
) -> Result<Ack> {
    let id = module_id(&args.module)?.to_owned();
    let before = current(store, &id, &args.version)?;
    let mut value = before.value.clone();
    open_module(&value)?;
    if value.core().is_some() && !value.id.starts_with("E-") {
        core_actor(&value, AgentRole::Reviewer, &args.actor)?;
    }
    input::field("changed_scope", strings(&args.changed_scope, 256, false))?;
    let target_index = value.target(&args.module).map_err(arguments)?;
    core_review_guard(&value, &mut args, target_index)?;
    if let Some(i) = target_index {
        if !value.child(i).id.starts_with("A-") {
            return Err(arguments("Tasks have no individual review."));
        }
        return review_atomic(store, &before, args, i, effects);
    }
    if value.id.starts_with("A-") && !value.modern() {
        return Err(arguments(
            "Report Atomic begin before opting into independent review.",
        ));
    }
    if (value.core().is_none() || (value.id.starts_with("A-") && value.participants.is_empty()))
        && args.actor.as_ref().is_some_and(|a| {
            value
                .lead
                .as_ref()
                .is_some_and(|l| &l.name == a || l.handle.as_ref() == Some(a))
                || value.id.starts_with("A-")
                    && value.result.as_ref().and_then(|r| r.actor.as_ref()) == Some(a)
        })
    {
        return Err(Error::new(
            "self_review",
            "A known lead cannot independently review the same module.",
        ));
    }
    input::field("summary", text(&args.summary, 1024))?;
    if args.findings.len() > 8 || args.checks.len() > 8 {
        return Err(arguments(
            "At most eight findings/check updates per verdict.",
        ));
    }
    if args.verdict == Verdict::Accepted && args.findings.iter().any(|f| f.must_fix) {
        return Err(Error::new(
            "acceptance",
            "Accepted verdict cannot contain must-fix findings.",
        ));
    }
    let at = store::now();
    let mut updates = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for input in args.checks {
        if !seen.insert((input.target.clone(), input.label.clone())) {
            return Err(arguments("Duplicate review check update."));
        }
        let index = value.target(&input.target).map_err(arguments)?;
        if index.is_some_and(|i| value.child(i).state == TaskState::Canceled) {
            return Err(Error::new(
                "canceled",
                "Review cannot change checks on canceled tasks.",
            ));
        }
        let checks = if let Some(i) = index {
            &mut value.child_mut(i).checks
        } else {
            &mut value.checks
        };
        let after = Check::from_input(
            CheckInput {
                label: input.label.clone(),
                status: input.status,
                detail: input.detail,
            },
            &at,
            &args.actor,
        );
        let previous = checks.iter().position(|c| c.label == input.label);
        let original = previous.map(|i| checks[i].clone());
        if let Some(i) = previous {
            checks[i] = after.clone();
        } else {
            checks.push(after.clone());
        }
        updates.push(CheckUpdate {
            target: input.target,
            label: input.label,
            before: original,
            after,
        });
    }
    value.validate().map_err(arguments)?;
    if args.verdict == Verdict::Accepted {
        let missing = store.acceptance(&value);
        if !missing.is_empty() {
            return Err(Error::new(
                "acceptance",
                format!(
                    "{} Read get_context for remaining conditions.",
                    missing.into_iter().take(3).collect::<Vec<_>>().join(" ")
                ),
            ));
        }
    }
    value.review_epoch = value
        .review_epoch
        .checked_add(1)
        .ok_or_else(|| store::invalid("Review epoch exhausted."))?;
    let basis = store.work_basis(&value)?;
    value.reviews.push(Review {
        verdict: args.verdict,
        summary: args.summary,
        findings: args.findings,
        basis,
        epoch: value.review_epoch,
        at: at.clone(),
        reviewer: args.actor.clone(),
        check_updates: updates,
        candidate: value.result.as_ref().and_then(|r| r.candidate.clone()),
        contracts: store
            .contract_ids(&value)
            .into_iter()
            .filter_map(|id| store.contract_facts(&id).ok().map(|f| (id, f.revision)))
            .collect(),
        changed_scope: args.changed_scope,
        resolved_findings: args.resolved_findings,
        core_policy: value.core().is_some(),
    });
    value
        .event(&args.module, "review recorded", &at, &args.actor)
        .map_err(store::invalid)?;
    value.validate().map_err(arguments)?;
    store.save(
        &work_path(&args.module).map_err(arguments)?,
        &value,
        Some(&before.bytes),
        args.verdict == Verdict::Accepted,
        effects,
    )?;
    let version = store.work_version(&value, &store::encode(&value)?)?;
    Ok(ack(args.module, version, store.phase(&value), true))
}
