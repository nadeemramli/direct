<script lang="ts">
  // Product Manager routines (DIR-78). Routines are created paused; the service
  // schedules due occurrences as Product Planner reviews and keeps quiet runs
  // quiet. Notices are local; external delivery is not available.
  import { untrack } from "svelte";
  import type { Snapshot, Routine, Status } from "./api";
  import { memberChoices, modelChoices, roleChoices } from "./members";
  import { WEEKDAYS, dueLabel, latestOccurrences, openNotices, routineState, triggerLabel } from "./routines";

  let {
    data,
    connected,
    commit,
    api,
  }: {
    data: Snapshot;
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
    api: <T>(command: Record<string, unknown>) => Promise<T>;
  } = $props();

  const routines = $derived(data.routines || []);
  let selectedId = $state("");
  const selected = $derived(routines.find((r) => r.id === selectedId) || null);
  let creating = $state(false);
  let busy = $state(false);
  let error = $state("");
  let preview = $state<{ keys: string[]; excluded: { key: string; reason: string }[]; next_due_at: number; dispatch_block: string | null } | null>(null);

  let form = $state({
    name: "Daily issue health",
    productId: untrack(() => data.products[0]?.id || ""),
    states: ["backlog"] as Status[],
    memberId: "",
    roleId: "",
    model: "",
    policy: "inspect_only",
    objective: "Daily issue health: flag unclear outcomes, oversized scope, missing criteria, duplicates and dependency problems. Leave clear issues alone.",
    kind: "daily",
    time: "09:00",
    weekday: "mon",
    timezone: "Asia/Kuala_Lumpur",
    maxIssues: 10,
    maxMinutes: 15,
    maxCost: "",
  });
  const members = $derived(memberChoices(data.agent_members || [], form.productId));
  const member = $derived((data.agent_members || []).find((m) => m.id === form.memberId));
  const roles = $derived(roleChoices(data.agent_roles || [], member));
  const models = $derived(modelChoices(member));
  const productKey = (id: string) => data.products.find((p) => p.id === id)?.key || "?";
  const memberName = (id: string) => (data.agent_members || []).find((m) => m.id === id)?.name || "?";

  function config() {
    return {
      product: productKey(form.productId),
      states: form.states,
      member_id: form.memberId,
      role_id: form.roleId,
      requested_model: form.model,
      policy: form.policy,
      objective: form.objective,
      trigger: { kind: form.kind, time: form.time, weekday: form.kind === "weekly" ? form.weekday : null, timezone: form.timezone },
      limits: { max_issues: Number(form.maxIssues), max_minutes: Number(form.maxMinutes), max_cost_usd: form.maxCost ? Number(form.maxCost) : null },
      notify: "local",
    };
  }
  async function run(fn: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await fn();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  const toggleState = (s: Status) => (form.states = form.states.includes(s) ? form.states.filter((x) => x !== s) : [...form.states, s]);
  const setStatus = (r: Routine, status: string) => run(() => commit({ op: "set_routine_status", id: r.id, expected_version: r.version, status }));
</script>

<section class="list-panel roles-list routines-view">
  <div class="page-heading">
    <div class="eyebrow">RECURRING PRODUCT CHECKS</div>
    <div class="heading-row"><h1>Routines</h1><span class="count-pill">{openNotices(data.routine_notices || []).length}</span></div>
    <p>Scheduled Product Planner reviews. Created paused; quiet runs stay quiet.</p>
  </div>
  <button class="secondary" disabled={!connected} onclick={() => { creating = true; selectedId = ""; preview = null; }}>New routine</button>
  {#each routines as r (r.id)}
    {@const rev = r.revisions[r.revisions.length - 1]}
    <button class="guidance-row" class:selected={selectedId === r.id} onclick={() => { selectedId = r.id; creating = false; }}>
      <span class="guidance-mark">{rev.trigger.kind === "weekly" ? "W" : "D"}</span>
      <span><b>{r.name}</b><small>{triggerLabel(rev.trigger)} · next {dueLabel(r.next_due_at, rev.trigger.timezone)}</small></span>
      <span class="source-state {r.status === 'active' ? 'cached' : 'stale'}">{routineState(r, data.restored_at || null)}</span>
    </button>
  {:else}<p class="muted">No routines yet.</p>{/each}
  {#if openNotices(data.routine_notices || []).length}
    <div class="section-label spaced">NOTICES</div>
    {#each openNotices(data.routine_notices || []) as n (n.id)}
      <article class="cloud-handoff">
        <div class="reference-heading"><span class="source-state {n.kind === 'failure' || n.kind === 'blocked' ? 'unavailable' : 'cached'}">{n.kind.replace("_", " ")}</span><small>{new Date(n.at * 1000).toLocaleString()}</small></div>
        <p>{n.message}</p>
        {#if n.action}<p class="hint">Next: {n.action}</p>{/if}
        <button class="text-button" disabled={busy || !connected} onclick={() => run(() => commit({ op: "acknowledge_routine_notice", id: n.id }))}>Mark seen</button>
      </article>
    {/each}
  {/if}
</section>

<aside class="detail-panel theoria-detail" aria-label="Routine">
  {#if creating}
    <div class="detail-top"><span>New routine (created paused)</span></div>
    <div class="modal-form routine-form">
      <label class="field">Name<input bind:value={form.name} maxlength="120" /></label>
      <label class="field">Product<select bind:value={form.productId} onchange={() => { form.memberId = ""; form.roleId = ""; form.model = ""; }}>{#each data.products as p (p.id)}<option value={p.id}>{p.key} · {p.name}</option>{/each}</select></label>
      <fieldset class="field"><legend>Issue states</legend>{#each ["backlog", "ready", "doing"] as s}<label class="release-claim"><input type="checkbox" checked={form.states.includes(s as Status)} onchange={() => toggleState(s as Status)} /> {s}</label>{/each}</fieldset>
      <label class="field">Member<select bind:value={form.memberId}><option value="">Choose…</option>{#each members as c (c.item.id)}<option value={c.item.id} disabled={!!c.blocked}>{c.item.name}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}</select></label>
      <label class="field">Role revision<select bind:value={form.roleId} disabled={!member}><option value="">Choose…</option>{#each roles as c (c.item.id)}<option value={c.item.id} disabled={!!c.blocked}>{c.item.key} r{c.item.revision}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}</select></label>
      <label class="field">Model<select bind:value={form.model} disabled={!member}><option value="">Choose…</option>{#each models as c (c.item)}<option value={c.item} disabled={!!c.blocked}>{c.item}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}</select></label>
      <fieldset class="field"><legend>Policy</legend>
        <label class="release-claim"><input type="radio" bind:group={form.policy} value="inspect_only" /> Inspect only</label>
        <label class="release-claim"><input type="radio" bind:group={form.policy} value="refine_backlog" /> Refine Backlog</label>
      </fieldset>
      <label class="field">Objective<textarea bind:value={form.objective} maxlength="2000"></textarea></label>
      <fieldset class="field"><legend>Schedule</legend>
        <select bind:value={form.kind}><option value="daily">Daily</option><option value="weekly">Weekly</option></select>
        {#if form.kind === "weekly"}<select bind:value={form.weekday}>{#each WEEKDAYS as d}<option value={d}>{d}</option>{/each}</select>{/if}
        <input type="time" bind:value={form.time} />
        <input aria-label="Timezone" bind:value={form.timezone} placeholder="Asia/Kuala_Lumpur" />
      </fieldset>
      <fieldset class="field"><legend>Limits per run</legend>
        <label>Issues <input type="number" min="1" max="20" bind:value={form.maxIssues} /></label>
        <label>Minutes <input type="number" min="1" max="120" bind:value={form.maxMinutes} /></label>
        <label>Max cost USD <input type="number" min="0" step="0.01" bind:value={form.maxCost} placeholder="none" /></label>
      </fieldset>
      <p class="hint">Notices are local to Direct. External notifications need separate authorization.</p>
      <button class="secondary" disabled={busy || !connected || !form.memberId || !form.roleId || !form.model} onclick={() => run(async () => (preview = await api({ op: "preview_routine", config: config() })))}>Preview</button>
      {#if preview}
        <dl class="evidence reference-meta">
          <dt>Would review</dt><dd>{preview.keys.join(", ") || "Nothing in scope"}</dd>
          {#if preview.excluded.length}<dt>Excluded</dt><dd>{#each preview.excluded as x}<code>{x.key}</code> {x.reason}<br />{/each}</dd>{/if}
          <dt>First due</dt><dd>{dueLabel(preview.next_due_at, form.timezone)}</dd>
          {#if preview.dispatch_block}<dt>Blocked</dt><dd class="source-warning">{preview.dispatch_block}</dd>{/if}
        </dl>
      {/if}
      <button disabled={busy || !connected || !form.name.trim() || !form.memberId || !form.roleId || !form.model || !form.states.length} onclick={() => run(async () => { const r = await commit<Routine>({ op: "create_routine", name: form.name, config: config() }); creating = false; selectedId = r.id; })}>Create paused routine</button>
      {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
    </div>
  {:else if selected}
    {@const rev = selected.revisions[selected.revisions.length - 1]}
    <div class="detail-top"><span>{selected.name} · revision {rev.revision} · v{selected.version}</span></div>
    <div class="detail-body">
      <dl class="evidence reference-meta">
        <dt>State</dt><dd>{routineState(selected, data.restored_at || null)}</dd>
        <dt>Schedule</dt><dd>{triggerLabel(rev.trigger)}</dd>
        <dt>Next due</dt><dd>{dueLabel(selected.next_due_at, rev.trigger.timezone)}</dd>
        <dt>Scope</dt><dd>{productKey(rev.product_id)} · {rev.states.join(", ")} · up to {rev.limits.max_issues} issues · {rev.policy === "refine_backlog" ? "Refine Backlog" : "Inspect only"}</dd>
        <dt>Agent</dt><dd>{memberName(rev.member_id)} · <code>{rev.requested_model}</code></dd>
        <dt>Limits</dt><dd>{rev.limits.max_minutes} min{rev.limits.max_cost_usd ? ` · $${rev.limits.max_cost_usd}` : " · no cost limit"}</dd>
      </dl>
      <div class="routine-actions">
        {#if selected.status !== "retired"}
          {#if selected.status === "active" && !selected.held_reason}<button class="secondary" disabled={busy || !connected} onclick={() => setStatus(selected!, "paused")}>Pause</button>
          {:else}<button disabled={busy || !connected} onclick={() => setStatus(selected!, "active")}>{selected.status === "paused" && !selected.activated_at ? "Activate" : "Resume"}</button>{/if}
          <button class="secondary" disabled={busy || !connected} onclick={() => run(() => commit({ op: "run_routine_now", id: selected!.id }))}>Run now</button>
          <button class="secondary" disabled={busy || !connected} onclick={() => setStatus(selected!, "retired")}>Retire</button>
        {/if}
      </div>
      {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
      <div class="section-label spaced">OCCURRENCES</div>
      {#each latestOccurrences(data.routine_occurrences || [], selected.id) as o (o.id)}
        {@const runRow = (data.agent_runs || []).find((x) => x.id === o.run_id)}
        <article class="cloud-handoff">
          <div class="reference-heading"><span class="source-state {o.state === 'launched' ? 'cached' : o.state === 'blocked' ? 'unavailable' : 'stale'}">{o.state}{o.manual ? " · manual" : ""}{o.coalesced ? ` · ${o.coalesced} missed folded in` : ""}</span><small>due {dueLabel(o.due_at, rev.trigger.timezone)}</small></div>
          {#if o.reason}<p class="hint">{o.reason}</p>{/if}
          {#if runRow}<p class="hint">Review {runRow.state} · {runRow.actual_model || "no session yet"}{runRow.cost_usd != null ? ` · $${runRow.cost_usd.toFixed(2)}` : " · cost unknown"} · {runRow.summary}</p>{/if}
        </article>
      {:else}<p class="muted">No occurrences yet.</p>{/each}
    </div>
  {:else}
    <div class="detail-placeholder"><div class="outline-mark">◷</div><h2>Select or create a routine.</h2></div>
  {/if}
</aside>
