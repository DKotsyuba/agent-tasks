# Epic core workflow plan

Status: governing product target from the owner's latest instructions, 2026-10-06. This defines the baseline Epic workflow. The implemented architecture remains documented separately in [architecture.md](architecture.md); partial implementation does not imply conformance to every rule below.

An Epic turns high-level business requirements into logical Modules led by persistent agents. Leads discover their own implementation work and agree on boundary contracts before parallel development. Independently accepted Modules are assembled as connected parts become ready; integration never waits for unrelated unfinished Modules.

## Responsibilities

| Role | Responsibility |
|---|---|
| Owner | Business requirements, material scope decisions and unresolved exceptions |
| Orchestrator | Epic, provisional Module boundaries, actual runtime launches and ID bindings, contract coordination, review/integration dispatch and final Epic acceptance |
| Persistent Module lead | Code/context discovery, Module boundaries, Task decomposition, provides/consumes obligations, implementation, local Task completion, boundary tests and corrections |
| Persistent Module reviewer | Initial whole-Module review, findings and later review of the changed scope |
| Integration agent | Actual assembly, cross-Module contract compatibility and joint behavior of ready components |
| MCP | Durable structured context, relationships, progress, reports and conditions; no embedded model execution |

Agent identities are obtained from the runtime, not guessed. Planning and implementation use the same lead and its retained context. Tasks are bounded implementation units inside a Module, not an instruction to create a separately reviewed Atomic for each edit.

## 1 Create the Epic

The orchestrator records the existing business problem, intended outcome, high-level requirements, scope/exclusions and observable business acceptance criteria. Technical interfaces belong to Module coordination and their canonical artifacts, rather than being substituted for business requirements.

The orchestrator divides the Epic into provisional logical Modules with coherent responsibilities. Their roster may change during lead discovery and contract reconciliation; freeze it after agreement, immediately before parallel implementation. It supplies each Module's goal, relevant Epic context and an initial read boundary. This division does not require the orchestrator to invent all implementation Tasks or finalize interfaces without local code discovery.

## 2 Launch a lead before binding its ID

For each Module the orchestrator performs this order:

1. Create the Module and make its goal/context available through MCP.
2. Launch a lead using the selected supported harness, with the Module reference and the assignment to read the context/code, decompose Tasks, establish boundaries and identify contracts.
3. Obtain the real agent/session ID from the launch receipt.
4. Save the harness and actual ID in the Module's lead binding.
5. Continue planning, implementation and corrections through that same binding.

The ID is unknown before launch and remains absent until observed. A label such as "Storage lead" is not a session ID. The binding includes the supported communication/resume address needed for the orchestrator to reach that lead; a fabricated transcript URL or unsupported resume capability is not acceptable.

Replacement is permitted only when the original assigned agent is lost and cannot continue the work, and its session cannot be recovered or resumed. This sole exception applies to leads, reviewers and integration agents. If the original agent can continue, keep that agent; temporary unavailability, slow progress, review findings or a preference for another model do not permit replacement. Record the observed loss and inability to continue, retain the prior binding and work/review history, launch the replacement with a context-recovery assignment, then bind its actual returned ID.

Replacement is a new immersion process, not an ID swap. The replacement reads the relevant Epic/Module goals, code and exact candidates, agreed contracts, Task outcomes, prior decisions, reports, tests, review findings, integration coverage and unfinished work. It reconstructs what its predecessor did, what remains valid and what needs to happen next, and records its recovered understanding before continuing the role. Unresolved context gaps block the affected continuation until reconciled. A replacement reviewer retains the prior findings and changed-scope basis; repeat unchanged whole-Module review only where lost context creates a concrete verification gap.

If launch or binding replies are lost, inspect the existing launch/Module state before another launch. A successful launch whose binding write failed must be reconciled and bound, not duplicated. Runtime access and permitted filesystem/tool scope are configured by the launcher; Module text itself grants no permission.

The lead's first code familiarization is part of its assignment, not a separate replacement agent whose understanding is discarded before coding.

## 3 Leads discover Tasks and boundaries

Each persistent lead loads its Module through MCP and inspects its permitted code area. It records:

- responsibility and inclusion/exclusion boundaries supported by the code;
- atomic implementation Tasks, criteria and relevant local verification;
- capabilities/data/behavior it can provide through public boundaries;
- inputs/capabilities/behavior it needs and their known or missing sources;
- existing interfaces, constraints, uncertainties and actual prerequisites.

The lead creates and refines its Module's Tasks. The orchestrator coordinates cross-Module scope, shared ownership and business coverage; it does not replace local discovery with a fully invented Task list. Material business requirements cannot disappear silently during decomposition.

No Module implementation begins while that Module's necessary boundary obligations remain unresolved.

## 4 Agree on dependent contracts

