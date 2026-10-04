<script lang="ts">
  // Roles & skills (DIR-74). Agents register draft revisions and publish them
  // into disposable projects through the CLI; owners review the pins, skill
  // bundles and fresh-session evidence here, then activate or retire.
  import type { Snapshot, AgentRole, SkillPackage } from "./api";
  import { activationBlockers, harnessStatus, pinState, roleGroups, skillLabel } from "./roles";

  let {
    data,
    connected,
    commit,
  }: {
    data: Snapshot;
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
  } = $props();

  const roles = $derived(data.agent_roles || []);
  const skills = $derived(data.skill_packages || []);
  const publications = $derived(data.role_publications || []);
  const documents = $derived(data.theoria_documents || []);
  const groups = $derived(roleGroups(roles));

  let selectedId = $state("");
  let note = $state("");
  let reason = $state("");
  let busy = $state(false);
  let error = $state("");

  const selected = $derived(
    roles.find((r) => r.id === selectedId) || groups[0]?.revisions[0] || null,
  );
  const date = (at: number) => new Date(at * 1000).toLocaleString();

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
  const skillFor = (id: string): SkillPackage | undefined => skills.find((s) => s.id === id);
  const pubsFor = (r: AgentRole) => publications.filter((p) => p.role_id === r.id);
</script>

