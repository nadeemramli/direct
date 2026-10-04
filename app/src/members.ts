// Agent members and issue assignment (DIR-75): presentation helpers.
//
// The service enforces every rule. These helpers explain choices up front so
// the owner sees why a member, role or model is not offered.
import type { AgentMember, AgentRole } from "./api";

export const RUNTIMES = ["claude-code", "codex"] as const;

export interface Choice<T> {
  item: T;
  /** Why it cannot be chosen; empty when it can. */
  blocked: string;
}

export function memberChoices(members: AgentMember[], productId: string): Choice<AgentMember>[] {
  return members.map((m) => ({
    item: m,
    blocked: !m.enabled
      ? "disabled"
      : !m.product_ids.includes(productId)
        ? "not permitted in this product"
        : "",
  }));
}

export function roleChoices(roles: AgentRole[], member: AgentMember | undefined): Choice<AgentRole>[] {
  return roles
    .filter((r) => r.status === "active" || r.status === "draft")
    .map((r) => ({
      item: r,
      blocked:
        r.status !== "active"
          ? "not active"
          : member && !r.runtime_compatibility.includes(member.runtime)
            ? `not written for ${member.runtime}`
            : "",
    }));
}

/** Verified models first; the member's default appears as unverified if no check ran it. */
export function modelChoices(member: AgentMember | undefined): Choice<string>[] {
  if (!member) return [];
  const verified = member.capability?.verified_models || [];
  const choices: Choice<string>[] = verified.map((m) => ({ item: m, blocked: "" }));
  if (member.default_model && !verified.includes(member.default_model))
    choices.push({ item: member.default_model, blocked: "unverified — run a capability check" });
  return choices;
}

export function capabilityLabel(member: AgentMember): string {
  const c = member.capability;
  if (!c) return "Unverified: no capability check recorded";
  return c.verified_models.length
    ? `Verified ${c.verified_models.join(", ")} · ${c.harness_version}`
    : `Checked ${c.harness_version}; no model verified`;
}

export function checkCommand(member: AgentMember): string {
  const model = member.default_model || "<model>";
  return `direct members check --member-id ${member.id} --model ${model} --request-id <id>`;
}
