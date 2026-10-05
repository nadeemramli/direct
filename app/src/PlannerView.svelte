<script lang="ts">
  // Product Planner (DIR-77): start a bounded queue review and read findings.
  // Reviews run through the DIR-76 boundary: the session has no tools, and
  // Direct applies only findings (and, in refine mode, Backlog edits).
  import { untrack } from "svelte";
  import type { Snapshot } from "./api";
  import { memberChoices, modelChoices, roleChoices } from "./members";
  import { outcomeCounts, queueRuns, reviewable } from "./planner";
  import { RUN_STATE_LABEL, runStateClass } from "./runs";
  import PlannerFindings from "./PlannerFindings.svelte";

  let {
    data,
    connected,
    commit,
  }: {
    data: Snapshot;
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
  } = $props();

  // Opens on the first product; the owner picks another here.
  let productId = $state(untrack(() => data.products[0]?.id || ""));
  let keys = $state<string[]>([]);
  let memberId = $state("");
  let roleId = $state("");
  let model = $state("");
  let policy = $state<"inspect_only" | "refine_backlog">("inspect_only");
  let objective = $state("Review this queue for unclear outcomes, oversized scope, missing criteria, duplicates and dependency problems.");
  let busy = $state(false);
  let error = $state("");

  const product = $derived(data.products.find((p) => p.id === productId));
  const issues = $derived(reviewable(data.issues, productId));
  const members = $derived(memberChoices(data.agent_members || [], productId));
  const member = $derived((data.agent_members || []).find((m) => m.id === memberId));
  const roles = $derived(roleChoices(data.agent_roles || [], member));
  const models = $derived(modelChoices(member));
  const runs = $derived(queueRuns(data.agent_runs || [], productId));
  const findings = $derived((data.planner_findings || []).filter((f) => f.product_id === productId));
  const open = $derived(runs.some((r) => !["succeeded", "failed", "blocked", "unknown", "canceled"].includes(r.state)));
  const date = (at: number) => new Date(at * 1000).toLocaleString();

  function toggle(key: string) {
    keys = keys.includes(key) ? keys.filter((k) => k !== key) : keys.length < 20 ? [...keys, key] : keys;
  }
  async function start() {
    busy = true;
    error = "";
    try {
      await commit({ op: "create_queue_run", product: product?.key, keys, member_id: memberId, role_id: roleId, requested_model: model, policy, objective });
      keys = [];
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="list-panel roles-list planner-view">
  <div class="page-heading">
    <div class="eyebrow">ISSUE HEALTH AND SCOPE</div>
    <div class="heading-row"><h1>Planner</h1><span class="count-pill">{findings.length}</span></div>
    <p>Review an explicit queue with the Product Planner role. Findings are proposals; readiness stays with you.</p>
  </div>
  <label class="field">Product
    <select bind:value={productId} onchange={() => { keys = []; memberId = ""; roleId = ""; model = ""; }}>
      {#each data.products as p (p.id)}<option value={p.id}>{p.key} · {p.name}</option>{/each}
    </select>
  </label>
  <div class="section-label spaced">QUEUE <span>{keys.length}/20</span></div>
  <div class="planner-queue">
    {#each issues as i (i.key)}
      <label class="release-claim"><input type="checkbox" checked={keys.includes(i.key)} onchange={() => toggle(i.key)} /> <code>{i.key}</code> <small>{i.status}</small> {i.title}</label>
    {/each}
  </div>
  <label class="field">Member
    <select bind:value={memberId} onchange={() => { roleId = member?.default_role_key ? (data.agent_roles || []).find((r) => r.key === member.default_role_key && r.status === "active")?.id || "" : ""; model = ""; }}>
      <option value="">Choose a member…</option>
      {#each members as c (c.item.id)}<option value={c.item.id} disabled={!!c.blocked}>{c.item.name}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}
    </select>
  </label>
  <label class="field">Role revision
    <select bind:value={roleId} disabled={!member}>
      <option value="">Choose a role…</option>
      {#each roles as c (c.item.id)}<option value={c.item.id} disabled={!!c.blocked}>{c.item.key} r{c.item.revision}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}
    </select>
  </label>
  <label class="field">Model
    <select bind:value={model} disabled={!member}>
      <option value="">Choose a model…</option>
      {#each models as c (c.item)}<option value={c.item} disabled={!!c.blocked}>{c.item}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}
    </select>
  </label>
  <fieldset class="field"><legend>Policy</legend>
    <label class="release-claim"><input type="radio" bind:group={policy} value="inspect_only" /> Inspect only — findings, no issue writes</label>
    <label class="release-claim"><input type="radio" bind:group={policy} value="refine_backlog" /> Refine Backlog — may also comment and rewrite Backlog briefs (previous text kept)</label>
  </fieldset>
  <label class="field">Objective<textarea bind:value={objective} maxlength="2000"></textarea></label>
  {#if open}<p class="source-warning">A review of this product has not finished.</p>{/if}
  <button disabled={busy || !connected || open || !keys.length || !memberId || !roleId || !model || !objective.trim()} onclick={start}>Start review</button>
  {#if error}<p class="source-warning" role="alert">{error}</p>{/if}

  <div class="section-label spaced">REVIEWS <span>{runs.length}</span></div>
  {#each runs as r (r.id)}
    {@const n = outcomeCounts(r)}
    <article class="cloud-handoff">
      <div class="reference-heading">
        <span class="source-state {runStateClass(r.state)}">{RUN_STATE_LABEL[r.state]}</span>
        <small>{r.queue?.policy === "refine_backlog" ? "Refine Backlog" : "Inspect only"} · {date(r.created_at)}</small>
      </div>
      <dl class="evidence reference-meta">
        <dt>Queue</dt><dd>{r.queue?.keys.join(", ")}</dd>
        {#if r.queue?.blocked.length}<dt>Excluded</dt><dd>{#each r.queue.blocked as b}<code>{b.key}</code> {b.reason}<br />{/each}</dd>{/if}
        {#if r.actual_model}<dt>Session</dt><dd><code>{r.session_id}</code> · <code>{r.actual_model}</code></dd>{/if}
        <dt>Outcome</dt><dd>{n.applied} applied · {n.retained} retained · {n.denied} denied</dd>
        {#each r.actions.filter((a) => a.outcome === "denied") as a (a.index)}<dt>Denied</dt><dd><code>{(a.proposal as { op?: string }).op || "?"}</code> {(a.proposal as { issue_key?: string }).issue_key || ""} — {a.detail}</dd>{/each}
        {#if r.summary}<dt>Summary</dt><dd>{r.summary}</dd>{/if}
        {#if r.reason}<dt>Reason</dt><dd class="source-warning">{r.reason}</dd>{/if}
      </dl>
    </article>
  {/each}
</section>

<aside class="detail-panel theoria-detail" aria-label="Planner findings">
  <div class="detail-top"><span>{product?.key || ""} findings</span></div>
  <div class="detail-body">
    <PlannerFindings {findings} {connected} {commit} showIssue={true} />
  </div>
</aside>
