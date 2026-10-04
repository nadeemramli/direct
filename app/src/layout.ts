// Pane layout (DIR-53, DIR-37), collapsed sidebar product sections (DIR-87)
// and scroll anchors (DIR-40).
//
// The stored layout is a per-device convenience. Anything unreadable,
// out of range or from another format version falls back to defaults, and
// widths are clamped again against the current window on every render.

export const LAYOUT_KEY = "direct.layout.v1";
export const LEFT_MIN = 168;
export const LEFT_MAX = 360;
export const LEFT_RAIL = 56;
export const RIGHT_MIN = 300;
/** Upper bound for the detail pane, shared by the runtime and storage. */
export const RIGHT_MAX = 1100;
export const STEP = 16;
export const BIG_STEP = 64;
export const SECTIONS = ["praxis", "theoria", "sources", "products"] as const;
export type Section = (typeof SECTIONS)[number];

export interface Layout {
  /** Preferred sidebar width; null follows the responsive default. */
  leftWidth: number | null;
  leftCollapsed: boolean;
  /** Preferred detail width; null follows the responsive default. */
  rightWidth: number | null;
  /** Reading mode: the detail pane takes all space the list can spare. */
  rightExpanded: boolean;
  /** Collapsed sidebar sections. */
  collapsed: Record<Section, boolean>;
  /** Owner-defined product sections collapsed on this device (DIR-87). */
  collapsedProductSections: string[];
}

export function defaultLayout(): Layout {
  return {
    leftWidth: null,
    leftCollapsed: false,
    rightWidth: null,
    rightExpanded: false,
    collapsed: { praxis: false, theoria: false, sources: false, products: false },
    collapsedProductSections: [],
  };
}

export function clamp(value: number, min: number, max: number) {
  return Math.round(Math.min(Math.max(value, min), Math.max(min, max)));
}

function width(value: unknown, min: number, max: number) {
  return typeof value === "number" && Number.isFinite(value)
    ? clamp(value, min, max)
    : null;
}

function flag(value: unknown) {
  return typeof value === "boolean" ? value : false;
}

/** Parses stored layout text; corrupt, stale or foreign values are ignored. */
export function parseLayout(text: string | null): Layout {
  const layout = defaultLayout();
  if (!text) return layout;
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return layout;
  }
  if (!value || typeof value !== "object") return layout;
  const stored = value as Record<string, any>;
  if (stored.version !== 1) return layout;
  const left = stored.left && typeof stored.left === "object" ? stored.left : {};
  const right = stored.right && typeof stored.right === "object" ? stored.right : {};
  const sections =
    stored.sections && typeof stored.sections === "object" ? stored.sections : {};
  layout.leftWidth = width(left.width, LEFT_MIN, LEFT_MAX);
  layout.leftCollapsed = flag(left.collapsed);
  layout.rightWidth = width(right.width, RIGHT_MIN, RIGHT_MAX);
  layout.rightExpanded = flag(right.expanded);
  for (const section of SECTIONS) layout.collapsed[section] = flag(sections[section]);
  layout.collapsedProductSections = sectionIds(stored.productSections);
  return layout;
}

/** Stored section IDs: strings only, de-duplicated and bounded. */
function sectionIds(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const ids = value.filter((id): id is string => typeof id === "string" && id.length > 0 && id.length <= 64);
  return [...new Set(ids)].slice(0, 200);
}

/** Drop collapsed IDs for sections that no longer exist (renames keep the ID). */
export function liveCollapsedSections(collapsed: string[], existing: string[]) {
  const known = new Set(existing);
  return collapsed.filter((id) => known.has(id));
}

export function serializeLayout(layout: Layout) {
  return JSON.stringify({
    version: 1,
    left: { width: layout.leftWidth, collapsed: layout.leftCollapsed },
    right: { width: layout.rightWidth, expanded: layout.rightExpanded },
    sections: layout.collapsed,
    productSections: layout.collapsedProductSections,
  });
}

export function loadLayout(): Layout {
  try {
    return parseLayout(localStorage.getItem(LAYOUT_KEY));
  } catch {
    return defaultLayout();
  }
}

export function saveLayout(layout: Layout) {
  try {
    localStorage.setItem(LAYOUT_KEY, serializeLayout(layout));
  } catch {
    // Storage may be unavailable; the layout still works for this session.
  }
}

/** Responsive defaults that match the previous fixed breakpoints. */
export function defaultLeftWidth(viewport: number) {
  return viewport <= 900 ? 170 : viewport <= 1150 ? 195 : 224;
}
export function defaultRightWidth(viewport: number, wide = false) {
  if (wide) return 500;
  return viewport <= 900 ? 330 : viewport <= 1150 ? 360 : viewport >= 1600 ? 470 : 410;
}
export function listMinWidth(viewport: number) {
  return viewport <= 900 ? 260 : 320;
}
export function leftMaxWidth(viewport: number) {
  return clamp(Math.floor(viewport * 0.3), LEFT_MIN, LEFT_MAX);
}
/**
 * The widest the detail pane may be right now: what the list can spare, but
 * never beyond the bound that stored layouts are validated against, so any
 * width the UI saves loads back unchanged.
 */
export function rightMaxWidth(viewport: number, leftWidth: number) {
  return clamp(viewport - leftWidth - listMinWidth(viewport), RIGHT_MIN, RIGHT_MAX);
}

/**
 * A scroll position expressed relative to the element that contains the top
 * edge of a scroll container, so it survives reflow after a resize.
 */
export interface ScrollAnchor {
  scrollTop: number;
  index: number;
  fraction: number;
}

export function captureAnchor(container: Element | null | undefined): ScrollAnchor {
  if (!container) return { scrollTop: 0, index: -1, fraction: 0 };
  const anchor = { scrollTop: container.scrollTop, index: -1, fraction: 0 };
  if (container.scrollTop <= 0) return anchor;
  const top = container.getBoundingClientRect().top + 1;
  // Document order visits ancestors first, so the last match is the most
  // specific element crossing the top edge.
  container.querySelectorAll("*").forEach((element, index) => {
    const rect = element.getBoundingClientRect();
    if (rect.height > 0 && rect.top <= top && rect.bottom > top) {
      anchor.index = index;
      anchor.fraction = (top - rect.top) / rect.height;
    }
  });
  return anchor;
}

export function restoreAnchor(container: Element | null | undefined, anchor: ScrollAnchor) {
  if (!container) return;
  container.scrollTop = anchor.scrollTop;
  if (anchor.index < 0) return;
  const element = container.querySelectorAll("*")[anchor.index];
  if (!element) return;
  const rect = element.getBoundingClientRect();
  const top = container.getBoundingClientRect().top + 1;
  container.scrollTop += rect.top + anchor.fraction * rect.height - top;
}
