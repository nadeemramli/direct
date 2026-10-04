//! `direct roles …`: register skill bundles and publish role revisions into a
//! disposable project (DIR-74). The service records revisions and evidence;
//! this side owns every filesystem write and guards it: project scope only,
//! no escaping paths, no global or home-level destinations, no overwriting
//! files that differ, and rollback that removes only files it introduced.

use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use direct_core::{
    content_sha256, role_publication_plan, safe_relative_path, AgentRole, Command, PublishedFile,
    Request, Role, RolePublication, SkillFileInput, SkillOrigin, SkillPackage, SkillUpstream,
    HARNESS_CLAUDE_CODE,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Subcommand, Clone)]
pub enum RolesAction {
    /// Register an immutable skill package revision from a bundle directory.
    RegisterSkill {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        trigger: String,
        #[arg(long)]
        license: String,
        /// `local` or `upstream`.
        #[arg(long, default_value = "local")]
        origin: String,
        #[arg(long)]
        upstream_repo: Option<String>,
        #[arg(long)]
        upstream_commit: Option<String>,
        #[arg(long, default_value = "")]
        upstream_path: String,
        #[arg(long, default_value = "")]
        adaptations: String,
        #[arg(long)]
        request_id: String,
    },
    /// Preview (default) or apply a project-scoped publication of a role revision.
    Publish {
        #[arg(long)]
        role_id: String,
        /// Existing project directory to publish into.
        #[arg(long)]
        dest: PathBuf,
        #[arg(long, default_value = HARNESS_CLAUDE_CODE)]
        harness: String,
        /// Write the files and record the publication. Without it nothing is written.
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Remove only the files a publication introduced and still match.
    Rollback {
        #[arg(long)]
        publication_id: String,
        #[arg(long)]
        request_id: String,
    },
    /// Record a fresh harness session's output as activation evidence.
    Evidence {
        #[arg(long)]
        publication_id: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        marker: String,
        #[arg(long)]
        output_file: PathBuf,
        #[arg(long, default_value = "")]
        harness_version: String,
        #[arg(long)]
        request_id: String,
    },
}

fn call(client: &direct::Client, actor: &str, request_id: &str, command: Command) -> Result<Value> {
    client.call(
        &Request {
            actor: actor.into(),
            request_id: request_id.into(),
            command,
        },
        Role::Agent,
    )
}

fn snapshot(client: &direct::Client, actor: &str) -> Result<Value> {
    call(client, actor, "", Command::Snapshot)
}

/// Bundle files with `/`-separated relative paths, refusing links and escapes.
fn read_bundle(dir: &Path) -> Result<Vec<SkillFileInput>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<SkillFileInput>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_symlink() {
                bail!(
                    "{} is a link; bundles must contain plain files",
                    path.display()
                );
            }
            if kind.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if !safe_relative_path(&relative) {
                bail!("{relative} is not a safe bundle path");
            }
            let content = fs::read_to_string(&path)
                .with_context(|| format!("{relative} must be UTF-8 text"))?;
            out.push(SkillFileInput {
                path: relative,
                content,
            });
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    Ok(files)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .and_then(|h| fs::canonicalize(h).ok())
}

/// A project directory: existing, not a filesystem root, not the home
/// directory or an ancestor of it, and not inside a harness's global config.
pub fn check_destination(dest: &Path) -> Result<PathBuf> {
    let dest = fs::canonicalize(dest).with_context(|| {
        format!(
            "Destination {} must be an existing directory",
            dest.display()
        )
    })?;
    if !dest.is_dir() {
        bail!("Destination {} must be a directory", plain(&dest));
    }
    if dest.parent().is_none() {
        bail!("Refusing to publish into a filesystem root");
    }
    if let Some(home) = home() {
        if home.starts_with(&dest) {
            bail!(
                "Refusing to publish into {}: that is the home directory or above it, which would install globally",
                plain(&dest)
            );
        }
        for global in [".claude", ".codex", ".agents", ".config"] {
            if dest.starts_with(home.join(global)) {
                bail!(
                    "Refusing to publish inside {}: global harness configuration is out of scope",
                    plain(&home.join(global))
                );
            }
        }
    }
    Ok(dest)
}

