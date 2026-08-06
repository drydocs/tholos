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
