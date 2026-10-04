// Customer request list logic (DIR-24): status, search, filters and counts.
// The service stores each request once; this only derives views of them.

import type { CustomerSignal, SignalSourceKind } from "./api";

export const SOURCE_KINDS: { value: SignalSourceKind; label: string }[] = [
  { value: "email", label: "Email" },
  { value: "call", label: "Call" },
  { value: "chat", label: "Chat" },
  { value: "support", label: "Support ticket" },
  { value: "sales", label: "Sales conversation" },
  { value: "interview", label: "Interview" },
  { value: "survey", label: "Survey" },
  { value: "social", label: "Social" },
  { value: "other", label: "Other" },
];

export function sourceLabel(kind: SignalSourceKind) {
  return SOURCE_KINDS.find((k) => k.value === kind)?.label ?? kind;
}

/** Open: not yet linked; linked: tied to work; promoted: became an Inbox issue. */
export type SignalStatus = "open" | "linked" | "promoted" | "archived";
export type SignalFilter = SignalStatus | "active" | "all";

export function signalStatus(signal: CustomerSignal): SignalStatus {
  if (signal.archived) return "archived";
  if (signal.promoted_issue_key) return "promoted";
  if (signal.links.length) return "linked";
  return "open";
}

export function matchesFilter(signal: CustomerSignal, filter: SignalFilter) {
  const status = signalStatus(signal);
  if (filter === "all") return true;
  if (filter === "active") return status !== "archived";
  return status === filter;
}

export function matchesSearch(signal: CustomerSignal, query: string) {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return true;
  const haystack = [
    signal.summary,
    signal.source_reference,
    signal.customer_reference,
    signal.external_source ?? "",
    signal.external_id ?? "",
    signal.promoted_issue_key ?? "",
    ...signal.links.map((link) => link.target),
  ]
    .join("\n")
    .toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

export interface SignalQuery {
  product: string; // product ID or "all"
  filter: SignalFilter;
  search: string;
}

/** Newest received first; ties by capture order. */
export function visibleSignals(signals: CustomerSignal[], query: SignalQuery) {
  return signals
    .filter((s) => query.product === "all" || s.product_id === query.product)
    .filter((s) => matchesFilter(s, query.filter))
    .filter((s) => matchesSearch(s, query.search))
    .sort((a, b) => b.received_at - a.received_at || b.created_at - a.created_at);
}

/** Counts per status within the product and search scope (filter-independent). */
export function statusCounts(signals: CustomerSignal[], query: Omit<SignalQuery, "filter">) {
  const counts: Record<SignalFilter, number> = {
    active: 0,
    open: 0,
    linked: 0,
    promoted: 0,
    archived: 0,
    all: 0,
  };
  for (const s of signals) {
    if (query.product !== "all" && s.product_id !== query.product) continue;
    if (!matchesSearch(s, query.search)) continue;
    const status = signalStatus(s);
    counts[status] += 1;
    counts.all += 1;
    if (status !== "archived") counts.active += 1;
  }
  return counts;
}

/** Local date input value <-> epoch seconds at local midnight. */
export function dateInput(epochSeconds: number) {
  const d = new Date(epochSeconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
export function fromDateInput(value: string) {
  const [y, m, d] = value.split("-").map(Number);
  if (!y || !m || !d) return 0;
  return Math.floor(new Date(y, m - 1, d).getTime() / 1000);
}
