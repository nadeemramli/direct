// Sidebar product order and owner-defined sections (DIR-71).
//
// The service stores the arrangement; this module only derives the displayed
// groups and the complete `arrange_products` payload for a move. Every move
// sends the whole arrangement, so the service can refuse a stale one.

import type { Product, ProductSection } from "./api";

export interface SidebarGroup {
  /** null is the ungrouped list, shown first. */
  section: ProductSection | null;
  products: Product[];
}

export interface Arrangement {
  sections: string[];
  products: { product_id: string; section_id: string | null }[];
}

function byOrderThenId<T extends { id: string; sort_order?: number }>(a: T, b: T) {
  // Ties fall back to ID order, which is how the service lists legacy records.
  return (a.sort_order || 0) - (b.sort_order || 0) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

export function sidebarGroups(
  products: Product[],
  sections: ProductSection[] = [],
): SidebarGroup[] {
  const ordered = [...sections].sort(byOrderThenId);
  const known = new Set(ordered.map((section) => section.id));
  const sorted = [...products].sort(byOrderThenId);
  return [
    {
      section: null,
      products: sorted.filter((p) => !p.section_id || !known.has(p.section_id)),
    },
    ...ordered.map((section) => ({
      section,
      products: sorted.filter((p) => p.section_id === section.id),
    })),
  ];
}

export function arrangement(groups: SidebarGroup[]): Arrangement {
  return {
    sections: groups.flatMap((group) => (group.section ? [group.section.id] : [])),
    products: groups.flatMap((group) =>
      group.products.map((p) => ({
        product_id: p.id,
        section_id: group.section?.id ?? null,
      })),
    ),
  };
}

function clone(groups: SidebarGroup[]) {
  return groups.map((group) => ({ section: group.section, products: [...group.products] }));
}

function groupIndex(groups: SidebarGroup[], sectionId: string | null) {
  return groups.findIndex((group) => (group.section?.id ?? null) === sectionId);
}

/**
 * Move a product into `sectionId` (null = ungrouped) before the product
 * `beforeId`, or to the end of that group when `beforeId` is null.
 * Returns null when nothing would change or a reference is unknown.
 */
export function moveProduct(
  groups: SidebarGroup[],
  productId: string,
  sectionId: string | null,
  beforeId: string | null,
): SidebarGroup[] | null {
  if (productId === beforeId) return null;
  const next = clone(groups);
  const target = groupIndex(next, sectionId);
  const from = next.findIndex((group) => group.products.some((p) => p.id === productId));
  if (target < 0 || from < 0) return null;
  const source = next[from].products;
  const oldIndex = source.findIndex((p) => p.id === productId);
  const [product] = source.splice(oldIndex, 1);
  const list = next[target].products;
  let index = beforeId ? list.findIndex((p) => p.id === beforeId) : list.length;
  if (index < 0) return null;
  list.splice(index, 0, product);
  return same(groups, next) ? null : next;
}

/**
 * Keyboard move by one step. Inside a group it swaps with its neighbour; at
 * a group edge it crosses into the adjacent group, so every placement is
 * reachable without a pointer.
 */
export function nudgeProduct(
  groups: SidebarGroup[],
  productId: string,
  delta: -1 | 1,
): SidebarGroup[] | null {
  const from = groups.findIndex((group) => group.products.some((p) => p.id === productId));
  if (from < 0) return null;
  const list = groups[from].products;
  const index = list.findIndex((p) => p.id === productId);
  const sectionId = groups[from].section?.id ?? null;
  if (delta < 0 && index > 0) return moveProduct(groups, productId, sectionId, list[index - 1].id);
  if (delta > 0 && index < list.length - 1)
    return moveProduct(groups, productId, sectionId, list[index + 2]?.id ?? null);
  const neighbour = groups[from + delta];
  if (!neighbour) return null;
  const neighbourId = neighbour.section?.id ?? null;
  // Entering the previous group lands at its end; the next group, its start.
  return moveProduct(groups, productId, neighbourId, delta < 0 ? null : neighbour.products[0]?.id ?? null);
}

/** Move a section before `beforeId` (null = last). Ungrouped stays first. */
export function moveSection(
  groups: SidebarGroup[],
  sectionId: string,
  beforeId: string | null,
): SidebarGroup[] | null {
  if (sectionId === beforeId) return null;
  const next = clone(groups);
  const from = groupIndex(next, sectionId);
  if (from <= 0) return null;
  const [group] = next.splice(from, 1);
  const index = beforeId ? groupIndex(next, beforeId) : next.length;
  if (index <= 0) return null;
  next.splice(index, 0, group);
  return same(groups, next) ? null : next;
}

export function nudgeSection(
  groups: SidebarGroup[],
  sectionId: string,
  delta: -1 | 1,
): SidebarGroup[] | null {
  const index = groupIndex(groups, sectionId);
  if (index <= 0) return null;
  if (delta < 0) return index > 1 ? moveSection(groups, sectionId, groups[index - 1].section!.id) : null;
  if (index >= groups.length - 1) return null;
  return moveSection(groups, sectionId, groups[index + 2]?.section?.id ?? null);
}

function same(a: SidebarGroup[], b: SidebarGroup[]) {
  return JSON.stringify(arrangement(a)) === JSON.stringify(arrangement(b));
}

/** Apply an accepted arrangement to local records for immediate feedback. */
export function applyArrangement(
  products: Product[],
  sections: ProductSection[],
  next: Arrangement,
): { products: Product[]; sections: ProductSection[] } {
  const productIndex = new Map(next.products.map((p, index) => [p.product_id, index]));
  const sectionIndex = new Map(next.sections.map((id, index) => [id, index]));
  return {
    products: products.map((p) => {
      const index = productIndex.get(p.id);
      if (index === undefined) return p;
      return { ...p, sort_order: index, section_id: next.products[index].section_id };
    }),
    sections: sections.map((section) => {
      const index = sectionIndex.get(section.id);
      return index === undefined ? section : { ...section, sort_order: index };
    }),
  };
}
