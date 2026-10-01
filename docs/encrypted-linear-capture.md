# One-off encrypted Linear capture

The manual `Encrypted Linear capture` workflow prepares a private source package;
it never imports into Direct, changes Linear records or changes billing.
The repository is public: Actions logs and artifacts are not private merely
because the key is an Environment secret.

The owner authorized this transfer on 1 October 2026: Linear data is temporarily
plaintext on an ephemeral GitHub-hosted runner, then transferred as an encrypted
artifact to the owner's PC. Only the pinned age public recipient is in Git. Its
private identity stays outside Git on the PC. The captured archive and all
stdout/stderr (including attachment names and failed-request diagnostics) are
encrypted together. Upload uses one exact ciphertext path and one-day retention.
No cache, plaintext upload, issue attachment or release asset is used.

## Controls

- Manual dispatch only, from `nadeemramli/direct` main by `nadeemramli`.
- The job references `migration-linear-personal-api-key` and maps the managed
  `LINEAR_MIGRATION_API_KEY` secret to the capture subprocess's `LINEAR_API_KEY`.
  Read permission for ATT, PFN, PBK, TWI and CVS is required; no write permission.
- Checkout uses the immutable workflow commit. The capture script's canonical
  LF SHA-256 is pinned to the reviewed revision. Actions, Node and age are pinned;
  the downloaded age archive must match its recorded SHA-256 before execution.
- Synthetic encryption/decryption, private failure diagnostics, tamper, wrong
  configuration, retry-preservation and cleanup checks run before secret use.
  PR checks run separately without the Environment or Linear credentials.
- The job token has only `contents: read`; checkout does not persist it.
  A 55-minute job limit and 45-minute capture limit bound a single run.
- A capture failure still produces encrypted diagnostics when encryption works;
  the job remains failed. Encryption failure publishes no capture artifact.

## Receipt and reconciliation

1. Record the dispatched run ID and commit. Inspect its state before retrying;
   do not blindly dispatch another capture on a disconnect.
2. Download only that run's encrypted artifact to a new private local directory.
3. Decrypt with the local identity, safely extract, and verify manifest and file
   checksums. Inspect capture exit status, errors, missing coverage, team/issue
   inventory and comparisons with the prior package and current source.
4. Delete the remote ciphertext artifact only after the local encrypted copy,
   decrypted files and integrity checks are confirmed. Retain local private
   evidence; do not publish its contents or diagnostics.

An exit-zero capture is not proof of complete coverage or final freshness.
Deleted source records, external link contents and API coverage limitations are
reported separately. Existing intake may still change Linear after capture.
The live migration still requires a fresh Direct backup, explicit product map,
isolated preparation/reconciliation and owner-only application. See
[source capture](linear-migration-source.md) and [import](linear-import.md).

The temporary Linear key should be revoked after the verified capture handoff.
Deleting a GitHub secret is not equivalent to revoking the Linear API key.
