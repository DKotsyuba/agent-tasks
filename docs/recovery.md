# Recovering an interrupted request

Keep the `request_id` and the complete original arguments until the outcome is known. `outcome_unknown` is never success.

1. Keep one gateway writer. Restart it on the same fixed loopback port if it stopped.
2. For an existing issue, use `get_context` with `type: issue`. A prepared update includes its original tool request and intended native update.
3. Retry that same tool with the same `request_id` and arguments. A different request cannot replace a pending operation.

   An already applied target is finalized without reapplying the mutation. If native fields differ from both the saved source and intended target, MCP reports a conflict and preserves the manual content. Resolve that conflict explicitly in Linear before retrying; reads never restore old content.
   Linear's `-` to `*` list serialization is presentation-equivalent, while changed text or link destinations still conflict. An older pending Duplicate transition containing only `stateId` is replayed through the native duplicate relation operation using its saved `duplicate_of` field. Preserve all original arguments and metadata; recovery does not need a replacement request or a direct metadata edit.
4. For interrupted creates, retry the original create call. The same UUID addresses the same Project/Issue/Document. Default project documents have repeatable subordinate IDs. Existing objects are checked before reuse.
5. Review creation uses its request UUID as the native comment UUID. A retry reuses the matching report instead of posting another comment.

Review publication also persists its request and predecessor before comment creation. If its response is lost, retry the displayed `record_review` arguments: a cold restart resumes that intent, confirms the comment and finalizes only the captured predecessor. An earlier report retry is historical and never replaces a newer decision. Legacy comment-only recovery requires a matching current stamp and, when a predecessor exists, strictly newer native creation time; ambiguous order is a conflict.

Edits to Projects/Documents are native partial updates. Repeat the same field assignment after checking context if their result was uncertain. Do not replay an old edit after newer intentional edits; use a fresh request for a new intention.

Manual state violations are not automatically rolled back. Restore changed parent/project/type label in Linear. Returning In Review/Done work to In Progress validates and adopts a description-only change, preserving unrelated prose and code while clearing new-round outputs and invalidating the round/revision once. Invalid fields and structural drift block reopening. An active edit with explicit fields (including `{}`) adopts native description drift as a content edit; title/priority-only calls preserve it without adoption. Retiring a child under a reviewed/Done parent requires reopening that parent first. A rejected review directs the orchestrator to `move_status(In Progress)` before implementation continues.

If the object or its state attachment was deleted, do not silently recreate a different work item. Inspect the preserved native history and the original request. API authentication, rate limits, malformed/partial responses and missing objects remain distinct failures.

Only the transport configuration contains a secret bearer credential. Workflow state is in Linear; no signing key or local workflow database needs recovery.
