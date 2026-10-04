pub mod drafting;
pub mod server;
use anyhow::{bail, Context, Result};
use direct_core::{Request, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub agent_token: String,
    pub owner_token: String,
}
pub fn data_dir() -> Result<PathBuf> {
    resolve_data_dir(cfg!(windows), |key| std::env::var_os(key))
}

fn resolve_data_dir(
    windows: bool,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    let value = |key| env(key).filter(|v| !v.is_empty());
    if let Some(v) = value("DIRECT_DATA_DIR") {
        return Ok(PathBuf::from(v));
    }
    if windows {
        // MSIX callers can see a redirected LOCALAPPDATA. USERPROFILE is shared
        // by the desktop launched from Explorer and agents launched by Codex.
        let dir = PathBuf::from(value("USERPROFILE").context(
            "USERPROFILE is missing; set DIRECT_DATA_DIR to an explicit workspace path",
        )?)
        .join(".direct/data");
        if !dir.join("direct.db").exists()
            && value("LOCALAPPDATA")
                .is_some_and(|v| PathBuf::from(v).join("Direct/direct.db").exists())
        {
            bail!("An older Direct workspace exists in LocalAppData. Export and restore it into {} before launching (see docs/desktop-workspace.md). No empty workspace was created.", dir.display());
        }
        return Ok(dir);
    }
    if let Some(v) = value("XDG_DATA_HOME") {
        return Ok(PathBuf::from(v).join("direct"));
    }
    Ok(
        PathBuf::from(value("HOME").context("HOME is missing; set DIRECT_DATA_DIR")?)
            .join(".local/share/direct"),
    )
}
/// Write a validated pre-migration backup (archive plus `.sha256`) beneath the
/// data directory and return the archive path. Files are created new and
/// published by rename, so a partial backup is never mistaken for a complete one.
pub fn write_migration_backup(dir: &Path, archive: &direct_core::Archive) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    direct_core::validate_archive(archive)?;
    let backups = dir.join("migration-backups");
    protect_dir(&backups)?;
    let stem = format!(
        "pre-migration-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        uuid::Uuid::new_v4().simple()
    );
    let mut bytes = serde_json::to_vec_pretty(archive)?;
    bytes.push(b'\n');
    let checksum = format!("{:x}", Sha256::digest(&bytes));
    let final_path = backups.join(format!("{stem}.json"));
    for (path, contents) in [
        (final_path.clone(), bytes),
        (
            backups.join(format!("{stem}.sha256")),
            format!("{checksum}  {stem}.json\n").into_bytes(),
        ),
    ] {
        let temporary = backups.join(format!(
            ".{}.partial",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&contents)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
    }
    Ok(final_path)
}
pub fn endpoint(dir: &Path) -> Result<Endpoint> {
    let e: Endpoint = serde_json::from_slice(
        &fs::read(dir.join("endpoint.json"))
            .context("Direct is not running. Start `direct serve` first.")?,
    )?;
    if e.port == 0 {
        bail!("Invalid local service port");
    }
    Ok(e)
}
/// Whether a failed command may still have been applied by the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteOutcome {
    /// The service answered with a definitive refusal; nothing was applied.
    Rejected,
    /// No usable answer (unreachable, dropped, timed out or failed while
    /// handling it): the command may or may not have been applied. Retrying
    /// the exact request with the same request ID is safe and authoritative.
    Unknown,
}
/// An error reported by the service itself, with the outcome it implies.
#[derive(Debug, Clone)]
pub struct ServiceError {
    pub code: String,
    pub message: String,
    pub outcome: WriteOutcome,
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ServiceError {}
/// The structured failure the desktop shell hands to the interface for a
/// `Client::call` error: `{code, message, outcome}`, where `outcome` tells
/// the interface whether keeping the request ID for a retry matters.
pub fn command_failure(error: &anyhow::Error) -> Value {
    match error.downcast_ref::<ServiceError>() {
        Some(e) => serde_json::json!({"code": e.code, "message": e.message, "outcome": e.outcome}),
        None => serde_json::json!({
            "code": "unavailable",
            "message": error.to_string(),
            "outcome": WriteOutcome::Unknown,
        }),
    }
}
pub struct Client {
    pub endpoint: Endpoint,
    http: reqwest::blocking::Client,
}
impl Client {
    pub fn draft_brief(&self, input: &drafting::DraftRequest) -> Result<Value> {
        let response = self
            .http
            .post(format!(
                "http://127.0.0.1:{}/api/draft-brief",
                self.endpoint.port
            ))
            .bearer_auth(self.token(Role::Human))
            .json(input)
            .timeout(Duration::from_secs(drafting::DRAFT_TIMEOUT_SECONDS + 10))
            .send()?;
        let status = response.status();
        let value: Value = response.json()?;
        if !status.is_success() {
            anyhow::bail!(
                "{}",
                value["message"]
                    .as_str()
                    .unwrap_or("AI drafting failed; your input is unchanged")
            );
        }
        Ok(value)
    }
    pub fn new(dir: &Path) -> Result<Self> {
        Ok(Self {
            endpoint: endpoint(dir)?,
            http: reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()?,
        })
    }
    pub fn call(&self, request: &Request, role: Role) -> Result<Value> {
        let token = if role == Role::Human {
            &self.endpoint.owner_token
        } else {
            &self.endpoint.agent_token
        };
        let response = self
            .http
            .post(format!(
                "http://127.0.0.1:{}/api/command",
                self.endpoint.port
            ))
            .bearer_auth(token)
            .json(request)
            .timeout(if matches!(request.command, direct_core::Command::Export) {
                // Exports carry retained source bytes and can be large.
                Duration::from_secs(900)
            } else {
                Duration::from_secs(10)
            })
            .send()
            .context("Direct service is unavailable. Restart it and retry the same request ID.")?;
        let status = response.status();
        let body = response.json::<Value>();
        if status.is_success() {
            return body.context(
                "Direct did not return a complete reply; the command may have been applied. Retry the same request ID.",
            );
        }
        let value = body.unwrap_or_default();
        Err(ServiceError {
            code: value["code"].as_str().unwrap_or("error").to_string(),
            message: value["message"]
                .as_str()
                .unwrap_or("Request failed")
                .to_string(),
            // A server-side failure can happen after the decision to commit;
            // only a client-error status is a definitive refusal.
            outcome: if status.is_server_error() {
                WriteOutcome::Unknown
            } else {
                WriteOutcome::Rejected
            },
        }
        .into())
    }
    fn token(&self, role: Role) -> &str {
        if role == Role::Human {
            &self.endpoint.owner_token
        } else {
            &self.endpoint.agent_token
        }
    }
    /// Retained source file bytes (read-only, any role).
    pub fn source_file(&self, bundle_id: &str, path: &str, role: Role) -> Result<Vec<u8>> {
        let response = self
            .http
            .post(format!(
                "http://127.0.0.1:{}/api/source-file",
                self.endpoint.port
            ))
            .bearer_auth(self.token(role))
            .json(&serde_json::json!({"bundle_id": bundle_id, "path": path}))
            .timeout(Duration::from_secs(600))
            .send()
            .context("Direct service is unavailable")?;
        let status = response.status();
        if !status.is_success() {
            let value: Value = response.json().unwrap_or_default();
            bail!(
                "{}: {}",
                value["code"].as_str().unwrap_or("error"),
                value["message"].as_str().unwrap_or("Request failed")
            );
        }
        Ok(response.bytes()?.to_vec())
    }
    /// Owner-only migration preview (`expected` = None) or apply.
    pub fn migration(
        &self,
        artifact: Vec<u8>,
        expected: Option<(u64, &str)>,
        role: Role,
    ) -> Result<Value> {
        let url = match expected {
            None => format!(
                "http://127.0.0.1:{}/api/migration/preview",
                self.endpoint.port
            ),
            Some((cursor, sha)) => format!(
                "http://127.0.0.1:{}/api/migration/apply?expected_cursor={cursor}&artifact_sha256={sha}",
                self.endpoint.port
            ),
        };
        let response = self
            .http
            .post(url)
            .bearer_auth(self.token(role))
            .header("content-type", "application/octet-stream")
            .body(artifact)
            .timeout(Duration::from_secs(1800))
            .send()
            .context("Direct service is unavailable")?;
        let status = response.status();
        let value: Value = response.json()?;
        if !status.is_success() {
            bail!(
                "{}: {}",
                value["code"].as_str().unwrap_or("error"),
                value["message"].as_str().unwrap_or("Request failed")
            );
        }
        Ok(value)
    }
    pub fn launch_url(&self) -> Result<String> {
        let response = self
            .http
            .post(format!(
                "http://127.0.0.1:{}/api/launch",
                self.endpoint.port
            ))
            .bearer_auth(&self.endpoint.owner_token)
            .send()?
            .error_for_status()?;
        let value: Value = response.json()?;
        Ok(value["url"].as_str().context("Missing launch URL")?.into())
    }
    /// Authenticated liveness of the service on record. The service answers
    /// without touching the store, so a busy store (a long export or
    /// migration) still reads as alive; ordinary commands keep waiting for it.
    pub fn liveness(&self, timeout: Duration) -> Liveness {
        let Ok(response) = self
            .http
            .post(format!(
                "http://127.0.0.1:{}/api/health",
                self.endpoint.port
            ))
            .bearer_auth(&self.endpoint.agent_token)
            .timeout(timeout)
            .send()
        else {
            return Liveness::Unreachable;
        };
        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return Liveness::Refused;
        }
        match response.json::<Value>() {
            Ok(value) if status.is_success() && value["service"] == "direct" => Liveness::Alive,
            _ => Liveness::Unreachable,
        }
    }
    pub fn healthy(&self) -> bool {
        self.liveness(PROBE_TIMEOUT) == Liveness::Alive
    }
}
/// What one liveness probe learned about the endpoint on record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// The Direct service holding these credentials answered.
    Alive,
    /// Something on the recorded port refused these credentials.
    Refused,
    /// No usable answer in time: nothing listening, a dropped connection,
    /// a timeout or a reply that is not from Direct.
    Unreachable,
}
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const MIN_PROBE: Duration = Duration::from_millis(250);
/// Startup wait budget, plus the initial probe and at most 250 ms for a final probe.
pub const STARTUP_DEADLINE: Duration = Duration::from_secs(15);
fn probe(dir: &Path, timeout: Duration) -> Liveness {
    Client::new(dir).map_or(Liveness::Unreachable, |c| c.liveness(timeout))
}
/// Whether a `direct serve` process holds this data directory's service lock.
/// The lock is held for the service's whole life, so this tells a busy or
/// starting service from a dead one without asking the service anything.
pub fn service_running(dir: &Path) -> Result<bool> {
    let file = match fs::File::open(dir.join("service.lock")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        }
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

pub fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

/// Keep the service outside the desktop's lifecycle: closing the UI must not stop agents.
pub fn ensure_service(dir: &Path) -> Result<()> {
    if probe(dir, PROBE_TIMEOUT) == Liveness::Alive {
        return Ok(());
    }
    let exe = std::env::current_exe()?
        .parent()
        .context("Missing binary directory")?
        .join(if cfg!(windows) {
            "direct.exe"
        } else {
            "direct"
        });
    let packaged = exe.parent().unwrap().join("web");
    let assets = if packaged.exists() {
        packaged
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/dist")
    };
    // Dropping the handle leaves the service running for agents.
    start_service(dir, &exe, &assets, STARTUP_DEADLINE).map(drop)
}
/// Wait for this data directory's service, starting `exe` only while no
/// process holds the service lock, and give up after `deadline` in total.
/// A running service that is busy, still starting or answering with other
/// credentials is never duplicated. Returns the process it started, if any.
pub fn start_service(
    dir: &Path,
    exe: &Path,
    assets: &Path,
    deadline: Duration,
) -> Result<Option<std::process::Child>> {
    let end = std::time::Instant::now() + deadline;
    let mut child: Option<std::process::Child> = None;
    loop {
        let remaining = end.saturating_duration_since(std::time::Instant::now());
        // The last probe still gets long enough to be answered, so the
        // reported reason is real; it can overrun the deadline by this much.
        let state = probe(dir, PROBE_TIMEOUT.min(remaining).max(MIN_PROBE));
        if state == Liveness::Alive {
            return Ok(child);
        }
        let running = service_running(dir)?;
        if !running {
            match child.as_mut() {
                None => {
                    if !exe.exists() {
                        bail!("Build the service first: cargo build -p direct");
                    }
                    child = Some(
                        hidden(
                            Command::new(exe)
                                .arg("--data-dir")
                                .arg(dir)
                                .arg("serve")
                                .arg("--assets")
                                .arg(assets),
                        )
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()?,
                    );
                }
                Some(started) => {
                    if let Some(status) = started.try_wait()? {
                        bail!("The Direct service exited during startup ({status}). Run `direct serve` to see diagnostics.");
                    }
                }
            }
        }
        if std::time::Instant::now() >= end {
            let seconds = deadline.as_secs_f32();
            if running && state == Liveness::Refused {
                bail!("A Direct service holds this data directory but refused the credentials in its endpoint record, so no second service was started. Stop the running `direct serve`, then open Direct again.");
            }
            if running {
                bail!("A Direct service holds this data directory but did not answer within {seconds:.0} seconds. It may still be starting; retry shortly.");
            }
            bail!("The Direct service did not start within {seconds:.0} seconds. Run `direct serve` to see diagnostics.");
        }
        std::thread::sleep(Duration::from_millis(100).min(remaining));
    }
}

pub fn protect_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        let who =
            hidden(Command::new("whoami.exe").args(["/user", "/fo", "csv", "/nh"])).output()?;
        let output = String::from_utf8(who.stdout)?;
        let sid = output
            .trim()
            .split(',')
            .nth(1)
            .context("Cannot determine current user SID")?
            .trim_matches('"');
        if !sid.starts_with("S-1-") {
            bail!("Invalid user SID");
        }
        let result = hidden(
            Command::new("icacls.exe")
                .arg(dir)
                .arg("/inheritance:r")
                .arg("/grant:r")
                .arg(format!("*{sid}:(OI)(CI)F"))
                .arg("/grant:r")
                .arg("*S-1-5-18:(OI)(CI)F"),
        )
        .output()?;
        if !result.status.success() {
            bail!("Could not restrict Direct data-directory permissions");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_launchers_share_a_workspace_despite_redirected_appdata() {
        let profile = tempfile::tempdir().unwrap();
        let resolve = |local: &str| {
            resolve_data_dir(true, |key| match key {
                "USERPROFILE" => Some(profile.path().as_os_str().to_owned()),
                "LOCALAPPDATA" => Some(profile.path().join(local).into_os_string()),
                _ => None,
            })
            .unwrap()
        };
        assert_eq!(
            resolve("AppData/Local"),
            resolve("Packages/Codex/LocalCache/Local")
        );
        assert_eq!(
            resolve("AppData/Local"),
            profile.path().join(".direct/data")
        );
    }

    #[test]
    fn legacy_workspace_blocks_silent_empty_replacement_but_allows_migration() {
        let profile = tempfile::tempdir().unwrap();
        let local = profile.path().join("AppData/Local");
        let legacy = local.join("Direct");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("direct.db"), b"legacy sentinel").unwrap();
        let env = |key: &str| match key {
            "USERPROFILE" => Some(profile.path().as_os_str().to_owned()),
            "LOCALAPPDATA" => Some(local.as_os_str().to_owned()),
            _ => None,
        };
        assert!(resolve_data_dir(true, env)
            .unwrap_err()
            .to_string()
            .contains("No empty workspace"));
        let canonical = profile.path().join(".direct/data");
        assert!(!canonical.exists());
        assert_eq!(
            resolve_data_dir(true, |key| {
                if key == "DIRECT_DATA_DIR" {
                    Some(legacy.as_os_str().to_owned())
                } else {
                    env(key)
                }
            })
            .unwrap(),
            legacy
        );
        fs::create_dir_all(&canonical).unwrap();
        fs::write(canonical.join("direct.db"), b"restored sentinel").unwrap();
        assert_eq!(resolve_data_dir(true, env).unwrap(), canonical);
        assert_eq!(
            fs::read(legacy.join("direct.db")).unwrap(),
            b"legacy sentinel"
        );
    }

    #[test]
    fn explicit_workspaces_win_and_missing_profile_never_uses_working_directory() {
        assert_eq!(
            resolve_data_dir(true, |key| {
                (key == "DIRECT_DATA_DIR").then(|| "dedicated-workspace".into())
            })
            .unwrap(),
            PathBuf::from("dedicated-workspace")
        );
        assert!(resolve_data_dir(true, |_| None).is_err());
        assert!(resolve_data_dir(true, |_| Some("".into())).is_err());
        assert_eq!(
            resolve_data_dir(false, |key| {
                (key == "XDG_DATA_HOME").then(|| "/data".into())
            })
            .unwrap(),
            PathBuf::from("/data/direct")
        );
    }
}
