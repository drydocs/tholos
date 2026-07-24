# Glossary

**Assertion**
A claim about an outcome, posted with a bond via `assert_outcome`. Identified by a
`u64` id. See the `Assertion` type in [CONTRACT.md](CONTRACT.md).

**Asserter**
The address that posted an assertion. Receives the bond back if the assertion
finalizes uncontested, or if a resolver majority agrees with them after a dispute.

**Bond**
The amount of the configured token an asserter or disputer must post to make a
claim. Fixed per contract instance at `initialize`. Exists to make bad-faith
assertions and disputes costly.

**Challenge window**
The time period (in seconds, from `opened_at`) during which a `Pending` assertion
can be disputed. Fixed per contract instance at `initialize`.

**Disputer**
The address that disputed a `Pending` assertion within its challenge window,
matching its bond. Receives both bonds if a resolver majority disagrees with the
original asserter.

**Resolver**
An address in the resolver committee, entitled to vote on `Disputed` assertions
via `resolve`.

**Resolver committee**
The full set of resolvers for a contract instance, set at `initialize` and
replaceable via `update_resolvers`. It must be non-empty, have an odd number of
members, contain distinct addresses, and have no more than `MAX_RESOLVERS` (21)
members. Duplicate addresses are rejected with `DuplicateResolvers`.

**Majority**
`resolvers.len() / 2 + 1`. The number of matching votes needed to resolve a
disputed assertion, calculated against the resolver committee snapshotted when
the dispute opens. An odd-length committee makes the numeric threshold
unambiguous; reaching it still requires enough available resolver addresses.

**Finalize**
Closing out a `Pending` assertion after its challenge window has elapsed with no
dispute. `caller` must authorize the call. Returns the asserter's bond, minus an
optional reward paid to `caller` if `finalize_reward_bps` is non-zero.

**Finalizer**
The address that called `finalize` on an assertion. Recorded in
`Assertion.finalizer` and the `Finalized` event. Auth is required unconditionally,
so this is always a verified address.

**Finalize reward**
The optional cut of the bond (`finalize_reward_bps`, 0–1000 basis points, set at
`initialize`) paid to whoever calls `finalize`, as an incentive for prompt
finalization. 0 disables it entirely.

**Resolve**
Casting one resolver's vote on a `Disputed` assertion. Once a majority agrees,
the winning side receives both bonds and the assertion moves to `Resolved`.

**Pause**
An admin-controlled switch (`set_paused`) that blocks new assertions, disputes,
and resolver votes, without affecting `finalize` or `update_resolvers`. See
[ARCHITECTURE.md](ARCHITECTURE.md#pause-is-scoped-not-absolute).

**SEP-41**
The Stellar Ecosystem Proposal defining the standard token interface Soroban
contracts use (`transfer`, `balance`, etc.). Tholos's `token` parameter must
implement it.

**SAC (Stellar Asset Contract)**
The built-in Soroban contract wrapping a classic Stellar asset (like native XLM or
a Stellar-issued USDC) so it can be used as a SEP-41 token. What
`scripts/testnet-smoke.sh` uses for its bond token.
