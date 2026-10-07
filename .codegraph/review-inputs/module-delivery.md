# Module ownership and independent review

A task is a bounded implementation and commit unit. A module is a coherent system
responsibility and the default independent-review unit. Local task completion
lets implementation continue; it does not establish module acceptance.

Use [verification by phase](verification.md) for the module's check plan, targeted
selectors, evidence reuse and justified broader regression. Tests and review are
not automatic consequences of every patch, commit or task checkpoint.

## Keep one lead with the module

After boundaries and contracts are agreed, the orchestrator assigns one module
lead with the complete bounded implementation plan, relevant business criteria,
provided/consumed contracts and prepared starting worktree. The lead keeps module
context across its tasks and review fixes. Reuse the same session and prepared
worktree where the runtime supports it; do not relaunch a fresh coder or create a
new worktree just because the next task starts.

The lead implements coherent tasks, obtains current scoped check evidence under
the verification plan (reusing valid results), and commits each result under
`project-commit`, and records BASE/COMMIT/check evidence. It can close a task
locally and continue without a separate external review of every task. A task's
local completion is not independent review and does not close the module. Preserve
internal unit tests, contract checks and early integration throughout development.

The lead works within the approved design. A needed contract change returns to
bilateral negotiation immediately; it does not wait for final module review.
An unresolved material design or safety decision may need an early bounded review,
but do not silently turn that exception into external approval for every task.

## Add helpers only for useful independent work

A module normally has one lead. The orchestrator may allocate one or two helpers
for bounded implementation, tests or investigation when that improves progress.
Helpers are optional, not three mandatory agents multiplied by every module.
Use the installed delegation route for their actual role. All leads and helpers
count against the same current host/runtime/global concurrency limits.

The orchestrator approves the helper slots, scope and permissions. A lead may
coordinate only the allocated team; it must not create an unbounded nested tree.
Each writing helper has disjoint ownership and its own prepared worktree from an
agreed committed baseline. Shared mutable files or tests against another writer's
live directory do not become safe because both agents belong to one module.

Helpers return committed results and check evidence to the lead. The lead inspects
the scoped changes, integrates them into its module branch, resolves conflicts
within scope, and obtains the affected combined check evidence, running only missing
or invalidated checks. This is local integration,
not a mandatory separate independent-review round. Preserve the existing Git
write-boundary fallback when a worker cannot commit: the designated committer
must produce the task commit before local completion or reuse of that candidate.

## Keep context recoverable

The orchestrator keeps the module's authoritative work record current from compact
lead checkpoints: responsibility and contracts, agent/session/worktree identities,
helper ownership, completed task hashes and checks, integration decisions, open
issues and next steps. Use the host's existing work-state system; do not put
private coordination records into product source, documentation or Git messages.

A continuing session is an optimization, not the only copy of this information.
If the runtime cannot resume it or context is exhausted, preserve that record and
resume/reassign the same module responsibility from the committed state. Report
the actual capability limit; do not assume a resume feature or pretend agent
identity survived. Review fixes return to the existing lead whenever possible.

## Submit one module revision with evidence

The lead submits:

- repository and worktree, module starting BASE and final full COMMIT object ID;
- all completed task commits and their mapping to the integrated result, including
  helper commits that were cherry-picked or otherwise received new IDs;
- coverage of module requirements and the relevant business acceptance criteria;
- canonical provided/consumed contracts and their exact agreed revisions;
- commands and observed results for internal unit tests, boundary/contract checks
  and real integration, tied to the submitted revision and actual source tree;
- known limitations, unresolved issues and checks performed only on substitutes.

The submitted COMMIT contains the integrated module result, not several unrelated
helper heads or uncommitted files. Review uses its diff from module BASE and the
necessary context. A green test written alongside a change is evidence to inspect,
not proof that the requirements or contract were interpreted correctly.

## Independently review and accept the module

For real cross-module checks, the orchestrator may assemble committed but not yet
accepted module results in a disposable integration candidate. Record every input
commit, the combined revision and relevant evidence. This is a test assembly,
not promotion to the accepted/main target. See candidate write safety for the
boundary; never require acceptance as a prerequisite for building the very
candidate needed to establish acceptance.

Use an independent reviewer who did not author the module. Read the current
`delegation_guide` before selecting a route; a different model family is required
when the assignment calls for cross-family independence. A helper who implemented
or integrated part of this module is not its independent reviewer. Follow an
explicit owner restriction on delegation; report when independent review is
unavailable rather than labelling the lead's self-check independent.

Review the fixed revision's public behaviour, provided/consumed contracts, internal
correctness, test adequacy and composition, including callers and relevant error
paths. Read the submitted evidence and inspect the code; do not accept the module
by summing task reports. The review role stays read-only and requests any execution
it cannot perform from the orchestrator. Findings require concrete evidence.

The orchestrator owns final acceptance after independent review and required
checks. No accepted module while required real integration or blocking findings
remain. Fully completed task lists may therefore coexist with an unaccepted
module. Module acceptance is separate from final end-to-end business acceptance.

Require complete declared review coverage and complete blocker accounting. A short
top-ten summary or a fix-only pass is insufficient while initial scope/findings
remain unaccounted for. Use the same review record across rounds; a missing item
does not silently become resolved.

Return findings to the module lead. Fixes produce new commits; follow-up review
checks the changed diff since the previous review and the named findings, using
only context required to verify those changes. Do not re-audit unchanged code or
expand the scope on every round.

Standalone tasks and explicitly requested focused reviews remain valid; do not
invent a module or a team just to apply this workflow. Local commits do not imply
permission to publish, push or broaden the task.
