# V2 bond sizing and window analysis

This note models the deployment-time choices unique to protocol v2: the
`base_bond` / `min_resolution_bond` floor, the registration and reveal window
lengths, anti-sniping headroom, and the per-address position cap. The models are
not a substitute for live telemetry, but they give deployers a repeatable way to
pick initial values and revisit them as usage changes.

For the v1 bond-sizing framework (spam floors, dispute floors, and finalize-reward
constraints that still apply to `base_bond`) see [BOND_SIZING.md](BOND_SIZING.md).
This document extends that analysis with the three problems that are new in v2.

## Background: v2 outcome rule

Every disputed assertion produces a frozen eligible-weight total `W` at the
registration cutoff. `W` includes the asserter's `base_bond`, the disputer's
`base_bond`, and every third-party position registered before the cutoff.

A side wins by **strict majority**: revealed weight for that side must exceed
`W / 2`. If neither side crosses that threshold when the reveal window closes,
the **optimistic timeout default** applies and the originally asserted outcome
stands.

Settlement differs by terminal cause:

- **Strict majority**: winning positions recover principal and share the losing
  and non-revealed pool pro rata. Losing and non-revealed positions are forfeited.
- **Optimistic timeout**: all revealed positions (on either side) recover principal
  and share only the non-revealed pool. Non-revealed positions are forfeited.

Those two rules make non-revelation a double loss: the non-revealer forfeits its
bond and in a strict-majority scenario, its forfeiture is paid to the winning
side rather than returned to it. Under a timeout default, non-revealers still
forfeit to revealed positions.

## Part 1: default-frequency model

### The core asymmetry

The optimistic timeout assigns the outcome to the asserted side unless a revealed
majority actively overturns it. Non-revealed weight sits in the denominator `W`
but contributes no votes. This means a large pool of registered-but-silent
positions can make an honest majority impossible without being a majority itself.

Formally, let:

- `W` = frozen eligible total
- `F` = revealed weight agreeing with the asserted outcome
- `A` = revealed weight against
- `N` = non-revealed weight (`N = W - F - A`)

The assertion stands whenever `A <= W / 2`. Because `N > 0` inflates `W`, even a
large `A` may not cross the threshold.

### Default-frequency formula

For a dispute where the true outcome is *against* the asserted side, the assertion
nevertheless survives if:

```text
A / W <= 0.5
```

Define `t = (F + A) / W` as the **reveal turnout rate** (fraction of eligible
weight that actually reveals). For a symmetric dispute where roughly half the
revealed weight sides with each party:

```text
A ~= t x W / 2
```

Then the assertion survives if:

```text
t x W / 2 <= W / 2  ->  t <= 1.0
```

That is always true: turnout alone cannot prevent a timeout. The meaningful
question is what reveal turnout `t` is needed for the *against* side to reach a
strict majority when the split of revealed weight is skewed toward the losing
(assertion-supporting) side.

Let `s` be the fraction of revealed weight on the *against* side (honesty
share). A strict-majority against-the-assertion requires:

```text
s x t > 0.5
  ->  t > 0.5 / s
```

| Against-side honesty share `s` | Minimum turnout `t` for correction |
| ------------------------------ | ---------------------------------- |
| 100% (all revealers disagree)  | > 50%                              |
| 80%                            | > 62.5%                            |
| 66%                            | > 75.8%                            |
| 51%                            | > 98%                              |
| 50%                            | impossible (tie never crosses 50%) |

**Key finding**: when honest disagreers are 66% of revealers, three-quarters of
all eligible weight must reveal for correction. When they are 51%, virtually the
entire pool must reveal. Any non-reveal (absent staking, lost keys, sleeping
coordinators) benefits the asserted outcome.

### The issue-body example

The V2 design discussion raised this scenario explicitly:

> 1% reveals for the assertion, 49% reveals against, 50% does not reveal.

Here `W = 100`, `F = 1`, `A = 49`, `N = 50`. The against-side has 49 units,
but the threshold is `W / 2 = 50`. The assertion stands because `49 < 50`.

