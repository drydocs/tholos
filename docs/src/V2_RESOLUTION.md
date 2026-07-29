# Stake-weighted resolution (protocol v2 proposal)

> **Status:** Proposed; design-only and not implemented.
>
> **Tracking:** [Issue #19](https://github.com/drydocs/tholos/issues/19).
>
> **Versioning:** "v1" and "v2" in this document name protocol designs. They
> are independent of the Rust crate's current `0.2.0` package version.

This document proposes replacing v1's fixed, admin-selected resolver committee
with a vote whose electorate and weight come entirely from token bonds locked on
the dispute being decided. It records a recommendation for review; it does not
change the current contract, public interface, or deployment behavior.

## Decision summary

| Question | Proposed answer |
| --- | --- |
| Who may vote? | An address with a positive resolution position funded before that dispute's registration cutoff. The asserter and disputer are included through their existing bonds. |
| How much weight does it have? | One unit of voting weight per smallest token unit locked in that address's position: `weight(address) = locked_bond(address)`. Multiple deposits by one address are aggregated. |
| When is weight fixed? | Resolution policy is pinned when the assertion opens. Positions and total eligible weight are frozen when the dispute's registration phase closes, before any discretionary third-party choice is revealed. |
| What replaces `Assertion.resolvers`? | A per-dispute policy snapshot, an aggregate eligible-weight snapshot, and per-address `Position` records. No voter vector is copied or iterated. |
| What decides the result? | A side becomes mathematically irreversible once it has strictly more than half of all snapshotted eligible weight. If neither side does by the reveal deadline, the asserted outcome stands as the explicit optimistic default. Settlement waits until reveals close. |
| What happens to bonds? | After a strict-majority result, winning positions recover principal and share losing plus non-revealed stake. After an optimistic timeout default, all revealed positions recover principal and share only non-revealed stake. Permissionless O(1) settlement accrues owner-withdrawable credits; it never loops over all voters. |
| How do v1 deployments migrate? | Blue/green deployment: send only new assertions to a new v2 contract and attempt to drain each resolvable v1 assertion under the exact rules of its deployed WASM. Do not reinterpret or transfer in-flight v1 bonds; v2 cannot rescue an unresolvable v1 dispute. |

The timeout default and its separate settlement rule are economically material
and remain the two highest-priority review points before an implementation issue
is opened.

## Why v2 needs more than a weighted `resolve`

In v1, an asserter posts one fixed bond and a single disputer matches it. Those
are the only two bond posters. Giving only those two addresses voting weight
would create two equal positions with opposing incentives, so every honest
dispute would begin in a structural tie.

V2 therefore needs a bounded registration tier between `dispute` and voting.
The two parties can increase their positions and additional addresses can lock
capital specifically against that dispute during this phase. Once registration
closes, neither deposits, configuration changes, nor an administrator can change
that round's electorate or denominator.

This preserves the important invariant behind v1's resolver snapshot while
removing the fixed committee: the rules of an in-flight decision cannot change
after voting begins.

## Goals

- Scope both eligibility and influence to capital actually at risk on one
  dispute, rather than unrelated token holdings or an administrator's list.
- Make address splitting economically neutral: splitting one bond across many
  addresses must not increase its combined weight.
- Freeze policy, voters, and weights before discretionary third-party choices are
  revealed. The asserter and disputer sides are public by construction.
- Reach a deterministic terminal result by a bounded deadline, then allow
  permissionless O(1) position settlement and owner-authorized credit withdrawal.
- Preserve exact escrow accounting and make every token interaction reentrancy-safe.
- Keep storage and every call bounded without iterating an open-ended electorate.
- Give existing v1 deployments a cutover path that preserves their exact rules
  and custody without pretending v2 can rescue already stranded bonds.

## Non-goals and trust assumption

This mechanism does not prove an external fact, provide one-human-one-vote,
prevent bribery, or remove the need for evidence and off-chain coordination. It
also does not eliminate wealth concentration: an address or coalition controlling
more than half of the eligible bonded capital can determine the result.

The outcome rule has an explicitly asymmetric security assumption:

> A false assertion is corrected only if revealed weight against it becomes
> strictly greater than half of all eligible weight before the deadline. A true
> assertion remains unless incorrect revealed weight against it crosses that same
> threshold. For outcome purposes, every non-reveal favors the assertion.

This is narrower than a generic token-holder vote because every unit of influence
must be transferred into the contract, locked for the dispute, and assigned to a
position whose protocol-level payout depends on the result. It is not an
unconditional truth guarantee. A winning majority recovers its capital, and a
beneficial owner can self-hedge through multiple addresses, so nominal bonded
weight is not the same as guaranteed economic loss. Integrators must model bonds
relative to the external value that a false result could capture.

The practical cost of corruption is set by independent counter-stake present or
able to enter before the hard cutoff, not by the base bond alone. Once a coalition
has an irreversible majority, its winning positions are no longer economically
at risk from this dispute.

The proposal also does not specify final Rust types or function names. Those
belong in a later implementation issue after the economics are accepted.

## Lifecycle and the single weighted round

This proposal has an optimistic stage followed by at most one weighted round:

1. **Optimistic assertion.** The asserter posts the deployment's base bond. If no
   one disputes within the challenge window, `finalize` behaves as it does in v1.
2. **Dispute-scoped resolution.** A distinct disputer matches the base bond. A
   bounded registration period lets other addresses post resolution bonds, then
   the frozen set of bond posters decides the result by stake weight.

It does not specify recursive appeals or repeated stake rounds. A later,
more-highly-bonded tier remains a review question; adding one would require
separate round snapshots, deadlines, and a maximum round count.

```mermaid
stateDiagram-v2
    [*] --> Pending: assert_outcome + asserter bond
    Pending --> Resolved: challenge window ends uncontested
    Pending --> Registration: dispute + matching bond
    Registration --> Reveal: cutoff; freeze positions and total weight
    Reveal --> OutcomeLocked: either side exceeds 50% of eligible weight
    Reveal --> Resolved: reveal deadline; asserted outcome is default
    Reveal --> Resolved: all weight revealed; tie; asserted outcome is default
    OutcomeLocked --> Resolved: reveal deadline or all weight revealed
    Resolved --> [*]: position settlement, then credit withdrawal
```

Every deadline is derived from the policy pinned to the assertion. A caller may
advance an expired phase permissionlessly; progress cannot depend on an admin or
on one designated account remaining online.

## Policy is pinned when the assertion opens

V1 snapshots the live committee when `dispute` is called. That protects an open
vote, but an admin can still change the future decision-maker after the asserter
has committed its bond and before a dispute arrives. V2 should close that gap.

Each assertion stores a complete, immutable `PolicySnapshot` when it is created.
A version and canonical hash identify and authenticate that snapshot, but neither
is a substitute for storing the values the contract must execute. At minimum,
the snapshot covers:

- configured token and base bond;
- minimum third-party resolution bond;
- registration duration, any anti-sniping extension, and its hard maximum;
- reveal duration;
- weight-rule version (`LinearStakeV1` in this proposal);
- strict-majority threshold and optimistic timeout default;
- forfeiture and payout-rule version;
- maximum position and total eligible stake supported by checked tally and payout
  arithmetic; and
- maximum active resolution horizon and initial settlement/withdrawal grace used
  to size storage TTLs.

Admin changes, if future versions allow them at all, apply only to assertions
created after the change. No policy update may alter an already opened assertion.
Copying this small, bounded snapshot into the assertion avoids a mutable lookup
and avoids giving a separate policy record its own archival dependency.
`policy_hash` is the specified cryptographic hash of the versioned, canonical
encoding of those stored values.

This proposal retains v1's one-token-per-deployment model: the token address is
immutable for the lifetime of a v2 contract. The policy snapshot binds each
assertion to that token explicitly. A future multi-token deployment would instead
have to include the token in every credit key and liability aggregate.

## Registration and voter eligibility

Calling `dispute` starts a registration period and creates two initial positions:

- the asserter's existing base bond is fixed in support of the asserted outcome;
- the distinct disputer's matching bond is fixed against the asserted outcome.

Treating those initiating actions as fixed ballots avoids requiring either party
to come back online merely to restate the position its bond already expresses.
V2 should reject an address disputing its own assertion. This cannot prevent the
same owner from using another address, but it prevents one storage position from
occupying both protocol roles.

During registration, the asserter and disputer may top up their fixed-side
positions, and any other address may create one dispute-specific position. Every
new position or top-up transfers at least `minimum_resolution_bond` into escrow.
The recommended default is the assertion's base bond; allowing a one-unit top-up
or third position would make an equal asserter/disputer tie too cheap to break.

An external poster supplies a salted commitment to its eventual
`agrees_with_asserter` choice with the deposit. Its side remains hidden until the
reveal phase, while its amount is visible and economically committed. The
commitment must domain-separate at least:

```text
H(canonical_encode(
    "THOLOS_V2_VOTE", network_id, contract_address, policy_hash,
    assertion_id, round, voter, choice, salt_32
))
```

`H` must be a specified cryptographic hash, the encoding must be canonical and
length-delimited, and the 32-byte salt must be generated with enough entropy to
prevent brute-forcing a commitment whose only secret payload is a boolean.

Registration follows these rules:

- A position is keyed by `(assertion_id, address)` and is non-transferable.
- Repeated deposits from one address aggregate into one amount and therefore one
  vote. Asserter/disputer top-ups stay on their fixed sides; an external top-up
  may not replace the position's original commitment.
- Every amount is positive, denominated in the configured token's smallest unit,
  authorized by its poster, and transferred into escrow before it can be used as
  active weight.
- A deposit is rejected atomically before acquiring weight if the new position,
  eligible total, or worst-case settlement arithmetic would exceed its pinned
  bound or numeric type. A failed transfer leaves neither position nor weight.
- A poster cannot withdraw or reduce its position after funding it. It exits only
  through settlement.
- New positions and top-ups stop at the registration cutoff. A bounded
  anti-sniping extension may move the soft cutoff when a qualifying bond arrives
  near the end, but never past the snapshotted hard deadline.
- The contract maintains the eligible total incrementally as deposits arrive; it
  never discovers participants by scanning storage.

The exact registration and extension durations are deployment parameters, not
universal constants. Economic simulation should set them before implementation.
Clients must read an on-chain transition to `Reveal` before transmitting a choice
and salt. Guessing that the soft cutoff has passed is unsafe: a rejected reveal
transaction still publishes its preimage while an extension may leave
registration open.

## Voting weight

For address `i` with `s_i` token units locked at the cutoff:

```text
w_i = s_i
W   = sum(w_i) for all eligible positions
```

`W` is frozen before reveals begin. A position's amount cannot change afterward,
and token balances held elsewhere are irrelevant.

Linear weight is deliberate:

- splitting stake `s` across addresses leaves combined weight equal to `s`;
- combining deposits under one address produces the same result;
- weight is backed by an escrowed quantity the contract can verify; and
- the arithmetic and economic exposure are auditable.

One-address-one-vote, per-address caps, square-root weight, and other concave
rules all increase combined influence when a participant splits capital across
pseudonymous addresses. Without a separate Sybil-resistant identity system,
those rules create the appearance of limiting whales while making address
splitting profitable. Token-balance snapshots are also rejected: they are not
dispute-scoped and can admit borrowed or otherwise uncommitted voting power.

Linear weight limits a large holder to proportional, not superlinear, influence;
it does not stop that holder from dominating a small dispute. That residual risk
must be stated plainly rather than hidden behind an identity-dependent formula.

## Replacing `Assertion.resolvers`

V2 replaces the committee vector with two snapshots serving different purposes:

1. **Policy snapshot at assertion creation:** fixes how a future dispute will be
   funded, voted, timed, and settled.
2. **Eligibility snapshot at registration close:** freezes each funded position
   and the aggregate `W` used as the denominator.

The eligibility snapshot is logical, not a copied vector. A conceptual storage
layout is:

| Record | Purpose |
| --- | --- |
| `AssertionV2(id)` | Claim, parties, lifecycle, complete `PolicySnapshot`, and authoritative final outcome. |
| `Resolution(id)` | Phase, deadlines, frozen `W`, weighted tallies, immutable terminal cause, settlement class/aggregates, and rule version. Terminal cause distinguishes `StrictMajorityFor`, `StrictMajorityAgainst`, and `OptimisticTimeout`. |
| `Position(id, address)` | Escrowed amount, position kind, revealed side, and settlement state for one address. The kind is either protocol-fixed with a side (asserter/disputer) or external with a commitment hash. |
| `Credit(id, address)` | Dispute-scoped token liability accrued by permissionless position settlement and withdrawable by its owner to an authorized destination. Keeping the assertion ID in the key preserves per-dispute accounting when one owner participates in several disputes. |

The proposed design must not place an unbounded `Vec<Address>` inside the
assertion. The current cap of 21 resolvers exists precisely because v1 copies and
iterates that vector. Per-position keys plus aggregate totals keep registration,
reveal, finalization, position settlement, and credit withdrawal O(1) in the
number of posters.

Every persistent record needs a TTL policy covering the full active lifecycle and
an explicit archival/restoration path for later withdrawals. Updating the
assertion alone is not sufficient to keep separate `Position` entries live.

This proposal does not confiscate an entitlement after a withdrawal deadline. A
position is created with TTL covering the maximum active-phase horizon plus a
pinned settlement/withdrawal grace period; known positions can then be bumped or
settled permissionlessly. If a persistent record is archived later, its assertion,
resolution, position, and credit footprints must be restored before settlement or
withdrawal. The liability remains until paid. Events and an off-chain index are
therefore part of the recovery path. This makes the outcome deadline bounded, not
the time at which every owner actually receives funds, and it deliberately avoids
an admin sweep of unclaimed property.

Because these keys are intentionally not enumerable on-chain, v2 must emit
indexable events for position funding/top-ups, eligibility freeze and `W`, every
reveal, outcome lock/finalization, position settlement/credit accrual, and credit
withdrawal. These events are required for keepers and historical indexers, not an
optional observability enhancement.

## Reveal, majority, and timeout

After registration closes, the asserter and disputer weights are already tallied
on their protocol-fixed sides and included in `revealed_weight` exactly once. An
external voter reveals `choice` and `salt`.
The contract authenticates that address, verifies its frozen positive position
and commitment, rejects a second reveal, and adds exactly the frozen position
amount to one tally.

Let `F` be weight agreeing with the asserter and `A` weight against it. A side has
an absolute majority when:

```text
F > W / 2     or     A > W / 2
```

Implementation should use checked integer arithmetic and an overflow-safe
comparison such as `side_weight > W - side_weight`, not unchecked doubling.
Because the denominator includes non-revealed positions, a small fraction of
turnout cannot actively overturn a much larger frozen stake pool. This is not a
symmetric quorum guarantee: under the timeout rule below, abstention ultimately
favors the asserted outcome.

The outcome is safe to lock as soon as either side crosses the threshold;
unrevealed weight can no longer reverse it. Reveals nevertheless remain open
until their deadline so positions committed to the winning side can prove their
entitlement and avoid being treated as non-reveals. Settlement opens only after
the deadline. It may open earlier if `revealed_weight == W`; when all weight is
revealed but the tallies are tied, the optimistic default is already irreversible
and can be applied immediately.

If neither side crosses the threshold before the reveal deadline, the asserted
outcome stands. This optimistic default is recommended because it:

- gives every dispute a bounded terminal result;
- does not restore an admin or committee as a tie-breaker;
- makes a challenger bear the burden of assembling more bonded weight against an
  assertion; and
- is consistent with the existing rule that an uncontested assertion finalizes
  as stated.

This asymmetry is broader than tie-breaking. For example, if 1% of eligible
weight reveals for the assertion, 49% reveals against, and 50% does not reveal,
the assertion still stands because neither revealed side exceeded half of `W`.
Non-revealed weight is therefore functionally delegated to the status quo for the
outcome, even though its positions are penalized in settlement.

The cost is real: an evenly split dispute favors the asserter. The principal
alternative is a terminal `Inconclusive` state that delegates fallback to the
integrator. That avoids asserting truth on a tie, but breaks the promise that
Tholos returns a boolean and moves resolution complexity outside the protocol.
This choice must receive explicit maintainer approval before implementation.

Outcome and settlement are separate decisions. A timeout default has no bonded
majority, so it must not label every position against the assertion as a losing
vote. Under the recommended timeout settlement, all revealed positions on both
sides recover principal and share only the non-revealed pool pro rata. With only
the two equal initiating bonds, the assertion therefore stands but both bonds are
returned. A lone challenger pays fees, capital lock, and bounded delay rather than
donating its entire bond. If that is insufficient protection against frivolous
disputes, v2 needs a separately modeled non-refundable dispute fee; it should not
silently reuse majority slashing for a result that no majority chose.

## Bond settlement

Voting weight with unconditional refunds would let a large holder dictate a
strict-majority result while paying only temporary illiquidity. Settlement
depends on the terminal cause.

For a result locked by strict majority:

- Positions on the decided side recover principal.
- Losing positions and positions that fail to reveal are forfeited.
- The forfeited pool is distributed among winning positions in proportion to
  their weight.

For an optimistic timeout default without a strict majority:

- Every revealed position, agreeing or disagreeing, recovers principal.
- Only non-revealed positions are forfeited.
- That non-revealed pool is distributed among all revealed positions pro rata.

In both cases, settlement separates permissionless accounting from the external
token transfer. Any caller may settle a known position: the contract marks that
position settled and accrues its entitlement to the stored owner's `Credit`
without calling the token. The owner later authorizes withdrawal of that credit
to a chosen address. A receiver that rejects the token can therefore delay only
its own withdrawal, not another position's accounting or dust.

This is a protocol-level forfeiture, not proof of a beneficial owner's net loss.
A coalition that also controls reward-recipient positions recovers part or all of
value forfeited by its losing/non-revealed addresses, and a coalition already
above the majority threshold knows its winning stake will not be slashed. The
mechanism creates contestable exposure during registration; it does not make
capture cost equal to the attacker's nominal deposits.

For each reward-recipient position, the reward is fixed when settlement opens:

- `recipient_weight` is final winning-side weight after a strict majority; or
- `recipient_weight` is total revealed weight after a timeout default.

```text
reward_i = floor((s_i * forfeited_pool) / recipient_weight)
payout_i = s_i + reward_i
```

Integer implementation must conserve the exact escrow and avoid overflow in the
conceptual multiplication above. Every position settlement uses the original
`forfeited_pool` and `recipient_weight`, so its result is independent of settlement
order. Each position can immediately accrue principal plus that fixed reward. The
contract tracks unsettled recipient weight and distributed rewards.

Once recipient weight reaches zero, a permissionless dust settlement accrues the
exact undistributed remainder to a deterministic initiating party: the winning
asserter/disputer after a strict-majority result, or the asserter after a timeout
default. Only this indivisible dust, not that party's principal or pro-rata
reward, waits for other positions to settle. If `forfeited_pool == 0`, no dust
operation is needed.

This deterministic rule is O(1) per position, conserves the entire pool, and
prevents call order from changing base entitlements. It does give one initiating
party all indivisible remainder units; that explicit trade-off is preferable to
caller-selected ordering and must be property-tested. The multiply/divide itself
must be full-precision or operate under a validated bound that cannot overflow.

Credit withdrawal reduces that dispute's stored credit and outstanding liability,
and increases its withdrawn total, before the outgoing token transfer; a failed
transfer atomically restores all three. Incoming deposits
need an explicit reentrancy guard: a position must not be usable by a
permissionless cutoff or settlement callback while its token transfer is still
executing. The guard is entered before the external call and rolled back with the
transaction on transfer failure; the position becomes active only as part of a
successfully funded operation. Total withdrawals and credits for one dispute must
never exceed its funded positions, even if a token contract calls back into
Tholos. The authoritative `terminal_cause` and `final_outcome` are stored in v2
state as well as emitted, avoiding v1's sharp edge where `Assertion.outcome`
always remains the original claim.

### Example

With a base bond of 100 units:

- asserter: 100 agreeing;
- disputer: 100 against;
- voter A: 60 against; and
- voter B: 40 agreeing.

The frozen total is `W = 300`. Weight against is 160, strictly greater than 150,
so the assertion is overturned. The 160 units on the winning side recover their
principal and share the 140-unit forfeited pool pro rata. Address splitting would
not change `W` or either side's weight, though it can change smallest-unit dust
allocation under the deterministic rule above.

## Administration and pause semantics

V2 has no global resolver membership and no `update_resolvers` equivalent for v2
assertions. The admin cannot insert a voter, remove one, alter weight, or change a
pinned policy.

An emergency pause must not selectively censor a time-critical step of an
already funded assertion. In particular, blocking `dispute` while allowing
`finalize`, or blocking `reveal` while its deadline continues, can choose a winner
administratively. The recommended v2 pause blocks new assertions only. Dispute,
registration, reveal, finalization, settlement, and withdrawals for already
accepted assertions remain available. A stronger emergency mechanism would need
to freeze every affected deadline symmetrically or cancel the round with deterministic refunds;
that is a separate design and audit surface. Creation-only pause preserves voting
neutrality but cannot contain an exploit in registration, reveal, settlement, or
withdrawal, so this trade-off is blocking security review rather than a settled
operational detail.

## Required invariants

A future implementation and its tests must establish all of the following:

1. A position contributes weight only after the same amount is escrowed; a failed
   deposit leaves no position or weight.
2. At the cutoff, `W` equals the sum of all position amounts, with each initiating
   base bond represented exactly once.
3. For every dispute and after every settlement or withdrawal,
   `funded_total == withdrawn_total + accrued_credit + unsettled_entitlements`;
   `accrued_credit` is the outstanding sum of `Credit(id, *)`, so a withdrawal
   moves value from it to `withdrawn_total`. Across disputes, the contract's token
   balance is at least the sum of outstanding credit and unsettled entitlements;
   no cross-dispute or unsolicited balance is treated as available surplus.
4. Policy cannot change after assertion creation.
5. Position amount, eligibility, and `W` cannot change after the cutoff.
6. Each address has one aggregated position and at most one revealed choice.
7. Fixed asserter/disputer ballots enter both their tally and `revealed_weight`
   exactly once; at all times, `agree_weight + disagree_weight <= W`.
8. Phase transitions are monotonic and idempotent. No post-cutoff transition
   accepts a position or top-up.
9. No side locks an outcome early without strictly more than half of `W`, and
   neither `terminal_cause` nor `final_outcome` changes after it is locked.
10. After the deadline, the optimistic default and its distinct settlement class
    are deterministic; settlement and owner-authorized withdrawal need no admin.
11. A position accrues credit at most once to its stored owner, and deterministic
    dust accrues at most once to its pinned recipient. A failed credit withdrawal
    rolls back without consuming the credit.
12. Registration, reveal, result calculation, position settlement, and withdrawal
    are O(1); none loops over all positions.
13. Every token-moving path is reentrancy-safe. Outgoing withdrawal uses
    effects-before-interactions so a callback cannot spend the same credit twice;
    incoming stake cannot become usable during its transfer and rejects callbacks
    that could observe or act on a partially funded position.
14. Deposits that exceed a pinned or numeric bound are rejected. All additions,
    threshold comparisons, and pro-rata payouts use checked,
    conservation-preserving arithmetic.
15. Assertion, resolution, position, and credit TTLs cover every active phase and
    initial settlement/withdrawal grace; archived entitlements remain restorable.
16. Pausing new assertions does not alter a deadline or action available to an
    already accepted assertion.

## Threat analysis

| Threat | Proposed control | Residual risk |
| --- | --- | --- |
| Address splitting / Sybil voting | Linear weight and one aggregated position per address. | Splitting identities remains possible but provides no extra weight. |
| Committee/admin capture | No resolver list; both snapshots are dispute-local and immutable. | Admin still controls any powers retained outside resolution, such as deployment configuration. |
| Borrowed or flash voting power | Tokens are transferred and locked across registration and reveal, not read from a wallet balance. | Longer-duration borrowed capital remains possible and carries the same economic risk as owned capital. |
| Last-moment stake | Fixed cutoff plus a bounded anti-sniping extension and hard deadline. | A sufficiently funded late entrant can still dominate before the hard deadline. |
| Vote copying / tactical side selection | Salted third-party commitments are funded before those discretionary sides are revealed. | Initiating sides are public; off-chain disclosure and bribery cannot be prevented. |
| Double voting or staking both sides | One immutable commitment and one position per `(id, address)`. | A participant can use multiple addresses on both sides. Linear weight gives no extra aggregate influence, but self-hedging can reduce its net economic exposure and must be modeled. |
| Abstention to block majority | Non-revealed weight remains in `W`, its position is forfeited, and the assertion wins at timeout; all revealed positions share the forfeiture. | Abstention is effectively delegated to the status quo. A coalition with revealed addresses can recycle part of the forfeiture, so its net cost may be below the nominal bond. |
| Storage DoS | Minimum bond, per-address records, incremental aggregates, no voter vector or loop. | Many fully funded positions still consume storage and must be load-tested. |
| Arithmetic, reentrancy, or payout drain | Checked sums, bounded totals, permissionless credit accrual, liability invariants, outgoing effects before withdrawals, and a deposit guard. | Requires property, adversarial-token, and high-boundary tests plus audit. |
| Ambiguous real-world claim | Bind each v2 assertion to an immutable market/question identifier or content hash. | Evidence availability and interpretation remain off-chain concerns. |

The final row identifies an adjacent design dependency, not a decision approved
by this issue. V1 stores only a boolean and expects the integrator to map its
market to an assertion ID. An open electorate needs an unambiguous immutable
reference to the proposition and resolution rules before posting bonds. A
follow-up design must decide whether v2 commits an on-chain identifier or relies
on a versioned integrator registry; the evidence format itself can remain
off-chain.

## Alternatives considered

| Alternative | Why it is not recommended |
| --- | --- |
| Give the asserter and disputer one weighted vote each | Their required bonds are equal, producing a structural tie. |
| Count only revealed weight | A tiny turnout could decide a large eligible pool; strategic abstention changes the effective denominator. |
| One address, one vote | Pseudonymous address splitting creates voting power at negligible cost. |
| Cap or square-root each address's weight | Without identity, splitting a bond across addresses bypasses the cap or increases total concave weight. |
| Weight current wallet/token balance | Influence is not dispute-scoped or necessarily at risk and may be borrowed at snapshot time. |
| Keep weights live during voting | Deposits can change the denominator and required majority after votes are known, recreating the mutable-snapshot bug. |
| Let the admin committee break ties | Restores the centralization v2 is intended to remove and makes the fallback the real authority. |
| Require a supermajority | Raises manipulation cost but materially increases defaults and capital-locking grief; it needs evidence before replacing strict majority. |
| Return `Inconclusive` on timeout | Semantically safer on a tie, but pushes a second oracle/fallback into every integrator and no longer guarantees a boolean result. |

## Migration from existing v1 deployments

Existing v1 contracts cannot be converted in place. They expose no WASM upgrade
entry point, state importer, escrow transfer, or administrative withdrawal.
Changing the Rust crate later does not add those capabilities to already deployed
bytecode.

The migration is therefore blue/green. Cutover to v2 does not guarantee that v1
can be fully drained: v1 has no timeout or cancellation for an unresolvable
dispute, so dual operation and trapped v1 escrow may be permanent.

### 1. Inventory v1

For each deployment, record the network, contract ID, exact WASM hash and verified
source semantics, token, bond, challenge window, admin, committee, last observed
assertion ID, and every on-chain `Pending` or `Disputed` assertion, including
direct submissions not recognized by an integrator. Reconstruct configuration and
IDs from deployment transactions and events where necessary; v1 has no public
config, version, or `NextId` getter.

For each open assertion, reconcile expected liability:

- `Pending`: one assertion bond;
- `Disputed`: two bonds; and
- `Resolved`: no remaining liability under normal v1 settlement.

Audit whether every relevant committee has distinct, available addresses and a
reachable majority. V1 does not reject duplicate resolver addresses, while one
address can vote only once; a duplicate-filled snapshot can make its numeric
majority unreachable. For deployed v1 WASM that predates `MAX_BOND_AMOUNT`, also
verify that its bond-derived arithmetic is representable. Current v1 enforces
`MAX_BOND_AMOUNT` at initialization, but that source change does not alter older
deployed bytecode.

Archived assertion entries, instance storage, or contract code may need ledger
restoration before settlement, especially for deployments predating the current
TTL fix. Archive deployment transactions, assertion state, and events off-chain;
RPC event retention and live ledger TTL are not a historical archive.

### 2. Deploy v2 separately

Deploy and initialize a new contract at a new address. Do not copy v1 assertion
records, mint replacement positions, or manually move pooled token balances.
Every v1 bond remains a liability of v1. Its only normal exit is v1 `finalize` or
`resolve`; if the exact deployed rules cannot reach either path, v2 cannot rescue
or move that bond.

Record a cutover ledger/timestamp and route only newly accepted assertions to v2.
The complete identity of an assertion becomes `(contract_id, assertion_id)`;
both deployments can legitimately issue ID `0`.

### 3. Run both versions while v1 drains

Integrators and indexers keep v1 and v2 bindings side by side and execute each v1
assertion under the exact semantics of its deployed WASM:

- v1 assertions are read and completed under the v1 interface;
- in snapshot-capable v1 releases, an already `Disputed` assertion keeps its
  captured `Assertion.resolvers`, while a `Pending` assertion captures the live
  committee only if and when it is disputed;
- older v1 bytecode that predates `Assertion.resolvers` reads the live committee
  during voting, so a committee update has different consequences;
- v2 assertions use only v2 positions and weighted voting; and
- historical v1 `Finalized`/`Resolved` events remain authoritative for their
  outcomes.

Keep the v1 committee stable during drain unless an incident requires a change,
and analyze that change against the verified WASM first. Rotation cannot repair
an unavailable or malformed committee already captured by a snapshot.

Do not pause v1 during this drain. V1 pause blocks both `dispute` and `resolve`
while leaving `finalize` callable; pausing a pending assertion could remove its
chance to be challenged without preventing it from finalizing. It also cannot
rescue an open dispute whose snapshotted committee is unavailable.

The v1 contract cannot reject only new assertions. The cutover is therefore an
integrator policy, not a perfect on-chain gate: direct callers may still create
unrecognized v1 assertions, which remain v1 liabilities and must not be silently
treated as v2 work.

### 4. Retire v1 operationally

Operators can remove v1 from official submission interfaces after accepted
traffic has drained, but the conservative on-chain policy is to leave it unpaused
and clearly deprecated. There is no atomic operation that blocks a new assertion
while preserving `dispute` and `resolve`: an assertion can enter immediately
before a final pause, after which its short challenge window may expire before an
unpause can restore the right to dispute it.

An operator that nevertheless pauses v1 must first find no `Pending` or
`Disputed` assertion anywhere in the complete on-chain inventory, monitor through
the pause ledger, and be prepared to unpause and drain again if new activity
appears. This is best-effort, not a trustless safe-retirement guarantee; an
adversary can delay it indefinitely by continuing to submit assertions.

Keep the v1 address, exact ABI/WASM metadata, and an off-chain state/event archive
for historical use. A residual token balance can be called dust or an unsolicited
transfer only after liabilities, including archived or previously unindexed
assertions, are reconciled. V1 cannot sweep it and it must never be represented as
migrated escrow.

### Rollback boundary

Before v2 accepts its first bond, an integrator can route new traffic back to v1.
After either version has live bonds, there is no atomic rollback: each contract
must continue under its own deployed and per-assertion rules.

Absent a separately designed compatibility shim, on-chain consumers compiled
against v1 require v2 bindings and an upgrade or new deployment; changing only
the target contract ID is insufficient because v2 changes the assertion shape and
lifecycle calls. Migration planning must inventory consumers as well as Tholos
instances.

## Future implementation work

No item below is part of this design-only change. After this proposal is reviewed
and accepted, implementation should be split into auditable issues for:

1. versioned v2 state, policy pinning, registration, and authorization;
2. commitment/reveal voting and weighted-majority property tests;
3. liability accounting, permissionless credit accrual, owner-authorized
   withdrawals, rounding, and adversarial-token tests;
4. TTL behavior for assertion, resolution, position, and credit records;
5. v1/v2 consumer bindings and a blue/green migration runbook;
6. testnet volume tests with many positions and concurrent disputes;
7. economic simulation for window lengths, minimum bonds, whale capture, and
   default frequency; and
8. an independent security audit before meaningful-value mainnet use.

At minimum, randomized tests must cover address splitting, deposit aggregation,
exact half versus half-plus-one, abstention, timeout, maximum amounts, overflow,
arbitrary settlement order, exact escrow conservation, reentrancy, and immutable
snapshots under attempted admin updates.

## Questions for design review

1. Should timeout preserve the optimistic boolean result as proposed, or should
   safety on split votes take priority through an `Inconclusive` result?
2. Should the minimum external position always equal the base assertion bond, or
   be a separately pinned parameter?
3. Which deposits qualify for a bounded anti-sniping extension, and what hard
   maximum prevents extension griefing?
4. Should all non-revealed stake be forfeited and redistributed to the proposed
   recipient set, or should its destination be independent of the outcome? What
   happens during a symmetric operational cancellation?
5. Is the separate timeout settlement (refund every revealed principal and share
   only non-revealed stake) sufficient to deter frivolous disputes, or is a
   distinct non-refundable fee required?
6. Is perpetual, restorable credit entitlement acceptable, and who funds storage
   restoration or keeper calls after the initial settlement/withdrawal grace?
7. What canonical market/question identifier and evidence convention, if any,
   must a separate dependency add for an open electorate?
8. Should a later protocol tier allow a new, more highly bonded assertion after
   an `Inconclusive` alternative, or is one weighted round the final tier?
9. Is a creation-only pause acceptable despite its limited incident containment,
   or must a separate symmetric freeze/cancel mechanism be designed first?

Until those economic choices are approved and threat-modeled, this proposal must
remain `Proposed` and no implementation issue should treat its interface sketch
as final.
