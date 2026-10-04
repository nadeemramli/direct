//! Customer requests captured as provenance-bearing intake signals (DIR-24).
//!
//! A signal stores the request text once and links to issues or projects by
//! reference, so many requests can point at one piece of work and one request
//! at several. Promotion creates at most one Inbox issue. Signals are intake
//! data: capturing, linking or promoting never makes work Ready, changes
//! priority, records review or modifies the linked issue.

use super::*;

const SUMMARY_MAX: usize = 2_000;
const REFERENCE_MAX: usize = 300;
const CUSTOMER_MAX: usize = 120;
const PROVENANCE_MAX: usize = 200;
const NOTE_MAX: usize = 500;
const MAPPINGS_MAX: usize = 20;
const LINKS_MAX: usize = 50;
const TITLE_MAX: usize = 120;

pub(crate) fn put_signal(conn: &Connection, signal: &CustomerSignal) -> Result<()> {
    conn.execute(
        "INSERT INTO customer_signals VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![signal.id, serde_json::to_string(signal)?],
    )?;
    Ok(())
}

fn signal(conn: &Connection, id: &str) -> Result<CustomerSignal> {
    let data: Option<String> = conn
        .query_row("SELECT data FROM customer_signals WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", "Unknown customer request")
    })?)?)
}

fn current(conn: &Connection, id: &str, expected: u64) -> Result<CustomerSignal> {
    let signal = signal(conn, id)?;
    if signal.version != expected {
        return Err(err(
            "conflict",
            format!(
                "This customer request is version {}, not {expected}. Refresh before retrying.",
                signal.version
            ),
        ));
    }
    Ok(signal)
}

/// Contact details do not belong in the privacy-safe customer reference.
fn looks_like_contact(value: &str) -> bool {
    let email = value.split_whitespace().any(|word| {
        word.split_once('@')
            .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'))
    });
    let digits = value.chars().filter(|c| c.is_ascii_digit()).count();
    let phone = digits >= 7
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || " +-().".contains(c));
    email || phone
}

struct Fields {
    source_reference: String,
    summary: String,
    customer_reference: String,
    external_source: Option<String>,
    external_id: Option<String>,
    unresolved_mappings: Vec<UnresolvedMapping>,
}

fn validate_fields(
    source_reference: &str,
    summary: &str,
    received_at: i64,
    customer_reference: &str,
    external_source: Option<&str>,
    external_id: Option<&str>,
    mappings: &[UnresolvedMapping],
) -> Result<Fields> {
    let summary = summary.trim();
    required(summary, "summary")?;
    if summary.chars().count() > SUMMARY_MAX {
        return Err(err(
            "invalid",
            "Request text must be at most 2,000 characters",
        ));
    }
    let source_reference = source_reference.trim();
    if source_reference.chars().count() > REFERENCE_MAX {
        return Err(err("invalid", "Source reference is too long"));
    }
    let customer_reference = customer_reference.trim();
    if customer_reference.chars().count() > CUSTOMER_MAX {
        return Err(err("invalid", "Customer reference is too long"));
    }
    if looks_like_contact(customer_reference) {
        return Err(err(
            "invalid",
            "Use a privacy-safe customer reference (account, segment or ID), not an email address or phone number",
        ));
    }
    if received_at <= 0 {
        return Err(err("invalid", "Received time is required"));
    }
    let trim = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(String::from)
    };
    let (external_source, external_id) = (trim(external_source), trim(external_id));
    if external_source.is_some() != external_id.is_some() {
        return Err(err(
            "invalid",
            "External source and external ID must be provided together",
        ));
    }
    for value in external_source.iter().chain(external_id.iter()) {
        if value.chars().count() > PROVENANCE_MAX {
            return Err(err("invalid", "External provenance is too long"));
        }
    }
    if mappings.len() > MAPPINGS_MAX {
        return Err(err("invalid", "Too many unresolved mappings"));
    }
    let mut unresolved = Vec::new();
    for mapping in mappings {
        let source = mapping.external_source.trim();
        let id = mapping.external_id.trim();
        if source.is_empty()
            || id.is_empty()
            || source.chars().count() > PROVENANCE_MAX
            || id.chars().count() > PROVENANCE_MAX
            || mapping.note.chars().count() > NOTE_MAX
        {
            return Err(err("invalid", "Invalid unresolved mapping"));
        }
        let mapping = UnresolvedMapping {
            external_source: source.into(),
            external_id: id.into(),
            note: mapping.note.trim().into(),
        };
        if !unresolved.contains(&mapping) {
            unresolved.push(mapping);
        }
    }
    Ok(Fields {
        source_reference: source_reference.into(),
        summary: summary.into(),
        customer_reference: customer_reference.into(),
        external_source,
        external_id,
        unresolved_mappings: unresolved,
    })
}

