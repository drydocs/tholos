# Contributing

## Setup

- Rust toolchain (stable) with the `wasm32v1-none` target: `rustup target add wasm32v1-none`
- [Stellar CLI](https://developers.stellar.org/docs/tools/cli/install-cli), for building and deploying the contract

Clone the repo, then build Tholos's wasm once before anything else:

```sh
cargo build -p tholos --target wasm32v1-none --release
cargo test
```

The first command is required, not optional: `demo-consumer` imports Tholos's
compiled wasm at compile time (`contractimport!`), so `cargo test`,
`cargo clippy --workspace`, and any IDE build of the workspace will fail on a fresh
checkout until that file exists. Only re-run it after changing `contracts/tholos`;
`demo-consumer` alone doesn't need a rebuild between runs.

## Project layout

```text
contracts/
  tholos/               The assertion and dispute contract
    src/
      lib.rs             Contract logic
      test.rs            Unit tests (soroban-sdk testutils, mocked ledger and auth)
  demo-consumer/        Minimal example contract that calls into Tholos
    src/
      lib.rs             Cross-contract call pattern from docs/src/INTEGRATION.md
      test.rs            Validates that pattern against Tholos's real compiled wasm
scripts/
  testnet-smoke.sh      End-to-end check against real Stellar testnet infrastructure
.github/workflows/
  ci.yml                 Runs fmt, clippy, tests, and the wasm build on every push/PR
```

`demo-consumer` exists to keep [INTEGRATION.md](docs/src/INTEGRATION.md) honest: it's not a product, it's a
compiled check that the documented integration pattern actually works. If you
change Tholos's public interface, update `demo-consumer` too if it uses the
changed function, and re-run its test.

If a second real contract is added later (e.g. a market factory), it should live as
its own crate under `contracts/`, added to the `[workspace] members` list in the
root `Cargo.toml`, following the same layout as `contracts/tholos`.

## Testing philosophy

There are two layers, and they catch different things:

- **Unit tests** (`cargo test`) run against a mocked ledger and mocked auth. Fast,
  deterministic, and where most new behavior should be covered, including every new
  `Error` variant you introduce: if you add a new failure path, add a test that
  triggers it.
- **The testnet smoke script** (`scripts/testnet-smoke.sh`) deploys to a real
  network and exercises real auth, real storage TTLs, and a real SAC token. This is
  the only thing that can catch a class of bug unit tests structurally can't (for
  example, an auth check that's satisfied by `mock_all_auths()` in tests but fails
  against a real signature). Run it before opening a PR that changes contract
  behavior in a way that affects the deployed flow, not for every change.

