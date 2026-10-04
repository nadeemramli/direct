// Cloud handoff packets and reconciliation evidence (DIR-58).
//
// The packet is frozen by the service; this module only presents it. The
// Markdown mirrors `cloud_packet_markdown` in direct-core so the owner can copy
// exactly what a cloud session receives.
import type { CloudCheck, CloudHandoff } from "./api";

export function packetMarkdown(h: CloudHandoff): string {
  const p = h.packet;
  const lines = [
    `# Cloud handoff: ${p.issue_key} — ${p.title}`,
    "",
    `Handoff \`${h.id}\` · packet sha256 \`${h.packet_sha256}\``,
    "",
    `- Product: ${p.product_key}`,
    `- Issue version: ${p.issue_version}`,
    `- Claim: ${p.claim_actor} until ${p.claim_expires_at}`,
    `- Repository: ${p.repository} (base \`${p.base_ref}\`)`,
    `- Required model: ${p.required_model}`,
    "",
    "## Brief",
    "",
    p.body,
    "",
    "## Acceptance",
    "",
    p.acceptance,
    "",
    "## Guidance",
    "",
  ];
  if (!p.guidance.length) lines.push("None pinned.");
  for (const g of p.guidance)
    lines.push(
      `- ${g.title} (\`${g.document_id}\`, ${g.relative_path}) fingerprint \`${g.recorded_fingerprint ?? "unavailable"}\` · playbook ${g.playbook_version ?? "Unknown"}`,
    );
  lines.push("", "## Evidence plan", "", p.evidence_plan, "", "## Constraints", "");
  for (const c of p.constraints) lines.push(`- ${c}`);
  return lines.join("\n") + "\n";
}

/** Checks grouped by where they ran; cloud checks are never delivery evidence. */
export function checksByEnvironment(checks: CloudCheck[]) {
  return {
    cloud: checks.filter((c) => c.environment === "cloud"),
    local: checks.filter((c) => c.environment === "local"),
  };
}

export const STATUS_LABEL: Record<CloudHandoff["status"], string> = {
  prepared: "Open · awaiting cloud result",
  reconciled: "Reconciled",
  withdrawn: "Withdrawn",
};

/** Badge class for a handoff's state, reusing the guidance source-state colours. */
export function stateClass(h: CloudHandoff): string {
  if (h.status === "withdrawn") return "unavailable";
  if (h.status === "reconciled" && h.reconciliation?.cloud_verdict !== "passed") return "stale";
  return "cached";
}
