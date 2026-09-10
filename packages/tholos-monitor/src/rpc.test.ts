import assert from "node:assert/strict";
import { test } from "node:test";
import type { rpc } from "@stellar/stellar-sdk";
import { nativeToScVal } from "@stellar/stellar-sdk";
import { RetentionGapError, fetchNewEvents, type FetchNewEventsOptions } from "./rpc.js";

/** Builds one raw event shaped like `rpc.Api.EventResponse` closely enough
 * for `fetchNewEvents`/`decodeEvent` to work on (rpc.ts only ever reads
 * `.topic`, `.value`, `.id`, `.ledger`, `.ledgerClosedAt`, `.txHash` off of
 * it — see decodeEvent). `topic[0]` and `value` are built with the real
 * SDK's `nativeToScVal`, not a plain JS value standing in for one, so these
 * tests exercise the actual `scValToNative` round-trip `decodeEvent` depends
 * on — the same real, installed `@stellar/stellar-sdk` CI and `pnpm test`
 * use, not a stand-in for it. */
function makeRawEvent(eventName: string, data: unknown, id = "0000000001-0000000000") {
  return {
    id,
    ledger: 100,
    ledgerClosedAt: "2024-01-01T00:00:00Z",
    txHash: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
    topic: [nativeToScVal(eventName, { type: "symbol" })],
    value: nativeToScVal(data),
  };
}

type FakePage = { events: ReturnType<typeof makeRawEvent>[]; cursor: string } | Error;

/** A fake `rpc.Server` that returns one page per call from `pages`, in
 * order — standing in for a real RPC endpoint so a test can control exactly
 * what each page of `getEvents` returns (including failing partway through
 * pagination) without a network call. Cast at the boundary rather than
 * declared against `rpc.Server`'s full shape, since `fetchNewEvents` only
 * ever calls `getEvents`/`getLatestLedger` on whatever it's given. */
function makeFakeServer(pages: FakePage[]): rpc.Server {
  let call = 0;
  return {
    getEvents: async (_request: unknown) => {
      const page = pages[call++];
      if (page === undefined) {
        throw new Error(
          `makeFakeServer: getEvents called more times (${call}) than pages provided (${pages.length})`,
        );
      }
      if (page instanceof Error) throw page;
      return { events: page.events, cursor: page.cursor, latestLedger: 999 };
    },
    getLatestLedger: async () => ({ id: "latest", sequence: 1000 }),
  } as unknown as rpc.Server;
}

const BASE_OPTS: Omit<FetchNewEventsOptions, "cursor" | "startLedger"> = {
  deployment: "tholos",
  contractId: "test-contract-id",
};

test("fetchNewEvents: decodes a single page and stops once it's not full", async () => {
  const server = makeFakeServer([
    { events: [makeRawEvent("Asserted", { round: "1" })], cursor: "cursor-1" },
  ]);

  const result = await fetchNewEvents(server, { ...BASE_OPTS, startLedger: 1, limit: 2 });

  assert.equal(result.events.length, 1);
  assert.equal(result.events[0]?.eventName, "Asserted");
  assert.deepEqual(result.events[0]?.data, { round: "1" });
  assert.equal(result.nextCursor, "cursor-1");
  assert.equal(result.hitPageCap, false);
});

test("fetchNewEvents: a page-0 failure throws immediately (nothing decoded yet to preserve)", async () => {
  const server = makeFakeServer([new Error("connect ECONNREFUSED")]);

  await assert.rejects(
    () => fetchNewEvents(server, { ...BASE_OPTS, startLedger: 1, limit: 2 }),
    /ECONNREFUSED/,
  );
});

test("fetchNewEvents: a page-0 failure matching the retention-gap heuristic throws RetentionGapError", async () => {
  const server = makeFakeServer([new Error("start is before oldest ledger")]);

  await assert.rejects(
    () => fetchNewEvents(server, { ...BASE_OPTS, startLedger: 1, limit: 2 }),
    RetentionGapError,
  );
});

test("fetchNewEvents: a later-page failure returns the events already decoded, instead of discarding them (regression guard for the event-loss bug)", async () => {
  const server = makeFakeServer([
    // A full page (== limit) so fetchNewEvents continues on to page 1.
    {
      events: [
        makeRawEvent("Asserted", { round: "1" }),
        makeRawEvent("Asserted", { round: "2" }),
      ],
      cursor: "cursor-page-0",
    },
    new Error("socket hang up"),
  ]);

  const result = await fetchNewEvents(server, { ...BASE_OPTS, startLedger: 1, limit: 2 });

  // Page 0's two events must survive even though page 1 failed — this is
  // exactly the bug the maintainer flagged: a version that throws here
  // instead would discard real, already-fetched RPC data.
  assert.equal(result.events.length, 2);
  assert.deepEqual(
    result.events.map((e) => e.data),
    [{ round: "1" }, { round: "2" }],
  );
  assert.equal(result.nextCursor, "cursor-page-0");
  assert.equal(result.hitPageCap, false);
});

test("fetchNewEvents: the next call resumes from that same cursor, and surfaces the error normally if it's still happening", async () => {
  // Simulates the very next poll tick after the case above: called again
  // with the cursor it returned, and the same underlying problem persists.
  // Nothing has been decoded yet this call, so it's page 0 again — the
  // failure is not lost, just deferred by one run.
  const server = makeFakeServer([new Error("socket hang up")]);

  await assert.rejects(
    () => fetchNewEvents(server, { ...BASE_OPTS, cursor: "cursor-page-0", limit: 2 }),
    /socket hang up/,
  );
});

test("fetchNewEvents: sets hitPageCap when every page up to maxPages comes back full (regression guard for the cap-hit signal itself)", async () => {
  // Every prior test either stops on a non-full page or errors out well
  // before maxPages, so none of them actually drive the loop through a full
  // maxPages of full pages — the one case that exercises the
  // `page === maxPages - 1` branch in rpc.ts. Three full (== limit) pages
  // with no error, maxPages: 3: fetchNewEvents must report hitPageCap: true
  // (there may be more events already on chain this call didn't reach),
  // not just decode everything and call it done.
  const maxPages = 3;
  const limit = 2;
  const server = makeFakeServer([
    {
      events: [makeRawEvent("Asserted", { round: "0a" }), makeRawEvent("Asserted", { round: "0b" })],
      cursor: "cursor-page-0",
    },
    {
      events: [makeRawEvent("Asserted", { round: "1a" }), makeRawEvent("Asserted", { round: "1b" })],
      cursor: "cursor-page-1",
    },
    {
      events: [makeRawEvent("Asserted", { round: "2a" }), makeRawEvent("Asserted", { round: "2b" })],
      cursor: "cursor-page-2",
    },
  ]);

  const result = await fetchNewEvents(server, { ...BASE_OPTS, startLedger: 1, limit, maxPages });

  assert.equal(result.events.length, maxPages * limit);
  assert.equal(result.nextCursor, "cursor-page-2");
  assert.equal(result.hitPageCap, true);
});