<section class="list-panel roles-list">
  <div class="page-heading">
    <div class="eyebrow">WHO · WITH WHAT · UNDER WHICH GUIDANCE</div>
    <div class="heading-row">
      <h1>Roles & skills</h1>
      <span class="count-pill">{roles.length}</span>
    </div>
    <p>Immutable revisions of role contracts and selected project-scoped skills.</p>
  </div>
  <div class="authority-note">
    <b>Methods stay in DOS</b>
    <p>
      Direct records which DOS documents a role pinned, its exact skill bundle and
      its evidence. Role text is task context and grants no tool authority;
      activation is an owner decision.
    </p>
  </div>
  {#each groups as group (group.key)}
    <div class="list-label"><span>{group.key.toUpperCase()}</span><span>STATUS</span></div>
    {#each group.revisions as r (r.id)}
      <button class="guidance-row" class:selected={selected?.id === r.id} onclick={() => (selectedId = r.id)}>
        <span class="guidance-mark">r{r.revision}</span>
        <span><b>{r.name}</b><small>{r.registered_by} · {date(r.registered_at)}</small></span>
        <span class="source-state role-{r.status}">{r.status}</span>
      </button>
    {/each}
  {:else}
    <div class="empty compact">
      <div class="empty-symbol">◇</div>
      <h3>No roles registered</h3>
      <p>Agents register draft revisions with <code>register_agent_role</code>.</p>
    </div>
  {/each}
  <div class="section-label spaced">SKILL PACKAGES <span>{skills.length}</span></div>
  {#each skills as s (s.id)}
    <details class="skill-package">
      <summary>
        <b>{s.name} r{s.revision}</b>
        <small>{s.origin}{s.upstream ? ` · ${s.upstream.repository}@${s.upstream.commit.slice(0, 10)}` : ""} · {s.files.length} files · <code>{s.bundle_sha256.slice(0, 12)}</code>{s.retired ? " · retired" : ""}</small>
      </summary>
      <dl class="evidence reference-meta">
        <dt>Trigger</dt><dd>{s.trigger}</dd>
        <dt>License</dt><dd>{s.license}</dd>
        {#if s.adaptations}<dt>Adaptations</dt><dd>{s.adaptations}</dd>{/if}
        <dt>Registered</dt><dd>{s.registered_by} · {date(s.registered_at)}</dd>
        {#if s.retired}<dt>Retired</dt><dd>{s.retired.by} · {s.retired.reason}</dd>{/if}
      </dl>
      {#each s.files as f (f.path)}
        <details class="skill-file"><summary><code>{f.path}</code> <small>{f.sha256.slice(0, 12)}</small></summary>
          <pre class="guidance-content">{f.content}</pre></details>
      {/each}
      {#if !s.retired}<button class="secondary" disabled={busy || !connected || !reason.trim()}
          onclick={() => run({ op: "retire_skill_package", id: s.id, reason })}>Retire with the reason below</button>{/if}
    </details>
  {/each}
</section>

<aside class="detail-panel theoria-detail" aria-label="Role detail">
  {#if selected}
    {@const blockers = activationBlockers(selected, skills, documents)}
    <div class="detail-top"><span>{selected.key} · revision {selected.revision}</span></div>
    <div class="detail-heading">
      <span class="status-badge ready">◫ {selected.status}</span>
      <h2>{selected.name}</h2>
    </div>
    <div class="detail-body">
      <div class="section-label">RESPONSIBILITIES</div>
      <ul class="prose">{#each selected.responsibilities as item}<li>{item}</li>{/each}</ul>
      <div class="section-label">INPUTS</div>
      <ul class="prose">{#each selected.inputs as item}<li>{item}</li>{/each}</ul>
      <div class="section-label">OUTPUTS</div>
      <ul class="prose">{#each selected.outputs as item}<li>{item}</li>{/each}</ul>

      <div class="section-label">SKILLS</div>
      {#each selected.skills as id}
        {@const s = skillFor(id)}
        <p class="prose">{skillLabel(skills, id)}{#if s} · <code>{s.bundle_sha256.slice(0, 12)}</code>{s.retired ? " · retired" : ""}{/if}</p>
      {:else}<p class="muted">No skills required.</p>{/each}

      <div class="section-label">RUNTIMES</div>
      <p class="prose">
        {#each harnessStatus(selected, publications) as h}<span class="check-chip {h.verified ? 'passed' : ''}">{h.harness} · {h.verified ? "verified by fresh session" : "unverified"}</span>{/each}
      </p>

      <div class="section-label">PINNED DOS GUIDANCE</div>
      {#each selected.guidance as pin (pin.document_id)}
        {@const state = pinState(pin, documents)}
        <article class="guidance-reference">
          <div class="reference-heading"><span class="source-state {state}">{state}</span>{#if pin.mandatory}<small>mandatory</small>{/if}</div>
          <b>{documents.find((d) => d.id === pin.document_id)?.title || pin.document_id}</b>
          <dl class="evidence reference-meta">
            <dt>Recorded fingerprint</dt><dd><code>{pin.recorded_fingerprint || "unavailable"}</code></dd>
            <dt>Current fingerprint</dt><dd><code>{documents.find((d) => d.id === pin.document_id)?.fingerprint || "unavailable"}</code></dd>
            <dt>Playbook version</dt><dd class="playbook-version" class:unknown={!pin.playbook_version}>{pin.playbook_version || "Unknown — not recorded"}</dd>
          </dl>
          {#if state === "stale"}<p class="source-warning">The DOS source changed after this revision pinned it. The pin is kept; register a new revision to adopt the change.</p>{/if}
        </article>
      {/each}

      <div class="section-label">OWNER DIRECTION</div>
      <p class="prose" class:muted={!selected.owner_direction}>{selected.owner_direction || "Not recorded. Activation needs it."}</p>
      {#if selected.activation}<p class="hint">Activated by {selected.activation.by} · {date(selected.activation.at)}{selected.activation.note ? ` — ${selected.activation.note}` : ""}</p>{/if}
      {#if selected.retired}<p class="hint">Retired by {selected.retired.by} · {selected.retired.reason}</p>{/if}

      <div class="section-label spaced">PUBLICATIONS <span>{pubsFor(selected).length}</span></div>
      {#each pubsFor(selected) as p (p.id)}
        <article class="cloud-handoff">
          <dl class="evidence reference-meta">
            <dt>Destination</dt><dd><code>{p.destination}</code> · {p.harness}</dd>
            <dt>Introduced</dt><dd>{#each p.introduced as f}<code>{f.path}</code><br />{/each}</dd>
            <dt>Published</dt><dd>{p.published_by} · {date(p.published_at)}</dd>
            {#each p.evidence as e}
              <dt>Fresh session</dt><dd><code>{e.session_id}</code> · <code>{e.model}</code>{e.harness_version ? ` · ${e.harness_version}` : ""} · marker <code>{e.marker}</code></dd>
              <dt>Output</dt><dd><pre class="guidance-content">{e.output_excerpt}</pre></dd>
            {/each}
            {#if p.rollback}<dt>Rolled back</dt><dd>{p.rollback.by} · removed {p.rollback.removed.length}{p.rollback.kept_modified.length ? `, kept ${p.rollback.kept_modified.length} modified` : ""}</dd>{/if}
          </dl>
        </article>
      {:else}<p class="muted">Not published. Use <code>direct roles publish --role-id {selected.id} --dest &lt;project&gt;</code> to preview.</p>{/each}

      <div class="section-label spaced">OWNER DECISION</div>
      {#if selected.status === "draft"}
        {#if blockers.length}<ul class="source-warning">{#each blockers as b}<li>{b}</li>{/each}</ul>{/if}
        <input aria-label="Activation note" bind:value={note} placeholder="Activation note (scope, review cadence)…" />
        <button disabled={busy || !connected || blockers.length > 0} onclick={() => run({ op: "activate_agent_role", id: selected.id, note })}>Activate revision {selected.revision}</button>
      {/if}
      {#if selected.status !== "retired"}
        <input aria-label="Retirement reason" bind:value={reason} placeholder="Reason to retire…" />
        <button class="secondary" disabled={busy || !connected || !reason.trim()} onclick={() => run({ op: "retire_agent_role", id: selected.id, reason })}>Retire revision</button>
      {/if}
      {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
    </div>
  {:else}
    <div class="detail-placeholder">
      <div class="outline-mark">◫</div>
      <h2>No role selected.</h2>
      <p>Register a role revision to review it here.</p>
    </div>
  {/if}
</aside>
