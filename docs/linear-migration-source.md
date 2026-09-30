# Linear migration source capture

Direct's migration source package is a private, read-only snapshot of the
Canvasm Linear workspace. It is deliberately separate from Direct's database
backup and from Git.

## Prerequisites

Create a temporary Linear personal API key with only **Read** permission and
access to the five product teams. Keep the key in the process environment; do
not write it to a repository file, shell history, issue comment, or archive.

Linear's workspace CSV export is useful as an independent issue-count check,
but it omits comments and attachment files. Request it from **Settings →
Administration → Import & export**, then retain the emailed CSV beside the
source package after verifying its checksum.

## Capture

From the Direct checkout, run:

```powershell
node .\scripts\capture-linear-handoff.mjs
```

Open the one-time `http://127.0.0.1:...` URL printed by the command, paste the
temporary key into the password field, and choose **Run private capture**. The
handoff binds only to the loopback interface, uses an unguessable one-use path,
keeps the key in memory, and launches `capture-linear.mjs` without placing the
credential in command text, browser history, or a file. Stop an abandoned
handoff process before starting another.

For an already secured automation environment, `capture-linear.mjs` can also
read the key from the process-only `LINEAR_API_KEY` environment variable.

The default output is a timestamped directory beneath
`%USERPROFILE%\.direct\linear-migrations`. The script refuses an output path
inside the current repository. It uses GraphQL queries only, includes archived
records wherever Linear exposes `includeArchived`, downloads referenced
`uploads.linear.app` files, and computes SHA-256 checksums.

The package contains:

- `manifest.json` and `manifest.sha256` with per-team issue counts, query
  coverage, errors, explicit missing coverage, and file integrity data;
- `data/*.json` with original Linear UUIDs, identifiers, URLs, comments,
  documents, planning records, and relationships exposed by the current API;
- `attachment-manifest.json` with source references, download results, sizes,
  content types, and checksums; and
- `attachments/` with successfully retrieved uploaded files.

The utility records but does not follow external link attachments. It also
cannot recover records already deleted from Linear or nested connections over
Linear's 250-record page ceiling; those limitations appear in
`missing_coverage` and must be reconciled before cutover.

## Verification and retention

Treat a nonzero exit as incomplete. Review `errors` and `missing_coverage`,
compare the API totals with the workspace CSV, and verify the manifest hash:

```powershell
Get-FileHash "$env:USERPROFILE\.direct\linear-migrations\<capture>\manifest.json" -Algorithm SHA256
Get-Content "$env:USERPROFILE\.direct\linear-migrations\<capture>\manifest.sha256"
```

After the package and CSV reconcile, revoke the temporary API key. Keep the
package private. Do not commit it or attach it to a public issue or pull
request. A final cutover still requires a later delta capture after the dry-run
import has passed.
