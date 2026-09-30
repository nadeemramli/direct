# Foundation verification — 28 September 2026

## Automated checks

- Rust workflow tests cover human-only review, all-step passing, cancellation, failure feedback, retests, reopening, and rejection of results for an old run even with a current issue version.
- Version conflicts, exact-request replay, payload/ID conflicts, and expired claims are exercised.
- A real HTTP integration test races two clients for one claim, checks agent/owner boundaries, rejects missing authentication, foreign Origin and Host, checks one-use launch grants, proves only the owner capability can request graceful service shutdown, and verifies persistence across service restart.
- Archive round trips preserve identity, evidence, event cursors, and replay results; invalid references are rejected without changing restored data.
- Svelte type/accessibility checks and the production frontend build pass.
- All Rust integration tests pass; workspace Clippy and Rust formatting checks pass.
- Tauri desktop compilation succeeds. The portable-package harness verifies that the packaged native process exposes a Windows window, the service persists after window close, and an isolated fixture survives graceful restart. Visual correctness and real-workspace owner acceptance remain manual.

## Browser + Windows/WSL smoke test

Used a separate `%LOCALAPPDATA%\Direct-foundation-preview` workspace, not imported Linear data or the normal Direct workspace.

1. A WSL process invoked the Windows CLI to create DIR-1. The browser showed it without reloading.
2. The owner interface supplied acceptance criteria and ownership, then made it Ready.
3. The WSL agent claimed and submitted build `foundation-smoke-v1`. Verification child DIR-2 appeared.
4. A synthetic failed step returned the parent to Doing with feedback.
5. The agent reclaimed and submitted `foundation-smoke-v2`. The previous failed run remained available.
6. A passing synthetic review completed the parent and verification child. The Needs me count returned to zero.

These records test application behavior; they do not represent Nadeem accepting the application or any real product change. The build-specific review safeguard was subsequently added and regression-tested.

After rebuilding, the browser launch script restarted the service, and the same completed issue and evidence loaded successfully. The WSL wrapper read its context. The CLI exported the two issues and two test runs, then restored them into a fresh directory without overwriting any existing workspace.

![Direct foundation interface](screenshots/foundation-preview.png)

## Acceptance still needed

Launch the native desktop on the owner's normal workspace, create a real small task, complete it with an actual coding agent, and verify the delivered change. Test packaging, restore on a second machine, and large imported datasets before replacing Linear. Migration and project planning remain separate milestones.
