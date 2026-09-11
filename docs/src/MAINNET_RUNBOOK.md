# Mainnet operational runbook

**This is prep work, written ahead of the audit clearing, not a signal that
mainnet deployment is currently authorized.** [SECURITY.md](SECURITY.md) is
unambiguous: Tholos has not had an external security audit, and isn't for
mainnet deployments securing meaningful value until it has one. Nothing in
this document changes that. It exists so the operational half of a mainnet
launch — who holds which keys, what order to do things in, who gets paged
when — is worked out and reviewed now, rather than drafted under time
pressure the day the audit clears. Treat every "should" below as a plan to
review at that point, not a plan already put into action.

[DEPLOYMENT.md](DEPLOYMENT.md) already covers parameter selection (backed by
[V1_MAINNET_PARAMETERS.md](V1_MAINNET_PARAMETERS.md)), the deploy sequence,
and the on-chain mechanics of the admin runbook.
[RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md) already covers resolver key
custody, onboarding, and offboarding in depth. This document doesn't repeat
either. It covers what's still missing: admin key custody specifically (the
one role neither existing document addresses), a concrete go/no-go launch
sequence tying the existing readiness checklist into an actual procedure,
and incident escalation specifics for a mainnet context.

## Part 1: admin key custody

[RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md#key-generation-and-custody)
already lays out the custody spectrum (plain key, hardware wallet, multisig)
for resolver seats. The admin role needs the same kind of decision, but its
risk profile is different enough to warrant its own treatment: a single
address controls `set_paused`, `set_bond_amount`, `set_stall_timeout`,
`update_resolvers`, and `propose_admin` — every lever that doesn't require
resolver-committee consensus. Unlike a resolver, the admin acts alone; there
is no majority-vote check on any admin call the way there is on `resolve`.

### What's actually at stake if the admin key is compromised

The admin's on-chain powers are all *operational*, not *custodial* — nothing
in this list lets the admin move a bond or an assertion's funds to an
arbitrary address. But an attacker with the admin key can still do real
damage:

- `set_paused(true)` freezes the entire deployment (no new assertions,
  disputes, resolves, or finalizes) for as long as the attacker holds the
  key or until detected and countered.
- `update_resolvers` replaces the entire committee with addresses the
  attacker controls, after which every future dispute resolves however the
  attacker wants — a materially worse outcome than pausing, since it's not
  obviously visible as an incident until a dispute actually resolves wrong.
- `set_bond_amount` and `set_stall_timeout` can be set to values that grief
  future assertions (an unaffordable bond, a stall timeout so short
  legitimate disputes get force-reclaimed, or so long a stalled one is
  frozen indefinitely) without being an outright fund-theft mechanism.
- `propose_admin` to an attacker-controlled address, followed by
  `accept_admin` from that address, completes a full admin takeover. The
  two-step design (see [DEPLOYMENT.md](DEPLOYMENT.md#rotating-the-admin-key))
  protects against a *mistyped* address, not a *malicious* one with its own
  signature ready.

### There is no on-chain recovery from a lost or compromised admin key

This is the fact custody planning has to be built around: `propose_admin`
requires the *current* admin's signature (see
[CONTRACT.md](CONTRACT.md#propose_adminnew_admin)). If the admin key is
destroyed, lost, or seized before a rotation is proposed, there is no
contract-level path to recover the role — the deployment is stuck with
whatever admin-gated state it was last in (paused or not, whatever the
resolver committee and bond amount were) permanently. This is a deliberate
trade-off, not an oversight: any recovery mechanism that didn't require the
current admin's signature would itself be a takeover path. The mitigation is
entirely in custody strength and rotation discipline, not in anything the
contract can do after the fact.

### Custody recommendations

- **Never a plain single key for a mainnet deployment with real value at
  stake.** The admin's blast radius (every lever above, exercisable
  unilaterally) is larger than any single resolver seat's, so the weakest
  custody tier that's acceptable for a low-value resolver seat isn't
  acceptable here.
- **Multisig is the reasonable default**, not just an option for
  organizational resolver seats the way
  [RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md#key-generation-and-custody)
  frames it for resolvers. The admin role has no per-action peer check the
  way `resolve` does, so a multisig is the only way to introduce one: no
  single compromised or rogue signer can unilaterally pause, rewrite the
  committee, or take over the role.
- **Decide the signer set and threshold before deploying, not after.** Who
  holds a key, how many signatures an admin action requires, and how a
  signer is replaced if they leave or their key is compromised, are all
  things `propose_admin`/`accept_admin` can execute once decided, but the
  contract has no opinion on any of it. Document the decision the same way
  [RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md) asks resolver commitments
  to be documented, so it's auditable rather than tribal knowledge.
- **Plan the rotation cadence and the compromise-response path together.**
  A periodic planned rotation (e.g. annually, or on signer turnover) uses
  `propose_admin`/`accept_admin` normally. A *suspected* compromise needs
  the same call made under time pressure, racing an attacker who may have
  the same key — decide in advance who's authorized to initiate an
  emergency rotation and how fast the remaining signers can produce a
  quorum, rather than discovering the answer during the incident. See
  Part 3's escalation guidance for the detection side of this.

## Part 2: go/no-go launch sequence

[DEPLOYMENT.md](DEPLOYMENT.md#mainnet-readiness-checklist)'s checklist
states *what* should be true before launch. This section is the *order* to
verify and execute it in, and what happens if something goes wrong in the
first hours and days after.

### Before the launch window

1. **Confirm every unchecked item on the readiness checklist is either
   checked or explicitly accepted as a known gap.** In particular:
   independent security audit, and real-world dispute volume (all testing
   to date is synthetic). Launching against real value with either
   unaddressed is a decision someone with the authority to accept that risk
   needs to make explicitly, not an oversight.
2. **Finalize and sign off on parameters** (`bond_amount`,
   `challenge_window_secs`, `finalize_reward_bps`, resolver committee) per
   [V1_MAINNET_PARAMETERS.md](V1_MAINNET_PARAMETERS.md) and
   [BOND_SIZING.md](BOND_SIZING.md). `challenge_window_secs` and
   `finalize_reward_bps` are written once, inside `initialize`, and
   permanently fixed at deployment with no setter at all — get them right
   before deploying, not after. `bond_amount` and the resolver committee
   are not permanently fixed the same way: `bond_amount` has its own admin
   setter (`set_bond_amount`), and the committee can change post-deployment
   via the admin's `update_resolvers` (see Part 1) or resolver
   self-rotation — both are correctable later, unlike the other two.
3. **Confirm every resolver has completed onboarding**
   per [RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md#onboarding-a-new-resolver):
   custody model in place, response-time commitment made, conflict-of-interest
   disclosure done. Do not launch with a placeholder or stopgap committee the
   way the canonical testnet deployment explicitly does — that caveat exists
   precisely to distinguish testnet convenience from a mainnet-ready
   committee.
4. **Confirm the admin custody model is in place** per Part 1: signer set
   assembled, threshold agreed, and — if using a multisig — the multisig
   itself deployed and tested with a harmless call (e.g. `set_paused(false)`
   when already unpaused, a genuine no-op) before it holds the real admin
   role.
5. **Assemble the incident response roster** per Part 3: who gets paged, in
   what order, and confirm they know it before launch, not after the first
   incident.
6. **Sign-off.** Whoever has the authority to accept the residual risk in
   step 1 explicitly approves proceeding, on the record (a dated
   comment, a signed statement, whatever the deployment's own governance
   requires) — not an implicit "nobody objected."

### Launch

7. **Deploy and `initialize`** per
   [DEPLOYMENT.md](DEPLOYMENT.md#deploying), substituting the
   mainnet-specific values from steps 2–4 for the testnet example values
   shown there.
8. **Verify the submitted `initialize` transaction's arguments match what
   was signed off on** before announcing the contract id publicly or
   pointing any real value at it. This contract has no getters for
   `bond_amount`, `challenge_window_secs`, `finalize_reward_bps`, the
   resolver set, or the admin address (`get_assertion_state(id)` is the
   only public read entrypoint, and it returns per-assertion state, not
   configuration — see [CONTRACT.md](CONTRACT.md)), and `initialize` emits
   no event either, so there is no independent on-chain query to check
   these against after the fact. The only real verification available is
   the `initialize` transaction itself: read back its submitted arguments
   from a block explorer (or from your own CLI output, before it's even
   confirmed) and diff them against step 2–4's signed-off values. A
   mismatch here (a typo in a CLI argument, the wrong token address) is far cheaper to catch before anyone has
   posted an assertion than after.
9. **Run one low-stakes assertion through the full happy path** (assert,
   wait out the challenge window, finalize) before directing real users or
   real value at the deployment, the same way
   `scripts/testnet-smoke.sh` does on testnet. This confirms the deployed
   bytecode and configuration actually behave as expected against real
   network conditions, not just in the test suite.

### First hours and days after launch

10. **Heightened monitoring for a defined initial window** (a period the
    launch sign-off should set explicitly, e.g. 72 hours or the first N
    assertions) — closer attention than steady-state operation, since this
    is when a misconfiguration or an attack targeting the newly-live
    deployment is most likely to surface and most damaging if missed.
11. **Rollback plan, scoped to what's actually possible.** There is no
    "undo the deployment" primitive — funds already locked in open
    assertions can't be un-locked except through the contract's own
    dispute/resolve/finalize/`reclaim_stalled_dispute` paths. What *is*
    available as a rollback lever if something looks wrong early:
    - `set_paused(true)` immediately, stopping new activity while the
      issue is assessed (see Part 3).
    - Steering users away from the deployment (stop advertising the
      contract id, redirect integrators) if the issue is severe enough
      that continuing to onboard new assertions would compound it, even
      though pause alone doesn't prevent that.
    - There is no path to migrate already-open assertions to a fixed
      redeployment; a genuinely broken deployment's open assertions have
      to be wound down through the existing dispute/finalize/reclaim
      mechanisms, however long that takes.
    Decide, before launch, who has the authority to trigger this pause
    unilaterally during the heightened-monitoring window without waiting
    for the normal incident escalation in Part 3 to run its course — early
    hours after launch is exactly when speed matters most and confidence
    in root cause is lowest.

## Part 3: incident escalation

### Who gets paged, how fast

Not prescribed as specific names or tools here, since that's
deployment-specific infrastructure, but the decision to make in advance:

- **A tier for anything touching the admin key** (a suspected compromise,
  an admin action nobody recalls authorizing) — page immediately, all
  admin-multisig signers, regardless of time of day. This is the single
  highest-severity category: Part 1 established there's no on-chain
  recovery from a lost admin key, so speed in the window before an
  attacker acts is the only lever available.
- **A tier for resolver-committee problems** (a resolver reports a
  compromised key, a dispute has been open unusually long without votes) —
  page the remaining resolvers and whoever holds the admin key, on a
  shorter fuse than routine operations but not necessarily the same
  immediate all-hands as an admin-key incident.
- **A tier for contract-behavior anomalies** (a transaction failing in an
  unexpected way, state that doesn't match what the code should produce) —
  page whoever can assess whether this is a real bug versus a
  misunderstanding of expected behavior, since `set_paused` is cheap
  insurance while that assessment happens (see the decision tree below).

### Decision tree: set_paused vs. resolver rotation vs. reclaim vs. wait

| Situation | Action | Why |
| --- | --- | --- |
| A dispute resolves in a way that looks wrong, but the resolvers who voted did so within their normal process | **Wait.** Let the normal dispute flow run. | A resolution you disagree with isn't necessarily a broken resolution; second-guessing individual votes after the fact undermines the entire point of having a committee. Revisit resolver composition (per [V1_MAINNET_PARAMETERS.md](V1_MAINNET_PARAMETERS.md#part-3-resolver-committee-size-and-composition)) if this becomes a pattern, not a one-off override. |
| A specific dispute has been open far longer than `challenge_window_secs` would suggest, with no votes landing | **`reclaim_stalled_dispute`**, if `set_stall_timeout` is configured (see its doc comment in `contracts/tholos/src/lib.rs`; not yet in `CONTRACT.md`) — otherwise consider `update_resolvers` if the cause is a genuinely dead committee. | This is exactly the scenario the stall-timeout fallback exists for: a specific stuck case, not a systemic problem. It resolves that one dispute (no-winner, both bonds returned) without touching the committee itself. |
| A resolver reports (or is suspected of) a compromised key | **`update_resolvers`** (admin override), immediately, per [RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md#choosing-self-rotation-vs-the-admin-override) | This is explicitly the emergency case self-rotation isn't designed for: don't wait for a majority vote that the compromised seat could itself help block or corrupt. |
| A resolver resigns, discloses a conflict, or is inactive, with no urgency | **Self-rotation** (`propose_rotation`/`vote_rotation`) | The routine case; keeps the decision inside the committee per [RESOLVER_GOVERNANCE.md](RESOLVER_GOVERNANCE.md#choosing-self-rotation-vs-the-admin-override), no admin action needed. |
| A bug is suspected in contract behavior itself (not a specific dispute's outcome), or vote/finalize/dispute behavior looks inconsistent with the documented spec | **`set_paused(true)`**, immediately, investigate second | Per [DEPLOYMENT.md](DEPLOYMENT.md#pausing-during-an-incident): a `Pending` assertion whose window elapses during a pause simply waits rather than finalizing uncontested, so pausing costs little while buying time to assess whether this is a real bug. |
| Any signal the admin key itself may be compromised | **Emergency `propose_admin`/`accept_admin` rotation**, racing to complete it before an attacker acts, using whatever quorum the admin custody model requires | Per Part 1: there is no on-chain recovery once an attacker completes their own rotation first, so this is the one scenario where speed dominates every other consideration, including normal multisig-quorum-gathering timelines. |

The common thread: reach for `set_paused` when the *contract's behavior*
itself is in question, reach for the resolver-rotation paths when the
*committee* is the problem, reach for `reclaim_stalled_dispute` when it's
one specific stuck case rather than a systemic one, and do nothing when the
process worked as designed even if a specific outcome is unwelcome.
Treating every unwelcome outcome as an incident erodes the credibility of
the committee the same way never intervening when something is genuinely
broken erodes trust in the deployment.
