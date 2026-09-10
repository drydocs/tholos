import type { rpc } from "@stellar/stellar-sdk";
import { buildEventAlertPayload, buildLivenessAlertPayload, sendAlert } from "./alerts.js";
import type { Config } from "./config.js";
import { classify } from "./events.js";
import {
  RetentionGapError,
  createServer,
  fetchNewEvents,
  getLatestLedgerSequence,
  type FetchNewEventsOptions,
} from "./rpc.js";
import { loadState, saveState } from "./state.js";
import type {
  ClassifiedEvent,
  Deployment,
  DecodedEvent,
  DeploymentState,
  MonitorState,
  Severity,
} from "./types.js";

const SEVERITY_RANK: Record<Severity, number> = {
  info: 0,
  warning: 1,
  critical: 2,
};

function meetsThreshold(severity: Severity, min: Severity): boolean {
  return SEVERITY_RANK[severity] >= SEVERITY_RANK[min];
}

function classifyEvent(event: DecodedEvent): ClassifiedEvent {
  const { severity, description } = classify(event.eventName, event.data);
  return { ...event, severity, description };
}

function logEvent(event: ClassifiedEvent): void {
  console.log(
    JSON.stringify({
      at: new Date().toISOString(),
      level: "event",
      deployment: event.deployment,
      contractId: event.contractId,
      eventName: event.eventName,
      severity: event.severity,
      ledger: event.ledger,
      txHash: event.txHash,
      description: event.description,
    }),
  );
}

export interface ClassifyLogAndAlertOptions {
  alertMinSeverity: Severity;
  alertWebhookUrl: string;
  requestTimeoutMs: number;
  /** Caps how many `sendAlert` calls run at once (see `runWithConcurrencyLimit`
   * below for why this needs a real cap, not just "fire them all"). */
  maxConcurrentAlerts: number;
}

/**
 * Runs `fn` over `items`, at most `limit` calls in flight at once, and
 * resolves once every item has been processed. A small fixed-size worker
 * pool: each of up to `limit` workers pulls the next not-yet-started item
 * off the shared list and awaits `fn` on it before pulling another, so
 * concurrency never exceeds `limit` regardless of how many items there are
 * or how long any one call takes (unlike chunking `items` into groups of
 * `limit` and awaiting each group in turn, which would let a single slow
 * item in a group hold up starting the next group's items even though
 * there's spare capacity).
 */
async function runWithConcurrencyLimit<T>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<void>,
): Promise<void> {
  let nextIndex = 0;
  async function worker(): Promise<void> {
    while (nextIndex < items.length) {
      const item = items[nextIndex++] as T;
      await fn(item);
    }
  }
  const workerCount = Math.min(limit, items.length);
  await Promise.all(Array.from({ length: workerCount }, () => worker()));
}

/**
 * Classifies and logs every decoded event, then alerts on whichever of them
 * meet `alertMinSeverity` — concurrently, not one at a time, but bounded at
 * `maxConcurrentAlerts` in flight together rather than all of them at once.
 * sendAlert never throws (see alerts.ts), so running this many at a time
 * concurrently and awaiting them together is safe with respect to unhandled
 * rejections — but that's a different concern from how many simultaneous
 * requests land on `alertWebhookUrl` at once: on a first run against the
 * full INITIAL_LEDGER_LOOKBACK, or a run after downtime, dozens to hundreds
 * of events can meet threshold in one tick, and firing all of their alerts
 * at once risks overwhelming or getting rate-limited by whatever's actually
 * receiving the webhook. Bounding concurrency avoids that burst while still
 * being far faster than dispatching one at a time (see poller.test.ts for
 * why both — bounded, but still concurrent — are asserted).
 *
 * Pulled out of `pollOneDeployment` (which still calls this) so the
 * concurrency behavior itself is independently testable against plain
 * `DecodedEvent` fixtures and a local test HTTP server, without needing a
 * real or fake `rpc.Server` — see poller.test.ts.
 */
