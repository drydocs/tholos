# v1/v2 coexistence and migration runbook

V1 (`contracts/tholos`) and v2 (`contracts/tholos-v2`) are two independent
contracts, not one contract with an upgrade path between them. V1 has no
WASM upgrade entry point and no state importer, and even if it did, there's
no way to move a bond already locked in a v1 dispute into a v2 record
without changing who's liable for it. The two deployments run side by side
for as long as v1 has any open activity; this is the runbook for that
period, from inventorying an existing v1 deployment through retiring it.

See [INTEGRATION.md](INTEGRATION.md#tholos-v2) for the function-level
differences between the two contracts. This doc is about the operational
sequence of moving traffic from one to the other, not the interface itself.

[V2_RESOLUTION.md's "Migration from existing v1 deployments"](V2_RESOLUTION.md#migration-from-existing-v1-deployments)
already covers this same period from a design-time angle (why blue/green,
what can and can't be guaranteed, the rollback boundary). This doc restates
those steps as a practical runbook rather than duplicating them
independently; treat the two as one account split across two docs, not two
separate opinions, and update both together if either changes.

## Why there's no automated migration

- **No upgrade entry point.** V1's WASM is immutable once deployed; nothing
  in it can be replaced with v2's logic in place.
- **No state importer.** Even if v2 wanted to adopt v1's history, v1
  exposes no way to export its full assertion/dispute state in one
  authoritative read (see the inventory section below for what that
  actually takes to reconstruct).
- **Bonds can't move contracts.** A bond locked in an open v1 dispute is a
  liability of the v1 deployment's own token balance. There's no operation
  that transfers it into v2's balance and reissues it as a v2 position;
  doing so would require v2 to honor a liability it never received funds
  for. Every v1 bond stays a v1 liability until v1's own `finalize`/
  `resolve` pays it out, full stop.

Given that, migration is a *traffic* decision, not a *data* migration: stop
sending new assertions to v1, send them to v2 instead, and let v1's
already-open assertions run to completion on v1's own terms.

## 1. Inventory the v1 deployment

V1 has no public config getter, no version marker, and no way to enumerate
its own `NextId` range or list of open assertions; none of this can be read
back from the contract in one call. Reconstruct it from deployment
transactions and events instead:

- **Network, contract id, exact WASM hash.** From the `deploy` transaction
  itself (or `stellar contract info` against the live contract id). The
  WASM hash matters for the same reason CONTRIBUTING.md's review policy
  never accepts a bare contract address as proof of what code it runs: a
  contract id alone doesn't tell you what's actually deployed there.
- **`token`, `bond_amount`, `challenge_window_secs`, `resolvers`,
  `finalize_reward_bps`.** From the `initialize` invocation's arguments, or
  from `get_assertion_state` on any known assertion id if the invocation
  itself isn't handy (`Assertion` doesn't carry every policy field, but the
  ones it does are enough to cross-check).
- **Current `admin` and resolver committee.** From the latest
  `ResolversUpdated` event if the committee has ever rotated, otherwise
  from `initialize`'s original arguments.
- **Every open `Pending`/`Disputed` assertion.** There's no enumeration
  call for this either. Walk the contract's event history (`Asserted`,
  `Disputed`, `Finalized`, `Resolved`) from deployment to now, and take the
  set of `id`s that have an `Asserted` but no matching `Finalized`/
  `Resolved`. This set is exactly what still needs to drain before v1 can
  be retired; see step 4.

## 2. Deploy v2 fresh

Deploy `contracts/tholos-v2` as its own new contract, following
[DEPLOYMENT.md](DEPLOYMENT.md)'s parameter guidance (it's written for v1,
but the considerations for `token`/`bond_amount`/`challenge_window_secs`
apply the same way to v2's `initialize`; see
[CONTRACT.md](CONTRACT.md) and [V2_RESOLUTION.md](V2_RESOLUTION.md) for the
parameters v2 adds beyond v1's, like `registration_duration_secs` and
`reveal_duration_secs`).

Do not:

