<script lang="ts">
  // Planner findings (DIR-77). Proposals for the owner; they never change an
  // issue by themselves. Escalations wait for a coordinator's decision.
  import type { PlannerFinding } from "./api";
  import { KIND_LABEL, ROUTE_LABEL, orderedFindings } from "./planner";

  let {
    findings,
    connected,
    commit,
    showIssue = false,
  }: {
    findings: PlannerFinding[];
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
    showIssue?: boolean;
  } = $props();

  let notes = $state<Record<string, string>>({});
  let busy = $state(false);
  let error = $state("");
  const date = (at: number) => new Date(at * 1000).toLocaleString();

  async function decide(f: PlannerFinding, confirmed: boolean) {
    busy = true;
    error = "";
    try {
      await commit({ op: "confirm_planner_finding", id: f.id, expected_version: f.version, confirmed, note: notes[f.id] || "" });
      notes[f.id] = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#each orderedFindings(findings) as f (f.id)}
  {@const esc = f.escalation as Record<string, unknown> | null}
  <article class="cloud-handoff planner-finding">
    <div class="reference-heading">
      <span class="source-state {esc && !f.confirmation ? 'stale' : 'cached'}">{KIND_LABEL[f.kind] || f.kind}{showIssue ? ` · ${f.issue_key}` : ""}</span>
      <small>{f.route ? ROUTE_LABEL[f.route] || f.route : ""} · seen {f.seen}× · {date(f.updated_at)}</small>
    </div>
    <b>{f.summary}</b>
    <dl class="evidence reference-meta">
      <dt>Evidence</dt><dd>{#each f.evidence as e}<span class="check-chip">{e}</span>{/each}</dd>
      <dt>Recommendation</dt><dd>{f.recommendation}</dd>
      {#if f.revisions.length}<dt>Revisions</dt><dd>{f.revisions.length} earlier version{f.revisions.length === 1 ? "" : "s"} (inputs changed){#each f.revisions as r}<br /><small>{date(r.at)}: {r.summary}</small>{/each}</dd>{/if}
      {#if esc}
        <dt>Escalation</dt><dd>
          <b>Criterion:</b> {esc.criterion}<br />
          <b>Evidence:</b> {esc.evidence}<br />
          <b>Impact:</b> {esc.impact}<br />
          <b>Options:</b> {Array.isArray(esc.options) ? (esc.options as string[]).join("; ") : esc.options}<br />
          <b>Recommendation:</b> {esc.recommendation}
        </dd>
        {#if f.confirmation}
          <dt>Coordinator</dt><dd>{f.confirmation.confirmed ? "Confirmed" : "Dismissed"} by {f.confirmation.by} · {f.confirmation.note}</dd>
        {:else}
          <dt>Coordinator</dt><dd>
            <input aria-label="Decision note" bind:value={notes[f.id]} placeholder="Why you confirm or dismiss…" />
            <button disabled={busy || !connected || !(notes[f.id] || "").trim()} onclick={() => decide(f, true)}>Confirm</button>
            <button class="secondary" disabled={busy || !connected || !(notes[f.id] || "").trim()} onclick={() => decide(f, false)}>Dismiss</button>
          </dd>
        {/if}
      {/if}
    </dl>
  </article>
{:else}
  <p class="muted">No planner findings.</p>
{/each}
{#if error}<p class="source-warning" role="alert">{error}</p>{/if}
