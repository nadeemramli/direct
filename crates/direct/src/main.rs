use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use direct_core::{
    Archive, Command, GitTraceKind, PlanningScope, Request, Role, Store, TheoriaDocumentInput,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TheoriaCatalog {
    version: u32,
    documents: Vec<TheoriaCatalogEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TheoriaCatalogEntry {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    category: String,
    path: PathBuf,
}

#[derive(Parser)]
#[command(version, about = "Direct — local work tracking for humans and agents")]
struct Args {
    #[arg(long, global = true, env = "DIRECT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true, default_value = "local-agent")]
    actor: String,
    #[command(subcommand)]
    command: Cli,
}
#[derive(Subcommand)]
enum Cli {
    /// Run the single-writer service. Closing the UI does not stop this process.
    Serve {
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long, default_value = "app/dist")]
        assets: PathBuf,
    },
    List,
    Context {
        key: String,
    },
    /// Import the selected authoritative Markdown sources as a read-only Theoria cache.
    TheoriaSync {
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value = "theoria/catalog.json")]
        catalog: PathBuf,
        #[arg(long, default_value = "DIR")]
        product: String,
        #[arg(long)]
        request_id: String,
    },
    /// Link an already-created Git commit to an actively claimed Direct issue.
    RecordCommit {
        #[arg(long)]
        issue: String,
        #[arg(long)]
        expected_version: u64,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        commit_sha: String,
        #[arg(long)]
        branch: String,
        #[arg(long)]
        request_id: String,
    },
    /// Record a push only after the underlying Git push has completed successfully.
    RecordPush {
        #[arg(long)]
        issue: String,
        #[arg(long)]
        expected_version: u64,
        #[arg(long)]
        repository: String,
        #[arg(long)]
        commit_sha: String,
        #[arg(long)]
        branch: String,
        #[arg(long)]
        remote: String,
        #[arg(long)]
        remote_ref: String,
        #[arg(long)]
        request_id: String,
    },
    Create {
        #[arg(long, default_value = "DIR")]
        product: String,
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long, default_value = "inbox")]
        planning_scope: String,
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Execute a JSON agent command from a file or stdin. Human approval is unavailable here.
    Call {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Print a one-use local owner-interface launch link. Do not share this link.
    Open,
    /// Write a versioned JSON archive. Refuses to overwrite an existing file.
    Export {
        out: PathBuf,
    },
    /// Offline restore into a NEW --data-dir. Existing directories are never replaced.
    Restore {
        from: PathBuf,
    },
}

fn frontmatter_value(content: &str, key: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches(['\'', '"']).to_string());
        }
    }
    None
}

