# Architecture and limits

Direct is a single-owner local workspace for coordinating humans and coding agents. It does not require a subscription, remote account, or Linear connection.

## Components

- `direct-core`: typed commands, workflow validation, SQLite persistence, archive validation.
- `direct`: single-writer local HTTP service, agent CLI, explicit Theoria catalog importer, and offline Linear dry-run importer.
- `direct-mcp`: optional local stdio MCP server that forwards an allowlist of typed agent commands (`snapshot`, `context`, `claim`, `renew`, `submit`) through the `direct` client to the running service. It is agent-role only, opens no listener or database, and cannot perform owner operations. See [local MCP access](mcp.md).
- `app`: Svelte UI, embedded by the Tauri desktop shell or served locally for browser use.

The Windows service owns SQLite. WSL launches the Windows CLI, which talks to the same service. A process lock prevents two services from sharing the directory. SQLite uses WAL and full synchronous writes. Each mutation commits the data change, audit event, and idempotency result in one transaction.

Every mutable issue has a version. A stale command returns a conflict instead of silently overwriting another client. Request IDs are bound to actor, role, and command content; exact retries return the saved response, while changed payloads cannot reuse the ID. Claims expire and require explicit reacquisition before submission.

The UI polls an event cursor every 750 ms and refreshes affected views. This is automatic local polling, not WebSocket streaming. Editing drafts keep their original version, so concurrent updates are detected at save time.

## Git evidence

Direct stores commit and successful-push evidence as structured records linked to an issue. The agent must hold the active claim and supply the current issue version, repository identity, full commit object ID, and branch. Push evidence additionally requires the remote and remote ref. Logical duplicates are rejected independently of request replay, and records survive archive/restore.

The CLI does not execute Git. `record-commit` follows a successful commit; `record-push` follows a successful push. This boundary prevents a pre-push attempt from being presented as delivery. Pull requests, merges, deployments, and owner acceptance remain separate evidence and are not inferred from a commit or push.

## Releases

A release is a product-scoped delivery boundary, separate from project and milestone planning. It names a version and target ref, links same-product projects and real parent issues, and derives progress from the union of that work. Pending verification, failed/rework verification, owner-verified Done, Legacy done, and Canceled remain distinct; only Done contributes to verified completion.

Release evidence is append-only and typed: commit, successful push, passed owner verification, preview deployment, or production deployment. Git evidence references an existing structured issue trace, checks reference a passed verification run for a linked Done issue, and deployments record an exact deployment reference, full commit, target ref, URL, actor, and timestamp. A preview can only cite a commit already attached to the release. Production additionally requires a matching successful push and preview, at least one linked issue, every linked issue owner-verified Done, and the owner actor recorded as approver. Branch names, pushes, merges, previews, Legacy done items, and canceled tests cannot independently claim production.

Each product may configure its production ref, branch strategy, release-branch pattern, preview environment and URL template, and promotion policy. One-branch-per-release conventions use a single `{version}` placeholder and prevent two active releases from owning the same branch or commit. Repositories with another topology select the external strategy and keep their repository-specific orchestration outside Direct. Direct records successful external operations only after completion; failed/canceled deployment attempts and rollback/ref transitions are retained as append-only evidence without changing release status.

## Theoria boundary

Theoria is a navigation and traceability layer over an explicitly catalogued Development Operating System source root. The maintained Markdown stays external. Direct stores a bounded read-only cache, source metadata and SHA-256 fingerprint, issue-specific fingerprint/version references, and proposal-only method findings with evidence pointers. Sync never crawls a vault, executes document content, or edits source files.

Unavailable sources keep any prior cache but are visibly marked unavailable. A changed cache does not rewrite an issue's recorded fingerprint, which makes stale use auditable. Unknown playbook versions remain unknown. No Direct command accepts a method proposal, enrolls or promotes an experiment, or changes Jev routing. See [Theoria](theoria.md) for the full contract.

## Human verification

Only the owner interface can make work Ready, approve a test, or reopen a submitted/completed item. The agent CLI can capture, refine, claim, comment, release, and submit work. Submission records build/delivery references, automated checks, limitations, preconditions, and manual steps. A linked verification issue is reused across retests; each submission creates a distinct immutable-history run.

