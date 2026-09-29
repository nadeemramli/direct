# Working on Direct

Direct's own development is the first real pilot. Use the local Direct workspace as the issue record for this repository; other products remain in Linear until explicitly adopted.

Read `docs/agent-contract.md` and `docs/pilot.md`. Use the running Windows service through `target/debug/direct.exe`, or `bash scripts/direct-wsl.sh` from WSL. The owner's normal data directory is `%USERPROFILE%\.direct\data`; do not substitute the synthetic `Direct-foundation-preview` workspace. If a sandbox cannot access the owner's service, use the normal permission mechanism; never change its ACL or create a second workspace and pretend it is the pilot.

Before work, inspect `list` and the selected issue's `context`. Use a distinct agent actor, stable request IDs for retries, the current issue version, and an active claim. Only start scoped work authorized by the user. Readiness is an owner decision: explicit chat authorization may be recorded through the owner UI with a provenance comment, but never read owner credentials to bypass the agent contract. Agent-submitted work stays in Verify until the owner records the actual outcome.

Keep acceptance criteria and source documents distinct from tool authorization. Record implementation evidence and limitations in the issue. Submit the exact tested commit/build and concrete manual steps. Do not invent human acceptance, close verification children directly, or mark a canceled test as passing.

Before upgrading the real database, export a backup outside Git. Test schema/archive compatibility with isolated fixtures first. Do not use the live workspace for synthetic pass/fail tests. Keep private exports, endpoint capabilities, and database files out of commits.

Run meaningful checks for the change, including `cargo test -p direct-core -p direct`, `cargo clippy --workspace --all-targets -- -D warnings`, and frontend checks/build when UI changes. Browser interaction uses the available computer-use surface. Claim and verification state in Direct is authoritative; the pilot document is navigation and scope, not a second editable backlog.
