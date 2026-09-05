# Deployment and operations: Protocol v2

A practical guide for deploying a Tholos v2 instance and operating it afterward. For
what each function does, see [CONTRACT.md](CONTRACT.md). For design rationale,
see [V2_RESOLUTION.md](V2_RESOLUTION.md). For v1 deployment, see
[DEPLOYMENT.md](DEPLOYMENT.md#canonical-testnet-deployment).

## Before you deploy

**This is testnet-only until audited.** See [SECURITY.md](SECURITY.md). Don't
point a Tholos v2 instance at real value on mainnet without an independent security
review first.

Decide these parameters up front; none of them can be changed after
`initialize`. `admin` is the exception worth calling out separately: it's
pinned by the constructor at deploy time, not passed to `initialize` at
all, and can only change afterward if the current admin itself authorizes
a handoff via `set_admin` (see [Admin runbook](#admin-runbook) below) —
there's no deploy-time or `initialize`-time input that can override it:

### Core parameters

| Parameter | Guidance |
| --- | --- |
| `admin` | Passed to `stellar contract deploy` as a constructor argument, not to `initialize`. Controls `set_paused_v2`, `cancel_round`, and its own rotation via `set_admin` only; a hostile or lost admin key can grief active assertions via those levers but has no direct profit path (no fee-taking, no fund-sweeping power). Pick an address whose key custody you trust: nothing at deploy or `initialize` time can override it, though the admin can hand the role to a new address later via `set_admin` if needed. |
| `token` | Any SEP-41 token your users already hold. No swap step exists, so picking a token nobody has is a dead deployment. Must match v1's choice if accepting both v1 and v2 assertions in your integrations. |
| `base_bond` | Size from the spam/griefing model in [BOND_SIZING.md](BOND_SIZING.md). Equal to v1's `bond_amount` in principle, but v2 adds a third-party registration tier: a cheaper base bond attracts counter-stake faster, while a larger one deters frivolous disputes. Set it using the same analysis as v1 (start with the larger of the assertion-spam and bad-faith-dispute floors, add any target attacker-loss margin), then check that `max_total_weight` and `max_position` will accommodate realistic multi-party dispute scenarios. Also capped at `MAX_BOND_AMOUNT`, a contract-enforced ceiling well above any realistic bond size. It exists so the bond can never overflow `finalize`'s reward-multiply arithmetic (`bond * finalize_reward_bps`) or the token balance held across registration and settlement. |
| `challenge_window_secs` | Long enough that people who'd actually catch a bad assertion have a realistic chance to see it and act. Short windows finalize faster but catch less. In v2, this is the only deadline before the assertion is disputed; registration and reveal happen afterward, so budget time before this expires for dispute-scoped registration and reveal to complete. |
| `finalize_reward_bps` | Basis points (0–1000) of the bond paid to whoever calls `finalize`. `caller` must authorize the call unconditionally, even at 0. 0 means no reward: the full bond returns to the asserter. A non-zero value creates an economic incentive for prompt finalization at the cost of a small bond haircut the asserter accepts when posting. 100 bps (1%) is a reasonable starting point; 1000 bps (10%) is the maximum enforced by the contract. |

### Registration and voting windows

These v2-specific parameters control the dispute-scoped registration and reveal phases.
They are immutable per deployment and take effect when the assertion is created, not when
a dispute arrives. Every timeline runs from the moment `dispute` is called.

| Parameter | Guidance |
| --- | --- |
| `registration_duration_secs` | How long a dispute stays in registration, during which the asserter, disputer, and any third party can lock capital and commit votes. Must be at least 1 second; the contract enforces a practical upper bound to keep lifetimes reasonable. Deposit this commitment time into your business model: typical Internet disputes might use 1 day; urgent or time-sensitive ones might use 1 hour. If your dispute is about a sports result, a stock price, or anything with a known announcement, set this shorter than the time until the external event resolves, so resolution bonds are visible in time. |
| `anti_snipe_extension_secs` | How much longer the registration deadline moves if a position is funded within this many seconds of the ordinary cutoff. Prevents a late attacker from dominating an already-open dispute in the final second. Set it to 0 if you don't need anti-sniping (a trusted environment with no arms-race incentive), or to a reasonable backstab window (e.g., 5 minutes) if you expect contested disputes. The contract enforces an upper bound relative to `anti_snipe_hard_max_secs` (see below). |
| `anti_snipe_hard_max_secs` | The absolute maximum registration deadline, regardless of how many extensions occur. No deposit can extend registration past this time, even if extensions keep firing. Set it to at least `registration_duration_secs` (the contract enforces this) and at most `MAX_ANTI_SNIPE_HARD_MAX_SECS` (29 days), plus enough extension opportunities to feel fair (e.g., `registration_duration_secs + 100 * anti_snipe_extension_secs`). A very large hard max defeats anti-sniping; a very small one (barely above the base window) defeats extensions. |
| `reveal_duration_secs` | How long a dispute stays in the reveal phase after registration closes, during which all third-party commitments from registration become binding votes by revealing their salted choice. Must be at least 1 second; the contract enforces a practical upper bound. Typical disputes might use 6 hours to 1 day here: long enough for off-chain coordinators to run their own resolution process, short enough to finalize quickly. After the reveal deadline, any position that did not reveal is counted as abstaining (forfeited in settlement). |

### Arithmetic bounds

These v2-specific parameters limit the total stake and individual positions the contract
will accept. They exist to guarantee the arithmetic in settlement calculations cannot overflow and that the contract remains responsive.

| Parameter | Guidance |
| --- | --- |
| `max_position` | The largest single stake one address can lock in any dispute. Prevents a whale from unilaterally moving the total weight, forcing it to split stake across addresses if it wants to participate larger. Must be at least 1 and at most `max_total_weight`. Setting it equal to `max_total_weight` removes this constraint (a single address can be 100% of eligible weight); that's reasonable for small deployments or if you trust your stakers. For larger or more adversarial scenarios, set it well below, perhaps 10–20% of `max_total_weight`. |
| `max_total_weight` | The aggregate locked stake any single dispute can reach. Once `max_total_weight` is locked, registration stops accepting new positions or top-ups. Prevents unlimited storage growth and ensures settlement arithmetic stays bounded. The contract enforces a hard ceiling to prevent overflow. Set it to a realistic bound on the total value you want to put at risk in any one dispute: perhaps 10–100x your base bond if you expect vigorous counter-stakes, or just 2–3x if you expect assertions to finalize mostly uncontested. |

## Canonical testnet deployment

Before deploying your own v2 instance, check if there's a canonical one that already fits. Unlike v1
(which has a long-lived shared instance), v2 does not yet have an official canonical testnet deployment.
For now, deploy your own: `scripts/testnet-load-v2.sh` demonstrates the full sequence against real Stellar testnet infrastructure.

**Note:** No canonical v2 contract address will be added to this document until one is deployed and independently
verified. See [CONTRIBUTING.md](CONTRIBUTING.md)'s reviewing-PRs section for why a committed contract address is
never accepted without independent verification.

## Deploying

`admin` is pinned by the contract's constructor (`__constructor`), invoked
atomically as part of the deploy operation itself, not by a later call: pass
it to `stellar contract deploy` as a constructor argument, after the `--`.
This closes the front-running window a separate deploy-then-initialize(admin)
step used to leave open (see the `__constructor` entry in
[CONTRACT_V2.md](CONTRACT_V2.md)'s Functions section).
`initialize` no longer takes `admin` at all; it authenticates against the
admin already fixed at deploy.

```sh
# Build the optimized wasm
cd contracts/tholos-v2 && stellar contract build

# Deploy, pinning admin as a constructor argument
CONTRACT=$(stellar contract deploy --wasm target/wasm32v1-none/release/tholos_v2.wasm \
  --source deployer --network testnet -- --admin "$ADMIN_ADDRESS")

# Initialize the rest of the deployment-wide policy; requires that same
# admin's signature
stellar contract invoke --id "$CONTRACT" --source deployer --network testnet -- initialize \
  --token "$TOKEN_CONTRACT_ID" \
  --base_bond 1000000 \
  --challenge_window_secs 3600 \
  --finalize_reward_bps 0 \
  --registration_duration_secs 3600 \
  --anti_snipe_extension_secs 300 \
  --anti_snipe_hard_max_secs 7200 \
  --reveal_duration_secs 3600 \
  --max_position 50000000 \
  --max_total_weight 250000000
```

### Parameter selection for the deploy example above

The example uses these choices for illustration; adapt them to your use case:

- `base_bond`: 1,000,000 units (e.g., 0.1 XLM if using native SAC)
- `challenge_window_secs`: 3600 (1 hour)
- `finalize_reward_bps`: 0
- `registration_duration_secs`: 3600 (1 hour)
- `anti_snipe_extension_secs`: 300 (5 minutes)
- `anti_snipe_hard_max_secs`: 7200 (2 hours), twice `registration_duration_secs` so a handful of near-deadline extensions can't stall registration indefinitely
- `reveal_duration_secs`: 3600 (1 hour)
- `max_position`: 50,000,000 units, 50x `base_bond`, enough headroom for a real multi-party dispute without approaching `max_total_weight` on its own
- `max_total_weight`: 250,000,000 units, 5x `max_position`, so no single position can dominate the vote outright

`scripts/testnet-load-v2.sh` automates a similar sequence plus assert/dispute/register/reveal/resolve
against real testnet infrastructure; run it to sanity-check a fresh deploy before handing the contract
id to anyone.

## Admin runbook

V2 has a narrower admin surface than v1. Notably, v2 has no equivalent to v1's `update_resolvers`; there
is no resolver committee to rotate.

### Rotating the admin

If the admin key needs to change (planned custody handoff, or a compromised key that's still able to
sign), the current admin can hand the role to a new address:

```sh
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- set_admin \
  --new_admin "$NEW_ADMIN_ADDRESS"
```

This takes effect immediately: the old admin loses `set_paused_v2`/`cancel_round`/`set_admin` authority
the instant the call succeeds, with no grace period. If the current admin's key is lost outright (not just
compromised), there's no recovery path — `set_admin` requires the current admin's own signature, so a lost
key means the role is stuck until redeployment.

### Pausing new assertions during an incident

If something looks wrong (a bug is found, or vote behavior looks off), pause to prevent new assertions
from opening while investigation proceeds:

```sh
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- set_paused_v2 --paused true
```

This stops `assert_outcome` immediately, preventing the creation of new assertions. Critically, it does
**not** affect already-open assertions: existing disputes remain in registration, reveal, or resolution
as if nothing changed, and no deadline is altered or extended. A paused-out `assert_outcome` is the only
v2 pause available; v2 cannot (and does not) pause disputes, reveals, settlement, or withdrawals mid-flight.

This narrow scope is intentional; see [V2_RESOLUTION.md](V2_RESOLUTION.md#administration-and-pause-semantics)
for why. Minimize pause duration and unpause with `--paused false` as soon as incident handling permits.
If a deeper incident requires canceling an already-open round entirely, use `cancel_round` instead (see below).

### Canceling a round

If an assertion must be unwound, whether it's still `Pending` (never disputed) or already `Disputed`
(e.g., a bug is discovered mid-round, or the contract needs to be redeployed), cancel it:

```sh
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- cancel_round --id 0
```

`cancel_round` can only be called while the contract is paused (i.e., after `set_paused_v2 --paused true`).
It permanently finalizes the round: `phase` moves to `Resolved` and `terminal_cause` locks to
`AdminCancelled`, so the claim itself is not left open, cancellation is a real terminal outcome, not just a
fund restoration. What happens to locked funds depends on the phase it was cancelled from:

- **Still `Pending`** (never disputed): the asserter's bond is refunded directly, in the same call.
- **`Disputed`/`Registration`/`Reveal`** (third-party positions exist): `cancel_round` does not itself move
  any tokens. Every funded position, including the asserter's and disputer's, recovers its exact principal
  (no forfeiture, no reward) through the normal `settle` + `withdraw` path afterward, the same as any other
  resolved round.

Use this path only in genuine emergency scenarios (e.g., a discovered bug in voting logic, or a forced
redeployment). Canceling a round is visible to users and affects the integrity of the record, so document
why and coordinate with your users beforehand if possible.

### Checking state

Read-only, no auth required:

```sh
# Get a specific assertion
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- get_assertion --id 0

# Get the current policy
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- get_policy

# Get a position and its entitlements
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- get_position \
  --id 0 --address "$ADDRESS"

# Get a resolution round (phase, deadlines, tallies, etc.)
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- get_resolution --id 0

# Get an owner's available credit from a dispute
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- get_credit \
  --id 0 --address "$ADDRESS"
```

## Integration notes

### v1 and v2 coexistence

v1 and v2 are separate contracts with separate storage, separate assertion ID sequences, and separate
token contracts. They do not share state. An integrator may run both simultaneously (e.g., use v1 for
existing, low-stakes assertions and v2 for new, higher-stakes ones), but must treat them as independent
deployments for the purposes of routing assertions, checking assertion status, and settling disputes.

No automatic migration or bridging between v1 and v2 exists in the contract. See
[V2_MIGRATION.md](V2_MIGRATION.md) for application-level migration strategies.

### Voter secrecy and commit-reveal

Third-party positions in v2 use a salted commitment scheme: they lock capital and commit to a secret vote,
then reveal it in a later phase. The commitment is `sha256(canonical_encode(VoteCommitmentPreimage))`, where:

```
VoteCommitmentPreimage = {
  domain: "THOLOS_V2_VOTE",
  network_id: <soroban network id>,
  contract_address: <this v2 contract address>,
  policy_hash: <sha256 of the PolicySnapshotV2>,
  assertion_id: <id of the assertion>,
  round: <registration round counter>,
  voter: <address revealing this vote>,
  choice: <boolean, agrees with the asserter or not>,
  salt: <32 random bytes>
}
```

Compute the commitment off-chain, call `register` with it, and call `reveal` with the original preimage
when the reveal phase opens. The `compute-commitment` tool (`tools/compute-commitment/`) is provided
for this; see the usage comment at the top of `tools/compute-commitment/src/main.rs`. The commit-reveal
scheme prevents vote copying and keeps a voter's choice private until the reveal phase, when timing and
anonymity properties shift.

### TTLs and archival

Every position, credit, and assertion in v2 has an associated TTL (time-to-live). Once a TTL expires, the
entry may be archived by the ledger, becoming invisible on-chain. However, archival does not destroy the liability:
an archived position can be restored by reading its preimage from historical events or off-chain sources, and the
entitlement remains valid for settlement and withdrawal.

The contract emits an indexable event for every phase transition, deposit, reveal, settlement, and withdrawal. Off-chain
indexers should track these events to allow users to recover their state and claim entitlements after TTL archival.

This design prioritizes bounded on-chain storage (no unlimited per-dispute vectors) over unlimited on-chain
availability. It is the integrator's responsibility to preserve event history or provide a recovery mechanism.
