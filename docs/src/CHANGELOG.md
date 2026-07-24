# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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

- The `evil_token` test module (`contracts/tholos/src/test.rs`) now uses a typed
  `DataKey`-style enum for its own storage keys instead of ad hoc `symbol_short!`
  strings, matching the main contract's convention. Test-only, no behavior
  change. Closes #6.

### Fixed

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
