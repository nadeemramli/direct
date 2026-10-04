// Run with `npm test` (Node's built-in runner; types are stripped by Node).
import assert from "node:assert/strict";
import { test } from "node:test";
import { linkableGuidance } from "../src/guidance.ts";
import type { TheoriaDocument, TheoriaReference } from "../src/api.ts";

const doc = (id: string, product_id: string, shared = false) =>
  ({ id, product_id, shared }) as TheoriaDocument;
const pin = (document_id: string) => ({ document_id }) as TheoriaReference;

test("issues link their own guidance and explicitly shared guidance only", () => {
  const documents = [
    doc("shared-dos", "dir", true),
    doc("dir-only", "dir"),
    doc("ins-only", "ins"),
  ];
  const ids = (refs: TheoriaReference[], product_id: string) =>
    linkableGuidance(documents, { product_id, theoria_refs: refs }).map((d) => d.id);
  assert.deepEqual(ids([], "ins"), ["ins-only", "shared-dos"]);
  assert.deepEqual(ids([], "dir"), ["shared-dos", "dir-only"]);
  assert.deepEqual(ids([pin("shared-dos")], "ins"), ["ins-only"]);
  assert.deepEqual(ids([], "other"), ["shared-dos"]);
});
