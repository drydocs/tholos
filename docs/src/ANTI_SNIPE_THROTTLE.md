# Anti-snipe extension: does it need a per-position throttle?

This note answers the question posed in issue #213: given the hard-deadline
bound and the real-capital floor #155 added, is a per-position throttle worth
adding? It follows the same shape as `V1_MAINNET_PARAMETERS.md` and
`V2_BOND_SIZING.md`.

**Conclusion up front:** no throttle is warranted at any parameter values the
contract currently permits, and the reason is a property of the mechanism
rather than a judgement about attacker budgets. A throttle keyed on "this
position has already extended" cannot raise the cost of reaching the hard cap,
because that cost is fixed by `min_resolution_bond` and the extension
count, not by which address the deposits come from. The recommendation is to
keep `T_hard` conservative and leave the mechanism alone. The bound under
which that stops holding is derived in Part 4.

`V2_BOND_SIZING.md` already derives the extension count:

    Maximum extensions before hard cap:  floor((T_hard - T_reg) / T_ext)

What it does not do is price them, or ask what the attacker receives in return.
Those are the two questions this note adds.

---

## Part 1: what an extension actually costs

The trigger is a qualifying deposit landing within `T_ext` of the current
soft deadline. After #155, "qualifying" means at least
`policy.min_resolution_bond`, and `initialize` pins
`min_resolution_bond = base_bond` by construction, so the floor is the
same bond the original parties posted.

| Symbol | Meaning |
| --- | --- |
| `B` | `base_bond`, in token units |
| `T_reg` | `registration_duration_secs` |
| `T_ext` | `anti_snipe_extension_secs` |
| `T_hard` | `anti_snipe_hard_max_secs` |
| `P_max` | `max_position` |
| `W_max` | `max_total_weight`, bounded by `10 x P_max` |

Reaching the hard cap requires

    E = ceil((T_hard - T_reg) / T_ext)     qualifying deposits

because the soft deadline advances by at most `T_ext` per trigger and the
gap to close is `T_hard - T_reg`. Each of those deposits is at least
`B`, so:

    capital required to pin the deadline at T_hard:   E x B

Once `registration_deadline == registration_hard_deadline` the trigger
saturates: `min(now + T_ext, T_hard)` keeps returning `T_hard`, so
**no further deposits are needed to hold the cap.** The cost is paid once, not
per unit of delay held. With the worked example from `V2_BOND_SIZING.md`
(`T_reg = 3600`, `T_ext = 300`, `T_hard = 7200`, so
`E = 12`), pinning the cap costs `12 x B`.

## Part 2: what an extension buys

This is where the intuition in the issue's framing needs care, and it changes
the recommendation.

The extension does not grant the attacker a window to act unopposed. It pushes
the deadline out **for everyone**, and registration is permissionless: any third
party can deposit during the extended window. The mechanism's stated purpose —
in the code comment, in `CONTRACT_V2.md`, and in the issue body — is to
give honest late arrivals time to respond to a last-second injection.

So a triggered extension is simultaneously:

- **delay** for whoever wants the dispute resolved, and
- **more reaction time** for honest third parties, which is the mechanism
  working as designed.

An attacker who commits `E x B` of at-risk capital to buy
`T_hard - T_reg` seconds is spending that capital to hand the honest side
a longer window in which to counter-stake. Treating "an attacker can push the
deadline" as self-evidently harmful skips the step where that delay is shown to
favour the attacker.

The genuine cost is **finality latency**: `T_hard + T_rev` is the time
from dispute to `Resolved@, and a griefer can push a dispute toward that
maximum. That is a liveness cost, bounded by `T_hard`, and it is better
addressed by choosing `T_hard` than by adding mechanism.

Note also that the capital is not consumed. It is a real position at risk: if
the outcome goes against the side it was committed to, it is forfeited. The
deterrent is therefore the at-risk amount, which is the same quantity
`V2_BOND_SIZING.md` already sizes `base_bond` against.

## Part 3: why a per-position throttle cannot work

The issue proposes one of two mechanisms: a per-position "already extended"
flag, or restricting the trigger to first-time deposits. Both are keyed on the
**address**, and the capital cost above is not.

Let `M = floor(P_max / B)` be the number of qualifying deposits one
address can make before hitting `max_position`. There are two cases, and
neither is helped:

| Case | What happens | Does a per-position throttle help? |
| --- | --- | --- |
| `M >= E` | One address pins the cap by itself | **No.** Blocking that address forces a second one, at identical total cost |
| `M < E` | The attacker must split across `ceil(E / M)` addresses | **No.** Each sybil's *first* deposit already had to clear `B` before #155 and still does, so the total is `E x B` either way |

In both cases the throttle changes which addresses hold the capital, not how
much capital is required. The floor #155 added is what sets the price, and it
applies to first deposits as well as top-ups — which is the same statement as
the issue's observation that "a Sybil actor was never affected by #155".

The one thing a per-position throttle *would* do is change behaviour for a
genuine participant whose first deposit lands late and who then wants to add to
a position near the deadline — the participant the mechanism exists to protect.
That cost is real and the benefit is nil.

## Part 4: when the conclusion would change

The argument above holds while the hard deadline is the binding constraint. It
stops holding if the delay a griefer buys becomes large relative to
`T_hard` itself, i.e. if `T_hard - T_reg` approaches
`T_hard`.

The contract already bounds that geometry. `initialize` rejects
`anti_snipe_hard_max_secs < registration_duration_secs`, and permits at
most `MAX_ANTI_SNIPE_HARD_MAX_SECS = 29 days` against
`MAX_REGISTRATION_DURATION_SECS = 7 days`, so a deployment can choose a
hard cap between 1x and roughly 4.1x its base window.

The number to watch is therefore not the attacker's budget but the ratio:

    griefing-headroom = (T_hard - T_reg) / T_reg

At the sizing guidance in `V2_BOND_SIZING.md` (`T_ext` at 5-15% of
`T_reg`, `T_hard` set conservatively), the headroom is small and the
delay bought is a modest multiple of a window honest participants were already
expected to respond within. Keep it there: choose `T_hard` as the smallest
value that still admits a genuine late arrival, and do not raise it on the
assumption that extensions are an attack.

**If a future deployment does want a throttle**, the shape that would change the
economics is not per-position but per-*trigger*: requiring the qualifying deposit
to grow with each successive extension — a rising floor, e.g. `B x 2^k` for
the k-th extension — would make `E` extensions cost of order `2^E`
rather than `E`. That is a materially different mechanism with a materially
different storage and accounting cost, it changes behaviour for late-arriving
honest participants just as much as the per-position variant, and it is not
needed at any parameter set the contract permits today. It is recorded here as
the shape to evaluate if the bound above is ever reached, and deliberately not
implemented under this issue.

---

## Recommendation

1. **Do not add a per-position throttle.** It cannot raise the cost of
   saturating the hard cap, as shown in Part 3, and it penalises genuine late
   top-ups.
2. **Keep `T_hard` conservative**, and treat
   `(T_hard - T_reg) / T_reg` as the number to watch. That is already the
   guidance in `V2_BOND_SIZING.md`; this note supplies the reasoning for
   why it is sufficient.
3. **No follow-up implementation issue is warranted** from this analysis. If a
   future deployment adopts a hard cap far above its base registration window,
   the rising-floor variant in Part 4 is the mechanism to evaluate then.
