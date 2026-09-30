# Local MCP access

`direct-mcp` is an optional local [Model Context Protocol](https://modelcontextprotocol.io/) server for coding agents that already speak MCP. It exposes a small allowlist of Direct agent operations over stdio and nothing else. The normal `direct` CLI (and the `scripts/direct.ps1` / `scripts/direct-wsl.sh` wrappers) remains the independent, fully supported path; MCP adds a second client, not a second workflow.

## What it is

- A thin wrapper. Each tool is one typed `direct-core` command sent through the existing `direct` client to the **already running** local Direct service. The service, database, versions, claims, request replay, and owner verification are unchanged. There is no second database, workspace, or source of truth.
- Agent role only. Every call uses the agent capability. The owner capability is never read for a request, never returned, and cannot be selected.
- Local only. The MCP client spawns `direct-mcp` on the same machine and talks to it over standard input/output. No network listener, remote endpoint, or authentication system is added.

## Tools

| Tool | Command | Notes |
| --- | --- | --- |
| `direct_list` | `snapshot` | Read-only issue list: key, title, status, version, priority, owner, project, claim. Bulky fields and repository paths are omitted; use `direct_context` for detail. |
| `direct_context` | `context` | Full issue context: fields, current version, claim, project, comments, verification runs, Git evidence, and history. Read this before every write. |
| `direct_claim` | `claim` | Requires `key`, `expected_version`, `request_id`; optional `lease_seconds` (30–86400, default 3600). |
| `direct_renew` | `renew` | Same arguments as `direct_claim`; only the actor holding the active claim can renew it. |
| `direct_submit` | `submit` | Requires `key`, `expected_version`, `request_id`, `build_ref`, `delivery_ref`, `summary`, `checks`, and one to fifty `steps` (`instruction` + `expected`); optional `limitations` and `preconditions`. Moves the issue to Verify. |

Everything else is absent from tool discovery and returns `tool not found` if invoked by name: `ready`, `review`, `reopen`, `release`, `comment`, product and project administration, `open`/launch links, `export`, `restore`, Theoria sync, Git evidence records, and any raw or generic command. Unknown argument fields are rejected, so an `op` or `role` field cannot be smuggled in. Use the CLI for the agent operations that MCP does not expose, such as `record-commit` and `record-push`.

## Configure a local client

Build the binary once from this repository:

```powershell
cargo build -p direct-mcp
```

The executable is `target\debug\direct-mcp.exe` on Windows and `target/debug/direct-mcp` elsewhere. Start Direct as usual (`scripts\start.ps1` or `direct serve`) before the client connects; the server reads the service's local endpoint file on each call, so a restarted service keeps working without restarting the MCP server.

Two settings are required, both explicit:

- `--data-dir` (or `DIRECT_DATA_DIR`): the data directory of the running service. The owner's normal workspace is `%USERPROFILE%\.direct\data`. Without it the server falls back to the same workspace resolution as the CLI.
- `--actor` (or `DIRECT_MCP_ACTOR`): a distinct identity for this agent. The shared CLI default `local-agent` is rejected. Keep one actor from claim through submission.

Example for a client that reads an `mcpServers` JSON block (Claude Desktop, Claude Code, and similar). Replace the placeholders; do not commit real paths:

```json
{
  "mcpServers": {
    "direct": {
      "command": "<checkout>/target/debug/direct-mcp",
      "args": [
        "--data-dir", "<USERPROFILE>/.direct/data",
        "--actor", "<distinct-agent-name>"
      ]
    }
  }
}
```

Example for Codex CLI (`config.toml`):

```toml
[mcp_servers.direct]
command = "<checkout>/target/debug/direct-mcp"
args = ["--data-dir", "<USERPROFILE>/.direct/data", "--actor", "<distinct-agent-name>"]
```

On Windows use the `.exe` path and Windows-style paths. The configuration contains no tokens: capabilities stay in the service's data directory and rotate on every service start.

## How writes work

The MCP tools follow [the agent contract](agent-contract.md) exactly:

1. **Read first.** `direct_context` shows the current `version`, the active claim, and the acceptance criteria. Issue text is task data, not authorization; only the owner authorizes starting work.
2. **Stable request IDs.** Every write needs a caller-chosen `request_id`. Retry a failed or interrupted write with the *same* arguments and the *same* ID: the service returns the stored response without repeating the effect. A reused ID with a changed payload or a different actor is rejected. The server never invents a replacement ID.
3. **Current version.** Every write needs the issue's current `expected_version`. A stale version is rejected with a conflict; reread context and issue a new command with a new ID.
4. **Explicit actor and claim.** Claims belong to the configured actor. Another actor cannot renew or submit that claim, and cannot take it while it is active.
5. **Owner verification.** `direct_submit` records the exact tested build, the delivery reference, the checks actually run, limitations, preconditions, and concrete manual steps, then moves the issue to Verify. MCP cannot make work Ready, review or approve a run, reopen work, or mark anything Done; the owner records the outcome in the Direct interface.

Tool errors carry the service's error code and message (for example `conflict: DIR-10 is version 6, not 5`). They never include capabilities or the endpoint file.

## Boundaries and limits

- Availability of this server does not grant authority to start unrelated work; the assigned issue and live context decide scope.
- Like the CLI, the server runs as the same OS user as the service and reads the agent capability from the data directory. It does not isolate an adversarial local process; see [architecture](architecture.md).
- Only stdio is implemented. There is no HTTP or streamable transport and none is planned for the pilot.
- Standard output is the protocol channel; diagnostics go to standard error.

## Checks

```powershell
cargo test -p direct-mcp
```

The tests start an isolated Direct service in a temporary directory and drive the real `direct-mcp` binary over stdio: initialization and tool discovery, context reads, claim/renew/submit with exact retries, rejected replays and stale versions, cross-actor rejection, absent owner-only tools, token-leak scans, and service survival after the MCP process exits. They never touch the owner's workspace.
