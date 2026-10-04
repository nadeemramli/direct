//! Versioned agent roles and skill packages (DIR-74).
//!
//! Canonical methods stay in the Development Operating System. Direct records
//! immutable operational revisions: which DOS documents a role pinned (with
//! fingerprints), which exact skill bundle it uses, who activated it under
//! what owner direction, where it was published and what a fresh session did
//! with it. Role and skill text is task context only: no service permission
//! reads it. Activation and retirement are owner decisions.

use super::*;
use serde::Serialize;

pub const HARNESS_CLAUDE_CODE: &str = "claude-code";
const LIST_MAX: usize = 20;
const ITEM_MAX: usize = 500;
const FILES_MAX: usize = 20;
const BUNDLE_MAX: usize = 256 * 1024;
const EXCERPT_MAX: usize = 2_000;

fn hex(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn content_sha256(content: &str) -> String {
    hex(content.as_bytes())
}

fn put<T: Serialize>(conn: &Connection, table: &str, id: &str, row: &T) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO {table} VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data"
        ),
        params![id, serde_json::to_string(row)?],
    )?;
    Ok(())
}

pub(crate) fn put_skill(conn: &Connection, s: &SkillPackage) -> Result<()> {
    put(conn, "skill_packages", &s.id, s)
}
pub(crate) fn put_role(conn: &Connection, r: &AgentRole) -> Result<()> {
    put(conn, "agent_roles", &r.id, r)
}
pub(crate) fn put_publication(conn: &Connection, p: &RolePublication) -> Result<()> {
    put(conn, "role_publications", &p.id, p)
}

fn find<T: DeserializeOwned>(conn: &Connection, table: &str, id: &str, what: &str) -> Result<T> {
    let data: Option<String> = conn
        .query_row(
            &format!("SELECT data FROM {table} WHERE id=?1"),
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or_else(|| {
        err("not_found", format!("Unknown {what}"))
    })?)?)
}

fn skill(conn: &Connection, id: &str) -> Result<SkillPackage> {
    find(conn, "skill_packages", id, "skill package")
}
fn role(conn: &Connection, id: &str) -> Result<AgentRole> {
    find(conn, "agent_roles", id, "agent role")
}
fn publication(conn: &Connection, id: &str) -> Result<RolePublication> {
    find(conn, "role_publications", id, "role publication")
}

fn list(values: &[String], field: &str, min: usize) -> Result<Vec<String>> {
    if values.len() < min || values.len() > LIST_MAX {
        return Err(err("invalid", format!("Provide {min}–{LIST_MAX} {field}")));
    }
    values
        .iter()
        .map(|v| {
            let v = v.trim();
            required(v, field)?;
            limited(v, field, ITEM_MAX)?;
            Ok(v.to_string())
        })
        .collect()
}

/// A bundle-relative path: plain segments only, never absolute or escaping.
pub fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.starts_with('/')
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

/// An absolute destination string with no parent-directory segments.
fn absolute_destination(value: &str) -> bool {
    let drive = value.len() > 2
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value.as_bytes()[1] == b':'
        && matches!(value.as_bytes()[2], b'/' | b'\\');
    (drive || value.starts_with('/')) && !value.split(['/', '\\']).any(|segment| segment == "..")
}

fn frontmatter_names(skill_md: &str, name: &str) -> bool {
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    let mut named = false;
    let mut described = false;
    for line in lines {
        let line = line.trim();
        if line == "---" {
            return named && described;
        }
        if let Some(value) = line.strip_prefix("name:") {
            named = value.trim().trim_matches(['"', '\'']) == name;
        }
        if let Some(value) = line.strip_prefix("description:") {
            described = !value.trim().is_empty();
        }
    }
    false
}

fn bundle_sha256(files: &[SkillFile]) -> String {
    let mut sorted: Vec<_> = files.iter().map(|f| (&f.path, &f.sha256)).collect();
    sorted.sort();
    hex(serde_json::to_vec(&sorted).unwrap_or_default())
}

