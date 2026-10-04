# Native archive probes

Ordinary tests do not access credentials. The ignored `archive_native_gates`
test requires `ATL_ARCHIVE_LIVE=disposable-fixture`, `ATL_LIVE_TEAM_ID`, a new
absolute `ATL_LIVE_REPORT`, and the existing protected `LINEAR_API_KEY` or
`LINEAR_API_KEY_FILE` provider. Run only under the operator's explicit fixture
authorization:

```sh
cargo test --test archive_live archive_native_gates -- --ignored --exact --nocapture
```

The test creates a new Project, four owned Issues, and a separate Project with
a sentinel Issue. It writes each native mutation intent before sending it and
stops on an uncertain response. The report retains exact object identities and
observations. It is evidence, never a workflow database. A new run requires a
new report; reconcile an uncertain effect before further writes.

Native history may group edits. Quota relief requires an operator observation;
connection counts, document limits, retention, and latency are empirical.
Unknown observations are recorded explicitly and do not qualify compaction.
The current probe does not certify a destructive Compact implementation.

After reviewing the inventory, the same environment/report can run
`archive_native_cleanup` with the same Cargo flags. Cleanup validates every
deterministic ID, native team/project and fixture marker, then soft-deletes only
fixture Issues. Missing records are not accepted as trash confirmation. The
sentinel and both Projects remain; byte uploads may remain for retention evidence.
