<script lang="ts">
  import { onMount } from "svelte";
  import { api, connect } from "./api";
  import type {
    Snapshot,
    Issue,
    Context,
    Verification,
    Status,
    Outcome,
    Project,
  } from "./api";
  let data = $state<Snapshot>({
    workspace_id: "",
    products: [],
    projects: [],
    issues: [],
    cursor: 0,
  });
  let selected = $state("");
  let context = $state<Context | null>(null);
  let view = $state("all");
  let product = $state("all");
  let projectFilter = $state("all");
  let selectedProject = $derived(
    data.projects.find((p) => p.id === projectFilter),
  );
  let availableProjects = $derived(
    data.projects
      .filter((p) => product === "all" || p.product_id === product)
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  $effect(() => {
    if (
      selectedProject &&
      product !== "all" &&
      selectedProject.product_id !== product
    )
      projectFilter = "all";
  });
  let search = $state("");
  let connected = $state(false);
  let error = $state("");
  let busy = $state(false);
  let tab = $state("brief");
  let modal = $state<
    "issue" | "product" | "project" | "edit" | "submit" | null
  >(null);
  let projectDraft = $state({
    id: "",
    product: "DIR",
    name: "",
    description: "",
    version: 0,
  });
  let draft = $state({
    title: "",
    body: "",
    acceptance: "",
    owner: "",
    priority: "medium",
    product: "DIR",
    key: "",
    version: 0,
  });
  let productDraft = $state({
    key: "",
    name: "",
    repo_windows: "",
    repo_wsl: "",
    vault_windows: "",
    vault_wsl: "",
  });
  let handoff = $state({
    build_ref: "",
    delivery_ref: "",
    summary: "",
    checks: "",
    limitations: "",
    preconditions: "",
    steps: [{ instruction: "", expected: "" }],
  });
  let results = $state<{ outcome: Outcome; note: string }[]>([]);
  let reviewNote = $state("");
  let comment = $state("");
  let reopenReason = $state("");
  let clock = $state(Date.now() / 1000);
  let activeRunId = "";
  function showDialog(element: HTMLDialogElement) {
    element.showModal();
  }
  const labels: Record<Status, string> = {
    backlog: "Backlog",
    ready: "Ready",
    doing: "Doing",
    verify: "Verify",
    done: "Done",
    canceled: "Canceled",
  };
  const glyphs: Record<Status, string> = {
    backlog: "◌",
    ready: "○",
    doing: "◐",
    verify: "◈",
    done: "✓",
    canceled: "⊘",
  };
  let parents = $derived(data.issues.filter((i) => !i.parent));
  let current = $derived(data.issues.find((i) => i.key === selected));
  let activeRun = $derived(
    context?.verifications.find((v) => v.id === current?.current_run),
  );
  function needsMe(i: Issue) {
    return (
      i.status === "verify" ||
      i.needs_fix ||
      (i.status === "doing" && (!i.claim || i.claim.expires_at <= clock))
    );
  }
  let attention = $derived(parents.filter(needsMe).length);
  let visible = $derived(
    parents
      .filter(
        (i) =>
          (product === "all" || i.product_id === product) &&
          (projectFilter === "all" ||
            (projectFilter === "none"
              ? !i.project_id
              : i.project_id === projectFilter)) &&
          (view === "all" ||
            (view === "needs"
              ? needsMe(i)
              : view === "active"
                ? ["ready", "doing"].includes(i.status)
                : i.status === view)) &&
          `${i.key} ${i.title} ${i.body}`
            .toLowerCase()
            .includes(search.toLowerCase()),
      )
      .sort(
        (a, b) => b.updated_at - a.updated_at || a.key.localeCompare(b.key),
      ),
  );
  let title = $derived(
    product !== "all"
      ? data.products.find((p) => p.id === product)?.name || "Product"
      : view === "needs"
        ? "Needs me"
        : view === "active"
          ? "Ready & doing"
          : view === "backlog"
            ? "Inbox"
            : view === "done"
              ? "Completed"
              : "All work",
  );
  function date(at: number) {
    return new Date(at * 1000).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  async function loadContext(key = selected) {
    if (!key) return;
    const result = await api<Context>({ op: "context", key });
    if (selected !== key) return;
    context = result;
    const run = result.verifications.find(
      (v) => v.id === result.issue.current_run,
    );
    if ((run?.id || "") !== activeRunId) {
      activeRunId = run?.id || "";
      results =
        run?.steps.map(() => ({ outcome: "pending" as Outcome, note: "" })) ||
        [];
      reviewNote = "";
    }
  }
  async function refresh() {
    const snapshot = await api<Snapshot>({ op: "snapshot" });
    data = snapshot;
    connected = true;
    if (selected) await loadContext();
  }
  async function choose(key: string) {
    selected = key;
    context = null;
    tab = "brief";
    error = "";
    try {
      await loadContext(key);
    } catch (e) {
      error = String(e);
    }
  }
  async function act(command: Record<string, unknown>) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      const result = await api<Issue>(command, true);
      await refresh();
      return result;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
      return undefined;
    } finally {
      busy = false;
    }
  }
  function edit(i: Issue) {
    draft = {
      title: i.title,
      body: i.body,
      acceptance: i.acceptance,
      owner: i.owner,
      priority: i.priority,
      key: i.key,
      version: i.version,
      product: "",
    };
    modal = "edit";
  }
  function newIssue() {
    // Capture starts ungrouped; assignment is an explicit action on its detail.
    projectFilter = "all";
    draft = {
      title: "",
      body: "",
      acceptance: "",
      owner: "",
      priority: "medium",
      product:
        data.products.find((p) => p.id === product)?.key ||
        data.products[0]?.key ||
        "DIR",
      key: "",
      version: 0,
    };
    modal = "issue";
  }
  async function saveDraft(event: SubmitEvent) {
    event.preventDefault();
    const result =
      modal === "issue"
        ? await act({
            op: "create_issue",
            product: draft.product,
            title: draft.title,
            body: draft.body,
          })
        : await act({
            op: "update_issue",
            key: draft.key,
            expected_version: draft.version,
            title: draft.title,
            body: draft.body,
            acceptance: draft.acceptance,
            owner: draft.owner,
            priority: draft.priority,
          });
    if (result) {
      modal = null;
      await choose(result.key);
    }
  }
  async function createProduct(event: SubmitEvent) {
    event.preventDefault();
    if (await act({ op: "create_product", ...productDraft })) {
      modal = null;
      productDraft = {
        key: "",
        name: "",
        repo_windows: "",
        repo_wsl: "",
        vault_windows: "",
        vault_wsl: "",
      };
    }
  }
  function editProject(p?: Project) {
    projectDraft = p
      ? {
          id: p.id,
          product: data.products.find((product) => product.id === p.product_id)!
            .key,
          name: p.name,
          description: p.description,
          version: p.version,
        }
      : {
          id: "",
          product:
            data.products.find((p) => p.id === product)?.key ||
            data.products[0]?.key ||
            "DIR",
          name: "",
          description: "",
          version: 0,
        };
    modal = "project";
  }
  async function saveProject(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      const saved = await api<Project>(
        projectDraft.id
          ? {
              op: "update_project",
              id: projectDraft.id,
              expected_version: projectDraft.version,
              name: projectDraft.name,
              description: projectDraft.description,
            }
          : {
              op: "create_project",
              product: projectDraft.product,
              name: projectDraft.name,
              description: projectDraft.description,
            },
        true,
      );
      await refresh();
      product = saved.product_id;
      projectFilter = saved.id;
      modal = null;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      busy = false;
    }
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!current) return;
    if (
      await act({
        op: "submit",
        key: current.key,
        expected_version: draft.version,
        ...handoff,
      })
    ) {
      modal = null;
      tab = "verify";
    }
  }
  async function review(outcome: Outcome) {
    if (current && activeRun)
      await act({
        op: "review",
        key: current.key,
        expected_version: current.version,
        run_id: activeRun.id,
        outcome,
        results,
        note: reviewNote,
      });
  }
  async function exportData() {
    try {
      const archive = await api({ op: "export" });
      const url = URL.createObjectURL(
        new Blob([JSON.stringify(archive, null, 2)], {
          type: "application/json",
        }),
      );
      const a = document.createElement("a");
      a.href = url;
      a.download = `direct-backup-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (e) {
      error = String(e);
    }
  }
  onMount(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    function shortcut(e: KeyboardEvent) {
      if (
        e.key.toLowerCase() === "n" &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        !modal &&
        connected &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement) &&
        !(e.target instanceof HTMLSelectElement)
      ) {
        e.preventDefault();
        newIssue();
      }
    }
    window.addEventListener("keydown", shortcut);
    async function poll() {
      if (stopped) return;
      clock = Date.now() / 1000;
      try {
        const changes = await api<{ cursor: number }>({
          op: "changes",
          after: data.cursor,
        });
        if (changes.cursor !== data.cursor) await refresh();
        connected = true;
      } catch {
        connected = false;
      }
      if (!stopped) timer = setTimeout(poll, 750);
    }
    connect()
      .then(refresh)
      .then(() => {
        if (!stopped) timer = setTimeout(poll, 750);
      })
      .catch((e) => {
        error = String(e).replace(/^Error: /, "");
      });
    return () => {
      stopped = true;
      clearTimeout(timer);
      window.removeEventListener("keydown", shortcut);
    };
  });
</script>

<svelte:head><title>Direct · {title}</title></svelte:head>

<div class="shell">
  <aside class="sidebar">
    <a class="brand" href="/" onclick={(e) => e.preventDefault()}
      ><span class="brand-mark">↗</span> Direct
      <span class="version">LOCAL</span></a
    >
    <div class="workspace-label">
      <span class="avatar">Y</span>
      <div>Your workspace<small>Human + agents</small></div>
      <span class="tiny">⌄</span>
    </div>
    <button class="compose" onclick={newIssue} disabled={!connected}
      ><span>＋</span> New issue <kbd>N</kbd></button
    >
    <div class="nav-label">WORKSPACE</div>
    <nav aria-label="Workspace">
      <button
        class:active={view === "all" && product === "all"}
        onclick={() => {
          view = "all";
          product = "all";
        }}><span>▤</span> All work <small>{parents.length}</small></button
      >
      <button
        class:active={view === "needs" && product === "all"}
        onclick={() => {
          view = "needs";
          product = "all";
        }}
        ><span>◈</span> Needs me
        <small class:highlight={attention > 0}>{attention}</small></button
      >
      <button
        class:active={view === "backlog" && product === "all"}
        onclick={() => {
          view = "backlog";
          product = "all";
        }}
        ><span>▧</span> Inbox
        <small>{parents.filter((i) => i.status === "backlog").length}</small
        ></button
      >
      <button
        class:active={view === "active" && product === "all"}
        onclick={() => {
          view = "active";
          product = "all";
        }}><span>◐</span> Ready & doing</button
      >
      <button
        class:active={view === "done" && product === "all"}
        onclick={() => {
          view = "done";
          product = "all";
        }}><span>✓</span> Completed</button
      >
    </nav>
    <div class="nav-label products-label">
      PRODUCTS <button
        class="icon-button"
        aria-label="Add product"
        onclick={() => (modal = "product")}
        disabled={!connected}>＋</button
      >
    </div>
    <nav aria-label="Products">
      {#each data.products as p}<button
          class:active={product === p.id}
          onclick={() => {
            product = p.id;
            view = "all";
          }}
          ><span class="product-icon">{p.key.slice(0, 1)}</span>{p.name}<small
            >{parents.filter((i) => i.product_id === p.id).length}</small
          ></button
        >{/each}
    </nav>
    <div class="sidebar-bottom">
      <div class="local-note">
        <span class:online={connected} class="connection-dot"></span>
        <div>
          {connected ? "Connected locally" : "Service disconnected"}<small
            >{connected
              ? "Your work stays on this device"
              : "Reconnect to continue working"}</small
          >
        </div>
      </div>
      <button class="backup-button" onclick={exportData} disabled={!connected}
        >↧ Export workspace</button
      >
      <div class="foundation">Direct · Foundation preview</div>
    </div>
  </aside>

  <main>
    <header class="topbar">
      <div>
        <span class="breadcrumb">Workspace</span><span class="slash">/</span
        >{title}
      </div>
      <div class="top-right">
        <span class="privacy">◉ Local workspace</span><span class="avatar small"
          >Y</span
        >
      </div>
    </header>
    {#if error}<div class="error" role="alert">
        <span>{error}</span><button
          class="icon-button"
          aria-label="Dismiss error"
          onclick={() => (error = "")}>×</button
        >
      </div>{/if}
    <div class="work-area">
      <section class="list-panel">
        <div class="page-heading">
          <div class="eyebrow">A LITTLE LESS COORDINATION.</div>
          <div class="heading-row">
            <h1>{title}</h1>
            <span class="count-pill">{visible.length}</span>
          </div>
          <p>
            {view === "needs"
              ? "Your judgment is the next step."
              : "Clear intent. Focused work. Verified delivery."}
          </p>
        </div>
        <div class="toolbar">
          <label class="search"
            ><span>⌕</span><input
              aria-label="Search issues"
              bind:value={search}
              placeholder="Search issues…"
            /><kbd>⌕</kbd></label
          ><button class="secondary" onclick={newIssue} disabled={!connected}
            >＋ Issue</button
          >
        </div>
        <div class="project-toolbar">
          <label class="project-filter"
            >Project
            <select aria-label="Filter by project" bind:value={projectFilter}>
              <option value="all">All projects</option>
              <option value="none">No project</option>
              {#each availableProjects as p}<option value={p.id}
                  >{p.name}{product === "all"
                    ? ` · ${data.products.find((product) => product.id === p.product_id)?.name}`
                    : ""}</option
                >{/each}
            </select>
          </label>
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editProject()}>＋ New project</button
          >
          {#if selectedProject}<button
              class="text-button"
              disabled={!connected || busy}
              onclick={() => editProject(selectedProject)}>Edit project</button
            >{/if}
        </div>
        {#if selectedProject?.description}<p class="project-description">
            {selectedProject.description}
          </p>{/if}
        <div class="list-label"><span>ISSUE</span><span>STATUS</span></div>
        <div class="issue-list">
          {#each visible as i}<button
              class="issue-row"
              class:selected={selected === i.key}
              onclick={() => choose(i.key)}
              ><span class="state-icon {i.status}">{glyphs[i.status]}</span>
              <div class="row-content">
                <div class="issue-title">{i.title}</div>
                <div class="issue-meta">
                  <span>{i.key}</span><span class="dot-separator">·</span><span
                    >{data.products.find((p) => p.id === i.product_id)
                      ?.name}</span
                  >{#if i.needs_fix}<span class="fix-badge">Needs fix</span
                    >{/if}{#if i.claim}<span class="claim-meta"
                      >↗ {i.claim.actor}</span
                    >{/if}
                  {#if i.project_id}<span class="project-tag"
                      >{data.projects.find((p) => p.id === i.project_id)
                        ?.name}</span
                    >{/if}
                </div>
              </div>
              <span class="status-badge {i.status}">{labels[i.status]}</span
              ></button
            >
          {:else}<div class="empty">
              <div class="empty-symbol">
                {search ? "⌕" : view === "needs" ? "✓" : "↗"}
              </div>
              <h2>
                {search
                  ? "No matching issues"
                  : view === "needs"
                    ? "Nothing waiting on you"
                    : "Room for your next idea"}
              </h2>
              <p>
                {search
                  ? "Try another title or issue ID."
                  : view === "needs"
                    ? "Submitted work and decisions will appear here."
                    : "Capture an issue, shape the outcome, and hand it off."}
              </p>
              {#if !search && view !== "needs"}<button
                  class="primary"
                  onclick={newIssue}
                  disabled={!connected}>Create your first issue</button
                >{/if}
            </div>{/each}
        </div>
        <div class="list-footer">
          <span
            >{visible.length} work item{visible.length === 1 ? "" : "s"}</span
          ><span
            >Changes appear automatically <span
              class="connection-dot"
              class:online={connected}
            ></span></span
          >
        </div>
      </section>

      <aside class="detail-panel" aria-label="Issue detail">
        {#if current}
          <div class="detail-top">
            <span>{current.key}</span>
            <div>
              <span class="tiny">v{current.version}</span><button
                class="icon-button"
                aria-label="Close issue"
                onclick={() => {
                  selected = "";
                  context = null;
                }}>×</button
              >
            </div>
          </div>
          <div class="detail-heading">
            <span class="status-badge {current.status}"
              >{glyphs[current.status]} {labels[current.status]}</span
            >
            <h2>{current.title}</h2>
            <div class="properties">
              <span>Owner <b>{current.owner || "Unassigned"}</b></span><span
                >Priority <b>{current.priority}</b></span
              >
            </div>
            <label class="project-assignment"
              >Project
              <select
                aria-label="Issue project"
                value={current.project_id || ""}
                disabled={busy || !connected}
                onchange={async (event) => {
                  const control = event.currentTarget;
                  const result = await act({
                    op: "set_issue_project",
                    key: current.key,
                    expected_version: current.version,
                    project_id: control.value || null,
                  });
                  if (!result) control.value = current?.project_id || "";
                }}
              >
                <option value="">No project</option>
                {#each data.projects.filter((p) => p.product_id === current.product_id) as p}<option
                    value={p.id}>{p.name}</option
                  >{/each}
              </select>
            </label>
          </div>
          <div class="tabs" role="tablist" aria-label="Issue sections">
            {#each [["brief", "Brief"], ["verify", "Verification"], ["activity", "Activity"]] as [id, label]}<button
                role="tab"
                aria-selected={tab === id}
                class:active={tab === id}
                onclick={() => (tab = id)}
                >{label}{#if id === "verify" && current.status === "verify"}<span
                    class="tab-dot"
                  ></span>{/if}</button
              >{/each}
          </div>
          <div class="detail-body">
            {#if tab === "brief"}
              <div class="section-label">
                PROBLEM & OUTCOME {#if !["verify", "done", "canceled"].includes(current.status)}<button
                    class="text-button"
                    onclick={() => edit(current)}>Edit brief</button
                  >{/if}
              </div>
              <p class="prose" class:muted={!current.body}>
                {current.body ||
                  "Start with the problem this work should solve."}
              </p>
              <div class="section-label">ACCEPTANCE CRITERIA</div>
              <p class="prose" class:muted={!current.acceptance}>
                {current.acceptance ||
                  "Describe what a good result looks like before making this Ready."}
              </p>
              {#if current.claim}<div class="info-card">
                  <span class="card-symbol">↗</span>
                  <div>
                    <b>{current.claim.actor}</b>
                    <p>
                      {current.claim.expires_at > clock
                        ? "Claim active until"
                        : "Claim expired"}
                      {date(current.claim.expires_at)}
                    </p>
                  </div>
                </div>{/if}
              {#if current.needs_fix}<div class="info-card warning">
                  <span>↺</span>
                  <div>
                    <b>Another pass is needed</b>
                    <p>Review the previous test evidence before continuing.</p>
                  </div>
                </div>{/if}
              <div class="actions">
                {#if current.status === "backlog"}<button
                    class="primary"
                    disabled={busy}
                    onclick={() =>
                      act({
                        op: "ready",
                        key: current.key,
                        expected_version: current.version,
                      })}>Make ready <span>→</span></button
                  >
                  <p class="hint">
                    Requires a brief, acceptance criteria, and an owner.
                  </p>{/if}
                {#if ["ready", "doing"].includes(current.status) && (!current.claim || current.claim.expires_at <= clock)}<button
                    class="primary"
                    disabled={busy}
                    onclick={() =>
                      act({
                        op: "claim",
                        key: current.key,
                        expected_version: current.version,
                        lease_seconds: 3600,
                      })}>Claim this work <span>↗</span></button
                  >{/if}
                {#if current.status === "doing" && current.claim?.actor === "owner" && current.claim.expires_at > clock}<button
                    class="primary"
                    disabled={busy}
                    onclick={() => {
                      draft.version = current.version;
                      handoff = {
                        build_ref: "",
                        delivery_ref: "",
                        summary: "",
                        checks: "",
                        limitations: "",
                        preconditions: "",
                        steps: [{ instruction: "", expected: "" }],
                      };
                      modal = "submit";
                    }}>Submit for verification <span>→</span></button
                  ><button
                    class="secondary"
                    disabled={busy}
                    onclick={() =>
                      act({
                        op: "release",
                        key: current.key,
                        expected_version: current.version,
                      })}>Release claim</button
                  >{/if}
                {#if current.status === "verify"}<button
                    class="primary"
                    onclick={() => (tab = "verify")}
                    >Review the result <span>→</span></button
                  >{/if}
              </div>
              {#if context?.product.repo_windows || context?.product.vault_windows}<div
                  class="section-label"
                >
                  PROJECT CONTEXT
                </div>
                {#if context.product.repo_windows}<div class="context-path">
                    Repository<code>{context.product.repo_windows}</code>
                  </div>{/if}{#if context.product.vault_windows}<div
                    class="context-path"
                  >
                    Product knowledge<code>{context.product.vault_windows}</code
                    >
                  </div>{/if}{/if}
            {:else if tab === "verify"}
              {#if activeRun}
                <div class="verification-banner {activeRun.outcome}">
                  <span>◈</span>
                  <div>
                    <b
                      >{activeRun.outcome === "pending"
                        ? "Ready for your review"
                        : `Verification ${activeRun.outcome}`}</b
                    ><small
                      >{activeRun.submitted_by} · {date(
                        activeRun.submitted_at,
                      )}</small
                    >
                  </div>
                </div>
                <div class="section-label">WHAT CHANGED</div>
                <p class="prose">{activeRun.summary}</p>
                <dl class="evidence">
                  <dt>Tested build</dt>
                  <dd>{activeRun.build_ref}</dd>
                  <dt>Delivery</dt>
                  <dd>{activeRun.delivery_ref}</dd>
                  <dt>Checks</dt>
                  <dd>{activeRun.checks}</dd>
                  {#if activeRun.limitations}<dt>Limitations</dt>
                    <dd>{activeRun.limitations}</dd>{/if}
                </dl>
                {#if activeRun.preconditions}<div class="section-label">
                    BEFORE YOU TEST
                  </div>
                  <p class="prose">{activeRun.preconditions}</p>{/if}
                <div class="section-label">
                  VERIFICATION STEPS <span>{activeRun.steps.length}</span>
                </div>
                {#each activeRun.steps as step, index}<div class="test-step">
                    <div class="step-number">{index + 1}</div>
                    <div>
                      <b>{step.instruction}</b>
                      <p>{step.expected}</p>
                      {#if activeRun.outcome === "pending" && results[index]}<select
                          aria-label={`Result for step ${index + 1}`}
                          bind:value={results[index].outcome}
                          ><option value="pending">Not tested</option><option
                            value="passed">Passed</option
                          ><option value="failed">Failed</option></select
                        ><input
                          aria-label={`Evidence for step ${index + 1}`}
                          bind:value={results[index].note}
                          placeholder="Evidence or observation (optional)"
                        />{:else}<span class="result-label"
                          >{activeRun.results[index]?.outcome ||
                            "Not recorded"}</span
                        >{#if activeRun.results[index]?.note}<p>
                            {activeRun.results[index].note}
                          </p>{/if}{/if}
                    </div>
                  </div>{/each}
                {#if activeRun.outcome === "pending"}<label class="field"
                    >Review notes<textarea
                      rows="3"
                      bind:value={reviewNote}
                      placeholder="What worked, what failed, or why testing is canceled…"
                    ></textarea></label
                  >
                  <div class="review-actions">
                    <button
                      class="primary"
                      disabled={busy ||
                        results.some((r) => r.outcome !== "passed")}
                      onclick={() => review("passed")}>Pass & complete ✓</button
                    ><button
                      class="danger-button"
                      disabled={busy ||
                        !reviewNote.trim() ||
                        !results.some((r) => r.outcome === "failed")}
                      onclick={() => review("failed")}>Return for fixes</button
                    ><button
                      class="text-button"
                      disabled={busy || !reviewNote.trim()}
                      onclick={() => review("canceled")}>Cancel test</button
                    >
                  </div>
                  <p class="hint">
                    Every step must pass. Canceling never completes the work.
                  </p>{:else}<div class="info-card">
                    <div>
                      <b>Reviewed by {activeRun.reviewed_by}</b>
                      <p>
                        {activeRun.review_note || "All required steps passed."}
                      </p>
                    </div>
                  </div>{/if}
              {:else}<div class="empty compact">
                  <div class="empty-symbol">◈</div>
                  <h3>No submitted verification</h3>
                  <p>
                    A handoff will bring the tested build, checks, and
                    walkthrough here.
                  </p>
                </div>{/if}
              {#if context && context.verifications.length > 1}<details>
                  <summary
                    >Previous verification runs ({context.verifications.length -
                      1})</summary
                  >{#each context.verifications.filter((v) => v.id !== activeRun?.id) as run}<div
                      class="history-run"
                    >
                      <b>{run.outcome} · {run.build_ref}</b>
                      <p>{run.review_note || run.summary}</p>
                      <small>{date(run.submitted_at)}</small>
                    </div>{/each}
                </details>{/if}
            {:else}
              <div class="section-label">DISCUSSION</div>
              {#each context?.comments || [] as c}<article class="comment">
                  <b>{c.actor}</b><small>{date(c.at)}</small>
                  <p class="prose">{c.body}</p>
                </article>{/each}{#if !context?.comments.length}<p
                  class="muted"
                >
                  Keep decisions and handoff context with the work.
                </p>{/if}
              <form
                onsubmit={async (e) => {
                  e.preventDefault();
                  if (
                    await act({
                      op: "comment",
                      key: current.key,
                      expected_version: current.version,
                      body: comment,
                    })
                  )
                    comment = "";
                }}
              >
                <label class="field"
                  >Add a comment<textarea
                    bind:value={comment}
                    rows="3"
                    required
                    placeholder="Add evidence, a decision, or context…"
                  ></textarea></label
                ><button class="secondary" disabled={busy || !comment.trim()}
                  >Add comment</button
                >
              </form>
              <div class="section-label spaced">HISTORY</div>
              {#each context?.history || [] as entry}<div class="activity">
                  <span class="activity-dot"></span>
                  <div>
                    <b>{entry.kind.replaceAll("_", " ")}</b><small
                      >{entry.actor} · {date(entry.at)}</small
                    >
                  </div>
                </div>{/each}
            {/if}
            {#if ["done", "verify"].includes(current.status)}<details
                class="reopen"
              >
                <summary>Reopen work</summary>
                <p class="hint">Changed work needs a fresh verification run.</p>
                <input
                  aria-label="Reason for reopening"
                  bind:value={reopenReason}
                  placeholder="Why does this need another pass?"
                /><button
                  class="secondary"
                  disabled={busy || !reopenReason.trim()}
                  onclick={async () => {
                    if (
                      await act({
                        op: "reopen",
                        key: current.key,
                        expected_version: current.version,
                        reason: reopenReason,
                      })
                    )
                      reopenReason = "";
                  }}>Reopen with reason</button
                >
              </details>{/if}
          </div>
        {:else}<div class="detail-placeholder">
            <div class="outline-mark">↗</div>
            <h2>Make the next step clear.</h2>
            <p>
              Select an issue to see its brief,<br />handoff, and verification
              evidence.
            </p>
            <div class="mini-flow">
              <span>Intent</span><i>→</i><span>Work</span><i>→</i><span
                >Proof</span
              >
            </div>
          </div>{/if}
      </aside>
    </div>
  </main>
</div>

{#if modal}
  <div class="modal-backdrop">
    <dialog
      class="modal"
      use:showDialog
      oncancel={() => (modal = null)}
      aria-label={modal === "project"
        ? projectDraft.id
          ? "Edit project"
          : "New project"
        : modal === "product"
          ? "New product"
          : modal === "submit"
            ? "Submit for verification"
            : modal === "edit"
              ? "Edit issue"
              : "New issue"}
      tabindex="-1"
    >
      <div class="modal-header">
        <div>
          <div class="eyebrow">
            DIRECT / {modal === "submit" ? "HANDOFF" : "WORKSPACE"}
          </div>
          <h2>
            {modal === "project"
              ? projectDraft.id
                ? "Shape the project"
                : "Group work into a project"
              : modal === "product"
                ? "A space for your product"
                : modal === "submit"
                  ? "Hand it back with evidence"
                  : modal === "edit"
                    ? "Shape the work"
                    : "Capture an issue"}
          </h2>
        </div>
        <button
          class="icon-button"
          aria-label="Close dialog"
          onclick={() => (modal = null)}>×</button
        >
      </div>
      {#if error}<div class="error" role="alert">{error}</div>{/if}
      {#if modal === "project"}<form onsubmit={saveProject}>
          <label class="field"
            >Product<select
              bind:value={projectDraft.product}
              disabled={!!projectDraft.id}
            >
              {#each data.products as p}<option value={p.key}>{p.name}</option
                >{/each}
            </select></label
          >
          <label class="field"
            >Project name<input
              required
              maxlength="160"
              bind:value={projectDraft.name}
              placeholder="e.g. Direct pilot"
            /></label
          >
          <label class="field"
            >Project outcome<textarea
              rows="3"
              bind:value={projectDraft.description}
              placeholder="What should this project deliver?"></textarea></label
          >
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >{projectDraft.id ? "Save project" : "Create project"}</button
            >
          </div>
        </form>
      {:else if modal === "product"}<form onsubmit={createProduct}>
          <div class="form-grid">
            <label class="field"
              >Product name<input
                required
                bind:value={productDraft.name}
                placeholder="e.g. Pernance"
              /></label
            ><label class="field"
              >Issue prefix<input
                required
                pattern={"[A-Z]{1,8}"}
                maxlength="8"
                bind:value={productDraft.key}
                placeholder="PFN"
              /></label
            >
          </div>
          <details>
            <summary>Repository & knowledge paths (optional)</summary
            >{#each ["repo_windows", "repo_wsl", "vault_windows", "vault_wsl"] as field}<label
                class="field"
                >{field.replace("_", " ")}<input
                  bind:value={productDraft[field as keyof typeof productDraft]}
                /></label
              >{/each}
          </details>
          <div class="modal-footer">
            <button class="primary" disabled={busy}>Create product</button>
          </div>
        </form>
      {:else if modal === "submit"}<form onsubmit={submit}>
          <label class="field"
            >What changed<textarea
              rows="2"
              required
              bind:value={handoff.summary}></textarea></label
          >
          <div class="form-grid">
            <label class="field"
              >Tested build / commit<input
                required
                bind:value={handoff.build_ref}
                placeholder="Commit SHA or build identifier"
              /></label
            ><label class="field"
              >Delivery reference<input
                required
                bind:value={handoff.delivery_ref}
                placeholder="Merged PR, commit, or release"
              /></label
            >
          </div>
          <label class="field"
            >Checks run and results<textarea
              required
              rows="2"
              bind:value={handoff.checks}></textarea></label
          ><label class="field"
            >Known limitations<input bind:value={handoff.limitations} /></label
          ><label class="field"
            >Test preconditions<input
              bind:value={handoff.preconditions}
            /></label
          >
          <div class="section-label">WALKTHROUGH</div>
          {#each handoff.steps as step, index}<div class="form-grid step-form">
              <label class="field"
                >Step {index + 1}<input
                  required
                  bind:value={step.instruction}
                  placeholder="What should the reviewer do?"
                /></label
              ><label class="field"
                >Expected result<input
                  required
                  bind:value={step.expected}
                  placeholder="What should happen?"
                /></label
              >
            </div>{/each}<button
            class="text-button"
            type="button"
            onclick={() =>
              (handoff.steps = [
                ...handoff.steps,
                { instruction: "", expected: "" },
              ])}
            disabled={handoff.steps.length >= 50}>＋ Add step</button
          >
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >Submit for verification →</button
            >
          </div>
        </form>
      {:else}<form onsubmit={saveDraft}>
          {#if modal === "issue"}<label class="field"
              >Product<select bind:value={draft.product}
                >{#each data.products as p}<option value={p.key}
                    >{p.name} · {p.key}</option
                  >{/each}</select
              ></label
            >{/if}<label class="field"
            >Issue title<input
              required
              bind:value={draft.title}
              placeholder="What needs to change?"
            /></label
          ><label class="field"
            >Problem & expected outcome<textarea
              rows="4"
              bind:value={draft.body}
              placeholder="Give the next person or agent enough context to start."
            ></textarea></label
          >{#if modal === "edit"}<label class="field"
              >Acceptance criteria<textarea
                rows="3"
                bind:value={draft.acceptance}
                placeholder="How will we know this is complete?"
              ></textarea></label
            >
            <div class="form-grid">
              <label class="field"
                >Human owner<input
                  bind:value={draft.owner}
                  placeholder="Who will review the result?"
                /></label
              ><label class="field"
                >Priority<select bind:value={draft.priority}
                  ><option value="low">Low</option><option value="medium"
                    >Medium</option
                  ><option value="high">High</option><option value="urgent"
                    >Urgent</option
                  ></select
                ></label
              >
            </div>
            <p class="hint">
              Editing Ready work returns it to Backlog for a fresh readiness
              decision.
            </p>{/if}
          <div class="modal-footer">
            <span class="hint"
              >{modal === "issue"
                ? "Captured now. Shaped when it matters."
                : `Editing version ${draft.version}`}</span
            ><button class="primary" disabled={busy}
              >{modal === "issue" ? "Create issue" : "Save brief"} →</button
            >
          </div>
        </form>{/if}
    </dialog>
  </div>
{/if}
