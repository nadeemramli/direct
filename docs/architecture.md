# Architecture and limits

Direct is a single-owner local workspace for coordinating humans and coding agents. It does not require a subscription, remote account, or Linear connection.

## Components

- `direct-core`: typed commands, workflow validation, SQLite persistence, archive validation.
- `direct`: single-writer local HTTP service and agent CLI.
- `app`: Svelte UI, embedded by the Tauri desktop shell or served locally for browser use.

The Windows service owns SQLite. WSL launches the Windows CLI, which talks to the same service. A process lock prevents two services from sharing the directory. SQLite uses WAL and full synchronous writes. Each mutation commits the data change, audit event, and idempotency result in one transaction.

Every mutable issue has a version. A stale command returns a conflict instead of silently overwriting another client. Request IDs are bound to actor, role, and command content; exact retries return the saved response, while changed payloads cannot reuse the ID. Claims expire and require explicit reacquisition before submission.

The UI polls an event cursor every 750 ms and refreshes affected views. This is automatic local polling, not WebSocket streaming. Editing drafts keep their original version, so concurrent updates are detected at save time.

## Human verification

Only the owner interface can make work Ready, approve a test, or reopen a submitted/completed item. The agent CLI can capture, refine, claim, comment, release, and submit work. Submission records build/delivery references, automated checks, limitations, preconditions, and manual steps. A linked verification issue is reused across retests; each submission creates a distinct immutable-history run.

Reviews must identify the current run as well as the issue version. All steps must pass before Done. A failure needs a failing step and reason. Cancellation needs a reason and never counts as passing. Reopening preserves prior evidence.

## Local access

The service listens only on `127.0.0.1`. It validates Host and Origin, requires bearer capabilities, limits request bodies, and sends a restrictive browser CSP. Credentials rotate on service startup. Windows data-directory inheritance is removed and access is granted to the current user and SYSTEM; Unix directories use mode 0700.

The owner interface uses Tauri IPC, or a single-use browser launch grant that expires after 120 seconds. Browser sessions last eight hours and are invalidated by service restart. Browser launch URLs are capabilities, so do not share them.

This prevents accidental approval through the agent API and unsolicited website requests. It does **not** isolate adversarial processes running as the same OS user: such a process can read the owner capability or database. A stronger agent sandbox is separate future work. Product repository/vault paths are context only; Direct does not execute arbitrary agent commands or read those directories automatically.

## Current limits

No cloud sync, multi-user permissions, agent runner, MCP server, Linear importer, milestones/goals, general issue relationships, attachments, signed installer, automatic startup at login, or automatic scheduled backup yet. Product spaces have no artificial team cap. Full snapshots are not paginated; large workspaces need a later performance pass. Context includes the latest 100 comments and 50 events, with a truncation flag for comments; full archives preserve all records.

Projects belong to a product and have stable IDs, a name/outcome, and a version for concurrent edits. Grouping is planning metadata: it does not change readiness or approval evidence. Verification children inherit their parent's project. Schema 2 introduces projects atomically and causes older binaries to refuse opening the database. Archives are now format 2; the reader upgrades format-1 records with no project assignment and preserves old retry responses unchanged.

The desktop shell starts the service independently, so closing its window does not stop agent access. Source builds expect the CLI beside the desktop executable. A portable distribution must also place browser assets in a sibling `web` directory. Do not distribute just the desktop executable as an installer.
