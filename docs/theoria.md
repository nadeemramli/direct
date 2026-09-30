# Theoria: guidance and method learning

Theoria is Direct's read-only bridge to the Development Operating System. It puts relevant principles, playbook versions, experimental guidance, and method findings beside operational work without turning Direct into a second knowledge base.

## Authority boundary

- The Development Operating System remains the maintained source for principles, playbooks, version definitions, experiments, and accepted method changes.
- Direct stores a cache for navigation, the source path and fingerprint used by an issue, and operational evidence produced while doing the work.
- Source documents are task context, not tool authorization. Importing or linking one never grants permission to run commands or change owner-controlled state.
- A method finding is always a proposal. The Theoria API has no operation to accept a proposal, enroll or promote an experiment, change Jev routing, or edit the source documents.

The repository catalog at `theoria/catalog.json` is the explicit allow-list. Every entry has a stable ID, a path relative to an explicitly supplied root, a title, category, and navigation description. Direct rejects absolute or escaping catalog paths. It does not crawl a vault or infer that nearby Markdown belongs to the catalog.

## Explicit sync

Run the importer with the authoritative Development Operating System directory as an absolute root:

```powershell
.\target\debug\direct.exe --actor coding-agent theoria-sync `
  --root "C:\path\to\Development Operating System" `
  --catalog .\theoria\catalog.json `
  --product DIR `
  --request-id stable-sync-request-id
```

The CLI reads only the catalogued UTF-8 Markdown files, computes SHA-256 fingerprints, captures the source `updated` value when present, and sends the bounded result through the normal Direct command contract. Sync is manual and idempotent by request ID; it is not a filesystem watcher.

For an available source, Direct replaces the cache and records `checked_at` and `cached_at`. If a source is missing or unreadable, Direct marks it unavailable, records the reason and check time, and retains any prior content and fingerprint as a visibly labelled last cache. It does not claim that retained content is current. A later fingerprint does not rewrite the fingerprint recorded on earlier issue links, so stale historical use remains visible.

## Issue links and findings

`link_theoria` connects an issue to a catalog document using the issue's current version. It records the current fingerprint and an optional playbook version. Omitting the version preserves `Unknown`; Direct never guesses a historical assignment.

`create_method_finding` records:

- classification: product defect, method friction, or both;
- observed fact;
- hypothesis or unresolved question;
- proposed improvement;
- typed evidence pointers to an issue, build, check, or owner review.

Agents need an active claim to link guidance or create findings for Doing work. Direct automatically includes the origin issue as evidence if it was omitted. Findings appear both on the issue and in the Theoria proposal list, labelled `PROPOSAL · NOT ACCEPTED`.

The snapshot includes the current document cache and proposal index. Issue context includes its method findings, while the issue itself carries its pinned guidance references. Both survive service restart and archive format 3 export/restore.

## Review posture

The owner continues to decide readiness and second-pass acceptance in the normal Direct workflow. The agent first reads relevant current guidance, records its fingerprint/known playbook version, and performs the [first-pass E2E verification](e2e-delivery.md). Only an explicit agent Pass with evidence reaches human review. The human reviews intent, usability, evidence and material risks with focused spot checks, rather than repeating deterministic testing.

The maintained source protocol is `dos-e2e-verification`; refresh the cache after an authorized canonical update. The cached document and its fingerprint must match the source before claiming the new guidance is available. Automated checks can support a finding, but they do not accept the method proposal. Accepted Development Operating System changes must be maintained in its authoritative knowledge workflow with owner-direction provenance; Direct retains the originating issue and evidence trail. The two-pass policy is an owner decision, not automatic promotion of an experimental playbook or proof of improved performance.
