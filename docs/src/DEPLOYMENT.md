# Deployment and operations

A practical guide for deploying a Tholos instance and operating it afterward. For
what each function does, see [CONTRACT.md](CONTRACT.md). For design rationale,
see [ARCHITECTURE.md](ARCHITECTURE.md).

## Before you deploy

**This is testnet-only until audited.** See [SECURITY.md](SECURITY.md). Don't
point a Tholos instance at real value on mainnet without an independent security
review first.

Decide these parameters up front; none of them (except the resolver committee)
can be changed after `initialize`:

| Parameter | Guidance |
| --- | --- |
| `token` | Any SEP-41 token your users already hold. No swap step exists, so picking a token nobody has is a dead deployment. |
| `bond_amount` | High enough to make spam assertions and bad-faith disputes costly, low enough that legitimate use isn't priced out. There's no data-driven formula for this yet; start conservative and watch real usage. Also capped at `MAX_BOND_AMOUNT`, a contract-enforced ceiling well above any realistic bond size — it exists so the bond can never overflow `finalize`'s reward-multiply arithmetic (the binding constraint) or the token balance held across a dispute. |
| `challenge_window_secs` | Long enough that people who'd actually catch a bad assertion have a realistic chance to see it and act. Short windows finalize faster but catch less. |
| `resolvers` | Odd-length, non-zero. Pick people who'll actually be reachable to vote within a reasonable time of a dispute; a slow resolver committee stalls every disputed assertion until it acts. |
| `finalize_reward_bps` | Basis points (0–1000) of the bond paid to whoever calls `finalize`. `caller` must authorize the call unconditionally, even at 0. 0 means no reward: the full bond returns to the asserter. A non-zero value creates an economic incentive for prompt finalization at the cost of a small bond haircut the asserter accepts when posting. 100 bps (1 %) is a reasonable starting point; 1000 bps (10 %) is the maximum enforced by the contract. |

## Deploying

```sh
# Build the optimized wasm
cd contracts/tholos && stellar contract build

# Deploy
CONTRACT=$(stellar contract deploy --wasm target/wasm32v1-none/release/tholos.wasm \
  --source deployer --network testnet)

# Initialize
stellar contract invoke --id "$CONTRACT" --source deployer --network testnet -- initialize \
  --admin "$ADMIN_ADDRESS" \
  --token "$TOKEN_CONTRACT_ID" \
  --bond_amount 1000000 \
  --challenge_window_secs 3600 \
  --resolvers "[\"$R1\",\"$R2\",\"$R3\"]" \
  --finalize_reward_bps 0
```

`scripts/testnet-smoke.sh` automates this full sequence plus assert/dispute/resolve
against real testnet infrastructure; run it to sanity-check a fresh deploy before
handing the contract id to anyone.

## Admin runbook

### Pausing during an incident

If something looks wrong (a bug is found, a resolver key looks compromised, vote
behavior looks off), pause first and investigate second:

```sh
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- set_paused --paused true
```

This stops new `assert_outcome`, `dispute`, and `resolve` calls immediately.
Assertions already `Pending` can still `finalize` normally, so you aren't freezing
funds that were never at risk. Unpause the same way with `--paused false` once
the issue is resolved.

### Rotating the resolver committee

There are two paths. `update_resolvers` is the admin emergency override; it works
whether paused or not, so a compromised committee can be replaced without waiting to
unpause:

```sh
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- update_resolvers \
  --new_resolvers "[\"$NEW_R1\",\"$NEW_R2\",\"$NEW_R3\"]"
```

Day to day, the committee rotates itself by a strict majority vote, with no admin
key involved. A resolver proposes a single-slot swap, and the rest vote:

```sh
# R1 (a current resolver) proposes replacing themselves with R4.
stellar contract invoke --id "$CONTRACT" --source resolver1 --network testnet -- \
  propose_rotation --resolver "$R1" --old_resolver "$R1" --new_resolver "$R4"

# Two more resolvers vote yes; with a 3-member committee that's the majority,
# so the rotation executes as soon as the second yes lands.
stellar contract invoke --id "$CONTRACT" --source resolver2 --network testnet -- \
  vote_rotation --resolver "$R2" --approve true
stellar contract invoke --id "$CONTRACT" --source resolver3 --network testnet -- \
  vote_rotation --resolver "$R3" --approve true
```

Either path writes the same committee; both emit `ResolversUpdated`. A rotation has
no effect on disputes already open, because each dispute snapshots the committee at
`dispute` time. See [CONTRACT.md](CONTRACT.md) and
`docs/src/ROTATION_DESIGN.md` for the full detail.

### Checking state

Read-only, no auth required:

```sh
stellar contract invoke --id "$CONTRACT" --source admin --network testnet -- get_assertion_state --id 0
```

## Mainnet readiness checklist

Not a green light to deploy to mainnet on its own: a checklist of what's true
today, so you can judge what's still missing for your use case:

- [x] Core propose/dispute/resolve flow implemented and unit tested
- [x] Reentrancy hardened, with a regression test proving it
- [x] Admin pause and resolver rotation available for incident response
- [x] Exercised end-to-end against real Stellar testnet infrastructure
- [ ] Independent security audit
- [ ] Real-world dispute volume tested (all testing so far is synthetic)
- [ ] Bond sizing validated against real spam/griefing attempts, not just reasoned about
- [x] Fee/reward mechanism for uncontested finalizes (configurable `finalize_reward_bps`; see [CONTRACT.md](CONTRACT.md))