export async function classifyLogAndAlert(
  events: DecodedEvent[],
  options: ClassifyLogAndAlertOptions,
): Promise<void> {
  const toAlert: ClassifiedEvent[] = [];
  for (const decoded of events) {
    const classified = classifyEvent(decoded);
    logEvent(classified);
    if (meetsThreshold(classified.severity, options.alertMinSeverity)) {
      toAlert.push(classified);
    }
  }
  await runWithConcurrencyLimit(toAlert, options.maxConcurrentAlerts, (classified) =>
    sendAlert(buildEventAlertPayload(classified), {
      webhookUrl: options.alertWebhookUrl,
      timeoutMs: options.requestTimeoutMs,
    }),
  );
}

/** Builds a `DeploymentState`, omitting `cursor` entirely rather than
 * setting it to `undefined` when there isn't one — required under
 * `exactOptionalPropertyTypes` (an optional property may be absent, but not
 * explicitly `undefined`), and it's also just the correct shape: "no
 * cursor" and "cursor: undefined" would otherwise be two different-looking
 * ways to say the same thing in the persisted JSON. */
function buildDeploymentState(
  consecutiveFailures: number,
  alertedForFailureStreak: boolean,
  cursor?: string,
): DeploymentState {
  return cursor !== undefined
    ? { cursor, consecutiveFailures, alertedForFailureStreak }
    : { consecutiveFailures, alertedForFailureStreak };
}

/**
 * Polls one deployment exactly once. Returns `true` if this run did not
 * complete cleanly for this deployment (a retention gap or any other
 * error) — the caller uses this to decide the process's exit code, so a
 * bad run shows up as a failed GitHub Actions run, alongside the webhook
 * alert that already fired for the same problem.
 *
 * All of a deployment's cross-run state — not just the RPC cursor, but
 * also the consecutive-failure count and whether this streak already
 * alerted — lives in `state.deployments[deployment]` and is read/written
 * here. There's no in-memory equivalent: this is a scheduled script
 * re-invoked fresh by a GitHub Actions cron workflow (see README's
 * "Process lifecycle"), not a long-running daemon, so nothing survives
 * between runs except what's explicitly persisted (state.ts).
 */
