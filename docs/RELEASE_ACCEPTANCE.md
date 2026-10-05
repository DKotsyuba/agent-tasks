# Release acceptance for the portable core

## Qualified scope

Native target: aarch64-apple-darwin on macOS arm64. Independent host: MCP Inspector CLI 2.7.0, running with Node 24.4.0. Qualification is limited to that named target/client; it does not certify Linux/Windows, Codex/Claude, arbitrary filesystems or power-loss durability.

## Executed checks on 2026-10-05

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

Version 0.7.0 replaces the discontinued Linear API and shared HTTP gateway with local stdio/file storage. Old configuration, credentials, releases and history are preserved; they are not interpreted or migrated. Configure project aliases in HOME/.agent-tasks/config.toml or select an explicit path. Binary rollback does not restore work data or transparently restart the former gateway.
