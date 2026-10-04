// Agent roles and skill packages (DIR-74): presentation helpers.
//
// The service owns every rule; these helpers only explain state, such as why
// an owner cannot activate a revision yet, before they press the button.
import type { AgentRole, RoleGuidancePin, RolePublication, SkillPackage, TheoriaDocument } from "./api";

export type PinState = "cached" | "stale" | "unavailable";

/** Role revisions grouped by key, newest revision first. */
export function roleGroups(roles: AgentRole[]): { key: string; revisions: AgentRole[] }[] {
  const keys = [...new Set(roles.map((r) => r.key))].sort();
  return keys.map((key) => ({
    key,
    revisions: roles.filter((r) => r.key === key).sort((a, b) => b.revision - a.revision),
  }));
}

/** Same rule as issue guidance pins: unavailable, or stale when the cache moved on. */
export function pinState(pin: RoleGuidancePin, documents: TheoriaDocument[]): PinState {
  const document = documents.find((d) => d.id === pin.document_id);
  if (!document || document.availability === "unavailable") return "unavailable";
  return pin.recorded_fingerprint === document.fingerprint ? "cached" : "stale";
}

/** A harness is verified only by recorded fresh-session evidence for this revision. */
export function harnessStatus(
  role: AgentRole,
  publications: RolePublication[],
): { harness: string; verified: boolean }[] {
  return role.runtime_compatibility.map((harness) => ({
    harness,
    verified: publications.some(
      (p) => p.role_id === role.id && p.harness === harness && p.evidence.length > 0,
    ),
  }));
}

/** Reasons the service will refuse activation, in the order it checks them. */
export function activationBlockers(
  role: AgentRole,
  skills: SkillPackage[],
  documents: TheoriaDocument[],
): string[] {
  const blockers: string[] = [];
  if (role.status !== "draft") blockers.push(`This revision is ${role.status}.`);
  if (!role.owner_direction.trim())
    blockers.push("No owner direction is recorded; register a revision that includes it.");
  for (const id of role.skills) {
    const s = skills.find((k) => k.id === id);
    if (!s) blockers.push(`Skill revision ${id} is missing.`);
    else if (s.retired) blockers.push(`Skill ${s.name} r${s.revision} is retired.`);
  }
  for (const pin of role.guidance.filter((g) => g.mandatory))
    if (pinState(pin, documents) === "unavailable")
      blockers.push(`Mandatory guidance ${pin.document_id} is unavailable.`);
  return blockers;
}

export function skillLabel(skills: SkillPackage[], id: string): string {
  const s = skills.find((k) => k.id === id);
  return s ? `${s.name} r${s.revision}` : id;
}
