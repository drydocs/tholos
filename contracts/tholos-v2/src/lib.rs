#![no_std]

//! Tholos v2: stake-weighted resolution. Design in `docs/src/V2_RESOLUTION.md`.
//! Deployed as a wholly separate contract from v1 (`contracts/tholos`), never
//! upgraded in place: see the design doc's "Migration from existing v1
//! deployments" section for why.
//!
//! This crate currently implements #64 (the immutable `PolicySnapshotV2`
//! pinned at assertion creation, and the `AssertionV2` record it lives on),
//! #65 (bonded assertion posting, and the uncontested-finalize path), #66
//! (`dispute` and the third-party registration phase), #67 (the reveal
//! phase and commitment verification), #68 (weighted-majority outcome
//! resolution: the strict-majority lock and optimistic-timeout default),
//! #69 (settlement: converting a locked outcome into per-position
//! entitlements, forfeiture, and pro-rata reward distribution), #70
//! (credit withdrawal and the reentrancy guard on every token-moving
//! entrypoint), and #71 (`set_paused_v2` and `cancel_round`, the emergency
//! mechanism for an already-active round, distinct from v1's narrower
//! new-assertions-only pause). Remaining issues (#72 onward) land as this
//! crate grows.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, xdr::ToXdr, Address,
    BytesN, Env, Symbol,
};

/// Which weight rule this assertion's vote is decided under. A version marker
/// rather than a formula, so a future weight rule can be added without
/// reinterpreting already-pinned assertions under new math.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeightRuleVersion {
    /// `weight(address) = locked_bond(address)`, linear and unweighted by
    /// anything but the amount actually escrowed. See "Voting weight" in
    /// V2_RESOLUTION.md for why linear weight is deliberate.
    LinearStakeV1,
}

/// What happens if neither side reaches strict majority by the reveal
/// deadline. `AssertedOutcomeStands` is the only variant today (the
/// optimistic-default design decision), kept as an enum so a future
/// `Inconclusive`-style rule could be pinned per-deployment without touching
/// already-open assertions under the old rule.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutDefaultRule {
    AssertedOutcomeStands,
}

/// Version marker for the settlement/payout formula (forfeiture distribution,
/// dust handling). Bumped independently of `WeightRuleVersion` since the two
/// can change on different schedules.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayoutRuleVersion {
    ProRataV1,
}

/// The lifecycle phase of an `AssertionV2`. `OutcomeLocked` (a strict majority
/// reached before the reveal deadline) is folded into `Reveal`: an assertion
/// stays `Reveal` after locking so other positions can keep revealing to
/// prove entitlement for settlement (see `lock_outcome_if_undecided`), only
/// reaching `Resolved` once `revealed_weight` catches up with the frozen
/// eligible total or `reveal_deadline` passes.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseV2 {
    Pending,
    Registration,
    Reveal,
    Resolved,
}

/// Why an assertion reached `Resolved`. Distinguishes a real vote outcome
/// from the optimistic default and from an admin cancellation, so history and
/// indexers never have to infer which rule actually decided the result.
///
/// `NotYetDecided` stands in for `None`: the soroban-sdk 26.1.0 `contracttype`
/// derive doesn't generate an XDR `ScVal` conversion for `Option<EnumType>`
/// (only `Option` of built-in types like `Address`/`bool`), so `AssertionV2`
/// deriving `contracttype` fails to compile if `terminal_cause` is
/// `Option<TerminalCause>`. Same information, no SDK limitation.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCause {
    NotYetDecided,
    /// Never disputed within `challenge_window_secs`; `finalize` closed it
    /// out the same way v1's uncontested `finalize` does. The only terminal
    /// cause that doesn't go through registration/reveal at all.
    UncontestedFinalize,
    StrictMajorityFor,
    StrictMajorityAgainst,
    OptimisticTimeout,
    /// Set only by `cancel_round`, while `terminal_cause` is still
    /// `NotYetDecided`: `Pending`, or `Registration`/`Reveal` with no
    /// strict majority reached yet. Every funded position (including any
    /// that already revealed) recovers its exact principal through the
    /// same `settle`/`withdraw` path a normal outcome uses, no forfeiture
    /// and no reward: see `settlement_pool`.
    AdminCancelled,
}

#[contractevent]
pub struct Asserted {
    #[topic]
    pub id: u64,
    pub asserter: Address,
    pub outcome: bool,
}

#[contractevent]
pub struct Finalized {
    #[topic]
    pub id: u64,
    pub outcome: bool,
    /// Always a verified address: `finalize` requires the caller's auth
    /// unconditionally, the same hardening v1 applies, so this can't be
    /// spoofed regardless of whether a reward was configured.
    pub finalizer: Address,
    pub reward: i128,
}

#[contractevent]
pub struct Disputed {
    #[topic]
    pub id: u64,
    pub disputer: Address,
    pub registration_deadline: u64,
}

#[contractevent]
pub struct PositionFunded {
    #[topic]
    pub id: u64,
    pub voter: Address,
    /// The position's new total after this deposit (top-ups aggregate, so
    /// this isn't just the amount of this one call).
    pub amount: i128,
    /// The running eligible total `W` after this deposit.
    pub eligible_total: i128,
}

#[contractevent]
pub struct RevealOpened {
    #[topic]
    pub id: u64,
    pub reveal_deadline: u64,
}

#[contractevent]
pub struct Revealed {
    #[topic]
    pub id: u64,
    pub voter: Address,
    pub choice: bool,
}

#[contractevent]
pub struct Resolved {
    #[topic]
    pub id: u64,
    pub terminal_cause: TerminalCause,
    pub final_outcome: bool,
}

#[contractevent]
pub struct Settled {
    #[topic]
    pub id: u64,
    pub address: Address,
    /// Principal plus pro-rata reward, or 0 for a forfeited position. Does
    /// not include any leftover dust this settlement happened to route
    /// (see `settle`); dust is credited but not reflected in this payout
    /// figure, since it's incidental to which position happened to close
    /// the recipient side out, not part of that position's own formula. See
    /// `DustCredited`, emitted separately when this happens.
    pub payout: i128,
}

/// Emitted alongside `Settled`, at most once per assertion, when the
/// settlement that closes out the last recipient position also routes
/// leftover floor-division dust to the deterministic recipient (see
/// `settle`). Kept separate from `Settled` rather than folded into its
/// `payout` field: an indexer reconstructing withdrawable balances purely
/// from the event log needs this to add up to the same total `settle`
/// actually credits on-chain, which `Settled.payout` alone doesn't capture.
#[contractevent]
pub struct DustCredited {
    #[topic]
    pub id: u64,
    pub address: Address,
    pub amount: i128,
}

#[contractevent]
pub struct Withdrawn {
    #[topic]
    pub id: u64,
    /// Whose credit balance this was: `withdraw` requires this address's
    /// own authorization, so this can't be spoofed.
    pub owner: Address,
    /// Where the tokens actually went. Can differ from `owner`: `withdraw`
    /// lets the owner name any destination, so a token that rejects
    /// transfers to `owner` directly can't strand funds there permanently.
    pub destination: Address,
    pub amount: i128,
}

#[contractevent]
pub struct PauseUpdated {
    pub paused: bool,
}

#[contractevent]
pub struct RoundCancelled {
    #[topic]
    pub id: u64,
}

/// What kind of position this is, and (for a `Fixed` one) which side it's
/// on. The asserter's and disputer's sides are public by construction, no
/// commitment needed; a third party's side stays hidden in its commitment
/// until reveal (#67).
///
/// Tuple variants, not struct-like named-field variants: soroban-sdk
/// 26.1.0's `contracttype` derive doesn't support named fields on enum
/// variants, only unit variants or a single unnamed field, the same general
/// category of derive-macro limitation as the `Option<EnumType>` gap noted
/// on `TerminalCause` above.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PositionKind {
    /// `true` if this position agrees with the asserted outcome.
    Fixed(bool),
    /// The salted commitment hash to this position's eventual side.
    External(BytesN<32>),
}

/// One address's stake on one dispute. Non-transferable, keyed by
/// `(assertion_id, address)`. Once funded, only exits through settlement;
/// registration and reveal only ever grow `amount` via top-ups, never
/// shrink it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    pub amount: i128,
    pub kind: PositionKind,
    /// Whether this position's weight has been counted into
    /// `Resolution.agree_weight`/`disagree_weight`. For a `Fixed` position
    /// this is set `true` automatically when reveal opens (their side is
    /// already public); for an `External` position it's set by a successful
    /// `reveal` call. Either way, `reveal` rejects a position that's already
    /// `true` with `AlreadyRevealed`, so a `Fixed` voter calling `reveal`
    /// themselves (they have no commitment to verify) is rejected the same
    /// way a double-reveal is, no separate error variant needed.
    pub revealed: bool,
    /// Whether this position agrees with the assertion's originally
    /// asserted outcome. `None` until `revealed` is `true`: a `Fixed`
    /// position's side is already implied by `PositionKind::Fixed`, but is
    /// only copied in here (by `open_reveal_phase`) once reveal actually
    /// opens, so `settle` has one uniform field to read regardless of
    /// `PositionKind`, rather than needing to branch on it. `Option<bool>`
    /// is fine here, unlike an `Option` of a custom enum: see
    /// `TerminalCause`'s doc comment.
    pub agrees_with_outcome: Option<bool>,
    /// Whether `settle` has already run for this position. `settle` rejects
    /// a position that's already `true` with `AlreadySettled`: a position's
    /// payout is computed once and is final, since it depends on
    /// `Resolution.settled_recipient_weight`, which every settlement (this
    /// one included) advances.
    pub settled: bool,
}

