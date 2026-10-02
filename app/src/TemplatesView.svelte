<script lang="ts">
  // Owner management of workspace intake templates (DIR-22). Definitions are
  // immutable revisions: saving appends a revision, retiring stops new use, and
  // nothing here deletes provenance or touches existing issues and projects.
  import type { Snippet } from "svelte";
  import { api } from "./api";
  import type {
    ExecutionMode,
    PlanningScope,
    Snapshot,
    TemplateContent,
    TemplateRevision,
    TemplateShape,
    TemplateTarget,
    WorkspaceTemplate,
  } from "./api";
  import { MODES, SHAPES, emptyContent, modeLabel, shapeLabel } from "./templates";

  let {
    data,
    connected,
    refresh,
    controls,
  }: {
    data: Snapshot;
    connected: boolean;
    refresh: () => Promise<void>;
    controls?: Snippet;
  } = $props();

  interface SupplementDraft {
    product_id: string;
    note: string;
    checklist: string;
    suggested_labels: string[];
  }
  interface Draft {
    id: string;
    version: number;
    target: TemplateTarget;
    name: string;
    description: string;
    shape: TemplateShape;
    intent: string;
    execution_mode: ExecutionMode | "";
    boundaries: string;
    verification: string;
    checklist: string;
    suggested_priority: string;
    suggested_planning_scope: PlanningScope | "";
    suggested_labels: string[];
    supplements: SupplementDraft[];
    note: string;
  }

  let selectedId = $state("");
  let draft = $state<Draft | null>(null);
  let retiring = $state("");
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");

  let templates = $derived(
    [...(data.templates || [])].sort(
      (a, b) =>
        Number(a.status === "retired") - Number(b.status === "retired") ||
        a.name.localeCompare(b.name),
    ),
  );
  let selected = $derived(templates.find((item) => item.id === selectedId));
  let history = $derived(
    (data.template_revisions || [])
      .filter((item) => item.template_id === selectedId)
      .sort((a, b) => b.revision - a.revision),
  );
  let current = $derived(history[0]);
  let usage = $derived.by(() => {
    const records = [
      ...data.issues.filter((issue) => !issue.parent).map((issue) => issue.template),
      ...data.projects.map((project) => project.template),
    ].filter((used) => used?.template_id === selectedId);
    const counts = new Map<number, number>();
    for (const used of records) counts.set(used!.revision, (counts.get(used!.revision) || 0) + 1);
    return counts;
  });
  let outdatedUses = $derived(
    selected
      ? [...usage.entries()]
          .filter(([revision]) => revision < selected.current_revision)
          .reduce((sum, [, count]) => sum + count, 0)
      : 0,
  );
  let labels = $derived([...(data.labels || [])].sort((a, b) => a.name.localeCompare(b.name)));
  function labelName(id: string) {
    return labels.find((label) => label.id === id)?.name || "Unknown label";
  }
  function productName(id: string) {
    return data.products.find((product) => product.id === id)?.name || "Unknown product";
  }
  function appliesTo(labelId: string, productId: string) {
    const label = labels.find((item) => item.id === labelId);
    return (
      !!label &&
      (label.products.length === 0 || label.products.some((rule) => rule.product_id === productId))
    );
  }
  function lines(text: string) {
    return text
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
  }
  function date(at: number) {
    return new Date(at * 1000).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function startCreate() {
    selectedId = "";
    retiring = "";
    error = "";
    notice = "";
    draft = {
      id: "",
      version: 0,
      target: "issue",
      name: "",
      description: "",
      shape: "delivery",
      intent: "",
      execution_mode: "",
      boundaries: "",
      verification: "",
      checklist: "",
      suggested_priority: "",
      suggested_planning_scope: "",
      suggested_labels: [],
      supplements: data.products.map((product) => ({
        product_id: product.id,
        note: "",
        checklist: "",
        suggested_labels: [],
      })),
      note: "",
    };
  }
  function startRevise(head: WorkspaceTemplate, revision: TemplateRevision) {
    retiring = "";
    error = "";
    notice = "";
    const content = revision.content;
    draft = {
      id: head.id,
      version: head.version,
      target: head.target,
      name: revision.name,
      description: revision.description,
      shape: revision.shape,
      intent: content.intent,
      execution_mode: content.execution_mode || "",
      boundaries: content.boundaries,
      verification: content.verification,
      checklist: content.checklist.join("\n"),
      suggested_priority: content.suggested_priority || "",
      suggested_planning_scope: content.suggested_planning_scope || "",
      suggested_labels: [...content.suggested_labels],
      supplements: data.products.map((product) => {
        const existing = revision.supplements.find((item) => item.product_id === product.id);
        return {
          product_id: product.id,
          note: existing?.note || "",
          checklist: (existing?.checklist || []).join("\n"),
          suggested_labels: [...(existing?.suggested_labels || [])],
        };
      }),
      note: "",
    };
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!draft || busy) return;
    busy = true;
    error = "";
    const content: TemplateContent = {
      ...emptyContent(),
      intent: draft.intent,
      execution_mode: draft.execution_mode || null,
      boundaries: draft.boundaries,
      verification: draft.verification,
      checklist: lines(draft.checklist),
      suggested_priority: draft.suggested_priority || null,
      suggested_planning_scope:
        draft.target === "issue" ? draft.suggested_planning_scope || null : null,
      suggested_labels: draft.suggested_labels,
    };
    // Only explicit, non-empty supplements are sent; empty ones never exist.
    const supplements = draft.supplements
      .map((item) => ({
        product_id: item.product_id,
        note: item.note.trim(),
        checklist: lines(item.checklist),
        suggested_labels: item.suggested_labels,
      }))
      .filter((item) => item.note || item.checklist.length || item.suggested_labels.length);
    const common = {
      name: draft.name,
      description: draft.description,
      shape: draft.shape,
      content,
      supplements,
      note: draft.note,
    };
    try {
      const saved = await api<{ template: WorkspaceTemplate }>(
        draft.id
          ? { op: "revise_template", id: draft.id, expected_version: draft.version, ...common }
          : { op: "create_template", target: draft.target, ...common },
        true,
      );
      await refresh();
      selectedId = saved.template.id;
      notice = `Saved revision ${saved.template.current_revision}. Existing records keep the revision they were created from.`;
      draft = null;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      busy = false;
    }
  }
  async function retire(event: SubmitEvent) {
    event.preventDefault();
    if (!selected || busy) return;
    busy = true;
    error = "";
    try {
      await api(
        { op: "retire_template", id: selected.id, expected_version: selected.version, reason: retiring },
        true,
      );
      await refresh();
      retiring = "";
      notice = "Retired. It is no longer offered for new work; existing records and revisions remain readable.";
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      busy = false;
    }
  }