/// The user-facing form of a canonical path. Windows canonicalization yields
/// verbatim `\\?\C:\...` paths; Direct records and shows `C:\...`.
pub fn plain(path: &Path) -> String {
    let text = path.display().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => rest.to_string(),
        _ => text,
    }
}

/// Resolve a planned relative path under `dest`, refusing anything that could
/// land outside it through `..` or an existing link.
fn target(dest: &Path, relative: &str) -> Result<PathBuf> {
    if !relative
        .split('/')
        .all(|s| !s.is_empty() && s != "." && s != "..")
    {
        bail!("{relative} would escape the destination");
    }
    let path = dest.join(relative);
    let mut probe = path.as_path();
    while let Some(parent) = probe.parent() {
        if parent.exists() {
            let resolved = fs::canonicalize(parent)?;
            if !resolved.starts_with(dest) {
                bail!("{relative} would escape the destination through a link");
            }
            break;
        }
        probe = parent;
    }
    if path.exists() && fs::symlink_metadata(&path)?.file_type().is_symlink() {
        bail!("{relative} is a link in the destination");
    }
    Ok(path)
}

pub struct Planned {
    pub path: String,
    pub content: String,
    pub state: &'static str,
}

/// Classify every planned file: `new`, `identical` (already present, not
/// introduced) or a collision, which is an error listing every conflict.
pub fn plan_destination(dest: &Path, plan: Vec<(String, String)>) -> Result<Vec<Planned>> {
    let mut out = Vec::new();
    let mut collisions = Vec::new();
    for (relative, content) in plan {
        let path = target(dest, &relative)?;
        let state = if path.is_dir() {
            collisions.push(format!("{relative} (a directory exists there)"));
            "collision"
        } else if path.exists() {
            if fs::read(&path)? == content.as_bytes() {
                "identical"
            } else {
                collisions.push(format!("{relative} (different content exists)"));
                "collision"
            }
        } else {
            "new"
        };
        out.push(Planned {
            path: relative,
            content,
            state,
        });
    }
    if !collisions.is_empty() {
        bail!(
            "Destination collisions; nothing was written. Move or remove these first, or publish into another project:\n- {}",
            collisions.join("\n- ")
        );
    }
    Ok(out)
}

