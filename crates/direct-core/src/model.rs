use crate::IntakeContext;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Human,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Backlog,
    Ready,
    Doing,
    Verify,
    Done,
    LegacyCompleted,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    pub key: String,
    pub name: String,
    pub repo_windows: String,
    pub repo_wsl: String,
    pub vault_windows: String,
    pub vault_wsl: String,
    /// Sidebar position (format 15). Legacy products share 0 and fall back to ID order.
    #[serde(default)]
    pub sort_order: i64,
    /// Owner-defined sidebar section (format 15); `None` lists the product ungrouped.
    #[serde(default)]
    pub section_id: Option<String>,
}

/// A named owner-defined group of products in the sidebar (format 15).
/// Sections organise navigation only; they never change product data or access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductSection {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One product's place in a complete sidebar arrangement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductPlacement {
    pub product_id: String,
    #[serde(default)]
    pub section_id: Option<String>,
}

/// Where a customer request came from (format 16).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignalSourceKind {
    Email,
    Call,
    Chat,
    Support,
    Sales,
    Interview,
    Survey,
    Social,
    Other,
}

/// What a customer request is linked to. Links reference work; the request
/// text lives only on the signal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SignalTargetKind {
    Issue,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignalLink {
    pub kind: SignalTargetKind,
    /// Issue key or project ID.
    pub target: String,
    #[serde(default)]
    pub linked_by: String,
    #[serde(default)]
    pub linked_at: i64,
}

/// An external reference an import could not map to Direct work. It is kept
/// even after the request is linked, so the original source stays traceable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedMapping {
    pub external_source: String,
    pub external_id: String,
    #[serde(default)]
    pub note: String,
}

/// A provenance-bearing customer request or feedback item (format 16). It is
/// intake data: it never makes work Ready, sets priority or records review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerSignal {
    pub id: String,
    pub product_id: String,
    pub source_kind: SignalSourceKind,
    /// Where to find the original, e.g. a ticket number or call note title.
    #[serde(default)]
    pub source_reference: String,
    /// The concise request, stored once.
    pub summary: String,
    pub received_at: i64,
    /// A privacy-safe customer reference (account or segment, not contact details).
    #[serde(default)]
    pub customer_reference: String,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub unresolved_mappings: Vec<UnresolvedMapping>,
    #[serde(default)]
    pub links: Vec<SignalLink>,
    /// The Inbox issue this request was promoted into, at most once.
    #[serde(default)]
    pub promoted_issue_key: Option<String>,
    #[serde(default)]
    pub archived: bool,
    pub version: u64,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What a context document is attached to (DIR-23).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContextTargetKind {
    Issue,
    Project,
    Goal,
    Release,
}

/// Where a context document lives. Obsidian notes and URLs are linked, never
/// copied; Linear documents are the read-only records already retained from
/// the Linear capture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextSource {
    /// A note in a product's knowledge vault, as a path relative to its root.
    Obsidian { product_id: String, path: String },
    /// A retained source record (for example a Linear document).
    RetainedRecord { record_id: String },
    /// An external page. Direct does not fetch it.
    Url { url: String },
}

/// One observation of a context document. Fingerprints are SHA-256 of the
/// exact bytes read; URLs are never fetched, so they have no fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextObservation {
    pub available: bool,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub bytes: Option<u64>,
    pub checked_at: i64,
    /// Why it is unavailable or unchecked; empty when available.
    #[serde(default)]
    pub reason: String,
}

/// A typed, addressable link from planning work to a durable document
/// (format 17). Content is reference data: it is read on demand, is never
/// Theoria guidance and never grants tool authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextLink {
    pub id: String,
    pub target_kind: ContextTargetKind,
    /// Issue key, or project/goal/release ID.
    pub target: String,
    pub source: ContextSource,
    pub title: String,
    #[serde(default)]
    pub note: String,
    /// The source system's own identifier, when known (e.g. the Linear document ID).
    #[serde(default)]
    pub source_id: Option<String>,
    /// The original address, when known.
    #[serde(default)]
    pub url: Option<String>,
    /// Fingerprint recorded when the link was made; rechecks never rewrite it.
    #[serde(default)]
    pub pinned_fingerprint: Option<String>,
    pub observation: ContextObservation,
    pub version: u64,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub actor: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Planned,
    #[default]
    Active,
    Paused,
    Completed,
    Canceled,
}

fn default_project_priority() -> String {
    "medium".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub status: ProjectStatus,
    #[serde(default = "default_project_priority")]
    pub priority: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Intake template provenance (format 13). Absent for untemplated records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateUse>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectProgress {
    pub project_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Planned,
    #[default]
    Active,
    Paused,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub status: GoalStatus,
    #[serde(default = "default_project_priority")]
    pub priority: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub sort_order: i64,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalProgress {
    pub goal_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MilestoneProgress {
    pub milestone_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

/// Optional per-product rule for a workspace-level label. A label with no rules
/// applies to every product; listed rules restrict it to those products.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelProductRule {
    pub product_id: String,
    /// Attach automatically to new issues captured in this product.
    #[serde(default)]
    pub default_for_new_issues: bool,
}

/// Original Linear label identity preserved for a later import.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinearLabelOrigin {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// One canonical definition shared by issues and projects across products.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub products: Vec<LabelProductRule>,
    #[serde(default)]
    pub linear_origins: Vec<LinearLabelOrigin>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanningScope {
    Project,
    #[default]
    Inbox,
}

/// The record kind a workspace template shapes. Fixed for the template's lifetime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateTarget {
    Issue,
    Project,
}

/// The intake shape a template revision describes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateShape {
    Delivery,
    DiscoveryProbe,
    Bug,
    Release,
}

/// How the work is expected to be carried out. `prototype` keeps the
/// prototype-as-planning route: a bounded probe whose outcome is learning.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Agent,
    Owner,
    Paired,
    Prototype,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateStatus {
    #[default]
    Active,
    Retired,
}

/// The shared base of one template revision: concise prompts and suggestions only.
/// Nothing here grants authority, makes work Ready, or counts as verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct TemplateContent {
    #[serde(default)]
    pub intent: String,
    #[serde(default)]
    pub execution_mode: Option<ExecutionMode>,
    #[serde(default)]
    pub boundaries: String,
    #[serde(default)]
    pub verification: String,
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default)]
    pub suggested_priority: Option<String>,
    /// Issue templates only.
    #[serde(default)]
    pub suggested_planning_scope: Option<PlanningScope>,
    /// Workspace label IDs.
    #[serde(default)]
    pub suggested_labels: Vec<String>,
}

