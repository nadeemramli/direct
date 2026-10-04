<script lang="ts">
  // Workflow state icon. Each state has its own shape, not only its own color:
  // dashed ring (backlog), ring (ready), ring with a dot (needs fix), half and
  // three-quarter pies (doing, verify), filled check (done), filled cross (canceled).
  import { WORKFLOW_STATES, type WorkflowState } from "./workflow";

  let {
    state,
    size = 14,
    title = true,
  }: { state: WorkflowState; size?: number; title?: boolean } = $props();
</script>

<svg
  class="wf-icon {state}"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  role={title ? "img" : undefined}
  aria-label={title ? WORKFLOW_STATES[state].name : undefined}
  aria-hidden={title ? undefined : "true"}
  focusable="false"
>
  {#if state === "backlog"}
    <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.6" stroke-dasharray="2.1 2.1" />
  {:else if state === "ready"}
    <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.6" />
  {:else if state === "needs_fix"}
    <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.6" />
    <circle cx="8" cy="8" r="2" fill="currentColor" />
  {:else if state === "doing"}
    <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.6" />
    <path d="M8 8 L8 4.25 A3.75 3.75 0 0 1 8 11.75 Z" fill="currentColor" />
  {:else if state === "verify"}
    <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="1.6" />
    <path d="M8 8 L8 4.25 A3.75 3.75 0 1 1 4.25 8 Z" fill="currentColor" />
  {:else if state === "done" || state === "legacy_completed"}
    <circle cx="8" cy="8" r="7" fill="currentColor" />
    <path d="M5 8.2 L7.1 10.2 L11 6" fill="none" class="wf-icon-ink" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" />
  {:else}
    <circle cx="8" cy="8" r="7" fill="currentColor" />
    <path d="M5.6 5.6 L10.4 10.4 M10.4 5.6 L5.6 10.4" fill="none" class="wf-icon-ink" stroke-width="1.7" stroke-linecap="round" />
  {/if}
</svg>
