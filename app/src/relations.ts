// Relations chosen in the New issue form. They are sent with `create_issue`
// and written in the same transaction; the service applies the same rules as
// `create_issue_link`, so these checks only catch problems before saving.

export type DraftRelationKind = "parent" | "blocked_by" | "related";
export interface DraftRelation {
  target_key: string;
  kind: DraftRelationKind;
}
interface Candidate {
  key: string;
  title: string;
  product_id: string;
  parent?: string | null;
}

export const relationKinds: { kind: DraftRelationKind; label: string }[] = [
  { kind: "parent", label: "Parent" },
  { kind: "blocked_by", label: "Blocked by" },
  { kind: "related", label: "Related" },
];

/** Same-product work issues matching a key or title search. */
export function relationCandidates<T extends Candidate>(
  issues: T[],
  productId: string,
  query: string,
): T[] {
  const needle = query.trim().toLowerCase();
  return issues.filter(
    (issue) =>
      !issue.parent &&
      issue.product_id === productId &&
      (!needle ||
        issue.key.toLowerCase().includes(needle) ||
        issue.title.toLowerCase().includes(needle)),
  );
}

/** Why `next` cannot join the pending relations, or "" when it can. */
export function relationProblem(
  pending: DraftRelation[],
  next: DraftRelation,
): string {
  if (!next.target_key) return "Choose an issue to relate.";
  if (
    pending.some(
      (link) => link.kind === next.kind && link.target_key === next.target_key,
    )
  )
    return `${next.target_key} is already in this list.`;
  if (next.kind === "parent" && pending.some((link) => link.kind === "parent"))
    return "An issue can have only one parent.";
  return "";
}

/** Split pending relations into those still valid for `productId` and those that are not. */
export function relationsForProduct<T extends Candidate>(
  pending: DraftRelation[],
  issues: T[],
  productId: string,
): { kept: DraftRelation[]; dropped: DraftRelation[] } {
  const kept: DraftRelation[] = [];
  const dropped: DraftRelation[] = [];
  for (const link of pending) {
    const target = issues.find((issue) => issue.key === link.target_key);
    (target?.product_id === productId ? kept : dropped).push(link);
  }
  return { kept, dropped };
}