/// A link target must be real parent work or a project in the signal's product.
fn check_target(
    conn: &Connection,
    product_id: &str,
    kind: SignalTargetKind,
    target: &str,
) -> Result<()> {
    match kind {
        SignalTargetKind::Issue => {
            let issue = issue(conn, target)?;
            if issue.parent.is_some() {
                return Err(err(
                    "invalid",
                    "Link the parent issue, not its verification issue",
                ));
            }
            if issue.product_id != product_id {
                return Err(err(
                    "invalid",
                    "The issue belongs to another product than this request",
                ));
            }
        }
        SignalTargetKind::Project => {
            let project = project(conn, target)?;
            if project.product_id != product_id {
                return Err(err(
                    "invalid",
                    "The project belongs to another product than this request",
                ));
            }
        }
    }
    Ok(())
}

fn promotion_title(summary: &str) -> String {
    let first = summary
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(summary)
        .trim();
    if first.chars().count() <= TITLE_MAX {
        first.into()
    } else {
        let cut: String = first.chars().take(TITLE_MAX - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

fn kind_name(kind: SignalSourceKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role: Role,
    at: i64,
) -> Result<Value> {
    match cmd {
        Command::CaptureSignal {
            product,
            source_kind,
            source_reference,
            summary,
            received_at,
            customer_reference,
            external_source,
            external_id,
            unresolved_mappings,
        } => {
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.key == *product)
                .ok_or_else(|| err("not_found", "Unknown product"))?;
            let fields = validate_fields(
                source_reference,
                summary,
                *received_at,
                customer_reference,
                external_source.as_deref(),
                external_id.as_deref(),
                unresolved_mappings,
            )?;
            if let (Some(source), Some(external)) = (&fields.external_source, &fields.external_id) {
                if all::<CustomerSignal>(tx, "customer_signals")?
                    .iter()
                    .any(|other| {
                        other.product_id == product.id
                            && other.external_source.as_ref() == Some(source)
                            && other.external_id.as_ref() == Some(external)
                    })
                {
                    return Err(err(
                        "conflict",
                        "A customer request with this external source and ID is already recorded",
                    ));
                }
            }
            let signal = CustomerSignal {
                id: id(),
                product_id: product.id,
                source_kind: *source_kind,
                source_reference: fields.source_reference,
                summary: fields.summary,
                received_at: *received_at,
                customer_reference: fields.customer_reference,
                external_source: fields.external_source,
                external_id: fields.external_id,
                unresolved_mappings: fields.unresolved_mappings,
                links: vec![],
                promoted_issue_key: None,
                archived: false,
                version: 1,
                created_by: actor.into(),
                created_at: at,
                updated_at: at,
            };
            put_signal(tx, &signal)?;
            emit(tx, actor, "signal_captured", &signal.id, at)?;
            Ok(json!(signal))
        }
        Command::LinkSignal {
            id,
            expected_version,
            kind,
            target,
        } => {
            let mut signal = current(tx, id, *expected_version)?;
            let target = target.trim();
            check_target(tx, &signal.product_id, *kind, target)?;
            if signal
                .links
                .iter()
                .any(|link| link.kind == *kind && link.target == target)
            {
                return Err(err("conflict", "This request is already linked there"));
            }
            if signal.links.len() >= LINKS_MAX {
                return Err(err("invalid", "Too many links on one request"));
            }
            signal.links.push(SignalLink {
                kind: *kind,
                target: target.into(),
                linked_by: actor.into(),
                linked_at: at,
            });
            signal.version += 1;
            signal.updated_at = at;
            put_signal(tx, &signal)?;
            emit(tx, actor, "signal_linked", id, at)?;
            Ok(json!(signal))
        }
        Command::UnlinkSignal {
            id,
            expected_version,
            kind,
            target,
        } => {
            let mut signal = current(tx, id, *expected_version)?;
            let before = signal.links.len();
            signal
                .links
                .retain(|link| !(link.kind == *kind && link.target == target.trim()));
            if signal.links.len() == before {
                return Err(err("not_found", "This request is not linked there"));
            }
            signal.version += 1;
            signal.updated_at = at;
            put_signal(tx, &signal)?;
            emit(tx, actor, "signal_unlinked", id, at)?;
            Ok(json!(signal))
        }
        Command::PromoteSignal {
            id,
            expected_version,
            title,
        } => {
            let mut signal = current(tx, id, *expected_version)?;
            if let Some(key) = &signal.promoted_issue_key {
                return Err(err(
                    "conflict",
                    format!("This request was already promoted to {key}"),
                ));
            }
            if signal.archived {
                return Err(err(
                    "invalid",
                    "Restore the archived request before promoting it",
                ));
            }
            let product = all::<Product>(tx, "products")?
                .into_iter()
                .find(|p| p.id == signal.product_id)
                .ok_or_else(|| err("invalid", "The request's product is missing"))?;
            let title = title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(String::from)
                .unwrap_or_else(|| promotion_title(&signal.summary));
            let mut provenance = format!(
                "Promoted from customer request {} ({}",
                signal.id,
                kind_name(signal.source_kind)
            );
            if !signal.source_reference.is_empty() {
                provenance.push_str(&format!(" · {}", signal.source_reference));
            }
            if !signal.customer_reference.is_empty() {
                provenance.push_str(&format!("; customer: {}", signal.customer_reference));
            }
            provenance.push_str(").");
            let body = format!("Customer request\n{}\n\n{provenance}", signal.summary);
            // The normal capture path applies product defaults and records issue history.
            let created = super::mutate(
                tx,
                &Command::CreateIssue {
                    product: product.key,
                    title,
                    body,
                    acceptance: String::new(),
                    owner: String::new(),
                    priority: "medium".into(),
                    planning_scope: PlanningScope::Inbox,
                    project_id: None,
                    template: None,
                    intake: None,
                },
                actor,
                role,
                at,
            )?;
            let key = created["key"]
                .as_str()
                .ok_or_else(|| err("invalid", "Promotion did not create an issue"))?
                .to_string();
            signal.promoted_issue_key = Some(key.clone());
            signal.links.push(SignalLink {
                kind: SignalTargetKind::Issue,
                target: key.clone(),
                linked_by: actor.into(),
                linked_at: at,
            });
            signal.version += 1;
            signal.updated_at = at;
            put_signal(tx, &signal)?;
            emit(tx, actor, "signal_promoted", id, at)?;
            Ok(json!({"signal": signal, "issue": created}))
        }
        Command::ArchiveSignal {
            id,
            expected_version,
            archived,
        } => {
            let mut signal = current(tx, id, *expected_version)?;
            if signal.archived == *archived {
                return Err(err(
                    "conflict",
                    if *archived {
                        "This request is already archived"
                    } else {
                        "This request is not archived"
                    },
                ));
            }
            signal.archived = *archived;
            signal.version += 1;
            signal.updated_at = at;
            put_signal(tx, &signal)?;
            emit(
                tx,
                actor,
                if *archived {
                    "signal_archived"
                } else {
                    "signal_restored"
                },
                id,
                at,
            )?;
            Ok(json!(signal))
        }
        _ => unreachable!("not a customer signal command"),
    }
}

/// Requests linked to, or promoted into, an issue.
pub(crate) fn for_issue(conn: &Connection, key: &str) -> Result<Vec<CustomerSignal>> {
    Ok(all::<CustomerSignal>(conn, "customer_signals")?
        .into_iter()
        .filter(|signal| {
            signal.promoted_issue_key.as_deref() == Some(key)
                || signal
                    .links
                    .iter()
                    .any(|link| link.kind == SignalTargetKind::Issue && link.target == key)
        })
        .collect())
}

/// Deletion blockers: links can be removed; promotion provenance is retained.
pub(crate) fn deletion_references(
    conn: &Connection,
    key: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut linked = Vec::new();
    let mut promoted = Vec::new();
    for signal in for_issue(conn, key)? {
        if signal.promoted_issue_key.as_deref() == Some(key) {
            promoted.push(signal.id);
        } else {
            linked.push(signal.id);
        }
    }
    Ok((linked, promoted))
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 16 && !a.customer_signals.is_empty() {
        return Err(err(
            "invalid",
            "Customer request data requires archive format 16",
        ));
    }
    let invalid = |message: &str| {
        err(
            "invalid",
            format!("Invalid customer request archive: {message}"),
        )
    };
    let parents: HashMap<&str, &Issue> = a
        .issues
        .iter()
        .filter(|issue| issue.parent.is_none())
        .map(|issue| (issue.key.as_str(), issue))
        .collect();
    let projects: HashMap<&str, &Project> = a.projects.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut ids = HashSet::new();
    let mut external = HashSet::new();
    let mut promoted = HashSet::new();
    for signal in &a.customer_signals {
        if Uuid::parse_str(&signal.id).is_err()
            || !ids.insert(signal.id.as_str())
            || signal.version == 0
        {
            return Err(invalid("identity"));
        }
        if !a.products.iter().any(|p| p.id == signal.product_id) {
            return Err(invalid("unknown product"));
        }
        let fields = validate_fields(
            &signal.source_reference,
            &signal.summary,
            signal.received_at,
            &signal.customer_reference,
            signal.external_source.as_deref(),
            signal.external_id.as_deref(),
            &signal.unresolved_mappings,
        )
        .map_err(|_| invalid("fields"))?;
        if fields.summary != signal.summary
            || fields.source_reference != signal.source_reference
            || fields.customer_reference != signal.customer_reference
            || fields.unresolved_mappings != signal.unresolved_mappings
        {
            return Err(invalid("fields"));
        }
        if let (Some(source), Some(id)) = (&signal.external_source, &signal.external_id) {
            if !external.insert((signal.product_id.as_str(), source.as_str(), id.as_str())) {
                return Err(invalid("duplicate external ID"));
            }
        }
        let mut seen = HashSet::new();
        for link in &signal.links {
            if !seen.insert((link.kind, link.target.as_str())) {
                return Err(invalid("duplicate link"));
            }
            let product = match link.kind {
                SignalTargetKind::Issue => parents.get(link.target.as_str()).map(|i| &i.product_id),
                SignalTargetKind::Project => {
                    projects.get(link.target.as_str()).map(|p| &p.product_id)
                }
            };
            if product != Some(&signal.product_id) {
                return Err(invalid("link target"));
            }
        }
        if let Some(key) = &signal.promoted_issue_key {
            if parents.get(key.as_str()).map(|i| &i.product_id) != Some(&signal.product_id)
                || !promoted.insert(key.as_str())
            {
                return Err(invalid("promotion"));
            }
        }
    }
    Ok(())
}