More generally, any time `N >= 1` unit makes the strict-majority threshold
unattainable for the against-side, the assertion stands regardless of the
revealed margin. This is the intended behavior: the optimistic default favors the
status quo and treats abstention as implicit delegation to it.

### Implications for window sizing

The default-frequency risk is not eliminated by longer windows; it is controlled
by ensuring that enough bonded capital can *reach* the reveal phase. The levers
are:

1. **`registration_duration_secs`**: a longer window lets more third-party
   positions arrive, growing `W` with counter-stake rather than diluting a small
   `W` with silence.
2. **`reveal_duration_secs`**: a longer reveal window lets coordinators organize
   reveals. Too short a window and registered positions may silently miss it.
3. **`min_resolution_bond` = `base_bond`**: a higher floor means fewer positions
   but each one is more economically committed. It does not by itself prevent
   default.

The practical guard against frequent timeout-by-silence is ensuring that the
*disputer* plus a realistic cohort of third-party registrants can represent a
strict majority of `W`. With only two equal fixed positions (`2 x base_bond`) and
no third parties, `W = 2 x base_bond`, `A = base_bond`, and `A / W = 0.5`; the
disputer's weight never exceeds half. A lone dispute *always* times out unless at
least one additional third party registers on the against side. See Part 3 for the
window-length tradeoffs that govern third-party participation.

---

## Part 2: whale-capture model

### Definitions

A **whale** is a single address or coalition that can unilaterally reach strict
majority in one dispute. In v2 this means controlling more than `W / 2` of
eligible weight at the registration cutoff.

Because weight is strictly proportional to bonded capital and splitting across
addresses provides no extra weight, the analysis is purely about capital
concentration.

Let:

- `B` = `base_bond` (asserter's and disputer's positions, each)
- `R_w` = whale's registration deposit (one or more top-ups summing to at most
  `max_position`)
- `R_h` = total honest counter-stake registered by non-whale addresses
- `W = 2B + R_w + R_h`

The whale controls strict majority when:

```text
R_w > W / 2
  ->  R_w > (2B + R_w + R_h) / 2
  ->  2R_w > 2B + R_w + R_h
  ->  R_w > 2B + R_h
```

### Concentration threshold

Define `c` as the whale's share of total eligible weight:

```text
c = R_w / W
```

The whale reaches strict majority when `c > 0.5`. But the *cost* to reach that
threshold depends on how much honest counter-stake `R_h` is present:

```text
Minimum R_w for capture = 2B + R_h + 1 unit
```

With zero counter-stake (`R_h = 0`):

```text
R_w > 2B  ->  just over twice the base bond is enough to capture majority
```

With counter-stake equal to the base bond (`R_h = B`):

```text
R_w > 3B  ->  just over three times the base bond required
```

With counter-stake equal to five times the base bond (`R_h = 5B`):

```text
R_w > 7B  ->  just over seven times the base bond required
```

Each additional unit of honest counter-stake raises the whale's capture cost
one-for-one, so growing `W` with genuine third-party positions is the primary
defense.

### Role of `max_position`

Setting `max_position < max_total_weight` prevents a single address from
contributing all of `W`. If `max_position = p x base_bond`:

```text
Maximum single-address weight share = p / (2 + p) when R_h = 0
```

| `max_position` (multiples of `base_bond`) | Max share of W (no counter-stake) |
| ----------------------------------------- | --------------------------------- |
| 1x (equal to each fixed party)            | 33%                               |
| 2x                                        | 50% exactly; cannot reach majority |
| 3x                                        | 60%                               |
| 5x                                        | 71%                               |
| 10x                                       | 83%                               |
| unlimited (= `max_total_weight`)          | 100%                              |

**Key finding**: setting `max_position = 2 x base_bond` means a single address
with no counter-stake can only reach exactly 50%, which is not a strict majority.
Setting it at `3 x base_bond` allows capture with no counter-stake. This is the
most impactful single-parameter defense: even if no third party registers,
`max_position <= 2 x base_bond` prevents a whale from reaching majority on its own.