/// Delete introduced files whose content still matches their recorded hash,
/// then any directories that left empty (never `dest` itself). Modified files
/// are kept and returned; already-missing files count as removed.
pub fn remove_introduced(
    dest: &Path,
    files: &[PublishedFile],
) -> Result<(Vec<String>, Vec<String>)> {
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for f in files {
        let path = target(dest, &f.path)?;
        match fs::read(&path) {
            Ok(bytes) if content_sha256(&String::from_utf8_lossy(&bytes)) == f.sha256 => {
                fs::remove_file(&path)?;
                removed.push(f.path.clone());
            }
            Ok(_) => kept.push(f.path.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => removed.push(f.path.clone()),
            Err(e) => return Err(e.into()),
        }
        let mut dir = path.parent();
        while let Some(d) = dir {
            if d == dest || !d.starts_with(dest) || fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
    Ok((removed, kept))
}

pub fn run(client: &direct::Client, actor: &str, action: RolesAction) -> Result<()> {
    match action {
        RolesAction::RegisterSkill {
            dir,
            name,
            description,
            trigger,
            license,
            origin,
            upstream_repo,
            upstream_commit,
            upstream_path,
            adaptations,
            request_id,
        } => {
            let origin = match origin.as_str() {
                "local" => SkillOrigin::Local,
                "upstream" => SkillOrigin::Upstream,
                other => bail!("origin must be local or upstream, not {other}"),
            };
            let upstream = match (upstream_repo, upstream_commit) {
                (Some(repository), Some(commit)) => Some(SkillUpstream {
                    repository,
                    commit,
                    path: upstream_path,
                }),
                (None, None) => None,
                _ => bail!("Provide both --upstream-repo and --upstream-commit"),
            };
            let files = read_bundle(&dir)?;
            let value = call(
                client,
                actor,
                &request_id,
                Command::RegisterSkillPackage {
                    name,
                    description,
                    trigger,
                    origin,
                    upstream,
                    license,
                    adaptations,
                    files,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        RolesAction::Publish {
            role_id,
            dest,
            harness,
            apply,
            request_id,
        } => {
            let snap = snapshot(client, actor)?;
            let roles: Vec<AgentRole> = serde_json::from_value(snap["agent_roles"].clone())?;
            let skills: Vec<SkillPackage> = serde_json::from_value(snap["skill_packages"].clone())?;
            let role = roles
                .iter()
                .find(|r| r.id == role_id)
                .ok_or_else(|| anyhow!("Unknown role revision {role_id}"))?;
            let plan =
                role_publication_plan(role, &skills, &harness).map_err(|e| anyhow!("{e}"))?;
            let dest = check_destination(&dest)?;
            let planned = plan_destination(&dest, plan)?;
            let files: Vec<Value> = planned
                .iter()
                .map(|p| json!({"path":p.path,"state":p.state,"sha256":content_sha256(&p.content)}))
                .collect();
            if !apply {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "preview":true,"role":format!("{} r{}", role.key, role.revision),
                        "harness":harness,"destination":plain(&dest),"files":files,
                        "note":"Nothing was written. Re-run with --apply to publish."
                    }))?
                );
                return Ok(());
            }
            let request_id = request_id.ok_or_else(|| anyhow!("--apply needs --request-id"))?;
            let mut introduced = Vec::new();
            let written = (|| -> Result<()> {
                for p in planned.iter().filter(|p| p.state == "new") {
                    let path = target(&dest, &p.path)?;
                    fs::create_dir_all(path.parent().expect("planned files have a parent"))?;
                    let mut f = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .with_context(|| format!("{} appeared while publishing", p.path))?;
                    introduced.push(PublishedFile {
                        path: p.path.clone(),
                        sha256: content_sha256(&p.content),
                    });
                    std::io::Write::write_all(&mut f, p.content.as_bytes())?;
                }
                Ok(())
            })();
            let recorded = written.and_then(|()| {
                call(
                    client,
                    actor,
                    &request_id,
                    Command::RecordRolePublication {
                        role_id: role.id.clone(),
                        harness,
                        destination: plain(&dest),
                        introduced: introduced.clone(),
                    },
                )
            });
            match recorded {
                Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
                Err(e) => {
                    // Untracked files must not be left behind: remove what this run wrote.
                    let (removed, _) = remove_introduced(&dest, &introduced)?;
                    bail!(
                        "{e}\nNothing was recorded; removed the {} file(s) this run wrote.",
                        removed.len()
                    );
                }
            }
        }
        RolesAction::Rollback {
            publication_id,
            request_id,
        } => {
            let snap = snapshot(client, actor)?;
            let publications: Vec<RolePublication> =
                serde_json::from_value(snap["role_publications"].clone())?;
            let p = publications
                .iter()
                .find(|p| p.id == publication_id)
                .ok_or_else(|| anyhow!("Unknown publication {publication_id}"))?;
            if p.rollback.is_some() {
                bail!("This publication was already rolled back");
            }
            let dest = fs::canonicalize(&p.destination)
                .with_context(|| format!("Destination {} is gone", p.destination))?;
            let (removed, kept) = remove_introduced(&dest, &p.introduced)?;
            let value = call(
                client,
                actor,
                &request_id,
                Command::RecordPublicationRollback {
                    publication_id,
                    removed,
                    kept_modified: kept,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        RolesAction::Evidence {
            publication_id,
            session_id,
            model,
            marker,
            output_file,
            harness_version,
            request_id,
        } => {
            let output = fs::read_to_string(&output_file)
                .with_context(|| format!("Read {}", output_file.display()))?;
            let value = call(
                client,
                actor,
                &request_id,
                Command::RecordActivationEvidence {
                    publication_id,
                    session_id,
                    model,
                    harness_version,
                    marker,
                    output,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn plan(paths: &[&str]) -> Vec<(String, String)> {
        paths
            .iter()
            .map(|p| (p.to_string(), format!("content of {p}")))
            .collect()
    }

    #[test]
    fn collisions_escapes_and_identical_files_are_classified() {
        let dir = TempDir::new().unwrap();
        let dest = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(dest.join(".claude/agents")).unwrap();
        fs::write(
            dest.join(".claude/agents/same.md"),
            "content of .claude/agents/same.md",
        )
        .unwrap();
        fs::write(dest.join(".claude/agents/other.md"), "someone else's file").unwrap();

        let ok = plan_destination(
            &dest,
            plan(&[".claude/agents/new.md", ".claude/agents/same.md"]),
        )
        .unwrap();
        assert_eq!(
            ok.iter().map(|p| p.state).collect::<Vec<_>>(),
            ["new", "identical"]
        );

        let err = plan_destination(
            &dest,
            plan(&[".claude/agents/new.md", ".claude/agents/other.md"]),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            err.contains("other.md (different content exists)")
                && err.contains("nothing was written")
        );
        assert!(plan_destination(&dest, plan(&["../outside.md"])).is_err());
        assert!(plan_destination(&dest, plan(&[".claude/../../x.md"])).is_err());
    }

    #[test]
    fn rollback_removes_only_matching_introduced_files() {
        let dir = TempDir::new().unwrap();
        let dest = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(dest.join(".claude/skills/s")).unwrap();
        fs::create_dir_all(dest.join(".claude/agents")).unwrap();
        fs::write(dest.join(".claude/settings.json"), "{}").unwrap();
        fs::write(dest.join(".claude/skills/s/SKILL.md"), "skill").unwrap();
        fs::write(dest.join(".claude/agents/r.md"), "edited by the user").unwrap();
        let files = [
            PublishedFile {
                path: ".claude/skills/s/SKILL.md".into(),
                sha256: content_sha256("skill"),
            },
            PublishedFile {
                path: ".claude/agents/r.md".into(),
                sha256: content_sha256("agent"),
            },
            PublishedFile {
                path: ".claude/agents/gone.md".into(),
                sha256: content_sha256("x"),
            },
        ];
        let (removed, kept) = remove_introduced(&dest, &files).unwrap();
        assert_eq!(
            removed,
            [".claude/skills/s/SKILL.md", ".claude/agents/gone.md"]
        );
        assert_eq!(kept, [".claude/agents/r.md"]);
        assert!(
            !dest.join(".claude/skills").exists(),
            "emptied directories go"
        );
        assert!(
            dest.join(".claude/settings.json").exists(),
            "unrelated files stay"
        );
        assert!(
            dest.join(".claude/agents/r.md").exists(),
            "modified files stay"
        );
    }

    #[test]
    fn verbatim_windows_paths_are_recorded_plainly() {
        assert_eq!(plain(Path::new(r"\\?\C:\work\demo")), r"C:\work\demo");
        assert_eq!(
            plain(Path::new(r"\\?\UNC\server\share")),
            r"\\?\UNC\server\share"
        );
        assert_eq!(plain(Path::new("/tmp/demo")), "/tmp/demo");
    }

    #[test]
    fn global_and_root_destinations_are_refused() {
        if let Some(home) = home() {
            assert!(check_destination(&home)
                .unwrap_err()
                .to_string()
                .contains("install globally"));
            let global = home.join(".claude");
            if global.is_dir() {
                assert!(check_destination(&global)
                    .unwrap_err()
                    .to_string()
                    .contains("global harness"));
            }
        }
        let root = Path::new(if cfg!(windows) { "C:\\" } else { "/" });
        assert!(check_destination(root).is_err());
        let dir = TempDir::new().unwrap();
        assert!(check_destination(&dir.path().join("missing")).is_err());
        assert!(check_destination(dir.path()).is_ok());
    }
}
