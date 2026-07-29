# Bond sizing analysis

This note turns the deployment-time `bond_amount` choice into an operational model
for spam, bad-faith disputes, resolver rotation, and finalize reward griefing. It is
not a substitute for an audit or live production telemetry, but it gives deployers a
repeatable way to pick an initial value and revisit it as usage changes.

## Inputs to collect

Use token units consistently. For example, if the configured SEP-41 token has 7
decimals, `1_0000000` means one token.

| Symbol | Meaning |
| --- | --- |
| `V_min` | Minimum economically meaningful assertion value that should remain affordable. |
| `C_assert` | Attacker's non-bond cost to post one assertion: transaction fee, opportunity cost, and integration overhead. |
| `C_dispute` | Attacker's non-bond cost to dispute one assertion. |
| `R_case` | Resolver committee's off-chain cost to review and vote on one disputed assertion. |
| `K_spam` | Maximum unresolved spam assertions the deployment is willing to tolerate in one challenge window. |
| `K_dispute` | Maximum bad-faith disputes the deployment is willing to tolerate in one challenge window. |
| `A_window` | Number of legitimate assertions expected during one challenge window. |
| `p_dispute` | Expected share of legitimate assertions that receive good-faith disputes. |
| `target_attacker_loss` | Minimum token loss you want an attacker to bear for each successful spam or dispute attempt. |
| `min_finalizer_reward` | Minimum reward, in token units, needed to make third-party finalization worthwhile. |
| `reward_bps` | Configured `finalize_reward_bps`, from 0 to 1000. |

## Formula

Pick a target bond that satisfies all four constraints, then round up to a simple
operator-friendly value:

```text
bond_amount >= max(
  R_case / max(1, K_spam),
  R_case / max(1, K_dispute),
  target_attacker_loss - min(C_assert, C_dispute),
  min_finalizer_reward * 10_000 / max(1, reward_bps)
)
```

Then cap it with the affordability bound:

```text
bond_amount <= V_min * max_acceptable_bond_share
```

Use `max_acceptable_bond_share` between 5% and 20% for user-facing markets. If the
lower bound exceeds the affordability cap, the deployment is underpriced for its
threat model: raise the minimum assertion value, narrow access to assertion posting,
increase resolver capacity, lengthen monitoring coverage, or lower `finalize_reward_bps`
instead of quietly launching with an unaffordable bond.

The formula intentionally treats the asserter bond and disputer bond symmetrically:
each side must lock `bond_amount`, and the losing side forfeits it. A spammer can
still force resolver attention by accepting losses, but each extra unit of resolver
work burns a predictable amount of attacker capital.

## Scenario checks

### 1. Low-value assertion spam

Attack: an account posts many cheap assertions whose value is lower than resolver
review time, hoping resolvers ignore them or spend time triaging junk.

Sizing rule:

```text
bond_amount + C_assert >= R_case / K_spam
```

If the committee can tolerate at most 10 unresolved spam assertions per window and a
full review costs about 20 tokens of resolver time, the bond should be at least 2
tokens before considering transaction fees. For public deployments, use a larger
multiple, such as 2x to 5x `R_case / K_spam`, because attackers may value disruption
more than the direct token loss.

### 2. Bad-faith dispute spam

Attack: a disputer challenges legitimate assertions to lock both sides into the
resolver path, delay finality, and consume resolver attention.

Sizing rule:

```text
bond_amount + C_dispute >= R_case / K_dispute
```

Worked example: if one disputed case costs 30 tokens of resolver time and the
deployment tolerates no more than 5 bad-faith disputes per challenge window, set the
dispute-facing floor at 6 tokens. If expected legitimate disputed volume is
`A_window * p_dispute`, make sure the committee can process that baseline plus
`K_dispute`; otherwise the correct fix is resolver capacity, not only a larger bond.

### 3. Resolver self-rotation griefing

Attack: a resolver opens rotation proposals to distract the committee, block another
proposal, or churn membership during active disputes.

Bond sizing does not directly price this action because `propose_rotation` and
`vote_rotation` do not move tokens. The contract's mitigations are procedural and
structural:

- Only current resolvers can propose or vote, so the attack is limited to a trusted
  committee member or compromised resolver key.
- Only one rotation can be open at a time, and no-votes can make an impossible
  proposal auto-cancel, so a stale proposal cannot permanently deadlock rotation.
- A rotation does not affect in-flight disputes; each dispute snapshots the resolver
  committee at `dispute` time.
- The admin `update_resolvers` path clears any open rotation and remains the
  break-glass recovery path for a compromised or unavailable committee.

Operational guidance: include expected rotation review time in `R_case` when the
same people handle disputes and governance. If rotation noise becomes frequent,
replace the noisy resolver through self-rotation or the admin override; increasing
`bond_amount` will not punish rotation spam.

### 4. Finalize reward griefing

Attack: a bot finalizes every uncontested assertion only to extract
`finalize_reward_bps`, reducing asserter returns or making assertion posting feel
taxed.

The reward is bounded by:

```text
finalize_reward = floor(bond_amount * reward_bps / 10_000)
```

This is not a contract-balance drain: the reward is paid from the asserter's own
bond, and the remainder returns to the asserter. The risk is economic UX. Keep the
reward large enough to cover finalizer transaction fees and monitoring overhead, but
small enough that the haircut is acceptable:

```text
min_finalizer_reward <= bond_amount * reward_bps / 10_000
reward_bps <= max_acceptable_haircut_bps
```

Worked example: if finalizers need at least 0.02 tokens to bother calling and
`reward_bps = 100` (1%), the bond must be at least 2 tokens for the reward to meet
that target. If that bond is too high for low-value assertions, set
`finalize_reward_bps` to 0 and rely on the asserter or integrator to finalize their
own assertions.

## Recommended starting profiles

| Profile | `bond_amount` guidance | `finalize_reward_bps` guidance | Use when |
| --- | --- | --- | --- |
| Private beta | 1x to 2x expected resolver review cost divided by tolerated spam per window | 0–50 bps | Known users, low bot pressure, integrator can finalize. |
| Public testnet / low value | 2x to 5x the larger of assertion-spam and dispute-spam floors | 50–100 bps | Open participation with low economic stakes. |
| Higher-value mainnet candidate | 5x+ the larger spam floor, still within 5%–20% of `V_min` | 0–100 bps | Meaningful value, monitored resolvers, audited deployment. |

Do not set `bond_amount` near the contract's `MAX_BOND_AMOUNT`. That maximum exists
only to prevent arithmetic overflow in dispute balances and finalize rewards; it is
not an economic recommendation.

## Monitoring and adjustment

Review these metrics after every testnet campaign and before any mainnet launch:

- Assertions opened per challenge window, split by source integration.
- Dispute rate, dispute win rate, and repeated losing disputers.
- Median and p95 time from `Disputed` to `Resolved`.
- Rotation proposals opened, cancelled, and executed.
- Finalize calls by account and total reward paid.

Raise the bond if losing assertions or losing disputes cluster around a small number
of accounts and resolver latency rises. Lower the bond, or lower
`finalize_reward_bps`, if legitimate assertions are priced out relative to `V_min`.
Revisit the calculation whenever the token price, resolver compensation, challenge
window, committee size, or expected assertion value changes materially.
