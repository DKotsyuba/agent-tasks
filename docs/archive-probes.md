# Native archive probes

Ordinary tests do not access credentials. The ignored `archive_native_gates`
test requires `ATL_ARCHIVE_LIVE=disposable-fixture`, `ATL_LIVE_TEAM_ID`, a new
absolute `ATL_LIVE_REPORT`, and the existing protected `LINEAR_API_KEY` or
`LINEAR_API_KEY_FILE` provider. Run only under the operator's explicit fixture
authorization:

```sh
cargo test --test archive_live archive_native_gates -- --ignored --exact --nocapture
```

The test creates a new Project, six owned Issues, and a separate Project with
a sentinel Issue. It writes each native mutation intent before sending it and
stops on an uncertain response. The report retains exact object identities and
observations. It is evidence, never a workflow database. A new run requires a
new report; reconcile an uncertain effect before further writes.

Document-size probes use exactly three fixed sizes (64 KiB, 256 KiB, 1 MiB),
less than 2 MiB of aggregate generated source text. There is no environment
override for their count or size.

Native relations use a two-record page with three additional owned peers to
prove pagination without creating hundreds of Issues. Mock regressions force
multi-page history; native history may group edits, which the report records
honestly. Quota relief requires an operator observation;
connection counts, document limits, retention, and latency are empirical.
Unknown observations are recorded explicitly and do not qualify compaction.
The current probe does not certify a destructive Compact implementation.

After reviewing the inventory, the same environment/report can run
`archive_native_cleanup` with the same Cargo flags. Cleanup validates every
deterministic ID, native team/project and fixture marker, then soft-deletes only
fixture Issues. Missing records are not accepted as trash confirmation. The
sentinel and both Projects remain; byte uploads may remain for retention evidence.
After a failed cleanup attempt, another run first reads the same owned Issue
by exact ID. A new separately journaled attempt is allowed only when that read
confirms it remains untrashed; existing failed/unknown intents are preserved.
The native trash field is nullable. Sentinel and cleanup checks accept present
`false` or `null` as untrashed; missing/invalid fields are incomplete. A deletion
is confirmed only by an exact-ID read with a present `true`, never by absence.

## Targeted followup on an existing report

After reviewing the pending native mutation by exact readback, the operator can
use the SAME report/team with `ATL_ARCHIVE_FOLLOWUP=renderer-and-resources` and
run `archive_native_followup` with `--ignored --exact --nocapture`. A new report
or fixture is not created. The old `g6-1` pending intent remains unchanged; a
separate read-only observation records whether the old accepted metadata is
still present. No retry of that mutation is made.

The followup creates one managed Epic target in the existing fixture Project,
one small source artifact, its copied artifact and one readable renderer proof
Document. It runs the actual production prepare/attach/readback and compact
Document reparent/readback helpers, then renders and validates the full native
canonical archive. Existing comment/history volume is reused. It performs no
Issue deletion; sentinel/ownership checks bracket the work. Unknown effects stop
with their new exact action/intent preserved. These observations do not prove
post-deletion retention or quota relief.
