use anyhow::Result;
use clap::Parser;
use direct_mcp::remote::{serve, RemoteReads};
use std::path::PathBuf;

/// Separate read-only bridge. Never tunnel the main Direct service.
#[derive(Parser)]
struct Args {
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    access_file: PathBuf,
    #[arg(long)]
    actor: String,
    #[arg(long, default_value_t = 0)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let reads = RemoteReads::new(args.data_dir, args.access_file, args.actor)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port)).await?;
    println!(
        "direct-remote-read listening on 127.0.0.1:{}",
        listener.local_addr()?.port()
    );
    serve(listener, reads).await
}