/// Registration-phase bookkeeping for one disputed assertion. Separate from
/// `AssertionV2` per V2_RESOLUTION.md's storage layout ("Replacing
/// Assertion.resolvers"): `AssertionV2` is claim/parties/policy, `Resolution`
/// is the mutable per-dispute state that grows as reveal (#67) and
/// settlement (#69) land.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolution {
    pub registration_opened_at: u64,
    /// The soft cutoff: pushed out by `anti_snipe_extension_secs` on a
    /// qualifying late deposit, capped at `registration_hard_deadline`.
    pub registration_deadline: u64,
    /// Fixed at `dispute` time: `registration_opened_at +
    /// anti_snipe_hard_max_secs`. No sequence of extensions can push
    /// `registration_deadline` past this.
    pub registration_hard_deadline: u64,
    /// The frozen-at-reveal-cutoff eligible total `W`, maintained
    /// incrementally as deposits arrive; never discovered by scanning
    /// storage.
    pub eligible_total: i128,
    /// 0 until the lazy Registration -> Reveal transition happens (see
    /// `open_reveal_phase`), then the ledger timestamp of that transition.
    pub reveal_opened_at: u64,
    /// 0 until reveal opens, then `reveal_opened_at + reveal_duration_secs`.
    pub reveal_deadline: u64,
    /// Weight revealed agreeing with the asserted outcome, including the
    /// asserter's fixed position (counted automatically when reveal opens).
    pub agree_weight: i128,
    /// Weight revealed against the asserted outcome, including the
    /// disputer's fixed position (counted automatically when reveal opens).
    pub disagree_weight: i128,
    /// Cumulative weight of recipient (reward-eligible) positions already
    /// settled. `settle` compares this against the winning side's total
    /// recipient weight to detect the last settlement, the one that also
    /// receives any leftover dust from floor division. See `settle`.
    pub settled_recipient_weight: i128,
    /// Cumulative reward (principal excluded) already distributed to
    /// settled recipient positions. Needed to compute the exact leftover
    /// dust on the last settlement: `forfeited_pool - settled_reward_total`
    /// at that point, rather than re-deriving it from individual payouts
    /// nothing here retains a record of. See `settle`.
    pub settled_reward_total: i128,
    /// Total credit currently accrued (via `settle`) but not yet withdrawn
    /// (via `withdraw`) for this dispute. Increases whenever `settle`
    /// accrues a payout or dust, decreases whenever `withdraw` pays one out.
    /// `outstanding_liability + withdrawn_total` never exceeds
    /// `eligible_total`, the invariant `withdraw`'s tests check explicitly.
    pub outstanding_liability: i128,
    /// Cumulative amount actually transferred out via `withdraw` for this
    /// dispute, across every address that has withdrawn anything.
    pub withdrawn_total: i128,
}

impl Resolution {
    /// `agree_weight + disagree_weight`. Not stored separately: it's always
    /// derivable from the two tallies, and a redundant stored copy would
    /// just be one more place for the two to drift out of sync.
    pub fn revealed_weight(&self) -> i128 {
        self.agree_weight + self.disagree_weight
    }
}

/// The exact preimage `reveal` hashes and compares against a position's
/// stored commitment, matching V2_RESOLUTION.md's "Registration and voter
/// eligibility" section:
/// `H(canonical_encode("THOLOS_V2_VOTE", network_id, contract_address,
/// policy_hash, assertion_id, round, voter, choice, salt_32))`.
///
/// A struct hashed via `ToXdr`, not a hand-rolled byte concatenation, the
/// same canonical-encoding approach `PolicySnapshotV2.policy_hash` already
/// uses (see `create_pending_assertion`), so the domain separation is
/// unambiguous by construction rather than by convention.
///
/// `pub`, not crate-private: `tools/compute-commitment` (a separate crate,
/// a path dependency on this one) constructs one directly to compute a
/// real commitment off-chain (for `register`) the same way `reveal`
/// verifies one, rather than duplicating this struct's shape and risking
/// drift between the two. This doesn't change the contract's own on-chain
/// interface at all, `pub` here is a Rust visibility modifier for tooling,
/// not a new callable function or a change to this type's XDR encoding.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteCommitmentPreimage {
    pub domain: Symbol,
    pub network_id: BytesN<32>,
    pub contract_address: Address,
    pub policy_hash: BytesN<32>,
    pub assertion_id: u64,
    pub round: u32,
    pub voter: Address,
    pub choice: bool,
    pub salt: BytesN<32>,
}

/// Pinned in full onto every `AssertionV2` at creation time, never mutated
/// afterward. Deployment-wide parameter changes (via a future `initialize`-
/// adjacent admin call) only affect assertions created after the change; an
/// already-open assertion always executes under the snapshot it was created
/// with. See "Policy is pinned when the assertion opens" in
/// V2_RESOLUTION.md.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySnapshotV2 {
    pub token: Address,
    pub base_bond: i128,
    /// How long a `Pending` assertion can be disputed before it's eligible
    /// for uncontested `finalize`. Distinct from `registration_duration_secs`
    /// below: this window gates *whether* a dispute can start at all, the
    /// other gates how long third parties have to join *after* one has.
    pub challenge_window_secs: u64,
    /// Basis points (0-1000) of the bond paid to whoever calls `finalize` on
    /// an uncontested assertion, the same incentive-for-prompt-finalization
    /// mechanic v1 has, carried over because the problem it solves (nobody
    /// else is motivated to spend gas finalizing on the asserter's behalf)
    /// is identical in both versions for this specific uncontested case.
    pub finalize_reward_bps: u32,
    /// Always equal to `base_bond`: see the rationale on `initialize` below.
    pub min_resolution_bond: i128,
    pub registration_duration_secs: u64,
    /// A qualifying late deposit pushes the soft registration cutoff out by
    /// this much, never past `anti_snipe_hard_max_secs` total extension.
    pub anti_snipe_extension_secs: u64,
    pub anti_snipe_hard_max_secs: u64,
    pub reveal_duration_secs: u64,
    pub weight_rule: WeightRuleVersion,
    pub timeout_default: TimeoutDefaultRule,
    pub payout_rule: PayoutRuleVersion,
    /// Upper bound on any single position's size, checked at deposit time so
    /// settlement's forfeiture-distribution arithmetic (#69) can't overflow.
    pub max_position: i128,
    /// Upper bound on the frozen eligible total `W`, for the same reason.
    pub max_total_weight: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssertionV2 {
    pub id: u64,
    pub asserter: Address,
    /// Ledger timestamp `assert_outcome` posted this assertion. Used to check
    /// `challenge_window_secs` elapsed before an uncontested `finalize`.
    pub opened_at: u64,
    /// Set once `dispute` (#66) opens registration. `None` while `Pending`.
    pub disputer: Option<Address>,
    pub outcome: bool,
    pub phase: PhaseV2,
    pub policy: PolicySnapshotV2,
    /// Hash of `policy`'s canonical encoding, computed once at creation and
    /// stored alongside it. Lets a client or auditor confirm which exact
    /// policy an assertion is bound to without re-deriving the encoding
    /// themselves.
    pub policy_hash: BytesN<32>,
    /// `TerminalCause::NotYetDecided` until the outcome is decided, which
    /// can happen before `phase` reaches `Resolved`: a strict majority
    /// locks this (see `lock_outcome_if_undecided`) as soon as it's
    /// reached, while `phase` deliberately stays `Reveal` so other
    /// positions can keep revealing to prove entitlement for settlement.
    /// Once set, never changes. Read this field, not `phase`, to learn
    /// whether the outcome itself is decided.
    pub terminal_cause: TerminalCause,
    /// The authoritative resolved outcome. `None` until `terminal_cause` is
    /// decided (see above; not necessarily gated on `phase == Resolved`).
    /// Stored directly (not only in an event), unlike v1's `Assertion.outcome`
    /// which always stays the original claim even after a dispute overturns
    /// it, a sharp edge v1 needed a separate `final_outcome` field to fix.
    pub final_outcome: Option<bool>,
    /// Who called `finalize`. `None` until finalized. `Address` is a built-in
    /// type, so `Option<Address>` is fine here, unlike `Option<TerminalCause>`
    /// above.
    pub finalizer: Option<Address>,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Policy,
    NextId,
    AssertionV2(u64),
    Resolution(u64),
    Position(u64, Address),
    /// Token units `settle` has credited an address for one assertion, not
    /// yet withdrawn. A bare `i128` rather than a wrapper struct: it's the
    /// only value this key ever holds. `withdraw` is what actually moves
    /// tokens against this balance.
    Credit(u64, Address),
    /// A single contract-wide mutex, held (`true`) for the duration of any
    /// external token transfer this contract initiates. See
    /// `enter_reentrancy_guard`.
    ReentrancyGuard,
    /// Whether new assertions are currently blocked. See `set_paused_v2`.
    /// Unlike v1's broader pause, this only ever gates `assert_outcome`:
    /// an already-active round's registration, reveal, settlement, and
    /// withdrawal continue normally even while paused, since blocking them
    /// would strand real capital already locked into that round rather
    /// than protect it. `cancel_round` is the mechanism for protecting an
    /// already-active round.
    Paused,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    AssertionNotFound = 3,
    /// `base_bond` was not positive, or exceeded the interim safety bound
    /// below.
    InvalidBondAmount = 4,
    InvalidRegistrationDuration = 5,
    InvalidRevealDuration = 6,
    /// `anti_snipe_extension_secs` exceeded `anti_snipe_hard_max_secs`, which
    /// would let a single qualifying deposit blow past the deployment's own
    /// stated hard cap in one step.
    InvalidAntiSnipeParams = 7,
    InvalidMaxPosition = 8,
    InvalidMaxTotalWeight = 9,
    InvalidChallengeWindow = 10,
    /// `finalize_reward_bps` was greater than `MAX_FINALIZE_REWARD_BPS`.
    InvalidFinalizeReward = 11,
    /// Action requires `PhaseV2::Pending` but the assertion isn't.
    NotPending = 12,
    /// `finalize` called before `challenge_window_secs` has elapsed since
    /// `opened_at`.
    ChallengeWindowOpen = 13,
    /// The disputer address matched the assertion's own asserter. Can't stop
    /// the same owner using a second address, but prevents one storage
    /// position from occupying both protocol roles.
    DisputerIsAsserter = 14,
    /// Action requires `PhaseV2::Registration` but the assertion isn't.
    NotRegistration = 15,
    /// `register` called by the asserter or disputer; they top up their
    /// fixed position by other means (not yet implemented; tracked
    /// separately from this issue's third-party registration path).
    CannotRegisterAsFixedParty = 16,
    InvalidPositionAmount = 17,
    /// A new position's amount was below `policy.min_resolution_bond`.
    BelowMinimumResolutionBond = 18,
    /// A position's total (after aggregating this deposit) exceeded
    /// `policy.max_position`.
    PositionExceedsMax = 19,
    /// The eligible total `W` (after this deposit) would exceed
    /// `policy.max_total_weight`.
    EligibleTotalExceedsMax = 20,
    /// A top-up's commitment didn't match the one this position was created
    /// with. A position's committed side can never change after funding.
    CommitmentMismatch = 21,
    /// `register` called after `registration_deadline` has passed.
    RegistrationClosed = 22,
    /// `reveal` called while still `Registration` and `registration_deadline`
    /// hasn't passed yet.
    RegistrationNotClosed = 23,
    /// `reveal` called on an assertion that's `Pending` or `Resolved`; it
    /// must be `Registration` (past its deadline) or already `Reveal`.
    NotReveal = 24,
    /// `reveal` called after `reveal_deadline` has passed.
    RevealClosed = 25,
    /// This position has already been counted into the tally, either by a
    /// prior `reveal` call, or automatically at the point reveal opened (for
    /// the asserter's/disputer's fixed positions).
    AlreadyRevealed = 26,
    /// The supplied `(choice, salt)` didn't hash to the commitment this
    /// position was funded with.
    CommitmentVerificationFailed = 27,
    /// `resolve_outcome` called while still `Reveal`, before
    /// `reveal_deadline` has passed, and before all eligible weight has
    /// revealed.
    RevealNotClosed = 28,
    /// `settle` called before the assertion has reached `PhaseV2::Resolved`.
    NotResolved = 29,
    /// `settle` called on a position that's already settled.
    AlreadySettled = 30,
    /// A checked arithmetic operation in `settle` would have overflowed
    /// `i128`. Not expected to be reachable given `initialize`'s
    /// `max_total_weight`/`max_position` bounds, but settlement moves real
    /// funds, so it's checked rather than assumed.
    SettlementArithmeticOverflow = 31,
    /// `withdraw` called with no credit balance to withdraw (either never
    /// settled anything here, or already withdrew it).
    NoCreditToWithdraw = 32,
    /// A call that moves tokens was attempted while another one was still
    /// in progress: the token's own `transfer` reentered this contract
    /// instead of completing normally. See `enter_reentrancy_guard`.
    ReentrancyGuardActive = 33,
    /// `assert_outcome` called while `set_paused_v2` has paused new
    /// assertions.
    Paused = 34,
    /// `cancel_round` called while not paused.
    NotPaused = 35,
    /// `cancel_round` called on an assertion whose `terminal_cause` is
    /// already set, whether by a real outcome or an earlier cancellation.
    /// Rejected outright rather than treated as a no-op, so this call can
    /// never be read as altering an already-decided result.
    RoundAlreadyDecided = 36,
}

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

