/**
 * Every value here is env-overridable so this app can be pointed at a
 * different Tholos deployment (a future mainnet instance, for example)
 * without a code change. The defaults point at the deployed Tholos testnet
 * instance, so the app works out of the box on a fresh clone.
 */
export const RPC_URL = import.meta.env.VITE_SOROBAN_RPC_URL ?? "https://soroban-testnet.stellar.org";
export const NETWORK_PASSPHRASE =
  import.meta.env.VITE_NETWORK_PASSPHRASE ?? "Test SDF Network ; September 2015";
export const THOLOS_CONTRACT_ID: string =
  import.meta.env.VITE_THOLOS_CONTRACT_ID ?? "CD46FHEWSQNIFHCVYJXBIX67HTTJE4S5RDWJTJQ7FJOJM4SFNKP6VRJW";
