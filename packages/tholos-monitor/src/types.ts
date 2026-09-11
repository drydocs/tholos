/** Which deployment an event came from. `tholos` is v1 (`contracts/tholos`),
 * `tholos-v2` is the separate, never-upgraded-in-place v2 contract
 * (`contracts/tholos-v2`). Kept as a union rather than a boolean so a future
 * third deployment (or a v3) doesn't need a breaking rename. */
export type Deployment = "tholos" | "tholos-v2";

/** How urgently a human should be told about an event. `critical` is paged
 * immediately (webhook fires unconditionally); `warning` and `info` are
 * always logged, and alerted only if `ALERT_MIN_SEVERITY` is set low enough
 * to include them (see config.ts). */
export type Severity = "info" | "warning" | "critical";

/** A contract event, decoded from Soroban RPC's `getEvents` into native JS
 * values. `eventName` is topic[0] (every event emitted via soroban-sdk's
 * `#[contractevent]` macro carries the struct name as its first topic), and
 * `topics`/`data` are the remaining topic fields and the event body,
 * already run through `scValToNative`. */
export interface DecodedEvent {
  deployment: Deployment;
  contractId: string;
  /** Soroban RPC's opaque event id, e.g. "0000000123456789-0000000001". Not
   * the same as the pagination cursor (see rpc.ts). */
  id: string;
  ledger: number;
  ledgerClosedAt: string;
  txHash: string;
  eventName: string;
  /** topic[1..] (topic[0] is `eventName`), decoded to native values. */
  topics: unknown[];
  data: unknown;
}

/** A classified event, ready to log and (maybe) alert on. */
export interface ClassifiedEvent extends DecodedEvent {
  severity: Severity;
  /** Human-readable one-liner for why this event matters, from the event
   * registry (events.ts). Included in alert payloads so an on-call reader
   * doesn't have to go look up what e.g. `RotationCancelled` means. */
  description: string;
}

/** Fired when the poller itself can't keep the promise this service exists
 * to make ("you'll hear about a problem, not discover it later") — repeated
 * RPC failures, or a state gap from falling outside the RPC's retention
 * window. Distinct from a `ClassifiedEvent` alert: this is about the
 * monitor's own health, not a contract event. */
export interface LivenessAlert {
  deployment: Deployment | "global";
  kind: "rpc_failure" | "retention_gap" | "recovered";
  severity: Severity;
  message: string;
  consecutiveFailures?: number;
  lastError?: string;
}

export type AlertPayload =
  | { type: "contract_event"; event: ClassifiedEvent; timestamp: string }
  | { type: "monitor_liveness"; alert: LivenessAlert; timestamp: string };

/** Per-deployment state persisted to disk (state.ts). This is a scheduled
 * script, not a long-running daemon (see README's "Process lifecycle" —
 * that's the maintainer's call on issue #189, GitHub Actions cron workflow
 * re-invokes this fresh each time), so anything that needs to survive
 * between runs — not just the RPC cursor, but also the consecutive-failure
 * count — has to live here rather than in an in-memory tracker. */
export interface DeploymentState {
  /** Soroban RPC's opaque paging token; absent until the first successful
   * run. */
  cursor?: string;
  /** Runs in a row (across separate script invocations) that failed to
   * poll this deployment. Reset to 0 on the next successful run. */
  consecutiveFailures: number;
  /** Set once an `rpc_failure` liveness alert has fired for the current
   * failure streak, so recovery only alerts once too instead of on every
   * successful run forever. */
  alertedForFailureStreak: boolean;
}

export interface MonitorState {
  deployments: Partial<Record<Deployment, DeploymentState>>;
}
