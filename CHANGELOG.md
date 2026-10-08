# Changelog

## Unreleased

## 0.10.0

### Added

- `get_context` reads the full retained history of a compaction proposal (`view=history`, optional one based `revision`) as lossless paged rows, and one hash verified staged candidate (`view=content` with `revision` and `action`) in fixed 8192 byte `md-text-v1` pages with an offset-free snapshot. Misuse of the selectors is refused by field before any read.

### Changed

- Typed record, checklist and compaction history prose is rendered as quoted, escaped rows that are exact and cannot forge structural lines; long facts continue in `(continued)` rows without trimming or truncation.
- Argument decoding errors name the tool, and a bad nested enumerated value no longer reports an unknown operation. Oversized `record_work result` fields are named.

### Fixed

- `search state=superseded` no longer returns work records; one state predicate covers work, knowledge and documents.
- A compaction `view=history revision=N` header shows the title, hash and author of revision N instead of the current revision.

## 0.9.2

### Fixed

- Keep canonical YAML records containing ordinary punctuation readable, and validate encoded bytes with the same guarded decoder before publication.
- Close preflight bypasses involving plain continuation text, explicit flow keys, alternate raw line breaks and stray byte order marks. Native quoted scalars and flow collections must be single-line; canonical records remain compatible without read-side migration.
- Add direct preflight refusal, canonical round-trip and cold-router restart regressions with meaningful mutation controls.

## 0.9.1

### Fixed

- Give the multi-step real-assembly SDK scenario a bounded host-speed budget suitable for unoptimized CI binaries. Per-call checks, mutation controls, real AB/BC/ABC assembly and all business assertions remain unchanged.
- Retain the immutable 0.9.0 tag after its source-check timeout; this patch publishes the same Epic core behavior with the corrected verification budget.

## 0.9.0

### Added

- Epic business scope, Module membership, embedded Tasks and independently reviewed Atomics, with current integration and business-criterion coverage.
- Persistent observed lead, reviewer and integrator bindings; explicit loss-only replacement preserves history and requires context recovery before continuation.
- Lead-owned discovery and Task planning, reciprocal versioned contracts, agreement before coding, and meaningful correct/mutated/restored boundary-test observations.
- Incremental integration of ready connected Modules without waiting for the whole Epic or requiring delivery first. Final business verification binds the actual affected composition.
- Bounded read-only local Git report import, declared execution/dependency context and canonical orchestrator/Module-lead skills.

### Changed

- New work uses the Epic core workflow. Existing records remain readable without migration; explicit adopt_core activates the new rules and leaves earlier approvals historical.
- New Modules start without preplanned Tasks. Launch the lead, bind its returned runtime ID, record discovery, agree boundaries and freeze the Epic roster before coding.
- Module approval is pinned to the submitted candidate and affecting contract revisions. Delivery remains separate bookkeeping; Task completion belongs to the lead after tests or manual verification.
- Integration records retain pending claims and exact current coverage. Pairwise AB and BC checks do not establish an ABC business outcome.

### Fixed

- Preserve canonical definition/revision history across removal, consumer changes and provider transfers; reject stale approvals and duplicate active integration jobs.
- Keep review findings scoped to their actual target, prevent premature candidate-less review, and require an explicit assembly candidate for integration acceptance.
- Release publishing runs protocol and core business tests against the exact packaged payload before making the release visible.

### Compatibility

- Stored data with expanded Epic/core fields requires this binary; rolling back the executable does not downgrade or erase work records.
- Qualification remains limited to macOS arm64 and the named MCP Inspector client. Runtime launches, code correctness and owner installation are separate from durable reported facts.

## 0.8.0

### Added

- register_project creates portable project documentation, its generated manifest and allocator, a README and ignore rules, and a local Git repository with one initial commit before publishing the alias.
- get_project_list discovers project aliases and manifest-derived names/descriptions without exposing documentation locations; unavailable projects remain visible.
- Source repository metadata and the optional documentation Git origin are separate. Registration never pushes or fetches.

### Breaking changes

- Move the aliases table from config.toml to sibling projects.toml. Both files retain schema_version = 1; existing documentation roots and work records are unchanged. Inline aliases receive an explicit migration instruction rather than automatic conversion during reads.
- The MCP now exposes nine tools. Project registration is explicit and repeatable; ordinary work writes are still not automatically committed.

### Fixed

- Release owned advisory locks explicitly so inherited descriptors in forked Git children cannot retain a completed caller's lock.
- Preserve initial-commit failures and partial effects, with registration-specific recovery guidance. Operator Git signing and hooks remain enabled.
- Correct Project creation guidance to omit ref instead of suggesting the invalid ref=Project.

## 0.7.0

### Breaking changes

- Replace the retired Linear gateway with a portable local file-backed MCP core. The old Linear tools, HTTP gateway, credentials and workflow state are not migrated or used by the new runtime.
- Store Project intent and standalone Modules with embedded Tasks in machine-maintained YAML. Select independent documentation roots using TOML aliases and the common project argument.
- Provide seven tools: get_status, get_context, project_status, search, plan_work, record_work and review_module. Epics, Atomics, Markdown writers, automatic Git and agent orchestration remain future roadmap capabilities.

### Added

- Compact English context, current results/checks/artifacts, blockers, handoff, reasoned cancellation/reopen and retained independent Module review.
- One-call owner status and bounded lexical work search with explicit data/detail coverage and selection-bound pagination.
- Monotonic IDs, generated UTC metadata, advisory root locks, guarded atomic publication, original-byte normalization backups and explicit partial/uncertain outcomes.

### Fixed

- Advertise the required object root on operation-union tool schemas so independent MCP clients discover the write tools.
- Clarify tool mini documentation, correct Project search navigation, label Module phases in Task receipts and make empty-detail messages specific to their view.

### Delivery

- Use the standard Rust template's pinned SDK/toolchain, strict MiniJinja, structural/contract checks, independent CI source/payload gates and immutable single-binary installer.
- Native qualification covers aarch64-apple-darwin and MCP Inspector CLI 2.7.0. No Windows/Linux, Codex/Claude host or power-loss certification is claimed by that declaration.
- Preserve old releases and configuration. New file-core configuration defaults to HOME/.agent-tasks/config.toml; it does not interpret the discontinued Linear configuration.
