# Direct

Internal work tracking for human–agent development.

The first working foundation is implemented: a local SQLite service, Windows/WSL agent CLI, Svelte interface, and Tauri desktop shell. Work moves through **Backlog → Ready → Doing → Verify → Done**. Agents submit evidence; the owner records verification outcomes. Failed or canceled tests return work for fixes without erasing previous runs.

## Run on Windows

Prerequisites: Rust with the MSVC build tools, Node.js 22.12+ (24 recommended), and WebView2 for the desktop shell.

```powershell
.\scripts\start.ps1
```

This builds and opens the desktop app. Later, use `-SkipBuild` to launch the existing build. The service starts automatically and continues running after the window closes. Data lives in `%USERPROFILE%\.direct\data`, outside this repository. Use `-DataDir C:\path\to\dedicated-directory` for an isolated workspace.

Older LocalAppData workspaces need an explicit export/restore before first launch with this build. See [desktop workspace and migration](docs/desktop-workspace.md). This prevents Windows packaged-app redirection from splitting the browser and desktop into different workspaces.

For the browser interface, run `scripts\start.ps1 -Browser`. It prints a short-lived, single-use launch link. Open it promptly and do not share it. The browser preview needs a fresh `direct open` link after the service restarts; the desktop shell reconnects automatically.

```powershell
.\target\debug\direct.exe list
.\target\debug\direct.exe context DIR-1
.\target\debug\direct.exe --actor coding-agent create "Investigate a bug" --body "Observed behavior and reproduction"
```

From WSL, use `bash scripts/direct-wsl.sh list`. Both environments call the Windows service; WSL does not open the database. See [the agent contract](docs/agent-contract.md) for claims and submissions.

## Scope

Included: product spaces with repository/vault paths, projects and issue grouping, issue capture and filtering, acceptance criteria, ownership and priority, expiring claims, comments, change history, build-specific verification, retesting, reopening, automatic updates, and JSON export/restore.

Still to build: milestones/goals, general issue relationships, MCP, Linear migration, a signed installer, and multi-device collaboration. The desktop executable compiles; the browser interface has completed the full Windows/WSL smoke test. Native desktop rendering still needs an interactive acceptance check. This is a single-owner foundation, not yet the complete Linear replacement.

Direct now tracks its own development in the normal local workspace. See [the real pilot](docs/pilot.md); ten scoped tasks were captured and project grouping is the first delivery. Final acceptance is recorded by the owner in Direct.

Project grouping upgrades the database to schema 2 and writes archive format 2. Format-1 backups remain readable; older Direct binaries cannot open the upgraded database. Export before upgrading and use a new directory when restoring an older backup.

## Backup and restore

```powershell
.\target\debug\direct.exe export C:\backups\direct-2026-09-28.json
.\target\debug\direct.exe --data-dir C:\backups\direct-restored restore C:\backups\direct-2026-09-28.json
```

The export file must not exist, and restore requires a new directory beneath an existing parent. Export preserves workspace identity, issues, test evidence, comments, event cursors, and retry records. Backups contain your work content; store them privately. Never copy an active SQLite database by itself.

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

Read [architecture and limits](docs/architecture.md) and [verification evidence](docs/verification.md).

## Product research

The canonical audit and product notes remain in the Direct Obsidian folder:

- [Audit and product direction](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/Direct - Audit and Product Direction.md>)
- [Workflow audit](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/01 - Workflow Audit.md>)
- [Linear feature comparison](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/02 - Linear Feature Comparison.md>)
- [Scope and build plan](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/03 - Direct Scope and Build Plan.md>)
- [Live Linear audit — 28 September](<C:/Users/Nadeem/Desktop/Obsidian/build-blog/build-vault/5. Idea Vault/1. Internal Application/Direct - Linear-free alternative/04 - Live Linear Audit.md>)

These are local links for this workspace.

The signed-in browser audit verified five active teams with 468 issues shown, current workflow settings, and representative handoff/verification records. A complete migration export remains separate from this product audit.
