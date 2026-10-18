---
name: agent-tasks
description: Save an owner-agreed flat plan or request a compact Git-derived progress estimate; developers never maintain progress records.
---

# Lightweight Agent Tasks

This entry is versioned with the observer runtime. Import it through the canonical managed skill/policy source only when that runtime is activated. Do not overwrite the active legacy entry while the old server remains installed.

Use the current tool catalog. The runtime exposes identity-only get_status, plan and status. It has no project aliases, registration, Epics, Modules, Atomics, manual completion, agent bindings or ongoing reports. Never emulate the removed workflow with documents or another store.

## Planning

After owner discussion, save the agreed goal and flat high-level outcome texts once with plan(repository, op=save). Original host metadata automatically captures the creating agent's Bridge contact address. Ordinary revisions preserve that creator; a genuinely new agreed plan uses new_plan=true and captures a new creator. Missing metadata stays unknown; never fill it from server environment or model-authored plan arguments. The repository is the actual source checkout. On continuation, plan(op=get) supplies intent, creator and revision; this is optional when already supplied. Revisions require expected_revision; retain internal IDs when editing agreed outcomes. Scope changes require owner discussion.

Implementing agents code, test and commit as authorized by their normal task. They do not call tracking/completion tools, add required commit sections, or maintain task records.

## Owner status

On an explicit owner status request, call status for the actual repository. Pass caller_context using the actual originating Codex session and codex_queue transport; never derive a chat ID from a persistent server's inherited environment or source material.

Return the compact estimate without adding a long narrative. It is implementation estimated from supplied Git/runtime evidence, not test acceptance. Drafts and integration remain distinct. Unknown data is not zero work or proof of idleness.

With Bridge configured, call status once and leave its requested continuation running. The message already contains the final validated compact report: no answer/status retrieval, analyzer resume, acknowledgement or developer reporting is required. Delivery may create a native host turn; that is not new work. Without Bridge, preserve PENDING analysis_id and read that exact handle once after the actual completion notice. Never poll or launch a replacement merely because it is slow. When delivery is unavailable, the next explicit owner request recovers the same pending calculation. admission_unknown requires ordinary status to reconcile the original saved intent; never invent a new request key.

Only an explicit request for detail uses view=details. Follow page.next_start with the same analysis_id and start; evidence_id selects an indexed source's exact saved text. Read retained evidence; do not substitute current files for an old report. An expired snapshot is reported honestly.
