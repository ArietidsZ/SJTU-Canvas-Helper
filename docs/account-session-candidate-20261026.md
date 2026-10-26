# Account/session candidate: verification boundaries

This is a review candidate based on commit f804dd55477418a7c21eaa5401e2dc59e04ef50c, not a port onto the newer upstream main.

## Source provenance
- Original captured index tree: e2b43a79f8aa74b537ed5992bbe6225a44964848.
- 45 text files: 39 modified, 6 added.
- All 39 base files and all 45 reconstructed postimages matched their byte lengths, Git blob IDs and SHA-256 hashes before the follow-up edits below.
- Original source patch SHA-256: 09209ca64080ce76571488d0807cd81e28f1f8afabb36425d4d4f6d130657b7f.

## Follow-up source changes
- Video login propagates OAuth lookup errors and rejects missing/blank cookies or key; a new cookie/key pair commits together under the owner lock.
- Key-only refresh merges into the latest same-owner configuration, preserving ordinary settings saved during network waits, and uses the returned rotated owner.
- Three pure pair-validation test methods were added, but NOT compiled or executed.
- Removed one extra EOF blank line. Existing CRLF style is retained in five files; default git whitespace diagnostics for CRLF are not represented as resolved runtime failures.

## Historical evidence, not current-candidate passes
- 2026-10-23 original Cookie regression: 2 passed / 1 failed; authentication loss was preserved as a failure.
- 2026-10-25 media-auth v1: two redirect-scope failures. Subsequent v2 synthetic suite: 16 / 16 passed.
- 2026-10-25 locked frontend checks: typecheck and lint passed (0 errors / 69 warnings); 37 targeted tests passed.
- Latest integrated Rust gate has NOT run. The prior dependency-fetch denial was not retried.
- Eight planned login writeback regressions have NOT run.
- No real SJTU media authentication, full GUI, or cross-account live-service acceptance.

## Known remaining risks / merge gates
- Canvas/SSO login paths still use a shared live cookie Jar; the isolated legacy video login/key path does not prove all authentication paths isolated.
- Authentication clients still lack an explicit bounded request timeout.
- Configuration persistence inherits possible MCP-start failure after configuration commit; no blanket transactional-success claim.
- New pure validation tests do not prove owner races, disk atomicity, or persistence rollback.
- Reconcile with current upstream before upstream submission; do not overwrite newer upstream diagnostic/frontend behavior.
- Keep this PR Draft until full current-source Rust/frontend validation and required ownership/authentication regressions pass. No automatic merge or deployment.
