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
replaceable via `update_resolvers`. Must have an odd, non-zero length.

**Majority**
`resolvers.len() / 2 + 1`. The number of matching votes needed to resolve a
disputed assertion. Always achievable and never ambiguous because the committee
is odd-length.

**Finalize**
Closing out a `Pending` assertion after its challenge window has elapsed with no
dispute. Callable by anyone. Returns the asserter's bond.

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
