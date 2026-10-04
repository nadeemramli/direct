<script lang="ts">
  // Customer requests (DIR-24): capture provenance-bearing requests, link them
  // to existing work without copying their text, and promote one explicitly
  // into a single Inbox issue. Nothing here makes work Ready or sets priority.
  import { untrack, type Snippet } from "svelte";
  import { DirectError } from "./api";
  import type { CustomerSignal, SignalSourceKind, SignalTargetKind, Snapshot } from "./api";
  import {
    SOURCE_KINDS,
    dateInput,
    fromDateInput,
    signalStatus,
    sourceLabel,
    statusCounts,
    visibleSignals,
    type SignalFilter,
  } from "./signals";

  let {
    data,
    connected,
    commit,
    product,
    focus = "",
    openIssue,
    controls,
  }: {
    data: Snapshot;
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
    /** Selected product ID, or "all". */
    product: string;
    /** A request to show selected when the view opens. */
    focus?: string;
    openIssue: (key: string) => void;
    controls?: Snippet;
  } = $props();

  let filter = $state<SignalFilter>("active");
  // Opens on the sidebar's product; the owner can widen or change it here.
  let scope = $state(untrack(() => product));
  let search = $state("");
  let selectedId = $state(untrack(() => focus));
  let capturing = $state(false);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  // One capture form is one operation: an unchanged retry after an unconfirmed
  // save reuses its request ID, so Direct records it at most once.
  let operation = $state(crypto.randomUUID());
  let draft = $state(emptyDraft());
  let linkKind = $state<SignalTargetKind>("issue");
  let linkTarget = $state("");
  let promoteTitle = $state("");
  let confirmPromote = $state(false);
  // Correcting a captured request (DIR-24 repair). Provenance and links stay as they are.
  let editing = $state(false);
  let editDraft = $state({ version: 0, source_kind: "support" as SignalSourceKind, source_reference: "", received: "", customer_reference: "", summary: "" });

  function emptyDraft() {
    // Without a scoped product, default to the first product in sidebar order (DIR-71).
    const ordered = [...data.products].sort(
      (a, b) => (a.sort_order || 0) - (b.sort_order || 0) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
    );
    const fallback = ordered[0]?.key ?? "DIR";
    return {
      product: data.products.find((p) => p.id === scope)?.key ?? fallback,
      source_kind: "support" as SignalSourceKind,
      source_reference: "",
      received: dateInput(Math.floor(Date.now() / 1000)),
      customer_reference: "",
      summary: "",
    };
  }

  let signals = $derived(data.customer_signals || []);
  let shown = $derived(visibleSignals(signals, { product: scope, filter, search }));
  let counts = $derived(statusCounts(signals, { product: scope, search }));
  let selected = $derived(signals.find((s) => s.id === selectedId));
  let productName = (id: string) => data.products.find((p) => p.id === id)?.name ?? "Unknown product";
  let issueTitle = (key: string) => data.issues.find((i) => i.key === key)?.title;
  let projectName = (id: string) => data.projects.find((p) => p.id === id)?.name;
  let linkableIssues = $derived(
    selected
      ? data.issues
          .filter((i) => !i.parent && i.product_id === selected.product_id)
          .filter((i) => !selected.links.some((l) => l.kind === "issue" && l.target === i.key))
      : [],
  );
  let linkableProjects = $derived(
    selected
      ? data.projects
          .filter((p) => p.product_id === selected.product_id)
          .filter((p) => !selected.links.some((l) => l.kind === "project" && l.target === p.id))
      : [],
  );
  const FILTERS: { value: SignalFilter; label: string }[] = [
    { value: "active", label: "Active" },
    { value: "open", label: "Open" },
    { value: "linked", label: "Linked" },
    { value: "promoted", label: "Promoted" },
    { value: "archived", label: "Archived" },
    { value: "all", label: "All" },
  ];

  function failed(e: unknown) {
    return e instanceof DirectError && e.outcome === "unknown"
      ? `${e.message} It may already be saved; repeating the same action sends the same request, so Direct applies it at most once.`
      : String(e).replace(/^Error: /, "");
  }
  async function run(command: Record<string, unknown>, done: string, intent = "") {
    if (busy) return undefined;
    busy = true;
    error = "";
    notice = "";
    try {
      const result = await commit<any>(command, intent);
      notice = done;
      return result;
    } catch (e) {
      error = failed(e);
      return undefined;
    } finally {
      busy = false;
    }
  }
  function select(signal: CustomerSignal) {
    selectedId = signal.id;
    capturing = false;
    editing = false;
    error = "";
    notice = "";
    linkTarget = "";
    promoteTitle = "";
    confirmPromote = false;
  }
  function startCapture() {
    capturing = true;
    selectedId = "";
    draft = emptyDraft();
    operation = crypto.randomUUID();
    error = "";
    notice = "";
  }
  async function capture(event: SubmitEvent) {
    event.preventDefault();
    const command = {
      op: "capture_signal",
      product: draft.product,
      source_kind: draft.source_kind,
      source_reference: draft.source_reference,
      summary: draft.summary,
      received_at: fromDateInput(draft.received),
      customer_reference: draft.customer_reference,
    };
    const saved = await run(command, "Request captured.", operation);
    if (saved) {
      capturing = false;
      if (filter === "archived") filter = "active";
      select(saved as CustomerSignal);
      notice = "Request captured.";
    }
  }
  async function link() {
    if (!selected || !linkTarget) return;
    const target = linkTarget;
    const saved = await run(
      { op: "link_signal", id: selected.id, expected_version: selected.version, kind: linkKind, target },
      `Linked to ${linkKind === "issue" ? target : (projectName(target) ?? "project")}.`,
    );
    if (saved) linkTarget = "";
  }
  async function unlink(kind: SignalTargetKind, target: string) {
    if (!selected) return;
    await run(
      { op: "unlink_signal", id: selected.id, expected_version: selected.version, kind, target },
      "Link removed. The issue or project itself is unchanged.",
    );
  }
  async function promote() {
    if (!selected) return;
    if (!confirmPromote) {
      confirmPromote = true;
      return;
    }
    const result = await run(
      {
        op: "promote_signal",
        id: selected.id,
        expected_version: selected.version,
        ...(promoteTitle.trim() ? { title: promoteTitle.trim() } : {}),
      },
      "Promoted to an Inbox issue.",
    );
    confirmPromote = false;
    if (result?.issue?.key) notice = `Promoted to ${result.issue.key} in the Inbox. It is not Ready until you make it Ready.`;
  }
  function startEdit() {
    if (!selected) return;
    editDraft = {
      // The version this edit started from: a concurrent change makes the save stale.
      version: selected.version,
      source_kind: selected.source_kind,
      source_reference: selected.source_reference,
      received: dateInput(selected.received_at),
      customer_reference: selected.customer_reference,
      summary: selected.summary,
    };
    editing = true;
    error = "";
    notice = "";
  }
  async function saveEdit(event: SubmitEvent) {
    event.preventDefault();
    if (!selected) return;
    const saved = await run(
      {
        op: "update_signal",
        id: selected.id,
        expected_version: editDraft.version,
        source_kind: editDraft.source_kind,
        source_reference: editDraft.source_reference,
        summary: editDraft.summary,
        received_at: fromDateInput(editDraft.received),
        customer_reference: editDraft.customer_reference,
      },
      "Request updated. Links and promotion history are unchanged.",
    );
    if (saved) editing = false;
  }
  async function setArchived(archived: boolean) {
    if (!selected) return;
    await run(
      { op: "archive_signal", id: selected.id, expected_version: selected.version, archived },
      archived ? "Request archived. It stays searchable under Archived." : "Request restored.",
    );
  }
  function received(signal: CustomerSignal) {
    return new Date(signal.received_at * 1000).toLocaleDateString();
  }