However, `max_position <= 2 x base_bond` also caps legitimate counter-stake, which
means a single large honest voter cannot provide the full majority on its own
either. Whether that is acceptable depends on the deployment's expected electorate
size. For a deployment expecting 3+ participants in a contested dispute, tight
position caps are advisable. For a two-party deployment, the caps may be
irrelevant.

### Self-hedging and the cost illusion

A coalition that controls both the asserter role *and* a whale-sized registration
deposit can attempt to control both the assertion and the majority vote. If the
coalition has asserter position `B` (agreeing) plus registration `R_w > W / 2`
(also agreeing), it controls majority at total cost `B + R_w`. If the honest
side wins instead, the `StrictMajorityAgainst` settlement rule pays only
positions where `agrees_with_outcome == Some(false)`; the asserter's
`Fixed(true)` position has `agrees_with_outcome == Some(true)` and is therefore
forfeited along with `R_w`. The coalition's real net exposure when it loses is
`B + R_w`, not `R_w` alone. There is no self-hedging benefit: the asserter bond
does not reduce the coalition's downside.

A deployer should therefore think of capture cost as **`B + R_w`** in both the
success and the failure case. The minimum capital at risk for a capture attempt
is `B + R_w`, where `R_w > 2B + R_h`. A `max_position` limit forces the whale
to assemble multiple addresses, but linear weight prevents splitting from
amplifying its combined vote; it only complicates logistics and raises capital
requirements slightly.

---

## Part 3: window-length tradeoff model

### Variables

| Symbol | Meaning |
| --- | --- |
| `T_reg` | `registration_duration_secs` |
| `T_rev` | `reveal_duration_secs` |
| `T_hard` | `anti_snipe_hard_max_secs` |
| `T_ext` | `anti_snipe_extension_secs` |
| `t_react` | Typical time for a new voter to notice the dispute and decide to register |
| `t_coord` | Time needed to coordinate reveals off-chain (signal, salt collection) |
| `t_atk` | Time an attacker needs to move capital into a sniped deposit |

### Registration window: participation vs. capture speed

A longer `T_reg` window increases expected third-party participation by giving
more potential voters time to notice the dispute and post bonds. Every unit of
honest counter-stake `R_h` added during registration raises the whale's capture
cost one-for-one (Part 2). But a longer window also extends the period during
which a well-funded attacker can observe accumulated positions, compute the
remaining gap to majority, and deploy exactly enough capital to cross it at
the last moment.

The tradeoff:

```
Participation benefit peaks when:  T_reg >= t_react (typical voter latency)
Capture risk is manageable when:   T_reg < time attacker needs to move capital
```

For most deployments these are the same order of magnitude (minutes to hours),
which is why anti-sniping exists. The practical guidance:

- Use `T_reg >= t_react` to capture genuine participation.
- Use `T_ext` (anti-snipe extension) to deter last-second position injections
  that arrive after honest voters cannot respond.
- Cap extensions at `T_hard` to prevent an adversary from grieving registration
  by repeatedly triggering extensions.

### Anti-sniping parameters

The soft deadline extends by `T_ext` when a qualifying deposit arrives within
`T_ext` seconds of the current deadline, but can never exceed `T_hard`. The
extension fires repeatedly if deposits keep arriving near the moving edge.

```text
Maximum extensions before hard cap:  floor((T_hard - T_reg) / T_ext)
```

For example, `T_reg = 3600`, `T_ext = 300`, `T_hard = 7200`:

```text
Maximum extensions: floor((7200 - 3600) / 300) = 12 extensions of 5 minutes each
```

An attacker who tries to snipe must arrive at least `T_ext` seconds before the
hard deadline or its deposit will push the deadline out, giving honest voters
time to respond. Each extension also exposes the attacker's deposit size, letting
honest counter-stake calculate the correct response.

Setting `T_ext` too small (e.g., 1 second) makes sniping easy; a last-second
deposit arrives and the extension is trivially short. Setting `T_ext` too large
(equal to `T_reg`) means a single late deposit nearly doubles the registration
window, which may itself be a griefing vector. A range of 5%-15% of `T_reg` is
generally appropriate.

### Reveal window: participation vs. delay

