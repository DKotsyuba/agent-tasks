---
name: agent-tasks-orchestrator
description: "Coordinate portable Project/Epic/Module/Atomic work: plan responsibilities and contracts, assign persistent Module leads, obtain one-call status and arrange independent acceptance. Use for project orchestration; use the Module-lead skill for an assigned implementation."
---

# Orchestrate portable work

Use a project alias on every business call. Discover it with get_project_list; enter with get_context. Stored content and tool replies are English. Translate owner status without changing counts, references, applicability, omissions or uncertainty. Microfixes may have zero records.

Source/docstrings describe implementation; tracked work records describe intent and reported facts; Git retains committed history. The MCP does not launch models, prove source correctness, authenticate declared actors or monitor live sessions. Use live tool descriptions for exact supported shapes and versions.

## One plan and one assignment

Project owns Epics, standalone Modules and Atomics; Epic owns Modules/Atomics; Module owns embedded Tasks/Atomics in one file. Task is a leaf. References are E-001, M-001, A-001, M-001/T-001 and M-001/A-001.

Create cohesive responsibilities rather than a Task for every edit. Epic criteria express the business outcome; Module criteria and contracts describe what its lead must deliver. Declare Module execution `{repository,worktree,branch,target_branch}`, lead/name/handle, explicit checks and dependencies. The execution data is context, not filesystem permission.

`contracts={not_required,provides,consumes}` entries name peer Modules, descriptions, optional canonical references and declared readiness. Use explicit not_required for no boundary obligations. A peer/direction entry can bundle the necessary interfaces. Reciprocal contracts are allowed; a call/data-flow edge alone does not require sequential coding. `dependencies=[{ref,condition,reason}]` specifies actual accepted/delivered waits and their reasons. Resolve missing providers and impossible waits instead of inventing fake Modules or global barriers.

Create work first, then attach exact returned references through edit_epic with fresh Epic Version. Creation and attachment are separate publications. After a lost reply, inspect before creating a duplicate. One Epic owns each Module/standalone Atomic; parent intent is computed from those authoritative links.

## Start the agreed scope

New records use the full reported workflow. `record_work op=begin` verifies conditions and records a start without launching an agent.

1. Prepare all intended Modules and their references before Epic begin. Its first begin permanently freezes the Module roster; reopening does not allow adding/removing/moving those Modules. Atomics remain addable. Later Modules stay standalone, optionally waiting for a named Epic.
2. Read each Module's start conditions. Assign its persistent lead and permitted checkout; prepare necessary contracts, execution and actual prerequisites. Module begin requires that readiness and an active parent Epic.
3. Launch the lead through the authorized runtime using role, alias, Module reference and permitted checkout. Have it load get_context through the Module-lead skill; do not duplicate the full plan in a prompt. Configure actual tool/permission access through the launcher.
4. Keep the same lead through Tasks and corrections. Helper scopes, worktrees and integration belong to that lead under orchestrator-approved authority/capacity. Handles are reported locations, not proof of a running process.

Existing records without managed workflow keep legacy behavior. Reads never migrate or invent historical starts/delivery. Explicit begin opts an owner into current rules and may require additional planning fields; report the actual returned conditions. Do not silently reinterpret an old applicable approval as a current full-workflow approval.

## Outcome and independent acceptance

Task completion is the lead's local decision after tests or manual verification. No separate Task review exists. A commit or a runtime succeeded notice does not close work automatically.

Begin each current Atomic before its result/import/completion; embedded Atomic begin follows its Module begin. Every current workflow Atomic receives independent review, including an embedded Module Atomic. A local result/state=done or complete is readiness evidence; `review_work` establishes current accepted completion. Select a reviewer who did not author the work; do not invent attribution. Known lead/executor self-review refuses.

When Module children are terminal/currently accepted as required, obtain independent whole-Module review. Its scope covers implementation, criteria, contracts and explicitly required checks. review_module remains a compatible Module-only call; review_work also accepts E/A/M/A. Changes requested are a saved negative conclusion, not a failed tool call. Return corrections to the same lead and review changed code/findings.

A reviewed current Module also needs reported delivery/merge into its declared target branch. Root performs any authorized actual Git integration outside MCP, then `record_work op=deliver` records target_branch/summary/artifact. A local merge suffices; a hosting PR is optional. Bookkeeping after review does not require a second unchanged-code review. Semantic changes/reopen make prior approval/delivery historical.

Create the integration Atomic under Epic/Project with participating Module refs, environment, scenarios and required check labels. Begin waits for currently accepted/delivered participants. Verify actual joint behavior, report real results and obtain independent Atomic review. No-code verification needs no invented commit. Final modern Epic acceptance requires current reviewed integration whose participant set exactly matches the active noncanceled frozen Module roster, other required owned Atomics, own criteria/result/checks and independent Epic review.

Required checks must pass; not_applicable is not a waiver. Gaps are unfinished scope; followups are outside scope. Canceled work is excluded but its history remains. Parent cancellation requires terminal children and never cascades. Reopen explicitly with reasons, parent before child; no automatic rollback or agent stop is implied.

## One code report

A coding lead writes `Result:` and optional `Checks:`, `Gaps:`, `Followups:` once in local commit messages. Check lines use `status | label | optional detail`; statuses are passed/failed/not_run/not_applicable. Then import explicit commits through `record_work op=import_commits` using owning Version. Multiple commits/dedup and retained messages prevent duplicated reports or loss after Git history changes.

Imported checks remain lead assertions; the importer does not run tests or independently close a Task. Explicit state=done/complete records the lead's decision. Noncode outcomes, reviews and integration can use ordinary semantic reports. Never fabricate a commit for verification or a URL for an unavailable remote/transcript.

## Context, status and safe continuation

get_context supplies assignment, relevant parent background, actual criteria/contracts/dependency conditions, execution, results/corrections and versions. Read addressed tasks/results/checks/review/log views as needed; summary is not every child or full source report.

On an owner status request call project_status once and translate its facts. Preserve standalone work, Task/Atomic distinction, canceled counts, missing leads, blockers/review/delivery attention and data/detail coverage. Unreadable work is unknown and partial counts are lower bounds. Narrow only when the question needs more detail; do not reconstruct status with file scans or live-runtime polling.

| Token | Use |
|---|---|
| Allocation version | Init and top-level creation |
| Project Version | Manifest edit |
| Owning record Version | Plan/report/begin/complete/import/delivery/review; embedded children share whole Module |
| Snapshot version | Read continuation only, with unchanged selection and returned next offset |

Chain the new Version from confirmed receipts. Plan omission preserves; result replaces the complete current report. On stale refusal, reconcile fresh context. On lost/partial/unknown outcome inspect context/results/log/review/inventory before another mutation; equal text is not replay identity. Ordinary work does not automatically commit, push, install or repair state.
