//! Owner-managed workspace intake templates.
//!
//! A template has a stable identity and a mutable head (status, current revision);
//! every definition is an immutable revision row that is inserted once and never
//! rewritten. Issues and projects record the exact revision they were created
//! from, so later revisions and retirement never change existing records.
//! Templates only shape intake text and suggestions: they confer no tool
//! authority, never make work Ready and never count as verification.

use super::*;

const NAME_MAX: usize = 120;
const DESCRIPTION_MAX: usize = 1_000;
/// Prompts stay concise: templates shape a brief, they do not preload a context package.
const PROMPT_MAX: usize = 2_000;
const CHECKLIST_MAX: usize = 12;
const CHECKLIST_ITEM_MAX: usize = 200;
const SUGGESTED_LABELS_MAX: usize = 10;
const SUPPLEMENT_NOTE_MAX: usize = 1_000;
const SUPPLEMENT_CHECKLIST_MAX: usize = 5;
const SUPPLEMENT_LABELS_MAX: usize = 5;
const RETIRE_REASON_MAX: usize = 500;
/// Fields a creator may explicitly change from the template's suggestion.
pub(crate) const OVERRIDE_FIELDS: [&str; 4] =
    ["priority", "planning_scope", "execution_mode", "labels"];

fn target_name(target: TemplateTarget) -> &'static str {
    match target {
        TemplateTarget::Issue => "issue",
        TemplateTarget::Project => "project",
    }
}

fn bounded(value: &str, field: &str, max: usize) -> Result<()> {
    if value.chars().count() > max {
        return Err(err(
            "invalid",
            format!("{field} must be at most {max} characters"),
        ));
    }
    Ok(())
}

fn checklist(items: &[String], field: &str, max: usize) -> Result<()> {
    if items.len() > max {
        return Err(err(
            "invalid",
            format!("{field} allows at most {max} items"),
        ));
    }
    let mut seen = HashSet::new();
    for item in items {
        if item.trim().is_empty() {
            return Err(err("invalid", format!("{field} items cannot be blank")));
        }
        bounded(item, field, CHECKLIST_ITEM_MAX)?;
        if !seen.insert(item.trim().to_lowercase()) {
            return Err(err("invalid", format!("{field} repeats “{}”", item.trim())));
        }
    }
    Ok(())
}

fn trimmed(items: &[String]) -> Vec<String> {
    items.iter().map(|item| item.trim().to_string()).collect()
}