A longer `T_rev` gives registered voters more time to compute and submit their
reveal transaction. The risk of a short `T_rev` is that registered positions
silently miss the reveal deadline and become non-reveals, forfeiting their bond
and, more importantly, inflating `N` in a way that can prevent a legitimate
majority (Part 1).

However, every second of `T_rev` adds directly to the total time from `dispute`
to `Resolved`. The total dispute lifecycle is approximately:

```text
Total dispute time ~= T_hard + T_rev
```

The tradeoff is:

| `T_rev` choice | Effect on default frequency | Effect on finality latency |
| -------------- | --------------------------- | -------------------------- |
| Very short (< `t_coord`) | High: reveal coordination may fail | Low |
| Moderate (>= `t_coord`)  | Low: coordinators have time to act | Moderate |
| Very long                | Negligible marginal gain after saturation | High |

Recommended: set `T_rev >= t_coord` with at least a 2x safety margin. For most
disputes, `t_coord` is dominated by off-chain response time (monitoring, human
decision, client submission), not raw transaction throughput. 6 hours to 24 hours
is typical; sub-hour windows risk high default rates unless the deployment
guarantees automated reveal infrastructure.

### Window-length summary table

| Scenario | Longer window helps | Shorter window helps |
| -------- | ------------------- | -------------------- |
| High default frequency | `T_rev` (more reveal time) | n/a |
| Low third-party participation | `T_reg` (more registration time) | n/a |
| Last-second sniping | `T_ext`, `T_hard` | n/a |
| Slow resolution / time-sensitive outcomes | n/a | `T_reg`, `T_rev` |
| Griefing via repeated extensions | n/a | `T_hard` (set conservatively) |

---

## Part 4: recommended starting profiles

These profiles follow the same three-tier structure as [BOND_SIZING.md](BOND_SIZING.md).
They assume `min_resolution_bond = base_bond` (the contract enforces this). All
time values are in seconds; convert to your deployment's ledger approximation.

### Inputs to collect

Use token units consistently. The v1 inputs (`V_min`, `C_assert`, `C_dispute`,
`R_case`, `K_spam`, `K_dispute`, etc.) still apply to `base_bond`. The additional
v2 inputs are:

| Symbol | Meaning |
| --- | --- |
| `t_react` | Typical time for an interested third party to notice a dispute and register, in seconds. |
| `t_coord` | Time needed for off-chain reveal coordination: collecting salts, computing commitments, submitting transactions. |
| `N_voters` | Expected number of distinct third-party registrants in a contested dispute. |
| `max_honest_stake` | Expected total third-party honest counter-stake in a typical dispute, in token units. |
| `V_dispute` | Economic value that could be captured by falsely resolving one assertion. |

### Base-bond formula (extending v1)

The v2 minimum bond satisfies all v1 constraints plus one additional check:

```text
base_bond >= max(
  R_case / max(1, K_spam),
  R_case / max(1, K_dispute),
  target_attacker_loss - min(C_assert, C_dispute),
  min_finalizer_reward x 10_000 / max(1, reward_bps),
  V_dispute / capture_cost_multiplier   <- new in v2
)
```

`capture_cost_multiplier` is the minimum multiple of `base_bond` an attacker must
stake to reach majority with no counter-stake. From Part 2, with zero
counter-stake this is `2B + 1`, meaning `capture_cost_multiplier ~= 2`. With
realistic counter-stake the multiplier grows. Use `2` as the conservative
(zero-counter-stake) floor.

Affordability cap (unchanged from v1):

```text
base_bond <= V_min x max_acceptable_bond_share
```

### `max_position` guidance

Set `max_position` as a multiple of `base_bond`:

```text
max_position = k x base_bond,  k in [2, 20]
```

- `k = 2`: a single address cannot reach majority alone (even with no counter-stake).
  Maximally whale-resistant but restricts legitimate large-stake participants.
- `k = 5-10`: a single address needs at least 5-10x honest counter-stake before
  becoming a threat. Suitable for most deployments.
- `k = max_total_weight / base_bond`: no per-position constraint.
  Use only in private / trusted deployments.

### `max_total_weight` guidance

