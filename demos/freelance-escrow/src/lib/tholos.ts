import {
  Address,
  BASE_FEE,
  Contract,
  TransactionBuilder,
  nativeToScVal,
  scValToNative,
  rpc,
} from "@stellar/stellar-sdk";
import { signTransaction } from "@stellar/freighter-api";
import { NETWORK_PASSPHRASE, RPC_URL, THOLOS_CONTRACT_ID } from "./config";

export type AssertionStatus = "Pending" | "Disputed" | "Resolved";

export interface AssertionState {
  asserter: string;
  outcome: boolean;
  finalOutcome: boolean | null;
  bond: bigint;
  openedAt: bigint;
  status: AssertionStatus;
  disputer: string | null;
  votesForOutcome: number;
  votesAgainstOutcome: number;
  resolvers: string[];
  finalizer: string | null;
}

function server(): rpc.Server {
  return new rpc.Server(RPC_URL);
}

function contract(): Contract {
  return new Contract(THOLOS_CONTRACT_ID);
}

function addrArg(value: string) {
  return nativeToScVal(Address.fromString(value), { type: "address" });
}

/** Builds, signs (via Freighter), submits, and awaits one contract call. */
async function submit(method: string, args: ReturnType<typeof addrArg>[], sourcePublicKey: string) {
  const rpcServer = server();
  const account = await rpcServer.getAccount(sourcePublicKey);
  const tx = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(contract().call(method, ...args))
    .setTimeout(30)
    .build();

  const prepared = await rpcServer.prepareTransaction(tx);

  const { signedTxXdr, error } = await signTransaction(prepared.toXDR(), {
    networkPassphrase: NETWORK_PASSPHRASE,
  });
  if (error || !signedTxXdr) {
    throw new Error(`Wallet declined to sign the ${method} transaction.`);
  }

  const signedTx = TransactionBuilder.fromXDR(signedTxXdr, NETWORK_PASSPHRASE);
  const sent = await rpcServer.sendTransaction(signedTx);
  if (sent.status === "ERROR") {
    throw new Error(`${method} was rejected before submission.`);
  }

  return pollForResult(rpcServer, sent.hash, method);
}

async function pollForResult(rpcServer: rpc.Server, hash: string, method: string) {
  for (let attempt = 0; attempt < 20; attempt++) {
    const result = await rpcServer.getTransaction(hash);
    if (result.status === rpc.Api.GetTransactionStatus.SUCCESS) {
      return result.resultMetaXdr ? scValToNative(result.returnValue!) : undefined;
    }
    if (result.status === rpc.Api.GetTransactionStatus.FAILED) {
      throw new Error(`${method} failed on-chain (tx ${hash}).`);
    }
    await new Promise((resolve) => setTimeout(resolve, 1500));
  }
  throw new Error(`Timed out waiting for ${method} (tx ${hash}) to confirm.`);
}

export async function assertOutcome(asserter: string, outcome: boolean): Promise<bigint> {
  const id = await submit("assert_outcome", [addrArg(asserter), nativeToScVal(outcome)], asserter);
  return BigInt(id);
}

export async function disputeAssertion(disputer: string, id: bigint): Promise<void> {
  await submit("dispute", [addrArg(disputer), nativeToScVal(id, { type: "u64" })], disputer);
}

export async function resolveAssertion(
  resolver: string,
  id: bigint,
  agreesWithAsserter: boolean,
): Promise<boolean | null> {
  const result = await submit(
    "resolve",
    [addrArg(resolver), nativeToScVal(id, { type: "u64" }), nativeToScVal(agreesWithAsserter)],
    resolver,
  );
  return result ?? null;
}

export async function finalizeAssertion(caller: string, id: bigint): Promise<boolean> {
  return submit("finalize", [addrArg(caller), nativeToScVal(id, { type: "u64" })], caller);
}

/**
 * Read-only lookup. Simulation still needs a funded source account to build
 * against, so this piggybacks on whichever wallet address is currently
 * connected rather than requiring a second, app-owned account.
 */
export async function getAssertionState(id: bigint, readAs: string): Promise<AssertionState> {
  const rpcServer = server();
  const account = await rpcServer.getAccount(readAs);
  const tx = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(contract().call("get_assertion_state", nativeToScVal(id, { type: "u64" })))
    .setTimeout(30)
    .build();

  const sim = await rpcServer.simulateTransaction(tx);
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(sim.error);
  }
  if (!rpc.Api.isSimulationSuccess(sim) || !sim.result) {
    throw new Error(`Simulation for get_assertion_state(${id}) didn't return a value.`);
  }

  return mapAssertion(scValToNative(sim.result.retval));
}

function mapAssertion(raw: Record<string, unknown>): AssertionState {
  return {
    asserter: String(raw.asserter),
    outcome: Boolean(raw.outcome),
    finalOutcome: raw.final_outcome == null ? null : Boolean(raw.final_outcome),
    bond: BigInt(raw.bond as string | number | bigint),
    openedAt: BigInt(raw.opened_at as string | number | bigint),
    status: statusFromRaw(raw.status),
    disputer: raw.disputer == null ? null : String(raw.disputer),
    votesForOutcome: Number(raw.votes_for_outcome),
    votesAgainstOutcome: Number(raw.votes_against_outcome),
    resolvers: Array.isArray(raw.resolvers) ? raw.resolvers.map(String) : [],
    finalizer: raw.finalizer == null ? null : String(raw.finalizer),
  };
}

function statusFromRaw(status: unknown): AssertionStatus {
  // soroban-client decodes a Rust unit-variant enum to its tag string.
  const tag = typeof status === "string" ? status : Object.keys(status as object)[0];
  if (tag === "Pending" || tag === "Disputed" || tag === "Resolved") {
    return tag;
  }
  throw new Error(`Unrecognized assertion status: ${String(status)}`);
}