A contract describes how system parts interact: interface/protocol, input/output meaning, valid/invalid inputs, expected results/errors and relevant effects/guarantees. Each actual boundary has one canonical definition or artifact reference used by the participating Modules.

Module records identify provides and consumes relationships and actual dependencies. Provides/consumes means obligations, not merely request/response direction. A runtime/data-flow link alone does not require sequential development.

The orchestrator matches required inputs to providers. The same leads agree on the relevant interaction: what is available, what is required, how the contract is satisfied and which checks demonstrate compatibility. Contradictions return to those leads for reconciliation; silence or a modified counterproposal is not agreement.

Record the agreed revision and participating Modules' confirmation. Changes require renewed agreement by affected parties, updated relevant Tasks/artifacts and explicit impact handling. Independent work continues. This is practical coordination of trusted agents, not a revival of a universal certificate or cryptographic attestation platform.

Before development, the orchestrator checks actual prerequisites for cycles of mandatory waits. Such a cycle is an architecture problem: explicitly report the cycle and its impact to the owner, and resolve it as a separate architecture issue with the affected leads before starting the affected work. Do not let leads enter mutual completion waits. Contract/data-flow cycles that do not impose mandatory waits are distinct and remain allowed.

## 5 Start parallel implementation

Once the relevant contracts and actual prerequisites are agreed and the permitted environment is prepared, the same leads begin implementation in parallel. A consumer can work against the agreed interface and an appropriate substitute; it need not wait for an entire provider Module unless a real prerequisite requires that wait.

Each Module is developed in its own worktree. Leads do not share a writable checkout, and integration uses a separate checkout. Tests also isolate mutable state such as databases, temporary files and service fixtures so separate worktrees cannot affect each other's checks.

The lead maintains warm Module/code context, implements Tasks and records useful outcomes. Task completion belongs to the lead: it checks by tests or manually and decides when the Task is done. There is no independent Task review and no automatic completion merely because a commit, import or runtime success exists.

For code, retain the accepted single-report direction: write the meaningful result/checks/limits once in the local Git commit message and reference/import it through MCP. Noncode work can use a semantic report. Source reporting and completion authority are separate facts.

## 6 Prove the boundary tests work

The central Module test exercises its public input/output contract. For a contract mapping A to B, identical relevant conditions must produce B from A. Control initial state, time, randomness and external responses when those conditions influence behavior. Determinism does not automatically imply statelessness or idempotency.

The lead must prove that the test detects a real violation, not only show a green test:

1. Run the agreed boundary cases on the correct implementation and observe the expected results.
2. Introduce a controlled meaningful mutation or fault that violates an input/output obligation.
3. Run the same contract test and observe its intended failure for that violation.
4. Restore the implementation and rerun the control case successfully.

Do not mutate the test assertion itself or unrelated setup merely to manufacture a failure. Record the tested contract/candidate, conditions, deliberate violation and actual pass/fail observations. Applicable examples include wrong output mapping, accepted invalid input, wrong error behavior or a missing promised effect.

Run mutations only within the Module's isolated worktree/test state, without concurrent writes to the mutated files or state. Restore only the deliberate changes, preserve unrelated edits, and submit only the restored candidate after the successful control rerun. A mutation must never enter another Module's candidate or shared integration state.

Relevant internal tests remain useful. A substitute must reflect the agreed contract and must not reimplement its neighbor's business algorithm or complex persistence/state machine. A passing substitute is not evidence that actual Modules compose correctly.

## 7 Submit and review the Module

When implementation is complete, the lead submits a definite candidate with Task outcomes, contract definitions/revisions, local checks, boundary-test and mutation-control observations, known gaps and artifact references.

The orchestrator launches the Module reviewer, obtains its actual ID and retains the reviewer binding. Initial review covers the whole submitted Module, including implementation, public boundary behavior and meaningful verification. If the reviewer is irrecoverably lost, apply the participant replacement and context-recovery process from section 2 before review continues.

If findings require correction:

1. Return the findings to the same Module lead through its recorded runtime ID.
2. The lead fixes the Module and submits an updated candidate and changed-scope information.
3. Resume the same reviewer through its recorded ID.
4. Review the changes and named findings; do not repeat an unchanged whole-Module review without a concrete reason.

A current positive review makes the Module ready for integration. Delivery/merge into a final target is a separate fact; it is not a prerequisite for recognizing a reviewed Module as ready to be assembled.

A positive review applies to the exact submitted candidate and the affecting contract revisions it reviewed. An affecting contract change invalidates Module readiness even when the code candidate is unchanged. The same lead assesses and updates the affected work, or explains why no code change is needed; the same reviewer checks the changed obligations before restoring readiness, subject to the explicit replacement process. Refresh boundary/mutation evidence for the changed tested scope. Internal Module corrections found during integration use this same correction/re-review loop. Prior approvals remain history and cannot authorize the new obligations; affected integration coverage is invalidated as well.

