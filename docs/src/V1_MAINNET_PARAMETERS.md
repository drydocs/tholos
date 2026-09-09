# V1 mainnet parameter selection

This note covers the two `initialize` parameters that [BOND_SIZING.md](BOND_SIZING.md)
does not: `challenge_window_secs` and the resolver committee (size and
composition). For `bond_amount` and `finalize_reward_bps`, use
[BOND_SIZING.md](BOND_SIZING.md) directly; nothing here duplicates it.

## A note on what "grounded in real data" means here

This analysis was written without live Stellar testnet access: the sandbox it
was produced in cannot reach Stellar's testnet RPC or Friendbot (confirmed
with a direct `stellar keys generate --fund`, which failed with an HTTP 403
from the network egress layer, not a DNS or config problem). `scripts/testnet-load.sh`
could not be run end-to-end as a result.

What *is* real and measured in this document:

- **Per-operation CPU/memory cost**, measured directly against the actual
  compiled `tholos.wasm` using `soroban-sdk`'s local test environment
  (`Env::cost_estimate().budget()`), not estimated or looked up. See
  [Part 1](#part-1-per-operation-cost-what-testnet-load-would-also-show).
- **Network resource limits** (100,000,000 CPU instructions per transaction),
  from Stellar's published fees-and-metering documentation, a stable
  protocol-level constant rather than something that needs a live run to
  confirm.

What is *not* measured here and needs a live run to confirm precisely:

- Wall-clock transaction latency (ledger close time, RPC round-trip) for each
  operation type under `scripts/testnet-load.sh`'s own timing methodology.
- Real-network behavior under concurrent/adversarial load rather than this
  document's sequential local simulation.

[Part 5](#part-5-filling-in-the-live-testnet-numbers) gives the exact commands
and a table to fill in once someone with testnet access runs
`scripts/testnet-load.sh` at the N/D scales below. Until that happens, treat
the wall-clock figures in this document as informed placeholders derived from
Stellar's published ~5 second ledger close time, not measurements.

## Part 1: per-operation cost (what testnet-load.sh would also show)

Measured by deploying the actual compiled `contracts/tholos` wasm into
`soroban-sdk`'s test `Env`, then calling each entry point and reading
`Env::cost_estimate().budget()` immediately after (this reads real cost from
executing the real wasm bytecode through the host VM, the same measurement
`stellar-cli`'s own simulation step would report against a live network,
minus the network round-trip itself). Methodology and exact numbers below are
reproducible with the snippet in the [Appendix](#appendix-reproducing-these-measurements).

### Baseline: N=20 assertions, D=8 disputed (a comparable scale to
`scripts/testnet-load.sh`'s own default of N=5, D=3, run larger to make any
scaling trend visible)

| Operation | Avg CPU instructions | Max CPU instructions | Avg memory (bytes) | Max memory (bytes) |
| --- | --- | --- | --- | --- |
| `assert_outcome` | 475,133 | 512,027 | 175,158 | 187,794 |
| `dispute` | 560,170 | 577,441 | 206,758 | 211,117 |
| `resolve` (per call; 2 calls needed for a 3-member committee) | 508,340 | 517,665 | 194,544 | 197,296 |
| `finalize` | 527,810 | 536,391 | 195,503 | 198,121 |

Against the network's 100,000,000 CPU instruction ceiling per transaction,
every operation here uses well under 1% of budget. **Compute is not a binding
constraint on any of these four operations at realistic scale**; the
`bond_amount` and `challenge_window_secs` choices should be driven by the
economic and human-response considerations in this document and
[BOND_SIZING.md](BOND_SIZING.md), not by resource exhaustion concerns.

### An unexpected finding: cost grows with the contract's total lifetime usage

Repeating `assert_outcome` against the *same* contract instance shows cost
climbing steadily, not staying flat:

| Assertions so far | Avg CPU instructions for the next `assert_outcome` |
| --- | --- |
| 1–25 | 475,283 |
| 26–50 | 557,672 |
| 51–75 | 633,736 |
| 76–100 | 709,434 |
| 101–125 | 782,106 |
| 126–150 | 856,139 |

The very first call in a fresh instance costs 429,829 instructions. The
150th costs 890,671, almost exactly double, and the growth is close to
linear (~3,100 extra instructions per prior assertion). This repeats a
pattern already flagged for `bond_amount` sizing purposes in
[BOND_SIZING.md](BOND_SIZING.md#monitoring-and-adjustment): usage-dependent
cost isn't unique to this operation, but this document is the first place it's
been directly measured.

Each `assert_outcome` call writes a new, separate persistent storage entry
(`DataKey::Assertion(id)`, one per id, not a shared growing collection), so
this isn't a case of one contract-level data structure growing unbounded. The
most likely explanation is that Soroban's ledger read/write cost accounting
is sensitive to the *total* footprint the transaction's ledger-entry
accesses touch, which can grow as a contract's overall persistent storage
does, but this document does not have access to Soroban's cost-model source
to confirm that specific mechanism with certainty. Regardless of the exact
cause, the growth itself is real and repeatable.

At the still-tiny fractions of the CPU ceiling involved (0.9% of budget even
at call 150), this remains far from a practical constraint for any
deployment lifetime measured in thousands of assertions. It is worth
re-measuring periodically (e.g. with the appendix snippet, at whatever `N`
approximates a deployment's expected multi-year assertion count) rather than
assumed to stay negligible forever, and is flagged here mainly because
nothing in the existing documentation mentions it at all.

## Part 2: challenge_window_secs

`scripts/testnet-load.sh`'s `CHALLENGE_WINDOW_SECS=120` exists purely so an
automated CI-style run finishes in about two minutes rather than hours. It is
not, and was never intended to be, a production candidate. The script picks
it for the same reason a unit test doesn't sleep for a real day. Running the
script, even with full testnet access, would not produce a "real-world
dispute-response time" figure, because the script has no human in the loop
deciding whether to dispute: nothing about its timing reflects how long an
actual interested party takes to notice a bad assertion and act.

### What actually bounds `challenge_window_secs`

The window has to be long enough for the realistic chain of events between
"a bad assertion is posted" and "a disputer's `dispute` transaction lands":

```text
t_notice   Time for a monitoring party (the asserter's counterpart, an
           indexer operator, a manual watcher) to become aware the
           assertion exists at all.
t_verify   Time to check the assertion against the real-world fact it
           claims, which may require off-chain research, human judgement,
           or waiting on an external data source.
t_decide   Time to decide disputing is worthwhile, weighing the bond cost
           against the expected value of correcting the assertion.
t_submit   Time to actually construct, sign, and submit the dispute
           transaction (seconds, from Part 1, not the bottleneck).
```

`t_submit` is the only one of these Part 1's measurements bear on directly,
and it's the smallest by a wide margin: dispute itself costs 560K
instructions, executes in well under a network ledger close cycle
(Stellar's mainnet ledger close time is approximately 5 seconds), and
imposes no meaningful floor on the window on its own. The window is bounded
by `t_notice + t_verify + t_decide`, which are entirely human/operational
factors this document cannot measure and depend on what a given deployment
is actually securing.

### Sizing guidance

There is no formula that converts "monitoring infrastructure exists" into a
specific number of seconds the way [BOND_SIZING.md](BOND_SIZING.md)'s
`bond_amount` formula does, because `t_notice`/`t_verify`/`t_decide` are
governed by who is watching and how automated their process is, not by
anything the contract or network expose. What can be stated concretely:

- **120 seconds (the load-test value) is not viable for any real
  deployment.** No realistic monitoring setup notices a new assertion,
  independently verifies its claim, and submits a dispute transaction within
  two minutes, even with dedicated automation. Treat this purely as a
  load-test artifact, exactly as the issue that requested this analysis
  flagged it.
- **A deployment with dedicated, automated dispute monitoring** (a keeper
  bot watching `Asserted` events against a known-good data feed, empowered
  to dispute automatically) is bounded mainly by `t_notice` (event
  propagation and indexing lag, typically seconds to low minutes) and
  `t_submit` (seconds). A window in the 1–6 hour range gives comfortable
  margin over automated response without meaningfully delaying honest
  finalization.
- **A deployment relying on manual/human monitoring** (an integrator's team,
  or the general public, watching for bad assertions without automation)
  needs to budget for `t_notice` measured in hours (someone has to actually
  look), `t_verify` that may involve real research, and `t_decide` that may
  wait on a human decision-maker's availability (time zones, working hours,
  on-call rotations). A 12–48 hour window is a reasonable starting range;
  below 6 hours risks systematically favoring assertions that happen to post
  outside a monitor's active hours.
- **Every hour added to `challenge_window_secs` is an hour added to the
  uncontested-assertion finality time** (`Pending` -> `finalize`-eligible).
  Size the window against how time-sensitive a correct-but-slow resolution
  is for what's actually being asserted; a deployment where finality speed
  matters more than dispute coverage should lean toward the shorter end
  of its monitoring tier, not stretch the window further "to be safe."

### The canonical testnet deployment's own window is informative, not exemplary

[DEPLOYMENT.md](DEPLOYMENT.md#canonical-testnet-deployment) uses `21600`
(6 hours) for the canonical testnet instance. That sits within the automated-
monitoring range above, but the canonical deployment's resolvers are
`resolver1`/`resolver2`/`resolver3` test identities with, per DEPLOYMENT.md's
own words, "no real-world accountability behind them yet." Its window
choice reflects testnet convenience, not a production recommendation either.
Don't copy it directly into a mainnet `initialize` call without reasoning
through the monitoring tier above for your specific deployment.

## Part 3: resolver committee size and composition

`initialize` enforces odd-length, non-zero, distinct, at most 21 addresses
(`DuplicateResolvers` otherwise); `resolve` requires a strict majority
(more `agrees_with_asserter` votes than the alternative) among that fixed,
per-dispute-snapshotted committee. Composition can only change afterward via
`update_resolvers` (admin override) or `propose_rotation`/`vote_rotation`
(self-rotation, one slot at a time, majority vote); see
[CONTRACT.md](CONTRACT.md) for the exact mechanics of both paths.

### Size trade-off

| Committee size | Majority needed | Trade-off |
| --- | --- | --- |
| 1 | 1 | A single point of failure and a single point of trust; any dispute resolves at that one party's sole discretion. Only appropriate when the "committee" is genuinely one trusted, accountable operator and the deployment accepts that concentration explicitly. |
| 3 | 2 | Smallest size with any redundancy: one resolver being slow, unavailable, or compromised still leaves 2 who can act. This is what the canonical testnet deployment and `scripts/testnet-load.sh` both use. |
| 5 | 3 | Tolerates 2 simultaneous unavailable/compromised resolvers rather than 1. Meaningfully more resilient than 3 for a modest coordination-cost increase (each dispute still needs only a bare majority to act, not unanimity). |
| 7+ (odd, up to 21) | `(n/2)+1` | Diminishing returns per added seat: coordination cost (getting a majority of a larger group to actually vote in a reasonable time) grows roughly linearly while the marginal resilience gain per seat shrinks. Justified mainly when the resolver role itself needs broad, visible representation (e.g. a public curated list) rather than for redundancy alone. |

**Recommendation**: default to 3 for anything below "meaningful value at
stake," move to 5 once bond sizes or assertion values reach a point where a
single compromised or colluding pair of resolvers becomes an attractive
target, and treat anything above 7 as a governance/legitimacy choice rather
than a security one. `resolve`'s gas/compute cost (Part 1) is not a
constraint on committee size in either direction; the practical ceiling is
how many people can realistically be reached and vote within
`challenge_window_secs` once a dispute opens.

### Composition

- **Reachability within the window matters more than credentials.** A
  resolver who cannot be reached to vote within `challenge_window_secs`
  might as well not be on the committee for that dispute; a slow committee
  stalls every disputed assertion until enough of it acts (per
  [DEPLOYMENT.md](DEPLOYMENT.md)'s own guidance). Pick people with a
  demonstrated pattern of being available on the timescale the window
  implies, not simply the most qualified reviewers in the abstract.
- **Independence reduces collusion and correlated-failure risk.** Resolvers
  who share an employer, a wallet-custody provider, or a single point of
  off-chain coordination (the same group chat, the same on-call rotation)
  aren't functioning as independent checks even if they're distinct
  addresses; a single incident (that provider's outage, that group's
  compromise) can take out a majority at once.
- **Self-rotation lets the committee evolve without an admin override,
  but only by strict majority and one seat at a time.** It cannot recover
  a committee that's lost the ability to reach majority at all (e.g. more
  than `(n-1)/2` resolvers simultaneously unresponsive). `update_resolvers`
  is the explicit break-glass path for that scenario; a real deployment
  needs a plan (who holds the admin key, under what conditions they'd use
  it) for exercising it, not just the contract-level capability existing.

## Part 4: explicit assumptions

The guidance above depends on what a given deployment is actually securing.
This document does not know that for any specific deployer, so it states the
assumption ranges the sizing tiers above are built around:

| Input | Assumed range | Where it matters |
| --- | --- | --- |
| Expected assertion value | Low-value (informational/reputational) to moderate (meaningful but not life-changing per assertion) | Drives which `challenge_window_secs` monitoring tier and which committee-size tier apply. This document does not model a high-value tier (see below). |
| Expected dispute frequency | Occasional, not every assertion | The whale/spam-style attack models in [BOND_SIZING.md](BOND_SIZING.md) already cover adversarial dispute volume; this document assumes disputes are the exception, not the norm, when reasoning about resolver reachability load. |
| Monitoring sophistication | Ranges from no dedicated monitoring to a single automated keeper; does not assume a professional, multi-operator monitoring service | If a deployment has stronger guarantees here (e.g. a paid monitoring SLA), the window can be sized shorter than Part 2's manual-monitoring tier suggests. |

**Not covered by this document**: a "higher-value mainnet candidate" tier
analogous to [BOND_SIZING.md](BOND_SIZING.md)'s own top profile, where
assertion values are large enough to justify professional, SLA-backed
monitoring and a larger, more formally accountable resolver committee (real
legal identity, insurance, or similar). That tier needs deployment-specific
threat modeling this document can't assume generically, and should be
revisited once Tholos has a concrete deployment targeting that value range.

## Part 5: filling in the live testnet numbers

Run these once against real testnet access to replace this document's
modeled wall-clock figures with measured ones, and to get the phase timing
`scripts/testnet-load.sh` itself reports (setup, assertion/dispute/resolve/
finalize phase durations, and average invocation latency):

```bash
# Small scale, close to the script's own default
bash scripts/testnet-load.sh 5 3

# Matches this document's Part 1 baseline
bash scripts/testnet-load.sh 20 8

# Larger scale, to see whether Part 1's cost-growth finding is visible in
# real wall-clock timing too (e.g. later resolves/finalizes taking
# perceptibly longer than earlier ones due to increasing ledger footprint)
bash scripts/testnet-load.sh 50 20
```

Record, for each run:

| N | D | Setup (s) | Avg assert (s) | Avg dispute (s) | Avg resolve (s) | Avg finalize (s) | Total wall-clock (s) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 5 | 3 | | | | | | |
| 20 | 8 | | | | | | |
| 50 | 20 | | | | | | |

If the per-operation wall-clock times climb across runs the way Part 1's CPU
instruction counts do, that's independent confirmation of the cost-growth
finding via a completely different measurement method (real network timing
instead of local instruction counting), and would be worth its own follow-up
investigation into the root cause.

## Appendix: reproducing these measurements

The exact test used to produce Part 1's numbers (not committed to the test
suite, since it's a one-off benchmark rather than a correctness test --
paste it into `contracts/tholos/src/test.rs` inside `mod test { ... }` to
rerun or extend it):

```rust
#[test]
fn measure_real_operation_costs_for_v1_bond_sizing_doc() {
    extern crate std;
    use std::println;

    let f = Fixture::new();
    const N: u32 = 20;
    const D: u32 = 8;

    let asserter = f.generate();
    f.mint(&asserter, i128::from(N) * DEFAULT_BOND);
    let disputer = f.generate();
    f.mint(&disputer, i128::from(D) * DEFAULT_BOND);

    let mut assert_cpu = std::vec::Vec::new();
    let mut ids = std::vec::Vec::new();
    for i in 0..N {
        f.env.cost_estimate().budget().reset_default();
        let id = f.client.assert_outcome(&asserter, &(i % 2 == 0));
        assert_cpu.push(f.env.cost_estimate().budget().cpu_instruction_cost());
        ids.push(id);
    }

    // ... dispute/resolve/finalize loops follow the same
    // reset_default() -> call -> cpu_instruction_cost() pattern.

    let avg = assert_cpu.iter().sum::<u64>() / assert_cpu.len() as u64;
    println!("assert_outcome avg CPU: {avg}");
}
```

Run with:

```bash
cargo test -p tholos measure_real_operation_costs_for_v1_bond_sizing_doc -- --nocapture
```
