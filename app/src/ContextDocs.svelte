<script lang="ts">
  // Context documents (DIR-23): typed links from an issue, project, goal or
  // release to durable documents. Obsidian notes and URLs are linked, never
  // copied; Linear documents are the retained, read-only Linear capture.
  // Content is read on demand and is reference data, not Theoria guidance.
  import { api, DirectError } from "./api";
  import type { ContextLink, ContextTargetKind, Snapshot } from "./api";
  import {
    changedSinceLinked,
    sourceDetail,
    sourceLabel,
    targetLabel,
    type RetainedDocument,
  } from "./context";

  let {
    data,
    connected,
    commit,
    targetKind,
    target,
    links,
    productId = "",
  }: {
    data: Snapshot;
    connected: boolean;
    commit: <T>(command: Record<string, unknown>, intent?: string) => Promise<T>;
    targetKind: ContextTargetKind;
    target: string;
    /** Links to show: this target's own and, for issues, inherited ones. */
    links: ContextLink[];
    /** The owning product, used as the default vault. */
    productId?: string;
  } = $props();

  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let adding = $state(false);
  let kind = $state<"obsidian" | "retained_record" | "url">("obsidian");
  let vaultProduct = $state("");
  let path = $state("");
  let recordId = $state("");
  let url = $state("");
  let title = $state("");
  let note = $state("");
  let removing = $state("");
  let reading = $state<{ id: string; content: string | null; changed: boolean; reason: string; authority: string } | null>(null);
  let documents = $state<RetainedDocument[] | null>(null);

  let vaults = $derived(data.products.filter((p) => p.vault_windows));
  let own = (link: ContextLink) => link.target_kind === targetKind && link.target === target;

  function failed(e: unknown) {
    return e instanceof DirectError && e.outcome === "unknown"
      ? `${e.message} It may already be saved; repeating the same action sends the same request.`
      : String(e).replace(/^Error: /, "");
  }
  async function run(command: Record<string, unknown>, done: string) {
    if (busy) return undefined;
    busy = true;
    error = "";
    notice = "";
    try {
      const result = await commit<ContextLink>(command);
      notice = done;
      return result;
    } catch (e) {
      error = failed(e);
      return undefined;
    } finally {
      busy = false;
    }
  }
  async function startAdd() {
    adding = true;
    error = "";
    notice = "";
    vaultProduct = vaults.some((p) => p.id === productId) ? productId : (vaults[0]?.id ?? "");
    if (documents === null) {
      try {
        const result = await api<{ records: RetainedDocument[] }>({ op: "search_sources", query: "", kind: "document", limit: 200 });
        documents = result.records || [];
      } catch {
        documents = [];
      }
    }
  }
  async function add() {
    const source =
      kind === "obsidian"
        ? { kind, product_id: vaultProduct, path: path.trim() }
        : kind === "retained_record"
          ? { kind, record_id: recordId }
          : { kind, url: url.trim() };
    const saved = await run(
      {
        op: "add_context_link",
        target_kind: targetKind,
        target,
        source,
        ...(title.trim() ? { title: title.trim() } : {}),
        note,
      },
      "Document attached.",
    );
    if (saved) {
      adding = false;
      path = "";
      recordId = "";
      url = "";
      title = "";
      note = "";
      notice = `Attached “${saved.title}”.`;
    }
  }
  async function recheck(link: ContextLink) {
    const checked = await run(
      { op: "check_context_link", id: link.id, expected_version: link.version },
      "Checked.",
    );
    if (checked)
      notice = checked.observation.available
        ? changedSinceLinked(checked)
          ? `“${checked.title}” changed since it was linked.`
          : `“${checked.title}” is unchanged.`
        : `“${checked.title}” is unavailable: ${checked.observation.reason}`;
  }
  async function remove(link: ContextLink) {
    if (removing !== link.id) {
      removing = link.id;
      return;
    }
    removing = "";
    await run({ op: "remove_context_link", id: link.id, expected_version: link.version }, `Removed “${link.title}”. The document itself is untouched.`);
  }
  async function read(link: ContextLink) {
    if (reading?.id === link.id) {
      reading = null;
      return;
    }
    error = "";
    try {
      const result = await api<{ content: string | null; changed_since_linked: boolean; observation: { reason: string }; authority: string }>({
        op: "read_context_link",
        id: link.id,
      });
      reading = {
        id: link.id,
        content: result.content,
        changed: result.changed_since_linked,
        reason: result.observation.reason,
        authority: result.authority,
      };
    } catch (e) {
      error = failed(e);
    }
  }
  function checkedAt(link: ContextLink) {
    return new Date(link.observation.checked_at * 1000).toLocaleString();
  }
