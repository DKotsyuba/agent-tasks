# Verification by development phase

A check answers a concrete question about changed behaviour. Its value comes from
coverage and trustworthy evidence, not repeated execution. Keep local safety,
behaviour tests, module review and system acceptance distinct. Do not add a new
test framework, automatic test cache or evidence database for this workflow.

## Plan once for the module

The module lead maps the changed behaviour, callers/consumers and contracts to
existing checks during preparation. Record a compact verification plan in the
existing authoritative module record: purpose, actual scope/selectors and command,
phase/trigger, executor, relevant source/runtime/configuration, expected outcome,
and a reference to the latest result. Record duration when observed, never invent
it. Reuse this plan across tasks and revise only affected entries as facts change.
Use the configured project runtime and installed runtime-selection rules. A result
from another interpreter/dependency set is not a comparable baseline; do not
silently switch environments merely to turn a failing check green.

Read project-required checks and existing hooks/CI to distinguish mandatory gates
from example commands. Do not copy a README's complete test command into every
small task brief. A supplied command still needs a justified scope and phase.
If an explicit required gate conflicts with the intended plan, resolve the rule
rather than silently skipping it. Existing safety, security and data-loss checks
remain binding.

| Phase | Trigger and required evidence |
|---|---|
| Module preparation | Inspect the relevant baseline once when needed to distinguish existing failures; verify tool/runtime/source resolution. No automatic full-suite baseline. |
| Development | Make a coherent batch of related edits; inspect actual file changes and available editor/writer diagnostics. Run a focused test when it answers a debugging question or verifies a completed behaviour, not after every keystroke, patch or tool call. |
| Local task completion | Supply the smallest adequate check evidence for the changed behaviour and affected consumers, then commit. Reuse an already valid result; do not repeat it because the task is closing. Documentation/comment-only work may need syntax, formatting or document checks without runtime tests. |
| Module assembly | The lead verifies the integrated candidate's module/contract tests and affected real connections. Execute missing or invalidated checks; retain valid earlier results. Combining helper commits is not permission to ignore new interaction risk. |
| Independent module review | Review code, contract interpretation, coverage and actual execution evidence at the submitted revision. Request only missing/invalid evidence or a concrete challenge; a new reviewer does not automatically rerun every test. |
| System integration/release | Run affected cross-module scenarios and the required broader regression on the combined candidate. A full suite belongs here when required or justified by the scope; a new functional change may invalidate relevant evidence. |

Tasks should represent coherent verifiable outcomes, not arbitrary tiny edits
that create artificial test or review barriers. Retain one lead/session and the
prepared module worktree where supported. Task commits/checkpoints do not require
new coder sessions, rereading unchanged code or independent task-by-task review.
A new supervisor job ID alone does not prove a new runtime session.

## Choose the smallest adequate scope

Start from the changed behaviour, not merely the edited filenames. Include
relevant callers, shared types, consumers, fixtures and error/contract paths.
Prefer named test cases, a test file or the affected package/module. Expand only
when the narrower set cannot establish the required property.

Build/typecheck is a check with its own purpose, not a mandatory prefix to every
test run. Reuse compilation performed by a test/build only when it covers the
required scope and options; transpile-only tests do not prove type correctness.
Do not repeatedly collect or rediscover the same tests when the mapping is current.
When tool defaults are broad, supply explicit selectors and inspect the emitted
command/collected scope. An optional `path` or a changed cwd is not proof of narrow
execution. Use the existing repository runner if the standard dispatcher cannot
express the needed scope, preserving the source-edit/tool-availability gates.
No matching tests means a coverage question to resolve, not an automatic full run.

Before a full suite, record its concrete reason: broad/uncertain impact that cannot
be bounded adequately, shared dependency/configuration changes, a required
integration/release gate, or a measured tiny suite whose full run is cheaper and
adequate. "To be safe", a new commit, another edit, another agent or another phase
is not a reason. Never weaken a required check solely to hit a time budget.

## Reuse evidence only while its inputs remain valid

Record command/selectors, observed result and primary output/CI reference, actual
source location/revision, runtime/dependencies/configuration, scope, executor and
observed duration. A confident "tests passed" summary without recoverable execution
evidence is insufficient. Reuse remains a verification decision by the lead or
accepting orchestrator, not blind trust in a delegate's conclusion.

Repeat the affected check when its relevant implementation, tests, fixtures,
configuration, dependencies, environment or externally supplied test state changed;
when the earlier run failed, was incomplete, used the wrong source/runtime, lacks
adequate evidence; or when a concrete diagnosis/acceptance requirement needs it.
The change of an unrelated file or a hash label does not by itself invalidate all
checks. Use the actual diff, dependency knowledge and environment evidence; if
validity cannot be established, run the smallest adequate check and say why.

A failed run is evidence to diagnose, not an instruction to repeat an unchanged
command until green. Fix the cause or name a concrete diagnostic question first.
A bounded retry for suspected flakiness records the original failure and all
outcomes; it does not erase the failure or justify an unbounded retry loop.

A pre-commit run may support the resulting COMMIT when its checked code, tests,
inputs and relevant configuration/environment are demonstrably identical. Record
that attribution after inspecting the commit. Do not rerun solely to obtain an
after-commit timestamp. Conversely, never label an older result as a test of new
behaviour. A module submission states which results were reused and why they
still cover the submitted tree. This does not prove unrelated untested behaviour.

Preserve the original test result as evidence; do not overwrite it with a new
invented execution time or claim that a reused check ran again.

## Avoid duplicate and opaque runs

Choose one executor for each required check on the same candidate. Reuse suitable
local, hook or CI evidence; do not run the same suite in all three places by habit.
A reviewer asks for missing evidence through the orchestrator rather than having
several agents independently launch the same suite. Honor mandatory CI at its
prescribed event and avoid a redundant manual copy when trustworthy CI covers it.

Before relaunching a slow command, inspect its existing process/session and output;
do not create a duplicate while the first may still run. Use the runner's supported
timeout/session controls, and distinguish a missing timeout utility or failed
launcher from a test failure. Preserve the real exit status when piping output.
Keep progress observable; do not hide the whole live run behind a final `tail` and
then assume silence means a hang or success. Unknown duration is a reason to
observe the existing run, not to kill or blindly repeat it.

## Completion remains evidence-based

Every implementation task still has local checks appropriate to its change and
its own commit. If a shared verification batch is still pending, a checkpoint
commit preserves work but the task does not claim verified completion yet.
Every module still needs independent review, contract evidence and required real
integration. Fixes return to its lead and invalidate only relevant checks and
review scope. Broader tests are justified by broader impact, not by nervousness.