## 8 Integrate ready connected components asynchronously

The orchestrator reacts to actual completion/review/change reports and current MCP facts. It derives ready connected sets from Module contract relationships and current accepted candidates.

When at least two dependent Modules are ready, dispatch an integration agent for their connected ready component. Do not wait for every Module in the Epic and do not combine unrelated Modules solely because they are ready.

The integration assignment identifies exact Module candidates, agreed contracts, assembly scope, environment and joint scenarios. The agent must:

- assemble the real candidates in an authorized integration checkout;
- verify that providers' output and consumers' input obligations actually match;
- check that valid/invalid cases, errors and relevant effects compose correctly;
- run the actual combined behavior and report limitations/findings honestly.

A previous integration result can be reused only while its relevant candidates/contracts/scenarios/environment remain applicable. New ready Modules trigger the next relevant assembly. Changed or reopened participants invalidate affected coverage; unrelated development continues.

The integration agent owns wiring, assembly and integration checks. Internal Module defects return to that same Module lead, or its explicitly recorded replacement after irrecoverable session loss. The integrator does not silently take ownership of Module internals.

Store candidate-set and contract-revision coverage so an integration job is not duplicated for unchanged inputs. Unknown launch/job outcomes are inspected before retrying. No-code verification needs no fabricated commit. Model execution remains the orchestrator's responsibility through supported runtime tools, not an embedded MCP daemon or scheduler.

## 9 Accept the Epic

Epic acceptance requires every required business criterion to be covered by current accepted Module work and actual applicable integration/business verification. Individually green Modules or a complete Task progress bar do not establish the Epic's business outcome.

Integration coverage is incremental over ready connected parts. Final acceptance checks the relevant currently accepted composition and end-to-end criteria; it does not retroactively require that all earlier integration jobs waited for a single complete roster or contained every unrelated Module.

Preserve report/review/integration history, explicit cancellation/reopen reasons and unknown/partial coverage. Cancellation never silently cascades or deletes results. Microfixes may still require no tracked hierarchy.

## Implementation plan

| Slice | Required implementation and verification |
|---|---|
| Persistent agent bindings | Runtime/harness plus observed ID and communication/resume reference, launch-then-bind/recovery, explicit replacement of any irrecoverably lost participant with retained history and a new context-immersion process; scenarios for lost launch/binding, same-ID continuation and recovered replacement context |
| Lead planning | Module context before lead launch, lead-owned Task/boundary decomposition, provides/consumes/actual dependency records and sufficient code/assignment context |
| Contract coordination | Canonical boundary references, agreement/revision/affected-party changes, separate descriptive links from real waits; mandatory-wait cycles reported and resolved as architecture issues before affected development |
| Boundary verification | Separate Module worktrees and isolated mutable test state, meaningful mutant/fault failure and restored control without foreign edits; manual Task completion remains distinct from Module-quality evidence |
| Review loop | Definite candidate and affecting contract revisions, retained reviewer ID/findings, same lead corrections, same reviewer changed-scope follow-up; affecting contract changes invalidate readiness and refresh relevant evidence |
| Incremental integration | Ready connected components of at least two Modules, exact candidate coverage, actual assembly checks, applicability/reopen handling, dedup and partial/unknown recovery |
| Context and skills | One assignment through Module ref, one-call overview with planning/review/integration attention, accurate orchestrator/lead guidance and lifecycle examples |
| Compatibility | Portable Rust/file stack, one YAML per Module, stable existing references/history, no read migration, preserved bounded publication/version/recovery contracts |

## Settled exception policies

The owner resolved these policies on 2026-10-06:

- Module roster is provisional during discovery/negotiation; freeze after contract agreement and before parallel coding.
- Replacement is permitted only when the original assigned agent is lost and cannot continue, with no recoverable/resumable session. Otherwise retain the same agent. The replacement must undergo a new immersion process, reconstruct the predecessor's work/context and record its understanding before continuation; retain identity/work/review history.
- Each Module develops in a separate worktree; mutation checks and mutable test state must not interfere with another Module.
- Cycles of mandatory Module waits are architecture problems. The orchestrator explicitly reports and resolves them separately before affected development starts.
- An affecting contract revision invalidates the prior Module review/readiness, even with unchanged code; renewed changed-scope review is required.
- Integration agents change only wiring/assembly/checks; internal Module repairs belong to the Module lead.

## Implementation conformance

This plan supersedes earlier target assumptions that the orchestrator finalizes Tasks before assigning leads, IDs exist before launch, Module readiness requires target-branch merge, integration waits for the whole Epic, or every integration job must match the complete frozen roster.

Core-active portable records and MCP operations implement this coordination baseline. Core-absent records retain their prior rules until explicit adoption. Runtime launches/resumes, context immersion, code verification and real assembly remain the orchestrator and agents' responsibilities; [architecture.md](architecture.md) records the implemented boundary and local validation.
