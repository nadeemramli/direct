import { invoke, isTauri } from "@tauri-apps/api/core";
export type Status =
  "backlog" | "ready" | "doing" | "verify" | "done" | "canceled";
export type Outcome = "pending" | "passed" | "failed" | "canceled";
export interface Product {
  id: string;
  key: string;
  name: string;
  repo_windows: string;
  repo_wsl: string;
  vault_windows: string;
  vault_wsl: string;
}
export interface Issue {
  id: string;
  key: string;
  product_id: string;
  project_id: string | null;
  title: string;
  body: string;
  acceptance: string;
  owner: string;
  priority: string;
  status: Status;
  version: number;
  created_at: number;
  updated_at: number;
  claim: null | { actor: string; expires_at: number };
  needs_fix: boolean;
  parent: string | null;
  verification_key: string | null;
  current_run: string | null;
}
export interface Verification {
  id: string;
  issue_key: string;
  build_ref: string;
  delivery_ref: string;
  summary: string;
  checks: string;
  limitations: string;
  preconditions: string;
  steps: { instruction: string; expected: string }[];
  outcome: Outcome;
  results: { outcome: Outcome; note: string }[];
  submitted_by: string;
  submitted_at: number;
  reviewed_by: string | null;
  reviewed_at: number | null;
  review_note: string;
}
export interface Context {
  issue: Issue;
  product: Product;
  project: Project | null;
  comments: { id: string; actor: string; body: string; at: number }[];
  more_comments: boolean;
  verifications: Verification[];
  history: { seq: number; kind: string; actor: string; at: number }[];
}
export interface Snapshot {
  workspace_id: string;
  products: Product[];
  projects: Project[];
  issues: Issue[];
  cursor: number;
}
export interface Project {
  id: string;
  product_id: string;
  name: string;
  description: string;
  version: number;
  created_at: number;
  updated_at: number;
}
let token = sessionStorage.getItem("direct.session") || "";
const pending = new Map<string, string>();
export async function connect() {
  if (isTauri()) return;
  const grant = new URLSearchParams(location.hash.slice(1)).get("grant");
  if (grant) {
    history.replaceState(null, "", location.pathname);
    const response = await fetch("/api/session", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ grant }),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.message);
    token = data.token;
    sessionStorage.setItem("direct.session", token);
  }
  if (!token)
    throw new Error(
      "Open Direct from the desktop app or run “direct open” to get a fresh local launch link.",
    );
}
export async function api<T = unknown>(
  command: Record<string, unknown>,
  mutation = false,
): Promise<T> {
  const key = JSON.stringify(command);
  const requestId = mutation ? pending.get(key) || crypto.randomUUID() : "";
  if (mutation) pending.set(key, requestId);
  const request = { actor: "owner", request_id: requestId, ...command };
  let data: T;
  if (isTauri()) data = await invoke<T>("direct_command", { request });
  else {
    const response = await fetch("/api/command", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify(request),
    });
    const value = await response.json();
    if (!response.ok)
      throw new Error(
        value.message || "Direct could not complete this action.",
      );
    data = value;
  }
  pending.delete(key);
  return data;
}
