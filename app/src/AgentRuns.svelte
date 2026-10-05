<script lang="ts">
  // Agent runs for one issue (DIR-76). "Run now" records an intent and Direct
  // starts its runner; the executor has no tools and only proposes, and the
  // service applies or denies each proposal.
  import type { AgentRun, AssignmentContext, Issue } from "./api";
  import { RUN_STATE_LABEL, runBlockers, runStateClass } from "./runs";

  let {
    issue,
    assignment,
    runs,
    connected,
    clock,
    commit,
  }: {
    issue: Issue;
    assignment: AssignmentContext | null;
    runs: AgentRun[];
    connected: boolean;
    clock: number;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
  } = $props();

  let objective = $state("");
  let busy = $state(false);
  let error = $state("");
  const blockers = $derived(runBlockers(issue, assignment, runs, clock));
  const date = (at: number) => new Date(at * 1000).toLocaleString();

  async function run(command: Record<string, unknown>) {
    busy = true;
    error = "";
    try {
      await commit(command);
      objective = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if assignment || runs.length}
  <div class="section-label spaced">AGENT RUNS <span>{runs.length}</span></div>
  {#if assignment}
    <details class="reopen run-now">
      <summary>Run now…</summary>
      <p class="hint">
        Starts {assignment.member?.name || "the member"} with {assignment.role ? `${assignment.role.key} r${assignment.role.revision}` : "its role"}
        on <code>{assignment.requested_model}</code>. The session has no tools; it may only propose a comment
        or, while this issue is in Backlog, a new brief or acceptance. Anything else is refused.
      </p>
      <textarea bind:value={objective} maxlength="2000" placeholder="Objective, e.g. Tighten the acceptance criteria and flag missing owner checks"></textarea>
      {#if blockers.length}<ul class="source-warning">{#each blockers as b}<li>{b}</li>{/each}</ul>{/if}
      <button disabled={busy || !connected || !objective.trim() || blockers.length > 0}
        onclick={() => run({ op: "create_agent_run", key: issue.key, assignment_id: assignment!.assignment.id, objective })}>Run now</button>
      {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
    </details>
  {/if}
  {#each [...runs].reverse() as r (r.id)}
    <article class="cloud-handoff">
      <div class="reference-heading">
        <span class="source-state {runStateClass(r.state)}">{RUN_STATE_LABEL[r.state]}</span>
        {#if !["succeeded", "failed", "blocked", "unknown", "canceled", "cancel_pending"].includes(r.state)}
          <button class="text-button" disabled={busy || !connected} onclick={() => run({ op: "cancel_agent_run", id: r.id, expected_version: r.version })}>Cancel run</button>
        {/if}
      </div>
      <dl class="evidence reference-meta">
        <dt>Objective</dt><dd>{r.objective}</dd>
        <dt>Requested</dt><dd><code>{r.requested_model}</code>{r.actual_model ? "" : " · no session yet"}</dd>
        {#if r.actual_model}<dt>Session</dt><dd><code>{r.session_id}</code> · <code>{r.actual_model}</code>{r.actual_model !== r.requested_model ? " · substituted" : ""}{r.harness_version ? ` · ${r.harness_version}` : ""}</dd>{/if}
        <dt>Started</dt><dd>{r.created_by} · {date(r.created_at)} · issue v{r.input_issue_version}</dd>
        {#each r.actions as a (a.index)}
          <dt>Proposal {a.index + 1}</dt><dd><span class="check-chip {a.outcome === 'applied' ? 'passed' : 'failed'}">{a.outcome}</span> <code>{(a.proposal as { op?: string }).op || "?"}</code> — {a.detail}</dd>
        {/each}
        {#if r.summary}<dt>Summary</dt><dd>{r.summary}</dd>{/if}
        {#if r.reason}<dt>Reason</dt><dd class="source-warning">{r.reason}</dd>{/if}
      </dl>
    </article>
  {/each}
{/if}
