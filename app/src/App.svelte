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
    Label,
    TheoriaDocument,
    FindingClassification,
    EvidenceKind,
  } from "./api";
  let data = $state<Snapshot>({
    workspace_id: "",
    products: [],
    projects: [],
    project_progress: [],
    labels: [],
    theoria_documents: [],
    method_findings: [],
    git_traces: [],
    issues: [],
    cursor: 0,
  });
  let selected = $state("");
  let context = $state<Context | null>(null);
  let view = $state("all");
  let product = $state("all");
  let projectFilter = $state("all");
  let labelFilter = $state("all");
  let listMode = $state<"flat" | "project">("flat");
  let sortMode = $state<"updated" | "key">("updated");
  let selectedProject = $derived(
    data.projects.find((p) => p.id === projectFilter),
  );
  let availableProjects = $derived(
    data.projects
      .filter((p) => product === "all" || p.product_id === product)
      .sort(
        (a, b) =>
          projectPriority(a.priority) - projectPriority(b.priority) ||
          (a.sort_order || 0) - (b.sort_order || 0) ||
          a.name.localeCompare(b.name),
      ),
  );
  $effect(() => {
    if (
      selectedProject &&
      product !== "all" &&
      selectedProject.product_id !== product
    )
      projectFilter = "all";
  });
  let allLabels = $derived(
    [...(data.labels || [])].sort((a, b) => a.name.localeCompare(b.name)),
  );
  function labelApplies(label: Label, productId: string) {
    return (
      label.products.length === 0 ||
      label.products.some((rule) => rule.product_id === productId)
    );
  }
  function labelOf(id: string) {
    return data.labels?.find((label) => label.id === id);
  }
  function labelNames(ids: string[] | undefined) {
    return (ids || [])
      .map((id) => labelOf(id)?.name)
      .filter((name): name is string => !!name);
  }
  let availableLabels = $derived(
    allLabels.filter(
      (label) => product === "all" || labelApplies(label, product),
    ),
  );
  let selectedLabel = $derived(allLabels.find((l) => l.id === labelFilter));
  $effect(() => {
    if (
      labelFilter !== "all" &&
      labelFilter !== "none" &&
      !availableLabels.some((label) => label.id === labelFilter)
    )
      labelFilter = "all";
  });
  let labelDraft = $state({
    id: "",
    name: "",
    description: "",
    color: "",
    aliases: "",
    products: [] as {
      product_id: string;
      name: string;
      applies: boolean;
      default_for_new_issues: boolean;
    }[],
    linear_origins: "",
    version: 0,
  });
  let search = $state("");
  let connected = $state(false);
  let error = $state("");
  let busy = $state(false);
  let tab = $state("brief");
  let modal = $state<
    "issue" | "product" | "project" | "label" | "edit" | "submit" | null
  >(null);
  let projectDraft = $state({
    id: "",
    product: "DIR",
    name: "",
    description: "",
    status: "planned",
    priority: "medium",
    sort_order: 0,
    version: 0,
  });
  let draft = $state({
    title: "",
    body: "",
    acceptance: "",
    owner: "",
    priority: "medium",
    product: "DIR",
    planning_scope: "project" as "project" | "inbox",
    project_id: "",
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
  let selectedGuidanceId = $state("");
  let guidanceDocumentId = $state("");
  let guidanceVersion = $state("");
  let finding = $state({
    classification: "method_friction" as FindingClassification,
    observation: "",
    hypothesis: "",
    proposal: "",
    evidenceKind: "check" as EvidenceKind,
    evidenceReference: "",
    evidenceSummary: "",
  });
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
  let selectedGuidance = $derived(
    data.theoria_documents.find((document) => document.id === selectedGuidanceId),
  );
  let issueGuidance = $derived(
    (current?.theoria_refs || []).map((reference) => ({
      reference,
      document: data.theoria_documents.find(
        (document) => document.id === reference.document_id,
      ),
    })),
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
          (labelFilter === "all" ||
            (labelFilter === "none"
              ? !(i.labels || []).length
              : (i.labels || []).includes(labelFilter))) &&
          (view === "all" ||
            (view === "needs"
              ? needsMe(i)
              : view === "active"
                ? ["ready", "doing"].includes(i.status)
                : i.status === view)) &&
          `${i.key} ${i.title} ${i.body} ${labelNames(i.labels).join(" ")}`
            .toLowerCase()
            .includes(search.toLowerCase()),
      )
      .sort((a, b) =>
        sortMode === "key"
          ? issueCode(a.key) - issueCode(b.key) || a.key.localeCompare(b.key)
          : b.updated_at - a.updated_at || a.key.localeCompare(b.key),
      ),
  );
  let groupedVisible = $derived(
    [
      ...availableProjects.map((project) => ({
        id: project.id,
        name: project.name,
        project,
        issues: visible.filter((issue) => issue.project_id === project.id),
      })),
      {
        id: "none",
        name: "No project · Inbox",
        project: null,
        issues: visible.filter((issue) => !issue.project_id),
      },
    ].filter((group) => group.issues.length > 0),
  );
  let title = $derived(
    view === "theoria"
      ? "Theoria"
      : product !== "all"
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
  function shortSha(sha: string) {
    return sha.slice(0, 12);
  }
  function issueCode(key: string) {
    const value = Number(key.split("-").at(-1));
    return Number.isFinite(value) ? value : Number.MAX_SAFE_INTEGER;
  }
  function projectPriority(priority = "medium") {
    return { urgent: 0, high: 1, medium: 2, low: 3 }[priority] ?? 2;
  }
  function projectProgress(id: string) {
    const provided = data.project_progress?.find(
      (progress) => progress.project_id === id,
    );
    if (provided) return provided;
    const issues = parents.filter((issue) => issue.project_id === id);
    const canceled = issues.filter((issue) => issue.status === "canceled").length;
    const completed = issues.filter((issue) => issue.status === "done").length;
    const eligible = issues.length - canceled;
    return {
      project_id: id,
      total: issues.length,
      backlog: issues.filter((issue) => issue.status === "backlog").length,
      active: issues.filter((issue) => ["ready", "doing"].includes(issue.status))
        .length,
      pending_verification: issues.filter((issue) => issue.status === "verify")
        .length,
      completed,
      canceled,
      completion_percent: eligible ? Math.floor((completed * 100) / eligible) : 0,
    };
  }
  function guidanceState(
    document: TheoriaDocument | undefined,
    recordedFingerprint?: string | null,
  ) {
    if (!document || document.availability === "unavailable")
      return "unavailable";
    if (
      recordedFingerprint !== undefined &&
      recordedFingerprint !== document.fingerprint
    )
      return "stale";
    return "cached";
  }
  async function linkGuidance(event: SubmitEvent) {
    event.preventDefault();
    if (!current || !guidanceDocumentId) return;
    if (
      await act({
        op: "link_theoria",
        key: current.key,
        expected_version: current.version,
        document_id: guidanceDocumentId,
        playbook_version: guidanceVersion.trim() || null,
      })
    ) {
      guidanceDocumentId = "";
      guidanceVersion = "";
    }
  }
  async function createFinding(event: SubmitEvent) {
    event.preventDefault();
    if (!current) return;
    if (
      await act({
        op: "create_method_finding",
        key: current.key,
        expected_version: current.version,
        classification: finding.classification,
        observation: finding.observation,
        hypothesis: finding.hypothesis,
        proposal: finding.proposal,
        evidence: [
          {
            kind: finding.evidenceKind,
            reference: finding.evidenceReference,
            summary: finding.evidenceSummary,
          },
        ],
      })
    ) {
      finding = {
        classification: "method_friction",
        observation: "",
        hypothesis: "",
        proposal: "",
        evidenceKind: "check",
        evidenceReference: "",
        evidenceSummary: "",
      };
    }
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
    if (
      !selectedGuidanceId ||
      !snapshot.theoria_documents.some(
        (document) => document.id === selectedGuidanceId,
      )
    )
      selectedGuidanceId = snapshot.theoria_documents[0]?.id || "";
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
      planning_scope:
        i.planning_scope || (i.project_id ? "project" : "inbox"),
      project_id: i.project_id || "",
    };
    modal = "edit";
  }
  function newIssue() {
    const selectedProduct =
      data.products.find((p) => p.id === product) || data.products[0];
    const inheritedProject = selectedProject?.id || "";
    draft = {
      title: "",
      body: "",
      acceptance: "",
      owner: "",
      priority: "medium",
      product: selectedProduct?.key || "DIR",
      planning_scope: "project",
      project_id: inheritedProject,
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
            planning_scope: draft.planning_scope,
            project_id: draft.project_id || null,
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
            planning_scope: draft.planning_scope,
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
          status: p.status || "active",
          priority: p.priority || "medium",
          sort_order: p.sort_order || 0,
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
          status: "planned",
          priority: "medium",
          sort_order: 0,
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
              status: projectDraft.status,
              priority: projectDraft.priority,
              sort_order: projectDraft.sort_order,
            }
          : {
              op: "create_project",
              product: projectDraft.product,
              name: projectDraft.name,
              description: projectDraft.description,
              priority: projectDraft.priority,
              sort_order: projectDraft.sort_order,
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
  function editLabel(label?: Label) {
    labelDraft = {
      id: label?.id || "",
      name: label?.name || "",
      description: label?.description || "",
      color: label?.color || "",
      aliases: (label?.aliases || []).join(", "),
      products: data.products.map((p) => {
        const rule = label?.products.find((rule) => rule.product_id === p.id);
        return {
          product_id: p.id,
          name: p.name,
          applies: !!rule,
          default_for_new_issues: rule?.default_for_new_issues || false,
        };
      }),
      linear_origins: (label?.linear_origins || [])
        .map((origin) => (origin.name ? `${origin.id} | ${origin.name}` : origin.id))
        .join("\n"),
      version: label?.version || 0,
    };
    modal = "label";
  }
  async function saveLabel(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    const fields = {
      name: labelDraft.name,
      description: labelDraft.description,
      color: labelDraft.color.trim(),
      aliases: labelDraft.aliases
        .split(",")
        .map((alias) => alias.trim())
        .filter(Boolean),
      products: labelDraft.products
        .filter((rule) => rule.applies)
        .map((rule) => ({
          product_id: rule.product_id,
          default_for_new_issues: rule.default_for_new_issues,
        })),
      linear_origins: labelDraft.linear_origins
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          const [id, ...rest] = line.split("|");
          return { id: id.trim(), name: rest.join("|").trim() };
        }),
    };
    try {
      const saved = await api<Label>(
        labelDraft.id
          ? {
              op: "update_label",
              id: labelDraft.id,
              expected_version: labelDraft.version,
              ...fields,
            }
          : { op: "create_label", ...fields },
        true,
      );
      await refresh();
      labelFilter = saved.id;
      modal = null;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      busy = false;
    }
  }
  async function toggleIssueLabel(issue: Issue, labelId: string, attach: boolean) {
    if (!labelId) return;
    await act({
      op: attach ? "attach_issue_label" : "detach_issue_label",
      key: issue.key,
      expected_version: issue.version,
      label_id: labelId,
    });
  }
  async function toggleProjectLabel(
    project: Project,
    labelId: string,
    attach: boolean,
  ) {
    if (!labelId) return;
    await act({
      op: attach ? "attach_project_label" : "detach_project_label",
      id: project.id,
      expected_version: project.version,
      label_id: labelId,
    });
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

{#snippet labelChips(ids: string[] | undefined, remove?: (id: string) => void)}
  {#each ids || [] as id}
    {@const label = labelOf(id)}
    {#if label}<span
        class="label-chip"
        title={label.description || label.name}
        style={label.color ? `--label-color: ${label.color}` : ""}
        ><i></i>{label.name}{#if remove}<button
            type="button"
            aria-label={`Remove label ${label.name}`}
            disabled={busy || !connected}
            onclick={(event) => {
              event.stopPropagation();
              remove(id);
            }}>×</button
          >{/if}</span
      >{/if}
  {/each}
{/snippet}

{#snippet labelPicker(
  productId: string,
  attached: string[] | undefined,
  attach: (id: string) => void,
)}
  {@const choices = allLabels.filter(
    (label) => labelApplies(label, productId) && !(attached || []).includes(label.id),
  )}
  {#if choices.length}<select
      class="label-picker"
      aria-label="Add label"
      value=""
      disabled={busy || !connected}
      onchange={(event) => {
        const control = event.currentTarget;
        attach(control.value);
        control.value = "";
      }}
    >
      <option value="" disabled>＋ Add label…</option>
      {#each choices as label}<option value={label.id}>{label.name}</option>{/each}
    </select>{/if}
{/snippet}

{#snippet issueRow(i: Issue)}
  <button
    class="issue-row"
    class:selected={selected === i.key}
    onclick={() => choose(i.key)}
    ><span class="state-icon {i.status}">{glyphs[i.status]}</span>
    <div class="row-content">
      <div class="issue-title">{i.title}</div>
      <div class="issue-meta">
        <span>{i.key}</span><span class="dot-separator">·</span><span
          >{data.products.find((p) => p.id === i.product_id)?.name}</span
        >{#if i.needs_fix}<span class="fix-badge">Needs fix</span>{/if}{#if i.claim}<span
            class="claim-meta">↗ {i.claim.actor}</span
          >{/if}
        {#if i.project_id}<span class="project-tag"
            >{data.projects.find((p) => p.id === i.project_id)?.name}</span
          >{:else}<span class="inbox-tag">Inbox</span>{/if}
        {@render labelChips(i.labels)}
      </div>
    </div>
    <span class="status-badge {i.status}">{labels[i.status]}</span></button
  >
{/snippet}

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
    <div class="nav-label">PRAXIS</div>
    <nav aria-label="Praxis">
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
    <div class="nav-label products-label">THEORIA</div>
    <nav aria-label="Theoria">
      <button
        class:active={view === "theoria"}
        onclick={() => {
          view = "theoria";
          product = "all";
          selected = "";
          context = null;
          if (!selectedGuidanceId)
            selectedGuidanceId = data.theoria_documents[0]?.id || "";
        }}><span>◫</span> Guidance & findings <small>{data.method_findings.length}</small
        ></button
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
      {#if view === "theoria"}
        <section class="list-panel theoria-list">
          <div class="page-heading">
            <div class="eyebrow">WHY · HOW · LEARN</div>
            <div class="heading-row">
              <h1>Theoria</h1>
              <span class="count-pill">{data.theoria_documents.length}</span>
            </div>
            <p>Guidance for the work, shaped by evidence from the work.</p>
          </div>
          <div class="authority-note">
            <b>One source of truth</b>
            <p>
              Maintained principles and playbooks remain in the Development
              Operating System. Direct holds a read-only cache, fingerprints,
              operational findings, and review evidence.
            </p>
          </div>
          <div class="list-label"><span>GUIDANCE</span><span>SOURCE</span></div>
          <div class="guidance-list">
            {#each data.theoria_documents as document}<button
                class="guidance-row"
                class:selected={selectedGuidanceId === document.id}
                onclick={() => (selectedGuidanceId = document.id)}
              >
                <span class="guidance-mark">{document.category.slice(0, 1).toUpperCase()}</span>
                <span>
                  <b>{document.title}</b>
                  <small>{document.description}</small>
                </span>
                <span class="source-state {document.availability}"
                  >{document.availability === "available"
                    ? "Cached"
                    : "Unavailable"}</span
                >
              </button>{/each}
          </div>
          <div class="section-label spaced">
            PROPOSED IMPROVEMENTS <span>{data.method_findings.length}</span>
          </div>
          {#each data.method_findings as proposal}<button
              class="proposal-row"
              onclick={async () => {
                view = "all";
                await choose(proposal.issue_key);
                tab = "theoria";
              }}
            >
              <span class="proposal-state">PROPOSAL · NOT ACCEPTED</span>
              <b>{proposal.proposal}</b>
              <small
                >{proposal.issue_key} · {proposal.classification.replaceAll(
                  "_",
                  " ",
                )} · {date(proposal.created_at)}</small
              >
            </button>{:else}<div class="empty compact">
              <div class="empty-symbol">◇</div>
              <h3>No proposed improvements</h3>
              <p>Method findings recorded from real issues will appear here.</p>
            </div>{/each}
        </section>
        <aside class="detail-panel theoria-detail" aria-label="Theoria guidance detail">
          {#if selectedGuidance}
            <div class="detail-top">
              <span>{selectedGuidance.id}</span><span class="tiny"
                >catalog v{selectedGuidance.catalog_version}</span
              >
            </div>
            <div class="detail-heading">
              <span class="status-badge ready">◫ {selectedGuidance.category}</span>
              <h2>{selectedGuidance.title}</h2>
              <p class="prose">{selectedGuidance.description}</p>
            </div>
            <div class="detail-body">
              <div class="cache-banner {selectedGuidance.availability}">
                <b
                  >{selectedGuidance.availability === "available"
                    ? "Read-only cache from the authoritative source"
                    : "Authoritative source unavailable"}</b
                >
                <p>
                  {selectedGuidance.availability === "available"
                    ? `Checked ${date(selectedGuidance.checked_at)}. Refresh is explicit; “cached” does not claim the file is unchanged after that check.`
                    : `${selectedGuidance.unavailable_reason || "Source unavailable"}. ${selectedGuidance.content ? "The last cached content is retained and labelled." : "No cached content is available."}`}
                </p>
              </div>
              <dl class="evidence source-contract">
                <dt>Stable ID</dt><dd>{selectedGuidance.id}</dd>
                <dt>Source</dt><dd
                  ><code>{selectedGuidance.source_root}/{selectedGuidance.relative_path}</code></dd
                >
                <dt>Fingerprint</dt><dd
                  ><code>{selectedGuidance.fingerprint || "unavailable"}</code></dd
                >
                <dt>Source updated</dt><dd
                  >{selectedGuidance.source_updated || "Unknown"}</dd
                >
                <dt>Cached</dt><dd
                  >{selectedGuidance.cached_at
                    ? date(selectedGuidance.cached_at)
                    : "Never"}</dd
                >
              </dl>
              <div class="section-label">CACHED GUIDANCE</div>
              {#if selectedGuidance.content}<pre class="guidance-content"
                  >{selectedGuidance.content}</pre
                >{:else}<p class="muted">
                  Sync this catalog from an accessible source to read it here.
                </p>{/if}
              <div class="section-label spaced">OPERATIONAL TRACE</div>
              {#each data.issues.filter((issue) => issue.theoria_refs.some((reference) => reference.document_id === selectedGuidance?.id)) as linkedIssue}<button
                  class="trace-row"
                  onclick={async () => {
                    view = "all";
                    await choose(linkedIssue.key);
                    tab = "theoria";
                  }}
                >
                  <b>{linkedIssue.key} · {linkedIssue.title}</b>
                  <small>Open the recorded fingerprint and findings →</small>
                </button>{:else}<p class="muted">
                  No Direct issue currently references this guidance.
                </p>{/each}
            </div>
          {:else}<div class="detail-placeholder">
              <div class="outline-mark">◫</div>
              <h2>No imported guidance yet.</h2>
              <p>Run the explicit Theoria sync contract to populate the cache.</p>
            </div>{/if}
        </aside>
      {:else}
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
          <div class="view-switch" aria-label="Issue layout">
            <button
              class:active={listMode === "flat"}
              aria-pressed={listMode === "flat"}
              onclick={() => (listMode = "flat")}>Flat</button
            ><button
              class:active={listMode === "project"}
              aria-pressed={listMode === "project"}
              onclick={() => (listMode = "project")}>By project</button
            >
          </div>
          {#if listMode === "flat"}<label class="project-filter"
              >Sort<select aria-label="Sort issues" bind:value={sortMode}
                ><option value="updated">Recently updated</option><option value="key"
                  >Issue code</option
                ></select
              ></label
            >{/if}
        </div>
        <div class="project-toolbar label-toolbar">
          <label class="project-filter"
            >Label
            <select aria-label="Filter by label" bind:value={labelFilter}>
              <option value="all">All labels</option>
              <option value="none">No label</option>
              {#each availableLabels as label}<option value={label.id}
                  >{label.name}</option
                >{/each}
            </select>
          </label>
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editLabel()}>＋ New label</button
          >
          {#if selectedLabel}<button
              class="text-button"
              disabled={!connected || busy}
              onclick={() => editLabel(selectedLabel)}>Edit label</button
            >
            <span class="label-filter-note"
              >{selectedLabel.products.length
                ? `Applies to ${selectedLabel.products
                    .map(
                      (rule) =>
                        data.products.find((p) => p.id === rule.product_id)?.name ||
                        "?",
                    )
                    .join(", ")}`
                : "Applies to every product"}{selectedLabel.aliases.length
                ? ` · aliases: ${selectedLabel.aliases.join(", ")}`
                : ""}</span
            >{/if}
        </div>
        {#if selectedProject}
          {@const progress = projectProgress(selectedProject.id)}
          <div class="project-summary">
            <div>
              <strong>{selectedProject.description || "No project outcome recorded."}</strong>
              <span
                >{selectedProject.status || "active"} · {selectedProject.priority ||
                  "medium"} priority · order {selectedProject.sort_order || 0}</span
              >
            </div>
            <div class="project-progress-copy">
              <b>{progress.completion_percent}%</b>
              <span
                >{progress.completed}/{progress.total - progress.canceled} verified done · {progress.pending_verification} verify · {progress.canceled} canceled</span
              >
            </div>
            <div class="progress-track" aria-label="Project completion">
              <span style={`width: ${progress.completion_percent}%`}></span>
            </div>
            <div class="label-row project-labels" aria-label="Project labels">
              <span class="label-row-title">Labels</span>
              {@render labelChips(selectedProject.labels, (id) =>
                toggleProjectLabel(selectedProject, id, false),
              )}
              {@render labelPicker(
                selectedProject.product_id,
                selectedProject.labels,
                (id) => toggleProjectLabel(selectedProject, id, true),
              )}
            </div>
          </div>
        {/if}
        <div class="list-label"><span>ISSUE</span><span>STATUS</span></div>
        <div class="issue-list">
          {#if visible.length && listMode === "project"}
            {#each groupedVisible as group}
              {@const progress = group.project
                ? projectProgress(group.project.id)
                : null}
              <section class="issue-group">
                <header>
                  <div>
                    <strong>{group.name}</strong>
                    {#if group.project}<span
                        >{group.project.status || "active"} · {group.project.priority ||
                          "medium"}</span
                      >{@render labelChips(group.project.labels)}{:else}<span
                        >Explicitly ungrouped work</span
                      >{/if}
                  </div>
                  {#if progress}<small
                      >{progress.completion_percent}% · {progress.completed} done · {progress.pending_verification} verify</small
                    >{:else}<small>{group.issues.length} items</small>{/if}
                </header>
                {#each group.issues as i}{@render issueRow(i)}{/each}
              </section>
            {/each}
          {:else if visible.length}
            {#each visible as i}{@render issueRow(i)}{/each}
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
            </div>{/if}
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
              ><span
                >Route <b>{current.planning_scope === "project" ||
                    (!current.planning_scope && current.project_id)
                    ? "Project work"
                    : "Inbox / maintenance"}</b></span
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
            <div class="label-row issue-labels" aria-label="Issue labels">
              <span class="label-row-title">Labels</span>
              {#if !(current.labels || []).length}<span class="muted tiny"
                  >None</span
                >{/if}
              {@render labelChips(current.labels, (id) =>
                toggleIssueLabel(current, id, false),
              )}
              {@render labelPicker(current.product_id, current.labels, (id) =>
                toggleIssueLabel(current, id, true),
              )}
            </div>
          </div>
          <div class="tabs" role="tablist" aria-label="Issue sections">
            {#each [["brief", "Brief"], ["theoria", "Theoria"], ["verify", "Verification"], ["activity", "Activity"]] as [id, label]}<button
                role="tab"
                aria-selected={tab === id}
                class:active={tab === id}
                onclick={() => (tab = id)}
                >{label}{#if id === "verify" && current.status === "verify"}<span
                    class="tab-dot"
                  ></span>{:else if id === "theoria" && (current.theoria_refs.length || context?.method_findings.length)}<span
                    class="tab-count"
                    >{current.theoria_refs.length + (context?.method_findings.length || 0)}</span
                  >{/if}</button
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
                    disabled={busy ||
                      (current.planning_scope === "project" &&
                        !current.project_id)}
                    onclick={() =>
                      act({
                        op: "ready",
                        key: current.key,
                        expected_version: current.version,
                      })}>Make ready <span>→</span></button
                  >
                  <p class="hint">
                    {current.planning_scope === "project" && !current.project_id
                      ? "Choose a project before making project work Ready."
                      : "Requires a brief, acceptance criteria, and an owner."}
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
            {:else if tab === "theoria"}
              <div class="theoria-card">
                <div class="section-label">
                  RELEVANT GUIDANCE <span>{issueGuidance.length}</span>
                </div>
                <p class="hint">
                  Each link pins the source fingerprint actually used for this
                  issue. A blank playbook version remains explicitly Unknown.
                </p>
                {#each issueGuidance as item}
                  <article class="guidance-reference">
                    <div class="reference-heading">
                      <span class="source-state {guidanceState(item.document, item.reference.recorded_fingerprint)}"
                        >{guidanceState(item.document, item.reference.recorded_fingerprint)}</span
                      >
                      <button
                        class="text-button"
                        onclick={() => {
                          selectedGuidanceId = item.reference.document_id;
                          selected = "";
                          context = null;
                          view = "theoria";
                        }}>Open guidance →</button
                      >
                    </div>
                    <b>{item.document?.title || item.reference.document_id}</b>
                    <p>{item.document?.description || "Catalog entry is not currently available."}</p>
                    <dl class="evidence reference-meta">
                      <dt>Playbook version</dt><dd>{item.reference.playbook_version || "Unknown"}</dd>
                      <dt>Recorded fingerprint</dt><dd><code>{item.reference.recorded_fingerprint || "unavailable"}</code></dd>
                      <dt>Current fingerprint</dt><dd><code>{item.document?.fingerprint || "unavailable"}</code></dd>
                    </dl>
                    {#if guidanceState(item.document, item.reference.recorded_fingerprint) === "stale"}<p class="source-warning">
                        The current cache differs from the fingerprint recorded
                        for this issue. The historical reference is preserved.
                      </p>{:else if guidanceState(item.document, item.reference.recorded_fingerprint) === "unavailable"}<p class="source-warning">
                        The authoritative source could not be checked. Any last
                        cached content remains labelled and is not treated as current.
                      </p>{/if}
                  </article>
                {:else}<p class="muted">
                    No guidance is linked to this issue yet.
                  </p>{/each}

                <form class="inline-theoria-form" onsubmit={linkGuidance}>
                  <label class="field"
                    >Guidance<select bind:value={guidanceDocumentId} required>
                      <option value="" disabled>Select a catalog document…</option>
                      {#each data.theoria_documents.filter((document) => document.product_id === current.product_id && !current.theoria_refs.some((reference) => reference.document_id === document.id)) as document}<option value={document.id}
                          >{document.title}{document.availability === "unavailable" ? " · unavailable" : ""}</option
                        >{/each}
                    </select></label
                  >
                  <label class="field"
                    >Playbook version (optional)<input
                      bind:value={guidanceVersion}
                      placeholder="Leave blank to preserve Unknown"
                    /></label
                  >
                  <button class="secondary" disabled={busy || !guidanceDocumentId}
                    >Link recorded guidance</button
                  >
                </form>
              </div>

              <div class="theoria-card">
                <div class="section-label">
                  METHOD FINDINGS <span>{context?.method_findings.length || 0}</span>
                </div>
                <p class="hint">
                  Findings distinguish observed facts from hypotheses and
                  proposals. Recording one does not change a playbook or enroll an experiment.
                </p>
                {#each context?.method_findings || [] as item}
                  <article class="finding-card">
                    <div class="finding-heading">
                      <span class="proposal-state">PROPOSAL · NOT ACCEPTED</span>
                      <span>{item.classification.replaceAll("_", " ")}</span>
                    </div>
                    <div class="finding-part"><small>OBSERVED FACT</small><p>{item.observation}</p></div>
                    {#if item.hypothesis}<div class="finding-part"><small>HYPOTHESIS / UNRESOLVED</small><p>{item.hypothesis}</p></div>{/if}
                    <div class="finding-part"><small>PROPOSED IMPROVEMENT</small><p>{item.proposal}</p></div>
                    <div class="finding-part"><small>EVIDENCE</small>
                      {#each item.evidence as pointer}<p><b>{pointer.kind}</b> · <code>{pointer.reference}</code>{pointer.summary ? ` — ${pointer.summary}` : ""}</p>{/each}
                    </div>
                    <small class="finding-byline">{item.created_by} · {date(item.created_at)}</small>
                  </article>
                {/each}

                <form class="finding-form" onsubmit={createFinding}>
                  <label class="field">Classification<select bind:value={finding.classification}>
                      <option value="product_defect">Product defect</option>
                      <option value="method_friction">Method friction</option>
                      <option value="both">Both</option>
                    </select></label>
                  <label class="field">Observed fact<textarea rows="3" required bind:value={finding.observation} placeholder="What actually happened? Keep interpretation out of this field."></textarea></label>
                  <label class="field">Hypothesis or unresolved question<textarea rows="2" bind:value={finding.hypothesis} placeholder="What might explain it, or what remains unknown?"></textarea></label>
                  <label class="field">Proposed improvement<textarea rows="3" required bind:value={finding.proposal} placeholder="A reviewable proposal—not an accepted playbook change."></textarea></label>
                  <div class="form-grid">
                    <label class="field">Evidence type<select bind:value={finding.evidenceKind}>
                        <option value="issue">Issue</option>
                        <option value="build">Build</option>
                        <option value="check">Check</option>
                        <option value="owner_review">Owner review</option>
                      </select></label>
                    <label class="field">Evidence reference<input required bind:value={finding.evidenceReference} placeholder="Command, build, issue, or review reference" /></label>
                  </div>
                  <label class="field">Evidence summary<input bind:value={finding.evidenceSummary} placeholder="What the evidence showed" /></label>
                  <div class="proposal-contract">
                    This creates a proposal only. Acceptance, experiment enrollment,
                    promotion, and Jev routing remain outside this operation.
                  </div>
                  <button class="secondary" disabled={busy}>Record method finding</button>
                </form>
              </div>
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
              {#if context?.git_traces?.length}<div class="section-label">
                  GIT EVIDENCE <span>{context.git_traces.length}</span>
                </div>
                {#each [...(context.git_traces || [])].reverse() as trace}<article
                    class="git-trace"
                  >
                    <div class="git-trace-icon">{trace.kind === "push" ? "↥" : "◆"}</div>
                    <div>
                      <b>{trace.kind === "push" ? "Pushed" : "Committed"} <code
                          title={trace.commit_sha}>{shortSha(trace.commit_sha)}</code
                        ></b>
                      <p>
                        <code>{trace.repository}</code> · <code>{trace.branch}</code>
                        {#if trace.kind === "push"}<br />to <code
                            >{trace.remote} · {trace.remote_ref}</code
                          >{/if}
                      </p>
                      <small>{trace.recorded_by} · {date(trace.recorded_at)}</small>
                    </div>
                  </article>{/each}
                <div class="section-label spaced">DISCUSSION</div>
              {:else}<div class="section-label">DISCUSSION</div>{/if}
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
      {/if}
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
        : modal === "label"
          ? labelDraft.id
            ? "Edit label"
            : "New label"
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
              : modal === "label"
                ? labelDraft.id
                  ? "Refine a shared label"
                  : "Define a shared label"
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
          <div class="form-grid">
            {#if projectDraft.id}<label class="field"
                >Status<select bind:value={projectDraft.status}
                  ><option value="planned">Planned</option><option value="active"
                    >Active</option
                  ><option value="paused">Paused</option><option value="completed"
                    >Completed</option
                  ><option value="canceled">Canceled</option></select
                ></label
              >{/if}<label class="field"
              >Priority<select bind:value={projectDraft.priority}
                ><option value="urgent">Urgent</option><option value="high"
                  >High</option
                ><option value="medium">Medium</option><option value="low"
                  >Low</option
                ></select
              ></label
            ><label class="field"
              >Order<input
                type="number"
                min="0"
                max="1000000"
                bind:value={projectDraft.sort_order}
              /></label
            >
          </div>
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >{projectDraft.id ? "Save project" : "Create project"}</button
            >
          </div>
        </form>
      {:else if modal === "label"}<form onsubmit={saveLabel}>
          <div class="form-grid">
            <label class="field"
              >Label name<input
                required
                maxlength="80"
                bind:value={labelDraft.name}
                placeholder="e.g. Bug"
              /></label
            ><label class="field"
              >Color<input
                bind:value={labelDraft.color}
                pattern={"#[0-9a-fA-F]{6}"}
                maxlength="7"
                placeholder="#d73a49 (optional)"
              /></label
            >
          </div>
          <label class="field"
            >Description<input
              maxlength="1000"
              bind:value={labelDraft.description}
              placeholder="When should this label be used?"
            /></label
          >
          <label class="field"
            >Aliases<input
              bind:value={labelDraft.aliases}
              placeholder="Comma-separated, e.g. defect, bugfix"
            /><small
              >Aliases map imported or informal names to this one canonical
              label. Names and aliases are unique across the workspace.</small
            ></label
          >
          <div class="section-label">PRODUCT APPLICABILITY</div>
          <p class="hint">
            Leave every product unchecked to apply this label everywhere.
            Checked products restrict it; “default” attaches it to new issues in
            that product. One definition is shared either way.
          </p>
          {#each labelDraft.products as rule}<div class="label-rule">
              <label
                ><input type="checkbox" bind:checked={rule.applies} />
                {rule.name}</label
              ><label class:disabled={!rule.applies}
                ><input
                  type="checkbox"
                  disabled={!rule.applies}
                  bind:checked={rule.default_for_new_issues}
                /> Default for new issues</label
              >
            </div>{/each}
          <label class="field"
            >Linear origins (optional)<textarea
              rows="2"
              bind:value={labelDraft.linear_origins}
              placeholder="One per line: linear-label-id | Original name"
            ></textarea><small
              >Preserved for a later import; a Linear ID can map to only one
              canonical label.</small
            ></label
          >
          <p class="hint">
            Labels are filtering metadata only. They never change readiness,
            priority, ownership, or verification.
          </p>
          <div class="modal-footer">
            <span class="hint"
              >{labelDraft.id ? `Editing version ${labelDraft.version}` : ""}</span
            ><button class="primary" disabled={busy}
              >{labelDraft.id ? "Save label" : "Create label"}</button
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
              >Product<select
                value={draft.product}
                onchange={(event) => {
                  draft.product = event.currentTarget.value;
                  draft.project_id = "";
                }}
                >{#each data.products as p}<option value={p.key}
                    >{p.name} · {p.key}</option
                  >{/each}</select
              ></label
            >{/if}<label class="field"
            >Work route<select bind:value={draft.planning_scope}
              ><option value="project">Project work · feature or change</option><option
                value="inbox">Inbox / exceptional maintenance</option
              ></select
            ></label
          >{#if modal === "issue" && draft.planning_scope === "project"}<label
              class="field">Project<select required bind:value={draft.project_id}
                ><option value="" disabled>Choose the delivery scope</option>{#each data.projects.filter(
                    (project) =>
                      project.product_id ===
                      data.products.find((product) => product.key === draft.product)
                        ?.id,
                  ) as project}<option value={project.id}>{project.name}</option
                  >{/each}</select
              ><small
                >Feature and change work enters the project directly. Use Inbox only when ungrouped capture is intentional.</small
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
