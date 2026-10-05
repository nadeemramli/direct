// Delivery ledger and build identity (DIR-82): presentation helpers.
import type { DeliveryFact } from "./api";

export interface BuildStamp {
  commit: string;
  dirty: string;
  built_at?: number;
}

export interface BuildComparison {
  state: "match" | "mismatch" | "dirty" | "unknown";
  detail: string;
}

const short = (c: string | undefined) => (c && c !== "unknown" ? c.slice(0, 7) : "unknown");

/** Compare this UI's embedded stamp with the service and the bundle it serves. */
export function compareBuilds(ui: BuildStamp | null, service: BuildStamp | null, bundle: BuildStamp | null): BuildComparison {
  if (!ui || !service) return { state: "unknown", detail: "Build identity unavailable" };
  const parts = `UI ${short(ui.commit)} · service ${short(service.commit)}`;
  if (ui.commit === "unknown" || service.commit === "unknown")
    return { state: "unknown", detail: `${parts} — provenance unknown` };
  if (ui.commit !== service.commit || (bundle && bundle.commit !== service.commit))
    return { state: "mismatch", detail: `${parts}${bundle ? ` · served bundle ${short(bundle.commit)}` : ""} — builds differ` };
  if (ui.dirty !== "false" || service.dirty !== "false")
    return { state: "dirty", detail: `${parts} — built from uncommitted changes` };
  return { state: "match", detail: `Build ${short(service.commit)} (UI and service match)` };
}

export const GAP_LABEL: Record<string, string> = {
  unknown: "Unknown",
  unsaved: "Unsaved work",
  unpushed: "Unpushed",
  unmerged: "Unmerged",
  unverified: "Unverified",
  unbuilt: "Not built",
  uninstalled: "Not installed",
  stale: "Stale observation",
  drifted: "Drifted",
  failed: "Failed",
  delivered: "Delivered",
};

export function gapClass(gap: string): string {
  if (gap === "delivered") return "cached";
  if (gap === "failed" || gap === "drifted") return "unavailable";
  return "stale";
}

/** One line describing a fact, without implying any later stage. */
export function factLine(f: DeliveryFact): string {
  const d = f.detail as Record<string, unknown> & { kind: string };
  const sha = (v: unknown) => (typeof v === "string" ? v.slice(0, 12) : "?");
  switch (d.kind) {
    case "worker": return `${d.runtime} on ${d.host}${d.model ? ` · ${d.model}` : ""}${d.session_id ? ` · ${d.session_id}` : ""}`;
    case "working_tree": return `${d.branch} in ${d.checkout} · ${d.dirty_files} dirty${d.head ? ` · HEAD ${sha(d.head)}` : ""}`;
    case "commit": return `${sha(d.sha)} on ${d.branch}`;
    case "push": return `${sha(d.sha)} → ${d.remote} ${d.remote_ref}`;
    case "pull_request": return `PR #${d.number} → ${d.target} (head ${sha(d.head_sha)})`;
    case "integration": return `${d.method}: ${(d.sources as string[]).map(sha).join(", ")} → ${sha(d.result)} on ${d.target}`;
    case "check": return `${d.name} ${d.outcome} on ${sha(d.commit)}`;
    case "build": return `${sha(d.commit)} · dirty ${d.dirty} · ${(d.artifacts as { name: string }[]).map((a) => a.name).join(", ")}`;
    case "install": return `${d.path} · ${sha(d.sha256)}`;
    case "running": return `${d.path} · service ${sha(d.service_commit)}${d.bundle ? ` · ${d.bundle}` : ""}`;
    default: return d.kind;
  }
}