/// Bounds and references shared by live commands and archive validation.
/// `label_applicability` is checked only for new definitions: an archived
/// revision may suggest a label whose product rules were narrowed later.
pub(crate) struct DefinitionScope<'a> {
    pub products: &'a [Product],
    pub labels: &'a [Label],
    pub check_label_applicability: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_definition(
    scope: &DefinitionScope,
    target: TemplateTarget,
    name: &str,
    description: &str,
    content: &TemplateContent,
    supplements: &[TemplateSupplement],
    note: &str,
) -> Result<()> {
    required(name, "Template name")?;
    bounded(name, "Template name", NAME_MAX)?;
    bounded(description, "Template description", DESCRIPTION_MAX)?;
    bounded(note, "Revision note", DESCRIPTION_MAX)?;
    required(&content.intent, "Intent prompt")?;
    bounded(&content.intent, "Intent prompt", PROMPT_MAX)?;
    bounded(&content.boundaries, "Boundaries prompt", PROMPT_MAX)?;
    bounded(&content.verification, "Verification prompt", PROMPT_MAX)?;
    checklist(&content.checklist, "Checklist", CHECKLIST_MAX)?;
    if let Some(priority) = &content.suggested_priority {
        if !valid_priority(priority) {
            return Err(err("invalid", "Unknown suggested priority"));
        }
    }
    if target == TemplateTarget::Project && content.suggested_planning_scope.is_some() {
        return Err(err(
            "invalid",
            "Project templates cannot suggest an issue work route",
        ));
    }
    let known_label = |id: &str| scope.labels.iter().find(|label| label.id == id);
    let mut base_labels = HashSet::new();
    if content.suggested_labels.len() > SUGGESTED_LABELS_MAX {
        return Err(err(
            "invalid",
            format!("A template suggests at most {SUGGESTED_LABELS_MAX} labels"),
        ));
    }
    for id in &content.suggested_labels {
        if known_label(id).is_none() {
            return Err(err("invalid", "Suggested label does not exist"));
        }
        if !base_labels.insert(id.as_str()) {
            return Err(err("invalid", "Suggested labels must be unique"));
        }
    }
    let mut supplemented = HashSet::new();
    for supplement in supplements {
        if !scope
            .products
            .iter()
            .any(|product| product.id == supplement.product_id)
        {
            return Err(err("invalid", "Supplement names an unknown product"));
        }
        if !supplemented.insert(supplement.product_id.as_str()) {
            return Err(err(
                "invalid",
                "Each product can have at most one supplement per revision",
            ));
        }
        if supplement.note.trim().is_empty()
            && supplement.checklist.is_empty()
            && supplement.suggested_labels.is_empty()
        {
            return Err(err(
                "invalid",
                "A product supplement must add a note, checklist item or label",
            ));
        }
        bounded(&supplement.note, "Supplement note", SUPPLEMENT_NOTE_MAX)?;
        checklist(
            &supplement.checklist,
            "Supplement checklist",
            SUPPLEMENT_CHECKLIST_MAX,
        )?;
        let base_items: HashSet<String> = content
            .checklist
            .iter()
            .map(|item| item.trim().to_lowercase())
            .collect();
        if supplement
            .checklist
            .iter()
            .any(|item| base_items.contains(&item.trim().to_lowercase()))
        {
            return Err(err(
                "invalid",
                "A supplement only adds checklist items; it cannot repeat the shared base",
            ));
        }
        if supplement.suggested_labels.len() > SUPPLEMENT_LABELS_MAX {
            return Err(err(
                "invalid",
                format!("A supplement suggests at most {SUPPLEMENT_LABELS_MAX} labels"),
            ));
        }
        let mut seen = HashSet::new();
        for id in &supplement.suggested_labels {
            let label =
                known_label(id).ok_or_else(|| err("invalid", "Suggested label does not exist"))?;
            if base_labels.contains(id.as_str()) || !seen.insert(id.as_str()) {
                return Err(err(
                    "invalid",
                    "A supplement only adds labels the shared base does not suggest",
                ));
            }
            if scope.check_label_applicability && !label_applies(label, &supplement.product_id) {
                return Err(err(
                    "invalid",
                    format!(
                        "Label “{}” does not apply to the supplement's product",
                        label.name
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn revision_id(template_id: &str, revision: u32) -> String {
    // Zero padding keeps `ORDER BY id` in revision order.
    format!("{template_id}:{revision:08}")
}

pub(crate) fn template(conn: &Connection, id: &str) -> Result<WorkspaceTemplate> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM templates WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &data.ok_or_else(|| err("not_found", "Unknown template"))?,
    )?)
}

pub(crate) fn put_template(conn: &Connection, template: &WorkspaceTemplate) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO templates VALUES (?1,?2)",
        params![template.id, serde_json::to_string(template)?],
    )?;
    Ok(())
}

pub(crate) fn revision(
    conn: &Connection,
    template_id: &str,
    number: u32,
) -> Result<TemplateRevision> {
    let data: Option<String> = conn
        .query_row(
            "SELECT data FROM template_revisions WHERE id=?1",
            [revision_id(template_id, number)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", "Unknown template revision")
    })?)?)
}

/// Plain INSERT: a revision row can be written once and never replaced.
pub(crate) fn insert_revision(conn: &Connection, revision: &TemplateRevision) -> Result<()> {
    conn.execute(
        "INSERT INTO template_revisions VALUES (?1,?2)",
        params![
            revision_id(&revision.template_id, revision.revision),
            serde_json::to_string(revision)?
        ],
    )?;
    Ok(())
}

