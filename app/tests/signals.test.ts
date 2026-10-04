// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { CustomerSignal } from "../src/api.ts";
import {
  dateInput,
  fromDateInput,
  matchesSearch,
  signalStatus,
  statusCounts,
  visibleSignals,
} from "../src/signals.ts";

function signal(id: string, patch: Partial<CustomerSignal> = {}): CustomerSignal {
  return {
    id,
    product_id: "p1",
    source_kind: "support",
    source_reference: "",
    summary: `Request ${id}`,
    received_at: 100,
    customer_reference: "",
    unresolved_mappings: [],
    links: [],
    archived: false,
    version: 1,
    created_by: "owner",
    created_at: 100,
    updated_at: 100,
    ...patch,
  };
}

test("status follows archive, promotion and links in that order", () => {
  assert.equal(signalStatus(signal("a")), "open");
  assert.equal(signalStatus(signal("b", { links: [{ kind: "issue", target: "DIR-1", linked_by: "", linked_at: 0 }] })), "linked");
  assert.equal(signalStatus(signal("c", { promoted_issue_key: "DIR-2" })), "promoted");
  assert.equal(signalStatus(signal("d", { promoted_issue_key: "DIR-2", archived: true })), "archived");
});

test("search matches every term across text, source, customer and linked work", () => {
  const s = signal("a", {
    summary: "Export weekly report as CSV",
    source_reference: "Ticket 4412",
    customer_reference: "Acme account",
    links: [{ kind: "issue", target: "DIR-42", linked_by: "", linked_at: 0 }],
  });
  assert.ok(matchesSearch(s, ""));
  assert.ok(matchesSearch(s, "csv acme"));
  assert.ok(matchesSearch(s, "4412"));
  assert.ok(matchesSearch(s, "dir-42"));
  assert.ok(!matchesSearch(s, "csv globex"));
});

test("filters, product scope and newest-first ordering", () => {
  const all = [
    signal("old", { received_at: 10 }),
    signal("new", { received_at: 30 }),
    signal("other", { product_id: "p2", received_at: 20 }),
    signal("gone", { archived: true, received_at: 40 }),
    signal("linked", { received_at: 5, links: [{ kind: "project", target: "x", linked_by: "", linked_at: 0 }] }),
  ];
  const ids = (list: CustomerSignal[]) => list.map((s) => s.id);
  assert.deepEqual(ids(visibleSignals(all, { product: "all", filter: "active", search: "" })), ["new", "other", "old", "linked"]);
  assert.deepEqual(ids(visibleSignals(all, { product: "p1", filter: "open", search: "" })), ["new", "old"]);
  assert.deepEqual(ids(visibleSignals(all, { product: "p1", filter: "archived", search: "" })), ["gone"]);
  assert.deepEqual(ids(visibleSignals(all, { product: "all", filter: "all", search: "" })).length, 5);
  assert.deepEqual(statusCounts(all, { product: "p1", search: "" }), {
    active: 3,
    open: 2,
    linked: 1,
    promoted: 0,
    archived: 1,
    all: 4,
  });
});

test("date inputs round-trip through local midnight", () => {
  const value = "2026-10-04";
  assert.equal(dateInput(fromDateInput(value)), value);
  assert.equal(fromDateInput("not-a-date"), 0);
});
