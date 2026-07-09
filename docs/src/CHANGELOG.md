# Changelog

All notable changes to this project are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
  [INTEGRATION.md](docs/src/INTEGRATION.md) against Tholos's real compiled wasm.
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
