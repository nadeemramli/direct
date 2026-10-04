// Context document helpers (DIR-23): labels and provenance for display.

import type { ContextLink, ContextSource, Snapshot } from "./api";

export interface RetainedDocument {
  id: string;
  kind: string;
  label?: string | null;
  title?: string | null;
  source_id?: string | null;
}

export function sourceLabel(source: ContextSource) {
  switch (source.kind) {
    case "obsidian":
      return "Obsidian";
    case "retained_record":
      return "Linear document";
    case "url":
      return "Link";
  }
}

/** Where the document lives, including original IDs and addresses when known. */
export function sourceDetail(link: ContextLink) {
  const s = link.source;
  if (s.kind === "obsidian") return s.path;
  if (s.kind === "url") return s.url;
  const parts = [link.source_id ? `Linear ID ${link.source_id}` : "", link.url ?? ""].filter(Boolean);
  return parts.length ? parts.join(" · ") : "Retained record";
}

/** A recheck that saw different bytes from those seen when the link was made. */
export function changedSinceLinked(link: ContextLink) {
  return (
    !!link.pinned_fingerprint &&
    !!link.observation.fingerprint &&
    link.pinned_fingerprint !== link.observation.fingerprint
  );
}

export function targetLabel(link: ContextLink, data: Snapshot) {
  switch (link.target_kind) {
    case "issue":
      return link.target;
    case "project":
      return `project ${data.projects.find((p) => p.id === link.target)?.name ?? ""}`.trim();
    case "goal":
      return `goal ${data.goals.find((g) => g.id === link.target)?.name ?? ""}`.trim();
    case "release":
      return `release ${data.releases.find((r) => r.id === link.target)?.version_label ?? ""}`.trim();
  }
}

export function linksFor(links: ContextLink[] | undefined, kind: ContextLink["target_kind"], target: string) {
  return (links || []).filter((l) => l.target_kind === kind && l.target === target);
}
