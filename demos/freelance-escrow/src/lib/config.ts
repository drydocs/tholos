/**
 * Every value here is env-configured, the same way DEPLOYMENT.md's own deploy
 * walkthrough and scripts/testnet-smoke.sh treat a deployed contract id: never
 * committed to source, always supplied at build/run time. See README.md for
 * how to set VITE_THOLOS_CONTRACT_ID.
 */
export const RPC_URL = import.meta.env.VITE_SOROBAN_RPC_URL ?? "https://soroban-testnet.stellar.org";
export const NETWORK_PASSPHRASE =
  import.meta.env.VITE_NETWORK_PASSPHRASE ?? "Test SDF Network ; September 2015";
export const THOLOS_CONTRACT_ID: string = import.meta.env.VITE_THOLOS_CONTRACT_ID ?? "";

/**
 * `challenge_window_secs` as configured on the deployed contract instance.
 * The contract has no public getter for this (it's a deploy-time parameter,
 * see docs/src/DEPLOYMENT.md), so it's mirrored here the same way the
 * contract id itself is: env-configurable, defaulting to the canonical
 * testnet deployment's value (21600s / 6h). Used only to derive a
 * client-side "review window has likely closed" hint from a real
 * `Assertion.opened_at` read; it never gates the `finalize` call itself —
 * the contract remains the source of truth and rejects it if called early.
 */
const DEFAULT_CHALLENGE_WINDOW_SECS = 21600;

function parseChallengeWindowSecs(): number {
  const raw = import.meta.env.VITE_CHALLENGE_WINDOW_SECS;
  if (!raw) {
    return DEFAULT_CHALLENGE_WINDOW_SECS;
  }
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 0) {
    console.warn(
      `Invalid VITE_CHALLENGE_WINDOW_SECS "${raw}"; falling back to default (${DEFAULT_CHALLENGE_WINDOW_SECS}s).`,
    );
    return DEFAULT_CHALLENGE_WINDOW_SECS;
  }
  return parsed;
}

export const CHALLENGE_WINDOW_SECS: number = parseChallengeWindowSecs();
