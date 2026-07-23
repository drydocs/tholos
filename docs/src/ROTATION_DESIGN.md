# Resolver self-rotation: design

Design proposal for issue: *"Design a resolver self-rotation scheme."* The goal is to
remove the committee-membership change from the single-admin-key trust path. Today
only `update_resolvers` (admin signature) can change who sits on the committee, which
CONTRACT.md's Known gaps flags as a larger centralization point than the committee
itself. This design lets the committee replace one of its own by vote.

Implementation lives in `contracts/tholos/src/lib.rs`; the public interface is
documented in [CONTRACT.md](CONTRACT.md) and the rationale in
[ARCHITECTURE.md](ARCHITECTURE.md#resolver-self-rotation). This file is the design
record and the source of the three decisions the issue asks for.

## The three questions

### 1. What majority is required?

A **strict majority of the live committee, using the exact same formula as a dispute
tie-break**: `majority = committee.len() / 2 + 1`.

Reasoning, and why not something stricter:

- The contract already has one majority rule, derived from the odd-length invariant:
  a strict majority is always reachable and never ties. Introducing a *different*
  threshold for rotation would fork that invariant and require new reasoning about
  when ties are or aren't possible. Keeping a single rule is simpler and easier to
  audit.
- A colluding majority already controls every disputed assertion (2 of 3 decide any
  dispute in their favor). Rotation-by-majority does not *add* attack surface beyond
  what a colluding majority already has; it just lets that same majority change its
  own membership through the contract instead of needing the admin key.
- A higher bar (e.g. 2/3 supermajority, or quorum + majority) is the more conservative
  choice for a "constitutional" change and would be reasonable in a system where
  membership changes are rarer and higher-stakes than verdicts. We considered it and
  rejected it for v1: it would mean disputes resolve at a 2/3-equivalent only when the
  committee is size 3 or 5 (where strict majority already equals 2/3 or 3/5), but
  diverge for larger committees, re-introducing two thresholds to reason about. If a
  deployment wants a stricter bar later, it does it by choosing a smaller committee,
  not by a separate code path.

Votes are recorded as yes/no. Only yes-votes move the count; a no-vote records dissent
and prevents re-voting but never blocks (a proposal fails only by becoming
*mathematically impossible*, see liveness below). Execution fires the moment yes-votes
reach `majority`.

### 2. How does it interact with the per-dispute snapshot?

**It doesn't need to interact at all, by construction.** This is the key insight that
keeps the change small.

`Assertion.resolvers` is a *per-dispute* snapshot of the committee taken at `dispute`
time, used only to decide who may vote on *that dispute* and what majority means for
*that dispute*. Rotation is about *committee membership*, a different concern with its
own storage and its own vote. The two are independent objects.

Self-rotation writes the **same `Resolvers` instance-storage slot** that admin
`update_resolvers` writes. Because the dispute snapshot is taken at `dispute` time
against the live committee, any rotation that *completes after a dispute is already
open* inherits the exact behavior `update_resolvers` already has: it has no effect on
that dispute. The new (post-rotation) committee is used only for disputes opened
*after* the rotation executes. No change to `Assertion`, `dispute`, or `resolve` is
required; the existing snapshot invariant carries the rotation for free.

So the interaction is: **none at the dispute layer.** Rotation is just a
committee-governed alternative path to the same `Resolvers` value. The one place the
two paths do touch is the race where the admin overrides (`update_resolvers`) while a
self-rotation vote is in flight; that is handled explicitly (see coexistence below) by
having `update_resolvers` cancel any open self-rotation proposal.

### 3. Does it replace `update_resolvers` or coexist with it?

**Coexists.** Self-rotation is the day-to-day path; admin `update_resolvers` stays as
the emergency override. Three reasons:

- **Deadlock recovery.** If the committee is itself the problem (members go dark,
  or a majority colludes), the committee cannot self-heal — that is precisely the
  scenario where an external override is needed. Removing `update_resolvers` would
  trade one centralization point for a *permanent* deadlock risk.
- **The two paths are complementary, not redundant.** `update_resolvers` is already
  pause-exempt precisely so a compromised committee can be replaced without unpausing.
  Self-rotation, being a committee action, is useless exactly when the committee is
  the thing that's compromised. The admin key is the break-glass for that case.
- **The issue frames it as an alternative path,** not a replacement.

Both paths emit `ResolversUpdated` so the "committee just changed" signal stays unified
for indexers; self-rotation adds `RotationProposed` / `RotationExecuted` /
`RotationCancelled` for the governance audit trail.

## Mechanics

Three new functions, gated to one open rotation at a time:

- `propose_rotation(resolver, old_resolver, new_resolver)` — caller must be a current
  resolver (auth + membership). `old_resolver` must be on the committee; `new_resolver`
  must not be (and not equal `old`). One proposal open at a time. Emits
  `RotationProposed`.
- `vote_rotation(resolver, approve)` — caller must be a current resolver, not already
  voted. Yes-vote that reaches `majority` executes the swap (old → new in the live
  committee) and clears the proposal, emitting `RotationExecuted` and
  `ResolversUpdated`. A no-vote that makes the proposal mathematically impossible to
  pass auto-cancels it (liveness guard), emitting `RotationCancelled`. Otherwise the
  vote is recorded and the proposal stays open. Returns `Some(true)` (executed),
  `Some(false)` (auto-cancelled as dead), or `None` (still open).
- `cancel_rotation(resolver)` — the proposer may cancel any time; any current resolver
  may cancel once the proposal can no longer reach a majority. Emits `RotationCancelled`.

### Liveness / deadlock guard

At most one open proposal. A proposal is resolved in exactly one way that changes
state:

1. **Execute** — yes-votes hit `majority`.
2. **Cancel by proposer** — at any time.
3. **Deadlock auto-cancel** — if `yes + remaining_unvoted < majority`, it can never
   pass; `vote_rotation` cancels it automatically, and `cancel_rotation` lets any
   resolver cancel it too. This prevents a lost proposer key from permanently blocking
   all rotation.

### Pause

Proposal and voting are **pause-exempt**, matching `update_resolvers`. Rotation is
internal governance (no assertion, dispute, or dispute-vote is created), so the
"pause stops new exposure" rationale doesn't apply. A paused, incident-hit deployment
can still self-heal by committee vote; the admin override remains the fallback for a
compromised committee.

### Reentrancy

None. Rotation never moves tokens, so there is no external call to reenter through.
State is still written before events are published, matching the contract's
checks-effects convention.

### Admin override vs. in-flight rotation

`update_resolvers` clears any open self-rotation proposal (emitting `RotationCancelled`
when one was present). The only way the live committee changes is `update_resolvers`
and rotation-execution, and both clear the proposal — so whenever a proposal exists,
the live committee still matches the assumptions it was validated against. No stale
proposal can ever execute against a committee it wasn't built for.
