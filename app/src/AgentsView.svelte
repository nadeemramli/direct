<script lang="ts">
  // Agent members (DIR-75). The owner adds and configures members; capability
  // comes only from `direct members check` runs against the real harness.
  import type { Snapshot, AgentMember } from "./api";
  import { RUNTIMES, capabilityLabel, checkCommand } from "./members";

  let {
    data,
    connected,
    commit,
  }: {
    data: Snapshot;
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
  } = $props();

  const members = $derived(data.agent_members || []);
  const roleKeys = $derived([...new Set((data.agent_roles || []).map((r) => r.key))].sort());
  const productKey = (id: string) => data.products.find((p) => p.id === id)?.key || "?";

  let editing = $state<AgentMember | null>(null);
  let creating = $state(false);
  let draft = $state({ name: "", runtime: "claude-code", connection_ref: "", product_ids: [] as string[], default_role_key: "", default_model: "", enabled: true });
  let busy = $state(false);
  let error = $state("");

  function open(m: AgentMember | null) {
    error = "";
    editing = m;
    creating = !m;
    draft = m
      ? { name: m.name, runtime: m.runtime, connection_ref: m.connection_ref, product_ids: [...m.product_ids], default_role_key: m.default_role_key || "", default_model: m.default_model || "", enabled: m.enabled }
      : { name: "", runtime: "claude-code", connection_ref: "", product_ids: [], default_role_key: "", default_model: "", enabled: true };
  }

  async function save(e: Event) {
    e.preventDefault();
    busy = true;
    error = "";
    const common = {
      name: draft.name,
      connection_ref: draft.connection_ref,
      product_ids: draft.product_ids,
      default_role_key: draft.default_role_key || null,
      default_model: draft.default_model || null,
    };
    try {
      await commit(
        editing
          ? { op: "update_agent_member", id: editing.id, expected_version: editing.version, enabled: draft.enabled, ...common }
          : { op: "create_agent_member", runtime: draft.runtime, ...common },
      );
      editing = null;
      creating = false;
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }

  function toggleProduct(id: string) {
    draft.product_ids = draft.product_ids.includes(id)
      ? draft.product_ids.filter((p) => p !== id)
      : [...draft.product_ids, id];
  }
</script>

<section class="list-panel roles-list">
  <div class="page-heading">
    <div class="eyebrow">WHO CAN BE ASSIGNED</div>
    <div class="heading-row">
      <h1>Agents</h1>
      <span class="count-pill">{members.filter((m) => m.enabled).length}</span>
    </div>
    <p>Agent members of this workspace, their runtime, scope and verified models.</p>
  </div>
  <button class="secondary" disabled={!connected} onclick={() => open(null)}>Add agent member</button>
  {#each members as m (m.id)}
    <button class="guidance-row" class:selected={editing?.id === m.id} onclick={() => open(m)}>
      <span class="guidance-mark">{m.runtime === "codex" ? "X" : "C"}</span>
      <span><b>{m.name}</b><small>{m.runtime} · {m.product_ids.map(productKey).join(", ")} · {capabilityLabel(m)}</small></span>
      <span class="source-state {m.enabled ? 'cached' : 'unavailable'}">{m.enabled ? "enabled" : "disabled"}</span>
    </button>
  {:else}
    <div class="empty compact">
      <div class="empty-symbol">◇</div>
      <h3>No agent members</h3>
      <p>Add a member, then run a capability check before assigning work.</p>
    </div>
  {/each}
</section>

<aside class="detail-panel theoria-detail" aria-label="Agent member">
  {#if editing || creating}
    <div class="detail-top"><span>{editing ? `${editing.name} · version ${editing.version}` : "New agent member"}</span></div>
    <form class="modal-form member-form" onsubmit={save}>
      <label class="field">Name<input bind:value={draft.name} required maxlength="120" /></label>
      <label class="field">Runtime
        <select bind:value={draft.runtime} disabled={!!editing}>
          {#each RUNTIMES as r}<option value={r}>{r}</option>{/each}
        </select>
      </label>
      <label class="field">Connection reference
        <input bind:value={draft.connection_ref} required maxlength="200" placeholder="e.g. local Claude Code on this PC" />
        <small class="hint">A label only. URLs, accounts, tokens and keys are refused.</small>
      </label>
      <fieldset class="field"><legend>Permitted products</legend>
        {#each data.products as p (p.id)}
          <label class="release-claim"><input type="checkbox" checked={draft.product_ids.includes(p.id)} onchange={() => toggleProduct(p.id)} /> {p.key} · {p.name}</label>
        {/each}
      </fieldset>
      <label class="field">Default role
        <select bind:value={draft.default_role_key}>
          <option value="">None</option>
          {#each roleKeys as k}<option value={k}>{k}</option>{/each}
        </select>
      </label>
      <label class="field">Default model<input bind:value={draft.default_model} maxlength="80" placeholder="e.g. claude-opus-5-5" /></label>
      {#if editing}<label class="release-claim"><input type="checkbox" bind:checked={draft.enabled} /> Enabled (disabled members cannot receive new assignments)</label>{/if}
      {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
      <button disabled={busy || !connected || !draft.name.trim() || !draft.connection_ref.trim() || !draft.product_ids.length}>{editing ? "Save member" : "Add member"}</button>
    </form>
    {#if editing}
      <div class="section-label spaced">CAPABILITY</div>
      <p class="prose">{capabilityLabel(editing)}</p>
      {#if editing.capability}<p class="hint">Checked by {editing.capability.checked_by} · {new Date(editing.capability.checked_at * 1000).toLocaleString()} · {editing.capability.evidence}</p>{/if}
      <p class="hint">Verify models through the real harness: <code>{checkCommand(editing)}</code></p>
    {/if}
  {:else}
    <div class="detail-placeholder">
      <div class="outline-mark">◈</div>
      <h2>Select or add an agent.</h2>
      <p>Members carry a runtime, product scope and verified models.<br />Assign them from an issue's detail.</p>
    </div>
  {/if}
</aside>
