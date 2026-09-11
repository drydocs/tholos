import type { Deployment, Severity } from "./types.js";

/** Matches demos/freelance-escrow's src/lib/config.ts defaults (see its
 * README/config.ts): the same public Soroban RPC and the same testnet
 * network passphrase, just without the Vite `VITE_` prefix since this is a
 * plain Node service, not a Vite app. */
const DEFAULT_RPC_URL = "https://soroban-testnet.stellar.org";
const DEFAULT_NETWORK_PASSPHRASE = "Test SDF Network ; September 2015";
const DEFAULT_STATE_FILE_PATH = "./monitor-state.json";
const DEFAULT_ALERT_MIN_SEVERITY: Severity = "warning";
const DEFAULT_CONSECUTIVE_FAILURES_BEFORE_ALERT = 3;
const DEFAULT_REQUEST_TIMEOUT_MS = 10_000;
const DEFAULT_INITIAL_LEDGER_LOOKBACK = 17_280; // ~1 day at ~5s/ledger
const DEFAULT_ALERT_MAX_CONCURRENCY = 5;

export interface Config {
  rpcUrl: string;
  networkPassphrase: string;
  /** At least one of these two is always present; validated in
   * `loadConfig`. Both is the common case (monitor both deployments from
   * one process). */
  contracts: Partial<Record<Deployment, string>>;
  alertWebhookUrl: string;
  stateFilePath: string;
  alertMinSeverity: Severity;
  /** Consecutive failed runs (this is a scheduled script re-invoked by a
   * GitHub Actions cron workflow, not a long-running process — see
   * README's "Process lifecycle" — so "consecutive" spans separate
   * invocations, tracked in the persisted state file, not in memory) for a
   * given deployment before this service alerts on itself rather than just
   * logging. */
  consecutiveFailuresBeforeAlert: number;
  requestTimeoutMs: number;
  /** How far back to look when there's no saved cursor yet: the very first
   * run against a fresh state file, or the run right after a retention-gap
   * reset. */
  initialLedgerLookback: number;
  /** Caps how many alert webhook POSTs are in flight at once (see
   * poller.ts's `classifyLogAndAlert`). A poll tick that decodes many
   * threshold-meeting events at once — a first run against the full
   * `initialLedgerLookback`, or a run after downtime — dispatches them
   * concurrently rather than one at a time, but still bounded: an
   * unbounded burst risks overwhelming or getting rate-limited by
   * whatever's on the other end of `alertWebhookUrl`. */
  alertMaxConcurrency: number;
}

export class ConfigError extends Error {}

function optionalEnv(
  env: NodeJS.ProcessEnv,
  name: string,
): string | undefined {
  const value = env[name];
  return value && value.trim() !== "" ? value.trim() : undefined;
}

function parsePositiveInt(
  name: string,
  raw: string | undefined,
  fallback: number,
): number {
  if (raw === undefined) return fallback;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || !Number.isInteger(parsed) || parsed <= 0) {
    throw new ConfigError(
      `${name}="${raw}" must be a positive integer (got an invalid value).`,
    );
  }
  return parsed;
}

function parseSeverity(raw: string | undefined, fallback: Severity): Severity {
  if (raw === undefined) return fallback;
  const normalized = raw.trim().toLowerCase();
  if (normalized === "info" || normalized === "warning" || normalized === "critical") {
    return normalized;
  }
  throw new ConfigError(
    `ALERT_MIN_SEVERITY="${raw}" must be one of: info, warning, critical.`,
  );
}

/**
 * Reads and validates configuration from environment variables. Fails fast
 * (throws `ConfigError`) on anything that would otherwise surface as a
 * confusing runtime error later — see README.md for the full list of
 * variables and what each one does.
 */
export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const tholos = optionalEnv(env, "THOLOS_V1_CONTRACT_ID");
  const tholosV2 = optionalEnv(env, "THOLOS_V2_CONTRACT_ID");
  if (!tholos && !tholosV2) {
    throw new ConfigError(
      "Set at least one of THOLOS_V1_CONTRACT_ID or THOLOS_V2_CONTRACT_ID — nothing to monitor otherwise.",
    );
  }

  const alertWebhookUrl = optionalEnv(env, "ALERT_WEBHOOK_URL");
  if (!alertWebhookUrl) {
    throw new ConfigError("ALERT_WEBHOOK_URL is required.");
  }
  try {
    void new URL(alertWebhookUrl);
  } catch {
    throw new ConfigError(`ALERT_WEBHOOK_URL="${alertWebhookUrl}" is not a valid URL.`);
  }

  const contracts: Partial<Record<Deployment, string>> = {};
  if (tholos) contracts.tholos = tholos;
  if (tholosV2) contracts["tholos-v2"] = tholosV2;

  return {
    rpcUrl: optionalEnv(env, "SOROBAN_RPC_URL") ?? DEFAULT_RPC_URL,
    networkPassphrase:
      optionalEnv(env, "NETWORK_PASSPHRASE") ?? DEFAULT_NETWORK_PASSPHRASE,
    contracts,
    alertWebhookUrl,
    stateFilePath: optionalEnv(env, "STATE_FILE_PATH") ?? DEFAULT_STATE_FILE_PATH,
    alertMinSeverity: parseSeverity(
      optionalEnv(env, "ALERT_MIN_SEVERITY"),
      DEFAULT_ALERT_MIN_SEVERITY,
    ),
    consecutiveFailuresBeforeAlert: parsePositiveInt(
      "CONSECUTIVE_FAILURES_BEFORE_ALERT",
      optionalEnv(env, "CONSECUTIVE_FAILURES_BEFORE_ALERT"),
      DEFAULT_CONSECUTIVE_FAILURES_BEFORE_ALERT,
    ),
    requestTimeoutMs: parsePositiveInt(
      "REQUEST_TIMEOUT_MS",
      optionalEnv(env, "REQUEST_TIMEOUT_MS"),
      DEFAULT_REQUEST_TIMEOUT_MS,
    ),
    initialLedgerLookback: parsePositiveInt(
      "INITIAL_LEDGER_LOOKBACK",
      optionalEnv(env, "INITIAL_LEDGER_LOOKBACK"),
      DEFAULT_INITIAL_LEDGER_LOOKBACK,
    ),
    alertMaxConcurrency: parsePositiveInt(
      "ALERT_MAX_CONCURRENCY",
      optionalEnv(env, "ALERT_MAX_CONCURRENCY"),
      DEFAULT_ALERT_MAX_CONCURRENCY,
    ),
  };
}
