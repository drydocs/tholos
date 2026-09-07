# Resolver committee governance: onboarding and offboarding

This is the human, off-chain half of running a resolver committee. See
[DEPLOYMENT.md](DEPLOYMENT.md#rotating-the-resolver-committee) for the on-chain
mechanics (`update_resolvers`, `propose_rotation`, `vote_rotation`) and
[ROTATION_DESIGN.md](ROTATION_DESIGN.md) for why self-rotation works the way it
does. Neither of those documents says who should hold a resolver seat, how that
person secures their signing key, what they're expected to do when a dispute
lands, or what actually happens the day a resolver needs to be replaced. This
document is that operational layer, for anyone running a real (non-toy)
deployment.

None of this is enforced by the contract. `initialize`, `update_resolvers`, and
`propose_rotation` accept any distinct addresses; nothing on-chain checks who
controls them, how the key was generated, or whether the person behind it agreed
to anything. The guarantees below come entirely from the deployment operator
choosing to follow this process and documenting that they have.

## Onboarding a new resolver

### Selection criteria

[V1_MAINNET_PARAMETERS.md](V1_MAINNET_PARAMETERS.md#part-3-resolver-committee-size-and-composition)
already establishes the two properties that matter most for anyone on the
committee: reachability within `challenge_window_secs`, and independence from
the other resolvers (no shared employer, custody provider, or coordination
channel that turns several addresses into one correlated point of failure).
Onboarding is where those properties get checked before a seat is handed out,
not after:

- **Domain competence.** A resolver has to be able to actually judge the
  claims this specific deployment adjudicates. What that means varies by
  deployment: for a freelance-escrow use case (see `demos/freelance-escrow`)
  it might mean familiarity with the kind of work being disputed; for an
  oracle-style deployment it might mean the ability to verify a claimed
  real-world outcome from public sources. Generic trustworthiness isn't a
  substitute for being able to tell a good assertion from a bad one in the
  deployment's actual domain.
- **Demonstrated reachability.** Ask for evidence, not a promise: a track
  record of availability on a timescale compatible with `challenge_window_secs`
  for this deployment. Someone who is highly qualified but unreachable within
  the window is worse for the committee than someone less qualified who
  reliably responds.
- **Independence from the existing committee and from likely parties.** Screen
  for shared employer, shared custody provider, and shared off-chain
  coordination (the same group chat, the same on-call rotation) with current
  resolvers, per the composition guidance already linked above. Also screen
  for relationships to parties likely to assert or dispute on this deployment;
  see the conflict-of-interest section below for why this matters beyond what
  the contract itself checks.
- **No conflicting role in the deployment.** A resolver shouldn't also be an
  operator with an economic stake in outcomes (e.g. the deployer taking a cut
  of disputed bonds) beyond the ordinary `finalize_reward_bps` mechanism, which
  applies uniformly and isn't resolver-specific.

### Key generation and custody

The resolver's signing key is what `resolve`, `propose_rotation`, and
`vote_rotation` authenticate against, so its custody model should match what
the resolver seat is actually worth to an attacker: the value of the disputes
it's likely to swing, not the deployment's total value locked.

- **Plain single key (`stellar keys generate`).** What the canonical testnet
  deployment uses today, and reasonable for a low-value or clearly-labeled
  testnet/staging deployment where the committee itself is a stopgap (see
  [DEPLOYMENT.md](DEPLOYMENT.md#canonical-testnet-deployment)'s own caveat
  about `resolver1`/`resolver2`/`resolver3`). Fastest to set up, weakest
  guarantee: a single lost or leaked key is a fully compromised resolver seat
  with no recourse except rotating them out.
- **Hardware wallet.** Recommended default once a deployment has any real
  value at stake. The resolver holds one address's worth of signing power on
  a device that never exposes the private key to a networked machine.
  Meaningfully raises the bar against remote compromise without adding
  operational complexity for the resolver's day-to-day `resolve` calls, and
  doesn't require coordinating with anyone else to vote.
- **Multisig behind the resolver's address.** Appropriate when a resolver
  seat represents an organization rather than an individual (e.g. a curated
  public list, or the "broad, visible representation" case
  [V1_MAINNET_PARAMETERS.md](V1_MAINNET_PARAMETERS.md#part-3-resolver-committee-size-and-composition)
  describes for 7+-member committees), or when a single resolver's vote
  carries enough weight that no one person should be able to cast it
  unilaterally. Adds coordination overhead to every single vote, which cuts
  against reachability within the challenge window; weigh that cost against
  the deployment's actual value at stake before choosing this over a hardware
  wallet.

Whichever model is chosen, the resolver (not the deployment operator) should
control the key material. The operator's role is validating that a reasonable
custody model is in place before proposing the address for the committee, not
holding the key on the resolver's behalf.

### Commitments a resolver makes

Before an address is proposed for `initialize`, `update_resolvers`, or
`propose_rotation`, the person behind it should explicitly agree to:

- **A response-time commitment compatible with `challenge_window_secs`.**
  State it in concrete terms (e.g. "will vote on an open dispute within N
  hours of being notified") rather than "will be reachable," so both sides can
  later judge objectively whether it was kept. This is the off-chain
  counterpart to the reachability criterion above, made auditable.
- **Conflict-of-interest disclosure, both at onboarding and ongoing.** `resolve`
  rejects a vote where the resolver is also the assertion's `asserter` or
  `disputer` (`SelfVote`, added in PR #203, closing #165). That check is necessarily
  narrow. It only catches the resolver being a direct party to the specific
  dispute in front of them. It cannot catch a resolver with an undisclosed
  financial or personal relationship to one of the parties, since the
  contract has no way to know about relationships that don't show up as an
  on-chain address match. A resolver should disclose any such relationship to
  the rest of the committee (and recuse informally by voting neither way) the
  moment they become aware a specific dispute implicates it. This disclosure
  is not enforced by the contract and depends entirely on the resolver's own
  good faith.
- **Notice before going dark.** A resolver who knows they'll be unreachable
  for an extended period (travel, planned unavailability) should say so ahead
  of time, so the committee can decide whether to route around it or start a
  replacement conversation before a dispute actually stalls.

None of these commitments are enforceable by the contract; they're operating
norms the deployment should document and hold resolvers to. Keeping a record
of who agreed to what (and when) is what makes a later removal-for-cause
defensible instead of an admin's unilateral judgment call.

## Offboarding and replacement

### What triggers a replacement

- **Inactivity.** A resolver who repeatedly misses their response-time
  commitment, or who has gone dark without notice. Note the distinction from
  the contract's own liveness fallback: `reclaim_stalled_dispute` (when a
  deployment has `set_stall_timeout` configured) unwinds a single stalled
  dispute by returning both bonds with no winner — it resolves that one case,
  it does not remove the unresponsive resolver from the committee. Persistent
  inactivity is a reason to replace the resolver, not just a reason to reclaim
  the dispute they stalled.
- **Compromised key.** Any signal that a resolver's signing key may have
  leaked or been used without their authorization. Treat this as urgent: see
  the admin-override guidance below.
- **Conflict of interest.** A disclosed (or discovered) relationship to a
  party that makes continued service on the committee inappropriate, whether
  or not any specific vote was actually affected.
- **Resignation.** A resolver who no longer wants or is able to serve.

### Who initiates it

Either the affected resolver themselves (resignation, disclosed conflict,
planned unavailability) or another committee member who observes a problem
(inactivity, a suspected compromise, an undisclosed conflict they've learned
of). The deployment operator (the admin key holder) can also initiate a
replacement, but for anything short of a compromised key or a stalled
committee, initiating through the committee's own self-rotation path is
preferable; see the choice of path below.

### Choosing self-rotation vs. the admin override

Both paths write the same committee; the choice is about who should be making
the decision and how urgently it needs to happen, not a technical constraint
(both are documented in full in
[DEPLOYMENT.md](DEPLOYMENT.md#rotating-the-resolver-committee)):

- **Use self-rotation (`propose_rotation` / `vote_rotation`) for the routine
  cases**: resignation, a disclosed conflict of interest, planned
  unavailability, or inactivity the rest of the committee can still act on
  without the outgoing resolver's cooperation. This keeps the decision inside
  the committee, requires no admin involvement, and is the expected path for
  anything that isn't an emergency. A resolver proposes their own removal (or
  another resolver proposes it, naming the affected seat), and the rest vote.
- **Use the admin override (`update_resolvers`) only for the emergency
  cases**: a compromised key that needs to be cut off immediately, or a
  committee that has lost the ability to reach majority at all (more than
  `(n-1)/2` resolvers simultaneously unreachable, uncooperative, or otherwise
  unable to vote). This is exactly the deadlock scenario
  `docs/src/ROTATION_DESIGN.md` calls out: a compromised or deadlocked
  committee cannot be expected to self-heal by vote, which is why the admin
  path exists at all and stays pause-exempt. Using it for a routine,
  uncontested resignation is a bad default even though it's technically
  capable of doing the same thing — it reintroduces the single-admin-key
  trust path that self-rotation exists to avoid, for a case that didn't need
  it.

A real deployment needs to decide, in advance, who holds the admin key that
can exercise this override and under what conditions they're expected to use
it (per
[V1_MAINNET_PARAMETERS.md](V1_MAINNET_PARAMETERS.md#part-3-resolver-committee-size-and-composition)'s
own framing) — not discover the answer for the first time during an incident.

### Practical notes on executing a replacement

- A rotation only ever affects disputes opened after it executes; the
  outgoing resolver keeps their vote on any dispute already snapshotted
  against the old committee (see `update_resolvers` in
  [CONTRACT.md](CONTRACT.md) and `docs/src/ROTATION_DESIGN.md`'s section on
  the per-dispute snapshot). If a resolver is being removed for a compromised
  key or an active conflict, check whether they're a snapshotted voter on any
  currently `Disputed` assertion; removing them from the live committee does
  not retroactively remove their vote there.
- Confirm the incoming resolver's key and custody model (per the onboarding
  section above) before the rotation executes, not after — once
  `vote_rotation` reaches majority, the swap is immediate.
- Document the reason for every replacement (which of the four triggers
  above applied) alongside the on-chain `RotationExecuted` /
  `ResolversUpdated` event. The event proves *that* the committee changed;
  it doesn't record *why*, and that reasoning is what makes the deployment's
  dispute history auditable later.