```text
max_total_weight = m x base_bond,  m >= max(10, N_voters x k + 2)
```

Set `m` large enough to accommodate the expected number of participants at full
`max_position` each, plus the two fixed parties. The contract arithmetic is
safe as long as `max_total_weight <= MAX_SETTLEMENT_TOTAL_WEIGHT`; the practical
guidance is to give yourself at least 5-10x headroom over the expected populated
`W` so legitimate counter-stake is never blocked.

### Profile table

| Profile | `base_bond` | `T_reg` | `T_ext` | `T_hard` | `T_rev` | `max_position` | `max_total_weight` | Use when |
| ------- | ----------- | ------- | ------- | -------- | ------- | -------------- | ------------------ | -------- |
| Private beta | 1x-2x spam floor | 1-4 h | 5 min | 2x `T_reg` | 4-12 h | 10x `base_bond` | 50x `base_bond` | Known users, coordinated reveals, low bot pressure. |
| Public testnet / low value | 2x-5x larger spam floor | 4-12 h | 10 min | 3x `T_reg` | 12-24 h | 5x `base_bond` | 100x `base_bond` | Open participation, moderate value, expect uncoordinated voters. |
| Higher-value mainnet candidate | 5x+ spam floor, within 5%-20% of `V_min` | 12-24 h | 15-30 min | 2x-4x `T_reg` | 24-48 h | 3x `base_bond` | 200x `base_bond` | Meaningful value, monitored reveals, audited deployment. |

### Narrative guidance per profile

#### Private beta

The electorate is known. Reveals can be coordinated quickly. Default frequency
is low because coordinators will not silently miss the reveal window. The main
risk is a single insider dominating a dispute, so `max_position = 10 x base_bond`
is an informational bound rather than a security control (all participants are
trusted). A 1-4 hour registration window is long enough for beta testers to
notice a dispute; a 4-12 hour reveal window is long enough for manual
coordination. Set `anti_snipe_extension_secs` conservatively at 5 minutes to
prevent last-second test disputes from stalling registration.

#### Public testnet / low value

The electorate is open but economic stakes are low. Default frequency is a
secondary concern because a wrong resolution has limited financial impact.
The primary concern is UX: long enough windows so new users can register and
reveal without requiring dedicated keeper infrastructure, short enough that
resolutions complete within a day or two. A 4-12 hour registration window with
10-minute anti-snipe extensions and a 12-24 hour reveal window is a reasonable
starting point. Set `max_position = 5 x base_bond` to require at least 5 units
of honest counter-stake before a single whale can threaten majority.

#### Higher-value mainnet candidate

Every parameter is tightened toward security. Registration is 12-24 hours to
allow global participation across timezones. The reveal window is 24-48 hours
to give coordinators a full working day to organize reveals. `max_position =
3 x base_bond` forces a whale to recruit multiple addresses and raises logistics
cost. The anti-snipe extension is longer (15-30 minutes) to give honest
participants meaningful response time to a late deposit. The hard cap is 2-4x
the base registration window to allow 4-8 meaningful extensions without
permitting indefinite delay. The bond floor is computed at 5x+ the v1 spam/dispute
floor, checked against `V_dispute / 2`, and capped at 5%-20% of `V_min`.

---

## Part 5: scenario checks

### Default frequency check

Before launch, verify that a realistic dispute can correct a false assertion.
With your chosen parameters:

1. Assume only the disputer (one `base_bond` against) plus `N_voters` third parties
   each posting `base_bond`.
2. Compute `W = 2 x base_bond x (1 + N_voters / 2)` under a symmetric scenario
   where half the third parties side with each party.
3. Against-side weight: `base_bond x (1 + N_voters / 2)`.
4. Required threshold: strictly more than `W / 2`. The contract uses the
   comparison `side_weight > W - side_weight` rather than `side_weight > W / 2`:
   integer division rounds down, which would wrongly pass a side sitting exactly
   at the boundary on an odd `W`. For an integer `W`, this means against-side
   weight must be at least `floor(W / 2) + 1`.

