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
pub struct Client {
    pub endpoint: Endpoint,
    http: reqwest::blocking::Client,
}
impl Client {
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
    pub fn healthy(&self) -> bool {
        self.call(
            &Request {
                actor: "desktop".into(),
                request_id: String::new(),
                command: direct_core::Command::Changes { after: 0 },
            },
            Role::Agent,
        )
        .is_ok()
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
    if Client::new(dir).is_ok_and(|c| c.healthy()) {
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
    if !exe.exists() {
        bail!("Build the service first: cargo build -p direct");
    }
    let packaged = exe.parent().unwrap().join("web");
    let assets = if packaged.exists() {
        packaged
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/dist")
    };
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
    .spawn()?;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        if Client::new(dir).is_ok_and(|c| c.healthy()) {
            return Ok(());
        }
    }
    bail!("Service did not start. Run `direct serve` to see diagnostics.")
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
