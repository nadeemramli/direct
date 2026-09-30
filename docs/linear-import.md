# Linear import and cutover

The Linear importer works from a verified capture package and never calls
Linear. It has three offline entry points and one owner action in Direct:

- `linear-import-dry-run --project-id <uuid>` imports one project into a new
  isolated workspace (the bounded dry-run).
- `linear-import-dry-run --whole-workspace` rehearses the whole migration into
  a new isolated workspace.
- `linear-migration-prepare` builds a migration artifact for an **existing**
  workspace from a fresh export of it and rehearses applying it offline.
- The owner applies that artifact in Direct (**Imported sources → Owner
  migration**); see [Cutover into an existing workspace](#cutover-into-an-existing-workspace).

The offline commands never open the owner's database. Run them with absolute
private paths outside the repository and outside the Direct data directory;
both are refused. For the dry-run exactly one mode is required.

## Package verification (both modes)

Before anything is written, the importer checks `manifest.sha256` and then
every file the manifest lists, not only the files it reads:

- each data file and each uploaded file matches its recorded size and SHA-256;
- paths are flat `data/<file>` or `attachments/<file>` names (no traversal,
  absolute paths, drive letters or hidden names), and no package entry is a
  symlink or resolves outside the package;
- `data/` and `attachments/` contain no unlisted files;
- `attachment-manifest.json` agrees exactly with the manifest's upload
  integrity list and summary counts;
- declared record counts match each data file;
- the capture recorded no errors other than upload download failures that
  `attachment-manifest.json` lists as failed, no nested connection failed or
  was truncated (`pageInfo.hasNextPage` anywhere in a record), and every
  required record type is present.

A package that fails any check is rejected with a specific message and no
output directory is created.

## Whole-workspace rehearsal

```powershell
.\target\debug\direct.exe linear-import-dry-run --whole-workspace `
  --source C:\Users\you\.direct\linear-migrations\linear-source-... `
  --output C:\Users\you\.direct\linear-migrations\rehearsal-01
```

Everything is staged beside the output and published with one rename, so the
output directory is either complete or absent:

| Path | Contents |
|------|----------|
| `direct-import.json`, `direct-import.sha256` | Deterministic Direct archive (format 11) and its checksum |
| `workspace/direct.db` | Isolated Direct workspace restored from that archive |
| `reconciliation.json` | Counts, classification totals, field inventory, disambiguations, relation and cross-product reports, source limitations and cutover readiness |
| `accounting.json` | One entry per source record, plus components (label assignments, parents, previous identifiers, history, uploaded files) and data files |
| `source-bundle/` | Byte-identical copy of `manifest.json`, `manifest.sha256`, `attachment-manifest.json`, every `data/` file and every uploaded file |
| `source-bundle.sha256` | `sha256sum -c` compatible list of the bundle |
| `index.html` | Read-only human index mapping Direct keys to source records and bundle files |
| `import-complete.json` | Completion marker with the source manifest and generated file digests; written last |

### What is imported natively

All teams become products. All projects, milestones, initiatives and issues are
imported, including archived records and issues without a project (those use
the Inbox route). Source IDs, identifiers, URLs, titles, descriptions,
timestamps, issue history and original issue state are kept in Direct fields
and external provenance. `issue.external.history` keeps every captured history
entry; snapshot polling omits it (reporting `history_entries`) so large
imports stay light, while issue context, export and the retained record keep
every value.

- Completed issues become **Legacy done**. Open work (triage, unstarted,
  started) becomes Backlog; readiness, claims, verification runs and owner
  acceptance are never created.
- Every Linear label definition becomes its own Direct label with its Linear
  ID and original name in `linear_origins`; team labels are limited to their
  product. Names that collide case-insensitively are all kept distinct as
  `Name (TEAM)` or `Name (workspace)` and listed in `label_disambiguations`.
- Parent links run from child to parent. `A blocks B` is stored on B as
  `blocked_by A`. Related links, including links across projects of one
  product, are imported. A semantic duplicate (the reverse related record, or
  a repeated blocker) is merged into the existing link; its relation ID is
  still accounted and points to that link.
- A project that spans teams is assigned to the team with the most of its
  issues; issues from other teams keep their own product and their project
  membership is reported, not forced. An initiative whose projects span
  products becomes one goal per product with external IDs
  `<initiative id>:<team key>`.
- Duplicate project, goal or milestone names within a scope, over-long names,
  label descriptions over 1000 bytes and project/milestone ordering are
  normalized and reported.

### Retained sources: readable in Direct

Documents, planning updates, link attachments, uploaded files, users, comments
that do not belong to an issue, archived comments and relations, `duplicate`
(or other unsupported) relations, cross-product links and memberships,
initiatives without a product, previous identifiers, and populated fields
without a Direct field (for example due dates, estimates or project content)
are not converted into Direct planning records. They stay as **retained
sources**: every verified capture file is stored in Direct byte for byte, and
every accounted source record is indexed as the exact byte span of its
original file.

The owner reads them in **Imported sources**: bundle status, search by title,
identifier (including original Linear identifiers of renumbered issues) or
text, filters by kind and class, readable text fields shown as plain text, the
original record, and checksum-verified downloads of uploaded and data files.
Each issue's **Source** tab lists the retained records linked to it. Agents can
read the same data through `source_bundles`, `search_sources`,
`source_record` and `/api/source-file`. Retained content is untrusted data:
it is never rendered as HTML, executed, treated as instructions, or added to
Theoria guidance. If an imported issue is later deleted, its retained records
keep the reserved key and stay readable.

### Readiness gates

Reports separate four questions instead of one blanket flag:

| Gate | Meaning |
|------|---------|
| `preservation` | Every manifest-listed file matched its checksum and is retained. |
| `access` | `native` (a Direct record exists), `retained` (readable as an original source record in Direct) or `missing` (bytes never captured, for example a failed upload download). |
| `freshness` | Always `unverified`: the importer cannot know whether Linear changed after the capture. |
| `application` | `not_applied` (dry-run), `prepared` (artifact) or, in Direct, the live applied cursor. |

`cutover_readiness.status` stays `not_complete`. Blockers are the unverified
final-capture freshness, the pending live application, the owner's decision,
and any missing source bytes. `review_items` list what the owner should look
at: unsupported fields and relation semantics, unresolved records,
cross-product mappings, source limitations and trashed issues.

### Accounting classes

| Class | Meaning |
|-------|---------|
| `native` | Represented by Direct records and fields. |
| `transformed` | Represented after a reported change: a mapping, rename, split, merge, normalization, or populated fields that exist only in the bundle. |
| `preserved` | Not a Direct planning record; readable as a retained source record at the given pointer. |
| `unresolved` | Could not be represented or is missing (for example a failed upload download, a cross-product link, a cycle, or an initiative without a product). |

The importer refuses to finish unless every record of every data file has
exactly one record-level entry and every upload manifest entry is accounted.

### The index

`index.html` is a static page: all source text is HTML-escaped, a
`Content-Security-Policy` of `default-src 'none'` blocks scripts and remote
loads, and links point only to verified files in `source-bundle/`. Source URLs
are shown as text. Document text is shown escaped for reading, not imported as
Direct content or Theoria guidance.

### Replay and recovery

Run the exact command again against the same output. The importer regenerates
everything and succeeds only when the completion marker, every generated file,
every bundle file and the isolated workspace export match byte for byte; it
reports `idempotent_replay: true` and `duplicates_added: 0`. A changed source,
an incomplete output (no marker), a modified bundle file, or a workspace
edited after import is refused. Nothing is overwritten or merged.

Use `recovery-check` with a new directory to prove `direct-import.json`
restores byte for byte. The isolated workspace can also be opened with
`direct --data-dir <output>\workspace serve` for inspection.

## Single-project dry-run

```powershell
.\target\debug\direct.exe linear-import-dry-run `
  --source C:\Users\you\.direct\linear-migrations\linear-source-... `
  --project-id 00000000-0000-0000-0000-000000000000 `
  --output C:\Users\you\.direct\linear-migrations\dry-run-project
```

This writes `direct-import.json` and its checksum, `reconciliation.json` with
counts, unknown state or relation types, source limitations and cross-project
links omitted by the bounded selection, and `workspace/direct.db`. Replay and
Legacy done behave as above. Its `verified` flag covers only package integrity
and the deterministic restore of that project; `cutover_ready` is always
`false`.

## Cutover into an existing workspace

1. **Freeze and capture.** Freeze Linear intake, take a final capture
   ([source capture](linear-migration-source.md)) and verify it.
2. **Export the target.** Take a fresh backup of the owner's workspace
   (`direct backup <dir>`); the artifact binds to its workspace ID and event
   cursor. Never copy `direct.db`.
3. **Prepare.**

   ```powershell
   .\target\debug\direct.exe linear-migration-prepare `
     --source C:\Users\you\.direct\linear-migrations\linear-source-... `
     --target-export C:\Users\you\.direct\backups\direct-backup-....json `
     --output C:\Users\you\.direct\linear-migrations\cutover-01 `
     --map-team <linear team id>=<existing Direct product id>
   ```

   A Linear team whose key matches an existing Direct product is refused
   unless it is mapped with `--map-team`. A mapped team's issues go into that
   product without changing it; they keep their Linear identifiers when the
   product has no conflicting or history-reserved key, and otherwise receive
   new keys after the highest used number, each listed in
   `previous_identifiers`. Label names that collide with existing labels are
   kept distinct; an existing label whose `linear_origins` records a Linear
   label ID is reused, and that pairing is re-verified at preview and apply.
   Imported record IDs that already exist are refused.

   The output holds `linear-migration.direct-migration` (a binary artifact:
   header plus exact retained file bytes), its `.sha256`, `reconciliation.json`,
   `accounting.json`, `preview.json`, and `rehearsal-report.json`. The rehearsal
   restores the export into `rehearsal/direct.db`, then applies, replays,
   rolls back and re-applies the artifact through the same core transaction,
   comparing every pre-existing record each time. Preparation fails unless the
   rehearsal passes. Re-running the same command is an exact replay; a
   changed source or target is refused.
4. **Apply as the owner.** In Direct, open **Imported sources → Owner
   migration**, choose the artifact and **Check artifact**. The service
   re-checks every collision against the live workspace (IDs, product and issue
   keys, keys reserved by history, project and goal names, label taxonomy and
   reused-label origins) and validates the merged workspace. **Apply
   migration** requires the previewed cursor and artifact hash, writes a
   pre-import backup to `migration-backups/` under the data directory, and adds
   everything in one transaction. Existing records are never modified.
   Imported completed issues are Legacy done; no readiness, claims,
   verification runs or owner acceptance are created.
5. **Verify.** Review the bundle's gates, spot-check imported issues and
   retained records, and export a backup.

Applying the same artifact again reports `already_applied` and changes
nothing. A different artifact for the same capture, a changed capture whose
records already exist, or an artifact whose imported records were edited
since is refused.

**Undo.** While nothing has changed since the import, **Roll back this
import** removes exactly what it added (and the retained bundle) and records a
`migration_rolled_back` event. After later changes, recover from the
pre-import backup as described in [backup and recovery](backup-recovery.md).

### Limits

Preview and apply accept artifacts up to 1 GiB; a retained bundle may hold up
to 768 MiB with at most 256 MiB per file. The service reads the uploaded
artifact into memory, so expect memory use of roughly twice the artifact size
during preview and apply. Retained sources are included in every export and
backup.

## Limits of this increment

The captured Linear package has no release/deployment record type, so no
Direct release or production evidence is invented. The capture cannot recover
records already deleted from Linear or the contents of third-party link
attachments. Whether the final capture is current, the live application to
the owner's workspace, the owner's cutover decision and switching intake away
from Linear are operational steps outside the importer; reports never claim
them.