fn next_revision<'a>(revisions: impl Iterator<Item = &'a u32>) -> u32 {
    revisions.max().copied().unwrap_or(0) + 1
}

/// The files a Claude Code project-scoped publication writes, relative to the
/// destination, with their exact content. Pure: the CLI previews and applies it.
pub fn role_publication_plan(
    role: &AgentRole,
    skills: &[SkillPackage],
    harness: &str,
) -> Result<Vec<(String, String)>> {
    if harness != HARNESS_CLAUDE_CODE {
        return Err(err(
            "invalid",
            format!("Publishing for {harness} is not supported yet; it stays unverified"),
        ));
    }
    let mut plan = Vec::new();
    let mut names = Vec::new();
    for id in &role.skills {
        let s = skills
            .iter()
            .find(|s| s.id == *id)
            .ok_or_else(|| err("invalid", format!("Skill revision {id} is missing")))?;
        names.push(s.name.clone());
        for f in &s.files {
            plan.push((
                format!(".claude/skills/{}/{}", s.name, f.path),
                f.content.clone(),
            ));
        }
    }
    let bullets = |items: &[String]| items.iter().map(|i| format!("- {i}\n")).collect::<String>();
    let mut agent = format!(
        "---\nname: {key}\ndescription: {name} (Direct role {key}, revision {rev}). {first}\n---\n\n# {name}\n\nDirect role `{key}` revision {rev} (`{id}`).\n\n## Responsibilities\n\n{resp}\n## Inputs\n\n{inputs}\n## Outputs\n\n{outputs}",
        key = role.key,
        name = role.name,
        rev = role.revision,
        id = role.id,
        first = role.responsibilities.first().map(String::as_str).unwrap_or(""),
        resp = bullets(&role.responsibilities),
        inputs = bullets(&role.inputs),
        outputs = bullets(&role.outputs),
    );
    if !names.is_empty() {
        agent += "\n## Skills\n\n";
        for n in &names {
            agent += &format!("- Use the `{n}` skill.\n");
        }
    }
    agent += "\n## Guidance\n\nCanonical guidance lives in the Development Operating System; read it through Direct Theoria.\n\n";
    for g in &role.guidance {
        agent += &format!(
            "- `{}`{} · fingerprint `{}` · playbook {}\n",
            g.document_id,
            if g.mandatory { " (mandatory)" } else { "" },
            g.recorded_fingerprint.as_deref().unwrap_or("unavailable"),
            g.playbook_version.as_deref().unwrap_or("Unknown"),
        );
    }
    if !role.owner_direction.is_empty() {
        agent += &format!("\n## Owner direction\n\n{}\n", role.owner_direction);
    }
    agent += "\n## Authority\n\nThis file is task context. It grants no tool authority; Direct enforces permissions in the service, and readiness, review and method acceptance stay with the owner.\n";
    plan.push((format!(".claude/agents/{}.md", role.key), agent));
    Ok(plan)
}

