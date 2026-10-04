<script lang="ts">
  // Segmented workflow progress: one bar per scope, split by workflow state and
  // ordered from furthest along (done) to least (backlog). Canceled and legacy work
  // are excluded from the bar, as they are from the completion percentage.
  import StatusIcon from "./StatusIcon.svelte";
  import {
    WORKFLOW_STATES,
    percentLabel,
    progressSegments,
    progressSummary,
    type WorkflowBreakdown,
    type WorkflowState,
  } from "./workflow";

  let {
    data,
    label = "Progress",
    size = "sm",
    legend = false,
    selected = null,
    onselect,
  }: {
    data: WorkflowBreakdown;
    label?: string;
    size?: "xs" | "sm" | "md";
    legend?: boolean;
    /** Highlights this state's legend entry (an active filter). */
    selected?: WorkflowState | null;
    /** Makes legend entries buttons that choose a state. */
    onselect?: (state: WorkflowState) => void;
  } = $props();

  const EXCLUDED: WorkflowState[] = ["legacy_completed", "canceled"];
  let segments = $derived(progressSegments(data));
  let hover = $state<{ state: WorkflowState; x: number } | null>(null);
  let hovered = $derived(hover ? segments.find((s) => s.state === hover?.state) : undefined);

  function track(event: PointerEvent) {
    const el = (event.target as HTMLElement).closest<HTMLElement>("[data-state]");
    const state = el?.dataset.state as WorkflowState | undefined;
    hover = el && state ? { state, x: el.offsetLeft + el.offsetWidth / 2 } : null;
  }
  function tip(state: WorkflowState, count: number) {
    return `${WORKFLOW_STATES[state].name} · ${count} of ${data.eligible} (${percentLabel(count, data.eligible)})`;
  }
</script>

{#snippet entry(state: WorkflowState, count: number, note: string)}
  <StatusIcon {state} size={11} title={false} />
  <span>{WORKFLOW_STATES[state].name}</span>
  <b>{count}</b>
  <small>{note}</small>
{/snippet}
{#snippet item(state: WorkflowState, count: number, note: string)}
  {#if onselect}<button
      class="wf-legend-item"
      aria-pressed={selected === state}
      title={selected === state ? "Show all statuses" : `Show only ${WORKFLOW_STATES[state].name}`}
      onclick={() => onselect(state)}>{@render entry(state, count, note)}</button
    >{:else}{@render entry(state, count, note)}{/if}
{/snippet}

<div class="wf-progress {size}">
  <div class="wf-bar-wrap">
  <div
    class="wf-bar"
    role="img"
    aria-label={`${label}: ${progressSummary(data)}`}
    onpointermove={track}
    onpointerleave={() => (hover = null)}
  >
    {#each segments as segment (segment.state)}
      <span
        class="wf-seg {segment.state}"
        style:flex-grow={segment.count}
        data-state={segment.state}
      ></span>
    {/each}
  </div>
  {#if hovered}<span class="wf-tooltip" style:left={`${hover?.x ?? 0}px`} aria-hidden="true"
      ><StatusIcon state={hovered.state} size={11} title={false} />{tip(hovered.state, hovered.count)}</span
    >{/if}
  </div>
  {#if legend}
    <ul class="wf-legend" aria-label={`${label} by state`}>
      {#each segments as segment (segment.state)}
        <li class:selected={selected === segment.state}>
          {@render item(segment.state, segment.count, percentLabel(segment.count, data.eligible))}
        </li>
      {/each}
      {#each EXCLUDED as state (state)}
        {#if data.counts[state]}<li class="excluded" class:selected={selected === state}>
            {@render item(state, data.counts[state], "not counted")}
          </li>{/if}
      {/each}
      {#if !data.total}<li class="excluded"><span>No issues yet</span></li>{/if}
    </ul>
  {/if}
</div>
