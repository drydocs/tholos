import { rpc, scValToNative } from "@stellar/stellar-sdk";
import type { Deployment, DecodedEvent } from "./types.js";

/** Soroban RPC only retains events for a bounded ledger window (a public
 * RPC endpoint typically keeps on the order of a day's worth on mainnet,
 * more on testnet — the exact figure is an operator setting, not a
 * contract-level constant). A saved cursor (or a computed `startLedger`)
 * that falls outside that window makes `getEvents` fail rather than
 * silently skip ahead, which is exactly the kind of gap this service
 * exists to surface rather than paper over.
 *
 * There's no dedicated error code for this in the JSON-RPC response as of
 * @stellar/stellar-sdk 16.2.0 — it comes back as a generic invocation
 * error, so this is a message-text heuristic. If the RPC provider's wording
 * doesn't match, `fetchNewEvents` still throws (surfaced as a normal
 * `rpc_failure` liveness alert via the poller's consecutive-failure
 * tracking, see poller.ts) — it just isn't distinguished as a gap
 * specifically. Tighten this if you see false negatives/positives against
 * your actual RPC provider's error text.
 */
const RETENTION_GAP_PATTERN = /oldest ledger|start.*ledger|ledger.*range|cursor/i;

export class RetentionGapError extends Error {
  constructor(cause: unknown) {
    super(
      `Requested a ledger range/cursor the RPC no longer retains — there is a gap in event coverage. Underlying error: ${
        cause instanceof Error ? cause.message : String(cause)
      }`,
    );
    this.name = "RetentionGapError";
  }
}

export function isLikelyRetentionGap(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err);
  return RETENTION_GAP_PATTERN.test(message);
}

export function createServer(rpcUrl: string): rpc.Server {
  return new rpc.Server(rpcUrl, { allowHttp: rpcUrl.startsWith("http://") });
}

export async function getLatestLedgerSequence(
  server: rpc.Server,
): Promise<number> {
  const { sequence } = await server.getLatestLedger();
  return sequence;
}

function decodeEvent(
  deployment: Deployment,
  contractId: string,
  event: rpc.Api.EventResponse,
): DecodedEvent {
  const topics = event.topic.map((t) => scValToNative(t));
  // Every event emitted through soroban-sdk's `#[contractevent]` macro
  // carries the struct name as its first topic (see e.g. `Asserted`,
  // `PauseUpdated` in contracts/tholos/src/lib.rs) — that's `eventName`
  // here. The rest of `topic` (fields marked `#[topic]` after the first)
  // are kept in `topics`, separate from `data` (the non-topic fields,
  // Soroban RPC's `value`).
  const [eventName, ...restTopics] = topics;
  return {
    deployment,
    contractId,
    id: event.id,
    ledger: event.ledger,
    ledgerClosedAt: event.ledgerClosedAt,
    txHash: event.txHash,
    eventName: typeof eventName === "string" ? eventName : String(eventName),
    topics: restTopics,
    data: scValToNative(event.value),
  };
}

export interface FetchNewEventsOptions {
  deployment: Deployment;
  contractId: string;
  /** Resume from here if set (a prior poll's saved cursor). Mutually
   * exclusive with `startLedger` in practice — pass exactly one. */
  cursor?: string;
  /** Only used when there's no saved cursor yet (first run against a fresh
   * state file). */
  startLedger?: number;
  /** Per-request page size, forwarded to `getEvents`. */
  limit?: number;
  /** Safety cap on pages fetched in a single call, so a contract that's
   * emitted an unusually large backlog since the last poll can't turn one
   * tick into an unbounded loop. Hitting this is itself worth knowing
   * about (see poller.ts's `hitPageCap` handling) — it means this poll
   * tick didn't catch up to the chain tip and the next tick has to
   * continue from where this one stopped. */
  maxPages?: number;
}

export interface FetchNewEventsResult {
  events: DecodedEvent[];
  /** Cursor to pass as `cursor` on the next call. Always present: Soroban
   * RPC returns one even for an empty page (it still advances past the
   * ledgers it checked and found nothing in). */
  nextCursor: string;
  /** True if `maxPages` was hit — there may be more events already on
   * chain that this call didn't reach. */
  hitPageCap: boolean;
}

/**
 * Fetches every new "contract" event (i.e., excluding system/diagnostic
 * events, which aren't meaningful application state changes) for one
 * contract since the last saved cursor, paginating until caught up or
 * `maxPages` is hit.
 *
 * Deliberately does not filter by topic: soroban-sdk's `getEvents` topic
 * filters need each segment pre-encoded to base64 XDR, which would mean
 * hardcoding one filter per event name in `events.ts`'s registry — and
 * silently missing any event a future contract upgrade adds until this
 * package is updated to filter for it too. Fetching every event for the
 * contract and classifying client-side (see `events.ts`) means a new,
 * not-yet-registered event still shows up, tagged `warning` by
 * `classify`'s fallback, instead of vanishing.
 */
export async function fetchNewEvents(
  server: rpc.Server,
  opts: FetchNewEventsOptions,
): Promise<FetchNewEventsResult> {
  const limit = opts.limit ?? 100;
  const maxPages = opts.maxPages ?? 20;

  const events: DecodedEvent[] = [];
  let cursor = opts.cursor;
  let startLedger = cursor ? undefined : opts.startLedger;
  let nextCursor = cursor;
  let hitPageCap = false;

  for (let page = 0; page < maxPages; page++) {
    let response: rpc.Api.GetEventsResponse;
    try {
      // Api.GetEventsRequest is a strict discriminated union (cursor mode
      // XOR startLedger mode, each explicitly typing the other's field as
      // `never`) — not one interface with both fields optional. A single
      // object literal built by conditionally assigning either field
      // afterwards doesn't type-check against that union, so each mode is
      // constructed as its own complete literal instead.
      const filters = [{ type: "contract" as const, contractIds: [opts.contractId] }];
      let request: rpc.Api.GetEventsRequest;
      if (cursor !== undefined) {
        request = { filters, cursor, limit };
      } else if (startLedger !== undefined) {
        request = { filters, startLedger, limit };
      } else {
        // Guarded by the caller (poller.ts always supplies one or the
        // other), but fail loudly rather than silently defaulting to some
        // arbitrary ledger if that invariant is ever broken.
        throw new Error(
          "fetchNewEvents: neither cursor nor startLedger was provided.",
        );
      }
      response = await server.getEvents(request);
    } catch (err) {
      if (isLikelyRetentionGap(err)) {
        throw new RetentionGapError(err);
      }
      throw err;
    }

    for (const raw of response.events) {
      events.push(decodeEvent(opts.deployment, opts.contractId, raw));
    }

    nextCursor = response.cursor;
    cursor = response.cursor;
    startLedger = undefined;

    if (response.events.length < limit) {
      // Fewer than a full page means we've caught up to the chain tip (or
      // the RPC's own latest indexed ledger) for this contract.
      break;
    }
    if (page === maxPages - 1) {
      hitPageCap = true;
    }
  }

  if (nextCursor === undefined) {
    // Only possible if maxPages is 0, which the default never produces —
    // guard anyway so the return type doesn't have to be optional.
    throw new Error("fetchNewEvents: no cursor advanced (maxPages was 0?).");
  }

  return { events, nextCursor, hitPageCap };
}