</script>

<section class="list-panel templates-list" aria-label="Intake templates">
  <div class="page-heading">
    <div class="eyebrow">SHARED INTAKE · OWNER-MANAGED</div>
    <div class="heading-row">
      <h1>Intake templates</h1>
      <span class="count-pill">{templates.length}</span>
    </div>
    <p>
      One shared base for every product, with explicit product supplements. Templates shape
      intent, execution mode, boundaries and verification prompts. They never make work Ready,
      grant tool authority or count as verification.
    </p>
  </div>
  <div class="project-toolbar">
    <button class="secondary" disabled={!connected || busy} onclick={startCreate}
      >＋ New template</button
    >
  </div>
  <div class="template-rows">
    {#each templates as item}<button
        class="trace-row template-row"
        class:active={item.id === selectedId}
        aria-label={`Template ${item.name}`}
        onclick={() => {
          selectedId = item.id;
          draft = null;
          retiring = "";
          error = "";
          notice = "";
        }}
        ><b>{item.name}</b>
        <small
          >{item.target} · {shapeLabel(item.shape)} · v{item.current_revision}{item.status ===
          "retired"
            ? " · retired"
            : ""}</small
        ></button
      >{:else}<div class="empty compact">
        <div class="empty-symbol">▦</div>
        <h3>No templates yet</h3>
        <p>Nothing is seeded automatically. Create the shapes your intake needs.</p>
      </div>{/each}
  </div>
</section>
<aside class="detail-panel template-detail" aria-label="Template detail">
  <div class="detail-top">
    <span>{draft ? (draft.id ? "Revise template" : "New template") : "Template"}</span>
    <div>{#if controls}{@render controls()}{/if}</div>
  </div>
  <div class="detail-body">
    {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
    {#if notice}<p class="hint" role="status">{notice}</p>{/if}
    {#if draft}
      <form class="template-form" onsubmit={save}>
        {#if !draft.id}<label class="field"
            >Shapes<select aria-label="Template target" bind:value={draft.target}
              ><option value="issue">Issue intake</option><option value="project"
                >Project intake</option
              ></select
            ></label
          >{/if}
        <div class="form-grid">
          <label class="field"
            >Name<input aria-label="Template name" required maxlength="120" bind:value={draft.name} /></label
          ><label class="field"
            >Shape<select aria-label="Template shape" bind:value={draft.shape}
              >{#each SHAPES as shape}<option value={shape.value}>{shape.label}</option>{/each}</select
            ></label
          >
        </div>
        <label class="field"
          >Description<input aria-label="Template description" maxlength="1000" bind:value={draft.description} /></label
        >
        <label class="field"
          >Intent prompt<textarea aria-label="Intent prompt" required rows="2" maxlength="2000" bind:value={draft.intent}
          ></textarea></label
        >
        <div class="form-grid">
          <label class="field"
            >Suggested execution mode<select aria-label="Suggested execution mode" bind:value={draft.execution_mode}
              ><option value="">None</option>{#each MODES as mode}<option value={mode.value}
                  >{mode.label}</option
                >{/each}</select
            ></label
          ><label class="field"
            >Suggested priority<select aria-label="Suggested priority" bind:value={draft.suggested_priority}
              ><option value="">None</option><option value="urgent">Urgent</option><option
                value="high">High</option
              ><option value="medium">Medium</option><option value="low">Low</option></select
            ></label
          >
        </div>
        {#if draft.target === "issue"}<label class="field"
            >Suggested work route<select
              aria-label="Suggested work route"
              bind:value={draft.suggested_planning_scope}
              ><option value="">None</option><option value="project">Project work</option><option
                value="inbox">Inbox / maintenance</option
              ></select
            ></label
          >{/if}
        <label class="field"
          >Boundaries prompt<textarea aria-label="Boundaries prompt" rows="2" maxlength="2000" bind:value={draft.boundaries}
          ></textarea></label
        >
        <label class="field"
          >Verification prompt<textarea aria-label="Verification prompt" rows="2" maxlength="2000" bind:value={draft.verification}
          ></textarea></label
        >
        <label class="field"
          >Checklist (one per line, at most 12)<textarea
            aria-label="Checklist"
            rows="3"
            bind:value={draft.checklist}
          ></textarea></label
        >
        {#if labels.length}<fieldset class="field template-labels">
            <legend>Suggested labels</legend>
            {#each labels as label}<label class="checkbox"
                ><input
                  type="checkbox"
                  checked={draft.suggested_labels.includes(label.id)}
                  onchange={(event) => {
                    if (!draft) return;
                    draft.suggested_labels = event.currentTarget.checked
                      ? [...draft.suggested_labels, label.id]
                      : draft.suggested_labels.filter((id) => id !== label.id);
                  }}
                />{label.name}</label
              >{/each}
          </fieldset>{/if}
        <div class="section-label spaced">PRODUCT SUPPLEMENTS · ADDITIVE ONLY</div>
        <p class="hint">
          A supplement can add a note, up to 5 checklist items and up to 5 labels for one product.
          It cannot replace the shared base. Leave it empty for the base alone.
        </p>
        {#each draft.supplements as supplement}<details
            class="template-supplement"
            open={!!(supplement.note || supplement.checklist || supplement.suggested_labels.length)}
          >
            <summary>{productName(supplement.product_id)}</summary>
            <label class="field"
              >Note<input
                aria-label={`Supplement note for ${productName(supplement.product_id)}`}
                maxlength="1000"
                bind:value={supplement.note}
              /></label
            >
            <label class="field"
              >Additional checklist<textarea
                aria-label={`Supplement checklist for ${productName(supplement.product_id)}`}
                rows="2"
                bind:value={supplement.checklist}
              ></textarea></label
            >
            {#each labels.filter((label) => !draft?.suggested_labels.includes(label.id) && appliesTo(label.id, supplement.product_id)) as label}<label
                class="checkbox"
                ><input
                  type="checkbox"
                  checked={supplement.suggested_labels.includes(label.id)}
                  onchange={(event) => {
                    supplement.suggested_labels = event.currentTarget.checked
                      ? [...supplement.suggested_labels, label.id]
                      : supplement.suggested_labels.filter((id) => id !== label.id);
                  }}
                />{label.name}</label
              >{/each}
          </details>{/each}
        <label class="field"
          >Revision note<input aria-label="Revision note" maxlength="1000" bind:value={draft.note} /></label
        >
        <div class="modal-footer">
          <button type="button" class="text-button" onclick={() => (draft = null)}>Cancel</button>
          <button class="primary" disabled={busy || !connected}
            >{draft.id ? "Save new revision" : "Create template"}</button
          >
        </div>
      </form>
    {:else if selected && current}
      <div class="detail-heading">
        <span class="status-badge" class:ready={selected.status === "active"}
          >{selected.status === "active" ? "Active" : "Retired"}</span
        >
        <h2>{selected.name}</h2>
        <p class="hint">
          {selected.target} intake · {shapeLabel(selected.shape)} · current revision {selected.current_revision}
          {#if outdatedUses}· <b>{outdatedUses} record{outdatedUses === 1 ? "" : "s"} on an older revision</b>{/if}
        </p>
        {#if selected.status === "retired"}<p class="source-warning">
            Retired {selected.retired_at ? date(selected.retired_at) : ""} by {selected.retired_by}: {selected.retired_reason}.
            Not offered for new work; existing provenance stays readable.
          </p>{/if}
      </div>
      {#if selected.status === "active"}<div class="actions">
          <button class="secondary" disabled={!connected || busy} onclick={() => startRevise(selected, current)}
            >Revise</button
          >
        </div>
        <form class="retire-form" onsubmit={retire}>
          <label class="field"
            >Retire with reason<input
              aria-label="Retirement reason"
              maxlength="500"
              required
              bind:value={retiring}
              placeholder="Why this shape should no longer be used"
            /></label
          ><button class="text-button" disabled={!connected || busy || !retiring.trim()}>Retire template</button>
        </form>{/if}
      {#each history as revision}<section
          class="template-revision"
          aria-label={`Revision ${revision.revision}`}
        >
          <div class="section-label spaced">
            REVISION {revision.revision}{revision.revision === selected.current_revision ? " · CURRENT" : ""}
            <span class="tiny">{date(revision.created_at)} · {revision.created_by} · used by {usage.get(revision.revision) || 0}</span>
          </div>
          {#if revision.note}<p class="hint">{revision.note}</p>{/if}
          <dl class="template-fields">
            <dt>Name</dt><dd>{revision.name} · {shapeLabel(revision.shape)}</dd>
            <dt>Intent</dt><dd>{revision.content.intent}</dd>
            <dt>Execution mode</dt><dd>{modeLabel(revision.content.execution_mode)}</dd>
            {#if revision.content.boundaries}<dt>Boundaries</dt><dd>{revision.content.boundaries}</dd>{/if}
            {#if revision.content.verification}<dt>Verification</dt><dd>{revision.content.verification}</dd>{/if}
            {#if revision.content.checklist.length}<dt>Checklist</dt><dd>{revision.content.checklist.join(" · ")}</dd>{/if}
            {#if revision.content.suggested_priority}<dt>Priority</dt><dd>{revision.content.suggested_priority}</dd>{/if}
            {#if revision.content.suggested_planning_scope}<dt>Work route</dt><dd>{revision.content.suggested_planning_scope}</dd>{/if}
            {#if revision.content.suggested_labels.length}<dt>Labels</dt><dd>{revision.content.suggested_labels.map(labelName).join(", ")}</dd>{/if}
            {#each revision.supplements as supplement}<dt>+ {productName(supplement.product_id)}</dt><dd
                >{[supplement.note, ...supplement.checklist, ...supplement.suggested_labels.map(labelName)]
                  .filter(Boolean)
                  .join(" · ")}</dd
              >{/each}
          </dl>
        </section>{/each}
    {:else}<div class="detail-placeholder">
        <div class="outline-mark">▦</div>
        <h2>Select or create a template.</h2>
        <p>Revisions are immutable; issues and projects keep the exact revision they were created from.</p>
      </div>{/if}
  </div>
</aside>
