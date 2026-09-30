# Direct

Internal work tracking for human–agent development.

The first working foundation is implemented: a local SQLite service, Windows/WSL agent CLI, Svelte interface, and Tauri desktop shell. Work moves through **Backlog → Ready → Doing → Verify → Done**. Agents submit evidence; the owner records verification outcomes. Failed or canceled tests return work for fixes without erasing previous runs. Theoria adds read-only Development Operating System guidance and proposal-only method learning beside that operational Praxis.

## Run on Windows

Prerequisites: Rust with the MSVC build tools, Node.js 22.12+ (24 recommended), and WebView2 for the desktop shell.

```powershell
.\scripts\start.ps1
```

This builds and opens the desktop app. Later, use `-SkipBuild` to launch the existing build. The service starts automatically and continues running after the window closes. Data lives in `%USERPROFILE%\.direct\data`, outside this repository. Use `-DataDir C:\path\to\dedicated-directory` for an isolated workspace.

Older LocalAppData workspaces need an explicit export/restore before first launch with this build. See [desktop workspace and migration](docs/desktop-workspace.md). This prevents Windows packaged-app redirection from splitting the browser and desktop into different workspaces.

For the browser interface, run `scripts\start.ps1 -Browser`. It prints a short-lived, single-use launch link. Open it promptly and do not share it. The browser preview needs a fresh `direct open` link after the service restarts; the desktop shell reconnects automatically.

```powershell
.\scripts\direct.ps1 list
.\scripts\direct.ps1 context DIR-1
.\scripts\direct.ps1 --actor coding-agent create "Investigate a bug" --body "Observed behavior and reproduction"
```

From WSL, use `bash scripts/direct-wsl.sh list`. Both wrappers call the Windows service and fall back to the primary Git worktree's built CLI when a fresh managed worktree has no local build; WSL does not open the database. See [the agent contract](docs/agent-contract.md) for claims and submissions.

The routine agent handoff does not require editing JSON: inspect the assigned issue, then use the native `claim`, `renew`, and `submit` commands with the current issue version, one explicit actor, and caller-chosen stable request IDs. `submit --step` accepts an instruction followed by its expected result and may be repeated. Exact retry commands keep the same request ID; changed commands need a new one.

## Scope

Included: product spaces with repository/vault paths, projects and issue grouping, issue capture and filtering, acceptance criteria, ownership and priority, expiring claims, comments, change history, build-specific verification, retesting, reopening, automatic updates, JSON export/restore, and Theoria guidance references with source fingerprints and structured method findings.

Still to build: milestones/goals, general issue relationships, MCP, Linear migration, a signed installer, and multi-device collaboration. The desktop executable compiles; the browser interface has completed the full Windows/WSL smoke test. Native desktop rendering still needs an interactive acceptance check. This is a single-owner foundation, not yet the complete Linear replacement.

Direct now tracks its own development in the normal local workspace. See [the real pilot](docs/pilot.md); ten scoped tasks were captured and project grouping is the first delivery. Final acceptance is recorded by the owner in Direct.

Theoria upgrades the database to schema 3 and writes archive format 3. Format-1 and format-2 backups remain readable; older Direct binaries cannot open the upgraded database. Export before upgrading and use a new directory when restoring an older backup. See [Theoria's authority and import contract](docs/theoria.md).

## Backup and restore

```powershell
.\scripts\backup.ps1
.\scripts\direct.ps1 recovery-check C:\backups\direct-backup-....json C:\backups\direct-recovery-drill
.\target\debug\direct.exe export C:\backups\direct-2026-09-28.json
.\target\debug\direct.exe --data-dir C:\backups\direct-restored restore C:\backups\direct-2026-09-28.json
```

The routine script writes a validated archive and SHA-256 checksum, then retains the newest 14 managed snapshots by default. The recovery check restores into a new isolated directory and verifies the complete archive round trip with record counts. The export file must not exist, and restore requires a new directory beneath an existing parent. Export preserves workspace identity, projects, issues, issue links, comments, test evidence, Git traces, Theoria records, event cursors, and retry records. Archive format 6 does not support attachments. Backups contain your work content; store them privately. Never copy an active SQLite database by itself. See [the backup and recovery runbook](docs/backup-recovery.md).

## Development checks

```powershell
cargo test -p direct-core -p direct
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cd app
npm ci
npm run check
npm run build
cd ..
cargo build -p direct -p direct-desktop --features direct-desktop/custom-protocol
```

Read [architecture and limits](docs/architecture.md), [Theoria's authority and import contract](docs/theoria.md), and [verification evidence](docs/verification.md).

## Product research

The canonical audit and product notes remain in the Direct Obsidian folder:

- [Audit and product direction](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/Direct - Audit and Product Direction.md>)
- [Workflow audit](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/01 - Workflow Audit.md>)
- [Linear feature comparison](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/02 - Linear Feature Comparison.md>)
- [Scope and build plan](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/03 - Direct Scope and Build Plan.md>)
- [Live Linear audit — 28 September](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/04 - Live Linear Audit.md>)
- [Linear migration source capture](docs/linear-migration-source.md)

These are local links for this workspace.

The signed-in browser audit verified five active teams with 468 issues shown, current workflow settings, and representative handoff/verification records. A complete migration export remains separate from this product audit.