If `N_voters` is small (e.g., 1), then `W = 3 x base_bond`, against-side weight
is `1.5 x base_bond`, and the required threshold is `floor(3B / 2) + 1`; with
integer bonds, against-side needs at least `floor(3B / 2) + 1` units. For
`base_bond = 2` this is 4 against 3, requiring one more unit on the honest side.
A deployment expecting zero third parties cannot deterministically correct a false
assertion: the disputer's lone bond reaches exactly `W / 2` at `W = 2 x base_bond`,
which does not satisfy `side_weight > W - side_weight`.

**Rule of thumb**: budget `N_voters >= 3` realistically achievable third-party
registrants for a deployment that intends to correct false assertions without
relying on a dominant single voter. Note that `N_voters >= 3` is a floor for a
healthy electorate, not a sufficient condition for correction on its own. The
model above assumes a symmetric split of revealed weight, under which the
against-side weight equals exactly `W / 2` regardless of `N_voters`: correction
always requires an honest-side *imbalance* (more revealed weight against than
for), not just a larger headcount. A deployment with three third-party voters
all revealing on the honest side will correct a false assertion; a deployment
where half reveal for the asserted outcome will not, regardless of how many
participants registered.

### Whale-capture check

For your chosen `base_bond`, `max_position`, and expected `max_honest_stake`:

```text
Minimum capture bond = 2 x base_bond + max_honest_stake + 1 unit
```

If `minimum capture bond > max_position`, capture is impossible for a single
address regardless of counter-stake. If `minimum capture bond <= max_position`,
the whale risk exists and should be controlled by expecting `max_honest_stake`
to be present in contested disputes.

Worked example (higher-value mainnet candidate profile):

- `base_bond = 100` units
- `max_position = 300` units (3x)
- Expected honest counter-stake: `200` units (two third parties at `base_bond` each)

```text
Minimum capture: 2(100) + 200 + 1 = 401 units > max_position (300)
```

A single address cannot reach majority even with no honest counter-stake beyond
the two fixed parties. The deployment is whale-resistant by construction under
this example.

### Window-length sanity check

Verify:

```text
T_rev >= 2 x t_coord   (2x safety margin on reveal coordination)
T_reg >= t_react       (voters can notice and register)
T_ext >= t_atk / 10   (extension is nontrivial relative to attacker response time)
T_hard <= T_reg + 20 x T_ext  (hard cap prevents indefinite extension griefing)
```

If `T_rev < t_coord`, shorten the coordination process, add keeper automation, or
increase `T_rev`. If `T_hard` would exceed your acceptable total dispute lifetime
(`T_hard + T_rev`), tighten the anti-snipe parameters or reduce `T_ext`.

---

## Part 4: Withheld-reveal manipulation and reveal-quorum analysis

### The withheld-reveal attack vector

In protocol v2, an assertion disputed by a counterparty enters registration, where third
parties can bond additional stake. When registration closes, the eligible total `W` is
frozen. Outcome determination relies on two distinct mechanisms:

1. **Strict majority**: if either side reveals strictly more than `W / 2`, that side locks
   immediately as `StrictMajorityFor` or `StrictMajorityAgainst`.
2. **Optimistic timeout default**: if neither side reaches strict majority before
   `reveal_deadline`, the originally asserted outcome stands as `OptimisticTimeout`.

Without a reveal quorum, this structure introduces a strategic withholding vulnerability
for malicious asserters. Consider an attacker who asserts a false claim and faces an honest
challenger:

- Asserter posts `base_bond = B`.
- Disputer posts `base_bond = B`.
- Honest third-party participants observe the false assertion and register stake `S_H` on the
  disagreeing side.
- To prevent honest defenders from reaching strict majority, the attacker registers stake
  `S_A` during registration under sybil addresses.
- At the registration cutoff, eligible weight is frozen at:
  ```text
  W = 2B + S_H + S_A
  ```

For honest defenders to overturn the false claim by strict majority, revealed disagreeing
weight must exceed `W / 2`:
```text
B + S_H > W / 2 = B + (S_H + S_A) / 2
  -> S_H > S_A
```