/// An explicit, bounded, additive product supplement. It cannot replace any
/// base field; it only appends guidance, checklist items and label suggestions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TemplateSupplement {
    pub product_id: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default)]
    pub suggested_labels: Vec<String>,
}

/// Mutable head of an owner-managed template. Definitions live in immutable revisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTemplate {
    pub id: String,
    pub target: TemplateTarget,
    /// Mirrors the current revision for listing.
    pub name: String,
    pub shape: TemplateShape,
    pub status: TemplateStatus,
    pub current_revision: u32,
    #[serde(default)]
    pub retired_reason: Option<String>,
    #[serde(default)]
    pub retired_by: Option<String>,
    #[serde(default)]
    pub retired_at: Option<i64>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One immutable template revision. Never rewritten or deleted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateRevision {
    pub template_id: String,
    pub revision: u32,
    pub target: TemplateTarget,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub shape: TemplateShape,
    pub content: TemplateContent,
    #[serde(default)]
    pub supplements: Vec<TemplateSupplement>,
    #[serde(default)]
    pub note: String,
    pub created_by: String,
    pub created_at: i64,
}

/// What a creator asks for when applying a template at intake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TemplateSelection {
    pub template_id: String,
    /// Must be the template's current revision; a stale form is refused.
    pub revision: u32,
    /// Omit to accept the template's suggestion; `null` explicitly clears it;
    /// a mode explicitly chooses it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_value"
    )]
    pub execution_mode: Option<Option<ExecutionMode>>,
    /// Suggested labels (base or product supplement) the creator keeps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// Distinguishes an explicit `null` (`Some(None)`) from an absent field (`None`).
