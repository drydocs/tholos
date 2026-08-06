# Freelance milestone escrow

A real freelance/gig milestone-payment app: a client and freelancer agree on
milestones, the freelancer marks a milestone done, funds release automatically
once the challenge window passes uncontested, or a resolver panel decides if
the client disputes it. Tholos is the settlement layer underneath, not
something this app puts on display: there are no "call assert_outcome" buttons
anywhere in the UI. Milestone actions read as ordinary product actions
(mark done, dispute, release), and Tholos's bonded assertion/dispute contract
is what actually backs the "funds release uncontested or a panel decides"
guarantee. See [docs/src/INTEGRATION.md](../../docs/src/INTEGRATION.md) for
the pattern this app follows (end user as asserter).

This app talks to a live Tholos instance, so every action (marking a milestone
done, disputing, voting, finalizing) is a real signed transaction, not a
simulation. You'll need testnet XLM in your Freighter wallet to post the bond;
get some from [Friendbot](https://friendbot.stellar.org/) if you're testing
with a fresh address.

## What maps to what

| App action | Tholos call |
| --- | --- |
| Freelancer marks a milestone done | `assert_outcome(freelancer, true)` |
| Nobody disputes within the challenge window | `finalize` releases the bond, milestone pays out |
| Client disputes a submitted milestone | `dispute(client, id)` |
| Resolver panel decides a dispute | `resolve(resolver, id, agrees_with_freelancer)` |

Job and milestone metadata (title, description, client, freelancer, the
milestone's face-value amount) lives entirely in this app, off-chain. Tholos
only ever sees a bonded assertion per milestone; the mapping from milestone to
assertion id is tracked client-side (see `assertionId` on `Milestone` in
`src/data/jobs.ts`), matching the "store that mapping on your side" guidance
in `INTEGRATION.md`.

The bond posted on-chain is a fixed amount set at deploy time (see
`docs/src/DEPLOYMENT.md`), not the milestone's face-value amount shown in the
UI: Tholos v1 has one bond size per instance, not a per-call bond, so the bond
is collateral that deters a bad-faith claim, separate from whatever the
client and freelancer actually agreed to pay for the milestone.

## Running it

Contract addresses are never committed to source (see CONTRIBUTING.md), so
point this at a deployed Tholos instance yourself. The
[canonical testnet deployment](../../docs/src/DEPLOYMENT.md#canonical-testnet-deployment)
works for this app out of the box:

```sh
cp .env.example .env.local
# fill in VITE_THOLOS_CONTRACT_ID with the canonical testnet contract id from
# docs/src/DEPLOYMENT.md (or your own deployment, if you have a reason to need one)

pnpm install
pnpm dev
```

Requires the [Freighter](https://www.freighter.app/) browser extension to
connect a wallet.

There's no role system of its own: Tholos only knows addresses, not "client"
or "freelancer," so this demo has a role switcher in the header letting the
connected wallet act as freelancer, client, or resolver to exercise all sides
of a dispute from one browser.

## Structure

```text
src/
  lib/
    config.ts    RPC URL, network passphrase, and contract id (env-overridable)
    tholos.ts    Tholos contract client: assert_outcome/dispute/resolve/finalize/get_assertion_state
    wallet.ts    Freighter connect/detect
  state/
    JobsContext.tsx  Job and milestone state, wired to the Tholos client
    RoleContext.tsx  Which side of the transaction the connected wallet is currently playing
  components/
    JobCard.tsx, MilestoneRow.tsx, PostJobForm.tsx, WalletButton.tsx, RoleSwitcher.tsx
  data/
    jobs.ts      Seed jobs and the Job/Milestone types
```