If the attacker has bonded `S_A >= S_H`, honest defenders cannot reach strict majority.
Now consider the reveal phase:
- If the attacker reveals `S_A` for the assertion, it risks honest voters observing the
  proceedings or counter-reveals occurring. More importantly, if `S_A < B + S_H`, revealing
  does not guarantee victory.
- If the attacker **intentionally withholds revealing** `S_A`, then `S_A` remains silent. It
  sits in the denominator `W` without casting votes.
- Disagreeing revealed weight is `B + S_H <= W / 2`.
- Agreeing revealed weight is `B < W / 2`.
- When `reveal_deadline` passes, neither side has reached a strict majority.

Without a reveal quorum, `resolve_outcome` resolves as `OptimisticTimeout`. The false assertion
stands. Under timeout settlement:
- All revealed positions (`2B + S_H`) recover principal and share the forfeited silent stake `S_A`.
- The attacker forfeits `S_A`.
- However, if the external value `V` extracted by finalizing the false assertion exceeds the
  forfeited stake `S_A` (`V > S_A`), the attack yields a net positive profit of `V - S_A`.

By flooding registration with silent capital, an attacker could artificially suppress honest
voters below `W / 2` and force an optimistic timeout victory.

---

### Mathematical model and quorum defense

To eliminate this manipulation vector, `PolicySnapshotV2` introduces `reveal_quorum_bps`:
a minimum basis point threshold (where `10_000 bps = 100%`) of eligible weight `W` that must
be revealed before an optimistic timeout default is permitted to resolve.

Let:
- `Q = reveal_quorum_bps / 10_000` (where `0 <= Q <= 1`).
- `R = F + A` be total revealed weight at deadline expiration.

The contract enforces:
```text
R x 10_000 >= W x reveal_quorum_bps
```

If `terminal_cause == NotYetDecided` at `reveal_deadline` and `R < Q x W`, `resolve_outcome`
aborts with `Error::RevealQuorumNotMet`.

#### Proof of defense against withheld-reveal suppression

Assume an attacker attempts to suppress an honest majority `S_H` by withholding stake `S_A`.
At the deadline, revealed weight is at most:
```text
R = 2B + S_H
```
(assuming both fixed parties and all honest third-party voters reveal).

For the attack to succeed, the attacker must simultaneously satisfy two conflicting conditions:

1. **Suppression condition (prevent strict majority)**:
   ```text
   B + S_H <= W / 2 = B + (S_H + S_A) / 2
     -> S_A >= S_H
   ```

2. **Quorum condition (allow optimistic timeout to resolve)**:
   ```text
   R >= Q x W
     -> 2B + S_H >= Q x (2B + S_H + S_A)
     -> (1 - Q)(2B + S_H) >= Q x S_A
     -> S_A <= ((1 - Q) / Q) x (2B + S_H)
   ```

Combining both bounds yields the attacker's feasible withholding interval:
```text
S_H <= S_A <= ((1 - Q) / Q) x (2B + S_H)
```

A non-empty feasible interval requires:
```text
S_H <= ((1 - Q) / Q) x (2B + S_H)
  -> Q x S_H <= (1 - Q)(2B + S_H) = 2B(1 - Q) + S_H - Q x S_H
  -> (2Q - 1) x S_H <= 2B(1 - Q)
```

#### Analysis for standard parameter `Q = 0.5` (`reveal_quorum_bps = 5_000`):

When `Q = 0.5`:
```text
(2(0.5) - 1) x S_H <= 2B(1 - 0.5)
  -> 0 <= B
```
This is trivially satisfied, but examine the upper bound on `S_A`:
```text
S_A <= ((1 - 0.5) / 0.5) x (2B + S_H) = 2B + S_H
```
Together with the suppression condition `S_A >= S_H`, the attacker is restricted to:
```text
S_H <= S_A <= S_H + 2B
```

If honest participants register significant stake relative to the base bond (`S_H >> 2B`),
any meaningful suppression requires `S_A` significantly exceeding `S_H`. Specifically, if
`S_A > 2B + S_H`, total revealed turnout strictly satisfies:
```text
R / W = (2B + S_H) / (2B + S_H + S_A) < (2B + S_H) / (2B + S_H + 2B + S_H) = 1/2
```
Turnout strictly drops below 50%. Quorum fails (`R x 10_000 < W x 5_000`).