Issue deletion is an owner-only cleanup for unstarted Backlog or Ready records. Direct rejects deletion when an active claim, relationship, explicit release reference, release evidence, comment, submission, Git trace, or method finding exists, so deletion cannot erase delivery evidence or orphan another record. One eligibility calculation feeds both issue context, where the owner interface explains each blocker before offering deletion, and the delete command, which recomputes it in its own transaction. The row is removed, an `issue_deleted` activity event remains, and future issue allocation consults activity history so the key is never reused.

Reviews must identify the current run as well as the issue version. All steps must pass before Done. A failure needs a failing step and reason. Cancellation needs a reason and never counts as passing. Reopening preserves prior evidence.

## Local access

The service listens only on `127.0.0.1`. It validates Host and Origin, requires bearer capabilities, limits request bodies, and sends a restrictive browser CSP. Credentials rotate on service startup. Windows data-directory inheritance is removed and access is granted to the current user and SYSTEM; Unix directories use mode 0700.

The owner interface uses Tauri IPC, or a single-use browser launch grant that expires after 120 seconds. Browser sessions last eight hours and are invalidated by service restart. Browser launch URLs are capabilities, so do not share them.

This prevents accidental approval through the agent API and unsolicited website requests. It does **not** isolate adversarial processes running as the same OS user: such a process can read the owner capability or database. A stronger agent sandbox is separate future work. Product repository/vault paths are context only; Direct does not execute arbitrary agent commands. Explicit CLI-side imports read only the selected Theoria catalog or captured Linear package. The Linear dry-run writes a separate offline workspace and never targets the live service.

## Current limits

No cloud sync, multi-user permissions, agent runner, remote or HTTP MCP transport, full-workspace Linear cutover, attachments, signed installer, automatic startup at login, or automatic scheduled backup yet. The Linear importer is intentionally project-bounded and offline; cross-project links are reported but not followed. Product spaces have no artificial team cap. Full snapshots are not paginated; large workspaces need a later performance pass. Context includes the latest 100 comments and 50 events, with a truncation flag for comments; full archives preserve all records.

Projects belong to a product and have stable IDs, a name/outcome, fixed status, shared priority, owner-controlled order, and a version for concurrent edits. Project progress is derived from real parent issues: only owner-verified Done counts as completion; Verify and Canceled are reported separately. Issues explicitly choose project work or the Inbox/maintenance route. Project work cannot become Ready without a same-product project. Grouping remains planning metadata and does not change approval evidence; verification children inherit their parent's project and route.

Goals form one typed layer above projects; ordered milestones sit inside projects; issues may select a milestone only from their current project. The hierarchy deliberately has no reverse or nested-goal operation, so cycles are structurally rejected. Goal and milestone progress derives from real parent issues: Canceled and Legacy done work are excluded from the denominator, Verify remains pending, and only owner-verified Done counts as complete. Imported planning and issue records retain external provenance.

Schema 10 adds per-product release workflow configuration, release-branch ownership, deployment outcomes/environments, and rollback history. Archives are format 10. Readers accept formats 1–9 with compatibility defaults while preserving old retry responses unchanged. Schema 9 added product-scoped releases and typed release evidence atomically. Schema 8 added external project/issue/comment provenance, captured Linear issue history, and the distinct Legacy done state. Legacy done is terminal historical provenance, never a Direct verification result and never counted as owner-verified completion. General parent and blocker links are directed and cycle-checked; related links are symmetric; imported legacy verification links remain separate from Direct's generated verification runs and require external provenance.

Labels are a single workspace-level taxonomy. Each label has a stable ID, one canonical name, explicit aliases, optional per-product applicability with an optional default for new issues, and preserved Linear label IDs/names for a later import. Issues and projects reference label IDs; a label with no product rules applies to every product, and a rule set restricts it without creating per-product copies. Canonical names, aliases, and Linear IDs are unique across the workspace. Narrowing a label's products is refused while work outside those products still carries it. Only the owner defines labels or labels projects; agents attach and detach issue labels under the same claim rules as project grouping. Labels never change readiness, priority, ownership, or verification evidence, and verification children carry no labels.

The desktop shell starts the service independently, so closing its window does not stop agent access. Source builds expect the CLI beside the desktop executable. A portable distribution must also place browser assets in a sibling `web` directory. Do not distribute just the desktop executable as an installer.

Schema/archive 11 integrates the shared label taxonomy with planning/releases and adds optional historical E2E evidence to verification records. New submissions require passing evidence for identical tested, delivered, and submitted builds. Formats 1–10 remain readable; old evidence is never invented. Label-bearing cloud format 6 was never deployed to the pilot; label data must use the integrated format 11.
