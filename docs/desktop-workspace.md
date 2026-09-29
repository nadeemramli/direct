# Desktop and agent workspace

On Windows the default workspace is `%USERPROFILE%\.direct\data`. The desktop,
CLI, WSL wrapper and pilot seeder use the same resolver. `DIRECT_DATA_DIR` (or
the CLI's `--data-dir`) still selects an explicit isolated workspace.

Do not use `%LOCALAPPDATA%\Direct` as the default. Processes launched from a
packaged app such as Codex can see Windows MSIX-redirected AppData, while the
same executable launched from Explorer sees another folder. That produced a
populated browser pilot and an empty desktop workspace on 29 September 2026.

The resolver refuses to create a new default when it detects a legacy database
and no canonical database. It never automatically chooses or merges a legacy
workspace. A missing USERPROFILE also produces an error instead of falling back
to the current directory.

## Move an existing workspace

1. Use the CLI with an explicit source path and inspect `list`. Identify the
   populated workspace by its ID and tasks, not by a familiar-looking path.
   For a packaged caller, use its physical `Packages/.../LocalCache/Local/Direct`
   path when referring to that source outside the package.
2. Close Direct windows, export the authoritative workspace, and stop its
   service before allowing further edits. Retain the export and original data.
3. Create `%USERPROFILE%\.direct` if absent. Run `direct --data-dir
   "%USERPROFILE%\.direct\data" restore <archive>` using a NEW target directory.
   Restore refuses to overwrite existing data. Do not copy a live SQLite file.
4. Start the rebuilt desktop, or run the service against the restored directory.
   Export again and compare hashes with the source archive before making edits.
5. Confirm the desktop shortcut, CLI and WSL show the same tasks. Open a fresh
   `direct open` link for a browser tab; old tabs refer to the retired service.

The desktop shortcut points at `target/debug/direct-desktop.exe` and requires
both rebuilt executables next to each other. Build with
`cargo build -p direct -p direct-desktop --features direct-desktop/custom-protocol`.
`scripts/start.ps1 -SkipBuild` launches the same desktop. This remains a developer
build; a signed installer and broader native acceptance are tracked by DIR-4.

## Pilot repair evidence

DIR-12 tracks this repair. The original pilot had ten parent tasks and a pending
verification child; DIR-12 adds an eleventh parent task. Migration preserves the
workspace ID, project, issue IDs, comments, claims, events and pending test run.
The pre-migration and restored exports were byte-identical. Eleven Rust tests
passed, including three new path-resolution regressions; workspace Clippy and
format checks passed. The desktop build started the canonical service, closing
the desktop left CLI access alive, and the existing shortcut reopened the build.
Windows and WSL returned the same workspace ID and event cursor. The browser
rendered all eleven parent tasks from the restored service.
Developer checks do not count as owner acceptance. Native-window automation was
unavailable during diagnosis; the owner must confirm the shortcut displays the
restored pilot before marking DIR-12 passed.
