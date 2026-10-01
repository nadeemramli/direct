# Workspace intake templates (DIR-22)

Every product uses the same owner-managed intake shapes, so no product drifts onto an obsolete management format. A template shapes a concise brief (intent, execution mode, boundaries and verification). It does not preload a context package, and it grants no authority.

## Model

- **Template**: a stable ID, a fixed target (`issue` or `project`), a status (`active` or `retired`) and a pointer to its current revision. The head mirrors the current revision's name and shape for listing.
- **Revision**: an immutable row numbered 1, 2, 3, and so on. A revision contains the name, description, shape (`delivery`, `discovery_probe`, `bug` or `release`), base content, product supplements, a change note, the author and a timestamp. Revisions are inserted once and never replaced or deleted.
- **Base content**: an `intent` prompt (required), plus optional fields:
  - `execution_mode`: `agent`, `owner`, `paired` or `prototype`
  - `boundaries` and `verification` prompts, at most 2,000 characters each
  - a `checklist` of at most 12 items
  - `suggested_priority`
  - `suggested_planning_scope` (issue templates only)
  - up to 10 `suggested_labels`
- **Product supplement**: an explicit, bounded and additive extension for one product. It can contain a note (at most 1,000 characters), up to 5 extra checklist items and up to 5 extra labels that apply to the product. A supplement cannot replace or repeat a base field; unknown fields are rejected when the request is parsed. A revision has at most one supplement per product, and an empty supplement is refused, so silent copied variants cannot exist.

Templates are workspace-level and apply to every product. They have no per-product applicability list, because that would allow divergence; supplements are the only per-product difference.

## Commands

| Operation | Role | Notes |
| --- | --- | --- |
| `templates` | any (read-only) | Lists every head, retired ones included, and every revision. `snapshot` includes `templates` and `template_revisions`. |
| `create_template` | owner | Creates `target`, `name`, `shape`, `content`, optional `description`, `supplements` and `note`; the result is revision 1. |
| `revise_template` | owner | Takes `id`, `expected_version` and the full definition. Appends revision N+1. The target cannot change, and a retired template is read-only. |
| `retire_template` | owner | Takes `id`, `expected_version` and a nonblank `reason`. The template is no longer offered. Nothing is deleted and retirement is not reversible. |
| `create_issue` / `create_project` `template` | agent or owner (project creation stays owner-only) | `{template_id, revision, execution_mode?, labels?}` applies the current revision of an active template. |

Applying a template is refused when:

- the template is unknown (`not_found`)
- it is retired, or its target does not match the record (`invalid`)
- `revision` is not the current revision (`conflict`, so a stale form must reload)
- a kept label is not a suggestion of the base or of this product's supplement, or no longer applies to the product (`invalid`)

The record stores exact provenance in `template`:

- `template_id` and `revision`
- `supplement_product_id`, set when the revision had a supplement for the record's product
- the effective `execution_mode`
- `overrides`, the suggested fields the creator explicitly changed (`priority`, `planning_scope`, `execution_mode`, `labels`)
- `applied_by` and `applied_at`

The kept suggested labels are attached in the same transaction.

Issue `context` returns `template` and `project_template` as `{use, template, revision, current_revision, outdated, retired}`. The UI shows the exact revision, flags it as outdated or retired, and lists overrides.

## What templates never do

- Creating from a template leaves the record in Backlog with no claim, verification child or run. `ready` remains owner-only and still requires a real brief, acceptance criteria and a human owner. Checklist items are unchecked prompts in the acceptance text and are never recorded as passed.
- Definitions confer no permissions. Agents can read and apply them, but cannot create, revise or retire them. The existing operation-level role checks are unchanged.
- Later revisions and retirement never rewrite an existing record. Editing an issue's brief keeps its original provenance.
- Nothing is seeded. An empty or upgraded workspace has no templates until the owner creates them, and existing records stay untemplated.
- Template selection is optional. Free-form capture, **Draft from title** and the prototype-as-planning route all remain available. The `discovery_probe` shape with `prototype` execution mode lets a bounded probe be the plan.

## Storage, archives and compatibility

Schema and archive format 13 add the `templates` and `template_revisions` tables and collections, plus an optional `template` field on issues and projects.

- **Older archives**: formats 1–12 restore unchanged. They cannot carry template data; any template data requires format 13.
- **Request hashes**: new command fields are omitted from serialization when absent. A legacy request therefore hashes byte-for-byte as before, and exact retries replay their saved responses. The test `format_12_archive_restores_and_legacy_requests_replay_with_unchanged_hashes` uses a fixture generated by the pre-template build.
- **Untemplated records**: these records have no `template` key and serialize exactly as before.
- **Validation**: archive validation enforces:
  - contiguous revision chains (1..=current) whose target matches the head
  - a head name and shape that mirror the current revision
  - complete retirement provenance
  - bounded definitions with known products and labels
  - record provenance that references an existing revision with the matching target, the exact supplement for the record's product and known override names

  It does not re-check label applicability for archived revisions, so a label narrowed after the revision does not invalidate history; applying a template does check it.
- **Migration**: applying a migration validates the merged workspace at format 13, so workspaces with templates can still accept a prepared Linear migration. Imported records carry no template provenance.

## Tradeoffs

- Applying a template requires its current revision. Rejecting a stale form is safer than silently applying a newer definition than the one the creator saw.
- Overrides are computed by the service from typed suggestions; edits to the free-text starter are expected and are not counted as overrides.
- Retirement is one-way. To reuse a shape, create a new template, which keeps provenance unambiguous.
- Supplements live inside a revision. Changing one product's supplement creates a new revision of the shared template, so every use states exactly which base and supplement it saw.
