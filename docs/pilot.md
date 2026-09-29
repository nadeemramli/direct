# Direct builds Direct

Started 29 September 2026. The normal local workspace (`%USERPROFILE%\.direct\data`) owns Direct development tasks. The separate `Direct-foundation-preview` workspace contains only synthetic smoke-test records and does not count toward adoption.

## First delivery

DIR-1 adds project grouping: create/rename a project, assign/unassign an issue, filter work, include the project in agent context, and preserve it through backup/restore. The user authorized starting the pilot in chat. Implementation is tracked under actor `codex-pilot`; the owner records the final verification outcome in Direct.

Projects are the first increment. Milestones/goals and general issue relationships follow as separate deliveries. No Linear data is being migrated as part of this first task.

## Seeded work

| Issue | Next capability |
| --- | --- |
| DIR-1 | Project grouping |
| DIR-2 | Milestones and goals |
| DIR-3 | Parent, blocker and related links |
| DIR-4 | Native acceptance and daily-use packaging |
| DIR-5 | Repository agent workflow and CLI usability |
| DIR-6 | Routine backups and recovery drill |
| DIR-7 | Complete Linear source package |
| DIR-8 | Repeatable importer and reconciliation |
| DIR-9 | Ten real deliveries and adoption decision |
| DIR-10 | Optional local MCP |

Read live status and acceptance criteria in Direct. `scripts/seed-pilot.ps1` performs the initial capture using stable request IDs and leaves existing evolved issues untouched on rerun. It does not make work Ready, claim work, or mark it Done.

## Working loop

1. Owner authorizes a bounded task and readiness is recorded.
2. Agent reads context, claims, implements, and records actual checks.
3. Agent submits a specific build and manual steps. The item appears in Needs me.
4. Owner tests and passes or fails it. Failed work is fixed and resubmitted as a new run; cancellation is never success.

Count only real parent issues with owner-recorded passing outcomes toward the ten-delivery pilot. Verification children and synthetic tests do not add to the count. Record failures when they happen; do not manufacture failures to satisfy a metric.

Existing products remain in Linear. A later cutover requires a complete source package, reconciled dry-run, restorable backup, final delta, updated agent/intake paths, and a per-product adoption decision. No billing cancellation is part of this pilot.
