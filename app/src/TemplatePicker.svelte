<script lang="ts">
  // Intake template selection for issue and project creation (DIR-22).
  // Selecting a template only pre-fills editable starter text and suggestions;
  // the server records the exact revision and any explicit overrides.
  import type {
    ExecutionMode,
    Label,
    Snapshot,
    TemplateSelection,
    TemplateTarget,
    PlanningScope,
  } from "./api";
  import {
    MODES,
    composeBrief,
    composeVerification,
    effective,
    modeLabel,
    revisionOf,
    selectable,
    shapeLabel,
  } from "./templates";

  export interface Applied {
    brief: string;
    verification: string;
    priority: string | null;
    planning_scope: PlanningScope | null;
  }

  let {
    data,
    target,
    productId,
    selection = $bindable(null),
    onapply,
  }: {
    data: Snapshot;
    target: TemplateTarget;
    productId: string;
    selection?: TemplateSelection | null;
    onapply: (applied: Applied) => void;
  } = $props();

  let options = $derived(selectable(data, target));
  let chosenId = $state("");
  let mode = $state<ExecutionMode | "">("");
  let kept = $state<string[]>([]);
  let head = $derived(options.find((item) => item.id === chosenId));
  let revision = $derived(head ? revisionOf(data, head.id, head.current_revision) : undefined);
  let labels = $derived(
    revision
      ? effective(revision, productId)
          .labels.map((id) => (data.labels || []).find((label) => label.id === id))
          .filter(
            (label): label is Label =>
              !!label &&
              (label.products.length === 0 ||
                label.products.some((rule) => rule.product_id === productId)),
          )
      : [],
  );
  let supplement = $derived(revision ? effective(revision, productId).supplement : null);

  function sync() {
    if (!head || !revision) {
      selection = null;
      return;
    }
    selection = {
      template_id: head.id,
      revision: head.current_revision,
      // Omitted accepts the suggestion; a different mode, or null for "Not set",
      // is the creator's explicit choice and is recorded as an override.
      ...((mode || null) !== (revision.content.execution_mode ?? null)
        ? { execution_mode: mode || null }
        : {}),
      ...(kept.length ? { labels: kept } : {}),
    };
  }
  function apply() {
    // No template: nothing is suggested, so untouched fields return to the form defaults.
    if (!revision) {
      onapply({ brief: "", verification: "", priority: null, planning_scope: null });
      return;
    }
    onapply({
      brief: composeBrief(revision, productId, mode || null),
      verification: composeVerification(revision, productId),
      priority: revision.content.suggested_priority,
      planning_scope: revision.content.suggested_planning_scope,
    });
  }
  // Suggested labels the creator explicitly unchecked, per template, for the life of
  // this form. Suggestions start kept; an explicit un-check survives product changes
  // (including A→B→A, where the label is absent for B) and template switches.
  const declined = new Map<string, Set<string>>();
  function declinedFor(templateId: string) {
    if (!declined.has(templateId)) declined.set(templateId, new Set());
    return declined.get(templateId)!;
  }
  function keptSuggestions() {
    const off = declinedFor(chosenId);
    return labels.map((label) => label.id).filter((id) => !off.has(id));
  }
  function toggle(labelId: string, checked: boolean) {
    const off = declinedFor(chosenId);
    if (checked) off.delete(labelId);
    else off.add(labelId);
    kept = keptSuggestions();
    sync();
  }
  function choose(id: string) {
    chosenId = id;
    const next = options.find((item) => item.id === id);
    const nextRevision = next ? revisionOf(data, next.id, next.current_revision) : undefined;
    mode = nextRevision?.content.execution_mode || "";
    kept = id ? keptSuggestions() : [];
    sync();
    apply();
  }
  // A product change swaps the supplement: refresh suggestions for the new product.
  let lastProduct: string | null = null;
  $effect(() => {
    const previous = lastProduct;
    lastProduct = productId;
    if (previous === null || previous === productId) return;
    if (!revision) return;
    kept = keptSuggestions();
    sync();
    apply();
  });
  // A retired or revised template disappears from (or changes in) the options:
  // never keep submitting a stale selection.
  $effect(() => {
    if (chosenId && (!head || (selection && selection.revision !== head.current_revision))) {
      chosenId = "";
      sync();
    }
  });
</script>

<div class="template-picker">
  <label class="field"
    >Intake template<select
      aria-label="Intake template"
      value={chosenId}
      onchange={(event) => choose(event.currentTarget.value)}
      ><option value="">No template · free-form capture</option>{#each options as item}<option
          value={item.id}>{item.name} · {shapeLabel(item.shape)} · v{item.current_revision}</option
        >{/each}</select
    ><small
      >{options.length
        ? "Prompts shape the brief; nothing becomes Ready or verified because of a template."
        : "No active templates for this kind of work. The owner manages them under Intake templates."}</small
    ></label
  >
  {#if head && revision}
    <div class="template-meta">
      <span class="template-chip">{shapeLabel(revision.shape)} · revision {revision.revision}</span>
      {#if supplement}<span class="template-chip supplement">+ product supplement</span>{/if}
      {#if revision.description}<small>{revision.description}</small>{/if}
    </div>
    <div class="form-grid">
      <label class="field"
        >Execution mode<select
          aria-label="Execution mode"
          bind:value={mode}
          onchange={() => {
            sync();
            apply();
          }}
          ><option value=""
            >{revision.content.execution_mode
              ? "Not set · clear the suggestion"
              : "Not set (suggested)"}</option
          >{#each MODES as item}<option value={item.value}
              >{item.label}{item.value === revision.content.execution_mode
                ? " (suggested)"
                : ""}</option
            >{/each}</select
        ><small>Suggested: {modeLabel(revision.content.execution_mode)}</small></label
      >
      {#if labels.length}<fieldset class="field template-labels">
          <legend>Suggested labels</legend>
          {#each labels as label}<label class="checkbox"
              ><input
                type="checkbox"
                checked={kept.includes(label.id)}
                onchange={(event) => toggle(label.id, event.currentTarget.checked)}
              />{label.name}</label
            >{/each}
        </fieldset>{/if}
    </div>
  {/if}
</div>
