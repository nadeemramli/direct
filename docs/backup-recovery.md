# Backup and recovery

Direct backups are versioned JSON archives obtained from the running single-writer service. Do not copy `direct.db`, `direct.db-wal`, or `direct.db-shm` while the service is active.

## Routine backup

Run this from the Direct checkout:

```powershell
.\scripts\backup.ps1
```

The default private location is `%USERPROFILE%\.direct\backups`. Each successful run:

1. asks the running service for one consistent archive;
2. validates the archive before publishing it;
3. writes through a unique `.partial` file and atomically renames it;
4. writes a sibling SHA-256 file; and
5. retains the newest 14 `direct-backup-*.json` snapshots and their checksums.

Only files with Direct's managed backup name are eligible for retention cleanup. Change the explicit destination or retention count when required:

```powershell
.\scripts\backup.ps1 -BackupDirectory E:\Private\Direct -Retain 30
```

Backups contain issue text, comments, evidence, source paths, and cached Theoria content. Keep the directory private, exclude it from Git and cloud sharing unless that destination has been deliberately approved, and monitor the command's exit status. A nonzero exit means the run did not complete cleanly and must be investigated; do not count it as a retained recovery point.

For a daily routine, configure Windows Task Scheduler to run this exact command under the owner's normal account while the Direct service is running:

```text
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "C:\path\to\Direct\scripts\backup.ps1"
```

Task Scheduler should retain task history and treat a nonzero exit as failure. Scheduling is an owner/host operation; the repository does not silently install or modify scheduled tasks.

## Recovery drill

Choose a retained archive and a new directory that does not exist:

```powershell
.\scripts\direct.ps1 recovery-check `
  C:\Users\you\.direct\backups\direct-backup-....json `
  C:\Users\you\.direct\recovery-drills\2026-09-30
```

The command verifies the checksum when its sibling `.sha256` file is present, validates the archive, restores it into the new isolated workspace, re-exports the restored store, and requires the complete semantic archive to match. Older supported formats are compared after Direct's documented compatibility normalization, and the report identifies any format upgrade. Its JSON report includes the workspace identity and counts for products, goals, projects, milestones, releases, release evidence, release workflows, issues, issue links, comments, verification runs, Git evidence, Theoria documents/findings, events, and replay records. A current-format archive should also reproduce byte for byte; an older archive may differ because its restored export uses the current format.

Archive format 12 carries retained external sources: bundle metadata, the record index, and every retained file as base64 bytes. Validation re-checks each file's size and SHA-256 and that every indexed record span is well-formed JSON from its file, so a corrupted retained file fails the drill. The report's `retained_sources` section counts bundles, files and bytes. A retained Linear capture is large (the real package is roughly 110 MB of JSON before uploads), so backups that include it are correspondingly large; export allows up to 15 minutes. Direct still has no native attachment record for new work.

The recovery directory is intentionally left in place for inspection. It is an isolated Direct workspace; do not point the live service at it during a drill. Remove it only after the evidence has been reviewed and any needed report has been retained.

## Recovery response

If the live workspace is lost or corrupt, stop the service before changing any data. Preserve the damaged directory and latest backup, run `recovery-check` into a new directory, inspect the report, and then make an explicit owner decision about which workspace becomes authoritative. Never restore over an existing directory.

## Recovery after a migration

Applying a migration writes `migration-backups/pre-migration-<time>-<id>.json` (with `.sha256`) inside the data directory before anything changes, and the bundle records that file name. While nothing has changed since the import, the owner can undo it exactly with **Roll back this import** in Imported sources (`rollback_migration`). After later changes, recover the pre-import state by stopping the service, running `recovery-check` on that backup into a new directory, and making the recovered directory authoritative only by an explicit owner decision; work done after the import is not in that backup.
