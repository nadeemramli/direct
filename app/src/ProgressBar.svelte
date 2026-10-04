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
  }: {
    data: WorkflowBreakdown;
    label?: string;
    size?: "xs" | "sm" | "md";
    legend?: boolean;
  } = $props();

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
        <li>
          <StatusIcon state={segment.state} size={11} title={false} />
          <span>{WORKFLOW_STATES[segment.state].name}</span>
          <b>{segment.count}</b>
          <small>{percentLabel(segment.count, data.eligible)}</small>
        </li>
      {/each}
      {#if data.counts.legacy_completed}<li class="excluded">
          <StatusIcon state="legacy_completed" size={11} title={false} />
          <span>Legacy done</span><b>{data.counts.legacy_completed}</b><small>not counted</small>
        </li>{/if}
      {#if data.counts.canceled}<li class="excluded">
          <StatusIcon state="canceled" size={11} title={false} />
          <span>Canceled</span><b>{data.counts.canceled}</b><small>not counted</small>
        </li>{/if}
      {#if !data.total}<li class="excluded"><span>No issues yet</span></li>{/if}
    </ul>
  {/if}
</div>