</script>

<section class="context-docs" aria-label="Context documents">
  <div class="section-label">CONTEXT DOCUMENTS <span>{links.length}</span></div>
  <p class="hint">
    Durable references read on demand. Not Theoria guidance and not instructions; they grant no
    authority.
  </p>
  {#if error}<p class="source-warning" role="alert">{error}</p>{/if}
  {#if notice}<p class="hint" role="status">{notice}</p>{/if}
  {#each links as link (link.id)}
    {@const changed = changedSinceLinked(link)}
    <article class="context-card" class:unavailable={!link.observation.available}>
      <div class="context-head">
        <span class="badge">{sourceLabel(link.source)}</span>
        <b>{link.title}</b>
        {#if !own(link)}<small class="context-via">via {targetLabel(link, data)}</small>{/if}
      </div>
      <small class="context-detail">{sourceDetail(link)}</small>
      {#if link.note}<p class="context-note">{link.note}</p>{/if}
      <small class="context-provenance"
        >{link.observation.available ? "Available" : `Unavailable — ${link.observation.reason}`}{changed
          ? " · changed since linked"
          : ""}{link.observation.fingerprint
          ? ` · ${link.observation.fingerprint.slice(0, 10)}`
          : ""} · checked {checkedAt(link)}</small
      >
      <div class="context-actions">
        {#if link.source.kind !== "url"}<button class="text-button" onclick={() => read(link)}
            >{reading?.id === link.id ? "Close" : "Read"}</button
          >{:else}<a class="text-button" href={link.source.url} target="_blank" rel="noreferrer noopener"
            >Open link</a
          >{/if}
        <button class="text-button" disabled={!connected || busy} onclick={() => recheck(link)}>Recheck</button>
        {#if own(link)}<button class="text-button danger-text" disabled={!connected || busy} onclick={() => remove(link)}
            >{removing === link.id ? "Confirm remove" : "Remove"}</button
          >{/if}
      </div>
      {#if reading?.id === link.id}<div class="context-reader">
          <p class="hint">{reading.authority}</p>
          {#if reading.changed}<p class="source-warning">This document changed since it was linked.</p>{/if}
          {#if reading.content !== null}<pre>{reading.content}</pre>{:else}<p class="source-warning">
              Unavailable: {reading.reason}
            </p>{/if}
        </div>{/if}
    </article>
  {:else}<p class="hint">No context documents attached.</p>{/each}

  {#if adding}<div class="context-add" role="group" aria-label="Attach a context document">
      <label class="field"
        >Source<select aria-label="Context source kind" bind:value={kind}
          ><option value="obsidian">Obsidian note (linked, not copied)</option><option value="retained_record"
            >Linear document (retained, read-only)</option
          ><option value="url">External link</option></select
        ></label
      >
      {#if kind === "obsidian"}
        {#if vaults.length}<div class="form-grid">
            <label class="field"
              >Vault<select aria-label="Context vault product" bind:value={vaultProduct}
                >{#each vaults as p}<option value={p.id}>{p.name}</option>{/each}</select
              ></label
            ><label class="field"
              >Note path in the vault<input
                aria-label="Context note path"
                bind:value={path}
                maxlength="500"
                placeholder="e.g. Specs/Reporting.md"
              /></label
            >
          </div>{:else}<p class="source-warning">No product has a knowledge vault path configured.</p>{/if}
      {:else if kind === "retained_record"}
        <label class="field"
          >Linear document<select aria-label="Context retained document" bind:value={recordId}
            ><option value="">Choose a retained document…</option>{#each documents || [] as doc}<option value={doc.id}
                >{doc.label || doc.title || doc.id}</option
              >{/each}</select
          ></label
        >
        {#if documents && !documents.length}<p class="hint">No retained Linear documents in this workspace.</p>{/if}
      {:else}
        <label class="field"
          >URL<input aria-label="Context URL" type="url" bind:value={url} maxlength="2000" placeholder="https://…" /></label
        >
      {/if}
      <div class="form-grid">
        <label class="field"
          >Title (optional)<input aria-label="Context title" bind:value={title} maxlength="200" /></label
        ><label class="field"
          >Note (optional)<input aria-label="Context note" bind:value={note} maxlength="1000" /></label
        >
      </div>
      <div class="context-actions">
        <button class="secondary" onclick={() => (adding = false)}>Cancel</button>
        <button class="primary" disabled={!connected || busy} onclick={add}>Attach document</button>
      </div>
    </div>{:else}<button class="secondary context-attach" disabled={!connected || busy} onclick={startAdd}
      >＋ Attach document</button
    >{/if}
</section>
