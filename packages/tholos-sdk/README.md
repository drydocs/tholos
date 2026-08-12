# tholos-sdk

A generated TypeScript client for [Tholos](../../README.md), the bonded
assertion and dispute contract. Generated with the Stellar CLI's
`stellar contract bindings typescript`, so most of the manual
transaction-building, simulate/sign/submit/poll-for-result boilerplate a
JS/TS integrator would otherwise hand-roll (see
[docs/src/INTEGRATION.md](../../docs/src/INTEGRATION.md)) is already handled.

## Status

Committed in-repo, not published to npm. This package is new and hasn't been
consumed by a real integration yet (the actual migration of
`demos/freelance-escrow` off its hand-rolled client is a separate,
deliberately out-of-scope follow-up). Publish it once it's actually proven
out end to end, rather than before.

## Regenerating

The bindings are generated from `contracts/tholos`'s compiled wasm, not from
a live deployment, so regenerating never needs network access or a contract
id. Generate into a scratch directory rather than pointing `--output-dir`
at this package: the CLI overwrites the whole target directory, including
`package.json` and this README, not just the generated client, so
generating straight into `packages/tholos-sdk` would clobber both. Copy
back only `src/`, the same thing CI's drift check does:

```sh
cargo build -p tholos --target wasm32v1-none --release
stellar contract bindings typescript \
  --wasm target/wasm32v1-none/release/tholos.wasm \
  --output-dir /tmp/tholos-sdk-regenerated \
  --overwrite
cp -r /tmp/tholos-sdk-regenerated/src/. packages/tholos-sdk/src/
```

Regenerate whenever `contracts/tholos`'s public interface changes, and
re-run `pnpm install && pnpm build` in this directory afterward to confirm
it still compiles.

## Using it

```sh
pnpm install
pnpm build
```

```ts
import { Client } from "tholos-sdk";

const client = new Client({
  contractId: "<the deployed contract id, see docs/src/DEPLOYMENT.md>",
  networkPassphrase: "Test SDF Network ; September 2015",
  rpcUrl: "https://soroban-testnet.stellar.org",
});

const tx = await client.assert_outcome({ asserter: "<address>", outcome: true });
const { result } = await tx.signAndSend();
```

No `networks` constant is exported (unlike some generated packages): these
bindings were generated from the wasm directly, not from a live
`--contract-id`, on purpose, so no contract address is baked into committed
source. Same reasoning as `demos/freelance-escrow`'s own config: contract
addresses are supplied at runtime, never committed (see CONTRIBUTING.md).
