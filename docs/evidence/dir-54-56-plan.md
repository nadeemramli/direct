# DIR-54 / DIR-55 / DIR-56 criterion-to-evidence plan

Written before implementation (cloud checkpoint, branch `claude/direct-reliability-54-56`,
base `origin/main` 17df1c9). Synthetic, isolated workspaces only; no owner data, credentials or
live Direct writes. The cloud cannot reach the Windows service, so every Windows-native item below
is marked **coordinator** and stays open until the integration agent observes it.

Evidence locations: browser scripts under `scripts/e2e/`, Rust tests under `crates/*/tests/`,
sanitized run logs and screenshots in the PR description / handoff comment (not committed).

## DIR-54 — comment drafts stay bound to their originating issue

| # | Criterion | Entrypoint and action | Expected | Evidence |
|---|-----------|-----------------------|----------|----------|
| 54.1 | Distinct drafts on A and B survive switching | Built UI over a real `direct serve`; type on A, open B, type, back to A, then B | Each textarea shows only its own issue's text | `comment-drafts.browser.mjs` checks; run on old build (expect FAIL) and fixed build |
| 54.2 | Guidance trip and back keeps the draft | Type on A, open a guidance card, "Back to issue" | A's draft intact | same script |
| 54.3 | No silent retarget | Type on A, switch to B, press "Add comment" | B is posted only with B's text (or disabled when B empty); A's text never reaches B | pre-fix reproduction posts A's text to B; post-fix asserts B unchanged |
| 54.4 | Post from A lands once on A only | Post A; re-read `context` A and B through the service, reload the page, restart the service | A has the exact text exactly once; B has none | same script, service restart in-script |
| 54.5 | Switching during an in-flight post | Delay the `comment` response (route interception that forwards to the real service), switch to B, type, release | A gets its text once; B's draft is neither cleared nor submitted | same script |
| 54.6 | Empty and failure states | Empty draft: button disabled; forced transport failure on post: draft kept, error shown, retry posts once | same script |
| 54.7 | Keyboard flow | Tab into textarea, type, Ctrl/Cmd+Enter or Tab+Enter submits; focus remains usable | same script |
| 54.8 | Windows native smoke | **coordinator**: installed desktop + browser entrypoint | — | handoff steps |

## DIR-55 — committed writes vs failed refreshes; retry identity

| # | Criterion | Entrypoint and action | Expected | Evidence |
|---|-----------|-----------------------|----------|----------|
| 55.1 | Reproduce the duplicate window | Old build: create issue, fail the follow-up `snapshot` once, press Save again | **pre-fix**: two issues persisted | `write-recovery.browser.mjs` pre-fix run |
| 55.2 | Committed + refresh failure reported accurately | Fixed build: same fault | Modal closes; UI says saved and reconnecting; after transport restore exactly one issue in service export, reload, restart | same script |
| 55.3 | Dropped mutation response, retry exact payload | Forward `create_issue` to the real service, then abort the response; press Save again | Same request ID reused; one issue persisted; UI says outcome unknown and retry is safe | same script + request-ID capture |
| 55.4 | Distinct user operation creates a distinct record | Create an identical issue a second time through a new "＋ Issue" form | Two records (different request IDs) | same script |
| 55.5 | Version conflict while editing | Edit issue; change it through a second actor; Save | Typed fields kept; conflict notice with explicit "Load latest and merge"; other actor's change not overwritten; re-save uses fresh version and new request ID | same script |
| 55.6 | Version conflict while submitting a handoff | Owner claim → submit modal; renew claim from another session; Submit | Handoff text kept; explicit refresh action; resubmission with fresh version | same script |
| 55.7 | Desktop transport contract | `direct::Client` + Tauri `direct_command` error mapping | Service rejections are classified `rejected` with code; unreachable / unreadable responses are `unknown` | Rust test in `crates/direct/tests/service.rs` against a real service |
| 55.8 | Server idempotency remains authoritative | Replay identical request; replay with different payload | identical replay returns the stored response, different payload is a conflict | existing core tests + new service test |
| 55.9 | Windows native smoke | **coordinator** | — | handoff steps |

## DIR-56 — malformed verification relationships during restoration

| # | Criterion | Entrypoint and action | Expected | Evidence |
|---|-----------|-----------------------|----------|----------|
| 56.1 | Reproduce the panic | Archive with Verify parent, valid pending run, `verification_key: null`; `direct restore`, `serve`, owner `review` | **pre-fix**: restore accepted; review returns "Worker failed"; subsequent `list`/`context` fail ("Store lock unavailable") | `crates/direct/tests/service.rs` pre-fix run (logged) |
| 56.2 | Malformed archives rejected before destination creation | `restore` and `recovery-check` with: missing child, mismatched child, dangling child, wrong-parent run, child current-run mismatch, child/run status mismatch, run attached to a child, run without child, orphan run | Actionable error naming the issue and relationship; destination directory not created | `crates/direct/tests/service.rs` + core unit tests |
| 56.3 | No review path panics | Store-level defensive errors where `unwrap` was used | Error returned; service keeps serving list/context | core + service tests |
| 56.4 | Valid lifecycle round trip | submit → fail → resubmit → review pass → export → restore → recovery-check (real service, synthetic agent + owner actors) | History, both runs and distinct outcomes preserved; recovery-check matches | service test |
| 56.5 | Legacy archives remain readable | All existing format-1…12 fixtures and compatibility tests | Unchanged pass | full core/CLI test suites |
| 56.6 | Windows smoke | **coordinator**: private archive restore drill with new binary | — | handoff steps |

## Commands (exit codes recorded in the PR)

`cargo fmt --all --check`; `cargo test -p direct-core -p direct --locked`; `cargo test -p direct-mcp --locked`;
`cargo clippy --workspace --all-targets --locked -- -D warnings` (whole workspace including the Tauri crate,
if its Linux system libraries can be installed); `npm run check`; `npm run build`; the existing
`navigation.browser.mjs` and new browser scripts against the final SHA.
