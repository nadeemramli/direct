# Direct cloud and local work

Owner model preference (1 October 2026): all future Claude Code tasks use **Opus 5.5**, including implementation, review and resumed assignments. The coordinator must explicitly select and verify the session model before dispatch; if unavailable, report that limitation instead of silently falling back.

Read `AGENTS.md`, `docs/agent-contract.md`, and `docs/e2e-delivery.md` before work. Direct's live issue and claim are authoritative.

A cloud commit or successful push is an implementation checkpoint. For a Windows delivery, keep work in Doing until the integration agent has merged, tested, installed, and observed the exact build through the user's actual entrypoint. Record a handoff comment with branch, full SHA, checks, remaining acceptance scenarios, and integration requirements. Do not request owner verification from a cloud-only result. The submit API requires matching tested/delivered build evidence; never fabricate that evidence.
