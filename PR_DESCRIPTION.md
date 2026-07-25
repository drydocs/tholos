# PR #23 description — Resolver self-rotation via committee vote

Paste the contents below into the PR body on GitHub (the template was previously
deleted; this fills it out). `gh` is not authenticated in this environment, so this
file is the source of truth until you paste it (or run the `gh pr edit` command in the
final report).

---

## Summary

Closes #18. Implements resolver self-rotation, removing the single-admin-key as the
only path to committee membership. The committee can now replace one of its own by a
strict majority vote, routed through the contract rather than the admin.

Three new resolver-callable functions: `propose_rotation`, `vote_rotation`,
`cancel_rotation`. Design record: `docs/src/ROTATION_DESIGN.md`. Rationale and the
per-dispute snapshot interaction are in `docs/src/ARCHITECTURE.md` (Resolver
self-rotation section).

**Economic consequences (explicit per CONTRIBUTING.md):**

- **Bond amounts: unchanged.** No change to `assert_outcome` / `dispute` / `finalize` /
  `resolve` economics. Rotation never moves tokens, so there is no reentrancy surface.
- **Resolver behavior: changed (intended).** The set of addresses entitled to decide
  disputes can now change by committee vote, not just by the admin. This is the
  centralization reduction the issue asked for.
- **In-flight disputes: unaffected.** Rotation writes the same `Resolvers` slot
  `update_resolvers` writes, and a dispute snapshots the live committee at `dispute`
  time (`Assertion.resolvers`). A rotation completing mid-dispute behaves exactly like
  an `update_resolvers` mid-dispute: no effect on that dispute. No change to
  `Assertion`, `dispute`, or `resolve` was needed.
- **Liveness:** at most one rotation open at a time; a deterministic deadlock guard
  auto-cancels a proposal that can no longer reach a majority, so a lost proposer key
  cannot permanently block rotation.
- **Admin override wins races:** `update_resolvers` clears an open self-rotation
  proposal, so a live proposal always matches the committee it was validated against.
- **Pause-exempt**, like `update_resolvers`: rotation is internal governance, not new
  exposure.

`update_resolvers` is retained as the emergency override (the one case self-rotation
can't solve: a compromised/deadlocked committee can't heal itself). Both paths emit
`ResolversUpdated`; rotation adds `RotationProposed` / `RotationExecuted` /
`RotationCancelled` for the governance trail.

## Test plan

- [x] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test` pass locally (44 tests, 0 failures)
- [x] `CONTRACT.md` updated (rotation functions, events, and the new `Error` variants documented; snapshot-invariant interaction noted)
- [x] `ROTATION_DESIGN.md` added as the design record for issue #18
- [ ] `scripts/testnet-smoke.sh` — N/A to run in this environment. Rotation is internal
      governance with no token movement, so the existing smoke flow
      (deploy / initialize / assert / dispute / resolve) still exercises the unchanged
      economic paths. A manual testnet run is recommended before merge if desired; it
      does not need to cover rotation specifically.
- **What I manually verified beyond automated tests:**
  - Every new `Error` variant has a triggering test, including the two previously
    untested `NoRotationProposal` paths: `test_cannot_vote_rotation_without_proposal`
    and `test_cannot_cancel_rotation_without_proposal`.
  - Majority math (`len / 2 + 1`) and the deadlock guard:
    `test_rotation_requires_majority_then_executes`,
    `test_deadlock_autocancels_rotation`.
  - Snapshot invariance: `test_rotation_does_not_affect_in_flight_dispute`.
  - Admin override clears an in-flight proposal:
    `test_admin_update_resolvers_cancels_open_rotation`.
  - Pause-exemption: `test_rotation_is_pause_exempt`.
  - Each rotation function carries an `// Audited via: <test>` comment in `lib.rs`
    tying the code path to its verifying test (CONTRIBUTING.md "wire in the trace").
