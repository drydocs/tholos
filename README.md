# Tholos

Bonded assertion and dispute oracle for resolving real world outcomes. Resolution infra for prediction markets and anything else that needs a trustworthy yes/no.

## Status

The assertion and dispute contract (`contracts/tholos`) is implemented, tested, and has been deployed and exercised on Stellar testnet.

- Core propose/dispute/resolve flow: done
- Admin-controlled resolver committee updates: done
- CI (fmt, clippy, tests, wasm build): done
- Fee-funded reward for uncontested finalizes: not yet (no fee-generating market layer exists to fund it)
- Pause / emergency-stop: not yet

See [CONTRACT.md](CONTRACT.md) for the full interface and known gaps.

## Why

Prediction markets and similar products eventually need to answer a hard question: who decides what actually happened? Existing approaches either rely on token holder votes that can be captured by large holders with a stake in the outcome, or on a centralized, regulated party acting as sole resolver.

Tholos is a bonded assertion and dispute contract: anyone can propose an outcome by posting a bond, and a challenge window gives others the chance to dispute it before it finalizes. It is designed to be standalone and composable, so any contract that needs a trustworthy resolution of a real world event can plug into it rather than building its own oracle logic.

## How it works

```text
assert_outcome                     resolve (majority)
      |                                   ^
      v                                   |
  [Pending] -------- dispute -------> [Disputed]
      |
 challenge window elapses, no dispute
      |
      v
   finalize
      |
      v
  [Resolved]  (winner paid both bonds if contested,
               asserter's bond returned if not)
```

A bond gets posted, a window gives anyone the chance to dispute it, and if disputed, a resolver committee votes to decide who was right. See [CONTRACT.md](CONTRACT.md) for the full state diagram, function reference, and events.

## Tech stack

| Layer | Technology |
| --- | --- |
| Contract | Rust, [Soroban SDK](https://developers.stellar.org/docs/build/smart-contracts/overview) 26 |
| Network | Stellar (testnet today) |
| Token | Any SEP-41 / Stellar Asset Contract token, configured per deployment |
| CI | GitHub Actions: `cargo fmt`, `cargo clippy`, `cargo test`, wasm build |

## Project layout

```text
contracts/
  tholos/            The assertion and dispute contract
    src/
      lib.rs          Contract logic
      test.rs         Unit tests (soroban-sdk testutils, mocked ledger and auth)
scripts/
  testnet-smoke.sh    End-to-end check against real Stellar testnet infrastructure
.github/workflows/
  ci.yml              Runs fmt, clippy, tests, and the wasm build on every push/PR
```

## Development

Requires the Rust toolchain with the `wasm32v1-none` target, plus the [Stellar CLI](https://developers.stellar.org/docs/tools/cli/install-cli) for building and deploying the contract.

```sh
# Run unit tests
cargo test

# Check formatting and lints (same checks CI runs)
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings

# Build the optimized contract wasm
cd contracts/tholos && stellar contract build
```

To exercise a fresh deploy end-to-end against Stellar testnet (deploy, initialize, assert, dispute, resolve):

```sh
bash scripts/testnet-smoke.sh
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines.

## License

[MIT](LICENSE)
