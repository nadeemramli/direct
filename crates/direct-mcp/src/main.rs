use anyhow::{Context, Result};
use clap::Parser;
use direct_mcp::DirectMcp;
use rmcp::ServiceExt;
use std::path::PathBuf;

/// Local stdio MCP server that exposes selected Direct agent operations.
///
/// It talks to the already running local Direct service and never opens the
/// database, a network listener, or owner-only operations. Standard output is
/// the protocol channel, so diagnostics go to standard error.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Data directory of the running Direct service (contains endpoint.json).
    /// Defaults to the same workspace resolution as the direct CLI.
    #[arg(long, env = "DIRECT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Distinct agent identity for every write. Keep one actor from claim through submission.
    #[arg(long, env = "DIRECT_MCP_ACTOR")]
    actor: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("direct-mcp: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let dir = match args.data_dir.filter(|path| !path.as_os_str().is_empty()) {
        Some(dir) => dir,
        None => direct::data_dir()?,
    };
    let server = DirectMcp::new(&dir, &args.actor)?;
    tokio::runtime::Runtime::new()?.block_on(async {
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .context("MCP initialization over stdio failed")?;
        running.waiting().await?;
        Ok(())
    })
}