Property-based testing, via the [`proptest`](https://docs.rs/proptest) crate, is
used within the unit-test layer where hand-picked boundary values aren't enough to
be confident an invariant holds across a whole input space (e.g. numeric parameter
validation, or a vote-counting formula that must hold for every committee size).
`cargo-fuzz` isn't used: it needs the `wasm32` target and a libFuzzer-driven
executable, which doesn't fit Soroban's native, mocked-`Env` test profile that these
contracts' unit tests run against; `proptest` runs as ordinary `#[test]`s in that
same profile. Proptest-based tests live in their own `mod proptest_*` inside
`test.rs`, next to the hand-written tests they complement, and set
`fork = false` in their `ProptestConfig` because Soroban's `Env` isn't `Send`.

## Code standards

- **Naming:** `snake_case` for functions and variables, `PascalCase` for types
  (`Assertion`, `Status`, `Error`), `UPPER_SNAKE_CASE` for constants
  (`INSTANCE_BUMP_AMOUNT`).
- **Error handling:** contract entry points return `Result<T, Error>`; add a new
  `Error` variant rather than panicking for anything a caller could plausibly
  trigger (bad input, wrong state, missing auth). Reserve `.unwrap()` for values
  that are only unreachable because of a prior check in the same function (see
  `Self::get`, which unwraps instance storage that `initialize` is responsible for
  guaranteeing exists), and prefer propagating `Error::NotInitialized` where that
  precondition can't be locally guaranteed instead, as `update_resolvers` does.
- **Doc comments:** every public contract function gets a `///` summary covering
  what it does, who must sign it, and which `Error`s it can return.
- **Security:** validate all inputs and assume callers are adversarial. Never read
  a storage key without either handling the "missing" case explicitly or having a
  preceding check in the same function that guarantees it exists.

## Docs site

`docs/` is an [mdBook](https://rust-lang.github.io/mdBook/) that publishes this
repo's docs as a site, deployed automatically from `main` by
`.github/workflows/docs.yml`. Where a given doc's real content lives depends on
whether GitHub treats it specially:

- `README.md`, `CONTRIBUTING.md` (this file), and `SECURITY.md` stay at the repo
  root, because GitHub does something with them there (README renders on the repo
  homepage, CONTRIBUTING is linked when opening an issue/PR, SECURITY.md powers
  the Security tab). Their `docs/src/` copies are one-line
  `\{{#include ../../X.md}}` stubs; edit the root file, not the stub.
- `ARCHITECTURE.md`, `CHANGELOG.md`, `CONTRACT.md`, `DEPLOYMENT.md`,
  `GLOSSARY.md`, and `INTEGRATION.md` get no special treatment from GitHub at
  root, so their real content lives directly under `docs/src/`, with no root
  duplicate. Edit them there; they're still normal markdown files GitHub renders
  fine if you click into `docs/src/CONTRACT.md` directly, they just aren't at the
  repo's top level.

Preview locally with `mdbook serve docs` (requires `cargo install mdbook`).

## Opening issues

Use one of the two issue templates. Blank issues are disabled.

Every issue title uses the bracket prefix format `[Type] Short imperative
description`:

| Prefix | When to use |
| --- | --- |
| `[Bug]` | Something in a contract, script, or CI is broken or behaving incorrectly. Use the Bug Report template. |
| `[Feature]` | A new capability or a test that exercises new behavior. Use the Feature Request template. |
| `[Chore]` | Dependency bumps, CI/tooling tweaks, docs-only changes, or cleanup that isn't a new capability. Also uses the Feature Request template. |

If you think you've found a security vulnerability rather than a functional bug,
don't open an issue at all; see [SECURITY.md](SECURITY.md) instead.

## Before opening a PR

Run the same checks CI runs, in this order (see the note above on why the wasm
build has to come first):

```sh
cargo fmt --check
shellcheck scripts/*.sh
cargo build -p tholos --target wasm32v1-none --release
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

If you changed the contract's public interface (functions, types, errors), update
[CONTRACT.md](docs/src/CONTRACT.md) to match; it's meant to stay in sync with
`lib.rs`, not drift into a separate design doc.

## Reviewing PRs

Never accept a contract address a contributor provides as evidence their change
works, and never let one land in docs, examples, or code. A deployed address can't
be tied to a specific source commit without an independent rebuild: a PR's source
could be correct while the address offered alongside it points at different,
maliciously altered bytecode. If a change needs testnet verification, rebuild and
deploy it yourself (or have CI do it) from the PR's actual source; a pasted address
is never sufficient proof on its own. CI blocks any literal Stellar contract
address from being committed at all, as a backstop.

## Commit messages

One-line, imperative, conventional-commit style: `feat:`, `fix:`, `docs:`, `test:`,
`ci:`, etc., followed by a concise summary. No comma-separated lists of unrelated
changes in a single message; split them into separate commits instead.

## Opening a PR

CI (fmt, clippy, tests, wasm build) must pass before merge. The PR template
(`.github/pull_request_template.md`) is pre-filled when you open a PR; fill it out
rather than deleting it. If the change affects bond amounts, resolver behavior, or
anything with an economic consequence, say so explicitly in the summary so it's easy
to reason about from the PR alone.