pub(crate) fn listing(conn: &Connection) -> Result<Value> {
    let mut templates = all::<WorkspaceTemplate>(conn, "templates")?;
    templates.sort_by(|a, b| {
        (a.status == TemplateStatus::Retired)
            .cmp(&(b.status == TemplateStatus::Retired))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(json!({
        "templates": templates,
        "template_revisions": all::<TemplateRevision>(conn, "template_revisions")?,
        "authority": "Templates shape intake text and suggestions only. They grant no tool authority, never make work Ready and never count as verification.",
    }))
}

/// Resolved provenance for context: the exact revision used and whether it is outdated.
pub(crate) fn provenance(conn: &Connection, used: Option<&TemplateUse>) -> Result<Value> {
    let Some(used) = used else {
        return Ok(Value::Null);
    };
    let head = template(conn, &used.template_id)?;
    let revision = revision(conn, &used.template_id, used.revision)?;
    Ok(json!({
        "use": used,
        "template": head,
        "revision": revision,
        "current_revision": head.current_revision,
        "outdated": head.current_revision > used.revision,
        "retired": head.status == TemplateStatus::Retired,
    }))
}

fn definition_scope<'a>(products: &'a [Product], labels: &'a [Label]) -> DefinitionScope<'a> {
    DefinitionScope {
        products,
        labels,
        check_label_applicability: true,
    }
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role: Role,
    at: i64,
) -> Result<Value> {
    // Definitions are owner-managed. Agents read and apply them, never change them.
    human(role)?;
    let products = all::<Product>(tx, "products")?;
    let labels = all::<Label>(tx, "labels")?;
    match cmd {
        Command::CreateTemplate {
            target,
            name,
            description,
            shape,
            content,
            supplements,
            note,
        } => {
            validate_definition(
                &definition_scope(&products, &labels),
                *target,
                name,
                description,
                content,
                supplements,
                note,
            )?;
            let id = id();
            let revision = TemplateRevision {
                template_id: id.clone(),
                revision: 1,
                target: *target,
                name: name.trim().into(),
                description: description.clone(),
                shape: *shape,
                content: normalized(content),
                supplements: supplements.iter().map(normalized_supplement).collect(),
                note: note.clone(),
                created_by: actor.into(),
                created_at: at,
            };
            let head = WorkspaceTemplate {
                id: id.clone(),
                target: *target,
                name: revision.name.clone(),
                shape: *shape,
                status: TemplateStatus::Active,
                current_revision: 1,
                retired_reason: None,
                retired_by: None,
                retired_at: None,
                version: 1,
                created_at: at,
                updated_at: at,
            };
            insert_revision(tx, &revision)?;
            put_template(tx, &head)?;
            emit(tx, actor, "template_created", &id, at)?;
            Ok(json!({"template": head, "revision": revision}))
        }
        Command::ReviseTemplate {
            id,
            expected_version,
            name,
            description,
            shape,
            content,
            supplements,
            note,
        } => {
            let mut head = template(tx, id)?;
            if head.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Template changed; reopen its editor before retrying",
                ));
            }
            if head.status == TemplateStatus::Retired {
                return Err(err("invalid", "Retired templates are read-only"));
            }
            validate_definition(
                &definition_scope(&products, &labels),
                head.target,
                name,
                description,
                content,
                supplements,
                note,
            )?;
            let revision = TemplateRevision {
                template_id: head.id.clone(),
                revision: head.current_revision + 1,
                target: head.target,
                name: name.trim().into(),
                description: description.clone(),
                shape: *shape,
                content: normalized(content),
                supplements: supplements.iter().map(normalized_supplement).collect(),
                note: note.clone(),
                created_by: actor.into(),
                created_at: at,
            };
            insert_revision(tx, &revision)?;
            head.current_revision = revision.revision;
            head.name = revision.name.clone();
            head.shape = revision.shape;
            head.version += 1;
            head.updated_at = at;
            put_template(tx, &head)?;
            emit(tx, actor, "template_revised", id, at)?;
            Ok(json!({"template": head, "revision": revision}))
        }
        Command::RetireTemplate {
            id,
            expected_version,
            reason,
        } => {
            let mut head = template(tx, id)?;
            if head.version != *expected_version {
                return Err(err(
                    "conflict",
                    "Template changed; reopen it before retrying",
                ));
            }
            if head.status == TemplateStatus::Retired {
                return Err(err("conflict", "Template is already retired"));
            }
            required(reason, "Retirement reason")?;
            bounded(reason, "Retirement reason", RETIRE_REASON_MAX)?;
            head.status = TemplateStatus::Retired;
            head.retired_reason = Some(reason.trim().into());
            head.retired_by = Some(actor.into());
            head.retired_at = Some(at);
            head.version += 1;
            head.updated_at = at;
            put_template(tx, &head)?;
            emit(tx, actor, "template_retired", id, at)?;
            Ok(json!({"template": head}))
        }
        _ => unreachable!("only template commands are routed here"),
    }
}