fn explicit_value<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Exact provenance recorded on an issue or project created from a template.
/// Later revisions never rewrite it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateUse {
    pub template_id: String,
    pub revision: u32,
    /// Set when the revision carried a supplement for the record's product.
    #[serde(default)]
    pub supplement_product_id: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<ExecutionMode>,
    /// Suggested fields the creator explicitly changed: `priority`,
    /// `planning_scope`, `execution_mode`, `labels`.
    #[serde(default)]
    pub overrides: Vec<String>,
    pub applied_by: String,
    pub applied_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TheoriaAvailability {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoriaDocumentInput {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub category: String,
    pub relative_path: String,
    #[serde(default)]
    pub source_updated: Option<String>,
    #[serde(default)]
    pub source_modified_at: Option<i64>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoriaDocument {
    pub id: String,
    pub product_id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub source_root: String,
    pub relative_path: String,
    pub source_updated: Option<String>,
    pub source_modified_at: Option<i64>,
    pub fingerprint: Option<String>,
    pub content: Option<String>,
    pub availability: TheoriaAvailability,
    pub unavailable_reason: Option<String>,
    pub catalog_version: u32,
    pub checked_at: i64,
    pub cached_at: Option<i64>,
    /// Owner-set workspace sharing (DIR-57): issues in any product may link it.
    /// The source product still owns the content; only its catalog sync updates it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoriaReference {
    pub document_id: String,
    pub recorded_fingerprint: Option<String>,
    pub playbook_version: Option<String>,
    pub linked_by: String,
    pub linked_at: i64,
    /// Linked from another product through workspace sharing. The pin stays
    /// valid if the document is later unshared (DIR-57).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingClassification {
    ProductDefect,
    MethodFriction,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Issue,
    Build,
    Check,
    OwnerReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePointer {
    pub kind: EvidenceKind,
    pub reference: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodFinding {
    pub id: String,
    pub issue_key: String,
    pub classification: FindingClassification,
    pub observation: String,
    pub hypothesis: String,
    pub proposal: String,
    pub evidence: Vec<EvidencePointer>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GitTraceKind {
    Commit,
    Push,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitTrace {
    pub id: String,
    pub issue_key: String,
    pub kind: GitTraceKind,
    pub repository: String,
    pub commit_sha: String,
    pub branch: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub remote_ref: Option<String>,
    pub recorded_by: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    #[default]
    Planned,
    Active,
    Preview,
    Production,
    Retired,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseRecord {
    pub id: String,
    pub product_id: String,
    pub name: String,
    pub version_label: String,
    pub status: ReleaseStatus,
    pub target_ref: String,
    #[serde(default)]
    pub release_branch: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default)]
    pub issue_keys: Vec<String>,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseEvidenceKind {
    Commit,
    Push,
    Check,
    PreviewDeployment,
    ProductionDeployment,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseEvidence {
    pub id: String,
    pub release_id: String,
    pub kind: ReleaseEvidenceKind,
    #[serde(default)]
    pub issue_key: Option<String>,
    #[serde(default)]
    pub git_trace_id: Option<String>,
    #[serde(default)]
    pub verification_id: Option<String>,
    #[serde(default)]
    pub deployment_ref: Option<String>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub target_ref: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub approver: Option<String>,
    #[serde(default = "passed_outcome")]
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
    pub recorded_by: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseBranchStrategy {
    #[default]
    OneBranchPerRelease,
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromotionPolicy {
    #[default]
    VerifiedOwnerApproval,
    ExternalManual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseWorkflowConfig {
    pub product_id: String,
    pub branch_strategy: ReleaseBranchStrategy,
    pub production_ref: String,
    pub release_branch_pattern: String,
    pub preview_environment: String,
    pub preview_url_template: String,
    pub promotion_policy: PromotionPolicy,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseProgress {
    pub release_id: String,
    pub total: u64,
    pub backlog: u64,
    pub active: u64,
    pub pending_verification: u64,
    pub failed_verification: u64,
    pub completed: u64,
    pub legacy_completed: u64,
    pub canceled: u64,
    pub completion_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub key: String,
    pub product_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    #[serde(default)]
    pub planning_scope: PlanningScope,
    #[serde(default)]
    pub theoria_refs: Vec<TheoriaReference>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub title: String,
    pub body: String,
    /// Original owner context and pasted screenshots, distinct from the shaped brief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intake: Option<IntakeContext>,
    pub acceptance: String,
    pub owner: String,
    pub priority: String,
    pub status: Status,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub claim: Option<Claim>,
    pub needs_fix: bool,
    pub parent: Option<String>,
    pub verification_key: Option<String>,
    pub current_run: Option<String>,
    #[serde(default)]
    pub external: Option<ExternalIssueRecord>,
    /// Intake template provenance (format 13). Absent for untemplated records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateUse>,
}

/// Why an issue cannot be deleted right now. Issue `context` and the atomic
/// `delete_issue` command derive this from the same calculation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeletionBlockerKind {
    /// Only unstarted Backlog or Ready issues can be deleted.
    Status,
    /// A lease that has not yet expired. Expired claims never block.
    ActiveClaim,
    /// Retained discussion history.
    Comments,
    /// Submissions, verification runs, or a linked verification issue.
    VerificationHistory,
    /// Generated verification issues whose parent is this issue.
    VerificationChildren,
    MethodFindings,
    GitTraces,
    /// Parent, blocker, related, or legacy links where this issue is either end.
    IssueLinks,
    /// Releases that list this issue explicitly. Releases that only link the
    /// issue's project are visible in context but are not references.
    ReleaseReferences,
    ReleaseEvidence,
    /// Customer requests linked to the issue (removable) or promoted into it (retained).
    CustomerSignals,
    /// Context documents attached directly to the issue (removable).
    ContextLinks,
    /// Cloud-session handoffs prepared for the issue (retained).
    CloudHandoffs,
    /// Agent assignments made for the issue (retained).
    Assignments,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletionBlocker {
    pub kind: DeletionBlockerKind,
    pub count: u64,
    /// Identifiers the owner can act on: issue keys, release IDs, run IDs, commit SHAs.
    pub references: Vec<String>,
    /// True when the owner can clear this blocker without losing retained history
    /// (unlink an issue, remove the issue from a release, release a claim).
    pub removable: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletionEligibility {
    pub key: String,
    pub version: u64,
    pub eligible: bool,
    pub blockers: Vec<DeletionBlocker>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIssueState {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIssueRecord {
    pub source: String,
    pub id: String,
    pub url: String,
    pub state: ExternalIssueState,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub canceled_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub history: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueLinkKind {
    Parent,
    BlockedBy,
    Related,
    LegacyVerification,
}

/// A relation requested while creating an issue; the new issue is its source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NewIssueLink {
    pub target_key: String,
    pub kind: IssueLinkKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLink {
    pub id: String,
    pub source_key: String,
    pub target_key: String,
    pub kind: IssueLinkKind,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub issue_key: String,
    pub actor: String,
    pub body: String,
    pub at: i64,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub instruction: String,
    pub expected: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Passed,
    Failed,
    Canceled,
}
fn passed_outcome() -> Outcome {
    Outcome::Passed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct E2eEvidence {
    pub build_ref: String,
    pub environment: String,
    pub entrypoint: String,
    pub scenarios: String,
    pub outcome: Outcome,
    pub delivered_build_ref: String,
    pub delivery_check: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub id: String,
    pub issue_key: String,
    pub build_ref: String,
    pub delivery_ref: String,
    pub summary: String,
    pub checks: String,
    #[serde(default)]
    pub e2e: Option<E2eEvidence>,
    pub limitations: String,
    pub preconditions: String,
    pub steps: Vec<Step>,
    pub submitted_by: String,
    pub submitted_at: i64,
    pub outcome: Outcome,
    pub results: Vec<StepResult>,
    pub review_note: String,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub at: i64,
    pub actor: String,
    pub kind: String,
    pub entity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub request_id: String,
    pub actor: String,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Snapshot,
    /// Owner-only housekeeping; preserves workflow and imported history.
    ConsolidateHumanOwners {
        owner: String,
        expected_cursor: u64,
    },
    Context {
        key: String,
    },
    Changes {
        after: u64,
    },
    Export,
    CreateProject {
        product: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default = "default_project_priority")]
        priority: String,
        #[serde(default)]
        sort_order: i64,
        /// Omitted from the request hash when absent so legacy retries replay exactly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<TemplateSelection>,
    },
    UpdateProject {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
        #[serde(default)]
        status: Option<ProjectStatus>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        sort_order: Option<i64>,
    },
    SetIssueProject {
        key: String,
        expected_version: u64,
        project_id: Option<String>,
    },
    CreateGoal {
        product: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default = "default_project_priority")]
        priority: String,
        #[serde(default)]
        project_ids: Vec<String>,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    UpdateGoal {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
        status: GoalStatus,
        priority: String,
        project_ids: Vec<String>,
    },
    CreateMilestone {
        project_id: String,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        sort_order: i64,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    UpdateMilestone {
        id: String,
        expected_version: u64,
        name: String,
        description: String,
        sort_order: i64,
    },
    SetIssueMilestone {
        key: String,
        expected_version: u64,
        milestone_id: Option<String>,
    },
    CreateIssueLink {
        key: String,
        expected_version: u64,
        target_key: String,
        kind: IssueLinkKind,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    DeleteIssueLink {
        key: String,
        expected_version: u64,
        link_id: String,
    },
    CreateLabel {
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        color: String,
        #[serde(default)]
        aliases: Vec<String>,
        #[serde(default)]
        products: Vec<LabelProductRule>,
        #[serde(default)]
        linear_origins: Vec<LinearLabelOrigin>,
    },
    UpdateLabel {
        id: String,
        expected_version: u64,
        name: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        color: String,
        #[serde(default)]
        aliases: Vec<String>,
        #[serde(default)]
        products: Vec<LabelProductRule>,
        #[serde(default)]
        linear_origins: Vec<LinearLabelOrigin>,
    },
    AttachIssueLabel {
        key: String,
        expected_version: u64,
        label_id: String,
    },
    DetachIssueLabel {
        key: String,
        expected_version: u64,
        label_id: String,
    },
    AttachProjectLabel {
        id: String,
        expected_version: u64,
        label_id: String,
    },
    DetachProjectLabel {
        id: String,
        expected_version: u64,
        label_id: String,
    },
    SyncTheoria {
        product: String,
        source_root: String,
        catalog_version: u32,
        documents: Vec<TheoriaDocumentInput>,
    },
    /// Owner only: share a Theoria document with every product, or stop sharing it.
    /// Unsharing blocks new cross-product links; existing pins are kept (DIR-57).
    SetTheoriaSharing {
        document_id: String,
        shared: bool,
    },
    LinkTheoria {
        key: String,
        expected_version: u64,
        document_id: String,
        #[serde(default)]
        playbook_version: Option<String>,
    },
    CreateMethodFinding {
        key: String,
        expected_version: u64,
        classification: FindingClassification,
        observation: String,
        #[serde(default)]
        hypothesis: String,
        proposal: String,
        evidence: Vec<EvidencePointer>,
    },
    /// Owner only: add an agent member (DIR-75).
    CreateAgentMember {
        name: String,
        runtime: String,
        connection_ref: String,
        product_ids: Vec<String>,
        #[serde(default)]
        default_role_key: Option<String>,
        #[serde(default)]
        default_model: Option<String>,
    },
    /// Owner only: change a member's configuration or enabled state.
    UpdateAgentMember {
        id: String,
        expected_version: u64,
        name: String,
        enabled: bool,
        connection_ref: String,
        product_ids: Vec<String>,
        #[serde(default)]
        default_role_key: Option<String>,
        #[serde(default)]
        default_model: Option<String>,
    },
    /// Record a real harness capability check for a member.
    RecordMemberCapability {
        id: String,
        expected_version: u64,
        harness_version: String,
        verified_models: Vec<String>,
        evidence: String,
    },
    /// Owner only: assign an issue to a member with a role revision and model.
    AssignIssueAgent {
        key: String,
        /// Version of the issue's current active assignment; omit when none.
        #[serde(default)]
        expected_assignment_version: Option<u64>,
        member_id: String,
        role_id: String,
        requested_model: String,
        /// Required when another actor holds an active claim on the issue.
        #[serde(default)]
        reconcile_active_writer: Option<String>,
    },
    /// Owner only: clear an issue's active assignment.
    ClearIssueAssignment {
        id: String,
        expected_version: u64,
        reason: String,
    },
    /// Claim holder: record the session that actually works the assignment.
    RecordAssignmentSession {
        id: String,
        session_id: String,
        model: String,
    },
    /// Register an immutable skill package revision (DIR-74).
    RegisterSkillPackage {
        name: String,
        description: String,
        trigger: String,
        origin: SkillOrigin,
        #[serde(default)]
        upstream: Option<SkillUpstream>,
        license: String,
        #[serde(default)]
        adaptations: String,
        files: Vec<SkillFileInput>,
    },
    /// Owner only: retire a skill revision; active roles keep their history.
    RetireSkillPackage {
        id: String,
        reason: String,
    },
    /// Register a draft role revision with pinned DOS guidance.
    RegisterAgentRole {
        key: String,
        name: String,
        responsibilities: Vec<String>,
        inputs: Vec<String>,
        outputs: Vec<String>,
        #[serde(default)]
        skills: Vec<String>,
        runtime_compatibility: Vec<String>,
        guidance: Vec<RoleGuidanceInput>,
        #[serde(default)]
        owner_direction: String,
    },
    /// Owner only: accept a role revision for use. Supersedes the previous active one.
    ActivateAgentRole {
        id: String,
        note: String,
    },
    /// Owner only.
    RetireAgentRole {
        id: String,
        reason: String,
    },
    /// Record files a project-scoped publication actually introduced.
    RecordRolePublication {
        role_id: String,
        harness: String,
        destination: String,
        introduced: Vec<PublishedFile>,
    },
    /// Record a fresh harness session that used a publication.
    RecordActivationEvidence {
        publication_id: String,
        session_id: String,
        model: String,
        #[serde(default)]
        harness_version: String,
        marker: String,
        output: String,
    },
    /// Record a rollback that removed only introduced files.
    RecordPublicationRollback {
        publication_id: String,
        removed: Vec<String>,
        #[serde(default)]
        kept_modified: Vec<String>,
    },
    /// Freeze a bounded packet for one cloud session (DIR-58). Owner or the
    /// active claim holder; the issue must be claimed and is not modified.
    PrepareCloudHandoff {
        key: String,
        expected_version: u64,
        repository: String,
        base_ref: String,
        required_model: String,
        evidence_plan: String,
        #[serde(default)]
        constraints: Vec<String>,
    },
    /// Record the returned session, model, PR, tested SHA and checks.
    ReconcileCloudHandoff {
        id: String,
        expected_version: u64,
        issue_expected_version: u64,
        session_id: String,
        model: String,
        pr_url: String,
        tested_sha: String,
        checks: Vec<CloudCheck>,
        cloud_verdict: String,
        #[serde(default)]
        summary: String,
    },
    WithdrawCloudHandoff {
        id: String,
        expected_version: u64,
        reason: String,
    },
    RecordGitTrace {
        key: String,
        expected_version: u64,
        kind: GitTraceKind,
        repository: String,
        commit_sha: String,
        branch: String,
        #[serde(default)]
        remote: Option<String>,
        #[serde(default)]
        remote_ref: Option<String>,
    },
    CreateRelease {
        product: String,
        name: String,
        version_label: String,
        target_ref: String,
        #[serde(default)]
        release_branch: Option<String>,
        #[serde(default)]
        preview_url: Option<String>,
        #[serde(default)]
        notes: String,
        #[serde(default)]
        project_ids: Vec<String>,
        #[serde(default)]
        issue_keys: Vec<String>,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
    },
    UpdateRelease {
        id: String,
        expected_version: u64,
        name: String,
        version_label: String,
        status: ReleaseStatus,
        target_ref: String,
        #[serde(default)]
        release_branch: Option<String>,
        #[serde(default)]
        preview_url: Option<String>,
        #[serde(default)]
        notes: String,
        project_ids: Vec<String>,
        issue_keys: Vec<String>,
    },
    RecordReleaseEvidence {
        release_id: String,
        expected_version: u64,
        kind: ReleaseEvidenceKind,
        #[serde(default)]
        git_trace_id: Option<String>,
        #[serde(default)]
        verification_id: Option<String>,
        #[serde(default)]
        deployment_ref: Option<String>,
        #[serde(default)]
        commit_sha: Option<String>,
        #[serde(default)]
        target_ref: Option<String>,
        #[serde(default)]
        source_ref: Option<String>,
        #[serde(default)]
        environment: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default = "passed_outcome")]
        outcome: Outcome,
        #[serde(default)]
        note: String,
    },
    SetReleaseWorkflowConfig {
        product: String,
        #[serde(default)]
        expected_version: Option<u64>,
        branch_strategy: ReleaseBranchStrategy,
        production_ref: String,
        release_branch_pattern: String,
        preview_environment: String,
        preview_url_template: String,
        promotion_policy: PromotionPolicy,
    },
    CreateProduct {
        key: String,
        name: String,
        #[serde(default)]
        repo_windows: String,
        #[serde(default)]
        repo_wsl: String,
        #[serde(default)]
        vault_windows: String,
        #[serde(default)]
        vault_wsl: String,
    },
    /// Owner only: set a product's repository and knowledge-vault paths (DIR-23).
    /// Paths are configuration; nothing is moved and no issue changes.
    UpdateProductPaths {
        product: String,
        #[serde(default)]
        repo_windows: String,
        #[serde(default)]
        repo_wsl: String,
        #[serde(default)]
        vault_windows: String,
        #[serde(default)]
        vault_wsl: String,
    },
    CreateProductSection {
        name: String,
    },
    UpdateProductSection {
        id: String,
        expected_version: u64,
        name: String,
    },
    /// Removes the section only; its products return to the ungrouped list.
    DeleteProductSection {
        id: String,
        expected_version: u64,
    },
    /// Replaces the whole sidebar arrangement. Both lists must name every
    /// current section and product exactly once, in display order, so a stale
    /// arrangement is refused instead of silently dropping a concurrent change.
    ArrangeProducts {
        sections: Vec<String>,
        products: Vec<ProductPlacement>,
    },
    /// Record a customer request (DIR-24). Agents and the owner may capture.
    CaptureSignal {
        product: String,
        source_kind: SignalSourceKind,
        #[serde(default)]
        source_reference: String,
        summary: String,
        received_at: i64,
        #[serde(default)]
        customer_reference: String,
        #[serde(default)]
        external_source: Option<String>,
        #[serde(default)]
        external_id: Option<String>,
        #[serde(default)]
        unresolved_mappings: Vec<UnresolvedMapping>,
    },
    /// Correct a captured request. Import provenance, links, promotion and
    /// archive state are unchanged; the text is still stored once.
    UpdateSignal {
        id: String,
        expected_version: u64,
        source_kind: SignalSourceKind,
        #[serde(default)]
        source_reference: String,
        summary: String,
        received_at: i64,
        #[serde(default)]
        customer_reference: String,
    },
    LinkSignal {
        id: String,
        expected_version: u64,
        kind: SignalTargetKind,
        target: String,
    },
    UnlinkSignal {
        id: String,
        expected_version: u64,
        kind: SignalTargetKind,
        target: String,
    },
    /// Create one Inbox issue from the request, link it and record provenance.
    PromoteSignal {
        id: String,
        expected_version: u64,
        #[serde(default)]
        title: Option<String>,
    },
    ArchiveSignal {
        id: String,
        expected_version: u64,
        archived: bool,
    },
    /// Owner only: attach a context document to an issue, project, goal or release (DIR-23).
    AddContextLink {
        target_kind: ContextTargetKind,
        target: String,
        source: ContextSource,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        note: String,
    },
    /// Owner only.
    RemoveContextLink {
        id: String,
        expected_version: u64,
    },
    /// Re-observe availability and fingerprint now; the pinned fingerprint is kept.
    CheckContextLink {
        id: String,
        expected_version: u64,
    },
    /// Read a context document's current content on demand (read-only).
    ReadContextLink {
        id: String,
    },
    CreateIssue {
        product: String,
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        acceptance: String,
        #[serde(default)]
        owner: String,
        #[serde(default = "default_project_priority")]
        priority: String,
        #[serde(default)]
        planning_scope: PlanningScope,
        #[serde(default)]
        project_id: Option<String>,
        /// Omitted from the request hash when absent so legacy retries replay exactly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<TemplateSelection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intake: Option<IntakeContext>,
        /// Relations written atomically with the issue under the same rules as
        /// `CreateIssueLink`. Omitted from the request hash when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        links: Vec<NewIssueLink>,
    },
    UpdateIssue {
        key: String,
        expected_version: u64,
        title: String,
        body: String,
        acceptance: String,
        owner: String,
        priority: String,
        #[serde(default)]
        planning_scope: Option<PlanningScope>,
        /// Omission preserves context; an empty object explicitly clears it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intake: Option<IntakeContext>,
    },
    DeleteIssue {
        key: String,
        expected_version: u64,
    },
    Ready {
        key: String,
        expected_version: u64,
    },
    Claim {
        key: String,
        expected_version: u64,
        #[serde(default = "lease")]
        lease_seconds: i64,
    },
    Renew {
        key: String,
        expected_version: u64,
        #[serde(default = "lease")]
        lease_seconds: i64,
    },
    Release {
        key: String,
        expected_version: u64,
    },
    Comment {
        key: String,
        expected_version: u64,
        body: String,
    },
    Submit {
        key: String,
        expected_version: u64,
        build_ref: String,
        delivery_ref: String,
        summary: String,
        checks: String,
        #[serde(default)]
        e2e: Option<E2eEvidence>,
        #[serde(default)]
        limitations: String,
        #[serde(default)]
        preconditions: String,
        steps: Vec<Step>,
    },
    Review {
        key: String,
        expected_version: u64,
        run_id: String,
        outcome: Outcome,
        results: Vec<StepResult>,
        #[serde(default)]
        note: String,
    },
    /// Owner only: cancel non-deliverable work with a reason (DIR-86). History,
    /// comments, links, sources and runs are kept; `reopen` restores it.
    CancelIssue {
        key: String,
        expected_version: u64,
        reason: String,
        /// Required to cancel work that an agent actively holds.
        #[serde(default)]
        release_active_claim: bool,
    },
    Reopen {
        key: String,
        expected_version: u64,
        reason: String,
    },
    /// Read-only list of retained source bundles with their summaries.
    SourceBundles,
    /// Read-only bounded search over retained source records.
    SearchSources {
        #[serde(default)]
        query: String,
        #[serde(default)]
        bundle_id: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        classification: Option<String>,
        #[serde(default)]
        issue_key: Option<String>,
        #[serde(default = "source_page")]
        limit: u32,
        #[serde(default)]
        offset: u32,
    },
    /// Read-only: one retained record with its exact original source bytes.
    SourceRecord {
        id: String,
    },
    /// Owner only: undo an applied migration while nothing has changed since.
    RollbackMigration {
        bundle_id: String,
        expected_cursor: u64,
    },
    /// Read-only: every template head and immutable revision, including retired ones.
    Templates,
    /// Owner only: define a template; its first immutable revision is 1.
    CreateTemplate {
        target: TemplateTarget,
        name: String,
        #[serde(default)]
        description: String,
        shape: TemplateShape,
        content: TemplateContent,
        #[serde(default)]
        supplements: Vec<TemplateSupplement>,
        #[serde(default)]
        note: String,
    },
    /// Owner only: append the next immutable revision. Existing records keep theirs.
    ReviseTemplate {
        id: String,
        expected_version: u64,
        name: String,
        #[serde(default)]
        description: String,
        shape: TemplateShape,
        content: TemplateContent,
        #[serde(default)]
        supplements: Vec<TemplateSupplement>,
        #[serde(default)]
        note: String,
    },
    /// Owner only: stop offering a template for new work. Nothing is deleted.
    RetireTemplate {
        id: String,
        expected_version: u64,
        reason: String,
    },
}
fn source_page() -> u32 {
    50
}
fn lease() -> i64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub id: String,
    pub actor: String,
    pub role: String,
    pub hash: String,
    pub response: String,
}

/// Lifecycle of a bounded cloud-session handoff (DIR-58).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudHandoffStatus {
    /// Packet frozen and ready to hand to one cloud session.
    Prepared,
    /// The coordinator recorded the returned session, PR, SHA and checks.
    Reconciled,
    /// Abandoned before reconciliation; kept for audit.
    Withdrawn,
}

/// Guidance pinned on the issue when the packet was frozen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudGuidancePin {
    pub document_id: String,
    pub title: String,
    pub relative_path: String,
    pub recorded_fingerprint: Option<String>,
    pub playbook_version: Option<String>,
}

/// Everything a cloud session receives, and nothing else. Fields are an
/// explicit allow-list: no service address, grant, capability, local path,
/// owner identity, unrelated issue or database content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudPacket {
    pub format: u32,
    pub issue_key: String,
    pub product_key: String,
    pub title: String,
    pub body: String,
    pub acceptance: String,
    pub issue_version: u64,
    pub claim_actor: String,
    pub claim_expires_at: i64,
    pub repository: String,
    pub base_ref: String,
    pub required_model: String,
    pub guidance: Vec<CloudGuidancePin>,
    pub evidence_plan: String,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudCheckEnvironment {
    Cloud,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudCheckOutcome {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudCheck {
    pub name: String,
    pub outcome: CloudCheckOutcome,
    pub environment: CloudCheckEnvironment,
    #[serde(default)]
    pub detail: String,
}

/// The cloud session's own result. There is deliberately no delivered value:
/// a delivered Pass needs local integration, install and smoke evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudVerdict {
    Passed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudReconciliation {
    pub session_id: String,
    pub model: String,
    pub pr_url: String,
    pub tested_sha: String,
    pub checks: Vec<CloudCheck>,
    pub cloud_verdict: CloudVerdict,
    pub summary: String,
    pub reconciled_by: String,
    pub reconciled_at: i64,
}

/// A frozen packet for one cloud session and its coordinator reconciliation
/// (format 19). The issue itself is never modified by a handoff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudHandoff {
    pub id: String,
    pub issue_key: String,
    pub product_id: String,
    pub version: u64,
    pub status: CloudHandoffStatus,
    pub packet: CloudPacket,
    pub packet_sha256: String,
    /// Hash of the brief and guidance pins at prepare time; a change makes
    /// the packet stale for reconciliation.
    pub brief_sha256: String,
    pub prepared_by: String,
    pub prepared_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub reconciliation: Option<CloudReconciliation>,
    #[serde(default)]
    pub withdrawn_reason: Option<String>,
}

/// Where a skill bundle came from (DIR-74).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    /// Authored for this workspace; no upstream to track.
    Local,
    /// Selected from an upstream repository at an immutable commit.
    Upstream,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillUpstream {
    pub repository: String,
    pub commit: String,
    #[serde(default)]
    pub path: String,
}

/// One file of a skill bundle, stored so a publication is reproducible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFile {
    pub path: String,
    pub sha256: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFileInput {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Retirement {
    pub by: String,
    pub at: i64,
    pub reason: String,
}

/// An immutable revision of a selected, project-scoped skill bundle (format 20).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillPackage {
    pub id: String,
    pub name: String,
    pub revision: u32,
    pub description: String,
    pub trigger: String,
    pub origin: SkillOrigin,
    #[serde(default)]
    pub upstream: Option<SkillUpstream>,
    pub license: String,
    #[serde(default)]
    pub adaptations: String,
    pub files: Vec<SkillFile>,
    pub bundle_sha256: String,
    pub registered_by: String,
    pub registered_at: i64,
    #[serde(default)]
    pub retired: Option<Retirement>,
}

/// A DOS document a role revision depends on, pinned at registration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleGuidancePin {
    pub document_id: String,
    pub recorded_fingerprint: Option<String>,
    /// Only an explicitly recorded version; `None` stays Unknown.
    pub playbook_version: Option<String>,
    /// A mandatory reference must be available before activation.
    pub mandatory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleGuidanceInput {
    pub document_id: String,
    #[serde(default)]
    pub playbook_version: Option<String>,
    #[serde(default)]
    pub mandatory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoleStatus {
    Draft,
    Active,
    /// Replaced by a later activated revision of the same role.
    Superseded,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleActivation {
    pub by: String,
    pub at: i64,
    pub note: String,
}

/// An immutable revision of an agent role contract (format 20). Text here is
/// task context, never tool authority: service permissions do not read it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRole {
    pub id: String,
    pub key: String,
    pub revision: u32,
    pub name: String,
    pub responsibilities: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Exact skill package revisions (IDs) the role requires.
    pub skills: Vec<String>,
    /// Harnesses the role is written for, e.g. `claude-code`, `codex`.
    pub runtime_compatibility: Vec<String>,
    pub guidance: Vec<RoleGuidancePin>,
    /// Current owner direction for bounded use; required before activation.
    #[serde(default)]
    pub owner_direction: String,
    pub status: RoleStatus,
    pub registered_by: String,
    pub registered_at: i64,
    #[serde(default)]
    pub activation: Option<RoleActivation>,
    #[serde(default)]
    pub retired: Option<Retirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishedFile {
    pub path: String,
    pub sha256: String,
}

/// A fresh harness session observed using a publication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivationEvidence {
    pub session_id: String,
    pub model: String,
    pub harness_version: String,
    pub marker: String,
    pub output_excerpt: String,
    pub output_sha256: String,
    pub recorded_by: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicationRollback {
    pub by: String,
    pub at: i64,
    pub removed: Vec<String>,
    /// Introduced files left in place because they changed after publishing.
    pub kept_modified: Vec<String>,
}

/// One project-scoped publication of a role revision and its skills (format 20).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RolePublication {
    pub id: String,
    pub role_id: String,
    pub harness: String,
    pub destination: String,
    /// Files this publication wrote. Identical pre-existing files are not listed.
    pub introduced: Vec<PublishedFile>,
    pub published_by: String,
    pub published_at: i64,
    #[serde(default)]
    pub evidence: Vec<ActivationEvidence>,
    #[serde(default)]
    pub rollback: Option<PublicationRollback>,
}

/// A runtime adapter capability check for one member (DIR-75). Availability
/// comes only from a real harness run, never from a hardcoded promise.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemberCapability {
    pub harness_version: String,
    /// Models the harness actually ran in this check.
    pub verified_models: Vec<String>,
    /// Session or run reference the check produced.
    pub evidence: String,
    pub checked_by: String,
    pub checked_at: i64,
}

/// A logical agent member of this single-owner workspace (format 21).
/// Identity, runtime, role, requested model and actual session stay distinct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMember {
    pub id: String,
    pub name: String,
    /// `claude-code` or `codex`.
    pub runtime: String,
    pub enabled: bool,
    /// Opaque, non-secret label for how the member is reached (no URLs with
    /// credentials, tokens or keys).
    pub connection_ref: String,
    /// Products this member may be assigned work in.
    pub product_ids: Vec<String>,
    #[serde(default)]
    pub default_role_key: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub capability: Option<MemberCapability>,
    pub version: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Active,
    /// Replaced by a later assignment of the same issue.
    Superseded,
    Cleared,
}

/// The session that actually worked an assignment, recorded by the claim holder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssignmentSession {
    pub session_id: String,
    pub model: String,
    pub recorded_by: String,
    pub recorded_at: i64,
}

/// Owner assignment of an issue to an agent member with an exact role revision
/// and requested model (format 21). Never changes readiness, review or claims.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueAssignment {
    pub id: String,
    pub issue_key: String,
    pub member_id: String,
    pub role_id: String,
    pub requested_model: String,
    pub status: AssignmentStatus,
    pub version: u64,
    pub assigned_by: String,
    pub assigned_at: i64,
    /// Set when assigned while another actor held an active claim.
    #[serde(default)]
    pub reconciliation: Option<String>,
    #[serde(default)]
    pub sessions: Vec<AssignmentSession>,
    #[serde(default)]
    pub cleared: Option<Retirement>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archive {
    pub format: u32,
    pub workspace_id: String,
    pub products: Vec<Product>,
    /// Owner-defined sidebar sections (format 15).
    #[serde(default)]
    pub product_sections: Vec<ProductSection>,
    /// Customer requests (format 16). Distinct from `requests`, which is command idempotency.
    #[serde(default)]
    pub customer_signals: Vec<CustomerSignal>,
    /// Context document links (format 17). Obsidian content is never archived.
    #[serde(default)]
    pub context_links: Vec<ContextLink>,
    /// Bounded cloud-session handoffs and reconciliations (format 19).
    #[serde(default)]
    pub cloud_handoffs: Vec<CloudHandoff>,
    /// Skill package revisions, role revisions and publications (format 20).
    #[serde(default)]
    pub skill_packages: Vec<SkillPackage>,
    #[serde(default)]
    pub agent_roles: Vec<AgentRole>,
    #[serde(default)]
    pub role_publications: Vec<RolePublication>,
    /// Agent members and issue assignments (format 21).
    #[serde(default)]
    pub agent_members: Vec<AgentMember>,
    #[serde(default)]
    pub issue_assignments: Vec<IssueAssignment>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub milestones: Vec<Milestone>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub theoria_documents: Vec<TheoriaDocument>,
    #[serde(default)]
    pub method_findings: Vec<MethodFinding>,
    #[serde(default)]
    pub git_traces: Vec<GitTrace>,
    #[serde(default)]
    pub releases: Vec<ReleaseRecord>,
    #[serde(default)]
    pub release_evidence: Vec<ReleaseEvidence>,
    #[serde(default)]
    pub release_workflows: Vec<ReleaseWorkflowConfig>,
    #[serde(default)]
    pub issue_links: Vec<IssueLink>,
    /// Workspace intake templates and their immutable revisions (format 13).
    #[serde(default)]
    pub templates: Vec<WorkspaceTemplate>,
    #[serde(default)]
    pub template_revisions: Vec<TemplateRevision>,
    pub issues: Vec<Issue>,
    pub comments: Vec<Comment>,
    pub verifications: Vec<Verification>,
    pub events: Vec<Event>,
    pub requests: Vec<Replay>,
    /// Retained external sources (format 12). Files carry base64 bytes in archives.
    #[serde(default)]
    pub source_bundles: Vec<SourceBundle>,
    #[serde(default)]
    pub source_files: Vec<SourceFile>,
    #[serde(default)]
    pub source_records: Vec<SourceRecord>,
}

/// A verified external source package retained for native, read-only access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceBundle {
    pub id: String,
    pub source: String,
    pub label: String,
    pub captured_at: String,
    pub manifest_sha256: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub record_count: u64,
    /// Bounded reconciliation summary: preservation, access, freshness and gates.
    #[serde(default)]
    pub summary: serde_json::Value,
    /// Present when the bundle arrived through a migration applied to this workspace.
    #[serde(default)]
    pub application: Option<MigrationApplication>,
    pub imported_by: String,
    pub imported_at: i64,
}

/// IDs of every record a migration added, so replay and rollback are exact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MigrationEntities {
    #[serde(default)]
    pub products: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default)]
    pub milestones: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub comments: Vec<String>,
    #[serde(default)]
    pub issue_links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrationApplication {
    pub artifact_sha256: String,
    /// Event cursor the owner confirmed before applying.
    pub baseline_cursor: u64,
    /// Event cursor immediately after the import transaction.
    pub applied_cursor: u64,
    /// File name of the pre-import backup written by the service, when one was written.
    #[serde(default)]
    pub backup: Option<String>,
    pub entities: MigrationEntities,
}

/// One retained file. `data` (base64) is present in archives and absent in metadata views.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceFile {
    pub bundle_id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub content_type: String,
    /// `manifest`, `data` or `upload`.
    pub role: String,
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// Index entry for one source record: an exact byte span of a retained file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRecord {
    pub id: String,
    pub bundle_id: String,
    pub kind: String,
    /// `record`, `component` or `file`.
    pub level: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// `native`, `transformed`, `preserved` or `unresolved`.
    pub classification: String,
    /// `native` (also a Direct record), `retained` (source view only) or `missing`.
    pub access: String,
    pub file: String,
    pub start: u64,
    pub end: u64,
    #[serde(default)]
    pub pointer: String,
    #[serde(default)]
    pub direct: serde_json::Value,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub preserved_fields: Vec<String>,
    #[serde(default)]
    pub issue_keys: Vec<String>,
    /// Retained file this record describes (for uploaded files).
    #[serde(default)]
    pub download_path: Option<String>,
    #[serde(default)]
    pub search_text: String,
}
