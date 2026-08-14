import { Client, type Assertion } from "tholos-sdk";
import { signTransaction } from "@stellar/freighter-api";
import { NETWORK_PASSPHRASE, RPC_URL, THOLOS_CONTRACT_ID } from "./config";

export type { Assertion };

/**
 * One Client per call, not a shared module-level instance: `publicKey` (the
 * connected wallet, which is also the contract-argument identity in every
 * call this app makes) changes per caller, and `Client` bakes it in at
 * construction.
 */
function client(publicKey: string): Client {
  return new Client({
    contractId: THOLOS_CONTRACT_ID,
    networkPassphrase: NETWORK_PASSPHRASE,
    rpcUrl: RPC_URL,
    publicKey,
    signTransaction,
  });
}

export async function assertOutcome(asserter: string, outcome: boolean): Promise<bigint> {
  const tx = await client(asserter).assert_outcome({ asserter, outcome });
  const { result } = await tx.signAndSend();
  return result.unwrap();
}

export async function disputeAssertion(disputer: string, id: bigint): Promise<void> {
  const tx = await client(disputer).dispute({ disputer, id });
  const { result } = await tx.signAndSend();
  result.unwrap();
}

export async function resolveAssertion(
  resolver: string,
  id: bigint,
  agreesWithAsserter: boolean,
): Promise<boolean | null> {
  const tx = await client(resolver).resolve({
    resolver,
    id,
    agrees_with_asserter: agreesWithAsserter,
  });
  const { result } = await tx.signAndSend();
  return result.unwrap() ?? null;
}

export async function finalizeAssertion(caller: string, id: bigint): Promise<boolean> {
  const tx = await client(caller).finalize({ caller, id });
  const { result } = await tx.signAndSend();
  return result.unwrap();
}

/**
 * Read-only lookup. Simulation still needs a funded source account to build
 * against, so this piggybacks on whichever wallet address is currently
 * connected rather than requiring a second, app-owned account. No
 * `signAndSend()`: a read never needs a signature, so the already-simulated
 * `result` on the constructed transaction is the answer.
 */
export async function getAssertionState(id: bigint, readAs: string): Promise<Assertion> {
  const tx = await client(readAs).get_assertion_state({ id });
  return tx.result.unwrap();
}
