import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { classifyLogAndAlert } from "./poller.js";
import type { DecodedEvent } from "./types.js";

function makeEvent(overrides: Partial<DecodedEvent> = {}): DecodedEvent {
  return {
    deployment: "tholos",
    contractId: "test-contract-id",
    id: `id-${Math.random()}`,
    ledger: 100,
    ledgerClosedAt: "2024-01-01T00:00:00Z",
    txHash: "deadbeef",
    eventName: "PauseUpdated",
    topics: [],
    data: { paused: true },
    ...overrides,
  };
}

/** Starts a local webhook double that holds every request open for
 * `delayMs` before responding 200, and tracks how many requests were open
 * at once. Standing in for a real (slow) webhook without a network call or
 * a fixed sleep — see the "dispatches concurrently" test below for why the
 * timing/count it reports is what proves the concurrency fix. */
function startDelayedWebhook(delayMs: number) {
  let inFlight = 0;
  let maxInFlight = 0;
  let requestCount = 0;

  const server = http.createServer((_req, res) => {
    requestCount++;
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    setTimeout(() => {
      inFlight--;
      res.writeHead(200, { "content-type": "application/json" });
      res.end("{}");
    }, delayMs);
  });

  return {
    server,
    listen: () =>
      new Promise<string>((resolve) => {
        server.listen(0, "127.0.0.1", () => {
          const address = server.address();
          if (address === null || typeof address === "string") {
            throw new Error("expected the test webhook to bind to a TCP port");
          }
          resolve(`http://127.0.0.1:${address.port}`);
        });
      }),
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
    get requestCount() {
      return requestCount;
    },
    get maxInFlight() {
      return maxInFlight;
    },
  };
}

test("classifyLogAndAlert: dispatches alerts concurrently, not one at a time (regression guard for the concurrency fix)", async () => {
  const DELAY_MS = 200;
  const webhook = startDelayedWebhook(DELAY_MS);
  const webhookUrl = await webhook.listen();

  try {
    // All four are "critical" (PauseUpdated with paused:true), so all four
    // alert — with alertMinSeverity: "info" every one of them would too,
    // but using the real threshold here also doubles as a sanity check that
    // classification still runs correctly through the extracted function.
    // maxConcurrentAlerts >= events.length so the cap itself doesn't limit
    // this run — that's covered separately below.
    const events = [makeEvent(), makeEvent(), makeEvent(), makeEvent()];

    const start = Date.now();
    await classifyLogAndAlert(events, {
      alertMinSeverity: "warning",
      alertWebhookUrl: webhookUrl,
      requestTimeoutMs: 5000,
      maxConcurrentAlerts: events.length,
    });
    const elapsedMs = Date.now() - start;

    assert.equal(webhook.requestCount, 4);
    // Sequential dispatch (awaiting each sendAlert before starting the
    // next) would take roughly events.length * DELAY_MS (~800ms here);
    // concurrent dispatch takes roughly one DELAY_MS regardless of how many
    // events there are. The threshold is well inside that gap so ordinary
    // CI scheduling jitter can't make this flaky, while a reverted
    // concurrency fix still fails it.
    assert.ok(
      elapsedMs < DELAY_MS * events.length * 0.6,
      `expected concurrent dispatch (~${DELAY_MS}ms), took ${elapsedMs}ms — looks sequential (~${DELAY_MS * events.length}ms)`,
    );
    assert.ok(
      webhook.maxInFlight > 1,
      `expected more than one alert in flight at once, observed max ${webhook.maxInFlight}`,
    );
  } finally {
    await webhook.close();
  }
});

test("classifyLogAndAlert: never exceeds maxConcurrentAlerts in flight at once (regression guard for the unbounded-burst finding)", async () => {
  const DELAY_MS = 150;
  const MAX_CONCURRENT_ALERTS = 2;
  const webhook = startDelayedWebhook(DELAY_MS);
  const webhookUrl = await webhook.listen();

  try {
    // More events than the cap, so this only passes if dispatch is both
    // concurrent (not sequential — same bound as the test above) AND capped
    // (not "all 6 at once" — what the maintainer flagged: an unbounded
    // burst can overwhelm or get rate-limited by whatever's receiving the
    // webhook).
    const events = Array.from({ length: 6 }, () => makeEvent());

    const start = Date.now();
    await classifyLogAndAlert(events, {
      alertMinSeverity: "warning",
      alertWebhookUrl: webhookUrl,
      requestTimeoutMs: 5000,
      maxConcurrentAlerts: MAX_CONCURRENT_ALERTS,
    });
    const elapsedMs = Date.now() - start;

    assert.equal(webhook.requestCount, 6);
    assert.ok(
      webhook.maxInFlight <= MAX_CONCURRENT_ALERTS,
      `expected at most ${MAX_CONCURRENT_ALERTS} alerts in flight at once, observed max ${webhook.maxInFlight}`,
    );
    assert.ok(
      webhook.maxInFlight > 1,
      `expected more than one alert in flight at once (bounded concurrency, not sequential), observed max ${webhook.maxInFlight}`,
    );
    // 6 events at a cap of 2 takes 3 "rounds" of DELAY_MS (~450ms) — well
    // above one round (rules out "cap ignored, ran all 6 at once") and well
    // below 6 rounds (~900ms, rules out "fell back to fully sequential").
    assert.ok(
      elapsedMs > DELAY_MS * 2 && elapsedMs < DELAY_MS * 5,
      `expected ~${DELAY_MS * 3}ms (3 rounds at a cap of ${MAX_CONCURRENT_ALERTS}), took ${elapsedMs}ms`,
    );
  } finally {
    await webhook.close();
  }
});

test("classifyLogAndAlert: only alerts on events meeting alertMinSeverity (still true after the concurrency extraction)", async () => {
  const webhook = startDelayedWebhook(0);
  const webhookUrl = await webhook.listen();

  try {
    const events = [
      makeEvent({ eventName: "PauseUpdated", data: { paused: false } }), // info
      makeEvent({ eventName: "PauseUpdated", data: { paused: true } }), // critical
      makeEvent({ eventName: "SomeFutureEventNotYetRegistered", data: {} }), // warning
    ];

    await classifyLogAndAlert(events, {
      alertMinSeverity: "warning",
      alertWebhookUrl: webhookUrl,
      requestTimeoutMs: 5000,
      maxConcurrentAlerts: 5,
    });

    // The "info" event is below threshold and must not alert; the other two
    // (warning, critical) must.
    assert.equal(webhook.requestCount, 2);
  } finally {
    await webhook.close();
  }
});
