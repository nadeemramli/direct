# Scoped remote Direct reads

`direct-remote-read` is a separate read-only bridge over the running Direct service. It binds only to `127.0.0.1`. Publish this bridge through an HTTPS tunnel; never publish Direct's normal service. The bridge has three HTTP MCP tools at `/mcp` and two read endpoints: `GET /v1/manifest` and `GET /v1/context/{key}`. It has no mutation, list-all, archive, snapshot, raw command or owner endpoint.

The coordinator creates one access policy per cloud environment/session: one assigned issue plus at most seven explicit reference issues from the same product. References supply context, not new implementation objectives. Readiness, active claims, worker caps and owner acceptance still apply. Each policy expires within 24 hours. Its file contains only a SHA256 digest of a random 256-bit bearer credential. Revoking, removing or changing the policy invalidates access immediately without restarting Direct. A changed scope or credential requires a new bridge process. This implementation serves one policy per process; use distinct ports and credentials for concurrent sessions. Environment credentials apply to every session in that environment, so use a dedicated environment for each assignment and revoke before reusing it.

## Local setup

Build `cargo build -p direct-mcp --bin direct-remote-read`. In PowerShell 7, create access outside Git:

```powershell
./scripts/remote-read-access.ps1 -Action New -Session tile3-read-trial -Assigned TILE-3 -References TILE-1,TILE-2,TILE-4 -Hours 4
./target/debug/direct-remote-read.exe --data-dir "$env:USERPROFILE/.direct/data" --access-file "$env:USERPROFILE/.direct/remote-read/tile3-read-trial/policy.json" --actor direct-remote-tile3-read-trial --port 50771
```

The provisioning script protects the directory for the current Windows user and SYSTEM before writing the credential. The separate `client-secret.json` file is only for secure credential entry. Never paste it into prompts, commit it, include it in URLs, or print it in logs. Do not change the normal Direct data directory or its ACL.

Configure the HTTPS tunnel's origin as `http://127.0.0.1:50771` and its HTTP Host header as `127.0.0.1`. The bridge rejects other Host headers, browser Origin headers and URL queries. An outbound Cloudflare Tunnel is one supported option. A Quick Tunnel is only a temporary development trial: its hostname changes on restart and it has no uptime guarantee. Use a named tunnel with a stable hostname for ongoing use. Stop the bridge or revoke its policy when access is no longer needed.

```powershell
./scripts/remote-read-access.ps1 -Action Revoke -Session tile3-read-trial
./scripts/remote-read-access.ps1 -Action Inspect -Session tile3-read-trial
```

## Claude cloud setup and recovery

On Claude Pro/Max, create a dedicated cloud environment with an API credential for exactly the bridge hostname, header `Authorization`, prefix `Bearer`. The owner enters the token from the protected file into that credential form. Claude's agent proxy injects it after requests leave the VM; the model and shell do not receive the secret. Credential hosts are reachable independently of the environment's ordinary network allowlist. Keep network access at Trusted; no wildcard host or Full access is needed. See [Claude cloud environments](https://code.claude.com/docs/en/cloud-environments#add-api-credentials). Setup scripts do not receive proxy credentials; make the read after Claude starts.

For a real cloud trial, start a genuinely new chat, explicitly select and verify the actual authorized model, and attach the repository containing this client (or supply its non-secret source). Run:

```sh
python3 scripts/direct-remote-read.py --url https://YOUR-EXACT-BRIDGE-HOST --proxy-credential
```

For clients without credential injection, configure `DIRECT_REMOTE_READ_TOKEN` using their supported secret mechanism and omit `--proxy-credential`. The Python client follows no redirects, bounds responses, checks content fingerprints, and rereads the manifest after all context. A changed version produces a retrieval failure before a receipt is printed. The MCP equivalent is `direct_remote_manifest`, each `direct_remote_context`, then `direct_remote_receipt` for each key/version/fingerprint. No receipt creates a claim or records acceptance.

Dispatch must supply the credential-free HTTPS origin, assigned key, approved reference keys, source/build/claim pointers, and approved guidance packet. Require the first response to confirm manifest keys, current versions and fingerprints, and summarize the assigned acceptance criteria received. Do not say briefs are "supplied above" unless a verified packet or read receipt exists. On 401, repair expired/revoked credentials; on 403, repair scope; on 503, repair service/source availability. The coordinator verifies the problem against live Direct before escalating to the owner. Reread before mapping AC or continuing after a context update. Do not infer missing criteria.

## Boundary and limits

Each read calls the existing typed agent `Context` command, then positively selects key, version, status, title, brief and acceptance. Approved issue links require both ends in the policy. Guidance pins expose bounded document IDs and recorded fingerprints only; document contents and playbook version must arrive in the separately approved guidance packet. Owner identity, comments, intake, claim capabilities, history and raw records are withheld. Local paths are replaced with an explicit omission marker. A brief containing a local service capability or common credential pattern is withheld. Review authored briefs for portable scope before granting access; text can contain private information that no generic pattern can reliably identify.

The bridge uses constant-time digest comparison, revalidates policy before and after reads, serves no browser CORS credentials, sets `Cache-Control: no-store`, and never logs authorization headers or source bodies. It limits policy files to 8 KiB, MCP bodies to 16 KiB, projected issues to 64 KiB, concurrency to four, authenticated requests to 120 per minute, and request processing to 30 seconds. Rate limits are per bridge process. Fingerprints cover projected content and issue version; observation time is excluded. A manifest is individual live reads, not an atomic snapshot; the client's final manifest check rejects observed drift.

Isolated fixture tests exercise real HTTP/MCP calls against an isolated Direct service, positive projection, wrong/missing credentials, scope and mutation denial, unknown tool parameters, version changes and stale receipts, oversized bodies, rate/concurrency limits, immediate revocation, bridge restart and unavailable service/config. They do not replace actual cloud reachability and installed-build verification. Keep the delivery issue Doing until the real cloud first pass and required regression checks pass.
