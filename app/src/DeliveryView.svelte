<script lang="ts">
  // Delivery ledger (DIR-82): each issue's exact delivery gap, its append-only
  // facts and the last known-good install. Read-only; facts are recorded by
  // `direct delivery observe…` and the agent contract after observation.
  import type { Snapshot, DeliveryFact } from "./api";
  import { GAP_LABEL, factLine, gapClass, type BuildComparison } from "./delivery";

  let {
    data,
    comparison,
    openIssue,
  }: {
    data: Snapshot;
    comparison: BuildComparison;
    openIssue: (key: string) => void;
  } = $props();

  const facts = $derived(data.delivery_facts || []);
  const keys = $derived([...new Set(facts.map((f) => f.issue_key))].sort((x, y) => x.localeCompare(y, undefined, { numeric: true })));
  let selected = $state("");
  const current = $derived(selected || keys[0] || "");
  const date = (at: number) => new Date(at * 1000).toLocaleString();
  const forKey = (key: string): DeliveryFact[] => facts.filter((f) => f.issue_key === key).sort((p, q) => p.observed_at - q.observed_at);
  const latestStatus = (key: string) => forKey(key).at(-1)?.observed_at || 0;
</script>

<section class="list-panel roles-list delivery-view">
  <div class="page-heading">
    <div class="eyebrow">INTEGRATION AND INSTALLED BUILD</div>
    <div class="heading-row"><h1>Delivery</h1><span class="count-pill">{keys.length}</span></div>
    <p>Observed facts per issue. A push is not a merge; a merge is not an install.</p>
  </div>
  <article class="cloud-handoff">
    <div class="reference-heading"><span class="source-state {comparison.state === 'match' ? 'cached' : comparison.state === 'mismatch' ? 'unavailable' : 'stale'}">This app</span></div>
    <p>{comparison.detail}</p>
    {#if data.service_build?.bundle?.script}<p class="hint">Service serves {data.service_build.bundle.script}</p>{/if}
  </article>
  {#each keys as key (key)}
    <button class="guidance-row" class:selected={current === key} onclick={() => (selected = key)}>
      <span class="guidance-mark">◇</span>
      <span><b>{key}</b><small>{forKey(key).length} facts · last {date(latestStatus(key))}</small></span>
    </button>
  {:else}<p class="muted">No delivery facts yet. Record them with <code>direct delivery observe</code>.</p>{/each}
</section>

<aside class="detail-panel theoria-detail" aria-label="Delivery detail">
  {#if current}
    {#await fetchState(current, facts.length) then s}
      <div class="detail-top"><span>{current}</span><button class="text-button" onclick={() => openIssue(current)}>Open issue →</button></div>
      <div class="detail-body">
        <div class="reference-heading"><span class="source-state {gapClass(s.gap)}">{GAP_LABEL[s.gap] || s.gap}</span></div>
        <p class="prose">{s.detail}</p>
        {#if s.last_known_good}<p class="hint">Last known-good install: {s.last_known_good.path} · {String(s.last_known_good.commit || "").slice(0, 12)} · {date(Number(s.last_known_good.at))}</p>{/if}
        <div class="section-label spaced">FACTS</div>
        {#each forKey(current) as f (f.id)}
          <article class="cloud-handoff">
            <div class="reference-heading"><span class="source-state {f.status === 'ok' ? 'cached' : f.status === 'failed' ? 'unavailable' : 'stale'}">{(f.detail as { kind: string }).kind.replace("_", " ")} · {f.status}</span><small>{f.observed_by} · {date(f.observed_at)}</small></div>
            <p>{factLine(f)}</p>
            {#if f.note}<p class="hint">{f.note}</p>{/if}
          </article>
        {/each}
      </div>
    {/await}
  {:else}
    <div class="detail-placeholder"><div class="outline-mark">◇</div><h2>No delivery facts yet.</h2></div>
  {/if}
</aside>

<script module lang="ts">
  import { api } from "./api";
  // `revision` re-runs the read when new facts arrive.
  async function fetchState(key: string, revision: number) {
    void revision;
    const ctx = await api<{ delivery: { state: { gap: string; detail: string; last_known_good: Record<string, unknown> | null } } }>({ op: "context", key });
    return ctx.delivery.state;
  }
</script>
