# Integrating with Tholos

For contracts that need a trustworthy resolution of a real world outcome and want
to call into Tholos rather than build their own propose/dispute/resolve logic. If
you're looking for the function-by-function reference instead, see
[CONTRACT.md](CONTRACT.md).

## Should you deploy your own instance, or share one?

Default to sharing the [canonical deployment](DEPLOYMENT.md#canonical-testnet-deployment).
Tholos is only trustworthy as an oracle if its resolver committee's track
record accumulates somewhere: one committee, one dispute history, building a
reputation over time. Fragmenting into a separate deployment per integrator
throws that away, each new instance starts with zero history and a
committee nobody's evaluated yet, which is no better than each integrator
building its own bespoke escrow logic instead of using Tholos at all.

Each deployment is initialized once with a single token, bond amount,
challenge window, and resolver committee (`initialize` in [CONTRACT.md](CONTRACT.md)),
with no per-call override, so a separate deployment is only justified when your
parameters are genuinely incompatible with the canonical one: a materially
different bond size for a much higher- or lower-value market, or a token the
canonical instance doesn't use. If that's not your situation, share the
canonical instance and just track the assertion `id`s that belong to you.

There is currently no built-in way for a calling contract to distinguish "its"
assertions from anyone else's within one instance beyond tracking the `id`s it
received back from `assert_outcome`. Store that mapping on your side (e.g.
`market_id -> assertion_id`).

## Calling Tholos from another Soroban contract

`contracts/demo-consumer` is a working, tested example of this, not just a
snippet: its `create_assertion` and `get_status` functions are the pattern below,
and its test deploys Tholos's actual compiled wasm and calls through it. If
anything here goes stale, that crate's `cargo test -p demo-consumer` is what
would catch it.

Import the client from the deployed contract's WASM and call it like any other
cross-contract invocation:

```rust
use soroban_sdk::{contractimport, Address, Env};

mod tholos {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32v1-none/release/tholos.wasm"
    );
}

fn create_assertion(env: Env, tholos_id: Address, asserter: Address, outcome: bool) -> u64 {
    let client = tholos::Client::new(&env, &tholos_id);
    client.assert_outcome(&asserter, &outcome)
}
```

`contractimport!` reads the wasm file **at your crate's compile time**, so it has
to already exist on disk before you build. In this repo that means running
`cargo build -p tholos --target wasm32v1-none --release` before touching
`demo-consumer` (see [CONTRIBUTING.md](CONTRIBUTING.md)); if Tholos is a separate
repo for you, the same constraint applies to wherever its wasm gets built.

### Who should be the `asserter`: your contract, or the end user?

This is the decision that has the most integration friction, and it's worth
getting right before you write the code.

**End user as asserter (what `demo-consumer` does, and the default recommendation).**
Pass through an `Address` the caller provides, as above. The user's own signature
authorizes `assert_outcome` and the underlying bond transfer directly; your
contract doesn't need any special auth plumbing. The tradeoff: because that
signature lives on an argument to *your* function rather than the top-level call,
if you're writing tests against this you need
`env.mock_all_auths_allowing_non_root_auth()` rather than plain `mock_all_auths()`
(see `demo-consumer/src/test.rs`), and on a real network the transaction needs an
authorization entry for that address alongside whatever signs the outer call.

**Your contract's own address as asserter.** `contracts/asserter-consumer` is a
working, tested example of this pattern, the same way `demo-consumer` is for the
simple one above: its `create_assertion_as_self` function is the pattern below,
and its test deploys Tholos's actual compiled wasm and calls through it without
mocking the nested authorization it depends on. Bonds pool under your contract's
control (e.g. to later distribute pro-rata to your own users) instead of going
directly to an end user. This is meaningfully harder than it looks: Tholos's
`assert_outcome` calls the underlying token's `transfer`, which itself calls
`require_auth()` on the asserter. That's *two* contract calls away from your
contract (yours -> Tholos -> token), and Soroban only auto-grants a contract's
implicit self-authorization one call deep. The deeper call fails with
`Error(Auth, InvalidAction)` unless you explicitly pre-authorize it with
[`env.authorize_as_current_contract`](https://docs.rs/soroban-sdk/latest/soroban_sdk/struct.Env.html#method.authorize_as_current_contract)
before invoking Tholos, specifying the exact token contract, `transfer` args, and
amount Tholos will end up calling. That means you need to already know Tholos's
configured token and bond amount to construct the right authorization, since
there's no way to ask Tholos for the sub-invocation it's about to make ahead of
time. Only take this path if pooling bonds under your contract is a real
requirement, not a default choice.

## Calling Tholos from a browser or Node app

The Rust pattern above only helps if you're writing another Soroban contract.
An application calling Tholos directly, from a browser or a Node backend,
needs the same building/simulating/signing/submitting/polling machinery
`demo-consumer` gets from `contractimport!`, but in TypeScript.

`packages/tholos-sdk` is a generated client for exactly this, produced with
the Stellar CLI's `stellar contract bindings typescript` against Tholos's
compiled wasm (not a live deployment, so generating it never needs network
access or a contract id). It's committed in-repo, not yet published to npm;
see its own [README](../../packages/tholos-sdk/README.md) for regeneration
instructions and current status.

```ts
import { Client } from "tholos-sdk";

const client = new Client({
  contractId: "<the deployed contract id, see DEPLOYMENT.md>",
  networkPassphrase: "Test SDF Network ; September 2015",
  rpcUrl: "https://soroban-testnet.stellar.org",
});

const tx = await client.assert_outcome({ asserter: "<address>", outcome: true });
const { result } = await tx.signAndSend();
```

`demos/freelance-escrow` doesn't use this yet, it currently hand-rolls its
own client (`src/lib/tholos.ts`) predating this package. Migrating it is a
separate, deliberate follow-up rather than bundled here, so the SDK's
completion doesn't depend on unrelated demo-app churn.

## Lifecycle from an integrator's perspective

`finalize` requires `caller`'s authorization unconditionally — even when
`finalize_reward_bps` is 0 (the default). This ensures the address written into
`Assertion.finalizer` and the `Finalized` event is always a verified caller, not an
arbitrary address someone passed in. No funds are at risk (the caller only ever
receives its own reward), but without enforced auth the on-chain finalizer of record
could be spoofed. Pass `caller = some_address` and authorize the call regardless of
whether a reward is configured. When `finalize_reward_bps` is non-zero the caller
additionally receives `bond * bps / 10_000` tokens as an incentive and
`Assertion.finalizer` is set to that verified address. `resolve` requires
authorization from a member of the resolver committee snapshotted for the
dispute. Tholos does
not push a callback to your contract when an assertion resolves. If you need to
react automatically, two options:

1. **Poll** `get_assertion_state(id)` after the challenge window you configured has
   elapsed, and act once `status` is `Resolved`.
2. **Watch events.** Every state transition emits an event (see the table in
   [CONTRACT.md](CONTRACT.md#events)); an off-chain indexer or keeper watching
   `Finalized`/`Resolved` for your tracked `id`s can call back into your contract
   once the outcome is final.

Either way, build your integration assuming resolution is not instant: it takes at
least the full challenge window, and longer if disputed and resolver votes trickle
in slowly.

## Reading the outcome

```rust
let state = client.get_assertion_state(&id);
match state.status {
    tholos::Status::Resolved => {
        // `final_outcome` is guaranteed to be set when status is Resolved.
        let final_outcome = state.final_outcome.unwrap();
    }
    _ => { /* not resolved yet */ }
}
```

`Assertion.outcome` always remains the claim made at assertion time. Read
`Assertion.final_outcome` for the authoritative result once the assertion is
resolved; it is `None` while the assertion is still `Pending` or `Disputed`.

## Parameters you're choosing when you initialize

| Parameter | Consideration |
| --- | --- |
| `token` | Any SEP-41 token. Must be a token your users already hold or can acquire; bonds are paid in it directly, there's no swap step. |
| `bond_amount` | High enough to deter spam/bad-faith assertions, low enough that legitimate use isn't priced out. Fixed per instance, see above. |
| `challenge_window_secs` | Longer windows give more time to catch bad assertions but delay uncontested finalization. |
| `resolvers` | Must be odd-length, non-zero, distinct, and at most 21 addresses; v1 rejects duplicates with `DuplicateResolvers`. See [CONTRACT.md](CONTRACT.md) for what `update_resolvers` can and can't change mid-dispute. |
| `finalize_reward_bps` | 0–1000 basis points of the bond paid to whoever calls `finalize`. Auth is always required from the caller, regardless of this value. 0 (default) returns the full bond to the asserter with no reward; non-zero values incentivize prompt finalization. |

## Tholos v2

Everything above this section is v1: the fixed-committee-vote contract in
`contracts/tholos`, deployed and stable. `contracts/tholos-v2` is a wholly
separate, stake-weighted contract (design in
[V2_RESOLUTION.md](V2_RESOLUTION.md)), never an upgrade of v1 in place; the
two run side by side rather than one replacing the other. See
[V2_MIGRATION.md](V2_MIGRATION.md) for the coexistence period specifically:
how to inventory a v1 deployment, when to cut new traffic over, and how to
retire v1 operationally once its accepted assertions have drained.

### Assertion identity changes

Because v1 and v2 are independent deployments, each with its own `NextId`
counter starting at 0, an assertion `id` is only unique *within* one
contract. Two different assertions, one on each deployment, can both be id
`0` at the same time. Once you integrate with both, track `(contract_id,
assertion_id)` as the pair that actually identifies an assertion, not the
bare `id` alone; `market_id -> (contract_id, assertion_id)` if you're
already storing a mapping per the advice above.

### Lifecycle at a glance

v2 splits what v1 does in one `resolve` call into a multi-phase flow: an
optimistic stage identical in shape to v1's, followed by a bounded
registration window and a commit-reveal vote open to any address willing to
post a bond, not just a fixed committee.

```text
assert_outcome -> [uncontested: finalize]
                -> [disputed: dispute -> register* -> reveal* -> resolve_outcome]
                       -> settle* (once per funded position)
                       -> withdraw* (once per address with a credit balance)
```

(`*` marks calls made once per participant, not once per assertion.)

| Function | What it does |
| --- | --- |
| `assert_outcome(asserter, outcome) -> u64` | Posts a bonded claim. Same shape as v1's. |
| `finalize(caller, id) -> bool` | Closes an uncontested assertion out once `challenge_window_secs` has elapsed. Same caller-auth-always-required rule as v1. |
| `dispute(disputer, id)` | Opens the registration window. `disputer`'s bond becomes the fixed disagreeing position; the asserter's existing bond becomes the fixed agreeing one. |
| `register(voter, id, amount, commitment)` | Any third-party address posts a bond and a salted commitment to its eventual side, without revealing it yet. Repeated calls from the same voter top up one position; the commitment can't change after the first deposit. |
| `reveal(voter, id, choice, salt)` | Discloses and verifies a registered position's side. Lazily closes registration and opens reveal on the first call after `registration_deadline`, permissionlessly. |
| `resolve_outcome(id) -> TerminalCause` | Permissionlessly closes reveal out once it's decided: a strict majority locked, everyone eligible revealed, or the deadline passed. Needed specifically for the case nobody's `reveal` call would otherwise trigger it (see "Known gaps" below). |
| `settle(id, address) -> i128` | Converts one position's share of the decided outcome into withdrawable credit. Permissionless: anyone may settle anyone's known position. Doesn't move tokens. |
| `withdraw(owner, id, destination) -> i128` | Pays out `owner`'s full credit balance to `destination` (any address, not necessarily `owner`). |
| `get_credit(id, address) -> i128` | Read-only lookup of a withdrawable credit balance. |
| `set_paused_v2(paused)` | Admin-only. Blocks new `assert_outcome` calls; unlike v1's pause, an already-active round keeps running (registration, reveal, settlement, withdrawal) even while paused. |
| `cancel_round(id)` | Admin-only, and only while paused. Cancels a round before any terminal outcome has locked, refunding every funded position its exact principal. See [V2_RESOLUTION.md](V2_RESOLUTION.md) for why this exists and what it deliberately can't do. |

Reading the outcome and reacting to state changes follows the same two
options as v1 (poll `get_assertion(id)` for `phase == Resolved`, or watch
events), just against `AssertionV2`'s fields (`terminal_cause`,
`final_outcome`) instead of v1's `Assertion.status`.

### Known gaps

- **No canonical v2 deployment yet.** Unlike v1 (see
  [DEPLOYMENT.md](DEPLOYMENT.md#canonical-testnet-deployment)), there's no
  shared, long-lived v2 instance to point at yet. Deploy your own for now,
  following the same parameter guidance as v1's deployment section, until a
  canonical one exists.
- **`packages/tholos-sdk` targets v1 only.** The generated TypeScript client
  described above is built from `contracts/tholos`'s wasm, not
  `contracts/tholos-v2`'s. A browser or Node app integrating with v2 today
  needs its own `contractimport!`-equivalent tooling or hand-rolled calls
  until v2 gets its own generated bindings.
- **`demos/freelance-escrow` still talks to v1.** Migrating it to v2 is a
  separate follow-up, not bundled with the rest of the v2 work.

## Known caveats for integrators

- Finalize always requires caller's authorization: `caller.require_auth()` is
  called unconditionally, regardless of `finalize_reward_bps`. Pass a real
  address and sign the call. When `finalize_reward_bps` is non-zero (0–1000
  basis points of the bond, set once at `initialize` time), the caller also
  receives `bond * bps / 10_000` tokens as an incentive for prompt
  finalization. When it is 0 (the default), no reward is paid and the full
  bond is returned to the asserter, but auth is still required to keep the
  recorded finalizer trustworthy.
- The admin can pause `assert_outcome`, `dispute`, `resolve`, and `finalize` at
  any time via `set_paused`. Your integration should treat a `Paused` error as a
  distinct, expected failure mode (surface it to the user as "resolution
  temporarily unavailable") rather than an unexpected error. `update_resolvers`
  stays callable while paused. A pending assertion whose challenge window elapses
  while paused does not become finalizable until unpaused; do not assume a pause
  only affects new assertions and disputes.
