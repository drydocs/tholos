import { Buffer } from "buffer";
import { Address } from "@stellar/stellar-sdk";
import {
  AssembledTransaction,
  Client as ContractClient,
  ClientOptions as ContractClientOptions,
  MethodOptions,
  Result,
  Spec as ContractSpec,
} from "@stellar/stellar-sdk/contract";
import type {
  u32,
  i32,
  u64,
  i64,
  u128,
  i128,
  u256,
  i256,
  Option,
  Timepoint,
  Duration,
} from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}




export const Errors = {
  1: {message:"AlreadyInitialized"},
  2: {message:"NotInitialized"},
  3: {message:"InvalidResolverCount"},
  4: {message:"AssertionNotFound"},
  5: {message:"NotPending"},
  6: {message:"NotDisputed"},
  7: {message:"ChallengeWindowClosed"},
  8: {message:"ChallengeWindowOpen"},
  9: {message:"NotAResolver"},
  10: {message:"AlreadyVoted"},
  11: {message:"Paused"},
  /**
   * `bond_amount` was not positive, or exceeded `MAX_BOND_AMOUNT`.
   */
  12: {message:"InvalidBondAmount"},
  13: {message:"InvalidChallengeWindow"},
  14: {message:"TooManyResolvers"},
  /**
   * `finalize_reward_bps` was greater than `MAX_FINALIZE_REWARD_BPS` (1000).
   */
  15: {message:"InvalidFinalizeReward"},
  16: {message:"DuplicateResolvers"},
  17: {message:"RotationInProgress"},
  18: {message:"NoRotationProposal"},
  19: {message:"ResolverNotInCommittee"},
  20: {message:"RotationTargetAlreadyResolver"},
  21: {message:"NotProposer"},
  /**
   * The caller is the asserter of the assertion they are trying to dispute.
   * An asserter disputing their own assertion would consume the one dispute
   * slot without any economic risk (they receive both bonds back regardless
   * of the resolver vote), nullifying the bond-forfeiture deterrent.
   */
  22: {message:"SelfDispute"}
}

export type Status = {tag: "Pending", values: void} | {tag: "Disputed", values: void} | {tag: "Resolved", values: void};

export type DataKey = {tag: "Admin", values: void} | {tag: "Token", values: void} | {tag: "BondAmount", values: void} | {tag: "ChallengeWindow", values: void} | {tag: "Resolvers", values: void} | {tag: "Assertion", values: readonly [u64]} | {tag: "NextId", values: void} | {tag: "Paused", values: void} | {tag: "FinalizeRewardBps", values: void} | {tag: "RotationProposal", values: void};





export interface Assertion {
  asserter: string;
  /**
 * The bond amount required to dispute this assertion and the amount
 * paid out to the winning side. Pinned to the live `DataKey::BondAmount`
 * at the moment `assert_outcome` created this assertion; a later
 * `set_bond_amount` call never changes it retroactively. Every payout
 * path (`dispute`, `finalize`, `resolve`) reads this field, never the
 * live `DataKey::BondAmount`, so this guarantee holds structurally.
 */
bond: i128;
  disputer: Option<string>;
  /**
 * The authoritative outcome once the assertion is resolved. `None` while
 * the assertion is still pending or disputed.
 */
final_outcome: Option<boolean>;
  /**
 * Who called `finalize`. `None` until the assertion is finalized via
 * `finalize` (never set for assertions resolved via `resolve`). Always
 * `Some` after `finalize` completes — the caller must authorize the call
 * unconditionally, so this is always a verified address.
 */
finalizer: Option<string>;
  opened_at: u64;
  outcome: boolean;
  /**
 * The resolver committee at the moment this assertion was disputed.
 * Empty until `dispute` is called. Voting and majority are always
 * computed against this snapshot, not the live committee, so an
 * `update_resolvers` call mid-dispute can't change who gets to decide
 * an already-disputed assertion.
 */
resolvers: Array<string>;
  status: Status;
  voted: Array<string>;
  votes_against_outcome: u32;
  votes_for_outcome: u32;
}




/**
 * An in-flight single-slot committee rotation proposed by a current resolver.
 * Decided by a strict majority of the live committee via `vote_rotation`. Only
 * one may be open at a time. See `docs/src/ROTATION_DESIGN.md`.
 */
export interface RotationProposal {
  /**
 * The new resolver to add. Must not already be on the committee.
 */
new_resolver: string;
  /**
 * Resolvers who voted no, to prevent double-voting and detect deadlock.
 */
no: Array<string>;
  /**
 * The current resolver to remove. Must be on the committee when proposed.
 */
old_resolver: string;
  /**
 * The resolver who opened the proposal.
 */
proposed_by: string;
  /**
 * Resolvers who voted yes, to prevent double-voting.
 */
yes: Array<string>;
}