pub(crate) fn mutate(
    tx: &Transaction,
    cmd: &Command,
    actor: &str,
    role_kind: Role,
    at: i64,
) -> Result<Value> {
    match cmd {
        Command::RegisterSkillPackage {
            name,
            description,
            trigger,
            origin,
            upstream,
            license,
            adaptations,
            files,
        } => {
            let name = name.trim();
            if !stable_id(name) || name.len() > 64 {
                return Err(err("invalid", "Skill names are lowercase slugs"));
            }
            for (value, field, max) in [
                (description.trim(), "skill description", 1_000),
                (trigger.trim(), "skill trigger", 1_000),
                (license.trim(), "skill license", 200),
            ] {
                required(value, field)?;
                limited(value, field, max)?;
            }
            limited(adaptations, "adaptations", 4_000)?;
            match (origin, upstream) {
                (SkillOrigin::Local, None) => {}
                (SkillOrigin::Local, Some(_)) => {
                    return Err(err("invalid", "A local skill has no upstream"))
                }
                (SkillOrigin::Upstream, Some(u)) => {
                    if !u
                        .repository
                        .split_once('/')
                        .is_some_and(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'))
                    {
                        return Err(err("invalid", "Upstream repository must be owner/name"));
                    }
                    if !valid_commit_sha(&u.commit) {
                        return Err(err(
                            "invalid",
                            "Pin the upstream to an immutable commit SHA, not a branch or latest",
                        ));
                    }
                    if !u.path.is_empty() && !safe_relative_path(&u.path) {
                        return Err(err("invalid", "Upstream path must be repository-relative"));
                    }
                }
                (SkillOrigin::Upstream, None) => {
                    return Err(err(
                        "invalid",
                        "An upstream skill needs its repository and commit",
                    ))
                }
            }
            if files.is_empty() || files.len() > FILES_MAX {
                return Err(err("invalid", "Provide 1–20 bundle files"));
            }
            let mut seen = HashSet::new();
            let mut total = 0;
            let mut stored = Vec::with_capacity(files.len());
            for f in files {
                if !safe_relative_path(&f.path) {
                    return Err(err(
                        "invalid",
                        format!(
                            "Bundle path {} must be relative and stay inside the bundle",
                            f.path
                        ),
                    ));
                }
                if !seen.insert(f.path.clone()) {
                    return Err(err("invalid", format!("Duplicate bundle path {}", f.path)));
                }
                total += f.content.len();
                stored.push(SkillFile {
                    path: f.path.clone(),
                    sha256: content_sha256(&f.content),
                    content: f.content.clone(),
                });
            }
            if total > BUNDLE_MAX {
                return Err(err("invalid", "Skill bundles are limited to 256 KiB"));
            }
            let skill_md = stored
                .iter()
                .find(|f| f.path == "SKILL.md")
                .ok_or_else(|| err("invalid", "A skill bundle needs SKILL.md at its root"))?;
            if !frontmatter_names(&skill_md.content, name) {
                return Err(err(
                    "invalid",
                    format!("SKILL.md frontmatter must declare name: {name} and a description"),
                ));
            }
            let existing: Vec<SkillPackage> = all::<SkillPackage>(tx, "skill_packages")?
                .into_iter()
                .filter(|s| s.name == name)
                .collect();
            let bundle = bundle_sha256(&stored);
            if let Some(same) = existing
                .iter()
                .find(|s| s.bundle_sha256 == bundle && s.retired.is_none())
            {
                return Err(err(
                    "conflict",
                    format!(
                        "This bundle is unchanged; reuse {name} revision {}",
                        same.revision
                    ),
                ));
            }
            let s = SkillPackage {
                id: id(),
                name: name.into(),
                revision: next_revision(existing.iter().map(|s| &s.revision)),
                description: description.trim().into(),
                trigger: trigger.trim().into(),
                origin: origin.clone(),
                upstream: upstream.clone(),
                license: license.trim().into(),
                adaptations: adaptations.trim().into(),
                files: stored,
                bundle_sha256: bundle,
                registered_by: actor.into(),
                registered_at: at,
                retired: None,
            };
            put_skill(tx, &s)?;
            emit(tx, actor, "skill_package_registered", &s.id, at)?;
            Ok(serde_json::to_value(s)?)
        }
        Command::RetireSkillPackage { id, reason } => {
            human(role_kind)?;
            let mut s = skill(tx, id)?;
            if s.retired.is_some() {
                return Err(err("conflict", "This skill revision is already retired"));
            }
            let reason = reason.trim();
            required(reason, "retirement reason")?;
            limited(reason, "retirement reason", 1_000)?;
            s.retired = Some(Retirement {
                by: actor.into(),
                at,
                reason: reason.into(),
            });
            put_skill(tx, &s)?;
            emit(tx, actor, "skill_package_retired", &s.id, at)?;
            Ok(serde_json::to_value(s)?)
        }
        Command::RegisterAgentRole {
            key,
            name,
            responsibilities,
            inputs,
            outputs,
            skills,
            runtime_compatibility,
            guidance,
            owner_direction,
        } => {
            let key = key.trim();
            if !stable_id(key) || key.len() > 64 {
                return Err(err("invalid", "Role keys are lowercase slugs"));
            }
            let name = name.trim();
            required(name, "role name")?;
            limited(name, "role name", 120)?;
            let responsibilities = list(responsibilities, "responsibilities", 1)?;
            let inputs = list(inputs, "inputs", 1)?;
            let outputs = list(outputs, "outputs", 1)?;
            let runtimes = list(runtime_compatibility, "runtime compatibility entries", 1)?;
            if runtimes.iter().any(|r| !stable_id(r)) {
                return Err(err("invalid", "Runtime names are lowercase slugs"));
            }
            if skills.len() > LIST_MAX {
                return Err(err("invalid", "A role can require at most 20 skills"));
            }
            let mut skill_names = HashSet::new();
            for id in skills {
                let s = skill(tx, id)?;
                if s.retired.is_some() {
                    return Err(err(
                        "invalid",
                        format!("Skill {} revision {} is retired", s.name, s.revision),
                    ));
                }
                if !skill_names.insert(s.name) {
                    return Err(err("invalid", "A role can use one revision of each skill"));
                }
            }
            if guidance.is_empty() || guidance.len() > LIST_MAX {
                return Err(err("invalid", "Pin 1–20 DOS guidance documents"));
            }
            let mut pinned = HashSet::new();
            let mut pins = Vec::with_capacity(guidance.len());
            for g in guidance {
                if !pinned.insert(g.document_id.clone()) {
                    return Err(err("invalid", "Each guidance document is pinned once"));
                }
                let document = theoria_document(tx, &g.document_id)?;
                let playbook_version = g
                    .playbook_version
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);
                if let Some(v) = &playbook_version {
                    limited(v, "playbook version", 80)?;
                }
                pins.push(RoleGuidancePin {
                    document_id: document.id,
                    recorded_fingerprint: document.fingerprint,
                    playbook_version,
                    mandatory: g.mandatory,
                });
            }
            limited(owner_direction, "owner direction", 2_000)?;
            let existing: Vec<AgentRole> = all::<AgentRole>(tx, "agent_roles")?
                .into_iter()
                .filter(|r| r.key == key)
                .collect();
            let r = AgentRole {
                id: id(),
                key: key.into(),
                revision: next_revision(existing.iter().map(|r| &r.revision)),
                name: name.into(),
                responsibilities,
                inputs,
                outputs,
                skills: skills.clone(),
                runtime_compatibility: runtimes,
                guidance: pins,
                owner_direction: owner_direction.trim().into(),
                status: RoleStatus::Draft,
                registered_by: actor.into(),
                registered_at: at,
                activation: None,
                retired: None,
            };
            put_role(tx, &r)?;
            emit(tx, actor, "agent_role_registered", &r.id, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::ActivateAgentRole { id, note } => {
            human(role_kind)?;
            let mut r = role(tx, id)?;
            if r.status != RoleStatus::Draft {
                return Err(err(
                    "conflict",
                    "Only a draft role revision can be activated",
                ));
            }
            if r.owner_direction.trim().is_empty() {
                return Err(err(
                    "invalid",
                    "Record the owner direction for bounded use in a revision before activating it",
                ));
            }
            for sid in &r.skills {
                let s = skill(tx, sid)?;
                if s.retired.is_some() {
                    return Err(err(
                        "invalid",
                        format!("Skill {} revision {} is retired", s.name, s.revision),
                    ));
                }
            }
            let unavailable: Vec<String> = r
                .guidance
                .iter()
                .filter(|g| g.mandatory)
                .filter(|g| {
                    theoria_document(tx, &g.document_id)
                        .map(|d| d.availability != TheoriaAvailability::Available)
                        .unwrap_or(true)
                })
                .map(|g| g.document_id.clone())
                .collect();
            if !unavailable.is_empty() {
                return Err(err(
                    "invalid",
                    format!(
                        "Mandatory guidance is unavailable: {}. Restore the source and re-sync before activating",
                        unavailable.join(", ")
                    ),
                ));
            }
            let note = note.trim();
            limited(note, "activation note", 1_000)?;
            for mut previous in all::<AgentRole>(tx, "agent_roles")?
                .into_iter()
                .filter(|p| p.key == r.key && p.status == RoleStatus::Active)
            {
                previous.status = RoleStatus::Superseded;
                put_role(tx, &previous)?;
            }
            r.status = RoleStatus::Active;
            r.activation = Some(RoleActivation {
                by: actor.into(),
                at,
                note: note.into(),
            });
            put_role(tx, &r)?;
            emit(tx, actor, "agent_role_activated", &r.id, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::RetireAgentRole { id, reason } => {
            human(role_kind)?;
            let mut r = role(tx, id)?;
            if r.status == RoleStatus::Retired {
                return Err(err("conflict", "This role revision is already retired"));
            }
            let reason = reason.trim();
            required(reason, "retirement reason")?;
            limited(reason, "retirement reason", 1_000)?;
            r.status = RoleStatus::Retired;
            r.retired = Some(Retirement {
                by: actor.into(),
                at,
                reason: reason.into(),
            });
            put_role(tx, &r)?;
            emit(tx, actor, "agent_role_retired", &r.id, at)?;
            Ok(serde_json::to_value(r)?)
        }
        Command::RecordRolePublication {
            role_id,
            harness,
            destination,
            introduced,
        } => {
            let r = role(tx, role_id)?;
            if !matches!(r.status, RoleStatus::Draft | RoleStatus::Active) {
                return Err(err(
                    "invalid",
                    "Retired or superseded role revisions are not published",
                ));
            }
            let destination = destination.trim();
            if !absolute_destination(destination) || destination.len() > 1_000 {
                return Err(err(
                    "invalid",
                    "Destination must be an absolute project directory",
                ));
            }
            let skills: Vec<SkillPackage> = all(tx, "skill_packages")?;
            let plan = role_publication_plan(&r, &skills, harness)?;
            let mut seen = HashSet::new();
            for f in introduced {
                let expected = plan
                    .iter()
                    .find(|(path, _)| *path == f.path)
                    .ok_or_else(|| {
                        err(
                            "invalid",
                            format!("{} is not part of this publication", f.path),
                        )
                    })?;
                if content_sha256(&expected.1) != f.sha256 || !seen.insert(f.path.clone()) {
                    return Err(err(
                        "invalid",
                        format!("{} does not match the planned content", f.path),
                    ));
                }
            }
            let p = RolePublication {
                id: id(),
                role_id: r.id.clone(),
                harness: harness.clone(),
                destination: destination.into(),
                introduced: introduced.clone(),
                published_by: actor.into(),
                published_at: at,
                evidence: vec![],
                rollback: None,
            };
            put_publication(tx, &p)?;
            emit(tx, actor, "role_published", &r.id, at)?;
            Ok(serde_json::to_value(p)?)
        }
        Command::RecordActivationEvidence {
            publication_id,
            session_id,
            model,
            harness_version,
            marker,
            output,
        } => {
            let mut p = publication(tx, publication_id)?;
            if p.rollback.is_some() {
                return Err(err("invalid", "This publication was rolled back"));
            }
            for (value, field) in [
                (session_id, "session ID"),
                (model, "model"),
                (marker, "marker"),
            ] {
                required(value.trim(), field)?;
                limited(value, field, 200)?;
            }
            limited(harness_version, "harness version", 200)?;
            limited(output, "session output", 200_000)?;
            if !output.contains(marker.trim()) {
                return Err(err(
                    "invalid",
                    "The session output does not contain the expected marker; the role or skill was not shown to run",
                ));
            }
            p.evidence.push(ActivationEvidence {
                session_id: session_id.trim().into(),
                model: model.trim().into(),
                harness_version: harness_version.trim().into(),
                marker: marker.trim().into(),
                output_excerpt: output.chars().take(EXCERPT_MAX).collect(),
                output_sha256: content_sha256(output),
                recorded_by: actor.into(),
                recorded_at: at,
            });
            put_publication(tx, &p)?;
            emit(tx, actor, "role_activation_evidence", &p.role_id, at)?;
            Ok(serde_json::to_value(p)?)
        }
        Command::RecordPublicationRollback {
            publication_id,
            removed,
            kept_modified,
        } => {
            let mut p = publication(tx, publication_id)?;
            if p.rollback.is_some() {
                return Err(err("conflict", "This publication was already rolled back"));
            }
            let introduced: HashSet<&String> = p.introduced.iter().map(|f| &f.path).collect();
            let mut covered = HashSet::new();
            for path in removed.iter().chain(kept_modified) {
                if !introduced.contains(path) || !covered.insert(path) {
                    return Err(err(
                        "invalid",
                        format!("{path} was not introduced by this publication"),
                    ));
                }
            }
            if covered.len() != introduced.len() {
                return Err(err("invalid", "Account for every introduced file"));
            }
            p.rollback = Some(PublicationRollback {
                by: actor.into(),
                at,
                removed: removed.clone(),
                kept_modified: kept_modified.clone(),
            });
            put_publication(tx, &p)?;
            emit(tx, actor, "role_publication_rolled_back", &p.role_id, at)?;
            Ok(serde_json::to_value(p)?)
        }
        _ => unreachable!("not a role command"),
    }
}

pub(crate) fn validate_archive(a: &Archive) -> Result<()> {
    if a.format < 20
        && (!a.skill_packages.is_empty()
            || !a.agent_roles.is_empty()
            || !a.role_publications.is_empty())
    {
        return Err(err("invalid", "Roles and skills require archive format 20"));
    }
    let invalid = |m: &str| err("invalid", format!("Invalid roles archive: {m}"));
    let mut ids = HashSet::new();
    let mut revisions = HashSet::new();
    for s in &a.skill_packages {
        if !ids.insert(s.id.as_str()) || !revisions.insert(("skill", s.name.as_str(), s.revision)) {
            return Err(invalid("duplicate skill revision"));
        }
        if s.files
            .iter()
            .any(|f| content_sha256(&f.content) != f.sha256)
            || bundle_sha256(&s.files) != s.bundle_sha256
        {
            return Err(invalid("skill hash mismatch"));
        }
    }
    let mut active = HashSet::new();
    for r in &a.agent_roles {
        if !ids.insert(r.id.as_str()) || !revisions.insert(("role", r.key.as_str(), r.revision)) {
            return Err(invalid("duplicate role revision"));
        }
        if r.skills
            .iter()
            .any(|sid| !a.skill_packages.iter().any(|s| s.id == *sid))
            || r.guidance
                .iter()
                .any(|g| !a.theoria_documents.iter().any(|d| d.id == g.document_id))
        {
            return Err(invalid("unresolved role reference"));
        }
        if r.status == RoleStatus::Active && !active.insert(r.key.as_str()) {
            return Err(invalid("two active revisions of one role"));
        }
    }
    for p in &a.role_publications {
        if !ids.insert(p.id.as_str()) || !a.agent_roles.iter().any(|r| r.id == p.role_id) {
            return Err(invalid("unresolved publication"));
        }
    }
    Ok(())
}
