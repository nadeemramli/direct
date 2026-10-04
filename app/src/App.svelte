<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { Snippet } from "svelte";
  import { api, connect, DirectError, migration, sourceFile } from "./api";
  import { LOAD_DEADLINE_MS, SessionExpired, startConnection } from "./connection";
  import type { LoadState } from "./connection";
  import {
    BIG_STEP,
    LEFT_MIN,
    LEFT_RAIL,
    RIGHT_MIN,
    STEP,
    captureAnchor,
    clamp,
    defaultLeftWidth,
    defaultRightWidth,
    leftMaxWidth,
    listMinWidth,
    loadLayout,
    restoreAnchor,
    rightMaxWidth,
    saveLayout,
  } from "./layout";
  import type { ScrollAnchor, Section } from "./layout";
  import DraftAssistant from "./DraftAssistant.svelte";
  import TemplatePicker from "./TemplatePicker.svelte";
  import type { Applied as TemplateApplied } from "./TemplatePicker.svelte";
  import TemplatesView from "./TemplatesView.svelte";
  import SignalsView from "./SignalsView.svelte";
  import ContextDocs from "./ContextDocs.svelte";
  import { linksFor } from "./context";
  import { signalStatus } from "./signals";
  import { provenanceLabel } from "./templates";
  import {
    STATE_NOTE,
    guidanceState,
    guidanceUses,
    playbookContext,
    playbookContextLabel,
    recordedVersion,
    shortFingerprint,
    versionLabel,
  } from "./guidance";
  import type { PlanningScope, TemplateSelection } from "./api";
  import type {
    Snapshot,
    Issue,
    Context,
    Verification,
    Status,
    Outcome,
    Project,
    Label,
    FindingClassification,
    EvidenceKind,
    IssueLinkKind,
    Goal,
    Milestone,
    ReleaseRecord,
    ReleaseEvidence,
    SourceBundleDetail,
    SourceRecordSummary,
    SourceRecordView,
    MigrationPreview,
    ProductSection,
  } from "./api";
  import {
    applyArrangement,
    arrangement,
    moveProduct,
    moveSection,
    nudgeProduct,
    nudgeSection,
    sidebarGroups,
    type SidebarGroup,
  } from "./sidebar";
  let data = $state<Snapshot>({
    workspace_id: "",
    products: [],
    product_sections: [],
    customer_signals: [],
    context_links: [],
    projects: [],
    project_progress: [],
    goals: [],
    goal_progress: [],
    milestones: [],
    milestone_progress: [],
    labels: [],
    theoria_documents: [],
    method_findings: [],
    git_traces: [],
    releases: [],
    release_progress: [],
    release_evidence: [],
    release_workflows: [],
    issue_links: [],
    issues: [],
    cursor: 0,
  });
  let selected = $state("");
  let context = $state<Context | null>(null);
  let view = $state("all");
  let product = $state("all");
  let projectFilter = $state("all");
  let labelFilter = $state("all");
  let listMode = $state<"flat" | "project">("flat");
  let sortMode = $state<"updated" | "key">("updated");
  let selectedProject = $derived(
    data.projects.find((p) => p.id === projectFilter),
  );
  let selectedProjectGoals = $derived(
    selectedProject
      ? data.goals.filter((goal) => goal.project_ids.includes(selectedProject.id))
      : [],
  );
  let selectedProjectMilestones = $derived(
    selectedProject
      ? data.milestones
          .filter((milestone) => milestone.project_id === selectedProject.id)
          .sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name))
      : [],
  );
  let selectedProjectReleases = $derived(
    selectedProject
      ? data.releases.filter((release) =>
          release.project_ids.includes(selectedProject.id),
        )
      : [],
  );
  let availableProjects = $derived(
    data.projects
      .filter((p) => product === "all" || p.product_id === product)
      .sort(
        (a, b) =>
          projectPriority(a.priority) - projectPriority(b.priority) ||
          (a.sort_order || 0) - (b.sort_order || 0) ||
          a.name.localeCompare(b.name),
      ),
  );
  $effect(() => {
    if (
      selectedProject &&
      product !== "all" &&
      selectedProject.product_id !== product
    )
      projectFilter = "all";
  });
  let allLabels = $derived(
    [...(data.labels || [])].sort((a, b) => a.name.localeCompare(b.name)),
  );
  function labelApplies(label: Label, productId: string) {
    return (
      label.products.length === 0 ||
      label.products.some((rule) => rule.product_id === productId)
    );
  }
  function labelOf(id: string) {
    return data.labels?.find((label) => label.id === id);
  }
  function labelNames(ids: string[] | undefined) {
    return (ids || [])
      .map((id) => labelOf(id)?.name)
      .filter((name): name is string => !!name);
  }
  let availableLabels = $derived(
    allLabels.filter(
      (label) => product === "all" || labelApplies(label, product),
    ),
  );
  let selectedLabel = $derived(allLabels.find((l) => l.id === labelFilter));
  $effect(() => {
    if (
      labelFilter !== "all" &&
      labelFilter !== "none" &&
      !availableLabels.some((label) => label.id === labelFilter)
    )
      labelFilter = "all";
  });
  let labelDraft = $state({
    id: "",
    name: "",
    description: "",
    color: "",
    aliases: "",
    products: [] as {
      product_id: string;
      name: string;
      applies: boolean;
      default_for_new_issues: boolean;
    }[],
    linear_origins: "",
    version: 0,
  });
  let search = $state("");
  let connected = $state(false);
  // Initial load and browser-session state (DIR-59).
  let load = $state<LoadState>({ kind: "loading" });
  let retryLoad = () => {};
  let error = $state("");
  let busy = $state(false);
  let tab = $state("brief");
  let modal = $state<
    "issue" | "product" | "productPaths" | "section" | "project" | "goal" | "milestone" | "release" | "workflow" | "label" | "edit" | "delete" | "submit" | null
  >(null);
  // Sidebar arrangement (DIR-71).
  let sidebar = $derived(sidebarGroups(data.products, data.product_sections || []));
  let sectionDraft = $state({ id: "", name: "", version: 0, products: 0, confirmDelete: false });
  type Dragging = { kind: "product" | "section"; id: string };
  let dragging = $state<Dragging | null>(null);
  /** Where a drop would land: before an item, or at the end of a group. */
  let dropAt = $state<{ group: string | null; before: string | null } | null>(null);
  let arranging = $state(false);
  let projectDraft = $state({
    id: "",
    product: "DIR",
    name: "",
    description: "",
    status: "planned",
    priority: "medium",
    sort_order: 0,
    version: 0,
  });
  let goalDraft = $state({
    id: "",
    product: "DIR",
    name: "",
    description: "",
    status: "planned",
    priority: "medium",
    project_ids: [] as string[],
    version: 0,
  });
  let milestoneDraft = $state({
    id: "",
    project_id: "",
    name: "",
    description: "",
    sort_order: 0,
    version: 0,
  });
  let releaseDraft = $state({
    id: "",
    product: "DIR",
    name: "",
    version_label: "",
    status: "planned",
    target_ref: "refs/heads/main",
    release_branch: "",
    preview_url: "",
    notes: "",
    project_ids: [] as string[],
    issue_keys: [] as string[],
    version: 0,
  });
  let workflowDraft = $state({
    product: "DIR",
    expected_version: null as number | null,
    branch_strategy: "one_branch_per_release",
    production_ref: "refs/heads/main",
    release_branch_pattern: "refs/heads/release/{version}",
    preview_environment: "preview",
    preview_url_template: "",
    promotion_policy: "verified_owner_approval",
  });
  // Intake template (DIR-22): the selection sent with create_issue/create_project and
  // the last starter text applied, so re-applying never overwrites the owner's edits.
  let issueTemplate = $state<TemplateSelection | null>(null);
  let projectTemplate = $state<TemplateSelection | null>(null);
  let templateStarter = { brief: "", verification: "" };
  // Suggested fields the creator actually edited in this form (reset per form).
  // An edited field is explicit intent, even when it was returned to a suggested or
  // default value, and survives mode, product and template refreshes. Untouched
  // fields follow the active template, or the form default when it suggests nothing.
  let touched = { priority: false, planning_scope: false };
  let formDefaults: { priority: string; planning_scope: PlanningScope } = {
    priority: "medium",
    planning_scope: "project",
  };
  function applyIssueTemplate(applied: TemplateApplied) {
    if (!draft.body.trim() || draft.body === templateStarter.brief) draft.body = applied.brief;
    if (!draft.acceptance.trim() || draft.acceptance === templateStarter.verification)
      draft.acceptance = applied.verification;
    templateStarter = { brief: applied.brief, verification: applied.verification };
    if (!touched.priority) draft.priority = applied.priority || formDefaults.priority;
    if (!touched.planning_scope)
      draft.planning_scope = applied.planning_scope || formDefaults.planning_scope;
  }
  function applyProjectTemplate(applied: TemplateApplied) {
    const starter = [applied.brief, applied.verification].filter(Boolean).join("\n\n");
    if (!projectDraft.description.trim() || projectDraft.description === templateStarter.brief)
      projectDraft.description = starter;
    templateStarter = { brief: starter, verification: "" };
    if (!touched.priority) projectDraft.priority = applied.priority || formDefaults.priority;
  }
  let draft = $state({
    intake: "",
    title: "",
    body: "",
    acceptance: "",
    owner: "",
    priority: "medium",
    product: "DIR",
    planning_scope: "project" as "project" | "inbox",
    project_id: "",
    key: "",
    version: 0,
  });
  // Product repository and knowledge-vault paths (DIR-23 repair).
  let pathsDraft = $state({ product: "", name: "", repo_windows: "", repo_wsl: "", vault_windows: "", vault_wsl: "" });
  let productDraft = $state({
    key: "",
    name: "",
    repo_windows: "",
    repo_wsl: "",
    vault_windows: "",
    vault_wsl: "",
  });
  let handoff = $state({
    build_ref: "",
    delivery_ref: "",
    summary: "",
    checks: "",
    limitations: "",
    preconditions: "",
    steps: [{ instruction: "", expected: "" }],
  });
  let e2eDraft = $state({ environment: "", entrypoint: "", scenarios: "", delivered_build_ref: "", delivery_check: "" });
  let results = $state<{ outcome: Outcome; note: string }[]>([]);
  let reviewNote = $state("");
  // Comment drafts and unconfirmed posts belong to the issue they were
  // written on (DIR-54); switching issues never retargets them.
  let commentDrafts = $state<Record<string, string>>({});
  let commentAttempts = $state<Record<string, Record<string, unknown>>>({});
  let postingKey = $state("");
  // A write that committed but whose refresh failed (DIR-55).
  let notice = $state("");
  // The open form's user operation: retries of its unchanged payload reuse
  // one request ID; a newly opened form is a distinct operation.
  let formIntent = crypto.randomUUID();
  let openedModal: string | null = null;
  // The exact command and intent of the open form's last send. A retry
  // resends this, never the (possibly edited) form; it is cleared only by a
  // definitive answer.
  type FormAttempt = { command: Record<string, unknown>; intent: string };
  let formAttempt = $state<FormAttempt | null>(null);
  let formSending: FormAttempt | null = null;
  // Fields are locked from send until the outcome is known.
  let formPending = $state(false);
  // Set after a confirmed send whose form had changed since it was sent.
  let laterEdits = false;
  let formNotice = $state("");
  let keptNotice = $state("");
  let formConflict = $state<
    | null
    | { kind: "edit" | "submit"; latest: Issue | null }
    | { kind: "merged"; theirs: string[]; both: { field: string; theirs: string }[] }
  >(null);
  const BRIEF_FIELDS = [
    ["title", "Title"],
    ["body", "Problem & expected outcome"],
    ["intake", "Original task context"],
    ["acceptance", "Acceptance criteria"],
    ["owner", "Human owner"],
    ["priority", "Priority"],
    ["planning_scope", "Work route"],
  ] as const;
  type BriefField = (typeof BRIEF_FIELDS)[number][0];
  let editBase: Record<BriefField, string> = {
    intake: "",
    title: "",
    body: "",
    acceptance: "",
    owner: "",
    priority: "",
    planning_scope: "",
  };
  let reopenReason = $state("");
  let clock = $state(Date.now() / 1000);
  let activeRunId = "";
  let selectedGuidanceId = $state("");
  // Pane layout (DIR-53, DIR-37): stored per device, clamped to the window.
  let layout = $state(loadLayout());
  let viewport = $state(typeof window === "undefined" ? 1366 : window.innerWidth);
  let leftMax = $derived(leftMaxWidth(viewport));
  let leftPreferred = $derived(
    clamp(layout.leftWidth ?? defaultLeftWidth(viewport), LEFT_MIN, leftMax),
  );
  let leftWidth = $derived(layout.leftCollapsed ? LEFT_RAIL : leftPreferred);
  let listMin = $derived(listMinWidth(viewport));
  let rightMax = $derived(rightMaxWidth(viewport, leftWidth));
  let rightWidth = $derived(
    layout.rightExpanded
      ? rightMax
      : clamp(
          layout.rightWidth ?? defaultRightWidth(viewport, view === "theoria"),
          RIGHT_MIN,
          rightMax,
        ),
  );
  $effect(() => saveLayout(layout));
  let resizing = $state<null | {
    pane: "left" | "right";
    pointer: number;
    startX: number;
    startWidth: number;
  }>(null);
  function setPaneWidth(pane: "left" | "right", width: number) {
    if (pane === "left") {
      layout.leftWidth = clamp(width, LEFT_MIN, leftMax);
      layout.leftCollapsed = false;
    } else {
      layout.rightWidth = clamp(width, RIGHT_MIN, rightMax);
      layout.rightExpanded = false;
    }
  }
  function startResize(pane: "left" | "right", event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    resizing = {
      pane,
      pointer: event.pointerId,
      startX: event.clientX,
      startWidth: pane === "left" ? leftWidth : rightWidth,
    };
  }
  function moveResize(event: PointerEvent) {
    if (!resizing || event.pointerId !== resizing.pointer) return;
    const delta = event.clientX - resizing.startX;
    setPaneWidth(
      resizing.pane,
      resizing.pane === "left"
        ? resizing.startWidth + delta
        : resizing.startWidth - delta,
    );
  }
  function endResize(event: PointerEvent) {
    if (resizing && event.pointerId === resizing.pointer) resizing = null;
  }
  function resizeKey(pane: "left" | "right", event: KeyboardEvent) {
    const current = pane === "left" ? leftWidth : rightWidth;
    const step = event.shiftKey ? BIG_STEP : STEP;
    // The detail pane grows to the left, so its arrows are mirrored.
    const grow = pane === "left" ? "ArrowRight" : "ArrowLeft";
    const shrink = pane === "left" ? "ArrowLeft" : "ArrowRight";
    if (event.key === grow) setPaneWidth(pane, current + step);
    else if (event.key === shrink) setPaneWidth(pane, current - step);
    else if (event.key === "Home") setPaneWidth(pane, pane === "left" ? LEFT_MIN : RIGHT_MIN);
    else if (event.key === "End") setPaneWidth(pane, pane === "left" ? leftMax : rightMax);
    else if (event.key === "Enter") {
      if (pane === "left") toggleSidebar();
      else toggleDetailExpanded();
    } else return;
    event.preventDefault();
  }
  function resetPane(pane: "left" | "right") {
    if (pane === "left") {
      layout.leftWidth = null;
      layout.leftCollapsed = false;
    } else {
      layout.rightWidth = null;
      layout.rightExpanded = false;
    }
  }
  function toggleSidebar() {
    layout.leftCollapsed = !layout.leftCollapsed;
  }
  function toggleDetailExpanded() {
    layout.rightExpanded = !layout.rightExpanded;
  }
  function sectionOpen(section: Section) {
    return layout.leftCollapsed || !layout.collapsed[section];
  }
  function toggleSection(section: Section) {
    layout.collapsed[section] = !layout.collapsed[section];
  }
  // Return point for guidance opened from an issue (DIR-40).
  let returnTo = $state<null | {
    key: string;
    view: string;
    product: string;
    projectFilter: string;
    labelFilter: string;
    search: string;
    listMode: "flat" | "project";
    sortMode: "updated" | "key";
    tab: string;
    list: ScrollAnchor;
    rowVisible: boolean;
    detail: ScrollAnchor;
  }>(null);
  let listPane = $state<HTMLElement>();
  let detailBody = $state<HTMLElement>();
  let guidanceDocumentId = $state("");
  let guidanceVersion = $state("");
  let relationTarget = $state("");
  let relationKind = $state<IssueLinkKind>("related");
  let finding = $state({
    classification: "method_friction" as FindingClassification,
    observation: "",
    hypothesis: "",
    proposal: "",
    evidenceKind: "check" as EvidenceKind,
    evidenceReference: "",
    evidenceSummary: "",
  });
  function showDialog(element: HTMLDialogElement) {
    element.showModal();
  }
  const labels: Record<Status, string> = {
    backlog: "Backlog",
    ready: "Ready",
    doing: "Doing",
    verify: "Verify",
    done: "Done",
    legacy_completed: "Legacy done",
    canceled: "Canceled",
  };
  const glyphs: Record<Status, string> = {
    backlog: "◌",
    ready: "○",
    doing: "◐",
    verify: "◈",
    done: "✓",
    legacy_completed: "◇",
    canceled: "⊘",
  };
  let parents = $derived(data.issues.filter((i) => !i.parent));
  let ownerOptions = $derived(
    data.issues.some((issue) => issue.owner.trim())
      ? [...new Set(data.issues.map((issue) => issue.owner.trim()).filter(Boolean))].sort(
          (a, b) => a.localeCompare(b),
        )
      : ["Owner"],
  );
  let normalizedSearch = $derived(search.trim().toUpperCase());
  let exactSearchKey = $derived(
    parents.find((issue) => issue.key.toUpperCase() === normalizedSearch)?.key || "",
  );
  let current = $derived(data.issues.find((i) => i.key === selected));
  // Server-computed deletion eligibility for exactly the version on screen. The
  // delete command re-checks it atomically; this only decides what to explain.
  let deletion = $derived(
    context?.deletion &&
      current &&
      context.issue.key === current.key &&
      context.issue.version === current.version
      ? context.deletion
      : null,
  );
  let deletionExplained = $derived(
    !!deletion &&
      !deletion.eligible &&
      !deletion.blockers.some((blocker) => blocker.kind === "status"),
  );
  let activeRun = $derived(
    context?.verifications.find((v) => v.id === current?.current_run),
  );
  let selectedGuidance = $derived(
    data.theoria_documents.find((document) => document.id === selectedGuidanceId),
  );
  let issueGuidance = $derived(
    (current?.theoria_refs || []).map((reference) => ({
      reference,
      document: data.theoria_documents.find(
        (document) => document.id === reference.document_id,
      ),
    })),
  );
  let selectedGuidanceUses = $derived(
    selectedGuidance ? guidanceUses(data.issues, selectedGuidance.id) : [],
  );
  let issuePlaybook = $derived(playbookContext(current));
  const playbookFor = (key: string) =>
    playbookContextLabel(playbookContext(data.issues.find((issue) => issue.key === key)));
  function needsMe(i: Issue) {
    return (
      (i.status === "verify" && data.review_ready_runs?.includes(i.current_run || "")) ||
      i.needs_fix ||
      (i.status === "doing" && (!i.claim || i.claim.expires_at <= clock))
    );
  }
  let attention = $derived(parents.filter(needsMe).length);
  let visible = $derived(
    parents
      .filter(
        (i) =>
          (exactSearchKey
            ? i.key === exactSearchKey
            : (product === "all" || i.product_id === product) &&
          (projectFilter === "all" ||
            (projectFilter === "none"
              ? !i.project_id
              : i.project_id === projectFilter)) &&
          (labelFilter === "all" ||
            (labelFilter === "none"
              ? !(i.labels || []).length
              : (i.labels || []).includes(labelFilter))) &&
          (view === "all" ||
            (view === "needs"
              ? needsMe(i)
              : view === "active"
                ? ["ready", "doing"].includes(i.status)
                : view === "done"
                  ? ["done", "legacy_completed"].includes(i.status)
                  : i.status === view))) &&
          `${i.key} ${i.title} ${i.body} ${labelNames(i.labels).join(" ")} ${data.releases
            .filter(
              (release) =>
                release.issue_keys.includes(i.key) ||
                (!!i.project_id && release.project_ids.includes(i.project_id)),
            )
            .map((release) => `${release.name} ${release.version_label}`)
            .join(" ")}`
            .toLowerCase()
            .includes(search.toLowerCase()),
      )
      .sort((a, b) =>
        sortMode === "key"
          ? issueCode(a.key) - issueCode(b.key) || a.key.localeCompare(b.key)
          : b.updated_at - a.updated_at || a.key.localeCompare(b.key),
      ),
  );
  let visibleReleases = $derived(
    data.releases
      .filter(
        (release) =>
          (product === "all" || release.product_id === product) &&
          `${release.name} ${release.version_label} ${release.notes} ${release.target_ref}`
            .toLowerCase()
            .includes(search.toLowerCase()),
      )
      .sort((a, b) => b.updated_at - a.updated_at || a.name.localeCompare(b.name)),
  );
  let groupedVisible = $derived(
    [
      ...availableProjects.map((project) => ({
        id: project.id,
        name: project.name,
        project,
        issues: visible.filter((issue) => issue.project_id === project.id),
      })),
      {
        id: "none",
        name: "No project · Inbox",
        project: null,
        issues: visible.filter((issue) => !issue.project_id),
      },
    ].filter((group) => group.issues.length > 0),
  );
  let title = $derived(
    view === "theoria"
      ? "Theoria"
      : view === "templates"
      ? "Intake templates"
      : view === "signals"
      ? "Customer requests"
      : view === "sources"
      ? "Imported sources"
      : product !== "all"
      ? data.products.find((p) => p.id === product)?.name || "Product"
      : view === "needs"
        ? "Needs me"
        : view === "active"
          ? "Ready & doing"
          : view === "backlog"
            ? "Inbox"
            : view === "done"
              ? "Completed"
              : "All work",
  );
  function date(at: number) {
    return new Date(at * 1000).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  function shortSha(sha: string) {
    return sha.slice(0, 12);
  }
  function releaseEvidenceSummary(item: ReleaseEvidence) {
    if (item.git_trace_id) {
      const trace = data.git_traces?.find((candidate) => candidate.id === item.git_trace_id);
      if (trace)
        return `${shortSha(trace.commit_sha)} · ${trace.repository} · ${
          trace.kind === "push"
            ? `${trace.remote} ${trace.remote_ref}`
            : trace.branch
        }`;
    }
    if (item.verification_id)
      return `${item.issue_key} · verification ${item.verification_id}`;
    return `${item.outcome} · ${item.environment || "environment unknown"} · ${item.deployment_ref || "deployment"} · ${shortSha(item.commit_sha || "")} · ${item.source_ref ? `${item.source_ref} → ` : ""}${item.target_ref || "unknown ref"}${item.note ? ` · ${item.note}` : ""}`;
  }
  function issueCode(key: string) {
    const value = Number(key.split("-").at(-1));
    return Number.isFinite(value) ? value : Number.MAX_SAFE_INTEGER;
  }
  function projectPriority(priority = "medium") {
    return { urgent: 0, high: 1, medium: 2, low: 3 }[priority] ?? 2;
  }
  function projectProgress(id: string) {
    const provided = data.project_progress?.find(
      (progress) => progress.project_id === id,
    );
    if (provided) return provided;
    const issues = parents.filter((issue) => issue.project_id === id);
    const canceled = issues.filter((issue) => issue.status === "canceled").length;
    const completed = issues.filter((issue) => issue.status === "done").length;
    const legacy_completed = issues.filter(
      (issue) => issue.status === "legacy_completed",
    ).length;
    const eligible = issues.length - canceled - legacy_completed;
    return {
      project_id: id,
      total: issues.length,
      backlog: issues.filter((issue) => issue.status === "backlog").length,
      active: issues.filter((issue) => ["ready", "doing"].includes(issue.status))
        .length,
      pending_verification: issues.filter((issue) => issue.status === "verify")
        .length,
      completed,
      legacy_completed,
      canceled,
      completion_percent: eligible ? Math.floor((completed * 100) / eligible) : 0,
    };
  }
  function goalProgress(id: string) {
    return data.goal_progress.find((progress) => progress.goal_id === id);
  }
  function milestoneProgress(id: string) {
    return data.milestone_progress.find(
      (progress) => progress.milestone_id === id,
    );
  }
  function releaseProgress(id: string) {
    return data.release_progress.find((progress) => progress.release_id === id);
  }
  function workflowForProductKey(key: string) {
    const product = data.products.find((candidate) => candidate.key === key);
    return data.release_workflows.find((workflow) => workflow.product_id === product?.id);
  }
  async function linkGuidance(event: SubmitEvent) {
    event.preventDefault();
    if (!current || !guidanceDocumentId) return;
    if (
      await act({
        op: "link_theoria",
        key: current.key,
        expected_version: current.version,
        document_id: guidanceDocumentId,
        playbook_version: guidanceVersion.trim() || null,
      })
    ) {
      guidanceDocumentId = "";
      guidanceVersion = "";
    }
  }
  async function createFinding(event: SubmitEvent) {
    event.preventDefault();
    if (!current) return;
    if (
      await act({
        op: "create_method_finding",
        key: current.key,
        expected_version: current.version,
        classification: finding.classification,
        observation: finding.observation,
        hypothesis: finding.hypothesis,
        proposal: finding.proposal,
        evidence: [
          {
            kind: finding.evidenceKind,
            reference: finding.evidenceReference,
            summary: finding.evidenceSummary,
          },
        ],
      })
    ) {
      finding = {
        classification: "method_friction",
        observation: "",
        hypothesis: "",
        proposal: "",
        evidenceKind: "check",
        evidenceReference: "",
        evidenceSummary: "",
      };
    }
  }
  async function createRelation(event: SubmitEvent) {
    event.preventDefault();
    if (!current || !relationTarget) return;
    if (
      await act({
        op: "create_issue_link",
        key: current.key,
        expected_version: current.version,
        target_key: relationTarget,
        kind: relationKind,
      })
    ) {
      relationTarget = "";
      relationKind = "related";
    }
  }
  function relationLabel(
    kind: IssueLinkKind,
    direction: "incoming" | "outgoing",
  ) {
    if (kind === "parent")
      return direction === "outgoing" ? "Parent" : "Child";
    if (kind === "blocked_by")
      return direction === "outgoing" ? "Blocked by" : "Blocks";
    if (kind === "legacy_verification")
      return direction === "outgoing"
        ? "Legacy verification"
        : "Legacy verification source";
    return "Related";
  }
  // A refresh that has not finished by this deadline is abandoned (the read
  // and its body are aborted, and nothing it returns later is applied).
  const REFRESH_DEADLINE_MS = LOAD_DEADLINE_MS;
  // Reads can overlap (poll, post-save refresh); an older one that answers
  // late never replaces what a newer one already showed.
  let snapshotsStarted = 0;
  let snapshotShown = 0;
  let contextsStarted = 0;
  let contextShown = 0;
  async function loadContext(key = selected, signal?: AbortSignal) {
    if (!key) return;
    const started = ++contextsStarted;
    const result = await api<Context>({ op: "context", key }, false, "", signal);
    signal?.throwIfAborted();
    if (selected !== key || started < contextShown) return;
    contextShown = started;
    context = result;
    const run = result.verifications.find(
      (v) => v.id === result.issue.current_run,
    );
    if ((run?.id || "") !== activeRunId) {
      activeRunId = run?.id || "";
      results =
        run?.steps.map(() => ({ outcome: "pending" as Outcome, note: "" })) ||
        [];
      reviewNote = "";
    }
  }
  async function refresh(signal?: AbortSignal) {
    const started = ++snapshotsStarted;
    const snapshot = await api<Snapshot>({ op: "snapshot" }, false, "", signal);
    signal?.throwIfAborted();
    if (started < snapshotShown) return;
    snapshotShown = started;
    data = snapshot;
    // A guidance ID that is no longer cached stays selected and is shown as
    // missing rather than silently replaced by an unrelated document.
    if (!selectedGuidanceId)
      selectedGuidanceId = snapshot.theoria_documents[0]?.id || "";
    connected = true;
    notice = "";
    if (selected) await loadContext(selected, signal);
  }
  async function choose(key: string) {
    selected = key;
    context = null;
    tab = "brief";
    relationTarget = "";
    relationKind = "related";
    error = "";
    try {
      await loadContext(key);
    } catch (e) {
      error = String(e);
    }
  }
  function openGuidance(documentId: string) {
    if (current)
      returnTo = {
        key: current.key,
        view,
        product,
        projectFilter,
        labelFilter,
        search,
        listMode,
        sortMode,
        tab,
        list: captureAnchor(listPane),
        rowVisible: rowInView(),
        detail: captureAnchor(detailBody),
      };
    selectedGuidanceId = documentId;
    selected = "";
    context = null;
    view = "theoria";
  }
  async function returnFromGuidance() {
    const point = returnTo;
    if (!point) return;
    returnTo = null;
    view = point.view;
    product = point.product;
    projectFilter = point.projectFilter;
    labelFilter = point.labelFilter;
    search = point.search;
    listMode = point.listMode;
    sortMode = point.sortMode;
    if (!data.issues.some((issue) => issue.key === point.key)) {
      error = `${point.key} is no longer in this workspace.`;
      return;
    }
    await choose(point.key);
    if (selected !== point.key) return;
    tab = point.tab;
    await tick();
    restoreAnchor(listPane, point.list);
    // A resize can reflow the list; keep the issue in sight if it was.
    if (point.rowVisible && !rowInView())
      selectedRow()?.scrollIntoView({ block: "nearest" });
    restoreAnchor(detailBody, point.detail);
  }
  function selectedRow() {
    return listPane?.querySelector<HTMLElement>(".issue-row.selected");
  }
  function rowInView() {
    const row = selectedRow()?.getBoundingClientRect();
    const pane = listPane?.getBoundingClientRect();
    return !!row && !!pane && row.top >= pane.top && row.bottom <= pane.bottom;
  }
  $effect(() => {
    // The return point belongs to this one trip into Theoria.
    if (view !== "theoria" && returnTo) returnTo = null;
  });
  $effect(() => {
    if (modal === openedModal) return;
    openedModal = modal;
    formIntent = crypto.randomUUID();
    formAttempt = null;
    formConflict = null;
    // A send that belonged to the previous form no longer locks this one.
    formPending = false;
    if (!modal) formNotice = "";
  });
  $effect(() => {
    // The form is usually scrolled to its footer when a save fails; bring the
    // outcome into view and put focus on its action.
    if (!formAttempt && !formConflict) return;
    tick().then(() => {
      const card = document.querySelector<HTMLElement>("dialog.modal .form-outcome");
      card?.scrollIntoView({ block: "nearest" });
      card?.querySelector<HTMLButtonElement>("button.primary")?.focus();
    });
  });
  function failureText(e: unknown) {
    if (e instanceof DirectError && e.outcome === "unknown")
      return `${e.message} It may already be saved. Your text is kept; retrying sends the same request, so Direct applies it at most once.`;
    return String(e).replace(/^Error: /, "");
  }
  /**
   * Apply a write, then refresh within a deadline. Once the write is
   * confirmed it is reported as saved even if the refresh fails or hangs (it
   * is abandoned, never retried as a write); the poll reconnects and refreshes.
   */
  async function commit<T>(command: Record<string, unknown>, intent = "") {
    const result = await api<T>(command, true, intent);
    try {
      await refresh(AbortSignal.timeout(REFRESH_DEADLINE_MS));
    } catch {
      // The write itself was answered; the poll decides whether the service
      // is still reachable.
      const key = (result as { key?: unknown } | null)?.key;
      notice = `Saved${typeof key === "string" ? ` ${key}` : ""}. Direct couldn't refresh the view yet — reconnecting…`;
    }
    return result;
  }
  let consolidationAttempt: { owner: string; expected_cursor: number } | null = null;
  let consolidationOwner = $state<string | null>(null);
  async function consolidateOwners() {
    if (!consolidationOwner || formPending) return;
    const owner = consolidationAttempt?.owner || consolidationOwner;
    consolidationAttempt ??= { owner, expected_cursor: data.cursor };
    formPending = true;
    try {
      const result = await commit<{ changed_keys: string[] }>({
        op: "consolidate_human_owners", ...consolidationAttempt,
      });
      consolidationAttempt = null;
      consolidationOwner = null;
      notice = `${result.changed_keys.length} issues now use ${owner} as their human owner.`;
      modal = null;
    } catch (error) {
      if (error instanceof DirectError && error.outcome === "rejected") consolidationAttempt = null;
      formNotice = failureText(error);
    } finally {
      formPending = false;
    }
  }
  /**
   * Send the open form. A retry of an unconfirmed send resends the captured
   * command and intent verbatim; `built` (the form as it is now) is only
   * compared with it, so later edits are never submitted silently.
   */
  async function sendForm<T>(built: Record<string, unknown>) {
    const attempt = formAttempt ?? { command: built, intent: formIntent };
    // The reply belongs to this form only. A pending form cannot be closed,
    // but if it was replaced anyway its reply never touches the new form.
    const owner = formIntent;
    formSending = attempt;
    formPending = true;
    formNotice = "";
    try {
      let result: T;
      try {
        result = await commit<T>(attempt.command, attempt.intent);
      } catch (e) {
        if (formIntent !== owner) {
          keptNotice = `An earlier save ${e instanceof DirectError && e.outcome === "unknown" ? "was not confirmed" : `failed: ${failureText(e)}`}. Check the workspace before repeating it.`;
          throw new Superseded();
        }
        throw e;
      }
      if (formIntent !== owner) {
        keptNotice = "An earlier save was confirmed after its form was closed. Check the workspace for it.";
        throw new Superseded();
      }
      formAttempt = null;
      laterEdits = JSON.stringify(attempt.command) !== JSON.stringify(built);
      // Only the issue form can carry later changes forward as an edit; for
      // the others say plainly that they were not saved.
      if (laterEdits && modal !== "issue" && modal !== "edit")
        keptNotice = "Saved as first submitted. Changes made in the form after it was sent were not saved.";
      return result;
    } finally {
      if (formIntent === owner) formPending = false;
    }
  }
  /** A reply for a form that is no longer open; it changes nothing on screen. */
  class Superseded extends Error {}
  async function act(command: Record<string, unknown>, form = false) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      return form
        ? await sendForm<Issue>(command)
        : await commit<Issue>(command);
    } catch (e) {
      if (form) formFailure(e);
      else error = failureText(e);
      return undefined;
    } finally {
      busy = false;
    }
  }
  /** Keep the open form's entries; lock them while the outcome is unknown. */
  function formFailure(e: unknown) {
    if (e instanceof Superseded) return;
    if (e instanceof DirectError && e.outcome === "unknown") {
      formAttempt = formSending;
      error = "";
      return;
    }
    // A definitive answer: nothing was applied, so the form is editable again.
    formAttempt = null;
    error = failureText(e);
    if (
      e instanceof DirectError &&
      e.code === "conflict" &&
      (modal === "edit" || modal === "submit")
    ) {
      // Show what changed before offering the explicit reconcile action.
      const conflict = { kind: modal, latest: null as Issue | null };
      formConflict = conflict;
      latestIssue(modal === "edit" ? draft.key : current?.key || "")
        .then((latest) => {
          if (formConflict === conflict) formConflict = { ...conflict, latest };
        })
        .catch(() => undefined);
    }
  }
  function retryForm() {
    document
      .querySelector<HTMLFormElement>("dialog.modal form.modal-form")
      ?.requestSubmit();
  }
  function briefOf(i: Issue): Record<BriefField, string> {
    return {
      intake: i.intake ? JSON.stringify(i.intake) : "",
      title: i.title,
      body: i.body,
      acceptance: i.acceptance,
      owner: i.owner,
      priority: i.priority,
      planning_scope: i.planning_scope || (i.project_id ? "project" : "inbox"),
    };
  }
  async function latestIssue(key: string) {
    return (await api<Context>({ op: "context", key })).issue;
  }
  /**
   * Three-way merge of the edit form against the latest version: fields only
   * the other actor changed are adopted, fields only the owner changed are
   * kept, and fields both changed keep the owner's text with theirs shown.
   * Saving afterwards is a new, explicit request on the fresh version.
   */
  async function mergeLatest() {
    if (busy) return;
    busy = true;
    try {
      const latest = await latestIssue(draft.key);
      const theirs = briefOf(latest);
      const adopted: string[] = [];
      const both: { field: string; theirs: string }[] = [];
      for (const [field, label] of BRIEF_FIELDS) {
        const mine = String(draft[field]);
        if (theirs[field] === editBase[field] || theirs[field] === mine) continue;
        if (mine === editBase[field]) {
          draft[field] = theirs[field] as never;
          adopted.push(label);
        } else both.push({ field: label, theirs: theirs[field] });
      }
      draft.version = latest.version;
      editBase = theirs;
      error = "";
      formConflict = { kind: "merged", theirs: adopted, both };
    } catch (e) {
      error = failureText(e);
    } finally {
      busy = false;
    }
  }
  async function useCurrentVersion() {
    if (busy || !current) return;
    busy = true;
    try {
      const latest = await latestIssue(current.key);
      draft.version = latest.version;
      error = "";
      formConflict = null;
    } catch (e) {
      error = failureText(e);
    } finally {
      busy = false;
    }
  }
  async function postComment(key: string) {
    const issue = data.issues.find((i) => i.key === key);
    if (!issue || busy) return;
    // An unconfirmed post is retried verbatim (same version, same request
    // ID); anything else is a new post of this issue's own draft.
    const command = commentAttempts[key] ?? {
      op: "comment",
      key,
      expected_version: issue.version,
      body: commentDrafts[key] || "",
    };
    const body = String(command.body);
    if (!body.trim()) return;
    busy = true;
    postingKey = key;
    error = "";
    try {
      await commit(command, `comment:${key}`);
      delete commentAttempts[key];
      // Clear only this issue's draft, and only if it is still what was posted.
      if ((commentDrafts[key] || "") === body) delete commentDrafts[key];
    } catch (e) {
      if (e instanceof DirectError && e.outcome === "unknown")
        commentAttempts[key] = command;
      if (selected !== key || !(e instanceof DirectError && e.outcome === "unknown"))
        error =
          e instanceof DirectError && e.code === "conflict"
            ? `${key} changed while you were writing. Your comment is kept; post it again to add it to the current version.`
            : `Comment on ${key}: ${failureText(e)}`;
    } finally {
      busy = false;
      postingKey = "";
    }
  }
  function edit(i: Issue) {
    if (context?.issue.key === i.key) i = context.issue;
    editBase = briefOf(i);
    draft = {
      intake: i.intake ? JSON.stringify(i.intake) : "",
      title: i.title,
      body: i.body,
      acceptance: i.acceptance,
      owner: i.owner,
      priority: i.priority,
      key: i.key,
      version: i.version,
      product: "",
      planning_scope:
        i.planning_scope || (i.project_id ? "project" : "inbox"),
      project_id: i.project_id || "",
    };
    modal = "edit";
  }
  function newIssue() {
    const selectedProduct =
      data.products.find((p) => p.id === product) || data.products[0];
    const inheritedProject = selectedProject?.id || "";
    draft = {
      intake: "",
      title: "",
      body: "",
      acceptance: "",
      owner: ownerOptions[0] || "",
      priority: "medium",
      product: selectedProduct?.key || "DIR",
      planning_scope: "project",
      project_id: inheritedProject,
      key: "",
      version: 0,
    };
    issueTemplate = null;
    templateStarter = { brief: "", verification: "" };
    touched = { priority: false, planning_scope: false };
    formDefaults = { priority: draft.priority, planning_scope: draft.planning_scope };
    modal = "issue";
  }
  async function saveDraft(event: SubmitEvent) {
    event.preventDefault();
    const result =
      modal === "issue"
        ? await act(
            {
              op: "create_issue",
              product: draft.product,
              title: draft.title,
              body: draft.body,
              ...(draft.intake ? {intake: JSON.parse(draft.intake)} : {}),
              acceptance: draft.acceptance,
              owner: draft.owner,
              priority: draft.priority,
              planning_scope: draft.planning_scope,
              project_id: draft.project_id || null,
              ...(issueTemplate ? { template: issueTemplate } : {}),
            },
            true,
          )
        : await act(
            {
              op: "update_issue",
              key: draft.key,
              expected_version: draft.version,
              title: draft.title,
              body: draft.body,
              ...(draft.intake ? {intake: JSON.parse(draft.intake)} : {}),
              acceptance: draft.acceptance,
              owner: draft.owner,
              priority: draft.priority,
              planning_scope: draft.planning_scope,
            },
            true,
          );
    if (result && laterEdits) {
      // The confirmed record is what was first sent. Keep the changes made
      // after that as an unsaved edit of the same record for the owner to
      // save or discard; never resubmit them silently.
      editBase = briefOf(result);
      draft = { ...draft, key: result.key, version: result.version, product: "" };
      formIntent = crypto.randomUUID();
      modal = "edit";
      formNotice = `Saved ${result.key} as first submitted. Your later changes below are not saved — save them as an edit, or close to discard them.`;
      await choose(result.key);
      return;
    }
    if (result) {
      modal = null;
      // Intake can start from the template manager; show the new issue in the work list.
      if (view === "templates") view = "all";
      await choose(result.key);
    }
  }
  function makeReady(i: Issue) {
    const missing = [
      !i.body.trim() && "problem & expected outcome",
      !i.acceptance.trim() && "acceptance criteria",
      !i.owner.trim() && "human owner",
    ].filter(Boolean);
    if (missing.length) {
      edit(i);
      error = `Complete ${missing.join(", ")} before making this issue Ready.`;
      return;
    }
    return act({ op: "ready", key: i.key, expected_version: i.version });
  }
  async function deleteIssue(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    // An unconfirmed delete that did commit leaves nothing to retry.
    if (!current) {
      modal = null;
      return;
    }
    busy = true;
    error = "";
    try {
      await sendForm(
        {
          op: "delete_issue",
          key: current.key,
          expected_version: current.version,
        },
      );
      selected = "";
      context = null;
      modal = null;
    } catch (e) {
      formFailure(e);
      // The server refused under its own transaction; show the current reasons.
      if (e instanceof DirectError && e.outcome === "rejected")
        await refresh().catch(() => undefined);
    } finally {
      busy = false;
    }
  }
  async function createProduct(event: SubmitEvent) {
    event.preventDefault();
    if (await act({ op: "create_product", ...productDraft }, true)) {
      modal = null;
      productDraft = {
        key: "",
        name: "",
        repo_windows: "",
        repo_wsl: "",
        vault_windows: "",
        vault_wsl: "",
      };
    }
  }
  /**
   * Show a sidebar move at once, then store the complete arrangement. A
   * refusal (for example a product created elsewhere meanwhile) or an
   * unconfirmed write puts back what Direct actually holds.
   */
  async function arrange(next: SidebarGroup[] | null, focusId = "", moved = "") {
    if (!next || arranging || !connected) return;
    const payload = arrangement(next);
    const previous = { products: data.products, sections: data.product_sections || [] };
    const local = applyArrangement(previous.products, previous.sections, payload);
    data.products = local.products;
    data.product_sections = local.sections;
    arranging = true;
    error = "";
    if (focusId) {
      await tick();
      document.querySelector<HTMLElement>(`[data-sidebar-id="${focusId}"]`)?.focus();
    }
    try {
      await commit({ op: "arrange_products", ...payload });
      if (moved) sidebarStatus = moved;
    } catch (e) {
      data.products = previous.products;
      data.product_sections = previous.sections;
      error = `The sidebar order was not saved: ${failureText(e)}`;
      await refresh().catch(() => undefined);
    } finally {
      arranging = false;
    }
  }
  let sidebarStatus = $state("");
  function groupName(sectionId: string | null) {
    return (data.product_sections || []).find((s) => s.id === sectionId)?.name || "Ungrouped";
  }
  function placementText(next: SidebarGroup[], productId: string) {
    const group = next.find((g) => g.products.some((p) => p.id === productId));
    const index = group ? group.products.findIndex((p) => p.id === productId) : -1;
    const name = data.products.find((p) => p.id === productId)?.name || "Product";
    return `${name} moved to ${groupName(group?.section?.id ?? null)}, position ${index + 1} of ${group?.products.length ?? 0}.`;
  }
  function moveProductTo(productId: string, group: string | null, before: string | null) {
    const next = moveProduct(sidebar, productId, group, before);
    if (next) arrange(next, "", placementText(next, productId));
  }
  function productKey(event: KeyboardEvent, productId: string) {
    if (!event.altKey || (event.key !== "ArrowUp" && event.key !== "ArrowDown")) return;
    event.preventDefault();
    const next = nudgeProduct(sidebar, productId, event.key === "ArrowUp" ? -1 : 1);
    if (next) arrange(next, productId, placementText(next, productId));
  }
  function sectionKey(event: KeyboardEvent, sectionId: string) {
    if (!event.altKey || (event.key !== "ArrowUp" && event.key !== "ArrowDown")) return;
    event.preventDefault();
    const next = nudgeSection(sidebar, sectionId, event.key === "ArrowUp" ? -1 : 1);
    const index = next ? next.findIndex((g) => g.section?.id === sectionId) : -1;
    if (next)
      arrange(next, sectionId, `Section ${groupName(sectionId)} moved to position ${index} of ${next.length - 1}.`);
  }
  function startDrag(event: DragEvent, kind: Dragging["kind"], id: string) {
    if (!connected || arranging) {
      event.preventDefault();
      return;
    }
    dragging = { kind, id };
    event.dataTransfer?.setData("text/plain", id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
  }
  function endDrag() {
    dragging = null;
    dropAt = null;
    sectionDropBefore = undefined;
  }
  /** Upper half of an item drops before it; the lower half, after it. */
  function lowerHalf(event: DragEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    return event.clientY > rect.top + rect.height / 2;
  }
  function overProduct(event: DragEvent, group: SidebarGroup, index: number) {
    if (dragging?.kind !== "product") return;
    event.preventDefault();
    const after = lowerHalf(event);
    dropAt = {
      group: group.section?.id ?? null,
      before: after ? (group.products[index + 1]?.id ?? null) : group.products[index].id,
    };
  }
  function overGroupStart(event: DragEvent, group: SidebarGroup) {
    if (dragging?.kind !== "product") return;
    event.preventDefault();
    dropAt = { group: group.section?.id ?? null, before: group.products[0]?.id ?? null };
  }
  let sectionDropBefore = $state<string | null | undefined>(undefined);
  function overSection(event: DragEvent, index: number) {
    if (dragging?.kind === "product") return overGroupStart(event, sidebar[index]);
    if (dragging?.kind !== "section") return;
    event.preventDefault();
    sectionDropBefore = lowerHalf(event)
      ? (sidebar[index + 1]?.section?.id ?? null)
      : sidebar[index].section!.id;
  }
  function drop(event: DragEvent) {
    event.preventDefault();
    const current = dragging;
    const target = dropAt;
    const sectionTarget = sectionDropBefore;
    endDrag();
    if (current?.kind === "product" && target)
      moveProductTo(current.id, target.group, target.before);
    if (current?.kind === "section" && sectionTarget !== undefined) {
      const next = moveSection(sidebar, current.id, sectionTarget);
      if (next) arrange(next, "", `Section ${groupName(current.id)} moved.`);
    }
  }
  function editSection(section?: ProductSection) {
    sectionDraft = section
      ? {
          id: section.id,
          name: section.name,
          version: section.version,
          products: data.products.filter((p) => p.section_id === section.id).length,
          confirmDelete: false,
        }
      : { id: "", name: "", version: 0, products: 0, confirmDelete: false };
    modal = "section";
  }
  async function saveSection(event: SubmitEvent) {
    event.preventDefault();
    const command = sectionDraft.id
      ? {
          op: "update_product_section",
          id: sectionDraft.id,
          expected_version: sectionDraft.version,
          name: sectionDraft.name,
        }
      : { op: "create_product_section", name: sectionDraft.name };
    if (await act(command, true)) modal = null;
  }
  async function deleteSection() {
    if (!sectionDraft.confirmDelete) {
      sectionDraft.confirmDelete = true;
      return;
    }
    const command = {
      op: "delete_product_section",
      id: sectionDraft.id,
      expected_version: sectionDraft.version,
    };
    if (await act(command, true)) modal = null;
  }
  // Customer requests (DIR-24).
  let signalFocus = $state("");
  let signalScope = $state("all");
  function openSignals(focus = "") {
    // Open on the product chosen in the sidebar, if any.
    signalScope = product;
    signalFocus = focus;
    view = "signals";
    selected = "";
    context = null;
  }
  function openSignal(id: string) {
    openSignals(id);
  }
  async function openSignalIssue(key: string) {
    view = "all";
    product = "all";
    await choose(key);
  }
  function editProject(p?: Project) {
    projectDraft = p
      ? {
          id: p.id,
          product: data.products.find((product) => product.id === p.product_id)!
            .key,
          name: p.name,
          description: p.description,
          status: p.status || "active",
          priority: p.priority || "medium",
          sort_order: p.sort_order || 0,
          version: p.version,
        }
      : {
          id: "",
          product:
            data.products.find((p) => p.id === product)?.key ||
            data.products[0]?.key ||
            "DIR",
          name: "",
          description: "",
          status: "planned",
          priority: "medium",
          sort_order: 0,
          version: 0,
        };
    projectTemplate = null;
    templateStarter = { brief: "", verification: "" };
    touched = { priority: false, planning_scope: false };
    formDefaults = { priority: projectDraft.priority, planning_scope: "project" };
    modal = "project";
  }
  async function saveProject(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      const saved = await sendForm<Project>(
        projectDraft.id
          ? {
              op: "update_project",
              id: projectDraft.id,
              expected_version: projectDraft.version,
              name: projectDraft.name,
              description: projectDraft.description,
              status: projectDraft.status,
              priority: projectDraft.priority,
              sort_order: projectDraft.sort_order,
            }
          : {
              op: "create_project",
              product: projectDraft.product,
              name: projectDraft.name,
              description: projectDraft.description,
              priority: projectDraft.priority,
              sort_order: projectDraft.sort_order,
              ...(projectTemplate ? { template: projectTemplate } : {}),
            },
      );
      product = saved.product_id;
      projectFilter = saved.id;
      if (view === "templates") view = "all";
      modal = null;
    } catch (e) {
      formFailure(e);
    } finally {
      busy = false;
    }
  }
  function editLabel(label?: Label) {
    labelDraft = {
      id: label?.id || "",
      name: label?.name || "",
      description: label?.description || "",
      color: label?.color || "",
      aliases: (label?.aliases || []).join(", "),
      products: data.products.map((p) => {
        const rule = label?.products.find((rule) => rule.product_id === p.id);
        return {
          product_id: p.id,
          name: p.name,
          applies: !!rule,
          default_for_new_issues: rule?.default_for_new_issues || false,
        };
      }),
      linear_origins: (label?.linear_origins || [])
        .map((origin) => (origin.name ? `${origin.id} | ${origin.name}` : origin.id))
        .join("\n"),
      version: label?.version || 0,
    };
    modal = "label";
  }
  async function saveLabel(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    const fields = {
      name: labelDraft.name,
      description: labelDraft.description,
      color: labelDraft.color.trim(),
      aliases: labelDraft.aliases
        .split(",")
        .map((alias) => alias.trim())
        .filter(Boolean),
      products: labelDraft.products
        .filter((rule) => rule.applies)
        .map((rule) => ({
          product_id: rule.product_id,
          default_for_new_issues: rule.default_for_new_issues,
        })),
      linear_origins: labelDraft.linear_origins
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          const [id, ...rest] = line.split("|");
          return { id: id.trim(), name: rest.join("|").trim() };
        }),
    };
    try {
      const saved = await sendForm<Label>(
        labelDraft.id
          ? {
              op: "update_label",
              id: labelDraft.id,
              expected_version: labelDraft.version,
              ...fields,
            }
          : { op: "create_label", ...fields },
      );
      labelFilter = saved.id;
      modal = null;
    } catch (e) {
      formFailure(e);
    } finally {
      busy = false;
    }
  }
  function editGoal(goal?: Goal) {
    const productKey = goal
      ? data.products.find((product) => product.id === goal.product_id)?.key
      : data.products.find((candidate) => candidate.id === product)?.key ||
        data.products[0]?.key;
    goalDraft = goal
      ? {
          id: goal.id,
          product: productKey || "DIR",
          name: goal.name,
          description: goal.description,
          status: goal.status,
          priority: goal.priority,
          project_ids: [...goal.project_ids],
          version: goal.version,
        }
      : {
          id: "",
          product: productKey || "DIR",
          name: "",
          description: "",
          status: "planned",
          priority: "medium",
          project_ids: selectedProject ? [selectedProject.id] : [],
          version: 0,
        };
    modal = "goal";
  }
  async function saveGoal(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await sendForm<Goal>(
        goalDraft.id
          ? {
              op: "update_goal",
              id: goalDraft.id,
              expected_version: goalDraft.version,
              name: goalDraft.name,
              description: goalDraft.description,
              status: goalDraft.status,
              priority: goalDraft.priority,
              project_ids: goalDraft.project_ids,
            }
          : {
              op: "create_goal",
              product: goalDraft.product,
              name: goalDraft.name,
              description: goalDraft.description,
              priority: goalDraft.priority,
              project_ids: goalDraft.project_ids,
            },
      );
      modal = null;
    } catch (e) {
      formFailure(e);
    } finally {
      busy = false;
    }
  }
  function editMilestone(milestone?: Milestone) {
    if (!selectedProject && !milestone) return;
    milestoneDraft = milestone
      ? {
          id: milestone.id,
          project_id: milestone.project_id,
          name: milestone.name,
          description: milestone.description,
          sort_order: milestone.sort_order,
          version: milestone.version,
        }
      : {
          id: "",
          project_id: selectedProject!.id,
          name: "",
          description: "",
          sort_order: selectedProjectMilestones.length,
          version: 0,
        };
    modal = "milestone";
  }
  async function saveMilestone(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await sendForm<Milestone>(
        milestoneDraft.id
          ? {
              op: "update_milestone",
              id: milestoneDraft.id,
              expected_version: milestoneDraft.version,
              name: milestoneDraft.name,
              description: milestoneDraft.description,
              sort_order: milestoneDraft.sort_order,
            }
          : {
              op: "create_milestone",
              project_id: milestoneDraft.project_id,
              name: milestoneDraft.name,
              description: milestoneDraft.description,
              sort_order: milestoneDraft.sort_order,
            },
      );
      modal = null;
    } catch (e) {
      formFailure(e);
    } finally {
      busy = false;
    }
  }
  function editRelease(release?: ReleaseRecord) {
    const productKey = release
      ? data.products.find((product) => product.id === release.product_id)?.key
      : data.products.find((candidate) => candidate.id === product)?.key ||
        data.products[0]?.key;
    const workflow = workflowForProductKey(productKey || "DIR");
    releaseDraft = release
      ? {
          id: release.id,
          product: productKey || "DIR",
          name: release.name,
          version_label: release.version_label,
          status: release.status,
          target_ref: release.target_ref,
          release_branch: release.release_branch || "",
          preview_url: release.preview_url || "",
          notes: release.notes,
          project_ids: [...release.project_ids],
          issue_keys: [...release.issue_keys],
          version: release.version,
        }
      : {
          id: "",
          product: productKey || "DIR",
          name: "",
          version_label: "",
          status: "planned",
          target_ref: workflow?.production_ref || "refs/heads/main",
          release_branch: "",
          preview_url: "",
          notes: "",
          project_ids: selectedProject ? [selectedProject.id] : [],
          issue_keys: current ? [current.key] : [],
          version: 0,
        };
    modal = "release";
  }
  async function saveRelease(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await sendForm<ReleaseRecord>(
        releaseDraft.id
          ? {
              op: "update_release",
              id: releaseDraft.id,
              expected_version: releaseDraft.version,
              name: releaseDraft.name,
              version_label: releaseDraft.version_label,
              status: releaseDraft.status,
              target_ref: releaseDraft.target_ref,
              release_branch: releaseDraft.release_branch.trim() || null,
              preview_url: releaseDraft.preview_url.trim() || null,
              notes: releaseDraft.notes,
              project_ids: releaseDraft.project_ids,
              issue_keys: releaseDraft.issue_keys,
            }
          : {
              op: "create_release",
              product: releaseDraft.product,
              name: releaseDraft.name,
              version_label: releaseDraft.version_label,
              target_ref: releaseDraft.target_ref,
              release_branch: releaseDraft.release_branch.trim() || null,
              preview_url: releaseDraft.preview_url.trim() || null,
              notes: releaseDraft.notes,
              project_ids: releaseDraft.project_ids,
              issue_keys: releaseDraft.issue_keys,
            },
      );
      modal = null;
    } catch (e) {
      formFailure(e);
    } finally {
      busy = false;
    }
  }
  function editWorkflow() {
    const productKey =
      data.products.find((candidate) => candidate.id === product)?.key ||
      data.products[0]?.key ||
      "DIR";
    const workflow = workflowForProductKey(productKey);
    workflowDraft = workflow
      ? {
          product: productKey,
          expected_version: workflow.version,
          branch_strategy: workflow.branch_strategy,
          production_ref: workflow.production_ref,
          release_branch_pattern: workflow.release_branch_pattern,
          preview_environment: workflow.preview_environment,
          preview_url_template: workflow.preview_url_template,
          promotion_policy: workflow.promotion_policy,
        }
      : {
          product: productKey,
          expected_version: null,
          branch_strategy: "one_branch_per_release",
          production_ref: "refs/heads/main",
          release_branch_pattern: "refs/heads/release/{version}",
          preview_environment: "preview",
          preview_url_template: "",
          promotion_policy: "verified_owner_approval",
        };
    modal = "workflow";
  }
  function editProductPaths() {
    const p = data.products.find((candidate) => candidate.id === product);
    if (!p) return;
    pathsDraft = {
      product: p.key,
      name: p.name,
      repo_windows: p.repo_windows,
      repo_wsl: p.repo_wsl,
      vault_windows: p.vault_windows,
      vault_wsl: p.vault_wsl,
    };
    modal = "productPaths";
  }
  async function saveProductPaths(event: SubmitEvent) {
    event.preventDefault();
    const { name: _name, ...paths } = pathsDraft;
    if (await act({ op: "update_product_paths", ...paths }, true)) modal = null;
  }
  async function saveWorkflow(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await sendForm(
        {
          op: "set_release_workflow_config",
          ...workflowDraft,
        },
      );
      modal = null;
    } catch (e) {
      formFailure(e);
    } finally {
      busy = false;
    }
  }
  async function toggleIssueLabel(issue: Issue, labelId: string, attach: boolean) {
    if (!labelId) return;
    await act({
      op: attach ? "attach_issue_label" : "detach_issue_label",
      key: issue.key,
      expected_version: issue.version,
      label_id: labelId,
    });
  }
  async function toggleProjectLabel(
    project: Project,
    labelId: string,
    attach: boolean,
  ) {
    if (!labelId) return;
    await act({
      op: attach ? "attach_project_label" : "detach_project_label",
      id: project.id,
      expected_version: project.version,
      label_id: labelId,
    });
  }
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!current) return;
    if (
      await act(
        {
          op: "submit",
          key: current.key,
          expected_version: draft.version,
          ...handoff,
          e2e: { ...e2eDraft, build_ref: handoff.build_ref, outcome: "passed" },
        },
        true,
      )
    ) {
      modal = null;
      tab = "verify";
    }
  }
  async function review(outcome: Outcome) {
    if (current && activeRun)
      await act({
        op: "review",
        key: current.key,
        expected_version: current.version,
        run_id: activeRun.id,
        outcome,
        results,
        note: reviewNote,
      });
  }
  // Imported sources: lazy, scoped reads. Source text is untrusted data and is
  // only ever rendered as escaped text or downloaded as an opaque file.
  let sourceBundles = $state<SourceBundleDetail[]>([]);
  let sourceQuery = $state("");
  let sourceKind = $state("");
  let sourceClass = $state("");
  let sourceIssue = $state("");
  let sourceResults = $state<SourceRecordSummary[]>([]);
  let sourceTotal = $state(0);
  let sourceRecord = $state<SourceRecordView | null>(null);
  let sourceBusy = $state(false);
  let migrationFile = $state<File | null>(null);
  let migrationBytes = $state<ArrayBuffer | null>(null);
  let migrationPreview = $state<MigrationPreview | null>(null);
  let migrationResult = $state<Record<string, any> | null>(null);
  let sourceKinds = $derived(
    [...new Set(sourceBundles.flatMap((bundle) => Object.keys(bundle.records_by_kind)))].sort(),
  );
  async function loadSources() {
    try {
      const described = await api<{ bundles: SourceBundleDetail[] }>({
        op: "source_bundles",
      });
      sourceBundles = described.bundles;
      await searchSources(true);
    } catch (e) {
      error = String(e);
    }
  }
  async function searchSources(reset = false) {
    sourceBusy = true;
    try {
      const page = await api<{ total: number; records: SourceRecordSummary[] }>({
        op: "search_sources",
        query: sourceQuery,
        kind: sourceKind || null,
        classification: sourceClass || null,
        issue_key: sourceIssue || null,
        limit: 50,
        offset: reset ? 0 : sourceResults.length,
      });
      sourceResults = reset ? page.records : [...sourceResults, ...page.records];
      sourceTotal = page.total;
    } catch (e) {
      error = String(e);
    } finally {
      sourceBusy = false;
    }
  }
  async function openSource(id: string) {
    try {
      sourceRecord = await api<SourceRecordView>({ op: "source_record", id });
    } catch (e) {
      error = String(e);
    }
  }
  async function openSources(issueKey = "", recordId = "") {
    view = "sources";
    product = "all";
    sourceIssue = issueKey;
    sourceRecord = null;
    await loadSources();
    if (recordId) await openSource(recordId);
  }
  async function downloadSource(bundleId: string, path: string, name?: string | null) {
    try {
      const blob = await sourceFile(bundleId, path);
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = (name || path.split("/").pop() || "retained-file").replace(/[^A-Za-z0-9._-]/g, "_");
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (e) {
      error = String(e);
    }
  }
  async function previewMigration() {
    const file = migrationFile;
    if (!file) return;
    busy = true;
    error = "";
    migrationResult = null;
    migrationPreview = null;
    migrationBytes = null;
    try {
      const bytes = await file.arrayBuffer();
      const preview = await migration<MigrationPreview>(bytes);
      // Only the file still selected may supply the bytes and preview that
      // Apply sends; a result for an earlier selection is discarded.
      if (migrationFile === file) {
        migrationBytes = bytes;
        migrationPreview = preview;
      }
    } catch (e) {
      if (migrationFile === file) error = String(e);
    } finally {
      busy = false;
    }
  }
  async function applyMigration() {
    if (!migrationBytes || !migrationPreview) return;
    if (
      !confirm(
        `Apply this migration to your workspace?\n\n${migrationPreview.counts.issues} issues, ${migrationPreview.counts.retained_records} retained source records. Direct writes a pre-import backup first and applies everything in one transaction.`,
      )
    )
      return;
    busy = true;
    error = "";
    try {
      migrationResult = await migration<Record<string, any>>(migrationBytes, {
        cursor: migrationPreview.expected_cursor,
        sha256: migrationPreview.artifact_sha256,
      });
      migrationPreview = null;
      migrationBytes = null;
      await refresh();
      await loadSources();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function rollbackMigration(bundle: SourceBundleDetail) {
    if (
      !confirm(
        "Roll back this migration? Every record it added and its retained sources are removed. This is only possible while nothing has changed since the import.",
      )
    )
      return;
    await act({
      op: "rollback_migration",
      bundle_id: bundle.id,
      expected_cursor: data.cursor,
    });
    sourceRecord = null;
    await loadSources();
  }
  function bytes(value: number) {
    return value >= 1 << 20
      ? `${(value / (1 << 20)).toFixed(1)} MB`
      : value >= 1024
        ? `${(value / 1024).toFixed(1)} KB`
        : `${value} B`;
  }
  function pretty(content: string) {
    try {
      return JSON.stringify(JSON.parse(content), null, 2);
    } catch {
      return content;
    }
  }
  async function exportData() {
    try {
      const archive = await api({ op: "export" });
      const url = URL.createObjectURL(
        new Blob([JSON.stringify(archive, null, 2)], {
          type: "application/json",
        }),
      );
      const a = document.createElement("a");
      a.href = url;
      a.download = `direct-backup-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (e) {
      error = String(e);
    }
  }
  onMount(() => {
    function shortcut(e: KeyboardEvent) {
      if (
        e.altKey &&
        e.key === "ArrowLeft" &&
        returnTo &&
        view === "theoria" &&
        !modal
      ) {
        e.preventDefault();
        returnFromGuidance();
        return;
      }
      if (
        e.key.toLowerCase() === "n" &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        !modal &&
        connected &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement) &&
        !(e.target instanceof HTMLSelectElement)
      ) {
        e.preventDefault();
        newIssue();
      }
    }
    window.addEventListener("keydown", shortcut);
    const measure = () => (viewport = window.innerWidth);
    window.addEventListener("resize", measure);
    async function poll() {
      clock = Date.now() / 1000;
      try {
        const changes = await api<{ cursor: number }>({
          op: "changes",
          after: data.cursor,
        });
        // The service answered, so it is connected even if the refresh below
        // is slow; a refresh that misses its deadline keeps the notice and is
        // tried again on the next tick.
        connected = true;
        if (changes.cursor !== data.cursor || notice)
          await refresh(AbortSignal.timeout(REFRESH_DEADLINE_MS)).catch(() => undefined);
        else if (
          current?.claim &&
          current.claim.expires_at <= clock &&
          deletion?.blockers.some((blocker) => blocker.kind === "active_claim")
        ) {
          // Lease expiry emits no event. Ask the server again rather than
          // leaving a cached active-claim blocker on screen indefinitely.
          await loadContext();
        }
        connected = true;
      } catch (e) {
        connected = false;
        if (e instanceof SessionExpired) throw e;
      }
    }
    // The first load retries by itself until it succeeds, each attempt within
    // a deadline; then the steady poll takes over.
    const connection = startConnection({
      async load(signal) {
        await connect(signal);
        await refresh(signal);
      },
      poll,
      state(state) {
        load = state;
        if (state.kind !== "loaded") connected = false;
      },
    });
    retryLoad = connection.retryNow;
    return () => {
      connection.stop();
      window.removeEventListener("keydown", shortcut);
      window.removeEventListener("resize", measure);
    };
  });
</script>

{#snippet labelChips(ids: string[] | undefined, remove?: (id: string) => void)}
  {#each ids || [] as id}
    {@const label = labelOf(id)}
    {#if label}<span
        class="label-chip"
        title={label.description || label.name}
        style={label.color ? `--label-color: ${label.color}` : ""}
        ><i></i><span class="label-name">{label.name}</span>{#if remove}<button
            type="button"
            aria-label={`Remove label ${label.name}`}
            disabled={busy || !connected}
            onclick={(event) => {
              event.stopPropagation();
              remove(id);
            }}>×</button
          >{/if}</span
      >{/if}
  {/each}
{/snippet}

{#snippet labelPicker(
  productId: string,
  attached: string[] | undefined,
  attach: (id: string) => void,
)}
  {@const choices = allLabels.filter(
    (label) => labelApplies(label, productId) && !(attached || []).includes(label.id),
  )}
  {#if choices.length}<select
      class="label-picker"
      aria-label="Add label"
      value=""
      disabled={busy || !connected}
      onchange={(event) => {
        const control = event.currentTarget;
        attach(control.value);
        control.value = "";
      }}
    >
      <option value="" disabled>＋ Add label…</option>
      {#each choices as label}<option value={label.id}>{label.name}</option>{/each}
    </select>{/if}
{/snippet}

{#snippet issueRow(i: Issue)}
  <button
    class="issue-row"
    class:selected={selected === i.key}
    onclick={() => choose(i.key)}
    ><span class="state-icon {i.status}">{glyphs[i.status]}</span>
    <div class="row-content">
      <div class="issue-title">{i.title}</div>
      <div class="issue-meta">
        <span>{i.key}</span><span class="dot-separator">·</span><span
          >{data.products.find((p) => p.id === i.product_id)?.name}</span
        >{#if i.needs_fix}<span class="fix-badge">Needs fix</span>{/if}{#if i.claim}<span
            class="claim-meta">↗ {i.claim.actor}</span
          >{/if}
        {#if i.project_id}<span class="project-tag"
            >{data.projects.find((p) => p.id === i.project_id)?.name}</span
          >{:else}<span class="inbox-tag">Inbox</span>{/if}
        {@render labelChips(i.labels)}
      </div>
    </div>
    <span class="status-badge {i.status}">{i.status === "verify" && !data.review_ready_runs?.includes(i.current_run || "") ? "Agent E2E needed" : labels[i.status]}</span></button
  >
{/snippet}

{#snippet addProductButton()}
  <button
    class="icon-button add-section"
    aria-label="Add sidebar section"
    title="Add a section to group products"
    onclick={() => editSection()}
    disabled={!connected}>⊞</button
  ><button
    class="icon-button"
    aria-label="Add product"
    onclick={() => (modal = "product")}
    disabled={!connected}>＋</button
  >
{/snippet}

{#snippet sectionToggle(section: Section, name: string, extra?: Snippet)}
  <div class="nav-label section-row" class:products-label={section !== "praxis"}>
    <button
      class="section-toggle"
      aria-expanded={!layout.collapsed[section]}
      onclick={() => toggleSection(section)}
      ><span class="section-caret" aria-hidden="true"
        >{layout.collapsed[section] ? "▸" : "▾"}</span
      >{name}</button
    >{#if extra}{@render extra()}{/if}
  </div>
{/snippet}

{#snippet detailControls()}
  <button
    class="icon-button pane-expand"
    aria-label={layout.rightExpanded ? "Restore detail panel width" : "Expand detail panel"}
    title={layout.rightExpanded ? "Restore previous width" : "Expand for reading"}
    onclick={toggleDetailExpanded}>{layout.rightExpanded ? "⇥" : "⇤"}</button
  >
{/snippet}

<svelte:head><title>Direct · {title}</title></svelte:head>

<div class="shell" class:resizing={!!resizing}>
  <aside
    class="sidebar"
    class:collapsed={layout.leftCollapsed}
    style:width={`${leftWidth}px`}
    aria-label="Sidebar"
  >
    <div class="sidebar-top">
      <div class="brand-row">
        <a class="brand" href="/" onclick={(e) => e.preventDefault()}
          ><span class="brand-mark">↗</span> Direct
          <span class="version">LOCAL</span></a
        ><button
          class="icon-button rail-toggle"
          aria-label={layout.leftCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          title={layout.leftCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          onclick={toggleSidebar}>{layout.leftCollapsed ? "»" : "«"}</button
        >
      </div>
      <div class="workspace-label">
        <span class="avatar">Y</span>
        <div>Your workspace<small>Human + agents</small></div>
        <span class="tiny">⌄</span>
      </div>
      <button class="compose" title="New issue (N)" onclick={newIssue} disabled={!connected}
        ><span>＋</span> New issue <kbd>N</kbd></button
      >
    </div>
    <div class="sidebar-scroll">
      {@render sectionToggle("praxis", "PRAXIS")}
      {#if sectionOpen("praxis")}<nav aria-label="Praxis">
        <button
          title="All work"
          class:active={view === "all" && product === "all"}
          onclick={() => {
            view = "all";
            product = "all";
          }}><span>▤</span> All work <small>{parents.length}</small></button
        >
        <button
          title="Needs me"
          class:active={view === "needs" && product === "all"}
          onclick={() => {
            view = "needs";
            product = "all";
          }}
          ><span>◈</span> Needs me
          <small class:highlight={attention > 0}>{attention}</small></button
        >
        <button
          title="Inbox"
          class:active={view === "backlog" && product === "all"}
          onclick={() => {
            view = "backlog";
            product = "all";
          }}
          ><span>▧</span> Inbox
          <small>{parents.filter((i) => i.status === "backlog").length}</small
          ></button
        >
        <button
          title="Ready & doing"
          class:active={view === "active" && product === "all"}
          onclick={() => {
            view = "active";
            product = "all";
          }}><span>◐</span> Ready & doing</button
        >
        <button
          title="Completed"
          class:active={view === "done" && product === "all"}
          onclick={() => {
            view = "done";
            product = "all";
          }}><span>✓</span> Completed</button
        >
        <button
          title="Intake templates"
          class:active={view === "templates"}
          onclick={() => {
            view = "templates";
            product = "all";
            selected = "";
            context = null;
          }}><span>▦</span> Intake templates
          <small>{(data.templates || []).filter((t) => t.status === "active").length}</small></button
        >
        <button
          title="Customer requests"
          class:active={view === "signals"}
          onclick={() => openSignals()}
          ><span>◌</span> Customer requests
          <small>{(data.customer_signals || []).filter((s) => !s.archived).length}</small></button
        >
      </nav>{/if}
      {@render sectionToggle("theoria", "THEORIA")}
      {#if sectionOpen("theoria")}<nav aria-label="Theoria">
        <button
          title="Guidance & findings"
          class:active={view === "theoria"}
          onclick={() => {
            returnTo = null;
            view = "theoria";
            product = "all";
            selected = "";
            context = null;
            if (!selectedGuidanceId)
              selectedGuidanceId = data.theoria_documents[0]?.id || "";
          }}><span>◫</span> Guidance & findings <small>{data.method_findings.length}</small
          ></button
        >
      </nav>{/if}
      {@render sectionToggle("sources", "SOURCES")}
      {#if sectionOpen("sources")}<nav aria-label="Sources">
        <button
          title="Imported sources"
          class:active={view === "sources"}
          onclick={() => openSources()}
          ><span>⧉</span> Imported sources
          <small>{data.source_bundles?.length || 0}</small></button
        >
      </nav>{/if}
      {@render sectionToggle("products", "PRODUCTS", addProductButton)}
      {#if sectionOpen("products")}<nav
          aria-label="Products"
          aria-describedby="product-order-help"
          class="product-nav"
          class:dragging={!!dragging}
        >
        <span id="product-order-help" class="visually-hidden"
          >Drag products or sections to reorder them, or press Alt with the up or down arrow.</span
        >
        {#each sidebar as group, groupIndex (group.section?.id ?? "")}
          {@const groupId = group.section?.id ?? null}
          {#if group.section}
            {@const section = group.section}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="product-section-head"
              class:drop-before={dragging?.kind === "section" && sectionDropBefore === section.id}
              class:drop-into={dragging?.kind === "product" && dropAt?.group === section.id && dropAt.before === (group.products[0]?.id ?? null)}
              ondragover={(event) => overSection(event, groupIndex)}
              ondrop={drop}
            >
              <button
                class="product-section-name"
                data-sidebar-id={section.id}
                draggable={connected}
                title={`${section.name} · drag or Alt+↑/↓ to reorder sections`}
                aria-label={`Section ${section.name}, ${group.products.length} ${group.products.length === 1 ? "product" : "products"}`}
                ondragstart={(event) => startDrag(event, "section", section.id)}
                ondragend={endDrag}
                onkeydown={(event) => sectionKey(event, section.id)}
                ><span class="nav-text">{section.name}</span></button
              ><button
                class="icon-button section-edit"
                aria-label={`Rename or delete section ${section.name}`}
                title="Rename or delete section"
                disabled={!connected}
                onclick={() => editSection(section)}>✎</button
              >
            </div>
          {/if}
          {#each group.products as p, index (p.id)}<button
              title={`${p.name} · drag or Alt+↑/↓ to reorder`}
              class:active={product === p.id}
              class:grouped={!!group.section}
              class:drop-before={dragging?.kind === "product" && dropAt?.group === groupId && dropAt.before === p.id}
              class:drop-after={dragging?.kind === "product" && dropAt?.group === groupId && dropAt.before === null && index === group.products.length - 1}
              class:being-dragged={dragging?.id === p.id}
              data-sidebar-id={p.id}
              draggable={connected}
              ondragstart={(event) => startDrag(event, "product", p.id)}
              ondragend={endDrag}
              ondragover={(event) => overProduct(event, group, index)}
              ondrop={drop}
              onkeydown={(event) => productKey(event, p.id)}
              onclick={() => {
                product = p.id;
                view = "all";
              }}
              ><span class="product-icon">{p.key.slice(0, 1)}</span><span class="nav-text"
                >{p.name}</span
              ><small>{parents.filter((i) => i.product_id === p.id).length}</small></button
            >{/each}
          {#if group.products.length === 0 && (group.section || dragging?.kind === "product")}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="product-drop-zone"
              class:drop-into={dragging?.kind === "product" && dropAt?.group === groupId}
              ondragover={(event) => overGroupStart(event, group)}
              ondrop={drop}
            >
              <span class="nav-text">{group.section ? "Drag products here" : "Drop here to ungroup"}</span>
            </div>
          {/if}
        {/each}
        <span class="visually-hidden" aria-live="polite">{sidebarStatus}</span>
      </nav>{/if}
    </div>
    <div class="sidebar-bottom">
      <div class="local-note" title={connected ? "Connected locally" : "Service disconnected"}>
        <span class:online={connected} class="connection-dot"></span>
        <div>
          {connected
            ? "Connected locally"
            : load.kind === "session_expired"
              ? "Browser session ended"
              : "Service disconnected"}<small
            >{connected
              ? "Your work stays on this device"
              : load.kind === "session_expired"
                ? "Open a fresh launch link"
                : "Reconnecting automatically"}</small
          >
        </div>
      </div>
      <button class="backup-button" title="Export workspace" onclick={exportData} disabled={!connected}
        ><span aria-hidden="true">↧</span> <span class="nav-text">Export workspace</span></button
      >
      <div class="foundation">Direct · Foundation preview</div>
    </div>
  </aside>
  <!-- A focusable separator with a value is an ARIA window-splitter widget. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  {#if !layout.leftCollapsed}<div
      class="pane-resizer left"
      role="separator"
      aria-label="Resize sidebar"
      aria-orientation="vertical"
      aria-valuemin={LEFT_MIN}
      aria-valuemax={leftMax}
      aria-valuenow={leftWidth}
      tabindex="0"
      title="Drag to resize · double-click to reset · Enter to collapse"
      onpointerdown={(event) => startResize("left", event)}
      onpointermove={moveResize}
      onpointerup={endResize}
      onpointercancel={endResize}
      ondblclick={() => resetPane("left")}
      onkeydown={(event) => resizeKey("left", event)}
    ></div>{/if}

  <main>
    <header class="topbar">
      <div>
        <span class="breadcrumb">Workspace</span><span class="slash">/</span
        >{title}
      </div>
      <div class="top-right">
        <span class="privacy">◉ Local workspace</span><span class="avatar small"
          >Y</span
        >
      </div>
    </header>
    {#if load.kind === "retrying"}<div class="notice" role="status">
        <span
          >Can't load the workspace yet. {load.reason} Retrying automatically (attempt
          {load.attempt}).</span
        ><button class="secondary" onclick={() => retryLoad()}>Retry now</button>
      </div>{:else if load.kind === "session_expired"}<div class="error" role="alert">
        <span>{load.message}</span>
      </div>{/if}
    {#if error}<div class="error" role="alert">
        <span>{error}</span><button
          class="icon-button"
          aria-label="Dismiss error"
          onclick={() => (error = "")}>×</button
        >
      </div>{/if}{#if notice}<div class="notice" role="status">
        <span>{notice}</span>
      </div>{/if}{#if keptNotice}<div class="notice" role="status">
        <span>{keptNotice}</span><button
          class="icon-button"
          aria-label="Dismiss notice"
          onclick={() => (keptNotice = "")}>×</button
        >
      </div>{/if}
    <div
      class="work-area"
      style:--detail-width={`${rightWidth}px`}
      style:--list-min={`${listMin}px`}
    >
      {#if view === "theoria"}
        <section class="list-panel theoria-list">
          <div class="page-heading">
            <div class="eyebrow">WHY · HOW · LEARN</div>
            <div class="heading-row">
              <h1>Theoria</h1>
              <span class="count-pill">{data.theoria_documents.length}</span>
            </div>
            <p>Guidance for the work, shaped by evidence from the work.</p>
          </div>
          <div class="authority-note">
            <b>One source of truth</b>
            <p>
              Maintained principles and playbooks remain in the Development
              Operating System. Direct holds a read-only cache, fingerprints,
              operational findings, and review evidence.
            </p>
          </div>
          <div class="list-label"><span>GUIDANCE</span><span>SOURCE</span></div>
          <div class="guidance-list">
            {#each data.theoria_documents as document}<button
                class="guidance-row"
                class:selected={selectedGuidanceId === document.id}
                onclick={() => (selectedGuidanceId = document.id)}
              >
                <span class="guidance-mark">{document.category.slice(0, 1).toUpperCase()}</span>
                <span>
                  <b>{document.title}</b>
                  <small>{document.description}</small>
                </span>
                <span class="source-state {document.availability}"
                  >{document.availability === "available"
                    ? "Cached"
                    : "Unavailable"}</span
                >
              </button>{/each}
          </div>
          <div class="section-label spaced">
            PROPOSED IMPROVEMENTS <span>{data.method_findings.length}</span>
          </div>
          {#each data.method_findings as proposal}<button
              class="proposal-row"
              onclick={async () => {
                view = "all";
                await choose(proposal.issue_key);
                tab = "theoria";
              }}
            >
              <span class="proposal-state">PROPOSAL · NOT ACCEPTED</span>
              <b>{proposal.proposal}</b>
              <small
                >{proposal.issue_key} · {proposal.classification.replaceAll(
                  "_",
                  " ",
                )} · {date(proposal.created_at)}</small
              >
              <small class="playbook-context"
                >Issue playbook: {playbookFor(proposal.issue_key)}</small
              >
            </button>{:else}<div class="empty compact">
              <div class="empty-symbol">◇</div>
              <h3>No proposed improvements</h3>
              <p>Method findings recorded from real issues will appear here.</p>
            </div>{/each}
        </section>
        <aside class="detail-panel theoria-detail" aria-label="Theoria guidance detail">
          {#if returnTo}<div class="return-bar">
              <button
                class="back-button"
                title="Return to the issue, tab and scroll position (Alt+←)"
                onclick={returnFromGuidance}>← Back to {returnTo.key}</button
              ><span class="tiny"
                >{returnTo.tab === "theoria" ? "Theoria" : returnTo.tab} tab</span
              >
            </div>{/if}
          {#if selectedGuidance}
            <div class="detail-top">
              <span>{selectedGuidance.id}</span>
              <div>
                <span
                  class="tiny"
                  title="Revision of the synced catalog entry. It is not a playbook version."
                  >catalog revision {selectedGuidance.catalog_version}</span
                >
                {@render detailControls()}
              </div>
            </div>
            <div class="detail-heading">
              <span class="status-badge ready">◫ {selectedGuidance.category}</span>
              <h2>{selectedGuidance.title}</h2>
              <p class="prose">{selectedGuidance.description}</p>
            </div>
            <div class="detail-body">
              <div class="cache-banner {selectedGuidance.availability}">
                <b
                  >{selectedGuidance.availability === "available"
                    ? "Read-only cache from the authoritative source"
                    : "Authoritative source unavailable"}</b
                >
                <p>
                  {selectedGuidance.availability === "available"
                    ? `Checked ${date(selectedGuidance.checked_at)}. Refresh is explicit; “cached” does not claim the file is unchanged after that check.`
                    : `${selectedGuidance.unavailable_reason || "Source unavailable"}. ${selectedGuidance.content ? "The last cached content is retained and labelled." : "No cached content is available."}`}
                </p>
              </div>
              <dl class="evidence source-contract">
                <dt>Stable ID</dt><dd>{selectedGuidance.id}</dd>
                <dt>Source</dt><dd
                  ><code>{selectedGuidance.source_root}/{selectedGuidance.relative_path}</code></dd
                >
                <dt>Fingerprint</dt><dd
                  ><code>{selectedGuidance.fingerprint || "unavailable"}</code></dd
                >
                <dt>Source updated</dt><dd
                  >{selectedGuidance.source_updated || "Unknown"}</dd
                >
                <dt>Cached</dt><dd
                  >{selectedGuidance.cached_at
                    ? date(selectedGuidance.cached_at)
                    : "Never"}</dd
                >
              </dl>
              <div class="section-label">CACHED GUIDANCE</div>
              {#if selectedGuidance.content}<pre class="guidance-content"
                  >{selectedGuidance.content}</pre
                >{:else}<p class="muted">
                  Sync this catalog from an accessible source to read it here.
                </p>{/if}
              <div class="section-label spaced">OPERATIONAL TRACE</div>
              <p class="hint">
                The playbook version each issue explicitly recorded when it
                linked this guidance. This page never infers one.
              </p>
              {#each selectedGuidanceUses as pin}<button
                  class="trace-row"
                  onclick={async () => {
                    view = "all";
                    await choose(pin.issue.key);
                    tab = "theoria";
                  }}
                >
                  <b>{pin.issue.key} · {pin.issue.title}</b>
                  <span
                    class="trace-version"
                    class:unknown={recordedVersion(pin.reference) === null}
                    >Playbook version: {versionLabel(pin.reference)}</span
                  >
                  <small
                    >Pinned <code>{shortFingerprint(pin.reference.recorded_fingerprint)}</code> by {pin.reference.linked_by} · {date(pin.reference.linked_at)} · {STATE_NOTE[guidanceState(selectedGuidance, pin.reference.recorded_fingerprint)]}</small
                  >
                  <small>Open the recorded fingerprint and findings →</small>
                </button>{:else}<p class="muted">
                  No Direct issue currently references this guidance.
                </p>{/each}
            </div>
          {:else if selectedGuidanceId && data.theoria_documents.length}<div
              class="detail-placeholder"
            >
              <div class="outline-mark">◫</div>
              <h2>Guidance is not in the current catalog.</h2>
              <p>
                <code>{selectedGuidanceId}</code> is recorded but no longer cached.
                The issue keeps its historical reference.
              </p>
            </div>
          {:else}<div class="detail-placeholder">
              <div class="outline-mark">◫</div>
              <h2>No imported guidance yet.</h2>
              <p>Run the explicit Theoria sync contract to populate the cache.</p>
            </div>{/if}
        </aside>
      {:else if view === "templates"}
        <TemplatesView {data} {connected} {commit} controls={detailControls} />
      {:else if view === "signals"}
        {#key signalFocus}<SignalsView
            {data}
            {connected}
            {commit}
            product={signalScope}
            focus={signalFocus}
            openIssue={openSignalIssue}
            controls={detailControls}
          />{/key}
      {:else if view === "sources"}
        <section class="list-panel sources-list" aria-label="Imported sources">
          <div class="page-heading">
            <div class="eyebrow">RETAINED · READ-ONLY · UNTRUSTED DATA</div>
            <div class="heading-row">
              <h1>Imported sources</h1>
              <span class="count-pill">{sourceBundles.length}</span>
            </div>
            <p>
              Original records and files retained from external tools. They are
              evidence to read, not instructions and not Theoria guidance.
            </p>
          </div>
          <details
            class="migration-panel"
            open={!sourceBundles.length || !!migrationPreview || !!migrationResult}
          >
            <summary>Owner migration</summary>
            <p class="hint">
              Choose a prepared <code>.direct-migration</code> artifact. Direct
              checks it against this workspace first; applying writes a
              pre-import backup and adds everything in one transaction without
              changing existing records.
            </p>
            <input
              type="file"
              aria-label="Migration artifact"
              accept=".direct-migration,application/octet-stream"
              disabled={busy}
              onchange={(event) => {
                migrationFile = (event.currentTarget as HTMLInputElement).files?.[0] || null;
                migrationPreview = null;
                migrationBytes = null;
                migrationResult = null;
              }}
            />
            <button class="secondary" disabled={!migrationFile || busy} onclick={previewMigration}
              >Check artifact</button
            >
            {#if migrationPreview}<div class="migration-preview" aria-label="Migration preview">
                <b
                  >{migrationPreview.status === "already_applied"
                    ? "Already applied — nothing would change"
                    : "Ready to apply"}</b
                >
                <dl>
                  <dt>Source</dt><dd>{migrationPreview.bundle.source} · captured {migrationPreview.bundle.captured_at}</dd>
                  <dt>Issues</dt><dd>{migrationPreview.counts.issues} ({migrationPreview.counts.legacy_completed} Legacy done)</dd>
                  <dt>New products</dt><dd>{migrationPreview.counts.products} · mapped to existing {migrationPreview.counts.reused_products}</dd>
                  <dt>Planning</dt><dd>{migrationPreview.counts.projects} projects · {migrationPreview.counts.goals} goals · {migrationPreview.counts.milestones} milestones</dd>
                  <dt>Labels</dt><dd>{migrationPreview.counts.labels} new · {migrationPreview.counts.reused_labels} reused</dd>
                  <dt>Retained</dt><dd>{migrationPreview.counts.retained_records} records · {migrationPreview.counts.retained_files} files · {bytes(migrationPreview.counts.retained_bytes)}</dd>
                  <dt>Artifact</dt><dd><code>{migrationPreview.artifact_sha256.slice(0, 16)}…</code></dd>
                </dl>
                {#if migrationPreview.workspace_changed_since_preparation}<p class="hint">
                    The workspace changed since this artifact was prepared. Every
                    collision was checked again against the current records.
                  </p>{/if}
                {#if migrationPreview.status === "ready_to_apply"}<button
                    class="primary"
                    disabled={busy}
                    onclick={applyMigration}>Apply migration</button
                  >{/if}
              </div>{/if}
            {#if migrationResult}<p class="migration-result" role="status">
                {migrationResult.status === "applied"
                  ? `Applied. Pre-import backup: ${migrationResult.backup}`
                  : "Already applied; nothing changed."}
              </p>{/if}
          </details>
          {#each sourceBundles as bundle}<article class="bundle-card" aria-label="Source bundle">
              <header>
                <b>{bundle.label}</b>
                <small>{bundle.source} · captured {bundle.captured_at}</small>
              </header>
              <div class="gates">
                {#each Object.entries(bundle.summary?.cutover_readiness?.gates || {}).filter(([gate]) => gate !== "application") as [gate, info]}<span
                    class="gate {(info as any).status}"
                    title={(info as any).detail || ""}
                    >{gate.replaceAll("_", " ")}: {String((info as any).status).replaceAll("_", " ")}</span
                  >{/each}
                <span class="gate {bundle.application ? 'applied' : 'not_applied'}"
                  >live application: {bundle.application ? `applied at cursor ${bundle.application.applied_cursor}` : "not applied here"}</span
                >
              </div>
              <small
                >{bundle.record_count} records · {bundle.file_count} files · {bytes(bundle.total_bytes)} ·
                {Object.entries(bundle.records_by_access).map(([access, count]) => `${count} ${access}`).join(" · ")}</small
              >
              {#if bundle.application?.rollback_available}<button
                  class="text-button"
                  disabled={busy}
                  onclick={() => rollbackMigration(bundle)}>Roll back this import</button
                >{:else if bundle.application}<small class="muted"
                  >Rollback is unavailable because the workspace changed after the import. Use the pre-import backup {bundle.application.backup || ""} to recover.</small
                >{/if}
            </article>{/each}
          <div class="toolbar">
            <label class="search"
              ><span>⌕</span><input
                aria-label="Search imported sources"
                bind:value={sourceQuery}
                placeholder="Search titles, identifiers, text…"
                onkeydown={(event) => {
                  if (event.key === "Enter") searchSources(true);
                }}
              /></label
            >
          </div>
          <div class="project-toolbar">
            <label class="project-filter"
              >Kind
              <select aria-label="Filter sources by kind" bind:value={sourceKind} onchange={() => searchSources(true)}>
                <option value="">All kinds</option>
                {#each sourceKinds as kind}<option value={kind}>{kind.replaceAll("_", " ")}</option>{/each}
              </select></label
            >
            <label class="project-filter"
              >Class
              <select aria-label="Filter sources by classification" bind:value={sourceClass} onchange={() => searchSources(true)}>
                <option value="">All</option>
                {#each ["native", "transformed", "preserved", "unresolved"] as value}<option {value}>{value}</option>{/each}
              </select></label
            >
            {#if sourceIssue}<button
                class="text-button"
                onclick={() => {
                  sourceIssue = "";
                  searchSources(true);
                }}>Linked to {sourceIssue} ×</button
              >{/if}
          </div>
          <div class="list-label"><span>{sourceTotal} RECORDS</span><span>ACCESS</span></div>
          <div class="source-results">
            {#each sourceResults as record}<button
                class="source-row"
                class:selected={sourceRecord?.record.id === record.id}
                onclick={() => openSource(record.id)}
              >
                <span class="source-kind">{record.kind.replaceAll("_", " ")}</span>
                <span class="source-text"
                  ><b>{record.label || record.source_id || record.kind}</b>
                  {#if record.title}<small>{record.title}</small>{/if}</span
                >
                <span class="access {record.access}">{record.access}</span>
              </button>{:else}<div class="empty compact">
                <div class="empty-symbol">⧉</div>
                <h3>{sourceBundles.length ? "No matching records" : "No imported sources"}</h3>
                <p>{sourceBundles.length ? "Try another search." : "Apply a prepared migration to retain its sources here."}</p>
              </div>{/each}
            {#if sourceResults.length < sourceTotal}<button
                class="text-button"
                disabled={sourceBusy}
                onclick={() => searchSources(false)}>Load more</button
              >{/if}
          </div>
        </section>
        <aside class="detail-panel source-detail" aria-label="Imported source detail">
          {#if sourceRecord}
            <div class="detail-top">
              <span>{sourceRecord.record.kind.replaceAll("_", " ")}</span>
              <div>
                <span class="tiny"
                  >{sourceRecord.record.classification} · {sourceRecord.record.access}</span
                >
                {@render detailControls()}
              </div>
            </div>
            <div class="detail-heading">
              <span class="status-badge ready">⧉ Retained source</span>
              <h2>{sourceRecord.record.label || sourceRecord.record.source_id}</h2>
              {#if sourceRecord.record.title}<p class="prose">{sourceRecord.record.title}</p>{/if}
            </div>
            <div class="detail-body">
              <div class="cache-banner available">
                <b>Read-only original data</b>
                <p>{sourceRecord.authority}</p>
              </div>
              {#if sourceRecord.record.issue_keys.length}<div class="section-label">LINKED ISSUES</div>
                {#each sourceRecord.record.issue_keys as key}<button
                    class="trace-row"
                    onclick={async () => {
                      view = "all";
                      await choose(key);
                    }}
                    disabled={!data.issues.some((issue) => issue.key === key)}
                    ><b>{key}</b> <small>{data.issues.find((issue) => issue.key === key)?.title || "Deleted after import; the retained original remains here"}</small></button
                  >{/each}{/if}
              {#if sourceRecord.record.reasons.length}<div class="section-label spaced">HOW DIRECT HOLDS IT</div>
                <ul class="source-reasons">
                  {#each sourceRecord.record.reasons as reason}<li>{reason}</li>{/each}
                </ul>{/if}
              {#if sourceRecord.record.preserved_fields.length}<p class="hint">
                  Fields kept only here: {sourceRecord.record.preserved_fields.join(", ")}
                </p>{/if}
              {#each sourceRecord.readable as item}<div class="section-label spaced">{item.field.toUpperCase()}</div>
                <pre class="guidance-content source-text-block">{item.text}</pre>{/each}
              {#if sourceRecord.history_entries}<p class="hint">
                  {sourceRecord.history_entries} Linear history entries are in the original record below.
                </p>{/if}
              {#if sourceRecord.component}<div class="section-label spaced">COMPONENT {sourceRecord.record.pointer}</div>
                <pre class="guidance-content">{JSON.stringify(sourceRecord.component, null, 2)}</pre>{/if}
              <div class="section-label spaced">FILES</div>
              <div class="source-files">
                {#if sourceRecord.download}<button
                    class="secondary"
                    onclick={() => downloadSource(sourceRecord!.download!.bundle_id, sourceRecord!.download!.path, sourceRecord!.download!.original_name)}
                    >↧ Download {sourceRecord.download.original_name || sourceRecord.download.path} ({bytes(sourceRecord.download.bytes)})</button
                  >{/if}
                <button
                  class="text-button"
                  onclick={() => downloadSource(sourceRecord!.file.bundle_id, sourceRecord!.file.path)}
                  >↧ Original file {sourceRecord.file.path} ({bytes(sourceRecord.file.bytes)})</button
                >
                <small class="muted">sha256 {sourceRecord.file.sha256}</small>
              </div>
              <details class="source-original">
                <summary>Original record ({bytes(sourceRecord.content_bytes)}{sourceRecord.truncated ? ", truncated — download the file for the rest" : ""})</summary>
                <pre class="guidance-content">{sourceRecord.truncated ? sourceRecord.content : pretty(sourceRecord.content)}</pre>
              </details>
            </div>
          {:else}<div class="detail-placeholder">
              <div class="outline-mark">⧉</div>
              <h2>Select a retained record.</h2>
              <p>Search documents, comments, relations, planning updates and files kept from the original source.</p>
            </div>{/if}
        </aside>
      {:else}
      <section class="list-panel" bind:this={listPane}>
        <div class="page-heading">
          <div class="eyebrow">A LITTLE LESS COORDINATION.</div>
          <div class="heading-row">
            <h1>{title}</h1>
            <span class="count-pill">{visible.length}</span>
          </div>
          <p>
            {view === "needs"
              ? "Your judgment is the next step."
              : "Clear intent. Focused work. Verified delivery."}
          </p>
        </div>
        <div class="toolbar">
          <label class="search"
            ><span>⌕</span><input
              aria-label="Search issues"
              bind:value={search}
              placeholder="Search issues…"
              onkeydown={(event) => {
                if (event.key === "Enter" && exactSearchKey) {
                  event.preventDefault();
                  choose(exactSearchKey);
                }
              }}
            /><kbd>⌕</kbd></label
          ><button class="secondary" onclick={newIssue} disabled={!connected}
            >＋ Issue</button
          >
        </div>
        <div class="project-toolbar">
          <label class="project-filter"
            >Project
            <select aria-label="Filter by project" bind:value={projectFilter}>
              <option value="all">All projects</option>
              <option value="none">No project</option>
              {#each availableProjects as p}<option value={p.id}
                  >{p.name}{product === "all"
                    ? ` · ${data.products.find((product) => product.id === p.product_id)?.name}`
                    : ""}</option
                >{/each}
            </select>
          </label>
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editProject()}>＋ New project</button
          >
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editGoal()}>＋ New goal</button
          >
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editRelease()}>＋ New release</button
          >
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editWorkflow()}>Release setup</button
          >
          {#if product !== "all"}<button
              class="text-button"
              disabled={!connected || busy}
              onclick={editProductPaths}>Product settings</button
            >{/if}
          {#if selectedProject}<button
              class="text-button"
              disabled={!connected || busy}
              onclick={() => editProject(selectedProject)}>Edit project</button
            >{/if}
          <div class="view-switch" aria-label="Issue layout">
            <button
              class:active={listMode === "flat"}
              aria-pressed={listMode === "flat"}
              onclick={() => (listMode = "flat")}>Flat</button
            ><button
              class:active={listMode === "project"}
              aria-pressed={listMode === "project"}
              onclick={() => (listMode = "project")}>By project</button
            >
          </div>
          {#if listMode === "flat"}<label class="project-filter"
              >Sort<select aria-label="Sort issues" bind:value={sortMode}
                ><option value="updated">Recently updated</option><option value="key"
                  >Issue code</option
                ></select
              ></label
            >{/if}
        </div>
        {#if visibleReleases.length}<section class="release-strip" aria-label="Releases">
            <div class="section-label">RELEASES <span>{visibleReleases.length}</span></div>
            <div class="release-strip-items">
              {#each visibleReleases as release}
                {@const progress = releaseProgress(release.id)}
                <button class="release-chip" onclick={() => editRelease(release)}>
                  <span><b>{release.version_label}</b> · {release.name}</span>
                  <small>{release.status} · {progress?.completed || 0} done · {progress?.pending_verification || 0} verify · {progress?.failed_verification || 0} failed</small>
                </button>
              {/each}
            </div>
          </section>{/if}
        <div class="project-toolbar label-toolbar">
          <label class="project-filter"
            >Label
            <select aria-label="Filter by label" bind:value={labelFilter}>
              <option value="all">All labels</option>
              <option value="none">No label</option>
              {#each availableLabels as label}<option value={label.id}
                  >{label.name}</option
                >{/each}
            </select>
          </label>
          <button
            class="text-button"
            disabled={!connected || busy}
            onclick={() => editLabel()}>＋ New label</button
          >
          {#if selectedLabel}<button
              class="text-button"
              disabled={!connected || busy}
              onclick={() => editLabel(selectedLabel)}>Edit label</button
            >
            <span class="label-filter-note"
              >{selectedLabel.products.length
                ? `Applies to ${selectedLabel.products
                    .map(
                      (rule) =>
                        data.products.find((p) => p.id === rule.product_id)?.name ||
                        "?",
                    )
                    .join(", ")}`
                : "Applies to every product"}{selectedLabel.aliases.length
                ? ` · aliases: ${selectedLabel.aliases.join(", ")}`
                : ""}</span
            >{/if}
        </div>
        {#if selectedProject}
          {@const progress = projectProgress(selectedProject.id)}
          {@const legacyCompleted = progress.legacy_completed || 0}
          <div class="project-summary">
            <div>
              <strong>{selectedProject.description || "No project outcome recorded."}</strong>
              <span
                >{selectedProject.status || "active"} · {selectedProject.priority ||
                  "medium"} priority · order {selectedProject.sort_order || 0}</span
              >
              {#if selectedProject.template}<span class="template-chip" title="Intake template provenance"
                  >▦ {provenanceLabel(data, selectedProject.template)}</span
                >{/if}
            </div>
            <div class="project-progress-copy">
              <b>{progress.completion_percent}%</b>
              <span
                >{progress.completed}/{progress.total - progress.canceled - legacyCompleted} verified done · {legacyCompleted} legacy done · {progress.pending_verification} verify · {progress.canceled} canceled</span
              >
            </div>
            <div class="progress-track" aria-label="Project completion">
              <span style={`width: ${progress.completion_percent}%`}></span>
            </div>
            <div class="project-planning-links">
              <div>
                <span>GOALS</span>
                {#each selectedProjectGoals as goal}
                  {@const progress = goalProgress(goal.id)}
                  <button class="planning-link" onclick={() => editGoal(goal)}
                    ><b>{goal.name}</b><small
                      >{goal.status} · {goal.priority} · {progress?.completion_percent || 0}%</small
                    ></button
                  >
                {:else}<small>No goals link this project yet.</small>{/each}
              </div>
              <div>
                <span>MILESTONES</span>
                {#each selectedProjectMilestones as milestone}
                  {@const progress = milestoneProgress(milestone.id)}
                  <button class="planning-link" onclick={() => editMilestone(milestone)}
                    ><b>{milestone.name}</b><small
                      >order {milestone.sort_order} · {progress?.completion_percent || 0}% · {progress?.pending_verification || 0} verify</small
                    ></button
                  >
                {:else}<small>No milestones in this project yet.</small>{/each}
                <button class="text-button add-planning" onclick={() => editMilestone()}
                  >＋ Add milestone</button
                >
              </div>
              <div>
                <span>RELEASES</span>
                {#each selectedProjectReleases as release}
                  {@const progress = releaseProgress(release.id)}
                  <button class="planning-link" onclick={() => editRelease(release)}
                    ><b>{release.version_label} · {release.name}</b><small
                      >{release.status} · {progress?.completion_percent || 0}% · {progress?.pending_verification || 0} verify · {progress?.failed_verification || 0} failed</small
                    ></button
                  >
                {:else}<small>No releases link this project yet.</small>{/each}
              </div>
            </div>
            <div class="label-row project-labels" aria-label="Project labels">
              <span class="label-row-title">Labels</span>
              {@render labelChips(selectedProject.labels, (id) =>
                toggleProjectLabel(selectedProject, id, false),
              )}
              {@render labelPicker(
                selectedProject.product_id,
                selectedProject.labels,
                (id) => toggleProjectLabel(selectedProject, id, true),
              )}
            </div>
          </div>
        {/if}
        <div class="list-label"><span>ISSUE</span><span>STATUS</span></div>
        <div class="issue-list">
          {#if visible.length && listMode === "project"}
            {#each groupedVisible as group}
              {@const progress = group.project
                ? projectProgress(group.project.id)
                : null}
              <section class="issue-group">
                <header>
                  <div>
                    <strong>{group.name}</strong>
                    {#if group.project}<span
                        >{group.project.status || "active"} · {group.project.priority ||
                          "medium"}</span
                      >{@render labelChips(group.project.labels)}{:else}<span
                        >Explicitly ungrouped work</span
                      >{/if}
                  </div>
                  {#if progress}<small
                      >{progress.completion_percent}% · {progress.completed} done · {progress.pending_verification} verify</small
                    >{:else}<small>{group.issues.length} items</small>{/if}
                </header>
                {#each group.issues as i}{@render issueRow(i)}{/each}
              </section>
            {/each}
          {:else if visible.length}
            {#each visible as i}{@render issueRow(i)}{/each}
          {:else}<div class="empty">
              <div class="empty-symbol">
                {search ? "⌕" : view === "needs" ? "✓" : "↗"}
              </div>
              <h2>
                {search
                  ? "No matching issues"
                  : view === "needs"
                    ? "Nothing waiting on you"
                    : "Room for your next idea"}
              </h2>
              <p>
                {search
                  ? "Try another title or issue ID."
                  : view === "needs"
                    ? "Submitted work and decisions will appear here."
                    : "Capture an issue, shape the outcome, and hand it off."}
              </p>
              {#if !search && view !== "needs"}<button
                  class="primary"
                  onclick={newIssue}
                  disabled={!connected}>Create your first issue</button
                >{/if}
            </div>{/if}
        </div>
        <div class="list-footer">
          <span
            >{visible.length} work item{visible.length === 1 ? "" : "s"}</span
          ><span
            >Changes appear automatically <span
              class="connection-dot"
              class:online={connected}
            ></span></span
          >
        </div>
      </section>

      <aside class="detail-panel" aria-label="Issue detail">
        {#if current}
          <div class="detail-top">
            <span>{current.key}</span>
            <div>
              <span class="tiny">v{current.version}</span>{@render detailControls()}<button
                class="icon-button"
                aria-label="Close issue"
                onclick={() => {
                  selected = "";
                  context = null;
                }}>×</button
              >
            </div>
          </div>
          <div class="detail-heading">
            <span class="status-badge {current.status}"
              >{glyphs[current.status]} {labels[current.status]}</span
            >
            <h2>{current.title}</h2>
            <div class="properties">
              <span>Owner <b>{current.owner || "Unassigned"}</b></span><span
                >Priority <b>{current.priority}</b></span
              ><span
                >Route <b>{current.planning_scope === "project" ||
                    (!current.planning_scope && current.project_id)
                    ? "Project work"
                    : "Inbox / maintenance"}</b></span
              >
            </div>
            <label class="project-assignment"
              >Project
              <select
                aria-label="Issue project"
                value={current.project_id || ""}
                  disabled={busy || !connected || ["verify", "done", "legacy_completed", "canceled"].includes(current.status)}
                onchange={async (event) => {
                  const control = event.currentTarget;
                  const result = await act({
                    op: "set_issue_project",
                    key: current.key,
                    expected_version: current.version,
                    project_id: control.value || null,
                  });
                  if (!result) control.value = current?.project_id || "";
                }}
              >
                <option value="">No project</option>
                {#each data.projects.filter((p) => p.product_id === current.product_id) as p}<option
                    value={p.id}>{p.name}</option
                  >{/each}
              </select>
            </label>
            {#if current.project_id}
              <label class="project-assignment"
                >Milestone
                <select
                  aria-label="Issue milestone"
                  value={current.milestone_id || ""}
                  disabled={busy || !connected || ["verify", "done", "legacy_completed", "canceled"].includes(current.status)}
                  onchange={async (event) => {
                    const control = event.currentTarget;
                    const result = await act({
                      op: "set_issue_milestone",
                      key: current.key,
                      expected_version: current.version,
                      milestone_id: control.value || null,
                    });
                    if (!result) control.value = current?.milestone_id || "";
                  }}
                >
                  <option value="">No milestone</option>
                  {#each data.milestones.filter((milestone) => milestone.project_id === current.project_id).sort((a, b) => a.sort_order - b.sort_order) as milestone}<option
                      value={milestone.id}>{milestone.name}</option
                    >{/each}
                </select>
              </label>
              {@const issueGoals = data.goals.filter((goal) => goal.project_ids.includes(current.project_id!))}
              {#if issueGoals.length}<div class="issue-goals">
                  <span>Goals</span>
                  {#each issueGoals as goal}<button onclick={() => editGoal(goal)}
                      >{goal.name}</button
                    >{/each}
                </div>{/if}
            {/if}
            <div class="label-row issue-labels" aria-label="Issue labels">
              <span class="label-row-title">Labels</span>
              {#if !(current.labels || []).length}<span class="muted tiny"
                  >None</span
                >{/if}
              {@render labelChips(current.labels, (id) =>
                toggleIssueLabel(current, id, false),
              )}
              {@render labelPicker(current.product_id, current.labels, (id) =>
                toggleIssueLabel(current, id, true),
              )}
            </div>
          </div>
          <div class="tabs" role="tablist" aria-label="Issue sections">
            {#each [["brief", "Brief"], ["relations", "Relations"], ["sources", "Source"], ["theoria", "Theoria"], ["verify", "Verification"], ["activity", "Activity"]] as [id, label] (id)}<button
                role="tab"
                aria-selected={tab === id}
                class:active={tab === id}
                onclick={() => (tab = id)}
                >{label}{#if id === "verify" && current.status === "verify"}<span
                    class="tab-dot"
                  ></span>{:else if id === "relations" && context?.issue_links.length}<span
                    class="tab-count">{context.issue_links.length}</span
                  >{:else if id === "sources" && context?.retained_sources?.length}<span
                    class="tab-count">{context.retained_sources.length}</span
                  >{:else if id === "theoria" && (current.theoria_refs.length || context?.method_findings.length)}<span
                    class="tab-count"
                    >{current.theoria_refs.length + (context?.method_findings.length || 0)}</span
                  >{/if}</button
              >{/each}
          </div>
          <div class="detail-body" bind:this={detailBody}>
            {#if tab === "brief"}
              {#if current.external}<div class="info-card">
                  <span class="card-symbol">◇</span>
                  <div>
                    <b>Imported from {current.external.source}: {current.external.state.name}</b>
                    <p>
                      Historical completion is provenance only; it is not a passed Direct verification.
                      <a href={current.external.url} target="_blank" rel="noreferrer">Open source record</a>
                    </p>
                  </div>
                </div>{/if}
              {#if context?.template}<div class="info-card template-provenance" aria-label="Template provenance">
                  <span class="card-symbol">▦</span>
                  <div>
                    <b>Created from {context.template.revision.name} · revision {context.template.use.revision}{context.template.outdated ? ` · outdated (revision ${context.template.current_revision} is current)` : ""}{context.template.retired ? " · template retired" : ""}</b>
                    <p>
                      Execution mode: {context.template.use.execution_mode || "not set"}{context.template.use.supplement_product_id ? " · product supplement applied" : ""}{context.template.use.overrides.length ? ` · overridden: ${context.template.use.overrides.join(", ")}` : ""}.
                      Provenance only; it does not make work Ready or count as verification.
                    </p>
                  </div>
                </div>{/if}
              <div class="section-label">
                PROBLEM & OUTCOME {#if !["verify", "done", "legacy_completed", "canceled"].includes(current.status)}<button
                    class="text-button"
                    onclick={() => edit(current)}>Edit brief</button
                  >{/if}
              </div>
              <p class="prose" class:muted={!current.body}>
                {current.body ||
                  "Start with the problem this work should solve."}
              </p>
              {#if context?.issue.intake && (context.issue.intake.text || context.issue.intake.images.length)}
                <details class="saved-intake">
                  <summary>Original task context · {context.issue.intake.images.length} screenshots</summary>
                  <p class="prose">{context.issue.intake.text}</p>
                  {#each context.issue.intake.images as image, index}
                    <figure class="intake-image"><img src={image.data_url} alt={image.caption || `Context screenshot ${index+1}`} />
                      <figcaption>{image.caption || `Screenshot ${index+1}`}</figcaption></figure>
                  {/each}
                </details>
              {/if}
              <div class="section-label">ACCEPTANCE CRITERIA</div>
              <p class="prose" class:muted={!current.acceptance}>
                {current.acceptance ||
                  "Describe what a good result looks like before making this Ready."}
              </p>
              <ContextDocs
                {data}
                {connected}
                {commit}
                targetKind="issue"
                target={current.key}
                links={context?.context_links || []}
                productId={current.product_id}
              />
              {#if current.claim}<div class="info-card">
                  <span class="card-symbol">↗</span>
                  <div>
                    <b>{current.claim.actor}</b>
                    <p>
                      {current.claim.expires_at > clock
                        ? "Claim active until"
                        : "Claim expired"}
                      {date(current.claim.expires_at)}
                    </p>
                  </div>
                </div>{/if}
              {#if current.needs_fix}<div class="info-card warning">
                  <span>↺</span>
                  <div>
                    <b>Another pass is needed</b>
                    <p>Review the previous test evidence before continuing.</p>
                  </div>
                </div>{/if}
              <div class="actions">
                {#if current.status === "backlog"}<button
                    class="primary"
                    disabled={busy ||
                      (current.planning_scope === "project" &&
                        !current.project_id)}
                    onclick={() => makeReady(current)}>Make ready <span>→</span></button
                  >
                  <p class="hint">
                    {current.planning_scope === "project" && !current.project_id
                      ? "Choose a project before making project work Ready."
                      : "Requires a brief, acceptance criteria, and an owner."}
                  </p>{/if}
                {#if ["ready", "doing"].includes(current.status) && (!current.claim || current.claim.expires_at <= clock)}<button
                    class="primary"
                    disabled={busy}
                    onclick={() =>
                      act({
                        op: "claim",
                        key: current.key,
                        expected_version: current.version,
                        lease_seconds: 3600,
                      })}>Claim this work <span>↗</span></button
                  >{/if}
                {#if current.status === "doing" && current.claim?.actor === "owner" && current.claim.expires_at > clock}<button
                    class="primary"
                    disabled={busy}
                    onclick={() => {
                      draft.version = current.version;
                      e2eDraft = { environment: "", entrypoint: "", scenarios: "", delivered_build_ref: "", delivery_check: "" };
                      handoff = {
                        build_ref: "",
                        delivery_ref: "",
                        summary: "",
                        checks: "",
                        limitations: "",
                        preconditions: "",
                        steps: [{ instruction: "", expected: "" }],
                      };
                      modal = "submit";
                    }}>Submit for verification <span>→</span></button
                  ><button
                    class="secondary"
                    disabled={busy}
                    onclick={() =>
                      act({
                        op: "release",
                        key: current.key,
                        expected_version: current.version,
                      })}>Release claim</button
                  >{/if}
                {#if current.status === "verify"}<button
                    class="primary"
                    onclick={() => (tab = "verify")}
                    >Review the result <span>→</span></button
                  >{/if}
                {#if deletion?.eligible}<button
                    class="danger-button"
                    disabled={busy}
                    onclick={() => {
                      error = "";
                      modal = "delete";
                    }}>Delete issue</button
                  >{/if}
              </div>
              {#if deletion && deletionExplained}<div
                  class="info-card deletion-blockers"
                  role="note"
                  aria-label="Why this issue cannot be deleted"
                >
                  <span class="card-symbol">i</span>
                  <div>
                    <b>Deletion unavailable</b>
                    <ul>
                      {#each deletion.blockers as blocker}<li>
                          {blocker.message}.
                          {#if blocker.kind === "issue_links"}<button
                              class="text-button"
                              onclick={() => (tab = "relations")}>Open relations</button
                            >{:else if blocker.kind === "comments" || blocker.kind === "git_traces"}<button
                              class="text-button"
                              onclick={() => (tab = "activity")}>Open activity</button
                            >{:else if blocker.kind === "method_findings"}<button
                              class="text-button"
                              onclick={() => (tab = "theoria")}>Open Theoria</button
                            >{:else if blocker.kind === "verification_history" || blocker.kind === "verification_children"}<button
                              class="text-button"
                              onclick={() => (tab = "verify")}>Open verification</button
                            >{:else if blocker.kind === "release_references" && blocker.removable}{#each blocker.references as releaseId}{@const release = data.releases.find((item) => item.id === releaseId)}{#if release}<button
                                  class="text-button"
                                  onclick={() => editRelease(release)}
                                  >Edit {release.version_label}</button
                                >{/if}{/each}{/if}
                        </li>{/each}
                    </ul>
                    <p>
                      {deletion.blockers.every((blocker) => blocker.removable)
                        ? "Clear these and deletion becomes available."
                        : "Direct retains this issue because of the history or release scope listed above."}
                    </p>
                  </div>
                </div>{/if}
              {#if context?.releases.length}<div class="section-label">
                  RELEASES <span>{context.releases.length}</span>
                </div>
                {#each context.releases as release}
                  {@const progress = context.release_progress.find((item) => item.release_id === release.id)}
                  {@const evidence = context.release_evidence.filter((item) => item.release_id === release.id)}
                  <article class="info-card">
                    <span class="card-symbol">◆</span>
                    <div>
                      <b>{release.version_label} · {release.name}</b>
                      <p>
                        {release.status} · target <code>{release.target_ref}</code> ·
                        {progress?.completed || 0}/{(progress?.total || 0) - (progress?.canceled || 0) - (progress?.legacy_completed || 0)} verified done ·
                        {progress?.pending_verification || 0} pending · {progress?.failed_verification || 0} failed verification
                      </p>
                      <small>{evidence.length} evidence record{evidence.length === 1 ? "" : "s"}</small>
                      {#if release.preview_url}<p><a href={release.preview_url} target="_blank" rel="noreferrer">Open recorded preview</a></p>{/if}
                      <button class="text-button" onclick={() => editRelease(release)}>Edit release</button>
                    </div>
                  </article>
                {/each}
              {/if}
              {#if context?.product.repo_windows || context?.product.vault_windows}<div
                  class="section-label"
                >
                  PROJECT CONTEXT
                </div>
                {#if context.product.repo_windows}<div class="context-path">
                    Repository<code>{context.product.repo_windows}</code>
                  </div>{/if}{#if context.product.vault_windows}<div
                    class="context-path"
                  >
                    Product knowledge<code>{context.product.vault_windows}</code
                    >
                  </div>{/if}{/if}
            {:else if tab === "relations"}
              <div class="relations-panel">
                {#if context?.customer_signals?.length}<div class="section-label">
                    CUSTOMER REQUESTS <span>{context.customer_signals.length}</span>
                  </div>
                  {#each context.customer_signals as signal (signal.id)}<article class="relation-card">
                      <span class="relation-kind"
                        >{signal.promoted_issue_key === context.issue.key ? "Promoted from" : "Requested in"}</span
                      >
                      <button class="relation-target" onclick={() => openSignal(signal.id)}>
                        <b>{signal.summary.split("\n")[0]}</b>
                      </button>
                      <small
                        >{signal.source_kind}{signal.source_reference ? ` · ${signal.source_reference}` : ""}{signal.customer_reference
                          ? ` · ${signal.customer_reference}`
                          : ""} · {signalStatus(signal)}</small
                      >
                    </article>{/each}{/if}
                <div class="section-label">
                  ISSUE LINKS <span>{context?.issue_links.length || 0}</span>
                </div>
                <p class="hint">
                  Parent and blocker links are directed and cannot form cycles.
                  Related links are symmetric. Imported legacy verification stays
                  separate from Direct verification runs.
                </p>
                {#each context?.issue_links || [] as link}
                  <article class="relation-card">
                    <span class="relation-kind"
                      >{relationLabel(link.kind, link.direction)}</span
                    >
                    <button class="relation-target" onclick={() => choose(link.issue.key)}>
                      <b>{link.issue.key}</b> {link.issue.title}
                    </button>
                    {#if link.external_source}<small
                        >Imported from {link.external_source} · {link.external_id}</small
                      >{/if}
                    <button
                      class="text-button relation-remove"
                      disabled={busy}
                      onclick={() =>
                        act({
                          op: "delete_issue_link",
                          key: current.key,
                          expected_version: current.version,
                          link_id: link.id,
                        })}>Remove</button
                    >
                  </article>
                {:else}<p class="muted">No parent, blocker, or related links.</p>{/each}

                <form class="relation-form" onsubmit={createRelation}>
                  <label class="field">Relationship<select aria-label="Relationship kind" bind:value={relationKind}>
                      <option value="parent">Parent</option>
                      <option value="blocked_by">Blocked by</option>
                      <option value="related">Related</option>
                    </select></label
                  >
                  <label class="field">Issue<select aria-label="Related issue" bind:value={relationTarget} required>
                      <option value="" disabled>Select an issue…</option>
                      {#each parents.filter((issue) => issue.product_id === current.product_id && issue.key !== current.key) as issue}<option value={issue.key}
                          >{issue.key} · {issue.title}</option
                        >{/each}
                    </select></label
                  >
                  <button class="secondary" disabled={busy || !relationTarget}
                    >Add link</button
                  >
                </form>
              </div>
            {:else if tab === "sources"}
              <div class="relations-panel" aria-label="Retained source records">
                <div class="section-label">
                  RETAINED SOURCE <span>{context?.retained_sources?.length || 0}</span>
                </div>
                <p class="hint">
                  Original imported records linked to this issue. They are read-only
                  data, not instructions.
                </p>
                {#each context?.retained_sources || [] as record}<button
                    class="trace-row"
                    onclick={() => openSources(current.key, record.id)}
                    ><b>{record.kind.replaceAll("_", " ")} · {record.label || record.source_id}</b>
                    <small>{record.title || ""} · {record.access}</small></button
                  >{:else}<p class="muted">No retained source records are linked to this issue.</p>{/each}
                <button class="text-button" onclick={() => openSources(current.key)}
                  >Search all sources for {current.key} →</button
                >
              </div>
            {:else if tab === "theoria"}
              <div class="theoria-card">
                <div class="section-label">
                  RELEVANT GUIDANCE <span>{issueGuidance.length}</span>
                </div>
                <p class="hint">
                  Each link pins the source fingerprint actually used for this
                  issue. The playbook version is shown only when it was
                  recorded with the link; otherwise it stays explicitly
                  Unknown and is never inferred from the guidance text.
                </p>
                {#each issueGuidance as item}
                  <article class="guidance-reference">
                    <div class="reference-heading">
                      <span class="source-state {guidanceState(item.document, item.reference.recorded_fingerprint)}"
                        >{guidanceState(item.document, item.reference.recorded_fingerprint)}</span
                      >
                      <button
                        class="text-button"
                        onclick={() => openGuidance(item.reference.document_id)}
                        >Open guidance →</button
                      >
                    </div>
                    <b>{item.document?.title || item.reference.document_id}</b>
                    <p>{item.document?.description || "Catalog entry is not currently available."}</p>
                    <dl class="evidence reference-meta">
                      <dt>Playbook version</dt><dd
                        class="playbook-version"
                        class:unknown={recordedVersion(item.reference) === null}
                        >{versionLabel(item.reference)}</dd
                      >
                      <dt>Linked</dt><dd>{item.reference.linked_by} · {date(item.reference.linked_at)}</dd>
                      <dt>Recorded fingerprint</dt><dd><code>{item.reference.recorded_fingerprint || "unavailable"}</code></dd>
                      <dt>Current fingerprint</dt><dd><code>{item.document?.fingerprint || "unavailable"}</code></dd>
                    </dl>
                    {#if guidanceState(item.document, item.reference.recorded_fingerprint) === "stale"}<p class="source-warning">
                        The current cache differs from the fingerprint recorded
                        for this issue. The historical reference is preserved.
                      </p>{:else if guidanceState(item.document, item.reference.recorded_fingerprint) === "unavailable"}<p class="source-warning">
                        The authoritative source could not be checked. Any last
                        cached content remains labelled and is not treated as current.
                      </p>{/if}
                  </article>
                {:else}<p class="muted">
                    No guidance is linked to this issue yet.
                  </p>{/each}

                <form class="inline-theoria-form" onsubmit={linkGuidance}>
                  <label class="field"
                    >Guidance<select bind:value={guidanceDocumentId} required>
                      <option value="" disabled>Select a catalog document…</option>
                      {#each data.theoria_documents.filter((document) => document.product_id === current.product_id && !current.theoria_refs.some((reference) => reference.document_id === document.id)) as document}<option value={document.id}
                          >{document.title}{document.availability === "unavailable" ? " · unavailable" : ""}</option
                        >{/each}
                    </select></label
                  >
                  <label class="field"
                    >Playbook version (optional)<input
                      bind:value={guidanceVersion}
                      placeholder="Leave blank to preserve Unknown"
                    /></label
                  >
                  <button class="secondary" disabled={busy || !guidanceDocumentId}
                    >Link recorded guidance</button
                  >
                </form>
              </div>

              <div class="theoria-card">
                <div class="section-label">
                  METHOD FINDINGS <span>{context?.method_findings.length || 0}</span>
                </div>
                <p class="hint">
                  Findings distinguish observed facts from hypotheses and
                  proposals. Recording one does not change a playbook or enroll an experiment.
                </p>
                <dl class="evidence reference-meta playbook-context">
                  <dt>Playbook context</dt><dd
                    class:unknown={!issuePlaybook.versions.length}
                    >{playbookContextLabel(issuePlaybook)}</dd
                  >
                </dl>
                <p class="hint">
                  Findings carry no version of their own; this is the version
                  recorded on the guidance pinned to this issue.
                </p>
                {#each context?.method_findings || [] as item}
                  <article class="finding-card">
                    <div class="finding-heading">
                      <span class="proposal-state">PROPOSAL · NOT ACCEPTED</span>
                      <span>{item.classification.replaceAll("_", " ")}</span>
                    </div>
                    <div class="finding-part"><small>OBSERVED FACT</small><p>{item.observation}</p></div>
                    {#if item.hypothesis}<div class="finding-part"><small>HYPOTHESIS / UNRESOLVED</small><p>{item.hypothesis}</p></div>{/if}
                    <div class="finding-part"><small>PROPOSED IMPROVEMENT</small><p>{item.proposal}</p></div>
                    <div class="finding-part"><small>EVIDENCE</small>
                      {#each item.evidence as pointer}<p><b>{pointer.kind}</b> · <code>{pointer.reference}</code>{pointer.summary ? ` — ${pointer.summary}` : ""}</p>{/each}
                    </div>
                    <small class="finding-byline">{item.created_by} · {date(item.created_at)}</small>
                  </article>
                {/each}

                <form class="finding-form" onsubmit={createFinding}>
                  <label class="field">Classification<select bind:value={finding.classification}>
                      <option value="product_defect">Product defect</option>
                      <option value="method_friction">Method friction</option>
                      <option value="both">Both</option>
                    </select></label>
                  <label class="field">Observed fact<textarea rows="3" required bind:value={finding.observation} placeholder="What actually happened? Keep interpretation out of this field."></textarea></label>
                  <label class="field">Hypothesis or unresolved question<textarea rows="2" bind:value={finding.hypothesis} placeholder="What might explain it, or what remains unknown?"></textarea></label>
                  <label class="field">Proposed improvement<textarea rows="3" required bind:value={finding.proposal} placeholder="A reviewable proposal—not an accepted playbook change."></textarea></label>
                  <div class="form-grid">
                    <label class="field">Evidence type<select bind:value={finding.evidenceKind}>
                        <option value="issue">Issue</option>
                        <option value="build">Build</option>
                        <option value="check">Check</option>
                        <option value="owner_review">Owner review</option>
                      </select></label>
                    <label class="field">Evidence reference<input required bind:value={finding.evidenceReference} placeholder="Command, build, issue, or review reference" /></label>
                  </div>
                  <label class="field">Evidence summary<input bind:value={finding.evidenceSummary} placeholder="What the evidence showed" /></label>
                  <div class="proposal-contract">
                    This creates a proposal only. Acceptance, experiment enrollment,
                    promotion, and Jev routing remain outside this operation.
                  </div>
                  <button class="secondary" disabled={busy}>Record method finding</button>
                </form>
              </div>
            {:else if tab === "verify"}
              {#if activeRun}
                <div class="verification-banner {activeRun.outcome}">
                  <span>◈</span>
                  <div>
                    <b
                      >{activeRun.outcome === "pending"
                        ? activeRun.e2e ? "Ready for your review" : "Agent delivery check required"
                        : `Verification ${activeRun.outcome}`}</b
                    ><small
                      >{activeRun.submitted_by} · {date(
                        activeRun.submitted_at,
                      )}</small
                    >
                  </div>
                </div>
                <div class="section-label">WHAT CHANGED</div>
                <p class="prose">{activeRun.summary}</p>
                <dl class="evidence">
                  <dt>Tested build</dt>
                  <dd>{activeRun.build_ref}</dd>
                  <dt>Delivery</dt>
                  <dd>{activeRun.delivery_ref}</dd>
                  <dt>Checks</dt>
                  <dd>{activeRun.checks}</dd>
                  {#if activeRun.e2e}
                    <dt>Agent end-to-end evidence</dt>
                    <dd>{activeRun.e2e.environment} · {activeRun.e2e.entrypoint}</dd>
                    <dd class="prose">{activeRun.e2e.scenarios}</dd>
                    <dt>Delivered build checked</dt>
                    <dd><code>{activeRun.e2e.delivered_build_ref}</code> · {activeRun.e2e.delivery_check}</dd>
                  {:else}
                    <dt>End-to-end evidence missing</dt>
                    <dd>This earlier handoff has no recorded end-to-end delivery check. The agent must test the delivered build and resubmit before requesting your review.</dd>
                  {/if}
                  {#if activeRun.limitations}<dt>Limitations</dt>
                    <dd>{activeRun.limitations}</dd>{/if}
                </dl>
                {#if activeRun.preconditions}<div class="section-label">
                    BEFORE YOU TEST
                  </div>
                  <p class="prose">{activeRun.preconditions}</p>{/if}
                <div class="section-label">
                  VERIFICATION STEPS <span>{activeRun.steps.length}</span>
                </div>
                {#each activeRun.steps as step, index}<div class="test-step">
                    <div class="step-number">{index + 1}</div>
                    <div>
                      <b>{step.instruction}</b>
                      <p>{step.expected}</p>
                      {#if activeRun.outcome === "pending" && results[index]}<select
                          aria-label={`Result for step ${index + 1}`}
                          bind:value={results[index].outcome}
                          ><option value="pending">Not tested</option><option
                            value="passed">Passed</option
                          ><option value="failed">Failed</option></select
                        ><input
                          aria-label={`Evidence for step ${index + 1}`}
                          bind:value={results[index].note}
                          placeholder="Evidence or observation (optional)"
                        />{:else}<span class="result-label"
                          >{activeRun.results[index]?.outcome ||
                            "Not recorded"}</span
                        >{#if activeRun.results[index]?.note}<p>
                            {activeRun.results[index].note}
                          </p>{/if}{/if}
                    </div>
                  </div>{/each}
                {#if activeRun.outcome === "pending"}<label class="field"
                    >Review notes<textarea
                      rows="3"
                      bind:value={reviewNote}
                      placeholder="What worked, what failed, or why testing is canceled…"
                    ></textarea></label
                  >
                  <div class="review-actions">
                    <button
                      class="primary"
                      disabled={busy ||
                        !activeRun.e2e ||
                        results.some((r) => r.outcome !== "passed")}
                      onclick={() => review("passed")}>Pass & complete ✓</button
                    ><button
                      class="danger-button"
                      disabled={busy ||
                        !reviewNote.trim() ||
                        !results.some((r) => r.outcome === "failed")}
                      onclick={() => review("failed")}>Return for fixes</button
                    ><button
                      class="text-button"
                      disabled={busy || !reviewNote.trim()}
                      onclick={() => review("canceled")}>Cancel test</button
                    >
                  </div>
                  <p class="hint">
                    Every step must pass. Canceling never completes the work.
                  </p>{:else}<div class="info-card">
                    <div>
                      <b>Reviewed by {activeRun.reviewed_by}</b>
                      <p>
                        {activeRun.review_note || "All required steps passed."}
                      </p>
                    </div>
                  </div>{/if}
              {:else}<div class="empty compact">
                  <div class="empty-symbol">◈</div>
                  <h3>No submitted verification</h3>
                  <p>
                    A handoff will bring the tested build, checks, and
                    walkthrough here.
                  </p>
                </div>{/if}
              {#if context && context.verifications.length > 1}<details>
                  <summary
                    >Previous verification runs ({context.verifications.length -
                      1})</summary
                  >{#each context.verifications.filter((v) => v.id !== activeRun?.id) as run}<div
                      class="history-run"
                    >
                      <b>{run.outcome} · {run.build_ref}</b>
                      <p>{run.review_note || run.summary}</p>
                      <small>{date(run.submitted_at)}</small>
                    </div>{/each}
                </details>{/if}
            {:else}
              {#if context?.git_traces?.length}<div class="section-label">
                  GIT EVIDENCE <span>{context.git_traces.length}</span>
                </div>
                {#each [...(context.git_traces || [])].reverse() as trace}<article
                    class="git-trace"
                  >
                    <div class="git-trace-icon">{trace.kind === "push" ? "↥" : "◆"}</div>
                    <div>
                      <b>{trace.kind === "push" ? "Pushed" : "Committed"} <code
                          title={trace.commit_sha}>{shortSha(trace.commit_sha)}</code
                        ></b>
                      <p>
                        <code>{trace.repository}</code> · <code>{trace.branch}</code>
                        {#if trace.kind === "push"}<br />to <code
                            >{trace.remote} · {trace.remote_ref}</code
                          >{/if}
                      </p>
                      <small>{trace.recorded_by} · {date(trace.recorded_at)}</small>
                    </div>
                  </article>{/each}
                <div class="section-label spaced">DISCUSSION</div>
              {:else}<div class="section-label">DISCUSSION</div>{/if}
              {#each context?.comments || [] as c}<article class="comment">
                  <b>{c.actor}</b><small>{date(c.at)}</small>
                  <p class="prose">{c.body}</p>
                </article>{/each}{#if !context?.comments.length}<p
                  class="muted"
                >
                  Keep decisions and handoff context with the work.
                </p>{/if}
              <form
                class="comment-form"
                onsubmit={(e) => {
                  e.preventDefault();
                  postComment(current.key);
                }}
              >
                <label class="field"
                  >Add a comment<textarea
                    bind:value={
                      () =>
                        String(
                          commentAttempts[current!.key]?.body ??
                            commentDrafts[current!.key] ??
                            "",
                        ),
                      (text) => (commentDrafts[current!.key] = text)
                    }
                    onkeydown={(e) => {
                      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                        e.preventDefault();
                        e.currentTarget.form?.requestSubmit();
                      }
                    }}
                    readonly={!!commentAttempts[current.key] || postingKey === current.key}
                    rows="3"
                    required
                    placeholder="Add evidence, a decision, or context…"
                  ></textarea></label
                >{#if commentAttempts[current.key]}<div class="info-card warning comment-outcome" role="alert">
                    <span class="card-symbol">!</span>
                    <div>
                      <b>Direct did not confirm this comment</b>
                      <p>
                        It may already be posted to {current.key}. Retrying sends the
                        identical request, so it is added at most once.
                      </p>
                      <button
                        type="button"
                        class="text-button"
                        onclick={() => delete commentAttempts[current!.key]}
                        >Stop retrying and edit (posting again may add a second copy)</button
                      >
                    </div>
                  </div>{:else if (commentDrafts[current.key] || "").trim()}<small class="hint draft-hint"
                    >Draft kept with {current.key} when you switch issues · Ctrl+Enter posts</small
                  >{/if}<button
                  class="secondary"
                  disabled={busy ||
                    !(commentAttempts[current.key] || (commentDrafts[current.key] || "").trim())}
                  >{postingKey === current.key
                    ? "Posting…"
                    : commentAttempts[current.key]
                      ? "Retry posting"
                      : "Add comment"}</button
                >
              </form>
              <div class="section-label spaced">HISTORY</div>
              {#each context?.history || [] as entry}<div class="activity">
                  <span class="activity-dot"></span>
                  <div>
                    <b>{entry.kind.replaceAll("_", " ")}</b><small
                      >{entry.actor} · {date(entry.at)}</small
                    >
                  </div>
                </div>{/each}
            {/if}
            {#if ["done", "verify"].includes(current.status)}<details
                class="reopen"
              >
                <summary>Reopen work</summary>
                <p class="hint">Changed work needs a fresh verification run.</p>
                <input
                  aria-label="Reason for reopening"
                  bind:value={reopenReason}
                  placeholder="Why does this need another pass?"
                /><button
                  class="secondary"
                  disabled={busy || !reopenReason.trim()}
                  onclick={async () => {
                    if (
                      await act({
                        op: "reopen",
                        key: current.key,
                        expected_version: current.version,
                        reason: reopenReason,
                      })
                    )
                      reopenReason = "";
                  }}>Reopen with reason</button
                >
              </details>{/if}
          </div>
        {:else}<div class="detail-placeholder">
            <div class="outline-mark">↗</div>
            <h2>Make the next step clear.</h2>
            <p>
              Select an issue to see its brief,<br />handoff, and verification
              evidence.
            </p>
            <div class="mini-flow">
              <span>Intent</span><i>→</i><span>Work</span><i>→</i><span
                >Proof</span
              >
            </div>
          </div>{/if}
      </aside>
      {/if}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="pane-resizer right"
        role="separator"
        aria-label="Resize detail panel"
        aria-orientation="vertical"
        aria-valuemin={RIGHT_MIN}
        aria-valuemax={rightMax}
        aria-valuenow={rightWidth}
        tabindex="0"
        title="Drag to resize · double-click to reset · Enter to expand or restore"
        onpointerdown={(event) => startResize("right", event)}
        onpointermove={moveResize}
        onpointerup={endResize}
        onpointercancel={endResize}
        ondblclick={() => resetPane("right")}
        onkeydown={(event) => resizeKey("right", event)}
      ></div>
    </div>
  </main>
</div>

{#if modal}
  <div class="modal-backdrop">
    <dialog
      class="modal"
      use:showDialog
      oncancel={(event) => {
        // Escape never abandons a save that is still waiting for its reply.
        event.preventDefault();
        if (!formPending) modal = null;
      }}
      aria-label={modal === "project"
        ? projectDraft.id
          ? "Edit project"
          : "New project"
        : modal === "goal"
          ? goalDraft.id
            ? "Edit goal"
            : "New goal"
          : modal === "milestone"
            ? milestoneDraft.id
              ? "Edit milestone"
              : "New milestone"
          : modal === "release"
            ? releaseDraft.id
              ? "Edit release"
              : "New release"
          : modal === "workflow"
            ? "Release workflow setup"
        : modal === "label"
          ? labelDraft.id
            ? "Edit label"
            : "New label"
        : modal === "section"
          ? sectionDraft.id
            ? "Edit sidebar section"
            : "New sidebar section"
        : modal === "productPaths"
          ? "Product settings"
        : modal === "product"
          ? "New product"
          : modal === "submit"
            ? "Submit for verification"
            : modal === "delete"
              ? "Delete issue"
            : modal === "edit"
              ? "Edit issue"
              : "New issue"}
      tabindex="-1"
    >
      <div class="modal-header">
        <div>
          <div class="eyebrow">
            DIRECT / {modal === "submit" ? "HANDOFF" : "WORKSPACE"}
          </div>
          <h2>
            {modal === "project"
              ? projectDraft.id
                ? "Shape the project"
                : "Group work into a project"
              : modal === "goal"
                ? goalDraft.id
                  ? "Shape the goal"
                  : "Connect projects to an outcome"
                : modal === "milestone"
                  ? milestoneDraft.id
                    ? "Shape the milestone"
                    : "Add a project phase"
                : modal === "release"
                  ? releaseDraft.id
                    ? "Shape the release"
                    : "Define a delivery boundary"
                : modal === "workflow"
                  ? "Configure release delivery"
              : modal === "label"
                ? labelDraft.id
                  ? "Refine a shared label"
                  : "Define a shared label"
              : modal === "section"
                ? sectionDraft.id
                  ? "Rename or remove the section"
                  : "Group products in the sidebar"
              : modal === "productPaths"
                ? `Paths for ${pathsDraft.name}`
              : modal === "product"
                ? "A space for your product"
                : modal === "submit"
                  ? "Hand it back with evidence"
                  : modal === "delete"
                    ? "Delete this issue?"
                  : modal === "edit"
                    ? "Shape the work"
                    : "Capture an issue"}
          </h2>
        </div>
        <button
          class="icon-button"
          aria-label="Close dialog"
          disabled={formPending}
          title={formPending ? "Waiting for Direct to reply to this save" : undefined}
          onclick={() => (modal = null)}>×</button
        >
      </div>
      {#if error}<div class="error" role="alert">{error}</div>{/if}
      {#if formAttempt}<div class="info-card warning form-outcome" role="alert">
          <span class="card-symbol">!</span>
          <div>
            <b>Direct did not confirm this save</b>
            <p>
              The connection dropped before Direct replied, so it may already be
              saved. Your entries are kept and locked until the outcome is
              known. Retrying sends the identical request, so Direct applies it
              at most once.
            </p>
            <div class="card-actions">
              <button type="button" class="primary" disabled={busy} onclick={retryForm}
                >Retry the same save</button
              ><button type="button" class="secondary" onclick={() => (modal = null)}
                >Close and check the workspace</button
              >
            </div>
          </div>
        </div>{/if}
      {#if formConflict?.kind === "edit"}<div class="info-card warning form-outcome">
          <span class="card-symbol">i</span>
          <div>
            <b>{draft.key} changed while you were editing</b>
            <p>
              Nothing was saved and your text is kept.{formConflict.latest
                ? ` It is now version ${formConflict.latest.version} (you started from version ${draft.version}).`
                : ""} Load the latest version to merge: changes only the other side made are adopted, your own changes stay, and fields you both changed keep your text with theirs shown for comparison.
            </p>
            <div class="card-actions">
              <button type="button" class="primary" disabled={busy} onclick={mergeLatest}
                >Load latest and merge</button
              >
            </div>
          </div>
        </div>{:else if formConflict?.kind === "merged"}<div class="info-card form-outcome" role="status">
          <span class="card-symbol">✓</span>
          <div>
            <b>Merged with version {draft.version} — review, then save</b>
            {#if formConflict.theirs.length}<p>
                Adopted the other change to {formConflict.theirs.join(", ")}.
              </p>{/if}
            {#if formConflict.both.length}{#each formConflict.both as field}<details open>
                  <summary>You both changed {field.field} — your text is kept; theirs:</summary>
                  <pre class="prose">{field.theirs}</pre>
                </details>{/each}{:else}<p>None of your changes collide with theirs.</p>{/if}
          </div>
        </div>{:else if formConflict?.kind === "submit"}<div class="info-card warning form-outcome">
          <span class="card-symbol">i</span>
          <div>
            <b>{current?.key} changed since you opened this handoff</b>
            <p>
              Nothing was submitted and your handoff is kept.{formConflict.latest
                ? ` It is now version ${formConflict.latest.version} · ${labels[formConflict.latest.status]}${formConflict.latest.claim ? ` · claimed by ${formConflict.latest.claim.actor}` : " · unclaimed"}.`
                : ""} Review the change, then submit against the current version.
            </p>
            <div class="card-actions">
              <button type="button" class="primary" disabled={busy} onclick={useCurrentVersion}
                >Use the current version</button
              >
            </div>
          </div>
        </div>{/if}
      {#if formPending}<div class="info-card form-outcome" role="status">
          <span class="card-symbol">…</span>
          <div>
            <b>Saving — waiting for Direct to reply</b>
            <p>This form stays open and locked until Direct answers, so the reply cannot land on other work.</p>
          </div>
        </div>{/if}
      {#if formNotice}<div class="info-card form-outcome" role="status">
          <span class="card-symbol">i</span>
          <div><b>{formNotice}</b></div>
        </div>{/if}
      <fieldset class="modal-forms" disabled={formPending || !!formAttempt}>
      {#if modal === "delete"}<form class="modal-form" onsubmit={deleteIssue}>
          <div class="info-card warning">
            <span class="card-symbol">!</span>
            <div>
              <b>{current?.key} · {current?.title}</b>
              <p>
                This permanently removes an unstarted issue with no comments, links, explicit release references, submissions, or recorded evidence. Direct checks again when you confirm. The key stays reserved in activity history and is never reused.
              </p>
            </div>
          </div>
          {#if deletion && !deletion.eligible}<div class="info-card deletion-blockers" role="alert">
              <span class="card-symbol">i</span>
              <div>
                <b>This issue changed and can no longer be deleted</b>
                <ul>
                  {#each deletion.blockers as blocker}<li>{blocker.message}.</li>{/each}
                </ul>
              </div>
            </div>{/if}
          <div class="modal-footer">
            <button type="button" class="secondary" onclick={() => (modal = null)}
              >Keep issue</button
            ><button class="danger-button" disabled={busy || !deletion?.eligible}
              >Delete {current?.key}</button
            >
          </div>
        </form>
      {:else if modal === "project"}<form class="modal-form" onsubmit={saveProject}>
          <label class="field"
            >Product<select
              bind:value={projectDraft.product}
              disabled={!!projectDraft.id}
            >
              {#each data.products as p}<option value={p.key}>{p.name}</option
                >{/each}
            </select></label
          >
          {#if !projectDraft.id}<TemplatePicker
              {data}
              target="project"
              productId={data.products.find((p) => p.key === projectDraft.product)?.id || ""}
              bind:selection={projectTemplate}
              onapply={applyProjectTemplate}
            />{/if}
          <label class="field"
            >Project name<input
              required
              maxlength="160"
              bind:value={projectDraft.name}
              placeholder="e.g. Direct pilot"
            /></label
          >
          <label class="field"
            >Project outcome<textarea
              rows="3"
              bind:value={projectDraft.description}
              placeholder="What should this project deliver?"></textarea></label
          >
          <div class="form-grid">
            {#if projectDraft.id}<label class="field"
                >Status<select bind:value={projectDraft.status}
                  ><option value="planned">Planned</option><option value="active"
                    >Active</option
                  ><option value="paused">Paused</option><option value="completed"
                    >Completed</option
                  ><option value="canceled">Canceled</option></select
                ></label
              >{/if}<label class="field"
              >Priority<select
                bind:value={projectDraft.priority}
                onchange={() => (touched.priority = true)}
                ><option value="urgent">Urgent</option><option value="high"
                  >High</option
                ><option value="medium">Medium</option><option value="low"
                  >Low</option
                ></select
              ></label
            ><label class="field"
              >Order<input
                type="number"
                min="0"
                max="1000000"
                bind:value={projectDraft.sort_order}
              /></label
            >
          </div>
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >{projectDraft.id ? "Save project" : "Create project"}</button
            >
          </div>
        </form>
        {#if projectDraft.id}<div class="modal-context"><ContextDocs {data} {connected} {commit} targetKind="project" target={projectDraft.id} links={linksFor(data.context_links, "project", projectDraft.id)} productId={data.products.find((p) => p.key === projectDraft.product)?.id || ""} /></div>{/if}
      {:else if modal === "goal"}<form class="modal-form" onsubmit={saveGoal}>
          <label class="field"
            >Product<select bind:value={goalDraft.product} disabled={!!goalDraft.id}>
              {#each data.products as p}<option value={p.key}>{p.name}</option
                >{/each}
            </select></label
          >
          <label class="field"
            >Goal / initiative name<input required maxlength="160" bind:value={goalDraft.name}
          /></label>
          <label class="field"
            >Outcome<textarea rows="3" bind:value={goalDraft.description}></textarea></label
          >
          <div class="form-grid">
            {#if goalDraft.id}<label class="field"
                >Status<select bind:value={goalDraft.status}
                  ><option value="planned">Planned</option><option value="active"
                    >Active</option
                  ><option value="paused">Paused</option><option value="completed"
                    >Completed</option
                  ><option value="canceled">Canceled</option></select
                ></label
              >{/if}<label class="field"
              >Priority<select bind:value={goalDraft.priority}
                ><option value="urgent">Urgent</option><option value="high">High</option
                ><option value="medium">Medium</option><option value="low">Low</option
                ></select
              ></label
            >
          </div>
          <div class="field">
            <span>Linked projects</span>
            <div class="project-checklist">
              {#each data.projects.filter((project) => project.product_id === data.products.find((candidate) => candidate.key === goalDraft.product)?.id) as project}
                <label><input type="checkbox" value={project.id} bind:group={goalDraft.project_ids} /> {project.name}</label>
              {:else}<small>No projects in this product yet.</small>{/each}
            </div>
          </div>
          {#if goalDraft.id && data.goals.find((goal) => goal.id === goalDraft.id)?.external_source}<p class="source-warning"
              >Imported from {data.goals.find((goal) => goal.id === goalDraft.id)?.external_source} · {data.goals.find((goal) => goal.id === goalDraft.id)?.external_id}</p
            >{/if}
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >{goalDraft.id ? "Save goal" : "Create goal"}</button
            >
          </div>
        </form>
        {#if goalDraft.id}<div class="modal-context"><ContextDocs {data} {connected} {commit} targetKind="goal" target={goalDraft.id} links={linksFor(data.context_links, "goal", goalDraft.id)} productId={data.products.find((p) => p.key === goalDraft.product)?.id || ""} /></div>{/if}
      {:else if modal === "milestone"}<form class="modal-form" onsubmit={saveMilestone}>
          <label class="field"
            >Project<select disabled value={milestoneDraft.project_id}>
              {#each data.projects as project}<option value={project.id}>{project.name}</option>{/each}
            </select></label
          >
          <label class="field"
            >Milestone name<input required maxlength="160" bind:value={milestoneDraft.name}
          /></label>
          <label class="field"
            >Phase outcome<textarea rows="3" bind:value={milestoneDraft.description}></textarea></label
          >
          <label class="field"
            >Order<input type="number" min="0" max="1000000" bind:value={milestoneDraft.sort_order}
          /></label>
          {#if milestoneDraft.id && data.milestones.find((milestone) => milestone.id === milestoneDraft.id)?.external_source}<p class="source-warning"
              >Imported from {data.milestones.find((milestone) => milestone.id === milestoneDraft.id)?.external_source} · {data.milestones.find((milestone) => milestone.id === milestoneDraft.id)?.external_id}</p
            >{/if}
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >{milestoneDraft.id ? "Save milestone" : "Create milestone"}</button
            >
          </div>
        </form>
      {:else if modal === "release"}<form class="modal-form" onsubmit={saveRelease}>
          <label class="field"
            >Product<select bind:value={releaseDraft.product} disabled={!!releaseDraft.id}>
              {#each data.products as p}<option value={p.key}>{p.name}</option>{/each}
            </select></label
          >
          <div class="form-grid">
            <label class="field">Release name<input required maxlength="160" bind:value={releaseDraft.name} placeholder="Autumn pilot" /></label>
            <label class="field">Version<input required maxlength="80" bind:value={releaseDraft.version_label} oninput={() => {
              if (!releaseDraft.id) {
                const workflow = workflowForProductKey(releaseDraft.product);
                if (workflow?.branch_strategy === "one_branch_per_release")
                  releaseDraft.release_branch = workflow.release_branch_pattern.replace("{version}", releaseDraft.version_label);
              }
            }} placeholder="v0.2.0" /></label>
          </div>
          <label class="field">Target branch or ref<input required maxlength="512" bind:value={releaseDraft.target_ref} placeholder="refs/heads/main" /></label>
          <label class="field">Release branch<input maxlength="512" bind:value={releaseDraft.release_branch} placeholder={workflowForProductKey(releaseDraft.product)?.release_branch_pattern || "Optional for external strategy"} /></label>
          <label class="field">Planned notes<textarea rows="3" bind:value={releaseDraft.notes} placeholder="Scope, risks, and rollout intent"></textarea></label>
          <label class="field">Preview URL (optional)<input type="url" bind:value={releaseDraft.preview_url} placeholder="https://preview.example.com" /></label>
          {#if releaseDraft.id}<label class="field">Lifecycle status<select bind:value={releaseDraft.status}>
              <option value="planned">Planned</option>
              <option value="active">Active</option>
              <option value="preview">Preview (evidence-controlled)</option>
              <option value="production">Production (evidence-controlled)</option>
              <option value="retired">Retired</option>
              <option value="canceled">Canceled</option>
            </select></label>{/if}
          <p class="hint">Preview and production status can only be reached by recording exact deployment evidence. A push or merge alone never promotes a release.</p>
          <div class="field">
            <span>Linked projects</span>
            <div class="project-checklist">
              {#each data.projects.filter((project) => project.product_id === data.products.find((candidate) => candidate.key === releaseDraft.product)?.id) as project}
                <label><input type="checkbox" value={project.id} bind:group={releaseDraft.project_ids} /> {project.name}</label>
              {:else}<small>No projects in this product yet.</small>{/each}
            </div>
          </div>
          <div class="field">
            <span>Additional linked issues</span>
            <div class="project-checklist">
              {#each data.issues.filter((issue) => !issue.parent && issue.product_id === data.products.find((candidate) => candidate.key === releaseDraft.product)?.id) as issue}
                <label><input type="checkbox" value={issue.key} bind:group={releaseDraft.issue_keys} /> {issue.key} · {issue.title}</label>
              {:else}<small>No real issues in this product yet.</small>{/each}
            </div>
          </div>
          {#if releaseDraft.id}
            {@const evidence = data.release_evidence.filter((item) => item.release_id === releaseDraft.id)}
            <div class="section-label">RECORDED EVIDENCE <span>{evidence.length}</span></div>
            {#each evidence as item}<div class="activity">
                <span class="activity-dot"></span><div><b>{item.kind.replaceAll("_", " ")}</b><small>{releaseEvidenceSummary(item)} · {date(item.recorded_at)}{item.approver ? ` · approved by ${item.approver}` : ""}</small>{#if item.url}<a href={item.url} target="_blank" rel="noreferrer">Open deployment</a>{/if}</div>
              </div>{:else}<p class="muted">No commit, push, check, or deployment evidence has been recorded.</p>{/each}
          {/if}
          <div class="modal-footer"><button class="primary" disabled={busy}>{releaseDraft.id ? "Save release" : "Create release"}</button></div>
        </form>
        {#if releaseDraft.id}<div class="modal-context"><ContextDocs {data} {connected} {commit} targetKind="release" target={releaseDraft.id} links={linksFor(data.context_links, "release", releaseDraft.id)} productId={data.products.find((p) => p.key === releaseDraft.product)?.id || ""} /></div>{/if}
      {:else if modal === "workflow"}<form class="modal-form" onsubmit={saveWorkflow}>
          <label class="field">Product<select bind:value={workflowDraft.product} disabled={workflowDraft.expected_version !== null}>
              {#each data.products as p}<option value={p.key}>{p.name}</option>{/each}
            </select></label>
          <label class="field">Branch strategy<select bind:value={workflowDraft.branch_strategy}>
              <option value="one_branch_per_release">One branch per release</option>
              <option value="external">External / repository-specific strategy</option>
            </select></label>
          <div class="form-grid">
            <label class="field">Production branch or ref<input required bind:value={workflowDraft.production_ref} /></label>
            <label class="field">Release branch convention<input required={workflowDraft.branch_strategy === "one_branch_per_release"} bind:value={workflowDraft.release_branch_pattern} placeholder={"refs/heads/release/{version}"} /></label>
          </div>
          <div class="form-grid">
            <label class="field">Preview environment<input required bind:value={workflowDraft.preview_environment} placeholder="preview" /></label>
            <label class="field">Preview URL template<input bind:value={workflowDraft.preview_url_template} placeholder={"https://preview.example.com/{version}"} /></label>
          </div>
          <label class="field">Promotion policy<select bind:value={workflowDraft.promotion_policy}>
              <option value="verified_owner_approval">Verified work + explicit owner approval</option>
              <option value="external_manual">External/manual repository promotion</option>
            </select></label>
          <p class="hint">External strategy opts out of one-branch-per-release. Direct still records attempts and evidence only after external commands finish; it never runs Git or deployment commands itself.</p>
          <div class="modal-footer"><button class="primary" disabled={busy}>Save release workflow</button></div>
        </form>
      {:else if modal === "label"}<form class="modal-form" onsubmit={saveLabel}>
          <div class="form-grid">
            <label class="field"
              >Label name<input
                required
                maxlength="80"
                bind:value={labelDraft.name}
                placeholder="e.g. Bug"
              /></label
            ><label class="field"
              >Color<input
                bind:value={labelDraft.color}
                pattern={"#[0-9a-fA-F]{6}"}
                maxlength="7"
                placeholder="#d73a49 (optional)"
              /></label
            >
          </div>
          <label class="field"
            >Description<input
              maxlength="1000"
              bind:value={labelDraft.description}
              placeholder="When should this label be used?"
            /></label
          >
          <label class="field"
            >Aliases<input
              bind:value={labelDraft.aliases}
              placeholder="Comma-separated, e.g. defect, bugfix"
            /><small
              >Aliases map imported or informal names to this one canonical
              label. Names and aliases are unique across the workspace.</small
            ></label
          >
          <div class="section-label">PRODUCT APPLICABILITY</div>
          <p class="hint">
            Leave every product unchecked to apply this label everywhere.
            Checked products restrict it; “default” attaches it to new issues in
            that product. One definition is shared either way.
          </p>
          {#each labelDraft.products as rule}<div class="label-rule">
              <label
                ><input type="checkbox" bind:checked={rule.applies} />
                {rule.name}</label
              ><label class:disabled={!rule.applies}
                ><input
                  type="checkbox"
                  disabled={!rule.applies}
                  bind:checked={rule.default_for_new_issues}
                /> Default for new issues</label
              >
            </div>{/each}
          <label class="field"
            >Linear origins (optional)<textarea
              rows="2"
              bind:value={labelDraft.linear_origins}
              placeholder="One per line: linear-label-id | Original name"
            ></textarea><small
              >Preserved for a later import; a Linear ID can map to only one
              canonical label.</small
            ></label
          >
          <p class="hint">
            Labels are filtering metadata only. They never change readiness,
            priority, ownership, or verification.
          </p>
          <div class="modal-footer">
            <span class="hint"
              >{labelDraft.id ? `Editing version ${labelDraft.version}` : ""}</span
            ><button class="primary" disabled={busy}
              >{labelDraft.id ? "Save label" : "Create label"}</button
            >
          </div>
        </form>
      {:else if modal === "section"}<form class="modal-form" onsubmit={saveSection}>
          <label class="field"
            >Section name<input
              required
              maxlength="80"
              disabled={formPending}
              bind:value={sectionDraft.name}
              placeholder="e.g. Teroka"
            /></label
          >
          <p class="hint">
            {sectionDraft.id
              ? "Drag products onto the section in the sidebar to add or remove them."
              : "The section appears in the sidebar under Products. Drag products onto it to group them."}
          </p>
          {#if sectionDraft.id && sectionDraft.confirmDelete}<div class="info-card warning" role="alert">
              <span class="card-symbol">!</span>
              <div>
                <b>Delete “{sectionDraft.name}”?</b>
                <p>
                  {sectionDraft.products
                    ? `Its ${sectionDraft.products} ${sectionDraft.products === 1 ? "product returns" : "products return"} to the ungrouped list. No product or issue is deleted.`
                    : "It has no products. No product or issue is deleted."}
                </p>
              </div>
            </div>{/if}
          <div class="modal-footer">
            {#if sectionDraft.id}<button
                type="button"
                class="danger-button"
                disabled={busy}
                onclick={deleteSection}
                >{sectionDraft.confirmDelete ? "Delete section" : "Delete section…"}</button
              >{/if}
            <span class="hint"
              >{sectionDraft.id ? `Editing version ${sectionDraft.version}` : ""}</span
            ><button class="primary" disabled={busy}
              >{sectionDraft.id ? "Save section" : "Create section"}</button
            >
          </div>
        </form>
      {:else if modal === "productPaths"}<form class="modal-form" onsubmit={saveProductPaths}>
          <p class="hint">
            Where this product's code and Obsidian knowledge live. Context documents resolve note
            paths inside the Windows vault path. Changing a path moves nothing and changes no issue.
          </p>
          <label class="field"
            >Windows vault path<input
              aria-label="Windows vault path"
              maxlength="500"
              bind:value={pathsDraft.vault_windows}
              placeholder="C:/Users/…/Obsidian/vault/Product notes"
            /></label
          >
          <label class="field"
            >WSL vault path<input aria-label="WSL vault path" maxlength="500" bind:value={pathsDraft.vault_wsl} /></label
          >
          <label class="field"
            >Windows repository path<input aria-label="Windows repository path" maxlength="500" bind:value={pathsDraft.repo_windows} /></label
          >
          <label class="field"
            >WSL repository path<input aria-label="WSL repository path" maxlength="500" bind:value={pathsDraft.repo_wsl} /></label
          >
          <div class="modal-footer">
            <button class="primary" disabled={busy}>Save paths</button>
          </div>
        </form>
      {:else if modal === "product"}<form class="modal-form" onsubmit={createProduct}>
          <div class="form-grid">
            <label class="field"
              >Product name<input
                required
                bind:value={productDraft.name}
                placeholder="e.g. Pernance"
              /></label
            ><label class="field"
              >Issue prefix<input
                required
                pattern={"[A-Z]{1,8}"}
                maxlength="8"
                bind:value={productDraft.key}
                placeholder="PFN"
              /></label
            >
          </div>
          <details>
            <summary>Repository & knowledge paths (optional)</summary
            >{#each ["repo_windows", "repo_wsl", "vault_windows", "vault_wsl"] as field}<label
                class="field"
                >{field.replace("_", " ")}<input
                  bind:value={productDraft[field as keyof typeof productDraft]}
                /></label
              >{/each}
          </details>
          <div class="modal-footer">
            <button class="primary" disabled={busy}>Create product</button>
          </div>
        </form>
      {:else if modal === "submit"}<form class="modal-form" onsubmit={submit}>
          <p class="hint">Finish the complete user flow and check the delivered build before asking for owner verification.</p>
          <label class="field">Test environment<input required bind:value={e2eDraft.environment} placeholder="Windows, isolated test workspace" /></label>
          <label class="field">Tested entrypoint<input required bind:value={e2eDraft.entrypoint} placeholder="App URL, shortcut, or client command" /></label>
          <label class="field">Acceptance scenarios and observed results<textarea required rows="4" bind:value={e2eDraft.scenarios} placeholder="For each criterion: action, expected behavior, observed result, and evidence location"></textarea></label>
          <label class="field">Delivered build<input required bind:value={e2eDraft.delivered_build_ref} placeholder="Must match the tested build below" /></label>
          <label class="field">Delivery check<textarea required rows="2" bind:value={e2eDraft.delivery_check} placeholder="Where the build is running and how you confirmed the user can access the change"></textarea></label>
          <label class="field"
            >What changed<textarea
              rows="2"
              required
              bind:value={handoff.summary}></textarea></label
          >
          <div class="form-grid">
            <label class="field"
              >Tested build / commit<input
                required
                bind:value={handoff.build_ref}
                placeholder="Commit SHA or build identifier"
              /></label
            ><label class="field"
              >Delivery reference<input
                required
                bind:value={handoff.delivery_ref}
                placeholder="Merged PR, commit, or release"
              /></label
            >
          </div>
          <label class="field"
            >Checks run and results<textarea
              required
              rows="2"
              bind:value={handoff.checks}></textarea></label
          ><label class="field"
            >Known limitations<input bind:value={handoff.limitations} /></label
          ><label class="field"
            >Test preconditions<input
              bind:value={handoff.preconditions}
            /></label
          >
          <div class="section-label">WALKTHROUGH</div>
          {#each handoff.steps as step, index}<div class="form-grid step-form">
              <label class="field"
                >Step {index + 1}<input
                  required
                  bind:value={step.instruction}
                  placeholder="What should the reviewer do?"
                /></label
              ><label class="field"
                >Expected result<input
                  required
                  bind:value={step.expected}
                  placeholder="What should happen?"
                /></label
              >
            </div>{/each}<button
            class="text-button"
            type="button"
            onclick={() =>
              (handoff.steps = [
                ...handoff.steps,
                { instruction: "", expected: "" },
              ])}
            disabled={handoff.steps.length >= 50}>＋ Add step</button
          >
          <div class="modal-footer">
            <button class="primary" disabled={busy}
              >Submit for verification →</button
            >
          </div>
        </form>
      {:else}<form class="modal-form" onsubmit={saveDraft}>
          {#if modal === "issue"}<label class="field"
              >Product<select
                value={draft.product}
                onchange={(event) => {
                  draft.product = event.currentTarget.value;
                  draft.project_id = "";
                }}
                >{#each data.products as p}<option value={p.key}
                    >{p.name} · {p.key}</option
                  >{/each}</select
              ></label
            ><TemplatePicker
              {data}
              target="issue"
              productId={data.products.find((p) => p.key === draft.product)?.id || ""}
              bind:selection={issueTemplate}
              onapply={applyIssueTemplate}
            />{/if}<label class="field"
            >Work route<select
              bind:value={draft.planning_scope}
              onchange={() => (touched.planning_scope = true)}
              ><option value="project">Project work · feature or change</option><option
                value="inbox">Inbox / exceptional maintenance</option
              ></select
            ></label
          >{#if modal === "issue" && draft.planning_scope === "project"}<label
              class="field">Project<select required bind:value={draft.project_id}
                ><option value="" disabled>Choose the delivery scope</option>{#each data.projects.filter(
                    (project) =>
                      project.product_id ===
                      data.products.find((product) => product.key === draft.product)
                        ?.id,
                  ) as project}<option value={project.id}>{project.name}</option
                  >{/each}</select
              ><small
                >Feature and change work enters the project directly. Use Inbox only when ungrouped capture is intentional.</small
              ></label
            >{/if}<label class="field"
            >Issue title<input
              required
              bind:value={draft.title}
              placeholder="What needs to change?"
            /></label
          >{#key `${modal}:${draft.key}`}
            <DraftAssistant title={draft.title} bind:body={draft.body} bind:acceptance={draft.acceptance}
              bind:intake={draft.intake} product={draft.product || data.products.find(p => p.id === current?.product_id)?.name || ""}
              project={data.projects.find(p => p.id === draft.project_id)?.name || ""} disabled={busy} />
          {/key}
          <label class="field"
            >Problem & expected outcome<textarea
              rows="4"
              required={modal === "edit"}
              bind:value={draft.body}
              placeholder="Give the next person or agent enough context to start."
            ></textarea></label
          ><label class="field"
              >Acceptance criteria<textarea
                rows="3"
                required={modal === "edit"}
                bind:value={draft.acceptance}
                placeholder="How will we know this is complete?"
              ></textarea></label
            >
            <div class="form-grid">
              <label class="field"
                >Human owner<select required={modal === "edit"} bind:value={draft.owner}
                  ><option value="" disabled>Choose a reviewer</option>{#each ownerOptions as owner}<option
                      value={owner}>{owner}</option
                    >{/each}{#if draft.owner && !ownerOptions.includes(draft.owner)}<option
                      value={draft.owner}>{draft.owner}</option
                    >{/if}</select
                ></label
              ><label class="field"
                >Priority<select
                  bind:value={draft.priority}
                  onchange={() => (touched.priority = true)}
                  ><option value="low">Low</option><option value="medium"
                    >Medium</option
                  ><option value="high">High</option><option value="urgent"
                    >Urgent</option
                  ></select
                ></label
              >
            </div>
          {#if ownerOptions.length > 1}
            <p class="hint">For a workspace with one human reviewer, consolidate all existing issue owners. New issues will default to that reviewer.</p>
            {#if consolidationOwner}
              <p class="hint">Replace all existing named human owners with {consolidationOwner}? Unassigned issues, statuses and verification history will stay unchanged.</p>
              <button type="button" disabled={formPending} onclick={consolidateOwners}>Confirm owner consolidation</button>
              <button type="button" disabled={formPending} onclick={() => (consolidationOwner = null)}>Cancel owner consolidation</button>
            {:else}
              <button type="button" disabled={formPending || !draft.owner} onclick={() => (consolidationOwner = consolidationAttempt?.owner || draft.owner)}>Use selected owner for all issues</button>
            {/if}
          {/if}
          {#if modal === "edit"}
            <p class="hint">
              Editing Ready work returns it to Backlog for a fresh readiness
              decision.
            </p>{/if}
          <div class="modal-footer">
            <span class="hint"
              >{modal === "issue"
                ? "Captured now. Shaped when it matters."
                : `Editing version ${draft.version}`}</span
            ><button class="primary" disabled={busy}
              >{modal === "issue" ? "Create issue" : "Save brief"} →</button
            >
          </div>
        </form>{/if}
      </fieldset>
    </dialog>
  </div>
{/if}
