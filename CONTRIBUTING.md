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
      lib.rs             Cross-contract call pattern from INTEGRATION.md
      test.rs            Validates that pattern against Tholos's real compiled wasm
scripts/
  testnet-smoke.sh      End-to-end check against real Stellar testnet infrastructure
.github/workflows/
  ci.yml                 Runs fmt, clippy, tests, and the wasm build on every push/PR
```

`demo-consumer` exists to keep INTEGRATION.md honest: it's not a product, it's a
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

`book/` is an [mdBook](https://rust-lang.github.io/mdBook/) that publishes this
repo's markdown docs as a site, deployed automatically from `main` by
`.github/workflows/docs.yml`. It does not contain its own content: `book/src/*.md`
files are one-line `{{#include}}`s pointing at the real files at the repo root
(`README.md`, `CONTRACT.md`, `INTEGRATION.md`, this file), so there's a single
source of truth. Edit the root file, not the file under `book/src/`.

Preview locally with `mdbook serve book` (requires `cargo install mdbook`).

## Before opening a PR

Run the same checks CI runs, in this order (see the note above on why the wasm
build has to come first):

```sh
cargo fmt --check
cargo build -p tholos --target wasm32v1-none --release
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

If you changed the contract's public interface (functions, types, errors), update
[CONTRACT.md](CONTRACT.md) to match; it's meant to stay in sync with `lib.rs`, not
drift into a separate design doc.

## Commit messages

One-line, imperative, conventional-commit style: `feat: `, `fix: `, `docs: `, `test: `,
`ci: `, etc., followed by a concise summary. No comma-separated lists of unrelated
changes in a single message; split them into separate commits instead.

## Opening a PR

CI (fmt, clippy, tests, wasm build) must pass before merge. The PR template
(`.github/pull_request_template.md`) is pre-filled when you open a PR; fill it out
rather than deleting it. If the change affects bond amounts, resolver behavior, or
anything with an economic consequence, say so explicitly in the summary so it's easy
to reason about from the PR alone.
