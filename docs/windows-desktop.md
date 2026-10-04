# Windows desktop package and daily use

Direct's supported daily-use artifact is an unsigned portable Windows package. It keeps the native desktop, local service/CLI, and browser assets together while leaving the authoritative workspace in `%USERPROFILE%\.direct\data`. A signed MSI or MSIX remains future work.

## Build a package

From a normal Windows PowerShell session with Rust/MSVC, Node.js, and WebView2 available:

```powershell
.\scripts\package-windows.ps1
```

The script runs the Rust, Clippy, formatting, and frontend checks, creates release binaries, assembles `direct.exe`, `direct-desktop.exe`, and `web/`, writes a checksum manifest with the exact Git commit and dirty state, and produces a zip under `target\packages`. Use `-SkipChecks` only when the same source has already passed the checks. A dirty build is clearly marked in its directory name and manifest; do not submit it as exact commit evidence.

## Install, launch, and update

Extract the zip, open a normal Windows PowerShell session in the extracted directory, and run:

```powershell
powershell -ExecutionPolicy Bypass -File .\Install-Direct.ps1
```

The default application path is `%USERPROFILE%\.direct\app`. The installer verifies every manifest entry, stages the replacement beside the final directory, creates a stable per-user Start Menu shortcut named `Direct`, and launches the native desktop. It never bundles, copies, restores, or deletes the workspace.

For an update:

1. Run `scripts\backup.ps1` from the current source checkout and retain its archive/checksum outside Git.
2. Build or obtain the new package, inspect `manifest.json`, then run its `Install-Direct.ps1`.
3. The updater closes the installed desktop window and asks the installed service to shut down gracefully before swapping the application directory. The prior application directory is retained beside the install as a timestamped `.previous-*` rollback copy.
4. Launch `Direct` from the Start Menu and complete the manual checks below. Do not delete the prior application directory or backup until the build is accepted.

If a pre-DIR-4 installed service cannot accept graceful shutdown, the updater stops and preserves both versions. `-ForceStop` is available only for the exact executable in the explicit install directory; use it only after confirming no write is active and a backup exists. A service running from another checkout is not killed or presented as package evidence.

To roll back the application, stop Direct, move the current `%USERPROFILE%\.direct\app` aside, and rename the retained `.previous-*` directory back to `app`. Application rollback does not downgrade a workspace schema; consult the schema/archive compatibility notes and restore into a separate directory when a data rollback is required.

## Automated package smoke

Run the packaged harness from a normal Windows PowerShell session:

```powershell
powershell -ExecutionPolicy Bypass -File .\Test-DirectPackage.ps1 -PackageDir . -ReportPath .\smoke-report.json
```

The harness verifies manifest checksums, installs into a generated temporary directory, opens the real native Windows window, creates one synthetic issue in a new isolated workspace, closes the window and checks agent access remains available, gracefully stops and restarts the service, confirms the issue persisted, and removes the fixture. `-KeepArtifacts` retains the temporary application/data directories for diagnosis. Never point this smoke harness at `%USERPROFILE%\.direct\data`.

The harness proves process, package, persistence, and service-lifecycle behavior. It does not prove visual correctness or owner acceptance.

## Manual owner acceptance

After the exact submitted package passes the automated smoke, the owner should use the normal workspace and record the result in the current Direct verification run:

1. Launch `Direct` from the Start Menu and confirm the expected real workspace and issue list appear in the native window.
2. Edit a disposable real issue field or comment through the native UI, reload/reopen the issue, and confirm the edit persisted.
3. Close the native window, run `scripts\direct.ps1 list` from the source checkout, and confirm agent access still works.
4. Run the installed `%USERPROFILE%\.direct\app\direct.exe stop`, relaunch `Direct`, and confirm the native UI reconnects to the same workspace and the edit remains.
5. Record pass/fail for the exact submission. Automated checks, successful compilation, or a synthetic fixture cannot substitute for this owner decision.
