# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `contracts/tholos-v2`: a new, wholly separate contract crate for protocol
  v2 (stake-weighted resolution, design in `docs/src/V2_RESOLUTION.md`),
  never upgraded in place from v1. This first issue (#64) implements the
  immutable `PolicySnapshotV2` pinned at assertion creation and the
  `AssertionV2` record it lives on, plus `initialize` and read-only lookups.
  Registration, reveal, outcome resolution, settlement, and the freeze/cancel
  mechanism are separate issues (#65-#71) landing as the crate grows. Closes #64.

- `tholos-v2`: bonded assertion posting (`assert_outcome`) and the
  uncontested-`finalize` path, the same two-stage shape v1 has for an
  assertion nobody disputes. Adds `challenge_window_secs` and
  `finalize_reward_bps` to `PolicySnapshotV2` (distinct from
  `registration_duration_secs`, which only governs the post-dispute
  third-party join window), and `opened_at`/`finalizer` to `AssertionV2`.
  `finalize_reward_bps` is carried over from v1 unchanged: the problem it
  solves (incentivizing a third party to spend gas finalizing on the
  asserter's behalf) is identical in both versions for this uncontested case.
  Closes #65.

- `tholos-v2`: `dispute` and the third-party registration phase. New
  `Position` (one address's stake, keyed by `(assertion_id, address)`) and
  `Resolution` (per-dispute deadlines and the running eligible total `W`)
  records, per V2_RESOLUTION.md's storage layout. Third-party deposits carry
  a salted commitment hiding their side until reveal (#67); repeated
  deposits from one address aggregate into one position and can't change
  that position's original commitment. A qualifying late deposit extends the
  registration deadline, capped at a hard deadline fixed at `dispute` time.
  Also fixes a gap in #64's `initialize` validation: `anti_snipe_hard_max_secs`
  is now required to be at least `registration_duration_secs`, since the
  hard deadline is `registration_opened_at + anti_snipe_hard_max_secs`, an
  absolute duration that could otherwise fall before the ordinary soft
  deadline. Closes #66.

- `tholos-v2`: the reveal phase and commitment verification. `reveal`
  verifies a third-party position's `(choice, salt)` against its stored
  commitment (via a `VoteCommitmentPreimage` struct hashed the same
  canonical-encoding way `policy_hash` already is) and counts its weight
  into `Resolution.agree_weight`/`disagree_weight`. The Registration ->
  Reveal transition is lazy, triggered by the first `reveal` call after
  `registration_deadline`, which also auto-counts and auto-reveals the
  asserter's and disputer's fixed positions, since their sides are already
  public and they never call `reveal` themselves. Closes #67.

- `packages/tholos-sdk`: a generated TypeScript client for `contracts/tholos`,
  via `stellar contract bindings typescript` against the compiled wasm (never
  a live deployment, so regenerating needs no network access or contract id).
  Committed in-repo, not yet published to npm, since it hasn't been consumed
  by a real integration yet. CI regenerates it on every push/PR and fails if
  the committed package has drifted from the contract's current interface.
  `docs/src/INTEGRATION.md` documents this as the JS/TS integration path,
  alongside the existing Rust contract-to-contract pattern. Migrating
  `demos/freelance-escrow` off its hand-rolled client to use this instead is
  a separate, deliberately out-of-scope follow-up. Closes #60.

- `tholos-v2`: weighted-majority outcome resolution. After every reveal, a
  side locks in `terminal_cause`/`final_outcome` (`StrictMajorityFor`/
  `StrictMajorityAgainst`) the moment its revealed weight exceeds half of the
  frozen eligible total `W`, checked via subtraction rather than division to
  stay exact on an odd `W`. The assertion stays `Reveal` after locking so
  other positions can still reveal to prove entitlement for settlement,
  closing to `Resolved` once `revealed_weight` catches up with `W` or
  `reveal_deadline` passes, whichever comes first; if neither side ever
  reached a majority, `terminal_cause` defaults to `OptimisticTimeout` and
  the originally asserted outcome stands. New permissionless
  `resolve_outcome` entrypoint closes a `Reveal`-phase assertion out once its
  deadline has passed, and is the only way a dispute that drew zero
  third-party registrations can ever leave `Registration`, since nobody
  would otherwise have a position to call `reveal` with. Closes #68.

- `tholos-v2`: settlement, converting a `Resolved` assertion's decided
  outcome into per-position entitlements via a new permissionless
  `settle(id, address)`. A winning position (per `terminal_cause`: the
  agreeing side for `StrictMajorityFor`, the disagreeing side for
  `StrictMajorityAgainst`, either side if revealed for `OptimisticTimeout`)
  recovers its principal plus a pro-rata share of the forfeited pool from
  losing/never-revealed positions; a losing or never-revealed position
  recovers nothing. Every position's share is computed from the same
  `(recipient_weight, forfeited_pool)` pair, derived purely from `Resolution`
  fields already frozen once `phase == Resolved`, so settling positions in
  any order never changes any individual result. `settle` doesn't move
  tokens itself, it accrues the payout to a new `Credit(id, address)` record
  (`get_credit` reads it); withdrawal is a separate, not yet implemented,
  issue. Leftover dust from floor division is credited to a deterministic
  party (the winning asserter/disputer, or the asserter for a timeout
  default) once the last recipient position settles. Tightens
  `initialize`'s `max_total_weight` bound from `MAX_BOND_AMOUNT` (~1.7 *
  10^35) to a new `MAX_SETTLEMENT_TOTAL_WEIGHT` (10^19), since the old bound
  was nowhere near tight enough to keep settlement's `amount *
  forfeited_pool` multiply inside `i128`. Closes #69.

- `tholos-v2`: credit withdrawal, via a new `withdraw(owner, id, destination)`.
  Transfers `owner`'s entire withdrawable `Credit(id, owner)` balance
  (accrued by `settle`) to `destination`, which can be any address, not
  necessarily `owner` itself, so a token that rejects transfers to `owner`
  directly can't permanently strand funds there. Effects before
  interactions: the credit balance is zeroed and `Resolution`'s new
  `outstanding_liability`/`withdrawn_total` fields updated before the
  outgoing transfer, so a failed transfer rolls back the whole call and
  never consumes the credit. Also adds a contract-wide reentrancy guard
  (`enter_reentrancy_guard`/`check_reentrancy_guard`), held for the duration
  of every external token transfer this contract initiates
  (`assert_outcome`, `dispute`, `register`, `finalize`, `withdraw`) and
  checked at the entry of `reveal`/`resolve_outcome`/`settle` as well, so a
  non-standard token whose `transfer` calls back into this contract mid-
  transfer can't act on state that looks complete before the tokens backing
  it have actually moved. Closes #70.

- `tholos-v2`: the symmetric freeze/cancel emergency mechanism, via two new
  admin-only entrypoints. `set_paused_v2(paused)` blocks new `assert_outcome`
  calls; unlike v1's broader pause, it never affects an already-active
  round, whose registration, reveal, resolution, settlement, and withdrawal
  all continue normally while paused, since blocking them would strand
  capital already locked into that round rather than protect it.
  `cancel_round(id)`, callable only while paused, cancels a round before any
  terminal outcome has locked (`Pending`, or `Registration`/`Reveal` with no
  strict majority reached yet) and refunds every already-funded position
  its exact principal, no forfeiture, no reward, as if the round never
  happened; it fails outright, not as a no-op, with `RoundAlreadyDecided`
  once `terminal_cause` is set by any means, including an earlier
  cancellation, making it structurally impossible to alter an
  already-decided result. A `Pending` cancellation refunds the asserter's
  bond directly (no `Resolution`/`Position` exists yet at that phase); a
  `Registration`/`Reveal` cancellation instead sets a new
  `TerminalCause::AdminCancelled` and lets every position recover its
  principal through the normal `settle`/`withdraw` path, since
  `settlement_pool` treats every funded position as a recipient of a zero
  forfeited pool for that cause. Emits a distinct `RoundCancelled` event,
  separate from `Resolved`, so indexers can always tell a cancellation
  apart from a real outcome. Closes #71.

## [0.3.0] - 2026-08-08

### Added

- `Assertion` gains a `final_outcome: Option<bool>` field, set at `finalize` and
  `resolve`. Previously the authoritative resolved outcome only existed in the
  `Finalized`/`Resolved` event payload; `Assertion.outcome` always stays the
  original claim even when a dispute overturns it, which was a sharp edge for
  integrators reading state after the fact. Closes #37.

- A second integration example, `contracts/asserter-consumer`, demonstrating the
  "contract-as-asserter" pattern from INTEGRATION.md
  (`env.authorize_as_current_contract`), alongside `demo-consumer`'s existing
  end-user-as-asserter example. Closes #14.

- `docs/src/BOND_SIZING.md`: a bond-sizing analysis modeling spam, bad-faith
  disputes, resolver self-rotation griefing, and `finalize_reward_bps` griefing,
  with worked formulas for choosing `bond_amount`. `DEPLOYMENT.md` now points to
  it instead of only qualitative guidance. Closes #50.

- `scripts/testnet-load.sh`: an end-to-end load and volume test scenario against
  real Stellar testnet infrastructure (sequential assert/dispute/resolve/finalize
  phases with timing and integrity checks), complementing the single-flow
  `testnet-smoke.sh`. Closes #13.

- `.github/workflows/docs-check.yml`: builds the mdBook docs site on every PR
  that touches `docs/**`, `README.md`, `CONTRIBUTING.md`, or `SECURITY.md`, so a
  broken doc build is caught before merge instead of at deploy time. Closes #16.

- A `Makefile` wrapping the common dev commands (`make check`, `make test`,
  `make build-wasm`, etc.) documented in CONTRIBUTING.md's "Before opening a PR"
  section, so contributors don't have to remember the raw `cargo`/`stellar`
  invocations. Closes #15.

- CONTRIBUTING.md's Testing philosophy section now states the test snapshot
  commit policy explicitly: commit a `test_snapshots/` file if the test that
  wrote it is reproducible, `.gitignore` it if it isn't (any `proptest_*`
  module). Closes #24.

- Configurable finalize reward (`finalize_reward_bps`, 0–1000 basis points of the
  bond) paid to whoever calls `finalize` as an incentive for prompt finalization.
  The reward is funded by the asserter's bond: the caller receives
  `bond * bps / 10_000` tokens and the asserter receives the remainder. Setting
  `finalize_reward_bps` to 0 (the default) reproduces the original no-reward
  behavior: the full bond returns to the asserter. `caller` must authorize the
  call unconditionally, regardless of the reward value, so the address recorded
  in `Assertion.finalizer` and the `Finalized` event can never be spoofed.
  `initialize` now accepts `finalize_reward_bps` as a new parameter (validated
  ≤ 1000, failing with `InvalidFinalizeReward` otherwise). `finalize` signature
  changed from `finalize(id)` to `finalize(caller, id)`. The `Finalized` event
  gains two new fields: `finalizer: Address` and `reward: i128`. `Assertion`
  gains a new `finalizer: Option<Address>` field populated on finalize. Closes #17.

- Property-based tests for resolver vote counting and majority
  (`proptest_vote_counting`), generating random odd committee sizes and vote
  sequences and checking the result against an independent reference
  implementation of the `(size / 2) + 1` majority formula. Closes #12.

- Property-based tests for `initialize`'s `bond_amount` and
  `challenge_window_secs` validation (`proptest_initialize_bounds`), fuzzing the
  full `i128`/`u64` domains against a reference implementation of the same
  checks, plus a boundary-weighted pass around `MAX_CHALLENGE_WINDOW_SECS`.
  Documented in CONTRIBUTING.md why `proptest` is used over `cargo-fuzz` (the
  latter needs the `wasm32` target and libFuzzer, which doesn't fit Soroban's
  native, mocked-`Env` test profile). Closes #11.

- CI now verifies every `contracts/*/Cargo.toml` is registered in the root
  `Cargo.toml`'s `[workspace] members`. A crate that exists on disk but isn't
  a workspace member is invisible to `cargo build/test/clippy --workspace`, so
  CI could previously pass without ever building, testing, or linting it.
  Closes #43.

- Resolver self-rotation: the committee can now replace one of its own by a strict
  majority vote (`propose_rotation`, `vote_rotation`, `cancel_rotation`), removing
  the admin as the only path to committee membership. `update_resolvers` remains as
  the emergency override; both paths emit `ResolversUpdated`, and rotation adds
  `RotationProposed` / `RotationExecuted` / `RotationCancelled` for the governance
  trail. One rotation may be open at a time, with a deterministic deadlock guard so a
  lost proposer key can't block rotation. Writes the same `Resolvers` slot as
  `update_resolvers`, so it has no effect on disputes already open (their committee
  is snapshotted at `dispute` time). Design in `docs/src/ROTATION_DESIGN.md`. Closes
  the self-rotation item from CONTRACT.md's Known gaps.
- A design-only protocol v2 proposal for stake-weighted voting by bond posters,
  including eligibility and weight snapshots, settlement, threat analysis, and a
  blue/green migration path for existing v1 deployments. No contract behavior or
  public interface changed. Refs #19.
- Reentrancy regression tests for `assert_outcome`, `dispute`, and `resolve`,
  extending the pattern already used for `finalize`. Along the way, confirmed
  that Soroban's auth model itself rejects a reentrant token's dynamically-triggered
  nested `require_auth` call, so these three aren't actually reachable by a
  hostile token acting alone; documented in ARCHITECTURE.md and CONTRACT.md.
  (At the time this was written `finalize` needed no signature; it now requires
  `caller` to authorize unconditionally, see the `finalize_reward_bps` entry
  above.) Closes #3.
- `initialize` and `update_resolvers` now reject resolver committees larger than
  `MAX_RESOLVERS` (21), since the full committee is copied onto every disputed
  assertion. Closes #4.

### Changed

- Removed `PR_DESCRIPTION.md`, a contributor's scratch file that was
  accidentally committed to the repo root instead of pasted into the PR body.
  Closes #48.

- The `evil_token` test module (`contracts/tholos/src/test.rs`) now uses a typed
  `DataKey`-style enum for its own storage keys instead of ad hoc `symbol_short!`
  strings, matching the main contract's convention. Test-only, no behavior
  change. Closes #6.

- The repeated `(committee_len / 2) + 1` majority-threshold calculation in
  `vote_rotation`, `cancel_rotation`, and `resolve` is now a single
  `Self::majority_threshold` helper. No behavior change; `proptest_vote_counting`
  already exercises exactly this formula.

- CI now passes `--locked` to every `cargo build/test/clippy` invocation, so a
  `Cargo.lock` that's drifted from what `Cargo.toml` would currently resolve to
  fails the build loudly instead of Cargo silently re-resolving and using an
  unreviewed dependency graph.

- Added a `[workspace.lints.rust] warnings = "deny"` table (with each crate
  opting in via `[lints] workspace = true`), so a local `cargo build` enforces
  the same warnings-as-errors bar CI's `-D warnings` flag does, instead of only
  CI catching it.

### Fixed

- `finalize` is now blocked while paused, alongside `assert_outcome`, `dispute`,
  and `resolve`. Previously a pending assertion could finalize uncontested even if
  its entire challenge window overlapped a pause, during which `dispute` was
  blocked, so it had no real opportunity to be challenged. Closes #36.

- `initialize` now rejects a `bond_amount` above `MAX_BOND_AMOUNT`, the tighter of
  two independent overflow constraints: the asserter's and disputer's bonds
  summing past `i128::MAX` in the token balance across a dispute, and
  `finalize`'s reward-multiply (`bond * finalize_reward_bps`) overflowing before
  it divides. A compile-time guard fails the build if a future change to either
  constant reintroduces the overflow. Closes #34.

- Added `rust-toolchain.toml` pinning the exact Rust toolchain version, and fixed
  CI's install step to actually respect it (`dtolnay/rust-toolchain`'s
  `toolchain` input turned out to be hard-required with no file-reading
  fallback, so CI switched to plain `rustup` commands, which auto-detect the
  pinned version). Previously CI floated on `stable`, so wasm codegen could
  silently drift between runs with no source change, the root cause of several
  confusing snapshot diffs this cycle. Closes #38.

- `initialize` and `update_resolvers` now reject a resolver committee
  containing duplicate addresses. A committee like `[A, A, B]` previously
  passed the odd-length check while being an effective electorate of two,
  silently breaking the "majority can never tie" guarantee, and could make
  the majority denominator unreachable in the worst case, stranding both
  bonds on a dispute nobody could resolve. Closes #35.

- Committed test snapshot JSONs no longer show up as spuriously modified on
  Windows checkouts. Added a `.gitattributes` forcing LF line endings
  regardless of each contributor's local `core.autocrlf` setting. Closes #39.

- Corrected stale documentation in DEPLOYMENT.md and GLOSSARY.md that still
  described `finalize` as callable without authorization; `caller` has
  required auth unconditionally since the `finalize_reward_bps` change above.
- Persistent `Assertion` storage now has its TTL extended by 30 days on every
  write (`assert_outcome`, `dispute`, `finalize`, `resolve`), through a shared
  `set_assertion` helper. Previously only instance storage got a TTL bump, so a
  long-lived `Pending` or `Disputed` assertion could have its ledger entry
  archived before anyone acted on it. Closes #1.
- `initialize` now rejects `challenge_window_secs` over 7 days, not just zero.
  A window close to the 30-day TTL bump left little margin for `finalize` or
  `resolve` to actually be called before the entry risked archival. Closes #2.
- The internal `NextId` read in `assert_outcome` now goes through the same
  `NotInitialized`-returning helper as every other storage read, instead of
  silently defaulting via `.unwrap_or(0)`. No observable behavior change (the
  pause check already fails first on an uninitialized contract), but removes
  an inconsistent pattern. Closes #5.
- Added regression tests for `Error::NoRotationProposal`, triggered via both
  `vote_rotation` and `cancel_rotation` when no proposal is open. This closes the
  last CONTRIBUTING.md gap where a new `Error` variant introduced by the
  self-rotation feature lacked a triggering test; every new `Error` variant now
  has one.

## [0.2.0] - 2026-07-10

### Added

- Validation for `initialize`: `bond_amount` must be positive
  (`InvalidBondAmount`) and `challenge_window_secs` must be non-zero
  (`InvalidChallengeWindow`).
- `shellcheck` for `scripts/*.sh` in CI.
- Documentation reorganized into `docs/` (formerly `book/`), with GitHub-special
  files (`README.md`, `CONTRIBUTING.md`, `SECURITY.md`) staying at root and
  everything else (`ARCHITECTURE.md`, `CHANGELOG.md`, `CONTRACT.md`,
  `DEPLOYMENT.md`, `GLOSSARY.md`, `INTEGRATION.md`) living directly under
  `docs/src/`.

### Fixed

- Resolver committee is now snapshotted onto an assertion when it's disputed
  (`Assertion.resolvers`), and voting/majority for that dispute are decided
  against the snapshot for its whole lifetime. Previously `resolve` re-read the
  live committee on every call, so an `update_resolvers` call mid-dispute could
  change who was entitled to decide it and what majority meant, partway through
  voting.
- The internal `Self::get` storage helper no longer panics on missing storage;
  it returns `Error::NotInitialized` like the rest of the contract's error
  paths.

### Changed

- Test suite refactored around a shared `Fixture` helper to cut the boilerplate
  repeated across nearly every test (env setup, token registration, contract
  registration, initialization).

## [0.1.0] - 2026-07-09

Initial release: a working, tested, testnet-deployed assertion and dispute oracle.

### Added

- `contracts/tholos`: the core assertion and dispute contract, with `initialize`,
  `assert_outcome`, `dispute`, `finalize`, `resolve`, `update_resolvers`, and
  `set_paused`.
- Admin-controlled resolver committee updates (`update_resolvers`), so a
  compromised or unresponsive resolver can be replaced without redeploying.
- Admin-controlled pause (`set_paused`) for `assert_outcome`, `dispute`, and
  `resolve`. `finalize` and `update_resolvers` deliberately stay callable while
  paused.
- `contracts/demo-consumer`: a minimal example contract calling into Tholos,
  validating the cross-contract integration pattern documented in
  [INTEGRATION.md](INTEGRATION.md) against Tholos's real compiled wasm.
- `scripts/testnet-smoke.sh`: an end-to-end check against real Stellar testnet
  infrastructure (deploy, initialize, assert, dispute, resolve).
- CI (`fmt`, `clippy`, `test`, wasm build) on every push and pull request.
- Documentation: `README.md`, `CONTRACT.md`, `INTEGRATION.md`, `CONTRIBUTING.md`,
  published as a site via mdBook and GitHub Pages.

### Fixed

- Reentrancy: `assert_outcome`, `dispute`, `finalize`, and `resolve` now write
  their state change before calling the external token contract's `transfer`,
  closing a hole where a non-standard or malicious token could re-enter mid-call
  and drain bonds belonging to unrelated assertions. Covered by a regression test
  (`test_finalize_is_not_reentrant`) using a token that actively attempts the
  reentrant call.
