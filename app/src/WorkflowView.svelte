<script lang="ts">
  // Workflow overview: every workflow state grouped by category with its issue
  // count, plus completion progress for the scope and each project. Counts are
  // derived from the same parent issues the list shows; selecting a state or a
  // project opens the filtered issue list.
  import type { Issue, Product, Project } from "./api";
  import ProgressBar from "./ProgressBar.svelte";
  import StatusIcon from "./StatusIcon.svelte";
  import {
    DEFAULT_STATE,
    WORKFLOW_CATEGORIES,
    WORKFLOW_STATES,
    breakdown,
    percentLabel,
    type WorkflowState,
  } from "./workflow";

  let {
    issues,
    products,
    projects,
    reviewReady,
    product = $bindable("all"),
    onstate,
    onproject,
  }: {
    issues: Issue[];
    products: Product[];
    projects: Project[];
    reviewReady: string[];
    product?: string;
    onstate: (state: WorkflowState) => void;
    onproject: (projectId: string) => void;
  } = $props();

  let scoped = $derived(issues.filter((issue) => product === "all" || issue.product_id === product));
  let overall = $derived(breakdown(scoped));
  let verifyReady = $derived(
    scoped.filter((issue) => issue.status === "verify" && reviewReady.includes(issue.current_run || "")).length,
  );
  let projectRows = $derived(
    projects
      .filter((project) => product === "all" || project.product_id === product)
      .map((project) => ({
        project,
        progress: breakdown(scoped.filter((issue) => issue.project_id === project.id)),
      }))
      .filter((row) => row.progress.total > 0)
      .sort(
        (a, b) =>
          rank(a.project.status) - rank(b.project.status) ||
          (a.project.sort_order || 0) - (b.project.sort_order || 0) ||
          a.project.name.localeCompare(b.project.name),
      ),
  );
  let inbox = $derived(breakdown(scoped.filter((issue) => !issue.project_id)));

  function rank(status: string | undefined) {
    return { active: 0, planned: 1, paused: 2, completed: 3, canceled: 4 }[status || "active"] ?? 1;
  }
  function countLabel(count: number) {
    return `${count} issue${count === 1 ? "" : "s"}`;
  }
  function detail(state: WorkflowState) {
    if (state === "verify" && overall.counts.verify)
      return `${verifyReady} ready for your review · ${overall.counts.verify - verifyReady} awaiting agent E2E`;
    return WORKFLOW_STATES[state].description;
  }
  function productName(id: string) {
    return products.find((p) => p.id === id)?.name || "";
  }
</script>

<section class="list-panel workflow-page" aria-label="Workflow">
  <div class="page-heading">
    <div class="eyebrow">STATES · PROGRESS · FLOW</div>
    <div class="heading-row">
      <h1>Workflow</h1>
      <span class="count-pill">{scoped.length}</span>
    </div>
    <p>Where every piece of work sits, and how much of it is verified done.</p>
  </div>
  <div class="project-toolbar">
    <label class="project-filter"
      >Product
      <select aria-label="Workflow product" bind:value={product}>
        <option value="all">All products</option>
        {#each products as p}<option value={p.id}>{p.name}</option>{/each}
      </select>
    </label>
  </div>

  <div class="wf-overview">
    <div class="wf-overview-head">
      <div>
        <span class="wf-overview-label">Verified done</span>
        <b>{overall.percent}%</b>
      </div>
      <span class="wf-overview-note"
        >{overall.counts.done} of {overall.eligible} counted issues{overall.counts.canceled ||
        overall.counts.legacy_completed
          ? ` · ${overall.counts.canceled + overall.counts.legacy_completed} canceled or legacy not counted`
          : ""}</span
      >
    </div>
    <ProgressBar data={overall} label="Workflow progress" size="md" legend />
  </div>

  <div class="wf-board" aria-label="Workflow states">
    {#each WORKFLOW_CATEGORIES as category (category.id)}
      <div class="wf-category">{category.name}</div>
      {#each category.states as state (state)}
        {@const count = overall.counts[state]}
        <button class="wf-state-row" onclick={() => onstate(state)} aria-label={`${WORKFLOW_STATES[state].name}: ${countLabel(count)}. Open issues.`}>
          <span class="wf-tile {state}"><StatusIcon {state} size={16} title={false} /></span>
          <span class="wf-state-copy">
            <span class="wf-state-name"
              >{WORKFLOW_STATES[state].name}{#if state === DEFAULT_STATE}{" "}<span class="wf-default">· Default</span>{/if}</span
            >
            <span class="wf-state-sub"
              >{#if count}{countLabel(count)}<span class="dot-separator">·</span>{/if}{detail(state)}</span
            >
          </span>
          {#if count && state !== "canceled" && state !== "legacy_completed"}<span class="wf-share"
              >{percentLabel(count, overall.eligible)}</span
            >{/if}
          <span class="wf-open" aria-hidden="true">›</span>
        </button>
      {/each}
    {/each}
  </div>

  <div class="section-label wf-section">PROJECT PROGRESS <span>{projectRows.length}</span></div>
  <div class="wf-projects">
    {#each projectRows as row (row.project.id)}
      <button class="wf-project-row" onclick={() => onproject(row.project.id)}>
        <span class="wf-project-name"
          ><b>{row.project.name}</b><small
            >{row.project.status || "active"}{product === "all" ? ` · ${productName(row.project.product_id)}` : ""} · {row.progress.counts.done}/{row.progress.eligible} done{row.progress.counts.verify
              ? ` · ${row.progress.counts.verify} verify`
              : ""}{row.progress.counts.needs_fix ? ` · ${row.progress.counts.needs_fix} needs fix` : ""}</small
          ></span
        >
        <ProgressBar data={row.progress} label={row.project.name} size="sm" />
        <span class="wf-project-percent">{row.progress.percent}%</span>
      </button>
    {:else}<p class="wf-empty">No project has issues in this scope yet.</p>{/each}
    {#if inbox.total}<button class="wf-project-row" onclick={() => onproject("none")}>
        <span class="wf-project-name"
          ><b>No project · Inbox</b><small>{inbox.counts.done}/{inbox.eligible} done · explicitly ungrouped work</small></span
        >
        <ProgressBar data={inbox} label="No project" size="sm" />
        <span class="wf-project-percent">{inbox.percent}%</span>
      </button>{/if}
  </div>
</section>
