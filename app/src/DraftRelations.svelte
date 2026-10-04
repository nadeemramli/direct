<script lang="ts">
  // Relations for an issue that does not exist yet. They are saved with the
  // issue in one create; nothing is written until the form is submitted.
  import type { Issue } from "./api";
  import {
    relationCandidates,
    relationKinds,
    relationProblem,
    relationsForProduct,
    type DraftRelation,
    type DraftRelationKind,
  } from "./relations";

  let {
    issues,
    productId,
    links = $bindable([]),
    refused = "",
  }: {
    issues: Issue[];
    productId: string;
    links?: DraftRelation[];
    /** The service's reason for refusing a relation on the last create, if any. */
    refused?: string;
  } = $props();

  let kind = $state<DraftRelationKind>("related");
  let query = $state("");
  let target = $state("");
  let problem = $state("");
  let notice = $state("");
  let candidates = $derived(relationCandidates(issues, productId, query));
  // A search with a single match needs no separate selection.
  let chosen = $derived(
    candidates.some((issue) => issue.key === target)
      ? target
      : candidates.length === 1
        ? candidates[0].key
        : "",
  );

  $effect(() => {
    // Relations must stay within the issue's product; changing it drops the
    // others and says so rather than sending a create that would be refused.
    const { kept, dropped } = relationsForProduct(links, issues, productId);
    if (!dropped.length) return;
    links = kept;
    notice = `Removed ${dropped.map((link) => link.target_key).join(", ")}: relations must stay within the issue's product.`;
  });

  function label(value: DraftRelationKind) {
    return relationKinds.find((item) => item.kind === value)?.label || value;
  }
  function titleOf(key: string) {
    return issues.find((issue) => issue.key === key)?.title || "";
  }
  function add() {
    const next = { target_key: chosen, kind };
    problem = relationProblem(links, next);
    if (problem) return;
    links = [...links, next];
    notice = "";
    query = "";
    target = "";
  }
</script>

<fieldset class="draft-relations" aria-label="Relations">
  <legend>Relations <span class="muted">optional</span></legend>
  {#each links as link, index (`${link.kind}:${link.target_key}`)}
    <article class="relation-card">
      <span class="relation-kind">{label(link.kind)}</span>
      <span class="relation-target"><b>{link.target_key}</b> {titleOf(link.target_key)}</span>
      <button
        type="button"
        class="text-button relation-remove"
        aria-label={`Remove ${label(link.kind)} ${link.target_key}`}
        onclick={() => {
          links = links.filter((_, i) => i !== index);
          problem = "";
        }}>Remove</button
      >
    </article>
  {/each}
  <div class="draft-relation-form">
    <label class="field"
      >Relationship<select aria-label="New relation kind" bind:value={kind}
        >{#each relationKinds as item}<option value={item.kind}>{item.label}</option>{/each}</select
      ></label
    >
    <label class="field finder"
      >Find issue<input
        type="search"
        aria-label="Find issue by key or title"
        placeholder="Search key or title…"
        bind:value={query}
        oninput={() => (problem = "")}
        onkeydown={(event) => {
          // Enter adds the relation instead of submitting the whole form.
          if (event.key !== "Enter") return;
          event.preventDefault();
          add();
        }}
      /></label
    >
    <select
      class="target"
      aria-label="Issue to relate"
      value={chosen}
      onchange={(event) => {
        target = event.currentTarget.value;
        problem = "";
      }}
      ><option value="" disabled
        >{candidates.length ? `Select an issue (${candidates.length} found)…` : "No matching issue"}</option
      >{#each candidates as issue (issue.key)}<option value={issue.key}
          >{issue.key} · {issue.title}</option
        >{/each}</select
    >
    <button type="button" class="secondary" disabled={!chosen} onclick={add}>Add relation</button>
  </div>
  {#if problem || refused}<p class="hint relation-problem" role="alert">{problem || refused}</p>{/if}
  {#if notice}<p class="hint" role="status">{notice}</p>{/if}
</fieldset>

<style>
  .draft-relations {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 10px 12px 12px;
    border: 1px solid #303639;
    border-radius: 8px;
  }
  .draft-relations legend {
    padding: 0 4px;
  }
  .draft-relation-form {
    display: grid;
    grid-template-columns: 150px minmax(0, 1fr) auto;
    align-items: end;
    gap: 8px 12px;
  }
  .draft-relation-form .finder {
    grid-column: 2 / -1;
  }
  .draft-relation-form .target {
    grid-column: 1 / 3;
  }
  .relation-problem {
    color: #f0a39a;
  }
</style>
