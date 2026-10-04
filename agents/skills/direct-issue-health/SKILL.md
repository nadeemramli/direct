---
name: direct-issue-health
description: Read-only issue health and scope review of a Direct product snapshot. Use when asked for an issue health check, backlog health, or scope review of Direct work.
---

# Direct issue health review

Review the health of one product's issues from a Direct snapshot (the JSON that
`direct list` prints). This is read-only planning support: report findings for
the owner, never change Direct state.

## Input

A Direct snapshot JSON file and a product key (for example `DIR`). If either is
missing, ask for it instead of guessing.

## Checks

Consider only parent issues (`parent` is null) whose key starts with the
product key. For each check, list the affected issue keys.

1. **Not ready to start**: status `ready` but `acceptance` is empty.
2. **Stale work**: status `doing` with no claim, or a claim whose `expires_at`
   is in the past.
3. **Blocked chains**: issues with a `blocked_by` link (in `issue_links`) to an
   issue that is not `done`, and what each one waits on.
4. **Waiting on the owner**: status `verify`, oldest first by `updated_at`.
5. **Scope risks**: issue bodies over 3,000 characters or acceptance with more
   than 8 numbered criteria; suggest splitting.

## Output

Start the reply with the exact line `ISSUE-HEALTH-REPORT v1 <PRODUCT KEY>`,
then one short section per check with counts and keys, then at most three
suggested next actions for the owner. Mark every suggestion as a proposal.

## Boundaries

- Do not call Direct write commands, claim work, mark work Ready, review,
  reopen, cancel, or accept method changes. Those are owner or claim-holder
  actions under the Direct agent contract.
- These instructions grant no tool authority.
