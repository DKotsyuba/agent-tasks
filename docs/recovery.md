# Recovering an interrupted request

## Archive preservation actions

Archive preservation exposes a deterministic plan and one-action execution,
reconciliation and readback helpers. The caller records each in-flight action
before executing it and records confirmation before advancing. No helper loops
through an entire plan or deletes any Issue.

`PrepareCopy` verifies source bytes, reserves and PUTs a temporary upload, then
returns an `AttachCopy` action containing only the canonical asset URL. Signed
PUT URLs and headers remain in memory and must never enter workflow receipts,
reports or logs. A lost temporary reservation/PUT response may leave an orphan
temporary upload; an explicitly recorded fresh prepare attempt is permissible
after reconciling the deterministic published attachment by exact ID. There is
no invented native reservation lookup. `AttachCopy` publishes one artifact on
the permanent Epic and preserves the existing `metadata.artifact` intent plus
self-contained `compacted_from` provenance. Confirmation re-downloads the bytes
and checks exact size/digest, ownership and metadata. A lost publication reply
reconciles that same ID before another mutation.

`ReparentDocument` checks current ownership, timestamp, title, visibility and
complete content, then changes only ownership to the Epic and clears Project
ownership. A lost reply reads that Document by exact ID. Concurrent edits refuse;
unchanged confirmed reparents replay without a second write. Original Document
content and comments also remain in the complete archive.

Keep the `request_id` and the complete original arguments until the outcome is known. `outcome_unknown` is never success.

1. Keep one gateway writer. Restart it on the same fixed loopback port if it stopped.
2. For an existing issue, use `get_context` with `type: issue`. A prepared update includes its original tool request and intended native update.
3. Retry that same tool with the same `request_id` and arguments. A different request cannot replace a pending operation.
   An already applied target is finalized without reapplying the mutation. If native fields differ from both the saved source and intended target, MCP reports a conflict and preserves the manual content. Resolve that conflict explicitly in Linear before retrying; reads never restore old content.
   Linear's `-` to `*` list serialization is presentation-equivalent, while changed text or link destinations still conflict. An older pending Duplicate transition containing only `stateId` is replayed through the native duplicate relation operation using its saved `duplicate_of` field. Preserve all original arguments and metadata; recovery does not need a replacement request or a direct metadata edit.
4. For interrupted creates, retry the original create call. The same UUID addresses the same Project/Issue/Document. Default project documents have repeatable subordinate IDs. Existing objects are checked before reuse.
5. Review creation uses its request UUID as the native comment UUID. A retry reuses the matching report instead of posting another comment.

Edits to Projects/Documents are native partial updates. Repeat the same field assignment after checking context if their result was uncertain. Do not replay an old edit after newer intentional edits; use a fresh request for a new intention.

Manual state violations are not automatically rolled back. Restore changed parent/project/type label in Linear. An explicit return to In Progress can recover an externally changed workflow status and starts a fresh result/review round. A description-only change is adopted through an explicit edit, preserving unrelated sections.

If the object or its state attachment was deleted, do not silently recreate a different work item. Inspect the preserved native history and the original request. API authentication, rate limits, malformed/partial responses and missing objects remain distinct failures.

Only the transport configuration contains a secret bearer credential. Workflow state is in Linear; no signing key or local workflow database needs recovery.