fn normalized(content: &TemplateContent) -> TemplateContent {
    TemplateContent {
        checklist: trimmed(&content.checklist),
        ..content.clone()
    }
}

fn normalized_supplement(supplement: &TemplateSupplement) -> TemplateSupplement {
    TemplateSupplement {
        checklist: trimmed(&supplement.checklist),
        ..supplement.clone()
    }
}

/// The record fields a template's suggestions are compared with.
pub(crate) struct Intake<'a> {
    pub target: TemplateTarget,
    pub product: &'a Product,
    pub priority: &'a str,
    pub planning_scope: Option<&'a PlanningScope>,
}

/// Check a selection against the current definition and derive exact provenance
/// plus the suggested labels the creator kept. Applying a template changes no
/// status, claim, readiness or verification state.
pub(crate) fn apply(
    tx: &Transaction,
    selection: &TemplateSelection,
    intake: Intake,
    actor: &str,
    at: i64,
) -> Result<(TemplateUse, Vec<String>)> {
    let head = template(tx, &selection.template_id)?;
    if head.target != intake.target {
        return Err(err(
            "invalid",
            format!(
                "This template shapes {} intake, not {}",
                target_name(head.target),
                target_name(intake.target)
            ),
        ));
    }
    if head.status == TemplateStatus::Retired {
        return Err(err(
            "invalid",
            "This template is retired; choose an active template",
        ));
    }
    if selection.revision != head.current_revision {
        return Err(err(
            "conflict",
            format!(
                "Template is now at revision {}; reload it before applying",
                head.current_revision
            ),
        ));
    }
    let definition = revision(tx, &head.id, head.current_revision)?;
    let supplement = definition
        .supplements
        .iter()
        .find(|supplement| supplement.product_id == intake.product.id);
    let suggested: Vec<&String> = definition
        .content
        .suggested_labels
        .iter()
        .chain(
            supplement
                .into_iter()
                .flat_map(|s| s.suggested_labels.iter()),
        )
        .collect();
    let labels = all::<Label>(tx, "labels")?;
    let applicable: Vec<&String> = suggested
        .iter()
        .copied()
        .filter(|id| {
            labels
                .iter()
                .any(|label| label.id == **id && label_applies(label, &intake.product.id))
        })
        .collect();
    let mut kept = Vec::new();
    for id in &selection.labels {
        if !suggested.contains(&id) {
            return Err(err(
                "invalid",
                "Only labels this template suggests can be kept at intake",
            ));
        }
        if !applicable.contains(&id) {
            return Err(err("invalid", "Label does not apply to this product"));
        }
        if kept.contains(id) {
            return Err(err("invalid", "Kept labels must be unique"));
        }
        kept.push(id.clone());
    }
    let mut overrides = Vec::new();
    if definition
        .content
        .suggested_priority
        .as_deref()
        .is_some_and(|suggested| suggested != intake.priority)
    {
        overrides.push("priority".to_string());
    }
    if let (Some(suggested), Some(chosen)) = (
        definition.content.suggested_planning_scope.as_ref(),
        intake.planning_scope,
    ) {
        if suggested != chosen {
            overrides.push("planning_scope".to_string());
        }
    }
    // Absent accepts the suggestion; an explicit choice, including clearing it, is recorded.
    let suggested_mode = definition.content.execution_mode;
    let execution_mode = selection.execution_mode.unwrap_or(suggested_mode);
    if execution_mode != suggested_mode {
        overrides.push("execution_mode".to_string());
    }
    if applicable.iter().any(|id| !kept.contains(id)) {
        overrides.push("labels".to_string());
    }
    Ok((
        TemplateUse {
            template_id: head.id,
            revision: definition.revision,
            supplement_product_id: supplement.map(|s| s.product_id.clone()),
            execution_mode,
            overrides,
            applied_by: actor.into(),
            applied_at: at,
        },
        kept,
    ))
}