</script>

<section class="list-panel signals-list" aria-label="Customer requests">
  <div class="page-heading">
    <div class="eyebrow">CUSTOMER SIGNALS · INTAKE ONLY</div>
    <div class="heading-row">
      <h1>Customer requests</h1>
      <span class="count-pill">{counts.active}</span>
    </div>
    <p>
      Record what customers ask for, with its source. Link requests to existing work or
      promote one into an Inbox issue. Requests never make work Ready or set priority.
    </p>
  </div>
  <div class="project-toolbar signals-toolbar">
    <button class="secondary" disabled={!connected || busy} onclick={startCapture}
      >＋ Capture request</button
    >
    <select aria-label="Request product scope" bind:value={scope}
      ><option value="all">All products</option>{#each data.products as p}<option value={p.id}>{p.name}</option>{/each}</select
    >
    <input
      class="signal-search"
      type="search"
      aria-label="Search customer requests"
      placeholder="Search text, source, customer, issue…"
      bind:value={search}
    />
  </div>
  <div class="signal-filters" role="group" aria-label="Request status">
    {#each FILTERS as item}<button
        class="filter-chip"
        class:active={filter === item.value}
        aria-pressed={filter === item.value}
        onclick={() => (filter = item.value)}
        >{item.label} <small>{counts[item.value]}</small></button
      >{/each}
  </div>
  <div class="template-rows signal-rows">
    {#each shown as signal (signal.id)}<button
        class="trace-row signal-row"
        class:active={signal.id === selectedId}
        aria-label={`Request: ${signal.summary.split("\n")[0]}`}
        onclick={() => select(signal)}
        ><b>{signal.summary.split("\n")[0]}</b>
        <small
          >{sourceLabel(signal.source_kind)}{signal.source_reference
            ? ` · ${signal.source_reference}`
            : ""}{signal.customer_reference ? ` · ${signal.customer_reference}` : ""} · {received(
            signal,
          )}{scope === "all" ? ` · ${productName(signal.product_id)}` : ""}</small
        ><span class={`signal-status ${signalStatus(signal)}`}
          >{signalStatus(signal)}{signal.promoted_issue_key ? ` → ${signal.promoted_issue_key}` : ""}</span
        ></button
      >{:else}<div class="empty compact">
        <div class="empty-symbol">◌</div>
        <h3>{signals.length ? "No matching requests" : "No customer requests yet"}</h3>
        <p>
          {signals.length
            ? "Change the status filter or search."
            : "Capture what a customer asked for, then link it or promote it when it deserves work."}
        </p>
      </div>{/each}
  </div>
</section>
<aside class="detail-panel signal-detail" aria-label="Customer request detail">
  <div class="detail-top">
    <span>{capturing ? "Capture request" : selected ? "Customer request" : "Customer requests"}</span>
    <div>{#if controls}{@render controls()}{/if}</div>
  </div>
  <div class="detail-body">
    {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
    {#if notice}<p class="hint" role="status">{notice}</p>{/if}
    {#if capturing}
      <form class="template-form" onsubmit={capture}>
        <fieldset class="modal-forms" disabled={busy}>
          <div class="form-grid">
            <label class="field"
              >Product<select aria-label="Request product" bind:value={draft.product}
                >{#each data.products as p}<option value={p.key}>{p.name}</option>{/each}</select
              ></label
            ><label class="field"
              >Source<select aria-label="Request source" bind:value={draft.source_kind}
                >{#each SOURCE_KINDS as kind}<option value={kind.value}>{kind.label}</option>{/each}</select
              ></label
            >
          </div>
          <label class="field"
            >What did the customer ask for?<textarea
              aria-label="Request text"
              required
              rows="4"
              maxlength="2000"
              bind:value={draft.summary}
              placeholder="Keep it concise. The text is stored once on this request."
            ></textarea></label
          >
          <div class="form-grid">
            <label class="field"
              >Source reference<input
                aria-label="Source reference"
                maxlength="300"
                bind:value={draft.source_reference}
                placeholder="e.g. Ticket 4412, call notes 3 Oct"
              /></label
            ><label class="field"
              >Received<input aria-label="Received date" type="date" required bind:value={draft.received} /></label
            >
          </div>
          <label class="field"
            >Customer reference<input
              aria-label="Customer reference"
              maxlength="120"
              bind:value={draft.customer_reference}
              placeholder="Account, segment or ID — not an email or phone number"
            /></label
          >
          <div class="modal-footer">
            <button type="button" class="secondary" onclick={() => (capturing = false)}>Cancel</button
            ><button class="primary" disabled={!connected || busy}>Capture request</button>
          </div>
        </fieldset>
      </form>
    {:else if selected && editing}
      <form class="template-form" onsubmit={saveEdit}>
        <fieldset class="modal-forms" disabled={busy}>
          <p class="hint">
            Correct what was captured. Import provenance, links, promotion and archive state stay as
            they are.
          </p>
          <label class="field"
            >Source<select aria-label="Edit request source" bind:value={editDraft.source_kind}
              >{#each SOURCE_KINDS as kind}<option value={kind.value}>{kind.label}</option>{/each}</select
            ></label
          >
          <label class="field"
            >What did the customer ask for?<textarea
              aria-label="Edit request text"
              required
              rows="4"
              maxlength="2000"
              bind:value={editDraft.summary}
            ></textarea></label
          >
          <div class="form-grid">
            <label class="field"
              >Source reference<input aria-label="Edit source reference" maxlength="300" bind:value={editDraft.source_reference} /></label
            ><label class="field"
              >Received<input aria-label="Edit received date" type="date" required bind:value={editDraft.received} /></label
            >
          </div>
          <label class="field"
            >Customer reference<input
              aria-label="Edit customer reference"
              maxlength="120"
              bind:value={editDraft.customer_reference}
              placeholder="Account, segment or ID — not an email or phone number"
            /></label
          >
          <div class="modal-footer">
            <button type="button" class="secondary" onclick={() => (editing = false)}>Cancel</button
            ><button class="primary" disabled={!connected || busy}>Save changes</button>
          </div>
        </fieldset>
      </form>
    {:else if selected}
      {@const status = signalStatus(selected)}
      <div class="signal-summary">
        <span class={`signal-status ${status}`}>{status}</span>
        <p class="signal-text">{selected.summary}</p>
      </div>
      <dl class="signal-meta">
        <dt>Product</dt><dd>{productName(selected.product_id)}</dd>
        <dt>Source</dt><dd
          >{sourceLabel(selected.source_kind)}{selected.source_reference
            ? ` · ${selected.source_reference}`
            : ""}</dd
        >
        <dt>Received</dt><dd>{received(selected)}</dd>
        <dt>Customer</dt><dd>{selected.customer_reference || "Not recorded"}</dd>
        {#if selected.external_source}<dt>Imported from</dt><dd
            >{selected.external_source} · {selected.external_id}</dd
          >{/if}
        <dt>Captured by</dt><dd>{selected.created_by} · version {selected.version}</dd>
      </dl>
      {#if selected.unresolved_mappings.length}<div class="info-card warning">
          <span class="card-symbol">!</span>
          <div>
            <b>Unresolved source mappings</b>
            <ul>
              {#each selected.unresolved_mappings as mapping}<li>
                  {mapping.external_source} · {mapping.external_id}{mapping.note ? ` — ${mapping.note}` : ""}
                </li>{/each}
            </ul>
          </div>
        </div>{/if}

      <h3 class="signal-heading">Linked work</h3>
      {#if selected.links.length}<ul class="signal-links">
          {#each selected.links as item (item.kind + item.target)}<li>
              {#if item.kind === "issue"}<button class="text-button" onclick={() => openIssue(item.target)}
                  >{item.target}</button
                > <span>{issueTitle(item.target) ?? ""}</span>{:else}<span class="badge">Project</span>
                <span>{projectName(item.target) ?? item.target}</span>{/if}
              {#if selected.promoted_issue_key === item.target}<small>promoted</small>{/if}
              <button
                class="icon-button"
                aria-label={`Unlink ${item.kind === "issue" ? item.target : (projectName(item.target) ?? "project")}`}
                disabled={!connected || busy}
                onclick={() => unlink(item.kind, item.target)}>×</button
              >
            </li>{/each}
        </ul>{:else}<p class="hint">Not linked to any issue or project yet.</p>{/if}
      {#if !selected.archived}<div class="signal-link-form">
          <select aria-label="Link type" bind:value={linkKind} onchange={() => (linkTarget = "")}
            ><option value="issue">Issue</option><option value="project">Project</option></select
          >
          {#if linkKind === "issue"}<select aria-label="Issue to link" bind:value={linkTarget}
              ><option value="">Choose an issue…</option>{#each linkableIssues as issue}<option value={issue.key}
                  >{issue.key} · {issue.title}</option
                >{/each}</select
            >{:else}<select aria-label="Project to link" bind:value={linkTarget}
              ><option value="">Choose a project…</option>{#each linkableProjects as p}<option value={p.id}
                  >{p.name}</option
                >{/each}</select
            >{/if}
          <button class="secondary" disabled={!connected || busy || !linkTarget} onclick={link}>Link</button>
        </div>{/if}

      <h3 class="signal-heading">Promote</h3>
      {#if selected.promoted_issue_key}<p class="hint">
          Promoted to <button class="text-button" onclick={() => openIssue(selected.promoted_issue_key!)}
            >{selected.promoted_issue_key}</button
          >. A request is promoted at most once.
        </p>{:else if selected.archived}<p class="hint">Restore this request to promote it.</p>{:else}
        <label class="field"
          >Issue title (optional)<input
            aria-label="Promoted issue title"
            maxlength="120"
            bind:value={promoteTitle}
            placeholder={selected.summary.split("\n")[0].slice(0, 120)}
          /></label
        >
        {#if confirmPromote}<div class="info-card warning" role="alert">
            <span class="card-symbol">!</span>
            <div>
              <b>Create one Inbox issue from this request?</b>
              <p>
                The issue keeps a reference to this request. It starts in Backlog and is not Ready
                until you make it Ready. A request can only be promoted once.
              </p>
            </div>
          </div>{/if}
        <button class="primary" disabled={!connected || busy} onclick={promote}
          >{confirmPromote ? "Create Inbox issue" : "Promote to Inbox issue…"}</button
        >
      {/if}

      <div class="signal-actions">
        <button class="secondary" disabled={!connected || busy} onclick={startEdit}>Edit request</button>
        <button class="secondary" disabled={!connected || busy} onclick={() => setArchived(!selected.archived)}
          >{selected.archived ? "Restore request" : "Archive request"}</button
        >
      </div>
    {:else}
      <div class="empty compact">
        <div class="empty-symbol">◌</div>
        <h3>Select a request</h3>
        <p>Choose a request to link it, promote it or archive it.</p>
      </div>
    {/if}
  </div>
</aside>
