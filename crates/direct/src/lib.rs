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
pub fn data_dir() -> PathBuf {
    if let Some(v) = std::env::var_os("DIRECT_DATA_DIR") {
        return PathBuf::from(v);
    }
    if let Some(v) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(v).join("Direct");
    }
    if let Some(v) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(v).join("direct");
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
        .join(".local/share/direct")
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
