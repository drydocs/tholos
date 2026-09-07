import assert from "node:assert/strict";
import { test } from "node:test";
import { ConfigError, loadConfig } from "./config.js";

// Deliberately not shaped like a real Stellar contract address (which is
// "C" + 55 base32 characters) — loadConfig doesn't validate the format, it
// just passes the value through, and CI's "Block committed contract
// addresses" check (ci.yml) flags anything matching that shape wherever it
// appears, test fixtures included.
const BASE_ENV = {
  THOLOS_V1_CONTRACT_ID: "test-v1-contract-id",
  ALERT_WEBHOOK_URL: "https://example.com/hooks/tholos",
};

test("loadConfig: requires at least one contract id", () => {
  assert.throws(
    () => loadConfig({ ALERT_WEBHOOK_URL: BASE_ENV.ALERT_WEBHOOK_URL }),
    ConfigError,
  );
});

test("loadConfig: requires ALERT_WEBHOOK_URL", () => {
  assert.throws(
    () => loadConfig({ THOLOS_V1_CONTRACT_ID: BASE_ENV.THOLOS_V1_CONTRACT_ID }),
    ConfigError,
  );
});

test("loadConfig: rejects a malformed webhook URL", () => {
  assert.throws(
    () =>
      loadConfig({
        THOLOS_V1_CONTRACT_ID: BASE_ENV.THOLOS_V1_CONTRACT_ID,
        ALERT_WEBHOOK_URL: "not-a-url",
      }),
    ConfigError,
  );
});

test("loadConfig: applies documented defaults", () => {
  const config = loadConfig(BASE_ENV);
  assert.equal(config.rpcUrl, "https://soroban-testnet.stellar.org");
  assert.equal(config.networkPassphrase, "Test SDF Network ; September 2015");
  assert.equal(config.alertMinSeverity, "warning");
  assert.deepEqual(config.contracts, { tholos: BASE_ENV.THOLOS_V1_CONTRACT_ID });
});

test("loadConfig: both contract ids can be set together", () => {
  const config = loadConfig({
    ...BASE_ENV,
    THOLOS_V2_CONTRACT_ID: "test-v2-contract-id",
  });
  assert.deepEqual(config.contracts, {
    tholos: BASE_ENV.THOLOS_V1_CONTRACT_ID,
    "tholos-v2": "test-v2-contract-id",
  });
});

test("loadConfig: rejects a non-integer CONSECUTIVE_FAILURES_BEFORE_ALERT", () => {
  assert.throws(
    () => loadConfig({ ...BASE_ENV, CONSECUTIVE_FAILURES_BEFORE_ALERT: "not-a-number" }),
    ConfigError,
  );
});

test("loadConfig: rejects an invalid ALERT_MIN_SEVERITY", () => {
  assert.throws(
    () => loadConfig({ ...BASE_ENV, ALERT_MIN_SEVERITY: "urgent" }),
    ConfigError,
  );
});
