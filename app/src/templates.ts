// Workspace intake templates (DIR-22). Pure helpers shared by the picker,
// the management view and provenance badges.
import type {
  ExecutionMode,
  Snapshot,
  TemplateContent,
  TemplateRevision,
  TemplateShape,
  TemplateTarget,
  TemplateUse,
  WorkspaceTemplate,
} from "./api";

export const SHAPES: { value: TemplateShape; label: string }[] = [
  { value: "delivery", label: "Delivery" },
  { value: "discovery_probe", label: "Discovery probe" },
  { value: "bug", label: "Bug" },
  { value: "release", label: "Release" },
];

export const MODES: { value: ExecutionMode; label: string }[] = [
  { value: "agent", label: "Agent executes" },
  { value: "owner", label: "Owner executes" },
  { value: "paired", label: "Paired" },
  { value: "prototype", label: "Prototype as plan" },
];

export function shapeLabel(shape: TemplateShape) {
  return SHAPES.find((item) => item.value === shape)?.label || shape;
}

export function modeLabel(mode: ExecutionMode | null | undefined) {
  return MODES.find((item) => item.value === mode)?.label || "Not set";
}

export function emptyContent(): TemplateContent {
  return {
    intent: "",
    execution_mode: null,
    boundaries: "",
    verification: "",
    checklist: [],
    suggested_priority: null,
    suggested_planning_scope: null,
    suggested_labels: [],
  };
}

export function revisionOf(
  data: Snapshot,
  templateId: string,
  revision: number,
): TemplateRevision | undefined {
  return (data.template_revisions || []).find(
    (item) => item.template_id === templateId && item.revision === revision,
  );
}

export function templateOf(data: Snapshot, templateId: string): WorkspaceTemplate | undefined {
  return (data.templates || []).find((item) => item.id === templateId);
}

/** Active templates for a target, in a stable order. Retired ones are never offered. */
export function selectable(data: Snapshot, target: TemplateTarget) {
  return (data.templates || [])
    .filter((item) => item.target === target && item.status === "active")
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** The shared base plus the product's explicit supplement, if the revision has one. */
export function effective(revision: TemplateRevision, productId: string) {
  const supplement = revision.supplements.find((item) => item.product_id === productId) || null;
  return {
    supplement,
    checklist: [...revision.content.checklist, ...(supplement?.checklist || [])],
    labels: [...revision.content.suggested_labels, ...(supplement?.suggested_labels || [])],
  };
}

function section(heading: string, text: string) {
  return text.trim() ? `${heading}\n${text.trim()}` : "";
}

/** Concise starter text for the brief: intent, execution mode and boundaries. */
export function composeBrief(
  revision: TemplateRevision,
  productId: string,
  mode: ExecutionMode | null,
) {
  const { supplement } = effective(revision, productId);
  return [
    section("Intent", revision.content.intent),
    mode ? `Execution mode\n${modeLabel(mode)}` : "",
    section("Boundaries", revision.content.boundaries),
    section("Product note", supplement?.note || ""),
  ]
    .filter(Boolean)
    .join("\n\n");
}

/** Starter acceptance text. Checklist items are prompts, never recorded as passed. */
export function composeVerification(revision: TemplateRevision, productId: string) {
  const { checklist } = effective(revision, productId);
  return [
    revision.content.verification.trim(),
    checklist.map((item) => `- [ ] ${item}`).join("\n"),
  ]
    .filter(Boolean)
    .join("\n\n");
}

/** Short provenance label, e.g. "Delivery · v1 · outdated (v2 current)". */
export function provenanceLabel(data: Snapshot, used: TemplateUse) {
  const head = templateOf(data, used.template_id);
  const revision = revisionOf(data, used.template_id, used.revision);
  const parts = [`${revision?.name || head?.name || "Template"} · v${used.revision}`];
  if (head && head.current_revision > used.revision)
    parts.push(`outdated (v${head.current_revision} current)`);
  if (head?.status === "retired") parts.push("retired");
  return parts.join(" · ");
}
