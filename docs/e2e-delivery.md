# End-to-end delivery contract

Owner direction, 1 October 2026: every issue must represent a working user outcome before the agent asks the owner to verify it. This applies to existing work and every future issue, including cloud contributions.

## Two passes

The Development Operating System's maintained `9. Agent E2E Verification Protocol.md`, exposed through Theoria as `dos-e2e-verification`, defines the shared policy. Read relevant guidance and the issue's acceptance before implementation, verify freshness, and pin the source fingerprint with the actual known playbook version. Do not invent a version or load every playbook.

1. **Agent verification:** reproduce, implement, run the applicable checks and complete affected E2E journeys, fix failures, then verify the exact delivered artifact. Record **Pass**, **Fail**, or **Blocked**, with each criterion's action, expectation, observation and evidence. Missing, skipped, canceled or blocked required checks cannot be Pass. Fail/Blocked stays in Doing with evidence and a concrete next action.
2. **Human acceptance:** after Agent Pass, the owner reviews the outcome, usability, evidence and important tradeoffs, with focused spot checks. Give a concise first-pass verdict and usually one to three purposeful human checks; add more only for a specific risk. Do not transfer deterministic suite execution, build/install work or diagnosis to the owner. The human independently accepts, requests changes or cancels the current run.

Current data already separates these facts: `Verification.e2e.outcome` is the agent's passing first-pass assertion; `Verification.outcome` starts pending and records the owner's second-pass result. Fail/Blocked agent attempts belong in issue evidence/comments before submission; these labels do not introduce new native issue statuses. Evidence is auditable, not a guarantee that an assertion is true. A change request leads to agent repair, another first pass, and a new run tied to the retested build.

Verification scales to the deliverable. A documentation-only change needs source, link, consistency and delivery checks; it does not need an unrelated application test suite. UI/client changes must exercise their actual flow. Native-specific acceptance still needs native evidence.

## Before submission

1. Read the issue acceptance and previous failed reviews. Write an executable scenario for each criterion: entrypoint, setup, action, expected result, observed result, and evidence location. An unclear criterion remains unresolved, not passed.
2. Integrate the authorized branch into the delivery checkout and rerun relevant checks on the combined tree. A push, PR, or merge does not install an application.
3. Exercise the entire changed flow. UI work must go through the UI, finish the action, confirm persisted state, and check reload/restart where relevant. CLI/MCP work must use a real client and running service. Documentation work must run its documented path. Use isolated fixtures for destructive actions, synthetic reviews, schema upgrades, and archive restoration. Respect host approval requirements; a blocked action is unverified.
4. Build the exact tested revision. Back up the real workspace outside Git before a database upgrade. Update service, desktop executable, and frontend assets together, restart the correct service, and confirm workspace identity and existing records. Smoke-check the actual user entrypoint. For a separate product, use its authorized deployment target; a production approval requirement still applies.
5. Submit only after the acceptance scenarios and delivery checks pass. Include the exact matching build references, environment, entrypoint, observations, evidence pointers, automated checks, and remaining non-blocking limitations. Owner steps confirm the result; they must not ask the owner to merge, build, install, debug, or perform the agent's missing checks.

The `e2e` submission object records `build_ref`, `delivered_build_ref`, `environment`, `entrypoint`, `scenarios`, `outcome`, and `delivery_check`. Both references must equal the submission's `build_ref`, the outcome must be `passed`, and all evidence fields must be nonempty. These are auditable agent assertions, not independent attestation that arbitrary prose is true. The agent remains responsible for truthful, criterion-by-criterion evidence. All mutation entrypoints use the same server gate.

Existing verification history remains intact. Older pending runs are excluded from the review-ready queue and cannot be passed until an agent resubmits with E2E evidence; upgrading does not manufacture a passing check or owner acceptance. Completed work retains its actual owner result. Incomplete or failed acceptance scenarios require more agent work and a new run.

## Cloud handoff

Record full commit SHA, branch, actual cloud checks, untested platform boundaries, and the next integration action in the issue. Coordinate release/reacquisition of the claim through the contract. The integration agent uses its own actor, verifies the combined Windows build, installs it, and submits only its observed result. Never impersonate the cloud actor or treat its claimed checks as local observations.

## Direct Windows delivery

Use a staging `CARGO_TARGET_DIR` so the running executable does not block compilation. Run the required Rust tests, MCP tests, strict Clippy, Svelte checks, and frontend build. Test a private archive restore with the new binary before upgrading the real workspace. Preserve the previous executables and export. Stop only the verified pilot service and its desktop, copy the tested binaries together to the shortcut's directory, and restart with the canonical data directory and the built assets. Confirm the service's executable path, binary hashes, workspace ID, record counts, and visible feature controls. Open a fresh owner browser session after a service restart. Keep the installation record and private archives outside Git.
