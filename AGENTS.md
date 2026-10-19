# Agent Tasks observer — implementation instructions

Use the pinned Rust toolchain, locked prepared dependencies and `cargo xtask check`. Source is Rust; compact text uses trusted embedded MiniJinja. stdout of `mcp` is protocol-only. Keep language-native documentation synchronized with changed declarations.

Repository-authored technical text, skills, prompts, comments and agent-facing service messages must be English. Owner-facing report localization is explicitly supported through `[summary]` language/style preferences; user-supplied content and Unicode fixture data remain verbatim.

The owner-approved product is an on-demand progress observer. Runtime tools are `plan`, `status` and identity-only `get_status`. Do not restore project registration, Epic/Module/Atomic records, manual completion, bindings, developer reports or special commit-message sections. Save scope only during owner-authorized planning. Developers do not maintain progress.

Git collection is read-only. Never checkout, fetch, merge, commit, test or push source while gathering status. Preserve omissions and uncertainty. A draft or commit message alone does not prove implementation; current target content supersedes old ancestry after a revert.

Persist the exact analysis start intent before submission. Unknown admission replays its original key and caller namespace. Never resume summary analyzers. Validate input identity, plan revision, execution sequence and answer hash before accepting a result. No completion-polling loops. Reuse semantic reports when content is unchanged.

No daemon, database, background sampler or automatic paid fallback. One explicitly requested analysis may own a one-shot event-wait continuation for automatic Bridge delivery; it does not observe new development. Freeze the original destination and exact body before publication. Uncertain custody permits original-event status reconciliation only. Preserve the original planner contact from per-request host metadata, never model arguments or persistent server environment. All test/scratch paths follow the current task's project-specific temporary-directory policy. Existing installed state and legacy data are outside source work; no cleanup or installation is implicit. New qualification remains `not_verified` until actual checks justify it.
