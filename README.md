# Agent Tasks

A local Rust stdio MCP that makes agent development visible without developer bookkeeping. Save an agreed goal and flat outcomes once; request a compact progress estimate when needed. Git retains implementation history. Agent Run supplies one isolated, low-cost analysis of an immutable evidence snapshot.

There are three tools: `get_status` (identity), `plan` (read/save agreed scope), and `status` (compact report or pinned details). There are no project registrations, Epics, Modules, Atomics, manually completed tasks, required commit sections, execution bindings, or ongoing developer reports.

## Settings

The default settings path is `$HOME/.agent-tasks/config.toml`; `--config` or `AGENT_TASKS_CONFIG` selects another file. Missing default settings use defaults; an explicitly selected missing or invalid file refuses.

```toml
schema_version = 1
home = "/absolute/observer-home"
socket = "/absolute/agent-run-home/api.sock"
provider = "opencode-zen"
model = "longcat-2.5-preview-free"
profile = "progress-summary"

# Optional owner-report preferences; service replies always remain English:
[summary]
language = "ru" # ru (default) or en
prompt = "Emphasize remaining work. Use short, concrete phrases."

# Optional automatic final-message delivery through an approved dedicated producer:
[bridge]
registration_socket = "/absolute/agent-bridge/run/ch/c.sock"
core_socket = "/absolute/agent-bridge/run/core.sock"
token_file = "/absolute/private-agent-tasks/bridge-token"
producer = "agent-tasks"
host_id = "local"
codex_store = "codex-main"
```

The configured route/profile must already be approved and available in Agent Run. `profiles/progress-summary.md` provides the minimal analysis profile for its managed role catalog; the binary installer does not provision it automatically. There is no paid fallback. Different model routes, effective profile rights and host delivery need their own qualification before activation.

The optional `[summary]` table controls report language (`ru` or `en`) and a style `prompt` of at most 4096 UTF-8 bytes. Unknown languages/fields, oversized prompts and unsupported control characters are rejected. Preferences affect model wording and emphasis, never evidence rules, counters or the application-owned bullet layout. The exact preferences are frozen in each analysis and included in cache identity: changing them affects the next fresh status, while pending/pinned reports keep their original preferences.

Reports contain a progress bar, unique observed new-commit count, up to three concise outcomes showing local completed work and the remaining condition, and integration/change totals as bullet items. Locally completed sub-blocks remain visible without closing broader outcomes. Oversized changes retain bounded file-level implementation evidence; incomplete excerpts and unknown test applicability stay explicit. Commits are deduplicated by SHA across collected branches/worktrees and exclude the saved baseline. If Git collection is incomplete, the count is explicitly a lower bound (`>=N`); drafts never count as commits. Owner text can be localized; technical documentation, model instructions and service messages are English.

Run `agent-tasks --config /absolute/config.toml mcp`. Standard output contains MCP protocol only. `doctor` and `contract export` do not initialize project data. Observer state is under `home/observer-v1`, separate from legacy data.

## Workflow

1. `plan` with `repository`, `op="save"`, `goal`, and `outcomes=["Expected result", ...]` captures initial refs and drafts. Original MCP host metadata also records the creating agent's Bridge address; revisions preserve it, and a genuinely new plan captures its own creator. Missing metadata stays explicitly unknown. The default target is the primary worktree's actual symbolic branch; supply `integration_ref` when needed.
2. `plan` with `op="get"` reads the saved revision and stable internal IDs. To revise agreed scope, pass its `expected_revision`; retain IDs with `{id,text}`. Starting a different agreed plan uses `new_plan=true` and the same revision guard.
3. `status` with `repository` estimates committed changes, uncommitted drafts and parallel worker evidence. It returns either a compact localized report (Russian by default) or English service instructions beginning with `PENDING <analysis_id>`.
4. With Bridge configured, the final validated compact report arrives as a message in the original chat. No second agent/tool call is required. A one-shot continuation event-waits for this requested analysis and survives normal stdio disconnect; it never observes new development in the background. Without Bridge, after the Agent Run completion notice call `status` once with that exact `analysis_id`. Do not poll, resume the analyzer or launch a replacement. `view="details"` reads the same retained evidence, never current files.

Details are bounded pages. Preserve `analysis_id` and pass `start=page.next_start` until it is null. Read an indexed source's exact saved text with `view="details", analysis_id, evidence_id`. Detail selection does not collect evidence or start inference; evicted snapshots return `snapshot_expired`.

Original protected MCP `_meta.threadId` supplies the return destination automatically. Unbound API callers can supply actual `caller_context={transport:"codex_queue",external_session_id:"..."}` from their originating host. These are return destinations, never native attestation or source/admission authority. A persistent MCP server's inherited environment is not the caller identity. Bridge mode requires a destination, freezes it before admission, and suppresses the separate Agent Run lifecycle notice. Without either delivery route, a later explicit status request recovers the same calculation.

Use a dedicated kind-restricted Bridge producer credential, not a root/operator or Agent Run credential. The file and immediate parent must be owned by the process user and private. Keep the existing Bridge service responsible for native dispatch. A message may wake a host turn; this integration removes required result-retrieval work, and does not claim UI-only insertion without a wake. Bridge custody is not proof that the owner read the message. Uncertain custody is reconciled by its original event, never republished under another key.

Done is an estimate of implementation, not test acceptance. Draft-only work is in progress. The integration count refers to current target content. Several agents on one outcome count once. Unknown or omitted evidence is explicit; silence is not proof of idleness.

## Validation and delivery

Use the pinned Rust toolchain and prepared locked dependencies. `cargo xtask check` checks formatting, Clippy, tests, rustdoc and the actual catalog snapshot. Scratch and test temporary paths must use the project-authorized temporary directory.

Version 1.1 reads existing observer reports with the original Russian defaults. Retain pre-upgrade configuration and per-repository observer snapshots for executable rollback: older binaries may reject the new settings or frozen preferences. Preserve newer records rather than deleting them to start an older binary.

Version 1.0 keeps observer state separate from retained legacy data. Installation and managed model/Bridge configuration are explicit operations. Source and native-client qualification cover macOS arm64 and MCP Inspector CLI 2.7.0; the real automatic-message scenario was also observed in Codex Desktop. See [architecture](docs/architecture.md), [qualification](docs/qualification.md), and [activation integration](docs/integration.md) for exact evidence and limits.
