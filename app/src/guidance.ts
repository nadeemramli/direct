// Theoria guidance provenance for issues and method findings (DIR-39).
//
// A playbook version is only ever the value explicitly recorded on an issue's
// guidance link. Nothing here reads document content, titles, dates or
// catalog revisions to guess one: an unrecorded version stays Unknown.

import type { Issue, TheoriaDocument, TheoriaReference } from "./api";

export type GuidanceState = "cached" | "stale" | "unavailable";

/** How a pinned reference relates to the current cache entry. */
export function guidanceState(
  document: TheoriaDocument | undefined,
  recordedFingerprint?: string | null,
): GuidanceState {
  if (!document || document.availability === "unavailable") return "unavailable";
  if (recordedFingerprint !== undefined && recordedFingerprint !== document.fingerprint)
    return "stale";
  return "cached";
}

export const UNKNOWN_VERSION = "Unknown — not recorded when linked";

/** The recorded playbook version, or null when none was recorded. */
export function recordedVersion(reference: TheoriaReference): string | null {
  return reference.playbook_version?.trim() || null;
}

export function versionLabel(reference: TheoriaReference): string {
  return recordedVersion(reference) ?? UNKNOWN_VERSION;
}

export const STATE_NOTE: Record<GuidanceState, string> = {
  cached: "Pinned fingerprint matches the current cache",
  stale: "Current cache differs from the pinned fingerprint; the historical pin is preserved",
  unavailable: "Authoritative source unavailable; the pin cannot be checked",
};

export function shortFingerprint(fingerprint: string | null | undefined) {
  return fingerprint ? fingerprint.slice(0, 12) : "unavailable";
}

export interface GuidanceUse {
  issue: Issue;
  reference: TheoriaReference;
}

/** Every issue that pins this guidance, with the reference it recorded. */
export function guidanceUses(issues: Issue[], documentId: string): GuidanceUse[] {
  return issues.flatMap((issue) =>
    issue.theoria_refs
      .filter((reference) => reference.document_id === documentId)
      .map((reference) => ({ issue, reference })),
  );
}

export interface PlaybookContext {
  /** Distinct explicitly recorded versions, in link order. */
  versions: string[];
  /** Linked guidance without a recorded version. */
  unknown: number;
  linked: number;
}

/**
 * The playbook context of an issue's method findings. Findings carry no
 * version of their own, so this is the issue's pinned guidance only.
 */
export function playbookContext(issue: Issue | undefined): PlaybookContext {
  const references = issue?.theoria_refs || [];
  const versions: string[] = [];
  let unknown = 0;
  for (const reference of references) {
    const version = recordedVersion(reference);
    if (version === null) unknown += 1;
    else if (!versions.includes(version)) versions.push(version);
  }
  return { versions, unknown, linked: references.length };
}

export function playbookContextLabel(context: PlaybookContext): string {
  if (!context.linked) return "Unknown — no guidance linked to this issue";
  const parts = [...context.versions];
  if (context.unknown)
    parts.push(
      context.versions.length
        ? `Unknown for ${context.unknown} of ${context.linked} links`
        : UNKNOWN_VERSION,
    );
  return parts.join(" · ");
}

/**
 * Guidance an issue can link (DIR-57): its own product's catalog plus documents
 * the owner shared with the workspace, minus what it already pins. Own-product
 * documents come first so shared copies never crowd out scoped guidance.
 */
export function linkableGuidance(
  documents: TheoriaDocument[],
  issue: Pick<Issue, "product_id" | "theoria_refs">,
): TheoriaDocument[] {
  const pinned = new Set(issue.theoria_refs.map((reference) => reference.document_id));
  const open = documents.filter((document) => !pinned.has(document.id));
  return [
    ...open.filter((document) => document.product_id === issue.product_id),
    ...open.filter((document) => document.product_id !== issue.product_id && document.shared),
  ];
}