/// On top of a round's full registration-through-reveal horizon, extra
/// padding so `settle`/`withdraw` still have a comfortable window to run
/// after reveal closes, before any of that round's persistent entries risk
/// archival. See #72.
const SETTLEMENT_GRACE_SECS: u64 = 7 * 24 * 60 * 60;

/// Soroban's target ledger close time. `extend_ttl` operates in ledgers,
/// but every duration elsewhere in this contract is tracked in seconds via
/// `env.ledger().timestamp()`, so this only translates a
/// seconds-denominated policy value into an approximate ledger count for
/// TTL sizing. A conservative approximation is fine here: TTL is always
/// computed relative to "now" at write time, not pinned to an absolute
/// deadline, so a small margin of error just means slightly more or less
/// rent-covered headroom, never an incorrect result.
const SECS_PER_LEDGER: u64 = 5;

const MAX_REGISTRATION_DURATION_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_REVEAL_DURATION_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_ANTI_SNIPE_HARD_MAX_SECS: u64 = 29 * 24 * 60 * 60; // 29 days
/// Same 7-day cap as v1's `challenge_window_secs`, for the same reason: it
/// must leave real margin within the 30-day persistent-storage TTL bump
/// (`INSTANCE_BUMP_AMOUNT`) for `finalize` to actually get called before the
/// assertion's ledger entry risks archival.
const MAX_CHALLENGE_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;
/// 1000 bps (10%) caps the incentive without letting a deployment
/// accidentally haircut the asserter's bond by more than a tenth. Same
/// reasoning as v1's `MAX_FINALIZE_REWARD_BPS`, independently applicable
/// here since the uncontested-finalize case works identically in both
/// versions.
const MAX_FINALIZE_REWARD_BPS: u32 = 1_000;

/// Bound on `base_bond` so `finalize`'s reward-multiply
/// (`bond * finalize_reward_bps`, computed before the divide by 10,000) can't
/// overflow `i128`, for any `finalize_reward_bps` up to
/// `MAX_FINALIZE_REWARD_BPS`. At `assert_outcome` time only one bond has
/// entered escrow (no disputer yet), so this reward-multiply constraint is
/// the only one that applies at this stage.
const MAX_BOND_AMOUNT: i128 = i128::MAX / (MAX_FINALIZE_REWARD_BPS as i128);

/// Bound on `max_total_weight` (and, transitively, `max_position`, which
/// `initialize` already requires to be no larger) so `settle`'s
/// forfeiture-distribution multiply (`position.amount * forfeited_pool`,
/// computed before the divide by `recipient_weight`) can't overflow `i128`.
/// Both operands are themselves bounded by `max_total_weight`
/// (`position.amount <= max_position <= max_total_weight`, and
/// `forfeited_pool <= eligible_total <= max_total_weight`), so bounding
/// their product requires `max_total_weight^2 <= i128::MAX`, i.e.
/// `max_total_weight <= sqrt(i128::MAX)` (~1.3 * 10^19). This is far
/// tighter than `MAX_BOND_AMOUNT` above (~1.7 * 10^35), which only had to
/// keep a single multiply by `MAX_FINALIZE_REWARD_BPS` in range: bounding
/// `max_total_weight` by `MAX_BOND_AMOUNT` alone, as `initialize` did
/// before this issue, would let `settle`'s multiply overflow for any
/// realistically large, heavily-contested dispute. 10^19 leaves comfortable
/// headroom under the true `sqrt(i128::MAX)` limit while still supporting
/// any deployment token's full realistic supply range.
const MAX_SETTLEMENT_TOTAL_WEIGHT: i128 = 10_000_000_000_000_000_000;

/// This proposal has exactly one weighted round: no recursive appeals or
/// repeated stake rounds, per V2_RESOLUTION.md's "Lifecycle and the single
/// weighted round". `round` is part of the commitment preimage now so a
/// future multi-round tier wouldn't have to change the commitment format,
/// but it's always 0 until such a tier exists.
const ROUND: u32 = 0;

#[contract]
pub struct TholosV2;

