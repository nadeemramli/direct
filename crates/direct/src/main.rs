use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use direct_core::{Archive, Command, Request, Role, Store};
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
};

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
    Create {
        #[arg(long, default_value = "DIR")]
        product: String,
        title: String,
        #[arg(long, default_value = "")]
        body: String,
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
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    let dir = args.data_dir.unwrap_or_else(direct::data_dir);
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
                Cli::Create {
                    product,
                    title,
                    body,
                } => Request {
                    actor: args.actor,
                    request_id: uuid::Uuid::new_v4().to_string(),
                    command: Command::CreateIssue {
                        product,
                        title,
                        body,
                    },
                },
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