#### Consequence of quorum failure

When quorum fails:
1. `resolve_outcome` fails with `RevealQuorumNotMet`.
2. The assertion remains undecided in `PhaseV2::Reveal`.
3. The malicious claim **does not resolve** and cannot finalize.
4. The admin can invoke `set_paused_v2(true)` followed by `cancel_round(id)`. Under
   `cancel_round`, every participant (including honest voters and the disputer) is refunded
   their exact bonded principal in full. The attacker gains nothing, external capture is
   thwarted, and defender capital is protected.

If the attacker instead reveals `S_A` to satisfy quorum, the withholding attack is broken:
its revealed votes enter the tallies, allowing defenders to contest the dispute directly
via strict majority.

---

### Guidance on choosing `reveal_quorum_bps`

The choice of `reveal_quorum_bps` trades off attack resistance against dispute liveness.

| `reveal_quorum_bps` | Quorum % | Liveness vs. Security Tradeoff | Recommended Use Case |
| ------------------- | -------- | ------------------------------ | -------------------- |
| `5_000` | 50% | **Standard Default**: Aligns quorum with the strict-majority threshold (50%). Guarantees that at least half of the locked capital participates before status-quo timeout can bind. Eliminates large-scale silent withholding. | General-purpose oracle deployments, price feeds, prediction markets. |
| `6_000` – `7_500` | 60% – 75% | **High Security / Low Tolerance**: Requires substantial turnout. Narrows the attacker's withholding window to near zero even for small third-party stakes. Increases the risk of deadlock if benign voter apathy occurs. | High-value assertions securing large TVL, bridge settlements, governance gates. |
| `2_500` – `3_300` | 25% – 33% | **High Liveness / Apathy Permissive**: Allows timeout resolution even under low turnout. Reduces deadlock risk from uncoordinated small voters, but allows larger withholding bands before quorum triggers. | Micro-claims, high-frequency assertions with small capital at risk. |
| `0` | 0% | **Unconstrained Timeout (Legacy v2)**: Quorum check is disabled. Optimistic timeout fires whenever deadline passes regardless of turnout. Fully vulnerable to withheld-reveal suppression. | Private/consortium chains or testing environments where all participants are trusted. |

**Recommendation**: Set `reveal_quorum_bps = 5_000` (50%) for standard production deployments.
Pair this with a reveal duration `T_rev` that gives registered participants ample time and keeper
tooling to submit reveals.

---

## Monitoring and adjustment

Review after every testnet campaign and before mainnet launch:

- Fraction of disputes that end in `OptimisticTimeout` vs. `StrictMajority`. A
  high timeout rate with many registered positions suggests the reveal window is
  too short or coordination is breaking down. A high timeout rate with few or no
  third-party registrants may indicate the registration window or bond floor is
  too high for the expected electorate.
- Distribution of `W` across disputes. If `W ~= 2 x base_bond` consistently,
  third-party participation is absent; revisit window lengths and bond sizing.
- Non-reveal rate among registered third-party positions. High non-reveal rates
  inflate `N` and drive default frequency. Investigate whether `T_rev` is too
  short, whether keepers are missing reveals, or whether registrants are
  strategically abstaining.
- `max_position` hits. If deposits are being rejected for exceeding `max_position`,
  either the cap is too tight for the dispute's capital or a whale is probing.
  Examine addresses to distinguish the two.
- Total dispute lifecycle time (from `dispute` to `Resolved`). If disputes are
  consistently completing well inside the maximum window, the windows may safely
  be shortened for future deployments; if they are consistently running to the
  deadline, lengthen the reveal window first.

Raise `base_bond` if capture attempts cluster around small numbers of addresses
or if false assertions survive repeated disputes. Lengthen `T_rev` if the
non-reveal rate is high. Tighten `max_position` if large single-address positions
appear in contested disputes. Revisit the full calculation whenever the token
price, assertion value distribution, or expected electorate changes materially.
