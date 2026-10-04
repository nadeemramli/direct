Direct portable Windows package
================================

This is an unsigned, local portable build. It contains the native Direct desktop,
the local service/CLI, and the browser assets needed by that service. It is not a
signed MSI or MSIX installer.

Install or update from a normal Windows PowerShell session:

  powershell -ExecutionPolicy Bypass -File .\Install-Direct.ps1

The default install is %USERPROFILE%\.direct\app and the stable Start Menu
shortcut is "Direct". The authoritative workspace remains
%USERPROFILE%\.direct\data and is never copied into the package.

Before updating, create and retain a validated workspace backup as documented in
docs/backup-recovery.md in the source repository. The updater closes an installed
desktop window, gracefully stops a service running from the installed directory,
verifies every package checksum, stages the replacement beside the install, and
keeps the prior application directory for rollback. It does not rewrite the data
directory.

For an isolated package smoke test:

  powershell -ExecutionPolicy Bypass -File .\Test-DirectPackage.ps1 -PackageDir .

The smoke test uses a new temporary data directory, opens the real native window,
proves agent access survives window close, restarts the service, checks persistence,
and removes the fixture. It does not access the owner's normal Direct workspace.

See docs/windows-desktop.md in the source repository for the build, update,
rollback, automated smoke, and manual owner-acceptance procedures.
