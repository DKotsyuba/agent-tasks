# Changelog

## Unreleased

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
