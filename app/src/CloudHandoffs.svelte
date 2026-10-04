<script lang="ts">
  // Read-only view of an issue's cloud handoffs (DIR-58). Coordinators prepare
  // and reconcile through the agent contract; owners review the evidence here.
  import type { CloudHandoff } from "./api";
  import { STATUS_LABEL, checksByEnvironment, packetMarkdown, stateClass } from "./handoffs";

  let { handoffs }: { handoffs: CloudHandoff[] } = $props();
  let copied = $state<{ id: string; ok: boolean } | null>(null);

  const date = (at: number) => new Date(at * 1000).toLocaleString();
  async function copy(h: CloudHandoff) {
    try {
      await navigator.clipboard.writeText(packetMarkdown(h));
      copied = { id: h.id, ok: true };
    } catch {
      copied = { id: h.id, ok: false };
    }
  }
</script>

{#if handoffs.length}
  <div class="section-label spaced">CLOUD HANDOFFS <span>{handoffs.length}</span></div>
  <p class="hint">
    Bounded packets for one cloud session each. Cloud results are evidence for
    the coordinator; a delivered Pass still needs local integration, install
    and owner-entrypoint smoke.
  </p>
  {#each [...handoffs].reverse() as h (h.id)}
    {@const r = h.reconciliation}
    {@const groups = checksByEnvironment(r?.checks || [])}
    <article class="cloud-handoff {h.status}">
      <div class="reference-heading">
        <span class="source-state {stateClass(h)}">{STATUS_LABEL[h.status]}</span>
        <button class="text-button" onclick={() => copy(h)}
          >{copied?.id === h.id && copied.ok ? "Copied packet" : "Copy packet"}</button
        >
      </div>
      {#if copied?.id === h.id && !copied.ok}<p class="source-warning">
          The clipboard is unavailable here. Print the packet with
          <code>direct handoff {h.issue_key} --id {h.id}</code>.
        </p>{/if}
      <dl class="evidence reference-meta">
        <dt>Repository</dt><dd><code>{h.packet.repository}</code> from <code>{h.packet.base_ref}</code></dd>
        <dt>Required model</dt><dd><code>{h.packet.required_model}</code></dd>
        <dt>Prepared</dt><dd>{h.prepared_by} · {date(h.prepared_at)} · issue v{h.packet.issue_version} · claim {h.packet.claim_actor}</dd>
        <dt>Guidance</dt><dd>{h.packet.guidance.length ? h.packet.guidance.map((g) => g.document_id).join(", ") : "None pinned"}</dd>
        <dt>Packet</dt><dd><code>{h.packet_sha256.slice(0, 12)}</code></dd>
        {#if r}
          <dt>Session</dt><dd><code>{r.session_id}</code> · <code>{r.model}</code></dd>
          <dt>Pull request</dt><dd><a href={r.pr_url} target="_blank" rel="noreferrer">{r.pr_url}</a></dd>
          <dt>Tested SHA</dt><dd><code>{r.tested_sha}</code></dd>
          <dt>Cloud verdict</dt><dd class="cloud-verdict {r.cloud_verdict}">{r.cloud_verdict} · {r.reconciled_by} · {date(r.reconciled_at)}</dd>
          {#if groups.cloud.length}<dt>Cloud checks</dt><dd>
              {#each groups.cloud as c}<span class="check-chip {c.outcome}">{c.name} · {c.outcome}</span>{/each}
            </dd>{/if}
          {#if groups.local.length}<dt>Local checks</dt><dd>
              {#each groups.local as c}<span class="check-chip {c.outcome}">{c.name} · {c.outcome}</span>{/each}
            </dd>{/if}
          {#if r.summary}<dt>Summary</dt><dd>{r.summary}</dd>{/if}
        {/if}
        {#if h.withdrawn_reason}<dt>Withdrawn</dt><dd>{h.withdrawn_reason}</dd>{/if}
      </dl>
    </article>
  {/each}
{/if}
