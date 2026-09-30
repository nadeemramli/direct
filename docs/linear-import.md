# Linear import dry-run

The offline importer converts one project from a verified Linear source package
into a separate Direct workspace. It never calls Linear, the running Direct
service, or the owner's normal `%USERPROFILE%\.direct\data` workspace.

Run it with absolute private paths outside the repository:

```powershell
.\target\debug\direct.exe linear-import-dry-run `
  --source C:\Users\you\.direct\linear-migrations\linear-source-... `
  --project-id 00000000-0000-0000-0000-000000000000 `
  --output C:\Users\you\.direct\linear-migrations\dry-run-project
```

The command verifies `manifest.sha256` and every required data-file checksum,
then writes:

- `direct-import.json` and its SHA-256 checksum;
- `reconciliation.json` with source/import counts, unknown state or relation
  types, source-package limitations, and cross-project links omitted by the
  bounded selection; and
- `workspace/direct.db`, restored from that archive and compared with it.

Run the exact command again against the same output to prove a deterministic
idempotent replay. It succeeds only when the generated archive and existing
workspace match exactly and reports `duplicates_added: 0`. A different source
or incomplete output is refused instead of being overwritten.

Linear completed issues are stored as **Legacy done** with original source ID,
URL, state, timestamps, and captured history. Their comments, milestones,
initiative goal, parent links, blockers, and related links inside the selected
project are retained. No Direct verification run is created, and Legacy done
does not count as owner-verified completion. Cross-project relations are
counted in the reconciliation report but are not followed into the bounded
workspace.

Use `recovery-check` with a new directory to prove the imported archive restores
byte for byte. This dry-run is migration evidence, not permission to cut over a
product or replace the live Direct workspace. Full cutover still needs a final
delta capture, whole-scope reconciliation, updated intake paths, and an explicit
owner decision.
