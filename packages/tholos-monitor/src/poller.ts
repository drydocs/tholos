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

    // Classifying and logging is synchronous and cheap; the slow part is
    // sendAlert (up to 3 attempts, capped exponential backoff, up to ~10s
    // per attempt against a struggling webhook). Awaiting each alert before
    // starting the next serializes all of that — on a first run against the
    // full INITIAL_LEDGER_LOOKBACK, or right after downtime, dozens of
    // events could take many minutes to alert on one at a time, risking
    // overlap with the next 5-minute cron tick and delaying whichever
    // events land later in the batch. sendAlert never throws (see
    // alerts.ts), so firing them concurrently and awaiting them together is
    // safe — a slow or failing webhook no longer blocks the rest.
    const alertsInFlight: Promise<void>[] = [];
    for (const decoded of result.events) {
      const classified = classifyEvent(decoded);
      logEvent(classified);
      if (meetsThreshold(classified.severity, config.alertMinSeverity)) {
        alertsInFlight.push(
          sendAlert(buildEventAlertPayload(classified), {
            webhookUrl: config.alertWebhookUrl,
            timeoutMs: config.requestTimeoutMs,
          }),
        );
      }
    }
    await Promise.all(alertsInFlight);

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