export interface Client {
  /**
   * Construct and simulate a dispute transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Disputes a pending assertion within the challenge window by matching its bond.
   */
  dispute: ({disputer, id}: {disputer: string, id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a resolve transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * A resolver votes on a disputed assertion. Once a strict majority of
   * the resolver committee agrees, the assertion finalizes: the winning
   * side (asserter if the original outcome stands, disputer otherwise)
   * receives both bonds.
   */
  resolve: ({resolver, id, agrees_with_asserter}: {resolver: string, id: u64, agrees_with_asserter: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Option<boolean>>>>

  /**
   * Construct and simulate a finalize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Finalizes a pending assertion once its challenge window has elapsed
   * with no dispute. Fails with `Paused` if paused: a paused assertion may
   * have had no real opportunity to be disputed during its challenge
   * window (since `dispute` is also blocked while paused), so it must not
   * be able to finalize uncontested until unpaused. `caller` must
   * authorize the call unconditionally — regardless of whether
   * `finalize_reward_bps` is zero — so the address recorded in
   * `Assertion.finalizer` and the `Finalized` event is always a verified
   * caller and cannot be spoofed. When `finalize_reward_bps` is non-zero,
   * `caller` also receives `bond * finalize_reward_bps / 10_000` tokens as
   * an incentive for prompt finalization and the asserter receives the
   * remainder; when it is zero the full bond is returned to the asserter
   * and no reward is paid. Returns the asserted outcome.
   */
  finalize: ({caller, id}: {caller: string, id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<boolean>>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Initializes the contract. `resolvers` must have an odd length so a
   * simple majority vote can never tie. `finalize_reward_bps` sets the
   * fraction of the bond (in basis points, 0–1000) paid to whoever calls
   * `finalize` as an incentive for prompt finalization; 0 disables the
   * reward entirely and preserves the original behavior where the full
   * bond is returned to the asserter.
   */
  initialize: ({admin, token, bond_amount, challenge_window_secs, resolvers, finalize_reward_bps}: {admin: string, token: string, bond_amount: i128, challenge_window_secs: u64, resolvers: Array<string>, finalize_reward_bps: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a set_paused transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pauses or unpauses new assertions, disputes, resolver votes, and
   * finalization. A pending assertion may have had no real opportunity to
   * be disputed during its challenge window if that window overlapped a
   * pause, so `finalize` is blocked too rather than letting it finalize
   * uncontested; it becomes callable again once unpaused. Only callable by
   * the admin set at initialization.
   */
  set_paused: ({paused}: {paused: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a vote_rotation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * A resolver votes on the open rotation proposal. `approve` records a yes or
   * no (both prevent re-voting). Once yes-votes reach a strict majority of the
   * live committee, the rotation executes immediately: `old_resolver` is swapped
   * for `new_resolver` in the live committee, and the proposal is cleared.
   * If the remaining unvoted resolvers can no longer supply enough yes-votes to
   * reach a majority, the proposal is cancelled automatically (deadlock guard).
   * Returns `Some(true)` if the rotation executed, `Some(false)` if it was
   * auto-cancelled as dead, and `None` if the proposal remains open.
   */
  vote_rotation: ({resolver, approve}: {resolver: string, approve: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Option<boolean>>>>

  /**
   * Construct and simulate a assert_outcome transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Posts a bonded claim about an outcome. Returns the new assertion id.
   */
  assert_outcome: ({asserter, outcome}: {asserter: string, outcome: boolean}, options?: MethodOptions) => Promise<AssembledTransaction<Result<u64>>>

  /**
   * Construct and simulate a cancel_rotation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Cancels the open rotation proposal. The proposer may cancel at any time.
   * Any current resolver may also cancel once the proposal can no longer reach
   * a majority (deadlock guard), so a lost proposer key can't permanently
   * block rotation. Emits `RotationCancelled`.
   */
  cancel_rotation: ({resolver}: {resolver: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a set_bond_amount transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Updates the bond amount required for assertions created from this
   * point on. Only callable by the admin set at initialization, validated
   * against the same bounds `initialize` already enforces
   * (`new_bond_amount > 0`, `new_bond_amount <= MAX_BOND_AMOUNT`).
   * Pause-exempt, like `update_resolvers` and `set_paused`.
   * 
   * This only affects assertions created after the change: `Assertion.bond`
   * pins the bond amount at the moment `assert_outcome` creates the
   * assertion, and every payout path (`dispute`, `finalize`, `resolve`)
   * reads `assertion.bond`, never the live `DataKey::BondAmount`. An
   * already-open assertion's payout is therefore unaffected by a later
   * `set_bond_amount` call.
   * 
   * Fails with `NotInitialized` if called before `initialize`, or
   * `InvalidBondAmount` if `new_bond_amount` is zero, negative, or greater
   * than `MAX_BOND_AMOUNT`.
   */
  set_bond_amount: ({new_bond_amount}: {new_bond_amount: i128}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a propose_rotation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Proposes a single-slot committee rotation: remove `old_resolver` (must be
   * a current resolver) and add `new_resolver` (must not already be one). Only
   * a current resolver may propose, and only one rotation may be open at a
   * time. The proposal is decided by a strict majority of the live committee
   * (the same threshold used to resolve disputes) via `vote_rotation`. The
   * committee written on execution is the same `Resolvers` slot `update_resolvers`
   * writes, so a rotation has no effect on disputes already open (their
   * committee was snapshotted at `dispute` time). Pause-exempt, like
   * `update_resolvers`.
   */
  propose_rotation: ({resolver, old_resolver, new_resolver}: {resolver: string, old_resolver: string, new_resolver: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a update_resolvers transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Replaces the resolver committee. Only callable by the admin set at
   * initialization. `new_resolvers` must have an odd length so a simple
   * majority vote can never tie. Callable even while paused, so a
   * compromised committee can be replaced without waiting to unpause.
   * 
   * This is the emergency override path. It supersedes any in-flight
   * self-rotation vote: an open `RotationProposal` is cleared (emitting
   * `RotationCancelled` when one was present), so a proposal can never
   * execute against a committee it wasn't built for. Day-to-day committee
   * changes go through `propose_rotation` / `vote_rotation` instead.
   */
  update_resolvers: ({new_resolvers}: {new_resolvers: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_assertion_state transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   */
  get_assertion_state: ({id}: {id: u64}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Assertion>>>

}
export class Client extends ContractClient {
  static async deploy<T = Client>(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions &
      Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
      }
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy(null, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAFgAAAAAAAAASQWxyZWFkeUluaXRpYWxpemVkAAAAAAABAAAAAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAUSW52YWxpZFJlc29sdmVyQ291bnQAAAADAAAAAAAAABFBc3NlcnRpb25Ob3RGb3VuZAAAAAAAAAQAAAAAAAAACk5vdFBlbmRpbmcAAAAAAAUAAAAAAAAAC05vdERpc3B1dGVkAAAAAAYAAAAAAAAAFUNoYWxsZW5nZVdpbmRvd0Nsb3NlZAAAAAAAAAcAAAAAAAAAE0NoYWxsZW5nZVdpbmRvd09wZW4AAAAACAAAAAAAAAAMTm90QVJlc29sdmVyAAAACQAAAAAAAAAMQWxyZWFkeVZvdGVkAAAACgAAAAAAAAAGUGF1c2VkAAAAAAALAAAAPmBib25kX2Ftb3VudGAgd2FzIG5vdCBwb3NpdGl2ZSwgb3IgZXhjZWVkZWQgYE1BWF9CT05EX0FNT1VOVGAuAAAAAAARSW52YWxpZEJvbmRBbW91bnQAAAAAAAAMAAAAAAAAABZJbnZhbGlkQ2hhbGxlbmdlV2luZG93AAAAAAANAAAAAAAAABBUb29NYW55UmVzb2x2ZXJzAAAADgAAAEhgZmluYWxpemVfcmV3YXJkX2Jwc2Agd2FzIGdyZWF0ZXIgdGhhbiBgTUFYX0ZJTkFMSVpFX1JFV0FSRF9CUFNgICgxMDAwKS4AAAAVSW52YWxpZEZpbmFsaXplUmV3YXJkAAAAAAAADwAAAAAAAAASRHVwbGljYXRlUmVzb2x2ZXJzAAAAAAAQAAAAAAAAABJSb3RhdGlvbkluUHJvZ3Jlc3MAAAAAABEAAAAAAAAAEk5vUm90YXRpb25Qcm9wb3NhbAAAAAAAEgAAAAAAAAAWUmVzb2x2ZXJOb3RJbkNvbW1pdHRlZQAAAAAAEwAAAAAAAAAdUm90YXRpb25UYXJnZXRBbHJlYWR5UmVzb2x2ZXIAAAAAAAAUAAAAAAAAAAtOb3RQcm9wb3NlcgAAAAAVAAABGFRoZSBjYWxsZXIgaXMgdGhlIGFzc2VydGVyIG9mIHRoZSBhc3NlcnRpb24gdGhleSBhcmUgdHJ5aW5nIHRvIGRpc3B1dGUuCkFuIGFzc2VydGVyIGRpc3B1dGluZyB0aGVpciBvd24gYXNzZXJ0aW9uIHdvdWxkIGNvbnN1bWUgdGhlIG9uZSBkaXNwdXRlCnNsb3Qgd2l0aG91dCBhbnkgZWNvbm9taWMgcmlzayAodGhleSByZWNlaXZlIGJvdGggYm9uZHMgYmFjayByZWdhcmRsZXNzCm9mIHRoZSByZXNvbHZlciB2b3RlKSwgbnVsbGlmeWluZyB0aGUgYm9uZC1mb3JmZWl0dXJlIGRldGVycmVudC4AAAALU2VsZkRpc3B1dGUAAAAAFg==",
        "AAAAAgAAAAAAAAAAAAAABlN0YXR1cwAAAAAAAwAAAAAAAAAAAAAAB1BlbmRpbmcAAAAAAAAAAAAAAAAIRGlzcHV0ZWQAAAAAAAAAAAAAAAhSZXNvbHZlZA==",
        "AAAAAgAAAAAAAAAAAAAAB0RhdGFLZXkAAAAACgAAAAAAAAAAAAAABUFkbWluAAAAAAAAAAAAAAAAAAAFVG9rZW4AAAAAAAAAAAAAAAAAAApCb25kQW1vdW50AAAAAAAAAAAAAAAAAA9DaGFsbGVuZ2VXaW5kb3cAAAAAAAAAAAAAAAAJUmVzb2x2ZXJzAAAAAAAAAQAAAAAAAAAJQXNzZXJ0aW9uAAAAAAAAAQAAAAYAAAAAAAAAAAAAAAZOZXh0SWQAAAAAAAAAAAAAAAAABlBhdXNlZAAAAAAAAAAAAMhCYXNpcyBwb2ludHMgKDDigJMxMDAwKSBvZiB0aGUgYm9uZCBwYWlkIHRvIHdob2V2ZXIgY2FsbHMgYGZpbmFsaXplYCBhcwphbiBpbmNlbnRpdmUgZm9yIHByb21wdCBmaW5hbGl6YXRpb24uIDAgbWVhbnMgbm8gcmV3YXJkIGlzIHRha2VuOyB0aGUKZnVsbCBib25kIGlzIHJldHVybmVkIHRvIHRoZSBhc3NlcnRlciAob3JpZ2luYWwgYmVoYXZpb3IpLgAAABFGaW5hbGl6ZVJld2FyZEJwcwAAAAAAAAAAAAAAAAAAEFJvdGF0aW9uUHJvcG9zYWw=",
        "AAAAAAAAAE5EaXNwdXRlcyBhIHBlbmRpbmcgYXNzZXJ0aW9uIHdpdGhpbiB0aGUgY2hhbGxlbmdlIHdpbmRvdyBieSBtYXRjaGluZyBpdHMgYm9uZC4AAAAAAAdkaXNwdXRlAAAAAAIAAAAAAAAACGRpc3B1dGVyAAAAEwAAAAAAAAACaWQAAAAAAAYAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAN9BIHJlc29sdmVyIHZvdGVzIG9uIGEgZGlzcHV0ZWQgYXNzZXJ0aW9uLiBPbmNlIGEgc3RyaWN0IG1ham9yaXR5IG9mCnRoZSByZXNvbHZlciBjb21taXR0ZWUgYWdyZWVzLCB0aGUgYXNzZXJ0aW9uIGZpbmFsaXplczogdGhlIHdpbm5pbmcKc2lkZSAoYXNzZXJ0ZXIgaWYgdGhlIG9yaWdpbmFsIG91dGNvbWUgc3RhbmRzLCBkaXNwdXRlciBvdGhlcndpc2UpCnJlY2VpdmVzIGJvdGggYm9uZHMuAAAAAAdyZXNvbHZlAAAAAAMAAAAAAAAACHJlc29sdmVyAAAAEwAAAAAAAAACaWQAAAAAAAYAAAAAAAAAFGFncmVlc193aXRoX2Fzc2VydGVyAAAAAQAAAAEAAAPpAAAD6AAAAAEAAAAD",
        "AAAAAAAAA1hGaW5hbGl6ZXMgYSBwZW5kaW5nIGFzc2VydGlvbiBvbmNlIGl0cyBjaGFsbGVuZ2Ugd2luZG93IGhhcyBlbGFwc2VkCndpdGggbm8gZGlzcHV0ZS4gRmFpbHMgd2l0aCBgUGF1c2VkYCBpZiBwYXVzZWQ6IGEgcGF1c2VkIGFzc2VydGlvbiBtYXkKaGF2ZSBoYWQgbm8gcmVhbCBvcHBvcnR1bml0eSB0byBiZSBkaXNwdXRlZCBkdXJpbmcgaXRzIGNoYWxsZW5nZQp3aW5kb3cgKHNpbmNlIGBkaXNwdXRlYCBpcyBhbHNvIGJsb2NrZWQgd2hpbGUgcGF1c2VkKSwgc28gaXQgbXVzdCBub3QKYmUgYWJsZSB0byBmaW5hbGl6ZSB1bmNvbnRlc3RlZCB1bnRpbCB1bnBhdXNlZC4gYGNhbGxlcmAgbXVzdAphdXRob3JpemUgdGhlIGNhbGwgdW5jb25kaXRpb25hbGx5IOKAlCByZWdhcmRsZXNzIG9mIHdoZXRoZXIKYGZpbmFsaXplX3Jld2FyZF9icHNgIGlzIHplcm8g4oCUIHNvIHRoZSBhZGRyZXNzIHJlY29yZGVkIGluCmBBc3NlcnRpb24uZmluYWxpemVyYCBhbmQgdGhlIGBGaW5hbGl6ZWRgIGV2ZW50IGlzIGFsd2F5cyBhIHZlcmlmaWVkCmNhbGxlciBhbmQgY2Fubm90IGJlIHNwb29mZWQuIFdoZW4gYGZpbmFsaXplX3Jld2FyZF9icHNgIGlzIG5vbi16ZXJvLApgY2FsbGVyYCBhbHNvIHJlY2VpdmVzIGBib25kICogZmluYWxpemVfcmV3YXJkX2JwcyAvIDEwXzAwMGAgdG9rZW5zIGFzCmFuIGluY2VudGl2ZSBmb3IgcHJvbXB0IGZpbmFsaXphdGlvbiBhbmQgdGhlIGFzc2VydGVyIHJlY2VpdmVzIHRoZQpyZW1haW5kZXI7IHdoZW4gaXQgaXMgemVybyB0aGUgZnVsbCBib25kIGlzIHJldHVybmVkIHRvIHRoZSBhc3NlcnRlcgphbmQgbm8gcmV3YXJkIGlzIHBhaWQuIFJldHVybnMgdGhlIGFzc2VydGVkIG91dGNvbWUuAAAACGZpbmFsaXplAAAAAgAAAAAAAAAGY2FsbGVyAAAAAAATAAAAAAAAAAJpZAAAAAAABgAAAAEAAAPpAAAAAQAAAAM=",
        "AAAABQAAAAAAAAAAAAAACEFzc2VydGVkAAAAAQAAAAhhc3NlcnRlZAAAAAMAAAAAAAAAAmlkAAAAAAAGAAAAAQAAAAAAAAAIYXNzZXJ0ZXIAAAATAAAAAAAAAAAAAAAHb3V0Y29tZQAAAAABAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAACERpc3B1dGVkAAAAAQAAAAhkaXNwdXRlZAAAAAIAAAAAAAAAAmlkAAAAAAAGAAAAAQAAAAAAAAAIZGlzcHV0ZXIAAAATAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAACFJlc29sdmVkAAAAAQAAAAhyZXNvbHZlZAAAAAIAAAAAAAAAAmlkAAAAAAAGAAAAAQAAAAAAAAAHb3V0Y29tZQAAAAABAAAAAAAAAAI=",
        "AAAAAQAAAAAAAAAAAAAACUFzc2VydGlvbgAAAAAAAAwAAAAAAAAACGFzc2VydGVyAAAAEwAAAZFUaGUgYm9uZCBhbW91bnQgcmVxdWlyZWQgdG8gZGlzcHV0ZSB0aGlzIGFzc2VydGlvbiBhbmQgdGhlIGFtb3VudApwYWlkIG91dCB0byB0aGUgd2lubmluZyBzaWRlLiBQaW5uZWQgdG8gdGhlIGxpdmUgYERhdGFLZXk6OkJvbmRBbW91bnRgCmF0IHRoZSBtb21lbnQgYGFzc2VydF9vdXRjb21lYCBjcmVhdGVkIHRoaXMgYXNzZXJ0aW9uOyBhIGxhdGVyCmBzZXRfYm9uZF9hbW91bnRgIGNhbGwgbmV2ZXIgY2hhbmdlcyBpdCByZXRyb2FjdGl2ZWx5LiBFdmVyeSBwYXlvdXQKcGF0aCAoYGRpc3B1dGVgLCBgZmluYWxpemVgLCBgcmVzb2x2ZWApIHJlYWRzIHRoaXMgZmllbGQsIG5ldmVyIHRoZQpsaXZlIGBEYXRhS2V5OjpCb25kQW1vdW50YCwgc28gdGhpcyBndWFyYW50ZWUgaG9sZHMgc3RydWN0dXJhbGx5LgAAAAAAAARib25kAAAACwAAAAAAAAAIZGlzcHV0ZXIAAAPoAAAAEwAAAHJUaGUgYXV0aG9yaXRhdGl2ZSBvdXRjb21lIG9uY2UgdGhlIGFzc2VydGlvbiBpcyByZXNvbHZlZC4gYE5vbmVgIHdoaWxlCnRoZSBhc3NlcnRpb24gaXMgc3RpbGwgcGVuZGluZyBvciBkaXNwdXRlZC4AAAAAAA1maW5hbF9vdXRjb21lAAAAAAAD6AAAAAEAAAEHV2hvIGNhbGxlZCBgZmluYWxpemVgLiBgTm9uZWAgdW50aWwgdGhlIGFzc2VydGlvbiBpcyBmaW5hbGl6ZWQgdmlhCmBmaW5hbGl6ZWAgKG5ldmVyIHNldCBmb3IgYXNzZXJ0aW9ucyByZXNvbHZlZCB2aWEgYHJlc29sdmVgKS4gQWx3YXlzCmBTb21lYCBhZnRlciBgZmluYWxpemVgIGNvbXBsZXRlcyDigJQgdGhlIGNhbGxlciBtdXN0IGF1dGhvcml6ZSB0aGUgY2FsbAp1bmNvbmRpdGlvbmFsbHksIHNvIHRoaXMgaXMgYWx3YXlzIGEgdmVyaWZpZWQgYWRkcmVzcy4AAAAACWZpbmFsaXplcgAAAAAAA+gAAAATAAAAAAAAAAlvcGVuZWRfYXQAAAAAAAAGAAAAAAAAAAdvdXRjb21lAAAAAAEAAAEiVGhlIHJlc29sdmVyIGNvbW1pdHRlZSBhdCB0aGUgbW9tZW50IHRoaXMgYXNzZXJ0aW9uIHdhcyBkaXNwdXRlZC4KRW1wdHkgdW50aWwgYGRpc3B1dGVgIGlzIGNhbGxlZC4gVm90aW5nIGFuZCBtYWpvcml0eSBhcmUgYWx3YXlzCmNvbXB1dGVkIGFnYWluc3QgdGhpcyBzbmFwc2hvdCwgbm90IHRoZSBsaXZlIGNvbW1pdHRlZSwgc28gYW4KYHVwZGF0ZV9yZXNvbHZlcnNgIGNhbGwgbWlkLWRpc3B1dGUgY2FuJ3QgY2hhbmdlIHdobyBnZXRzIHRvIGRlY2lkZQphbiBhbHJlYWR5LWRpc3B1dGVkIGFzc2VydGlvbi4AAAAAAAlyZXNvbHZlcnMAAAAAAAPqAAAAEwAAAAAAAAAGc3RhdHVzAAAAAAfQAAAABlN0YXR1cwAAAAAAAAAAAAV2b3RlZAAAAAAAA+oAAAATAAAAAAAAABV2b3Rlc19hZ2FpbnN0X291dGNvbWUAAAAAAAAEAAAAAAAAABF2b3Rlc19mb3Jfb3V0Y29tZQAAAAAAAAQ=",
        "AAAABQAAAAAAAAAAAAAACUZpbmFsaXplZAAAAAAAAAEAAAAJZmluYWxpemVkAAAAAAAABAAAAAAAAAACaWQAAAAAAAYAAAABAAAAAAAAAAdvdXRjb21lAAAAAAEAAAAAAAAAt1dobyBjYWxsZWQgYGZpbmFsaXplYC4gQWx3YXlzIGEgdmVyaWZpZWQgYWRkcmVzcyDigJQgYGZpbmFsaXplYCByZXF1aXJlcwp0aGUgY2FsbGVyJ3MgYXV0aCB1bmNvbmRpdGlvbmFsbHksIHNvIHRoaXMgdmFsdWUgaXMgdHJ1c3R3b3J0aHkKcmVnYXJkbGVzcyBvZiB3aGV0aGVyIGEgcmV3YXJkIHdhcyBjb25maWd1cmVkLgAAAAAJZmluYWxpemVyAAAAAAAAEwAAAAAAAABqSG93IG1hbnkgdG9rZW5zIHdlcmUgcGFpZCB0byB0aGUgZmluYWxpemVyIGFzIGEgcmV3YXJkICgwIHdoZW4KYGZpbmFsaXplX3Jld2FyZF9icHNgIHdhcyBjb25maWd1cmVkIGFzIDApLgAAAAAABnJld2FyZAAAAAAACwAAAAAAAAAC",
        "AAAAAAAAAXRJbml0aWFsaXplcyB0aGUgY29udHJhY3QuIGByZXNvbHZlcnNgIG11c3QgaGF2ZSBhbiBvZGQgbGVuZ3RoIHNvIGEKc2ltcGxlIG1ham9yaXR5IHZvdGUgY2FuIG5ldmVyIHRpZS4gYGZpbmFsaXplX3Jld2FyZF9icHNgIHNldHMgdGhlCmZyYWN0aW9uIG9mIHRoZSBib25kIChpbiBiYXNpcyBwb2ludHMsIDDigJMxMDAwKSBwYWlkIHRvIHdob2V2ZXIgY2FsbHMKYGZpbmFsaXplYCBhcyBhbiBpbmNlbnRpdmUgZm9yIHByb21wdCBmaW5hbGl6YXRpb247IDAgZGlzYWJsZXMgdGhlCnJld2FyZCBlbnRpcmVseSBhbmQgcHJlc2VydmVzIHRoZSBvcmlnaW5hbCBiZWhhdmlvciB3aGVyZSB0aGUgZnVsbApib25kIGlzIHJldHVybmVkIHRvIHRoZSBhc3NlcnRlci4AAAAKaW5pdGlhbGl6ZQAAAAAABgAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAAAV0b2tlbgAAAAAAABMAAAAAAAAAC2JvbmRfYW1vdW50AAAAAAsAAAAAAAAAFWNoYWxsZW5nZV93aW5kb3dfc2VjcwAAAAAAAAYAAAAAAAAACXJlc29sdmVycwAAAAAAA+oAAAATAAAAAAAAABNmaW5hbGl6ZV9yZXdhcmRfYnBzAAAAAAQAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAXZQYXVzZXMgb3IgdW5wYXVzZXMgbmV3IGFzc2VydGlvbnMsIGRpc3B1dGVzLCByZXNvbHZlciB2b3RlcywgYW5kCmZpbmFsaXphdGlvbi4gQSBwZW5kaW5nIGFzc2VydGlvbiBtYXkgaGF2ZSBoYWQgbm8gcmVhbCBvcHBvcnR1bml0eSB0bwpiZSBkaXNwdXRlZCBkdXJpbmcgaXRzIGNoYWxsZW5nZSB3aW5kb3cgaWYgdGhhdCB3aW5kb3cgb3ZlcmxhcHBlZCBhCnBhdXNlLCBzbyBgZmluYWxpemVgIGlzIGJsb2NrZWQgdG9vIHJhdGhlciB0aGFuIGxldHRpbmcgaXQgZmluYWxpemUKdW5jb250ZXN0ZWQ7IGl0IGJlY29tZXMgY2FsbGFibGUgYWdhaW4gb25jZSB1bnBhdXNlZC4gT25seSBjYWxsYWJsZSBieQp0aGUgYWRtaW4gc2V0IGF0IGluaXRpYWxpemF0aW9uLgAAAAAACnNldF9wYXVzZWQAAAAAAAEAAAAAAAAABnBhdXNlZAAAAAAAAQAAAAEAAAPpAAAAAgAAAAM=",
        "AAAABQAAAAAAAAAAAAAADFBhdXNlVXBkYXRlZAAAAAEAAAANcGF1c2VfdXBkYXRlZAAAAAAAAAEAAAAAAAAABnBhdXNlZAAAAAAAAQAAAAAAAAAC",
        "AAAAAAAAAklBIHJlc29sdmVyIHZvdGVzIG9uIHRoZSBvcGVuIHJvdGF0aW9uIHByb3Bvc2FsLiBgYXBwcm92ZWAgcmVjb3JkcyBhIHllcyBvcgpubyAoYm90aCBwcmV2ZW50IHJlLXZvdGluZykuIE9uY2UgeWVzLXZvdGVzIHJlYWNoIGEgc3RyaWN0IG1ham9yaXR5IG9mIHRoZQpsaXZlIGNvbW1pdHRlZSwgdGhlIHJvdGF0aW9uIGV4ZWN1dGVzIGltbWVkaWF0ZWx5OiBgb2xkX3Jlc29sdmVyYCBpcyBzd2FwcGVkCmZvciBgbmV3X3Jlc29sdmVyYCBpbiB0aGUgbGl2ZSBjb21taXR0ZWUsIGFuZCB0aGUgcHJvcG9zYWwgaXMgY2xlYXJlZC4KSWYgdGhlIHJlbWFpbmluZyB1bnZvdGVkIHJlc29sdmVycyBjYW4gbm8gbG9uZ2VyIHN1cHBseSBlbm91Z2ggeWVzLXZvdGVzIHRvCnJlYWNoIGEgbWFqb3JpdHksIHRoZSBwcm9wb3NhbCBpcyBjYW5jZWxsZWQgYXV0b21hdGljYWxseSAoZGVhZGxvY2sgZ3VhcmQpLgpSZXR1cm5zIGBTb21lKHRydWUpYCBpZiB0aGUgcm90YXRpb24gZXhlY3V0ZWQsIGBTb21lKGZhbHNlKWAgaWYgaXQgd2FzCmF1dG8tY2FuY2VsbGVkIGFzIGRlYWQsIGFuZCBgTm9uZWAgaWYgdGhlIHByb3Bvc2FsIHJlbWFpbnMgb3Blbi4AAAAAAAANdm90ZV9yb3RhdGlvbgAAAAAAAAIAAAAAAAAACHJlc29sdmVyAAAAEwAAAAAAAAAHYXBwcm92ZQAAAAABAAAAAQAAA+kAAAPoAAAAAQAAAAM=",
        "AAAAAAAAAERQb3N0cyBhIGJvbmRlZCBjbGFpbSBhYm91dCBhbiBvdXRjb21lLiBSZXR1cm5zIHRoZSBuZXcgYXNzZXJ0aW9uIGlkLgAAAA5hc3NlcnRfb3V0Y29tZQAAAAAAAgAAAAAAAAAIYXNzZXJ0ZXIAAAATAAAAAAAAAAdvdXRjb21lAAAAAAEAAAABAAAD6QAAAAYAAAAD",
        "AAAAAAAAAQRDYW5jZWxzIHRoZSBvcGVuIHJvdGF0aW9uIHByb3Bvc2FsLiBUaGUgcHJvcG9zZXIgbWF5IGNhbmNlbCBhdCBhbnkgdGltZS4KQW55IGN1cnJlbnQgcmVzb2x2ZXIgbWF5IGFsc28gY2FuY2VsIG9uY2UgdGhlIHByb3Bvc2FsIGNhbiBubyBsb25nZXIgcmVhY2gKYSBtYWpvcml0eSAoZGVhZGxvY2sgZ3VhcmQpLCBzbyBhIGxvc3QgcHJvcG9zZXIga2V5IGNhbid0IHBlcm1hbmVudGx5CmJsb2NrIHJvdGF0aW9uLiBFbWl0cyBgUm90YXRpb25DYW5jZWxsZWRgLgAAAA9jYW5jZWxfcm90YXRpb24AAAAAAQAAAAAAAAAIcmVzb2x2ZXIAAAATAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAztVcGRhdGVzIHRoZSBib25kIGFtb3VudCByZXF1aXJlZCBmb3IgYXNzZXJ0aW9ucyBjcmVhdGVkIGZyb20gdGhpcwpwb2ludCBvbi4gT25seSBjYWxsYWJsZSBieSB0aGUgYWRtaW4gc2V0IGF0IGluaXRpYWxpemF0aW9uLCB2YWxpZGF0ZWQKYWdhaW5zdCB0aGUgc2FtZSBib3VuZHMgYGluaXRpYWxpemVgIGFscmVhZHkgZW5mb3JjZXMKKGBuZXdfYm9uZF9hbW91bnQgPiAwYCwgYG5ld19ib25kX2Ftb3VudCA8PSBNQVhfQk9ORF9BTU9VTlRgKS4KUGF1c2UtZXhlbXB0LCBsaWtlIGB1cGRhdGVfcmVzb2x2ZXJzYCBhbmQgYHNldF9wYXVzZWRgLgoKVGhpcyBvbmx5IGFmZmVjdHMgYXNzZXJ0aW9ucyBjcmVhdGVkIGFmdGVyIHRoZSBjaGFuZ2U6IGBBc3NlcnRpb24uYm9uZGAKcGlucyB0aGUgYm9uZCBhbW91bnQgYXQgdGhlIG1vbWVudCBgYXNzZXJ0X291dGNvbWVgIGNyZWF0ZXMgdGhlCmFzc2VydGlvbiwgYW5kIGV2ZXJ5IHBheW91dCBwYXRoIChgZGlzcHV0ZWAsIGBmaW5hbGl6ZWAsIGByZXNvbHZlYCkKcmVhZHMgYGFzc2VydGlvbi5ib25kYCwgbmV2ZXIgdGhlIGxpdmUgYERhdGFLZXk6OkJvbmRBbW91bnRgLiBBbgphbHJlYWR5LW9wZW4gYXNzZXJ0aW9uJ3MgcGF5b3V0IGlzIHRoZXJlZm9yZSB1bmFmZmVjdGVkIGJ5IGEgbGF0ZXIKYHNldF9ib25kX2Ftb3VudGAgY2FsbC4KCkZhaWxzIHdpdGggYE5vdEluaXRpYWxpemVkYCBpZiBjYWxsZWQgYmVmb3JlIGBpbml0aWFsaXplYCwgb3IKYEludmFsaWRCb25kQW1vdW50YCBpZiBgbmV3X2JvbmRfYW1vdW50YCBpcyB6ZXJvLCBuZWdhdGl2ZSwgb3IgZ3JlYXRlcgp0aGFuIGBNQVhfQk9ORF9BTU9VTlRgLgAAAAAPc2V0X2JvbmRfYW1vdW50AAAAAAEAAAAAAAAAD25ld19ib25kX2Ftb3VudAAAAAALAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAQAAANZBbiBpbi1mbGlnaHQgc2luZ2xlLXNsb3QgY29tbWl0dGVlIHJvdGF0aW9uIHByb3Bvc2VkIGJ5IGEgY3VycmVudCByZXNvbHZlci4KRGVjaWRlZCBieSBhIHN0cmljdCBtYWpvcml0eSBvZiB0aGUgbGl2ZSBjb21taXR0ZWUgdmlhIGB2b3RlX3JvdGF0aW9uYC4gT25seQpvbmUgbWF5IGJlIG9wZW4gYXQgYSB0aW1lLiBTZWUgYGRvY3Mvc3JjL1JPVEFUSU9OX0RFU0lHTi5tZGAuAAAAAAAAAAAAEFJvdGF0aW9uUHJvcG9zYWwAAAAFAAAAPlRoZSBuZXcgcmVzb2x2ZXIgdG8gYWRkLiBNdXN0IG5vdCBhbHJlYWR5IGJlIG9uIHRoZSBjb21taXR0ZWUuAAAAAAAMbmV3X3Jlc29sdmVyAAAAEwAAAEVSZXNvbHZlcnMgd2hvIHZvdGVkIG5vLCB0byBwcmV2ZW50IGRvdWJsZS12b3RpbmcgYW5kIGRldGVjdCBkZWFkbG9jay4AAAAAAAACbm8AAAAAA+oAAAATAAAAR1RoZSBjdXJyZW50IHJlc29sdmVyIHRvIHJlbW92ZS4gTXVzdCBiZSBvbiB0aGUgY29tbWl0dGVlIHdoZW4gcHJvcG9zZWQuAAAAAAxvbGRfcmVzb2x2ZXIAAAATAAAAJVRoZSByZXNvbHZlciB3aG8gb3BlbmVkIHRoZSBwcm9wb3NhbC4AAAAAAAALcHJvcG9zZWRfYnkAAAAAEwAAADJSZXNvbHZlcnMgd2hvIHZvdGVkIHllcywgdG8gcHJldmVudCBkb3VibGUtdm90aW5nLgAAAAAAA3llcwAAAAPqAAAAEw==",
        "AAAAAAAAAlNQcm9wb3NlcyBhIHNpbmdsZS1zbG90IGNvbW1pdHRlZSByb3RhdGlvbjogcmVtb3ZlIGBvbGRfcmVzb2x2ZXJgIChtdXN0IGJlCmEgY3VycmVudCByZXNvbHZlcikgYW5kIGFkZCBgbmV3X3Jlc29sdmVyYCAobXVzdCBub3QgYWxyZWFkeSBiZSBvbmUpLiBPbmx5CmEgY3VycmVudCByZXNvbHZlciBtYXkgcHJvcG9zZSwgYW5kIG9ubHkgb25lIHJvdGF0aW9uIG1heSBiZSBvcGVuIGF0IGEKdGltZS4gVGhlIHByb3Bvc2FsIGlzIGRlY2lkZWQgYnkgYSBzdHJpY3QgbWFqb3JpdHkgb2YgdGhlIGxpdmUgY29tbWl0dGVlCih0aGUgc2FtZSB0aHJlc2hvbGQgdXNlZCB0byByZXNvbHZlIGRpc3B1dGVzKSB2aWEgYHZvdGVfcm90YXRpb25gLiBUaGUKY29tbWl0dGVlIHdyaXR0ZW4gb24gZXhlY3V0aW9uIGlzIHRoZSBzYW1lIGBSZXNvbHZlcnNgIHNsb3QgYHVwZGF0ZV9yZXNvbHZlcnNgCndyaXRlcywgc28gYSByb3RhdGlvbiBoYXMgbm8gZWZmZWN0IG9uIGRpc3B1dGVzIGFscmVhZHkgb3BlbiAodGhlaXIKY29tbWl0dGVlIHdhcyBzbmFwc2hvdHRlZCBhdCBgZGlzcHV0ZWAgdGltZSkuIFBhdXNlLWV4ZW1wdCwgbGlrZQpgdXBkYXRlX3Jlc29sdmVyc2AuAAAAABBwcm9wb3NlX3JvdGF0aW9uAAAAAwAAAAAAAAAIcmVzb2x2ZXIAAAATAAAAAAAAAAxvbGRfcmVzb2x2ZXIAAAATAAAAAAAAAAxuZXdfcmVzb2x2ZXIAAAATAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAlZSZXBsYWNlcyB0aGUgcmVzb2x2ZXIgY29tbWl0dGVlLiBPbmx5IGNhbGxhYmxlIGJ5IHRoZSBhZG1pbiBzZXQgYXQKaW5pdGlhbGl6YXRpb24uIGBuZXdfcmVzb2x2ZXJzYCBtdXN0IGhhdmUgYW4gb2RkIGxlbmd0aCBzbyBhIHNpbXBsZQptYWpvcml0eSB2b3RlIGNhbiBuZXZlciB0aWUuIENhbGxhYmxlIGV2ZW4gd2hpbGUgcGF1c2VkLCBzbyBhCmNvbXByb21pc2VkIGNvbW1pdHRlZSBjYW4gYmUgcmVwbGFjZWQgd2l0aG91dCB3YWl0aW5nIHRvIHVucGF1c2UuCgpUaGlzIGlzIHRoZSBlbWVyZ2VuY3kgb3ZlcnJpZGUgcGF0aC4gSXQgc3VwZXJzZWRlcyBhbnkgaW4tZmxpZ2h0CnNlbGYtcm90YXRpb24gdm90ZTogYW4gb3BlbiBgUm90YXRpb25Qcm9wb3NhbGAgaXMgY2xlYXJlZCAoZW1pdHRpbmcKYFJvdGF0aW9uQ2FuY2VsbGVkYCB3aGVuIG9uZSB3YXMgcHJlc2VudCksIHNvIGEgcHJvcG9zYWwgY2FuIG5ldmVyCmV4ZWN1dGUgYWdhaW5zdCBhIGNvbW1pdHRlZSBpdCB3YXNuJ3QgYnVpbHQgZm9yLiBEYXktdG8tZGF5IGNvbW1pdHRlZQpjaGFuZ2VzIGdvIHRocm91Z2ggYHByb3Bvc2Vfcm90YXRpb25gIC8gYHZvdGVfcm90YXRpb25gIGluc3RlYWQuAAAAAAAQdXBkYXRlX3Jlc29sdmVycwAAAAEAAAAAAAAADW5ld19yZXNvbHZlcnMAAAAAAAPqAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAABQAAAAAAAAAAAAAAEFJlc29sdmVyc1VwZGF0ZWQAAAABAAAAEXJlc29sdmVyc191cGRhdGVkAAAAAAAAAQAAAAAAAAAJcmVzb2x2ZXJzAAAAAAAD6gAAABMAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEFJvdGF0aW9uRXhlY3V0ZWQAAAABAAAAEXJvdGF0aW9uX2V4ZWN1dGVkAAAAAAAAAgAAAAAAAAAMb2xkX3Jlc29sdmVyAAAAEwAAAAAAAAAAAAAADG5ld19yZXNvbHZlcgAAABMAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEFJvdGF0aW9uUHJvcG9zZWQAAAABAAAAEXJvdGF0aW9uX3Byb3Bvc2VkAAAAAAAAAwAAAAAAAAAMb2xkX3Jlc29sdmVyAAAAEwAAAAAAAAAAAAAADG5ld19yZXNvbHZlcgAAABMAAAAAAAAAAAAAAAtwcm9wb3NlZF9ieQAAAAATAAAAAAAAAAI=",
        "AAAABQAAAAAAAAAAAAAAEUJvbmRBbW91bnRVcGRhdGVkAAAAAAAAAQAAABNib25kX2Ftb3VudF91cGRhdGVkAAAAAAEAAAAAAAAAC2JvbmRfYW1vdW50AAAAAAsAAAAAAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAEVJvdGF0aW9uQ2FuY2VsbGVkAAAAAAAAAQAAABJyb3RhdGlvbl9jYW5jZWxsZWQAAAAAAAIAAAAAAAAADG9sZF9yZXNvbHZlcgAAABMAAAAAAAAAAAAAAAxuZXdfcmVzb2x2ZXIAAAATAAAAAAAAAAI=",
        "AAAAAAAAAAAAAAATZ2V0X2Fzc2VydGlvbl9zdGF0ZQAAAAABAAAAAAAAAAJpZAAAAAAABgAAAAEAAAPpAAAH0AAAAAlBc3NlcnRpb24AAAAAAAAD" ]),
      options
    )
  }
  public readonly fromJSON = {
    dispute: this.txFromJSON<Result<void>>,
        resolve: this.txFromJSON<Result<Option<boolean>>>,
        finalize: this.txFromJSON<Result<boolean>>,
        initialize: this.txFromJSON<Result<void>>,
        set_paused: this.txFromJSON<Result<void>>,
        vote_rotation: this.txFromJSON<Result<Option<boolean>>>,
        assert_outcome: this.txFromJSON<Result<u64>>,
        cancel_rotation: this.txFromJSON<Result<void>>,
        set_bond_amount: this.txFromJSON<Result<void>>,
        propose_rotation: this.txFromJSON<Result<void>>,
        update_resolvers: this.txFromJSON<Result<void>>,
        get_assertion_state: this.txFromJSON<Result<Assertion>>
  }
}