#[contractimpl]
impl TholosV2 {
    /// One-time setup, pinning the deployment-wide defaults every future
    /// assertion's `PolicySnapshotV2` is built from. Requires `admin`'s
    /// signature. Fails with `AlreadyInitialized` if called twice.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        base_bond: i128,
        challenge_window_secs: u64,
        finalize_reward_bps: u32,
        registration_duration_secs: u64,
        anti_snipe_extension_secs: u64,
        anti_snipe_hard_max_secs: u64,
        reveal_duration_secs: u64,
        max_position: i128,
        max_total_weight: i128,
    ) -> Result<(), Error> {
        admin.require_auth();

        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        if base_bond <= 0 || base_bond > MAX_BOND_AMOUNT {
            return Err(Error::InvalidBondAmount);
        }
        if challenge_window_secs == 0 || challenge_window_secs > MAX_CHALLENGE_WINDOW_SECS {
            return Err(Error::InvalidChallengeWindow);
        }
        if finalize_reward_bps > MAX_FINALIZE_REWARD_BPS {
            return Err(Error::InvalidFinalizeReward);
        }
        if registration_duration_secs == 0
            || registration_duration_secs > MAX_REGISTRATION_DURATION_SECS
        {
            return Err(Error::InvalidRegistrationDuration);
        }
        if reveal_duration_secs == 0 || reveal_duration_secs > MAX_REVEAL_DURATION_SECS {
            return Err(Error::InvalidRevealDuration);
        }
        if anti_snipe_extension_secs > anti_snipe_hard_max_secs {
            return Err(Error::InvalidAntiSnipeParams);
        }
        if anti_snipe_hard_max_secs > MAX_ANTI_SNIPE_HARD_MAX_SECS {
            return Err(Error::InvalidAntiSnipeParams);
        }
        // registration_hard_deadline is registration_opened_at +
        // anti_snipe_hard_max_secs (see dispute() below), an absolute
        // duration from registration opening, independent of
        // registration_duration_secs. If hard_max were shorter than the
        // base registration window itself, the hard deadline would fall
        // before the ordinary soft deadline even with zero extensions ever
        // granted, which is nonsensical.
        if anti_snipe_hard_max_secs < registration_duration_secs {
            return Err(Error::InvalidAntiSnipeParams);
        }
        // max_total_weight's own bounds must be checked before comparing
        // max_position against it, otherwise a max_total_weight <= 0 always
        // trips the max_position check first (no positive max_position can
        // ever be <= a non-positive total), making InvalidMaxTotalWeight's
        // <= 0 case unreachable.
        if max_total_weight <= 0 || max_total_weight > MAX_SETTLEMENT_TOTAL_WEIGHT {
            return Err(Error::InvalidMaxTotalWeight);
        }
        // A position can't usefully exceed the frozen total it's part of.
        if max_position <= 0 || max_position > max_total_weight {
            return Err(Error::InvalidMaxPosition);
        }

        let policy = PolicySnapshotV2 {
            token,
            base_bond,
            challenge_window_secs,
            finalize_reward_bps,
            // Equal to base_bond by design: a cheaper minimum would let a
            // third party break an asserter/disputer tie for a fraction of
            // what the original two parties risked. See #64/#66.
            min_resolution_bond: base_bond,
            registration_duration_secs,
            anti_snipe_extension_secs,
            anti_snipe_hard_max_secs,
            reveal_duration_secs,
            weight_rule: WeightRuleVersion::LinearStakeV1,
            timeout_default: TimeoutDefaultRule::AssertedOutcomeStands,
            payout_rule: PayoutRuleVersion::ProRataV1,
            max_position,
            max_total_weight,
        };

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Policy, &policy);
        env.storage().instance().set(&DataKey::NextId, &0u64);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        Ok(())
    }

    /// Read-only lookup of the deployment-wide policy defaults new assertions
    /// are currently pinned from. Fails with `NotInitialized` before
    /// `initialize`.
    pub fn get_policy(env: Env) -> Result<PolicySnapshotV2, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)
    }

    /// Blocks or unblocks new `assert_outcome` calls. Only callable by the
    /// admin set at `initialize`. Does not affect any already-active
    /// round: registration, reveal, `resolve_outcome`, `settle`, and
    /// `withdraw` all continue normally while paused, since blocking them
    /// would strand capital already locked into a round rather than
    /// protect it. `cancel_round` is the mechanism for protecting an
    /// already-active round instead. Emits `PauseUpdated`.
    pub fn set_paused_v2(env: Env, paused: bool) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Paused, &paused);
        PauseUpdated { paused }.publish(&env);

        Ok(())
    }

    /// Read-only lookup of one assertion. Fails with `AssertionNotFound` if
    /// the id doesn't exist.
    pub fn get_assertion(env: Env, id: u64) -> Result<AssertionV2, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)
    }

    /// TTL bump `(threshold, amount)`, in ledgers, sized to cover one
    /// dispute's full worst-case active-phase horizon (registration through
    /// reveal, per this specific assertion's own pinned policy) plus
    /// `SETTLEMENT_GRACE_SECS`, rather than a flat constant: a deployment's
    /// `anti_snipe_hard_max_secs`/`reveal_duration_secs` can be sized well
    /// above what a fixed 30-day bump covers, and every persistent record
    /// tied to a round (`AssertionV2`, `Resolution`, `Position`, `Credit`)
    /// needs to survive that same horizon, not just whatever margin the
    /// instance-storage bump happened to use. Floored at
    /// `INSTANCE_BUMP_AMOUNT` so this is never a *smaller* safety margin
    /// than before, only ever the same or larger. See #72.
    fn record_bump(policy: &PolicySnapshotV2) -> (u32, u32) {
        let horizon_secs = policy
            .anti_snipe_hard_max_secs
            .saturating_add(policy.reveal_duration_secs)
            .saturating_add(SETTLEMENT_GRACE_SECS);
        let horizon_ledgers = (horizon_secs / SECS_PER_LEDGER).min(u32::MAX as u64) as u32;
        let bump = horizon_ledgers.max(INSTANCE_BUMP_AMOUNT);
        (bump - DAY_IN_LEDGERS, bump)
    }

    /// Writes an assertion and extends its persistent storage TTL. Every
    /// write site uses this rather than a bare `.set()`, matching v1's
    /// `set_assertion` so an assertion's ledger entry can't be archived out
    /// from under it while still active.
    fn set_assertion(env: &Env, id: u64, assertion: &AssertionV2) {
        let key = DataKey::AssertionV2(id);
        env.storage().persistent().set(&key, assertion);
        let (threshold, bump) = Self::record_bump(&assertion.policy);
        env.storage().persistent().extend_ttl(&key, threshold, bump);
    }

    /// Read-only lookup of one assertion's registration-phase bookkeeping.
    /// Fails with `AssertionNotFound` if it doesn't exist (a `Resolution`
    /// always exists once `dispute` has been called, and never before).
    pub fn get_resolution(env: Env, id: u64) -> Result<Resolution, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Resolution(id))
            .ok_or(Error::AssertionNotFound)
    }

    fn set_resolution(env: &Env, id: u64, resolution: &Resolution, policy: &PolicySnapshotV2) {
        let key = DataKey::Resolution(id);
        env.storage().persistent().set(&key, resolution);
        let (threshold, bump) = Self::record_bump(policy);
        env.storage().persistent().extend_ttl(&key, threshold, bump);
    }

    /// Read-only lookup of one address's position on one assertion. Fails
    /// with `AssertionNotFound` if that address has no position there.
    pub fn get_position(env: Env, id: u64, address: Address) -> Result<Position, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Position(id, address))
            .ok_or(Error::AssertionNotFound)
    }

    fn set_position(
        env: &Env,
        id: u64,
        address: &Address,
        position: &Position,
        policy: &PolicySnapshotV2,
    ) {
        let key = DataKey::Position(id, address.clone());
        env.storage().persistent().set(&key, position);
        let (threshold, bump) = Self::record_bump(policy);
        env.storage().persistent().extend_ttl(&key, threshold, bump);
    }

    /// Acquires the contract-wide reentrancy mutex, failing with
    /// `ReentrancyGuardActive` if it's already held. Every function that
    /// initiates an external token transfer calls this immediately before
    /// that transfer (after writing whatever state the transfer follows,
    /// matching the existing state-before-external-call ordering) and
    /// `exit_reentrancy_guard` immediately after. A non-standard token
    /// whose `transfer` implementation calls back into this contract mid-
    /// transfer, instead of a well-behaved SEP-41 token that just updates
    /// balances, would otherwise be able to act on state that looks
    /// complete (because it was written before the transfer) while the
    /// tokens backing it haven't actually moved yet.
    ///
    /// `reveal`, `resolve_outcome`, `settle`, and `cancel_round` also check
    /// this at their own entry, even though none of them move tokens
    /// themselves: all four can act on a position's weight, credit, or
    /// terminal state, which the guard above exists specifically to keep
    /// provisional until its funding transfer actually completes.
    fn enter_reentrancy_guard(env: &Env) -> Result<(), Error> {
        Self::check_reentrancy_guard(env)?;
        env.storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &true);
        Ok(())
    }

    fn exit_reentrancy_guard(env: &Env) {
        env.storage()
            .instance()
            .set(&DataKey::ReentrancyGuard, &false);
    }

    /// Fails with `ReentrancyGuardActive` if the mutex `enter_reentrancy_guard`
    /// manages is currently held, without acquiring it. For entrypoints that
    /// don't themselves transfer tokens but still shouldn't run while one of
    /// this contract's own transfers is mid-flight (see
    /// `enter_reentrancy_guard`'s doc comment), and reused by
    /// `enter_reentrancy_guard` itself as the first half of acquiring it.
    fn check_reentrancy_guard(env: &Env) -> Result<(), Error> {
        if env
            .storage()
            .instance()
            .get(&DataKey::ReentrancyGuard)
            .unwrap_or(false)
        {
            return Err(Error::ReentrancyGuardActive);
        }
        Ok(())
    }

    /// Fails with `Paused` if `set_paused_v2` has paused new assertions.
    /// Only `assert_outcome` calls this: see `DataKey::Paused`'s doc
    /// comment for why this contract's pause is narrower than v1's.
    fn require_not_paused(env: &Env) -> Result<(), Error> {
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .ok_or(Error::NotInitialized)?;
        if paused {
            return Err(Error::Paused);
        }
        Ok(())
    }

    /// Builds and stores a `Pending` `AssertionV2` with a freshly pinned
    /// policy snapshot. Doesn't move any tokens itself; `assert_outcome`
    /// calls this first (matching v1's state-before-external-call ordering)
    /// and transfers the bond after.
    fn create_pending_assertion(env: &Env, asserter: Address, outcome: bool) -> Result<u64, Error> {
        let policy: PolicySnapshotV2 = env
            .storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)?;

        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextId)
            .ok_or(Error::NotInitialized)?;
        env.storage().instance().set(&DataKey::NextId, &(id + 1));

        let policy_hash = env.crypto().sha256(&policy.clone().to_xdr(env)).into();

        let assertion = AssertionV2 {
            id,
            asserter,
            opened_at: env.ledger().timestamp(),
            disputer: None,
            outcome,
            phase: PhaseV2::Pending,
            policy,
            policy_hash,
            terminal_cause: TerminalCause::NotYetDecided,
            final_outcome: None,
            finalizer: None,
        };

        Self::set_assertion(env, id, &assertion);

        Ok(id)
    }

    /// Posts a bonded claim, the optimistic first stage of a v2 assertion,
    /// before any dispute exists. Transfers the deployment's `base_bond` from
    /// `asserter` to the contract. Requires `asserter`'s signature. Fails
    /// with `Paused` if `set_paused_v2` has paused new assertions. Returns
    /// the new assertion id. Emits `Asserted`.
    pub fn assert_outcome(env: Env, asserter: Address, outcome: bool) -> Result<u64, Error> {
        asserter.require_auth();
        Self::enter_reentrancy_guard(&env)?;
        Self::require_not_paused(&env)?;

        let policy: PolicySnapshotV2 = env
            .storage()
            .instance()
            .get(&DataKey::Policy)
            .ok_or(Error::NotInitialized)?;

        // State is written (inside create_pending_assertion) before the
        // external token transfer below, matching v1's assert_outcome: a
        // reentrant call during the transfer can't be allocated the same
        // not-yet-incremented id.
        let id = Self::create_pending_assertion(&env, asserter.clone(), outcome)?;

        token::Client::new(&env, &policy.token).transfer(
            &asserter,
            env.current_contract_address(),
            &policy.base_bond,
        );
        Self::exit_reentrancy_guard(&env);

        Asserted {
            id,
            asserter,
            outcome,
        }
        .publish(&env);

        Ok(id)
    }

    /// Callable once a `Pending` assertion's `challenge_window_secs` has
    /// elapsed with no dispute. `caller` must authorize the call
    /// unconditionally, even when `finalize_reward_bps` is 0, the same
    /// hardening v1 applies, so `AssertionV2.finalizer` and the `Finalized`
    /// event can never be spoofed regardless of whether a reward is paid.
    ///
    /// When `finalize_reward_bps` is non-zero, `caller` receives
    /// `bond * finalize_reward_bps / 10_000` and the asserter receives the
    /// remainder; when zero, the full bond returns to the asserter.
    ///
    /// Returns the asserted outcome. Fails with `NotPending` if the
    /// assertion isn't `Pending`, `ChallengeWindowOpen` if called too early.
    /// Emits `Finalized`.
    pub fn finalize(env: Env, caller: Address, id: u64) -> Result<bool, Error> {
        caller.require_auth();

        let mut assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.phase != PhaseV2::Pending {
            return Err(Error::NotPending);
        }

        if env.ledger().timestamp() <= assertion.opened_at + assertion.policy.challenge_window_secs
        {
            return Err(Error::ChallengeWindowOpen);
        }

        let reward_bps = assertion.policy.finalize_reward_bps;
        let reward = if reward_bps > 0 {
            assertion.policy.base_bond * (reward_bps as i128) / 10_000
        } else {
            0
        };

        // State is written before the external token transfers below so a
        // reentrant call from a non-standard token sees this assertion as
        // already resolved, rather than still Pending. Mirrors v1's finalize.
        assertion.phase = PhaseV2::Resolved;
        assertion.terminal_cause = TerminalCause::UncontestedFinalize;
        assertion.final_outcome = Some(assertion.outcome);
        assertion.finalizer = Some(caller.clone());
        Self::set_assertion(&env, id, &assertion);
        Self::enter_reentrancy_guard(&env)?;
        let token_client = token::Client::new(&env, &assertion.policy.token);
        if reward > 0 {
            token_client.transfer(&env.current_contract_address(), &caller, &reward);
        }
        let asserter_payout = assertion.policy.base_bond - reward;
        token_client.transfer(
            &env.current_contract_address(),
            &assertion.asserter,
            &asserter_payout,
        );
        Self::exit_reentrancy_guard(&env);

        Finalized {
            id,
            outcome: assertion.outcome,
            finalizer: caller,
            reward,
        }
        .publish(&env);

        Ok(assertion.outcome)
    }

    /// Disputes a `Pending` assertion, opening the registration phase.
    /// Transfers `base_bond` from `disputer` into escrow, matching it
    /// against the asserter's existing bond. Requires `disputer`'s
    /// signature. Fails with `NotPending` if the assertion isn't `Pending`,
    /// `DisputerIsAsserter` if `disputer` is the assertion's own asserter.
    /// Emits `Disputed`.
    pub fn dispute(env: Env, disputer: Address, id: u64) -> Result<(), Error> {
        disputer.require_auth();
        Self::enter_reentrancy_guard(&env)?;

        let mut assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.phase != PhaseV2::Pending {
            return Err(Error::NotPending);
        }
        if disputer == assertion.asserter {
            return Err(Error::DisputerIsAsserter);
        }

        let policy = assertion.policy.clone();
        let now = env.ledger().timestamp();

        // State is written before the external token transfer below, the
        // same ordering assert_outcome and finalize use.
        assertion.disputer = Some(disputer.clone());
        assertion.phase = PhaseV2::Registration;
        Self::set_assertion(&env, id, &assertion);

        Self::set_position(
            &env,
            id,
            &assertion.asserter,
            &Position {
                amount: policy.base_bond,
                kind: PositionKind::Fixed(true),
                revealed: false,
                agrees_with_outcome: None,
                settled: false,
            },
            &policy,
        );
        Self::set_position(
            &env,
            id,
            &disputer,
            &Position {
                amount: policy.base_bond,
                kind: PositionKind::Fixed(false),
                revealed: false,
                agrees_with_outcome: None,
                settled: false,
            },
            &policy,
        );

        let resolution = Resolution {
            registration_opened_at: now,
            registration_deadline: now + policy.registration_duration_secs,
            registration_hard_deadline: now + policy.anti_snipe_hard_max_secs,
            eligible_total: policy.base_bond * 2,
            reveal_opened_at: 0,
            reveal_deadline: 0,
            agree_weight: 0,
            disagree_weight: 0,
            settled_recipient_weight: 0,
            settled_reward_total: 0,
            outstanding_liability: 0,
            withdrawn_total: 0,
        };
        Self::set_resolution(&env, id, &resolution, &policy);

        token::Client::new(&env, &policy.token).transfer(
            &disputer,
            env.current_contract_address(),
            &policy.base_bond,
        );
        Self::exit_reentrancy_guard(&env);

        Disputed {
            id,
            disputer,
            registration_deadline: resolution.registration_deadline,
        }
        .publish(&env);

        Ok(())
    }

    /// Funds (or tops up) a third-party position on a `Registration`-phase
    /// assertion, committing to a side without revealing it (see `reveal`,
    /// #67). Not callable by the assertion's own asserter or disputer, who
    /// already have fixed positions from `dispute`; a way for them to top up
    /// those positions is tracked separately from this issue.
    ///
    /// A first-time deposit must be at least `policy.min_resolution_bond`.
    /// A top-up (same voter, same assertion) aggregates into the existing
    /// position and must reuse its original `commitment`, a position's
    /// committed side can never change after funding. Rejects atomically,
    /// with no position or weight created, if the resulting position size or
    /// eligible total would exceed `policy.max_position` /
    /// `policy.max_total_weight`.
    ///
    /// A qualifying deposit (one landing within `anti_snipe_extension_secs`
    /// of the current deadline) pushes the registration deadline out by
    /// `anti_snipe_extension_secs`, capped at `registration_hard_deadline`.
    ///
    /// Fails with `NotRegistration` if the assertion isn't in the
    /// registration phase, `RegistrationClosed` if the deadline has passed.
    /// Emits `PositionFunded`.
    pub fn register(
        env: Env,
        voter: Address,
        id: u64,
        amount: i128,
        commitment: BytesN<32>,
    ) -> Result<(), Error> {
        voter.require_auth();
        Self::enter_reentrancy_guard(&env)?;
        let assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.phase != PhaseV2::Registration {
            return Err(Error::NotRegistration);
        }
        // Invariant: disputer is always Some once phase reaches
        // Registration, dispute() sets both together and nothing else
        // transitions into this phase.
        let disputer = assertion
            .disputer
            .clone()
            .expect("disputer set once phase reaches Registration");
        if voter == assertion.asserter || voter == disputer {
            return Err(Error::CannotRegisterAsFixedParty);
        }
        if amount <= 0 {
            return Err(Error::InvalidPositionAmount);
        }

        let mut resolution: Resolution = env
            .storage()
            .persistent()
            .get(&DataKey::Resolution(id))
            .ok_or(Error::AssertionNotFound)?;

        let now = env.ledger().timestamp();
        if now > resolution.registration_deadline {
            return Err(Error::RegistrationClosed);
        }
        // Anti-sniping: a deposit landing within the last extension-window
        // of the current deadline pushes it out, capped at the hard
        // deadline fixed at dispute() time.
        if now
            >= resolution
                .registration_deadline
                .saturating_sub(assertion.policy.anti_snipe_extension_secs)
        {
            let extended = now + assertion.policy.anti_snipe_extension_secs;
            resolution.registration_deadline = extended.min(resolution.registration_hard_deadline);
        }

        let position_key = DataKey::Position(id, voter.clone());
        let existing: Option<Position> = env.storage().persistent().get(&position_key);

        let previous_amount = match &existing {
            Some(position) => {
                if let PositionKind::External(stored_commitment) = &position.kind {
                    if *stored_commitment != commitment {
                        return Err(Error::CommitmentMismatch);
                    }
                }
                position.amount
            }
            None => {
                if amount < assertion.policy.min_resolution_bond {
                    return Err(Error::BelowMinimumResolutionBond);
                }
                0
            }
        };

        let new_amount = previous_amount + amount;
        if new_amount > assertion.policy.max_position {
            return Err(Error::PositionExceedsMax);
        }
        let new_total = resolution.eligible_total - previous_amount + new_amount;
        if new_total > assertion.policy.max_total_weight {
            return Err(Error::EligibleTotalExceedsMax);
        }

        // State is written before the external token transfer below, the
        // same ordering every other value-moving call in this contract uses.
        Self::set_position(
            &env,
            id,
            &voter,
            &Position {
                amount: new_amount,
                kind: PositionKind::External(commitment),
                // Always false here: register() only succeeds during
                // Registration, before reveal has even opened.
                revealed: false,
                agrees_with_outcome: None,
                settled: false,
            },
            &assertion.policy,
        );
        resolution.eligible_total = new_total;
        Self::set_resolution(&env, id, &resolution, &assertion.policy);

        token::Client::new(&env, &assertion.policy.token).transfer(
            &voter,
            env.current_contract_address(),
            &amount,
        );
        Self::exit_reentrancy_guard(&env);

        PositionFunded {
            id,
            voter,
            amount: new_amount,
            eligible_total: new_total,
        }
        .publish(&env);

        Ok(())
    }

    /// Lazily transitions a `Registration`-phase assertion to `Reveal` once
    /// `registration_deadline` has passed. Called from `reveal` and
    /// `resolve_outcome` rather than as a separate entrypoint, matching
    /// V2_RESOLUTION.md's "a caller may advance an expired phase
    /// permissionlessly" principle: the first call after the deadline both
    /// closes registration and (depending on the caller) reveals or
    /// resolves in one transaction.
    ///
    /// Counts the asserter's and disputer's fixed positions into the tally
    /// and marks them `revealed`, exactly once, here, since their sides are
    /// already public and they never call `reveal` themselves. `register`
    /// already refuses new deposits once phase leaves `Registration`, so `W`
    /// (`resolution.eligible_total`) is implicitly frozen by this
    /// transition without any separate "freeze" step.
    ///
    /// Immediately delegates to `close_reveal_if_ready`: if there were no
    /// third-party registrations, the asserter's and disputer's fixed
    /// positions alone already account for all of `W`, so the assertion can
    /// resolve in this same call rather than waiting on a reveal deadline
    /// nothing will ever arrive before.
    fn open_reveal_phase(
        env: &Env,
        id: u64,
        mut assertion: AssertionV2,
        mut resolution: Resolution,
    ) -> (AssertionV2, Resolution) {
        let now = env.ledger().timestamp();

        assertion.phase = PhaseV2::Reveal;

        resolution.reveal_opened_at = now;
        resolution.reveal_deadline = now + assertion.policy.reveal_duration_secs;

        let disputer = assertion
            .disputer
            .clone()
            .expect("disputer set once phase reaches Registration");

        for fixed_voter in [&assertion.asserter, &disputer] {
            let mut position: Position = env
                .storage()
                .persistent()
                .get(&DataKey::Position(id, fixed_voter.clone()))
                .expect("asserter and disputer positions created together in dispute()");
            let PositionKind::Fixed(agrees_with_asserter) = position.kind else {
                unreachable!("asserter/disputer positions are always Fixed, set in dispute()")
            };
            if agrees_with_asserter {
                resolution.agree_weight += position.amount;
            } else {
                resolution.disagree_weight += position.amount;
            }
            position.revealed = true;
            position.agrees_with_outcome = Some(agrees_with_asserter);
            Self::set_position(env, id, fixed_voter, &position, &assertion.policy);
        }

        Self::set_resolution(env, id, &resolution, &assertion.policy);

        RevealOpened {
            id,
            reveal_deadline: resolution.reveal_deadline,
        }
        .publish(env);

        assertion = Self::close_reveal_if_ready(env, id, assertion, &resolution, false);

        (assertion, resolution)
    }

    /// Locks `assertion.terminal_cause`/`final_outcome` the moment either
    /// side's revealed weight exceeds half of the frozen eligible total `W`
    /// (`resolution.eligible_total`), matching V2_RESOLUTION.md's "Reveal ->
    /// OutcomeLocked: either side exceeds 50% of eligible weight". Compares
    /// via subtraction (`side_weight > W - side_weight`) rather than
    /// `side_weight > W / 2`: integer division rounds down, which would
    /// wrongly pass a side sitting exactly at the boundary on an odd `W`.
    /// A no-op once `terminal_cause` is already locked: it never changes
    /// after that.
    fn lock_outcome_if_undecided(assertion: &mut AssertionV2, resolution: &Resolution) {
        if assertion.terminal_cause != TerminalCause::NotYetDecided {
            return;
        }

        let w = resolution.eligible_total;
        if resolution.agree_weight > w - resolution.agree_weight {
            assertion.terminal_cause = TerminalCause::StrictMajorityFor;
            assertion.final_outcome = Some(assertion.outcome);
        } else if resolution.disagree_weight > w - resolution.disagree_weight {
            assertion.terminal_cause = TerminalCause::StrictMajorityAgainst;
            assertion.final_outcome = Some(!assertion.outcome);
        }
    }

    /// Closes a `Reveal`-phase assertion out to `Resolved` once its closing
    /// condition is met: `force_close` (the caller has already confirmed
    /// `reveal_deadline` passed) or `revealed_weight` has caught up with the
    /// frozen eligible total `W` (nothing left that could still change the
    /// result). Always attempts `lock_outcome_if_undecided` first; if
    /// neither side ever reached strict majority by the time it closes,
    /// applies `TimeoutDefaultRule::AssertedOutcomeStands`
    /// (`terminal_cause = OptimisticTimeout`, `final_outcome = outcome`) as
    /// the fallback. Always persists `assertion` (an early lock, or the
    /// phase change a caller made before calling this, must survive even on
    /// a call that doesn't close: see `PhaseV2::Reveal`'s doc comment on why
    /// an early lock doesn't by itself stop further reveals); only emits
    /// `Resolved` when it actually closes.
    fn close_reveal_if_ready(
        env: &Env,
        id: u64,
        mut assertion: AssertionV2,
        resolution: &Resolution,
        force_close: bool,
    ) -> AssertionV2 {
        Self::lock_outcome_if_undecided(&mut assertion, resolution);

        let ready = force_close || resolution.revealed_weight() >= resolution.eligible_total;
        if !ready {
            Self::set_assertion(env, id, &assertion);
            return assertion;
        }

        if assertion.terminal_cause == TerminalCause::NotYetDecided {
            assertion.terminal_cause = TerminalCause::OptimisticTimeout;
            assertion.final_outcome = Some(assertion.outcome);
        }
        assertion.phase = PhaseV2::Resolved;
        Self::set_assertion(env, id, &assertion);

        Resolved {
            id,
            terminal_cause: assertion.terminal_cause,
            final_outcome: assertion
                .final_outcome
                .expect("terminal_cause locked above always sets final_outcome alongside it"),
        }
        .publish(env);

        assertion
    }

    /// Discloses the side an `External` position committed to during
    /// registration, and verifies it against the commitment stored then.
    /// Requires `voter`'s signature.
    ///
    /// Lazily transitions the assertion from `Registration` to `Reveal` if
    /// called after `registration_deadline` has passed (see
    /// `open_reveal_phase`); fails with `RegistrationNotClosed` if called
    /// too early instead. Fails with `NotReveal` if the assertion is
    /// `Pending` or `Resolved`, `RevealClosed` if `reveal_deadline` has
    /// passed, `AssertionNotFound` if `voter` has no position here,
    /// `AlreadyRevealed` if this position's weight is already counted (a
    /// double-reveal, or a `Fixed` voter who never had anything to reveal),
    /// `CommitmentVerificationFailed` if `(choice, salt)` doesn't hash to
    /// the stored commitment.
    ///
    /// On success, adds this position's full weight to
    /// `Resolution.agree_weight` if `choice` matches the asserted outcome,
    /// `disagree_weight` otherwise, and locks `terminal_cause`/
    /// `final_outcome` if that tips either side past strict majority (see
    /// `lock_outcome_if_undecided`); the assertion stays `Reveal` even after
    /// locking, so further reveals can still prove entitlement for
    /// settlement, unless this was the last outstanding weight, in which
    /// case the assertion closes to `Resolved` in this same call (see
    /// `close_reveal_if_ready`). A client must read the on-chain phase
    /// before submitting a reveal: guessing that registration has closed is
    /// unsafe, a rejected reveal transaction still publishes its
    /// `(choice, salt)` preimage on-chain even though it failed, and a
    /// qualifying late deposit may have extended the deadline. Emits
    /// `Revealed`, and `Resolved` if it closes the assertion out.
    pub fn reveal(
        env: Env,
        voter: Address,
        id: u64,
        choice: bool,
        salt: BytesN<32>,
    ) -> Result<(), Error> {
        voter.require_auth();
        Self::check_reentrancy_guard(&env)?;

        let mut assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        // Checked before fetching Resolution: an uncontested assertion that
        // reached Resolved via finalize() never had a Resolution created at
        // all (only dispute() creates one), so fetching it first would
        // surface a misleading AssertionNotFound instead of NotReveal for
        // that case.
        if matches!(assertion.phase, PhaseV2::Pending | PhaseV2::Resolved) {
            return Err(Error::NotReveal);
        }

        let mut resolution: Resolution = env
            .storage()
            .persistent()
            .get(&DataKey::Resolution(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.phase == PhaseV2::Registration {
            if env.ledger().timestamp() <= resolution.registration_deadline {
                return Err(Error::RegistrationNotClosed);
            }
            (assertion, resolution) = Self::open_reveal_phase(&env, id, assertion, resolution);
        }

        if env.ledger().timestamp() > resolution.reveal_deadline {
            return Err(Error::RevealClosed);
        }

        let mut position: Position = env
            .storage()
            .persistent()
            .get(&DataKey::Position(id, voter.clone()))
            .ok_or(Error::AssertionNotFound)?;
        if position.revealed {
            return Err(Error::AlreadyRevealed);
        }
        // Fixed positions are marked revealed in open_reveal_phase and never
        // reach here unrevealed, so this is always External in practice;
        // matched explicitly rather than assumed.
        let PositionKind::External(commitment) = &position.kind else {
            return Err(Error::AlreadyRevealed);
        };

        let preimage = VoteCommitmentPreimage {
            domain: Symbol::new(&env, "THOLOS_V2_VOTE"),
            network_id: env.ledger().network_id(),
            contract_address: env.current_contract_address(),
            policy_hash: assertion.policy_hash.clone(),
            assertion_id: id,
            round: ROUND,
            voter: voter.clone(),
            choice,
            salt,
        };
        let computed_commitment: BytesN<32> = env.crypto().sha256(&preimage.to_xdr(&env)).into();
        if computed_commitment != *commitment {
            return Err(Error::CommitmentVerificationFailed);
        }

        let agrees = choice == assertion.outcome;
        position.revealed = true;
        position.agrees_with_outcome = Some(agrees);
        Self::set_position(&env, id, &voter, &position, &assertion.policy);

        if agrees {
            resolution.agree_weight += position.amount;
        } else {
            resolution.disagree_weight += position.amount;
        }
        Self::set_resolution(&env, id, &resolution, &assertion.policy);

        // Not force_close: this reveal happened before reveal_deadline (the
        // check above already ruled out the alternative), so closing here
        // only happens if this reveal was the last weight outstanding.
        Self::close_reveal_if_ready(&env, id, assertion, &resolution, false);

        Revealed { id, voter, choice }.publish(&env);

        Ok(())
    }

    /// Permissionlessly closes a disputed assertion out to `Resolved` once
    /// its outcome can no longer change: called directly (not as a side
    /// effect of `register`/`reveal`) for the cases neither of those calls
    /// can reach on their own, most importantly when `reveal_deadline`
    /// passes without every eligible weight revealing, and the degenerate
    /// case where a dispute drew no third-party registrations at all, so
    /// nobody ever has a position to call `reveal` with. Requires no
    /// signature: it only applies a deterministic rule to already-committed
    /// weights and elapsed time, moving no funds.
    ///
    /// Lazily transitions `Registration` to `Reveal` first (see
    /// `open_reveal_phase`) if `registration_deadline` has passed; that step
    /// alone may already close the assertion out (see its doc comment).
    /// Otherwise requires `Reveal` phase; if `reveal_deadline` has passed or
    /// `revealed_weight` has caught up with the frozen eligible total `W`,
    /// locks the outcome (strict majority if reached, `OptimisticTimeout`
    /// otherwise) and moves the assertion to `Resolved`. Idempotent: calling
    /// it again on an already-`Resolved` assertion just returns the
    /// already-decided `terminal_cause`.
    ///
    /// Fails with `NotReveal` if the assertion is `Pending` (nothing to
    /// resolve yet, it hasn't been disputed), `RegistrationNotClosed` if
    /// still `Registration` before its deadline, `RevealNotClosed` if still
    /// `Reveal` before its deadline with unrevealed weight remaining. Emits
    /// `RevealOpened` and/or `Resolved` as those transitions actually
    /// happen.
    pub fn resolve_outcome(env: Env, id: u64) -> Result<TerminalCause, Error> {
        Self::check_reentrancy_guard(&env)?;

        let mut assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        if matches!(assertion.phase, PhaseV2::Pending | PhaseV2::Resolved) {
            if assertion.phase == PhaseV2::Resolved {
                return Ok(assertion.terminal_cause);
            }
            return Err(Error::NotReveal);
        }

        let mut resolution: Resolution = env
            .storage()
            .persistent()
            .get(&DataKey::Resolution(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.phase == PhaseV2::Registration {
            if env.ledger().timestamp() <= resolution.registration_deadline {
                return Err(Error::RegistrationNotClosed);
            }
            (assertion, resolution) = Self::open_reveal_phase(&env, id, assertion, resolution);
        }

        if assertion.phase == PhaseV2::Resolved {
            return Ok(assertion.terminal_cause);
        }

        let deadline_passed = env.ledger().timestamp() > resolution.reveal_deadline;
        if !deadline_passed && resolution.revealed_weight() < resolution.eligible_total {
            return Err(Error::RevealNotClosed);
        }

        assertion = Self::close_reveal_if_ready(&env, id, assertion, &resolution, true);

        Ok(assertion.terminal_cause)
    }

    /// The winning side's total recipient weight, and the total weight
    /// forfeited to it, for a `Resolved` assertion's `terminal_cause`.
    /// Purely a function of already-frozen `resolution` fields (nothing
    /// changes in `Resolution` once `phase == Resolved`), so every `settle`
    /// call recomputes the identical pair regardless of call order, exactly
    /// what "captured at outcome-lock time" in #69 requires without needing
    /// to separately persist it.
    ///
    /// - `StrictMajorityFor`: the agreeing side recovers principal; the
    ///   disagreeing side and anything never revealed is forfeited to it.
    /// - `StrictMajorityAgainst`: the mirror image, disagreeing side wins.
    /// - `OptimisticTimeout`: every revealed position on either side
    ///   recovers principal; only non-revealed weight is forfeited.
    /// - `AdminCancelled`: every funded position, revealed or not, is a
    ///   recipient (`recipient_weight = eligible_total`), so nothing is
    ///   ever forfeited (`forfeited_pool` is always 0). Only reachable via
    ///   `cancel_round` cancelling a `Registration`/`Reveal`-phase round;
    ///   a cancelled `Pending` assertion never had a `Resolution` created
    ///   at all, so it's refunded directly in `cancel_round` instead and
    ///   never reaches `settle`.
    ///
    /// Panics (via `unreachable!`) for `NotYetDecided` or
    /// `UncontestedFinalize`: `settle` rejects `UncontestedFinalize` itself
    /// before ever calling this (see its own doc comment), and
    /// `NotYetDecided` is impossible once `phase == Resolved`, which
    /// `close_reveal_if_ready` and `cancel_round` (the only two places
    /// that set it, besides `finalize`'s own `UncontestedFinalize` path)
    /// always pair with one of the four causes handled here.
    fn settlement_pool(terminal_cause: TerminalCause, resolution: &Resolution) -> (i128, i128) {
        let recipient_weight = match terminal_cause {
            TerminalCause::StrictMajorityFor => resolution.agree_weight,
            TerminalCause::StrictMajorityAgainst => resolution.disagree_weight,
            TerminalCause::OptimisticTimeout => resolution.revealed_weight(),
            TerminalCause::AdminCancelled => resolution.eligible_total,
            _ => unreachable!(
                "settle only runs once phase == Resolved, which always pairs with one of these four terminal causes"
            ),
        };
        let forfeited_pool = resolution.eligible_total - recipient_weight;
        (recipient_weight, forfeited_pool)
    }

    /// Adds `amount` to `(id, address)`'s withdrawable credit balance,
    /// leaving it untouched if `amount` is 0. Read-modify-write rather than
    /// a bare `set`: a single settlement can touch an address's credit up
    /// to twice (its own payout, and separately, if it happens to be the
    /// deterministic dust recipient described in `settle`), and those two
    /// additions must not clobber each other regardless of which happens
    /// first.
    fn add_credit(
        env: &Env,
        id: u64,
        address: &Address,
        amount: i128,
        policy: &PolicySnapshotV2,
    ) -> Result<(), Error> {
        if amount == 0 {
            return Ok(());
        }
        let key = DataKey::Credit(id, address.clone());
        let existing: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let updated = existing
            .checked_add(amount)
            .ok_or(Error::SettlementArithmeticOverflow)?;
        env.storage().persistent().set(&key, &updated);
        let (threshold, bump) = Self::record_bump(policy);
        env.storage().persistent().extend_ttl(&key, threshold, bump);
        Ok(())
    }

    /// Read-only lookup of one address's withdrawable credit balance on one
    /// assertion, accrued so far by `settle`. Returns 0 for an address with
    /// no credit record rather than failing: unlike `get_position`, "never
    /// settled anything here" isn't a caller error worth surfacing as one.
    pub fn get_credit(env: Env, id: u64, address: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Credit(id, address))
            .unwrap_or(0)
    }

    /// Permissionlessly re-extends TTL on an assertion's `AssertionV2` and,
    /// if it exists yet, its `Resolution`, plus (if `address` is given) that
    /// address's `Position` and `Credit` entries. Every other write path
    /// already bumps TTL as a side effect (see `set_assertion`/
    /// `set_resolution`/`set_position`/`add_credit`), but `Position` and
    /// `Credit` keys aren't enumerable on-chain (the design deliberately
    /// avoids an unbounded voter vector, see V2_RESOLUTION.md), so a record
    /// nobody happens to touch again before settlement (a registered voter
    /// who never reveals, or unclaimed credit sitting well past its
    /// settlement) would otherwise only get renewed if its own owner
    /// eventually calls `reveal`/`settle`/`withdraw`. This lets anyone who
    /// knows the `(id, address)` key, an off-chain indexer, the address's
    /// own owner, or anyone else with an interest in the record staying
    /// live, keep it from approaching archival in the meantime, without
    /// needing to be that owner or to move any funds. A no-op (not an
    /// error) for any of the four entries that doesn't exist for this `id`/
    /// `address`, since which subset exists depends on the assertion's
    /// phase and whether `address` ever registered or settled anything
    /// here. Fails only with `AssertionNotFound` if `id` itself doesn't
    /// exist. See #72.
    pub fn bump_ttl(env: Env, id: u64, address: Option<Address>) -> Result<(), Error> {
        let assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;
        Self::set_assertion(&env, id, &assertion);

        let resolution_key = DataKey::Resolution(id);
        if let Some(resolution) = env
            .storage()
            .persistent()
            .get::<_, Resolution>(&resolution_key)
        {
            Self::set_resolution(&env, id, &resolution, &assertion.policy);
        }

        if let Some(address) = address {
            let position_key = DataKey::Position(id, address.clone());
            if let Some(position) = env.storage().persistent().get::<_, Position>(&position_key) {
                Self::set_position(&env, id, &address, &position, &assertion.policy);
            }

            // Checks the actual balance, not just `has()`: withdraw()
            // deliberately leaves a zeroed credit entry's TTL untouched
            // (see its own comment), a zero balance holds nothing worth
            // paying rent to keep alive, and bump_ttl shouldn't undo that
            // by renewing it anyway just because the key still exists.
            let credit_key = DataKey::Credit(id, address);
            let credit: i128 = env.storage().persistent().get(&credit_key).unwrap_or(0);
            if credit > 0 {
                let (threshold, bump) = Self::record_bump(&assertion.policy);
                env.storage()
                    .persistent()
                    .extend_ttl(&credit_key, threshold, bump);
            }
        }

        Ok(())
    }

    /// Converts one position's share of a decided outcome into withdrawable
    /// credit. Permissionless: any caller may settle any known position,
    /// and settling doesn't move tokens itself (see `get_credit`); `withdraw`
    /// is the separate step that actually transfers tokens against the
    /// accrued balance.
    ///
    /// Requires `phase == Resolved` (`NotResolved` otherwise): settlement
    /// needs final, frozen tallies, which don't exist yet mid-`Reveal` even
    /// after the outcome has locked (see `PhaseV2::Reveal`'s doc comment).
    /// Fails with `AlreadySettled` if `address`'s position here has already
    /// settled.
    ///
    /// A position on the winning side (per `settlement_pool`'s rule for
    /// this assertion's `terminal_cause`) recovers its principal plus a
    /// pro-rata share of the forfeited pool: `reward = floor(amount *
    /// forfeited_pool / recipient_weight)`. A losing or never-revealed
    /// position recovers nothing; its principal is exactly what became
    /// part of the forfeited pool the winners are splitting.
    ///
    /// Every position's `reward` is computed from the same
    /// `(recipient_weight, forfeited_pool)` pair (see `settlement_pool`),
    /// so settling positions in any order, or interleaved with other
    /// assertions' settlements, never changes any individual result.
    /// Floor division leaves at most `recipient_weight - 1` units of
    /// indivisible dust; once this settlement brings
    /// `Resolution.settled_recipient_weight` up to the full
    /// `recipient_weight` (i.e. this was the last recipient position left
    /// to settle), any such dust is credited to a deterministic party: the
    /// winning asserter or disputer after a strict-majority result, or the
    /// asserter after a timeout default (both are always themselves a
    /// recipient position under this assertion's rule, so this never
    /// invents a new participant). Skipped entirely when `forfeited_pool`
    /// is 0.
    ///
    /// Returns this position's own payout (principal plus reward, or 0 if
    /// forfeited; never includes dust routed to a different address in the
    /// same call). Emits `Settled`.
    pub fn settle(env: Env, id: u64, address: Address) -> Result<i128, Error> {
        Self::check_reentrancy_guard(&env)?;

        let assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.phase != PhaseV2::Resolved {
            return Err(Error::NotResolved);
        }

        // An uncontested assertion resolved via finalize() never had a
        // Resolution or any Position created for it at all (only dispute()
        // creates those); the position lookup below would surface a
        // misleading AssertionNotFound for that case, so this is checked
        // first with a clearer, dedicated error instead.
        if assertion.terminal_cause == TerminalCause::UncontestedFinalize {
            return Err(Error::NotResolved);
        }

        let mut resolution: Resolution = env
            .storage()
            .persistent()
            .get(&DataKey::Resolution(id))
            .ok_or(Error::AssertionNotFound)?;

        let mut position: Position = env
            .storage()
            .persistent()
            .get(&DataKey::Position(id, address.clone()))
            .ok_or(Error::AssertionNotFound)?;
        if position.settled {
            return Err(Error::AlreadySettled);
        }

        let (recipient_weight, forfeited_pool) =
            Self::settlement_pool(assertion.terminal_cause, &resolution);

        let is_recipient = match assertion.terminal_cause {
            TerminalCause::StrictMajorityFor => position.agrees_with_outcome == Some(true),
            TerminalCause::StrictMajorityAgainst => position.agrees_with_outcome == Some(false),
            TerminalCause::OptimisticTimeout => position.agrees_with_outcome.is_some(),
            // Every funded position recovers its principal on a
            // cancellation, revealed or not: see settlement_pool.
            TerminalCause::AdminCancelled => true,
            _ => unreachable!("settlement_pool above already panics for any other terminal_cause"),
        };

        let payout = if is_recipient {
            // recipient_weight is always > 0 whenever is_recipient is true:
            // the asserter's and disputer's fixed positions are always
            // auto-revealed the moment Reveal opens, so agree_weight,
            // disagree_weight, and revealed_weight() can never be 0 by the
            // time phase == Resolved. Checked anyway; settlement moves real
            // funds, not something to lean on an argument for.
            let reward = position
                .amount
                .checked_mul(forfeited_pool)
                .and_then(|product| product.checked_div(recipient_weight))
                .ok_or(Error::SettlementArithmeticOverflow)?;
            let position_payout = position
                .amount
                .checked_add(reward)
                .ok_or(Error::SettlementArithmeticOverflow)?;

            resolution.settled_recipient_weight = resolution
                .settled_recipient_weight
                .checked_add(position.amount)
                .ok_or(Error::SettlementArithmeticOverflow)?;
            resolution.settled_reward_total = resolution
                .settled_reward_total
                .checked_add(reward)
                .ok_or(Error::SettlementArithmeticOverflow)?;
            let mut liability_increase = position_payout;

            if forfeited_pool > 0 && resolution.settled_recipient_weight == recipient_weight {
                let dust = forfeited_pool - resolution.settled_reward_total;
                if dust > 0 {
                    let dust_recipient = match assertion.terminal_cause {
                        TerminalCause::StrictMajorityFor => assertion.asserter.clone(),
                        TerminalCause::StrictMajorityAgainst => assertion
                            .disputer
                            .clone()
                            .expect("disputer set once phase reaches Registration"),
                        TerminalCause::OptimisticTimeout => assertion.asserter.clone(),
                        // Unreachable in practice: AdminCancelled's
                        // forfeited_pool is always 0 (see settlement_pool),
                        // so the forfeited_pool > 0 guard above already
                        // skips this whole block for it; any other cause
                        // would already have panicked in settlement_pool.
                        _ => unreachable!(
                            "dust only exists when forfeited_pool > 0, which excludes AdminCancelled, and settlement_pool above already panics for any other terminal_cause"
                        ),
                    };
                    Self::add_credit(&env, id, &dust_recipient, dust, &assertion.policy)?;
                    liability_increase = liability_increase
                        .checked_add(dust)
                        .ok_or(Error::SettlementArithmeticOverflow)?;
                    DustCredited {
                        id,
                        address: dust_recipient,
                        amount: dust,
                    }
                    .publish(&env);
                }
            }

            resolution.outstanding_liability = resolution
                .outstanding_liability
                .checked_add(liability_increase)
                .ok_or(Error::SettlementArithmeticOverflow)?;
            Self::set_resolution(&env, id, &resolution, &assertion.policy);

            position_payout
        } else {
            0
        };

        position.settled = true;
        Self::set_position(&env, id, &address, &position, &assertion.policy);

        Self::add_credit(&env, id, &address, payout, &assertion.policy)?;

        Settled {
            id,
            address,
            payout,
        }
        .publish(&env);

        Ok(payout)
    }

    /// Transfers `owner`'s entire withdrawable credit balance on one
    /// assertion to `destination`. Requires `owner`'s authorization.
    /// `destination` may be any address, not necessarily `owner` itself: a
    /// token that rejects transfers to `owner` directly can't permanently
    /// strand funds there, since the owner can route around it. Fails with
    /// `NoCreditToWithdraw` if the balance is 0 (never settled anything
    /// here, or already withdrew it).
    ///
    /// Effects before interactions: `Credit(id, owner)` is zeroed, and
    /// `Resolution.outstanding_liability`/`withdrawn_total` updated, before
    /// the outgoing transfer below. If the transfer fails, the whole call
    /// fails and every write above is rolled back with it (Soroban reverts
    /// all of an invocation's storage writes when it returns `Err`, the
    /// same guarantee every other value-moving function in this contract
    /// already relies on): the credit is never consumed by a failed
    /// transfer.
    ///
    /// Returns the amount withdrawn. Emits `Withdrawn`.
    pub fn withdraw(
        env: Env,
        owner: Address,
        id: u64,
        destination: Address,
    ) -> Result<i128, Error> {
        owner.require_auth();
        Self::check_reentrancy_guard(&env)?;
        Self::enter_reentrancy_guard(&env)?;
        let assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        // Checked before fetching Resolution: an uncontested assertion
        // resolved via finalize() never had a Resolution created for it at
        // all (only dispute() does), and such an owner also never has any
        // credit (settle() never ran either), so fetching Resolution first
        // would surface a misleading AssertionNotFound instead of the
        // documented NoCreditToWithdraw for that case. Mirrors settle()'s
        // equivalent ordering.
        let credit_key = DataKey::Credit(id, owner.clone());
        let credit: i128 = Self::get_credit(env.clone(), id, owner.clone());
        if credit <= 0 {
            return Err(Error::NoCreditToWithdraw);
        }

        let mut resolution: Resolution = env
            .storage()
            .persistent()
            .get(&DataKey::Resolution(id))
            .ok_or(Error::AssertionNotFound)?;

        // Not extend_ttl'd: a zeroed credit entry holds nothing worth
        // paying rent to keep alive, and get_credit already treats a
        // missing entry the same as a zero one. Mirrors add_credit's
        // early return on a zero amount, for the same reason.
        env.storage().persistent().set(&credit_key, &0i128);

        resolution.outstanding_liability = resolution
            .outstanding_liability
            .checked_sub(credit)
            .ok_or(Error::SettlementArithmeticOverflow)?;
        resolution.withdrawn_total = resolution
            .withdrawn_total
            .checked_add(credit)
            .ok_or(Error::SettlementArithmeticOverflow)?;
        Self::set_resolution(&env, id, &resolution, &assertion.policy);

        token::Client::new(&env, &assertion.policy.token).transfer(
            &env.current_contract_address(),
            &destination,
            &credit,
        );
        Self::exit_reentrancy_guard(&env);

        Withdrawn {
            id,
            owner,
            destination,
            amount: credit,
        }
        .publish(&env);

        Ok(credit)
    }

    /// Cancels an active round before any terminal outcome has locked,
    /// refunding every already-funded position its exact principal, no
    /// forfeiture, no reward, as if the round never happened. Only callable
    /// by the admin set at `initialize`, and only while paused
    /// (`NotPaused` otherwise): cancellation is an emergency measure, not
    /// a routine one, and requiring a pause first means it can never
    /// happen as a surprise to a caller in the middle of an ordinary
    /// transaction.
    ///
    /// Fails outright, rather than treating it as a no-op, with
    /// `RoundAlreadyDecided` if `terminal_cause` is already set, whether by
    /// a real outcome (a strict majority, or the optimistic timeout
    /// default) or an earlier cancellation: this makes it structurally
    /// impossible for this call to alter an already-decided result, the
    /// same "once locked, never changes" guarantee every other terminal
    /// cause has.
    ///
    /// A still-`Pending` assertion has no `Resolution` or `Position`
    /// records at all yet (only `dispute` creates those), so its single
    /// asserter bond is refunded directly here. A `Registration`- or
    /// `Reveal`-phase assertion's positions, including any that already
    /// revealed, instead recover their principal through the normal
    /// `settle`/`withdraw` path afterward: `settlement_pool` treats
    /// `AdminCancelled` as every funded position being a recipient of a
    /// zero forfeited pool, so `settle` naturally pays out `amount + 0`
    /// per position with no special-casing needed there, and the
    /// reentrancy and accounting guarantees stay identical to a normal
    /// settlement.
    ///
    /// Emits `RoundCancelled`, a distinct event from `Resolved` (which a
    /// real outcome emits instead), so indexers can always tell the two
    /// apart.
    pub fn cancel_round(env: Env, id: u64) -> Result<(), Error> {
        Self::check_reentrancy_guard(&env)?;

        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .ok_or(Error::NotInitialized)?;
        if !paused {
            return Err(Error::NotPaused);
        }

        let mut assertion: AssertionV2 = env
            .storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)?;

        if assertion.terminal_cause != TerminalCause::NotYetDecided {
            return Err(Error::RoundAlreadyDecided);
        }

        let still_pending = assertion.phase == PhaseV2::Pending;

        assertion.phase = PhaseV2::Resolved;
        assertion.terminal_cause = TerminalCause::AdminCancelled;
        Self::set_assertion(&env, id, &assertion);

        if still_pending {
            Self::enter_reentrancy_guard(&env)?;
            token::Client::new(&env, &assertion.policy.token).transfer(
                &env.current_contract_address(),
                &assertion.asserter,
                &assertion.policy.base_bond,
            );
            Self::exit_reentrancy_guard(&env);
        }

        RoundCancelled { id }.publish(&env);

        Ok(())
    }
}

mod test;
