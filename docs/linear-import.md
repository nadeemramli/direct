# Linear import dry-run

The offline importer converts a verified Linear source package into a separate
Direct workspace. It never calls Linear, the running Direct service, or the
owner's normal `%USERPROFILE%\.direct\data` workspace. It has two modes:

- `--project-id <uuid>` imports one project (the bounded dry-run).
- `--whole-workspace` rehearses the whole migration in one isolated workspace.

Exactly one mode is required. Run it with absolute private paths outside the
repository and outside the Direct data directory; both are refused.

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
and external provenance.

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

### What is only preserved, and why that blocks cutover

Documents, planning updates, link attachments, uploaded files, users, comments
that do not belong to an issue, archived comments and relations, `duplicate`
(or other unsupported) relations, cross-product links and memberships,
previous identifiers, and populated fields without a Direct field (for
example due dates, estimates or project content) are retained byte for byte in
`source-bundle/` and pointed to from `accounting.json` and `index.html`.

**The retained bundle is preservation, not native access.** Direct cannot show
or use that information, so `cutover_readiness.status` is always `blocked` in
this increment. Blockers always include the missing live apply command, the
final delta capture and the owner's cutover decision, and list any native
access gaps, unresolved records, unsupported fields and relation semantics,
cross-product mappings and source limitations. There is no blanket
`verified: true`.

### Accounting classes

| Class | Meaning |
|-------|---------|
| `native` | Represented by Direct records and fields. |
| `transformed` | Represented after a reported change: a mapping, rename, split, merge, normalization, or populated fields that exist only in the bundle. |
| `preserved` | Retained only in the source bundle at the given pointer. |
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

## Limits of this increment

The captured Linear package has no release/deployment record type, so no
Direct release or production evidence is invented. The capture cannot recover
records already deleted from Linear or the contents of third-party link
attachments. There is no live merge/apply command: the rehearsal never
replaces or combines with the owner's workspace. Full cutover still needs a
final delta capture, native access (or an explicit owner decision) for
preserved-only information, updated intake paths, and an explicit owner
decision.
