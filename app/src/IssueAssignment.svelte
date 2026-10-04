<script lang="ts">
  // The issue's agent assignment (DIR-75). Planning data only: assigning never
  // marks work Ready, reviews it, or takes or replaces a claim.
  import type { Snapshot, Issue, AssignmentContext } from "./api";
  import { memberChoices, modelChoices, roleChoices } from "./members";

  let {
    data,
    issue,
    assignment,
    connected,
    clock,
    commit,
  }: {
    data: Snapshot;
    issue: Issue;
    assignment: AssignmentContext | null;
    connected: boolean;
    clock: number;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
  } = $props();

  let memberId = $state("");
  let roleId = $state("");
  let model = $state("");
  let note = $state("");
  let reason = $state("");
  let busy = $state(false);
  let error = $state("");

  const members = $derived(memberChoices(data.agent_members || [], issue.product_id));
  const member = $derived((data.agent_members || []).find((m) => m.id === memberId));
  const roles = $derived(roleChoices(data.agent_roles || [], member));
  const models = $derived(modelChoices(member));
  const writer = $derived(issue.claim && issue.claim.expires_at > clock ? issue.claim.actor : null);
  const closed = $derived(["done", "legacy_completed", "canceled"].includes(issue.status) || !!issue.parent);

  async function run(command: Record<string, unknown>) {
    busy = true;
    error = "";
    try {
      await commit(command);
      note = "";
      reason = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  function assign() {
    run({
      op: "assign_issue_agent",
      key: issue.key,
      expected_assignment_version: assignment?.assignment.version ?? null,
      member_id: memberId,
      role_id: roleId,
      requested_model: model,
      reconcile_active_writer: writer ? note : null,
    });
  }
</script>

<div class="section-label spaced">AGENT ASSIGNMENT</div>
{#if assignment}
  {@const a = assignment.assignment}
  <article class="cloud-handoff">
    <dl class="evidence reference-meta">
      <dt>Member</dt><dd>{assignment.member?.name || a.member_id}{assignment.member ? ` · ${assignment.member.runtime}${assignment.member.enabled ? "" : " · disabled"}` : ""}</dd>
      <dt>Role</dt><dd>{assignment.role ? `${assignment.role.key} r${assignment.role.revision}` : a.role_id}</dd>
      <dt>Requested model</dt><dd><code>{assignment.requested_model}</code></dd>
      <dt>Actual model</dt><dd>{#if assignment.actual_model}<code>{assignment.actual_model}</code>{#if assignment.actual_model !== assignment.requested_model}{" · "}<span class="source-warning">differs from the request</span>{/if}{:else}<span class="muted">No session recorded yet</span>{/if}</dd>
      <dt>Assigned</dt><dd>{a.assigned_by} · {new Date(a.assigned_at * 1000).toLocaleString()} · version {a.version}</dd>
      {#if a.reconciliation}<dt>Reconciliation</dt><dd>{a.reconciliation}</dd>{/if}
    </dl>
  </article>
{:else}
  <p class="muted">Not assigned to an agent. The human owner field is unchanged.</p>
{/if}
{#if !closed}
  <details class="reopen assign-agent">
    <summary>{assignment ? "Reassign agent…" : "Assign agent…"}</summary>
    <label class="field">Member
      <select bind:value={memberId} onchange={() => { roleId = member?.default_role_key ? (data.agent_roles || []).find((r) => r.key === member.default_role_key && r.status === "active")?.id || "" : ""; model = ""; }}>
        <option value="">Choose a member…</option>
        {#each members as c (c.item.id)}<option value={c.item.id} disabled={!!c.blocked}>{c.item.name} · {c.item.runtime}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}
      </select>
    </label>
    <label class="field">Role revision
      <select bind:value={roleId} disabled={!member}>
        <option value="">Choose a role…</option>
        {#each roles as c (c.item.id)}<option value={c.item.id} disabled={!!c.blocked}>{c.item.key} r{c.item.revision}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}
      </select>
    </label>
    <label class="field">Requested model
      <select bind:value={model} disabled={!member}>
        <option value="">Choose a model…</option>
        {#each models as c (c.item)}<option value={c.item} disabled={!!c.blocked}>{c.item}{c.blocked ? ` — ${c.blocked}` : ""}</option>{/each}
      </select>
      {#if member && !models.some((c) => !c.blocked)}<small class="source-warning">No verified model for {member.name}. Run its capability check from Agents.</small>{/if}
    </label>
    {#if writer}
      <label class="field">Reconciliation with the active writer
        <textarea bind:value={note} maxlength="1000" placeholder="How {writer}'s current work is handled. Assigning never replaces their claim."></textarea>
      </label>
    {/if}
    <button disabled={busy || !connected || !memberId || !roleId || !model || (!!writer && !note.trim())} onclick={assign}>{assignment ? "Reassign" : "Assign"}</button>
    {#if assignment}
      <input aria-label="Reason to clear the assignment" bind:value={reason} placeholder="Reason to clear…" />
      <button class="secondary" disabled={busy || !connected || !reason.trim()} onclick={() => run({ op: "clear_issue_assignment", id: assignment!.assignment.id, expected_version: assignment!.assignment.version, reason })}>Clear assignment</button>
    {/if}
    {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
  </details>
{/if}
