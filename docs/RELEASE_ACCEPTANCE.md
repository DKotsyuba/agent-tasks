# Release acceptance for the portable core

## Qualified scope

Native target: aarch64-apple-darwin on macOS arm64. Independent host: MCP Inspector CLI 2.7.0, running with Node 24.4.0. Qualification is limited to that named target/client; it does not certify Linux/Windows, Codex/Claude, arbitrary filesystems or power-loss durability.

## Patch 0.9.1 — source checks on 2026-10-06

Version 0.9.1 changes only the connected multi-step SDK scenario's whole-test budget from 180 to 600 seconds. All real candidate assemblies, meaningful mutation controls, negative business-coverage checks and stale assertions remain; application behavior and request/read limits are unchanged. The release check had exhausted the previous cumulative debug/host budget rather than failed a business assertion. The targeted scenario passed against the native debug binary in 110.48 seconds and the exact optimized 0.9.0 CI payload in 18.80 seconds.

Fresh MCP Inspector 2.7.0/Node 24.4.0 calls identified the native source debug binary as 0.9.1, retained ten modern tools and read the existing isolated core Module's current readiness. Strict portability remained zero errors and 66 warnings across nine schema-bearing tools. This retains the named macOS arm64 client scope; it is source-level evidence, not new payload-byte or owner-installation qualification.

## Epic core 0.9.0 — source debug checks on 2026-10-06

The independently exercised native source debug binary identified itself as agent-tasks 0.9.0. These checks qualify the named native source/client interaction; they do not establish packaged-byte verification or owner installation.

- MCP Inspector CLI 2.7.0 with Node 24.4.0 retained all ten tools through legacy and modern discovery. Strict catalog portability diagnostics reported zero errors and 66 warnings across nine schema-bearing tools, concerning legal nullable JSON Schema type arrays.
- Fresh Inspector/server processes registered an isolated documentation alias and exercised a task-free core Module, explicitly synthetic fixture bindings, lead planning and Task creation, reported begin, manual Task completion, a definite fixture artifact, bound independent reviewer and positive Module readiness before delivery.
- An isolated public mapping fixture passed its valid/invalid cases, failed the same check after a meaningful wrong-output implementation mutation, and passed after restoring the implementation. Current local/revision1 evidence was recorded through the independent client. These are controlled fixture observations and reported bindings, not proof of actual model-session execution or actor authentication.
- A stale write version refused without replacing current work. Fresh-process context, search and one-call status retained the reviewed candidate and current results.
- Documentation bootstrap retained the operator's SSH signing policy. When the host sandbox refused access to the existing signing agent, the operation disclosed partial file publication and retained staging; its inspected, unpublished alias state was recovered using normal authorized agent access. Signing was not disabled.

The independent host scope remains macOS arm64/aarch64-apple-darwin and this named client. Packaged payload identity/discovery/persistence, payload SDK checks and disposable self-install verification are separate delivery gates.

## Registration release 0.8.0 — checks on 2026-10-05

- Native 0.8.0 source passed cargo xtask check: 75 tests, formatting, Clippy, rustdoc, structural rules and exported/live discovery contract checks. cargo deny --locked check reported advisories, bans, licenses and sources OK.
- Independent Inspector modern discovery retained all nine tools. Legacy identity returned agent-tasks 0.8.0, registration created the portable records and one initial Git commit, and an identical fresh-process registration returned UNCHANGED without a second commit.
- Independent modern project discovery returned the registered alias, manifest name and description without the documentation directory. Empty source/documentation remote strings were accepted as absent.
- Bootstrap preserved the operator's SSH signing policy. A minimal client environment needs SSH_AUTH_SOCK when that policy uses an SSH agent; missing signing capability refuses with retained local effects rather than silently disabling signing.
- Registry/alias conflicts, foreign content, incomplete bootstrap recovery, snapshot continuation, read-only missing-registry behavior and explicit lock release with a duplicated descriptor have native regression coverage.

Inspector reported zero schema portability errors and 32 warnings for the nine-tool catalog. Qualification remains limited to the named native target and independent client.

## Portable core 0.7.0 — checks on 2026-10-05

- The 0.7.0 source passed cargo xtask check: 71 tests, formatting, Clippy, rustdoc, structural rules and exported/live discovery contract checks.
- cargo deny --locked check reported advisories, bans, licenses and sources OK.
- Independent Inspector legacy connections performed explicit init, Module/Task creation, Task result/check completion, independent Module acceptance, lexical search and project status. Every call used a fresh client/server process; persisted closure remained visible.
- Stale result submission refused with isError=true and left the accepted result intact.
- Modern Inspector discovery retained all seven tools; modern status calls read persisted closure.
- The 0.7.0 binary returned the expected identity, declared qualification and seven-tool catalog; guarded handoff writes and fresh-process status succeeded through the independent host.

The host check found and corrected an interoperability defect before publication: operation unions lacked root type=object, causing the independent client to hide both write tools. A regression now requires that root type on every registered tool. SDK-only acceptance had not detected this client incompatibility.

Inspector schema portability checks reported no errors and 29 warnings for legal nullable type arrays. Those warnings remain documented compatibility limits, not certification of every other client.

## Artifact and installation gates

Packaging requires clean committed source. Verify the exact binary/manifest bytes, then run protocol and core lifecycle tests against that payload with MCP_TEST_BINARY. Exercise the standard self-installer in a disposable home/bin and verify installed identity plus an independent-host call before activating an owner installation.

Publication preserves separate CI source and payload jobs, the release environment approval, annotated tag/source/run binding, supply-chain checks and immutable asset verification. A local source check or declaration in family.toml does not itself prove publication or owner installation.

## Breaking transition

Version 0.7.0 replaced the discontinued Linear API and shared HTTP gateway with local stdio/file storage. Old configuration, credentials, releases and history are preserved; they are not interpreted or migrated.

Version 0.8.0 separates settings from project bindings: HOME/.agent-tasks/config.toml contains schema_version = 1; sibling projects.toml contains schema_version = 1 and the aliases table. Move that table explicitly when upgrading from 0.7.0. Existing YAML/Markdown roots are unchanged. Binary rollback does not restore configuration or work data; restore the retained 0.7.0 settings as a separate operation if rolling back.
