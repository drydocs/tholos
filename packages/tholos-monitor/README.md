# tholos-monitor

Off-chain event monitoring/alerting service for `contracts/tholos` (v1) and
`contracts/tholos-v2`. Closes [drydocs/tholos#189](https://github.com/drydocs/tholos/issues/189):
today an admin
only learns about a paused contract, a stalled dispute, or a cancelled
rotation by manually querying the chain. This service polls both
deployments' events via Soroban RPC and posts a webhook alert for the ones
that mean something needs a human, immediately rather than on next manual
check.

Doesn't touch either contract — this is purely an off-chain watcher, built
the same way `packages/tholos-sdk` is: a standalone TypeScript package, not
part of a JS workspace (there isn't one at the repo root; see that
package's own README for why).

## Approach

A polling script against Soroban RPC's `getEvents`, not a long-running
event-stream listener or a hosted indexer. For this service's traffic
(admin/governance actions and dispute-lifecycle liveness failures — not
high-frequency trading data), polling on a schedule is simple to run and
reason about: no process to keep alive, restart on crash, or babysit for
memory growth. The tradeoff is latency bounded by the schedule interval
rather than push-immediate; see "Process lifecycle" below for the interval
this runs at and why that's still well within "immediately."

### Process lifecycle

This runs as a scheduled script, invoked fresh on a GitHub Actions cron
trigger (`.github/workflows/tholos-monitor.yml`), not a long-running daemon
— that was the maintainer's explicit call on
[issue #189](https://github.com/drydocs/tholos/issues/189) between the two
options proposed there (this one, or an always-on process managing its own
poll loop). Each run does exactly one poll pass over both deployments and
exits; there's no internal loop, no timers, and nothing listens for
`SIGINT`/`SIGTERM` because nothing stays up between ticks. Everything that
needs to survive from one run to the next — the RPC pagination cursor and
the consecutive-failure counters — is persisted to `STATE_FILE_PATH` (see
"State and restarts") and restored via `actions/cache` at the start of the
next run (see the workflow file). The schedule is every 5 minutes
(`*/5 * * * *`, GitHub Actions' minimum cron granularity) — bounded latency
well inside "immediately, not on next manual check," without running an
always-on process for traffic this infrequent.

The workflow can also be triggered manually (`workflow_dispatch`) from the
Actions tab — useful right after setting it up, or to re-check without
waiting for the next tick. Two things to know about GitHub's scheduled
workflows generally: they only run from the repository's **default
branch** (this won't fire from a feature or PR branch, only after merge),
and GitHub disables a schedule automatically after 60 days with no commits
to the default branch (any commit resets that clock) — worth knowing if
alerts seem to silently stop.

## Which events alert, and why

Every event either contract currently emits is fetched and logged; only
some are alerted on by default (see `ALERT_MIN_SEVERITY` below). Severity is
assigned in `src/events.ts`:

| Severity | v1 (`contracts/tholos`) | v2 (`contracts/tholos-v2`) |
| --- | --- | --- |
| `critical` | `PauseUpdated` (paused=true only), `AdminUpdated`, `AdminRotationProposed`, `RotationCancelled`, `StalledDisputeReclaimed` | `PauseUpdated` (paused=true only), `AdminUpdated`, `RoundCancelled`, `RoundVoided` |
| `warning` | `BondAmountUpdated`, `ResolversUpdated`, `RotationProposed`, `RotationExecuted`, `StallTimeoutUpdated` | — |
| `info` | `Asserted`, `Disputed`, `Finalized`, `Resolved`, `PauseUpdated` (paused=false) | `Asserted`, `Disputed`, `Finalized`, `Resolved`, `PositionFunded`, `RevealOpened`, `Revealed`, `Settled`, `DustCredited`, `Withdrawn` |

`PauseUpdated` is classified from its payload, not statically: pausing is
the incident, unpausing is the incident ending.

Two events here weren't in issue #189's original "at minimum" list —
`StalledDisputeReclaimed` and `RoundVoided`. Both landed upstream (see
`contracts/*/src/lib.rs`) after the issue was filed: the former is v1's
stalled-dispute reclaim (the resolver committee didn't act in time, both
bonds returned with no winner), the latter is v2's reveal-quorum-not-met
round voiding. Both are exactly the "error/liveness condition worth knowing
about immediately" the issue asks for, so they're included as `critical`
here even though they postdate the issue text. `src/events.ts` documents
this; worth a second look from whoever reviews this against the issue as
filed.

Any event name not in this registry (a future contract upgrade adding one)
is still fetched, logged, and defaults to `warning` rather than being
silently dropped — see the "not filtering by topic" note below.

## Configuration

Everything is environment-configured, the same convention
`demos/freelance-escrow/src/lib/config.ts` uses (never commit a contract
address to source; see CONTRIBUTING.md) — just without the `VITE_` prefix,
since this is a plain Node service rather than a Vite app.

| Variable | Required | Default | Notes |
| --- | --- | --- | --- |
| `THOLOS_V1_CONTRACT_ID` | At least one of this or `THOLOS_V2_CONTRACT_ID` | — | v1 deployment to monitor, e.g. the [canonical testnet deployment](../../docs/src/DEPLOYMENT.md#canonical-testnet-deployment). |
| `THOLOS_V2_CONTRACT_ID` | At least one of this or `THOLOS_V1_CONTRACT_ID` | — | v2 deployment to monitor. v2 has no canonical testnet deployment yet (see `docs/src/DEPLOYMENT_V2.md`) — use your own. |
| `ALERT_WEBHOOK_URL` | Yes | — | Any endpoint that accepts a JSON `POST` — a Slack/Discord incoming webhook, or your own. See "Alert payload" below for the body shape. |
| `SOROBAN_RPC_URL` | No | `https://soroban-testnet.stellar.org` | Same default as the demo app's config. |
| `NETWORK_PASSPHRASE` | No | `Test SDF Network ; September 2015` | Currently informational (not sent to the RPC — `getEvents` is filtered by contract id, not network); kept alongside the RPC URL so a future mainnet config has an obvious place to set both together. |
| `ALERT_MIN_SEVERITY` | No | `warning` | `info` \| `warning` \| `critical`. Events below this are still logged, just not sent to the webhook. |
| `STATE_FILE_PATH` | No | `./monitor-state.json` | Where the per-deployment RPC pagination cursor is persisted (see "State and restarts"). |
| `CONSECUTIVE_FAILURES_BEFORE_ALERT` | No | `3` | Consecutive poll failures (for one deployment) before this service alerts on its own health, not just logs. |
| `REQUEST_TIMEOUT_MS` | No | `10000` | Timeout for the webhook POST. |
| `INITIAL_LEDGER_LOOKBACK` | No | `17280` (~1 day at ~5s/ledger) | Only used the very first time this runs against a fresh state file (no saved cursor yet): how far back from the chain tip to start. |
| `ALERT_MAX_CONCURRENCY` | No | `5` | Caps how many alert webhook `POST`s run at once. A poll tick with many threshold-meeting events (e.g. a first run against the full lookback, or a run after downtime) dispatches them concurrently rather than one at a time, but bounded — an unbounded burst can overwhelm or get rate-limited by whatever's receiving the webhook. |

Invalid or missing required configuration fails fast at startup with a
message naming the problem (`src/config.ts`), rather than surfacing later as
a confusing runtime error.

## Running it

Locally (one poll pass, then exit — see "Process lifecycle" above for how
this runs in production):

```sh
pnpm install
pnpm build
cp .env.example .env   # fill in at least one contract id and ALERT_WEBHOOK_URL
node --env-file=.env dist/main.js
# or, for local iteration without a build step:
pnpm dev
```

In GitHub Actions, `.github/workflows/tholos-monitor.yml` runs the same
built `dist/main.js` on a schedule. For it to actually alert on anything, a
repo admin needs to configure, under the repository's Settings → Secrets
and variables → Actions:

| Name | Kind | Notes |
| --- | --- | --- |
| `THOLOS_V1_CONTRACT_ID` | Variable | At least one of this or `THOLOS_V2_CONTRACT_ID`. |
| `THOLOS_V2_CONTRACT_ID` | Variable | At least one of this or `THOLOS_V1_CONTRACT_ID`. |
| `SOROBAN_RPC_URL` | Variable (optional) | Only if not using the default testnet RPC. |
| `ALERT_WEBHOOK_URL` | **Secret** | Kept as a secret, not a variable, since it's effectively a bearer credential for posting to whatever it points at (a Slack/Discord incoming webhook, typically). |

A `pnpm-lock.yaml` for this package (generated by running `pnpm install`
once locally and committing the result) is required for
`pnpm install --frozen-lockfile` and the workflow's dependency cache to
work — see CONTRIBUTING.md.

## State and restarts

Each deployment's Soroban RPC pagination cursor, plus its
consecutive-failure counters (see below), is persisted to `STATE_FILE_PATH`
after every run (atomic write: temp file + rename, so a crash mid-write
can't corrupt it). Because this is a scheduled script rather than a
long-running process (see "Process lifecycle"), this file — restored and
saved via `actions/cache` around each GitHub Actions run — is the *only*
thing that carries anything across runs; there's no in-memory state to
fall back on between them. The next run resumes from the saved cursor
rather than replaying already-seen events or re-deriving a start ledger.

Soroban RPC only retains events for a bounded ledger window (operator- and
network-dependent, not a contract constant). If a saved cursor falls outside
that window — this service was down long enough, or a state file is old —
`getEvents` fails rather than silently skipping ahead. This service detects
that (`RetentionGapError` in `src/rpc.ts`, matched by the RPC's error
wording — see the comment there on why that's a heuristic, not a dedicated
error code), sends a `critical` `monitor_liveness` alert saying so, drops
the stale cursor, and re-anchors from the current chain tip minus
`INITIAL_LEDGER_LOOKBACK`. Events in the gap are unrecoverable from this
RPC; the alert exists so that's a known, immediate fact, not a silent
skip discovered later — which is the whole reason this service exists.

Three consecutive failed runs of any other kind
(`CONSECUTIVE_FAILURES_BEFORE_ALERT`, tracked in the persisted state file
since nothing survives between runs in memory) also alert (`rpc_failure`),
and a `recovered` alert follows once a run succeeds again — both keep the
monitor's own health from being a silent assumption.

## Alert payload

A `POST` with one JSON body per alert, one of two shapes
(`AlertPayload` in `src/types.ts`):

```jsonc
// A classified contract event that met ALERT_MIN_SEVERITY
{
  "type": "contract_event",
  "timestamp": "2026-01-01T00:00:00.000Z",
  "event": {
    "deployment": "tholos",           // or "tholos-v2"
    "contractId": "CA...",
    "eventName": "PauseUpdated",
    "severity": "critical",
    "description": "Contract paused.",
    "ledger": 123456,
    "ledgerClosedAt": "2026-01-01T00:00:00Z",
    "txHash": "…",
    "topics": [],
    "data": { "paused": true }
  }
}

// This service's own health
{
  "type": "monitor_liveness",
  "timestamp": "2026-01-01T00:00:00.000Z",
  "alert": {
    "deployment": "tholos",
    "kind": "rpc_failure",            // or "retention_gap" | "recovered"
    "severity": "critical",
    "message": "tholos: 3 consecutive poll failures against https://soroban-testnet.stellar.org.",
    "consecutiveFailures": 3,
    "lastError": "…"
  }
}
```

A failed webhook delivery is retried with capped exponential backoff, then
logged and dropped — a webhook outage never crashes the poller (`src/alerts.ts`).

## Not filtering by topic

`getEvents` supports server-side topic filters, but each segment has to be
pre-encoded to base64 XDR — in practice that means hardcoding one filter
per event name this package already knows about. Instead, this service
fetches every `contract`-type event for each configured contract id and
classifies client-side (`src/events.ts`). The cost is a slightly larger
response per poll; the benefit is that a contract upgrade adding a new
event shows up immediately (logged, `warning`-severity by default) instead
of being invisibly filtered out until someone remembers to update a topic
filter here.

## Structure

```text
src/
  config.ts    Env var parsing/validation (loadConfig)
  types.ts     Shared types (DecodedEvent, ClassifiedEvent, AlertPayload, ...)
  events.ts    Event name -> severity/description registry, and PauseUpdated's
               payload-dependent classification
  rpc.ts       Soroban RPC wrapper: getEvents pagination, XDR decoding via
               scValToNative, retention-gap detection
  state.ts     Cursor + failure-counter persistence (atomic read/write of
               STATE_FILE_PATH)
  alerts.ts    Webhook POST with retry/backoff; alert payload builders
  poller.ts    runOnce: one poll pass over every configured deployment,
               per-deployment cursor + failure tracking, severity filtering
  main.ts      Entrypoint: loads config, calls runOnce once, sets the
               process exit code from its result
  *.test.ts    Unit tests (node's built-in test runner, no network — see below)
```

## GitHub Actions workflow

`.github/workflows/tholos-monitor.yml` (repo root) is what actually runs
this on a schedule — see "Process lifecycle" above for the reasoning, and
"Running it" for the secrets/variables it needs configured. It builds this
package, restores the last saved `monitor-state.json` from the GitHub
Actions cache, runs `dist/main.js` once, and saves the (possibly updated)
state file back to the cache regardless of whether the run succeeded —
that last part matters because a failed run still updates the
consecutive-failure counters, and losing that update would make the next
run mis-alert.

## Testing

```sh
pnpm test
```

Covers `loadConfig`'s validation/defaults and `classify`'s severity rules
(including the payload-dependent `PauseUpdated` case and the
registry-miss-degrades-to-warning fallback) — pure logic, no network calls,
using Node's built-in test runner via `tsx`. There's deliberately no test
here that hits real Soroban RPC; `rpc.ts`/`poller.ts`'s behavior against a
scripted fake RPC (pagination across ticks, cursor persistence across a
simulated restart, retention-gap handling, consecutive-failure alerting and
recovery) was verified during development but isn't part of the committed
suite, since building and maintaining a fake RPC server is more than this
package's test coverage needs going forward — `pnpm test`'s unit tests plus
a real run against testnet (point `.env` at the canonical testnet
deployment) cover it in practice.
