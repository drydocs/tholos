import {
  isConnected,
  isAllowed,
  requestAccess,
  getAddress,
  getNetwork,
} from "@stellar/freighter-api";

export type WalletState =
  | { status: "unavailable" }
  | { status: "disconnected" }
  | { status: "connected"; address: string; network: string };

/** Freighter must be installed and unlocked before any other call succeeds. */
export async function detectWallet(): Promise<WalletState> {
  const connected = await isConnected();
  if (connected.error || !connected.isConnected) {
    return { status: "unavailable" };
  }

  const allowed = await isAllowed();
  if (allowed.error || !allowed.isAllowed) {
    return { status: "disconnected" };
  }

  return resolveConnectedState();
}

export async function connectWallet(): Promise<WalletState> {
  const access = await requestAccess();
  if (access.error) {
    return { status: "disconnected" };
  }
  return resolveConnectedState();
}

async function resolveConnectedState(): Promise<WalletState> {
  const [addressResult, networkResult] = await Promise.all([
    getAddress(),
    getNetwork(),
  ]);

  if (addressResult.error || !addressResult.address) {
    return { status: "disconnected" };
  }

  return {
    status: "connected",
    address: addressResult.address,
    network: networkResult.network ?? "UNKNOWN",
  };
}

export function shortenAddress(address: string): string {
  return `${address.slice(0, 4)}...${address.slice(-4)}`;
}