/// Archive invariants: format, immutable revision chains, and exact provenance.
pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    let has_data = !a.templates.is_empty()
        || !a.template_revisions.is_empty()
        || a.issues.iter().any(|issue| issue.template.is_some())
        || a.projects.iter().any(|project| project.template.is_some());
    if a.format < 13 && has_data {
        return Err(err("invalid", "Template data requires archive format 13"));
    }
    let invalid = |message: &str| err("invalid", format!("Invalid template archive: {message}"));
    let mut heads = HashMap::new();
    for head in &a.templates {
        if Uuid::parse_str(&head.id).is_err()
            || head.version == 0
            || head.current_revision == 0
            || heads.insert(head.id.as_str(), head).is_some()
        {
            return Err(invalid("template identity"));
        }
        let retired = head.status == TemplateStatus::Retired;
        let complete = head
            .retired_reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty())
            && head
                .retired_by
                .as_deref()
                .is_some_and(|actor| !actor.trim().is_empty())
            && head.retired_at.is_some_and(|at| at > 0);
        let absent =
            head.retired_reason.is_none() && head.retired_by.is_none() && head.retired_at.is_none();
        if (retired && !complete) || (!retired && !absent) {
            return Err(invalid("retirement provenance"));
        }
    }
    let scope = DefinitionScope {
        products: &a.products,
        labels: &a.labels,
        check_label_applicability: false,
    };
    let mut chains: HashMap<&str, HashSet<u32>> = HashMap::new();
    let mut revisions = HashMap::new();
    for revision in &a.template_revisions {
        let head = heads
            .get(revision.template_id.as_str())
            .ok_or_else(|| invalid("revision without a template"))?;
        if revision.target != head.target
            || revision.revision == 0
            || revision.created_by.trim().is_empty()
            || !chains
                .entry(revision.template_id.as_str())
                .or_default()
                .insert(revision.revision)
        {
            return Err(invalid("revision identity"));
        }
        validate_definition(
            &scope,
            revision.target,
            &revision.name,
            &revision.description,
            &revision.content,
            &revision.supplements,
            &revision.note,
        )?;
        revisions.insert((revision.template_id.as_str(), revision.revision), revision);
    }
    for head in heads.values() {
        let chain = chains.get(head.id.as_str());
        let contiguous = chain.is_some_and(|chain| {
            chain.len() == head.current_revision as usize
                && (1..=head.current_revision).all(|n| chain.contains(&n))
        });
        let current = revisions.get(&(head.id.as_str(), head.current_revision));
        if !contiguous
            || current
                .is_none_or(|current| current.name != head.name || current.shape != head.shape)
        {
            return Err(invalid("revision history is incomplete"));
        }
    }
    let check_use = |used: &TemplateUse, target: TemplateTarget, product_id: &str| -> Result<()> {
        let revision = revisions
            .get(&(used.template_id.as_str(), used.revision))
            .ok_or_else(|| invalid("record references an unknown revision"))?;
        let expected_supplement = revision
            .supplements
            .iter()
            .find(|supplement| supplement.product_id == product_id)
            .map(|supplement| supplement.product_id.as_str());
        let mut seen = HashSet::new();
        if revision.target != target
            || used.supplement_product_id.as_deref() != expected_supplement
            || used.applied_by.trim().is_empty()
            || used.applied_at <= 0
            || used
                .overrides
                .iter()
                .any(|field| !OVERRIDE_FIELDS.contains(&field.as_str()) || !seen.insert(field))
        {
            return Err(invalid("record provenance"));
        }
        Ok(())
    };
    for issue in &a.issues {
        if let Some(used) = &issue.template {
            check_use(used, TemplateTarget::Issue, &issue.product_id)?;
        }
    }
    for project in &a.projects {
        if let Some(used) = &project.template {
            check_use(used, TemplateTarget::Project, &project.product_id)?;
        }
    }
    Ok(())
}