async function pollOneDeployment(
  server: rpc.Server,
  config: Config,
  deployment: Deployment,
  contractId: string,
  state: MonitorState,
): Promise<boolean> {
  const existing = state.deployments[deployment];
  const cursor = existing?.cursor;
  const consecutiveFailuresBefore = existing?.consecutiveFailures ?? 0;
  const alertedForFailureStreak = existing?.alertedForFailureStreak ?? false;

  try {
    let startLedger: number | undefined;
    if (!cursor) {
      // Covers both "first run against a fresh state file" and "the run
      // right after a retention-gap reset dropped the stale cursor below."
      // Deliberately inside the try: a failure fetching the latest ledger
      // (e.g. the RPC being down) is exactly the same kind of failure as
      // fetchNewEvents failing below, and needs the same
      // consecutive-failure tracking/alerting rather than crashing this
      // run uncaught.
      const latest = await getLatestLedgerSequence(server);
      startLedger = Math.max(1, latest - config.initialLedgerLookback);
      console.log(
        `[monitor] ${deployment}: no saved cursor, starting from ledger ${startLedger} (latest ${latest} minus ${config.initialLedgerLookback}-ledger lookback).`,
      );
    }

    const fetchOptions: FetchNewEventsOptions = {
      deployment,
      contractId,
      maxPages: 20,
    };
    if (cursor !== undefined) fetchOptions.cursor = cursor;
    if (startLedger !== undefined) fetchOptions.startLedger = startLedger;
    const result = await fetchNewEvents(server, fetchOptions);

    // See classifyLogAndAlert's own doc comment for why this dispatches
    // alerts concurrently rather than one at a time.
    await classifyLogAndAlert(result.events, {
      alertMinSeverity: config.alertMinSeverity,
      alertWebhookUrl: config.alertWebhookUrl,
      requestTimeoutMs: config.requestTimeoutMs,
      maxConcurrentAlerts: config.alertMaxConcurrency,
    });

    if (result.hitPageCap) {
      console.warn(
        `[monitor] ${deployment}: hit the per-run page cap with more events likely still pending; will continue from cursor ${result.nextCursor} next run.`,
      );
    }

    if (consecutiveFailuresBefore > 0 && alertedForFailureStreak) {
      await sendAlert(
        buildLivenessAlertPayload({
          deployment,
          kind: "recovered",
          severity: "info",
          message: `${deployment}: polling recovered after ${consecutiveFailuresBefore} consecutive failed run(s).`,
        }),
        { webhookUrl: config.alertWebhookUrl, timeoutMs: config.requestTimeoutMs },
      );
    }

    state.deployments[deployment] = buildDeploymentState(0, false, result.nextCursor);
    return false;
  } catch (err) {
    const lastError = (err as Error).message;

    if (err instanceof RetentionGapError) {
      console.error(`[monitor] ${deployment}: ${lastError}`);
      await sendAlert(
        buildLivenessAlertPayload({
          deployment,
          kind: "retention_gap",
          severity: "critical",
          message: `${deployment}: ${lastError} Re-anchoring to the current chain tip minus the configured lookback on the next run; events in the gap are unrecoverable from this RPC.`,
          lastError,
        }),
        { webhookUrl: config.alertWebhookUrl, timeoutMs: config.requestTimeoutMs },
      );
      // Drop the stale cursor so the next run re-derives a fresh
      // startLedger, instead of repeating the same failing request every
      // run from here on. A retention gap isn't the same kind of failure
      // as an RPC error below (it's expected once this falls behind far
      // enough, and it's already alerted unconditionally above), so it
      // deliberately doesn't touch the consecutive-failure count.
      state.deployments[deployment] = buildDeploymentState(
        consecutiveFailuresBefore,
        alertedForFailureStreak,
      );
      return true;
    }

    const consecutiveFailures = consecutiveFailuresBefore + 1;
    console.error(
      `[monitor] ${deployment}: poll failed (${consecutiveFailures} consecutive): ${lastError}`,
    );

    let nowAlerted = alertedForFailureStreak;
    if (
      consecutiveFailures >= config.consecutiveFailuresBeforeAlert &&
      !alertedForFailureStreak
    ) {
      nowAlerted = true;
      await sendAlert(
        buildLivenessAlertPayload({
          deployment,
          kind: "rpc_failure",
          severity: "critical",
          message: `${deployment}: ${consecutiveFailures} consecutive failed runs against ${config.rpcUrl}.`,
          consecutiveFailures,
          lastError,
        }),
        { webhookUrl: config.alertWebhookUrl, timeoutMs: config.requestTimeoutMs },
      );
    }

    state.deployments[deployment] = buildDeploymentState(
      consecutiveFailures,
      nowAlerted,
      cursor,
    );
    return true;
  }
}

/**
 * Runs one poll pass over every configured deployment and persists state
 * once at the end, then returns. No internal loop, no timers, no signal
 * handling — this is invoked fresh for each GitHub Actions cron run (see
 * `.github/workflows/tholos-monitor.yml` and README's "Process lifecycle";
 * that shape was the maintainer's explicit call on issue #189, not a
 * long-running daemon).
 *
 * Returns `true` if any deployment's poll didn't complete cleanly this
 * run, so `main.ts` can exit non-zero — that marks the GitHub Actions run
 * itself as failed, a second, independent signal alongside the webhook
 * alert that already fired for the same problem.
 */
export async function runOnce(config: Config): Promise<boolean> {
  const server = createServer(config.rpcUrl);
  const state = await loadState(config.stateFilePath);
  const deployments = Object.entries(config.contracts) as [Deployment, string][];

  console.log(
    `[monitor] run starting: ${deployments.map(([d, id]) => `${d}=${id}`).join(", ")}; rpc=${config.rpcUrl}; alertMinSeverity=${config.alertMinSeverity}`,
  );

  let anyFailures = false;
  for (const [deployment, contractId] of deployments) {
    const failed = await pollOneDeployment(server, config, deployment, contractId, state);
    if (failed) anyFailures = true;
  }

  try {
    await saveState(config.stateFilePath, state);
  } catch (err) {
    // A failed state write means the next run may reprocess or lose track
    // of a range — a real problem, and this run's exit code should say so
    // (the caller marks the GitHub Actions run failed), even though the
    // alerting/logging above already ran fine.
    console.error(`[monitor] failed to save state: ${(err as Error).message}`);
    anyFailures = true;
  }

  console.log(`[monitor] run finished${anyFailures ? " with failures" : ""}.`);
  return anyFailures;
}