fn load_theoria(root: &Path, catalog_path: &Path) -> Result<(u32, Vec<TheoriaDocumentInput>)> {
    if !root.is_absolute() {
        bail!("Theoria source root must be an absolute path");
    }
    let catalog: TheoriaCatalog = serde_json::from_slice(
        &fs::read(catalog_path).context("Read the explicit Theoria catalog")?,
    )?;
    if catalog.version == 0 || catalog.documents.is_empty() {
        bail!("Theoria catalog needs a positive version and at least one document");
    }
    let mut documents = Vec::with_capacity(catalog.documents.len());
    for entry in catalog.documents {
        if entry.path.is_absolute()
            || entry.path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("Theoria catalog paths must stay beneath the source root");
        }
        let source = root.join(&entry.path);
        match fs::read_to_string(&source) {
            Ok(content) => {
                if content.len() > 400_000 {
                    bail!("Theoria source {} exceeds the 400 KB cache limit", entry.id);
                }
                let source_modified_at = fs::metadata(&source)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs() as i64);
                documents.push(TheoriaDocumentInput {
                    id: entry.id,
                    title: entry.title,
                    description: entry.description,
                    category: entry.category,
                    relative_path: entry.path.to_string_lossy().replace('\\', "/"),
                    source_updated: frontmatter_value(&content, "updated"),
                    source_modified_at,
                    fingerprint: Some(format!("{:x}", Sha256::digest(content.as_bytes()))),
                    content: Some(content),
                    unavailable_reason: None,
                });
            }
            Err(error) => documents.push(TheoriaDocumentInput {
                id: entry.id,
                title: entry.title,
                description: entry.description,
                category: entry.category,
                relative_path: entry.path.to_string_lossy().replace('\\', "/"),
                source_updated: None,
                source_modified_at: None,
                fingerprint: None,
                content: None,
                unavailable_reason: Some(if error.kind() == io::ErrorKind::NotFound {
                    "source not found".into()
                } else {
                    "source unavailable".into()
                }),
            }),
        }
    }
    Ok((catalog.version, documents))
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    let dir = match args.data_dir.filter(|path| !path.as_os_str().is_empty()) {
        Some(dir) => dir,
        None => direct::data_dir()?,
    };
    match args.command {
        Cli::Serve { port, assets } => {
            tokio::runtime::Runtime::new()?.block_on(direct::server::serve(&dir, port, &assets))
        }
        Cli::Restore { from } => {
            if dir.exists() {
                bail!("Restore requires a new, nonexistent data directory; existing data was not changed");
            }
            let a: Archive = serde_json::from_slice(&fs::read(from)?)?;
            direct_core::validate_archive(&a)?;
            fs::create_dir(&dir)
                .context("Create a new restore destination beneath an existing parent")?;
            direct::protect_dir(&dir)?;
            let mut store = Store::open(&dir.join("direct.db"))?;
            store.restore(a)?;
            println!(
                "Restored to {}. Start direct serve with that data directory.",
                dir.display()
            );
            Ok(())
        }
        other => {
            let client = direct::Client::new(&dir)?;
            if matches!(other, Cli::Open) {
                println!("{}", client.launch_url()?);
                return Ok(());
            }
            let mut output = None;
            let request = match other {
                Cli::List => Request {
                    actor: args.actor,
                    request_id: String::new(),
                    command: Command::Snapshot,
                },
                Cli::Context { key } => Request {
                    actor: args.actor,
                    request_id: String::new(),
                    command: Command::Context { key },
                },
                Cli::TheoriaSync {
                    root,
                    catalog,
                    product,
                    request_id,
                } => {
                    let (catalog_version, documents) = load_theoria(&root, &catalog)?;
                    Request {
                        actor: args.actor,
                        request_id,
                        command: Command::SyncTheoria {
                            product,
                            source_root: root.to_string_lossy().into_owned(),
                            catalog_version,
                            documents,
                        },
                    }
                }
                Cli::RecordCommit {
                    issue,
                    expected_version,
                    repository,
                    commit_sha,
                    branch,
                    request_id,
                } => Request {
                    actor: args.actor,
                    request_id,
                    command: Command::RecordGitTrace {
                        key: issue,
                        expected_version,
                        kind: GitTraceKind::Commit,
                        repository,
                        commit_sha,
                        branch,
                        remote: None,
                        remote_ref: None,
                    },
                },
                Cli::RecordPush {
                    issue,
                    expected_version,
                    repository,
                    commit_sha,
                    branch,
                    remote,
                    remote_ref,
                    request_id,
                } => Request {
                    actor: args.actor,
                    request_id,
                    command: Command::RecordGitTrace {
                        key: issue,
                        expected_version,
                        kind: GitTraceKind::Push,
                        repository,
                        commit_sha,
                        branch,
                        remote: Some(remote),
                        remote_ref: Some(remote_ref),
                    },
                },
                Cli::Create {
                    product,
                    title,
                    body,
                    planning_scope,
                    project_id,
                } => {
                    let planning_scope = match planning_scope.as_str() {
                        "project" => PlanningScope::Project,
                        "inbox" => PlanningScope::Inbox,
                        _ => bail!("planning scope must be 'project' or 'inbox'"),
                    };
                    Request {
                        actor: args.actor,
                        request_id: uuid::Uuid::new_v4().to_string(),
                        command: Command::CreateIssue {
                            product,
                            title,
                            body,
                            planning_scope,
                            project_id,
                        },
                    }
                }
                Cli::Call { file } => {
                    let text = if let Some(f) = file {
                        fs::read_to_string(f)?
                    } else {
                        let mut text = String::new();
                        io::stdin().read_to_string(&mut text)?;
                        text
                    };
                    serde_json::from_str::<Request>(&text)?
                }
                Cli::Export { out } => {
                    output = Some(out);
                    Request {
                        actor: args.actor,
                        request_id: String::new(),
                        command: Command::Export,
                    }
                }
                _ => unreachable!(),
            };
            let value = client.call(&request, Role::Agent)?;
            if let Some(out) = output {
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&out)
                    .context("Export destination must not already exist")?;
                f.write_all(serde_json::to_string_pretty(&value)?.as_bytes())?;
                f.sync_all()?;
                println!("Exported to {}", out.display());
            } else {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn catalog(path: &str) -> String {
        format!(
            r#"{{"version":2,"documents":[{{"id":"dos-guide","title":"Guide","description":"Navigation","category":"workflow","path":{path:?}}}]}}"#
        )
    }

    #[test]
    fn importer_fingerprints_catalogued_markdown_and_reads_updated_frontmatter() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("source");
        fs::create_dir(&root).unwrap();
        let content = "---\nupdated: 2026-09-29\n---\n# Guide\n";
        fs::write(root.join("Guide.md"), content).unwrap();
        let catalog_path = dir.path().join("catalog.json");
        fs::write(&catalog_path, catalog("Guide.md")).unwrap();

        let (version, documents) = load_theoria(&root, &catalog_path).unwrap();
        assert_eq!(version, 2);
        assert_eq!(documents[0].source_updated.as_deref(), Some("2026-09-29"));
        assert_eq!(documents[0].content.as_deref(), Some(content));
        assert_eq!(
            documents[0].fingerprint.as_deref(),
            Some(format!("{:x}", Sha256::digest(content.as_bytes())).as_str())
        );
        assert!(documents[0].unavailable_reason.is_none());
    }

    #[test]
    fn importer_preserves_unavailable_state_and_rejects_escaping_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("source");
        fs::create_dir(&root).unwrap();
        let catalog_path = dir.path().join("catalog.json");
        fs::write(&catalog_path, catalog("Missing.md")).unwrap();

        let (_, documents) = load_theoria(&root, &catalog_path).unwrap();
        assert_eq!(
            documents[0].unavailable_reason.as_deref(),
            Some("source not found")
        );
        assert!(documents[0].content.is_none());
        assert!(documents[0].fingerprint.is_none());

        fs::write(&catalog_path, catalog("../outside.md")).unwrap();
        assert!(load_theoria(&root, &catalog_path)
            .unwrap_err()
            .to_string()
            .contains("stay beneath"));
    }
}
