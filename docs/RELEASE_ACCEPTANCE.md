# Release acceptance for the portable core

## Qualified scope

Native target: aarch64-apple-darwin on macOS arm64. Independent host: MCP Inspector CLI 2.7.0, running with Node 24.4.0. Qualification is limited to that named target/client; it does not certify Linux/Windows, Codex/Claude, arbitrary filesystems or power-loss durability.

## Epic E-001 qualification suite: baseline observations, 2026-10-07

Source-only observations of the qualification suite against the 0.9.2 baseline binary (native debug build, macOS arm64, Rust SDK stdio client with a scrubbed environment and disposable HOME, configuration, registry and documentation repository). No packaged payload, installed product or Inspector run is involved, so no host or payload claim follows. The pinned provider artifacts and the full matrix are in [contracts/knowledge-qualification.md](contracts/knowledge-qualification.md).

Observed on the baseline:

- The synthetic work lifecycle (create, bind, plan, add Task, begin, import, complete, boundary evidence, bind reviewer, review) ran through the real tools as eleven saved successes. Its binding receipts and boundary observations are synthetic fixture data used only to measure commit behavior.
- Reads of work records (`get_context`, `project_status`, `search`, `get_project_list`) changed no byte, lock, directory, modification time or index entry, created no knowledge home, and a cold restart read the same project.
- Lexical search over three work Modules: six declared queries (exact title, body field, spread over fields, case) hit at rank 1 with replies of 619 to 661 bytes; a paraphrase, a misspelling and a nonsense query were detected as misses. No semantic service exists or is proposed. Semantic retrieval is considered only after a demonstrated lexical need: a vocabulary-gap miss on a source that is present. The typed and document sources are absent on this baseline, so their unrun queries are capability gaps, not evidence that the lexical algorithm needs embeddings; the full corpus has not run.
- AT-003 reproduced on the baseline: an over-long `criteria` list returns `invalid_data: "At most eight values are allowed."` with no field named, and a wrong field name returns the generic shape message. The two regression tests are ignored until the field-named errors land.
- AT-004 reproduced on the baseline: `project_status module=E-001` prints `Modules: 0 current-reviewed / 0 readable` beside an Epic row declaring two Modules. The regression test is ignored until the Epic roll-up lands.
- The shared helpers (length-delimited frame parser with look-alike payloads and every wrong-frame case, the `md-text-v1` decoder, deterministic vectors, the tree snapshot) are checked by five self-tests that need no product tool.

Gate: `cargo xtask check` (fmt, all-features clippy, all tests, rustdoc, contract drift) passed with exit 0 on the source and tests of this checkpoint: 63 unit tests, 11 core scenarios plus 1 ignored receipt test, 5 helper self-tests and the qualification crates (their baseline-runnable tests passed; every producer-dependent test is ignored with its reason). This is a source-only baseline result.

Not run: every scenario that needs `knowledge_work`, `document_work`, `compaction_work`, `git_recovery` or production Git settlement is `#[ignore]`d with its exact reason (typed knowledge 10, documents 13, Git 13, compaction 6, catalog and field errors 3, full search corpus 1). They are written against the pinned artifacts and are not evidence until the combined candidate runs them with `cargo test -- --include-ignored`, together with the implementation-mutation controls of each consumed boundary.

## Epic E-001 release qualification plan (planned, none of it run)

The baseline observations above are source-only. A release with the full E-001 functionality needs every item below on one exact candidate; a missing item is reported as missing, never replaced by a substitute or inferred from an earlier source check.

1. Combined candidate: one signed commit carrying all provider source and the four registered tools (`knowledge_work`, `document_work`, `compaction_work`, `git_recovery`) with the exported schema snapshot, so discovery lists exactly fourteen tools. `cargo xtask check` passes on it (fmt, all-features Clippy, all tests, rustdoc, contract drift).
2. Real SDK families on that candidate with the ignored markers removed: catalog and field-named errors, typed knowledge, managed Markdown (exact bytes, framing, the 524288-byte body over stdio rather than an Inspector argument list, stale continuation, native drift, legacy files, move and clone), production Git (a commit per successful change, no commit for reads, no-ops, refused or partial calls, hook, signing, index and foreign-staging failures keeping saved bytes, hook-kill crash during settlement, explicit recovery) and reviewed compaction (action kinds, resume, held replacement and Move source gate, plain-clone completeness).
3. Implementation-mutation controls: for `kr-operations` and `registered-tool-surface`, and for every provider boundary a family exercises, a correct pass, a meaningful implementation mutant that fails the same cases, and a restored pass, each bound to the exact candidate commit and recorded with `boundary_evidence`. Assertion or setup mutation is not accepted.
4. Measured lexical quality: the declared query corpus over work, typed records and Markdown is run through the real `search`; the table of hits, ranks, misses, coverage lines and reply bytes is retained. Semantic retrieval is considered only if a vocabulary-gap miss on a present source is demonstrated.
5. Packaged artifact: the exact payload bytes and manifest are verified, then the protocol, core and E-001 families run against the payload through `MCP_TEST_BINARY`; the standard installer runs in a disposable home and bin and its identity is checked; MCP Inspector CLI (modern and legacy discovery, strict portability) lists all fourteen tools and performs fresh-process calls. This remains limited to the named native target and client.
6. Supply chain and publication: `cargo deny` after an explicit fetch, no stub tool, CI source and payload jobs, annotated tag, release approval and immutable asset verification as the existing artifact gates require. This worker neither pushes nor publishes.
7. Skills and clients: the three canonical skills pass the structural validator, and client copy bytes are compared with the component source after installation; links are never assumed to prove equality.
8. Observations: AT-002 (punctuation round trip), AT-003 (field-named errors) and AT-004 (Epic child counts) are retested on the candidate; Agent Run dependency observations stay attributed to Agent Run.

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