- **Copy v1's assertion records into v2.** V2's `AssertionV2`/`Resolution`/
  `Position` records are a different shape from v1's `Assertion`, and even
  a faithful reconstruction would misrepresent history: those assertions
  were decided (or are still being decided) under v1's fixed-committee
  rule, not v2's stake-weighted one. Let them stay v1 history.
- **Move v1's pooled token balance into v2.** V1's balance backs its own
  open liabilities (bonds not yet returned via `finalize`/`resolve`). Move
  it and v1 can no longer pay out assertions it's already committed to.

v2 starts with genuinely zero history, the same tradeoff any fresh
deployment accepts per
[INTEGRATION.md](INTEGRATION.md#should-you-deploy-your-own-instance-or-share-one).
That's expected here: this is a new contract, not a continuation of v1's
track record.

## 3. Cut new traffic over

Pick and record a cutover point (a ledger sequence number or timestamp
works well, since it's independently verifiable later). From that point on,
route new assertions to v2's contract id instead of v1's. This is purely a
decision your integration makes about which contract id it calls; neither
contract has a flag that enforces it for you.

Record the cutover point somewhere durable (a deploy note, a config entry,
whatever your integration already uses for this), since step 4 needs to
distinguish "opened before cutover, still draining on v1" from "opened
after cutover, already on v2."

## 4. Let v1 drain, without pausing it

**Do not pause v1 during drain.** `set_paused` blocks `assert_outcome`,
`dispute`, `resolve`, and `finalize` all together (see
[DEPLOYMENT.md](DEPLOYMENT.md#pausing-during-an-incident) and
[INTEGRATION.md](INTEGRATION.md#known-caveats-for-integrators)); there's no
way to pause only new direct-caller assertions while leaving every already-
open `Pending`/`Disputed` assertion free to finalize or resolve normally.
Pausing during drain doesn't protect anything, since drain is a routine
wind-down, not an incident, it just stalls every assertion still in flight
(a `Pending` one past its challenge window can't finalize, a `Disputed` one
can't resolve) for as long as the pause lasts, working directly against the
point of this step. This is the same reason `DEPLOYMENT.md`'s admin runbook
already warns not to use pause as a migration or retirement switch.

Instead, simply stop sending new `assert_outcome` calls to v1 (step 3
already does this) and let the inventory from step 1 run its natural
course: every open assertion either finalizes uncontested after its
challenge window, or gets disputed and resolved by the committee, same as
it always would have. There is no way to force this faster without
touching assertions that haven't had their full, promised window to be
contested; don't try to accelerate it.

Track the inventory set from step 1 against `Finalized`/`Resolved` events
as they arrive. V1 is fully drained once every id in that set has one, but
this isn't guaranteed to happen on any timeline: v1 has no timeout or
cancellation for a dispute whose snapshotted committee can no longer reach
a majority (a resolver gone unreachable, a duplicate-filled snapshot from
older v1 bytecode that never validated distinctness), so a stuck dispute
can leave drain, and full v1 retirement, permanently incomplete. See
[V2_RESOLUTION.md's "Migration from existing v1 deployments"](V2_RESOLUTION.md#migration-from-existing-v1-deployments)
for the fuller design-time treatment of this and the rollback boundary;
this runbook is the practical step-by-step version of the same period, and
the two should be read together rather than as competing accounts.

## 5. Retire v1 operationally

Once drained, there's nothing left for v1 to do: leave it deployed and
unpaused rather than pausing it as a final step. A paused-forever contract
with a genuine zero-liability balance is operationally identical to an
unpaused one nobody calls, so there's no safety benefit to pausing at this
point, only a documentation cost (an operator seeing it paused might
reasonably wonder why, and go looking for an incident that isn't there).
Update whatever integration-facing docs point at v1's contract id to point
at v2's instead, and note the retirement date alongside the inventory this
runbook started with, for anyone auditing the transition later.

## Updating your own integration

If you're a v1 integrator working through this runbook for your own
deployment: `demos/freelance-escrow` in this repo is in exactly this
position (it currently calls v1 directly, see its own `src/lib/tholos.ts`),
and migrating it is tracked as its own follow-up rather than bundled with
v2's implementation issues. Use it as a worked example once that follow-up
lands, not as a template today.
