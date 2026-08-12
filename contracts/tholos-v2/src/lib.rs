#![no_std]

//! Tholos v2: stake-weighted resolution. Design in `docs/src/V2_RESOLUTION.md`.
//! Deployed as a wholly separate contract from v1 (`contracts/tholos`), never
//! upgraded in place: see the design doc's "Migration from existing v1
//! deployments" section for why.
//!
//! This crate currently implements #64 (the immutable `PolicySnapshotV2`
//! pinned at assertion creation, and the `AssertionV2` record it lives on),
//! #65 (bonded assertion posting, and the uncontested-finalize path), #66
//! (`dispute` and the third-party registration phase), and #67 (the reveal
//! phase and commitment verification). Outcome resolution, settlement, and
//! the freeze/cancel mechanism are separate issues (#68-#71) and land as
//! this crate grows.

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
/// reached before the reveal deadline) is folded into `Reveal` for now:
/// distinguishing "revealing, outcome undecided" from "revealing, outcome
/// already locked" only matters once settlement exists, which is #69's scope.
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
    /// Set only by the freeze/cancel mechanism (#71), before any position
    /// reveals. Reserved here so `AssertionV2.terminal_cause`'s type doesn't
    /// need to change when #71 lands.
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
/// `(assertion_id, address)`. Once funded, only exits through settlement
/// (#69); this issue only ever grows `amount` via top-ups, never shrinks it.
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
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct VoteCommitmentPreimage {
    domain: Symbol,
    network_id: BytesN<32>,
    contract_address: Address,
    policy_hash: BytesN<32>,
    assertion_id: u64,
    round: u32,
    voter: Address,
    choice: bool,
    salt: BytesN<32>,
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
    /// `TerminalCause::NotYetDecided` until `phase == Resolved`.
    pub terminal_cause: TerminalCause,
    /// The authoritative resolved outcome. `None` until `phase == Resolved`.
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
}

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

const MAX_REGISTRATION_DURATION_SECS: u64 = 7 * 24 * 60 * 60;
const MAX_REVEAL_DURATION_SECS: u64 = 7 * 24 * 60 * 60;
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
/// the only one that applies at this stage; #69's settlement arithmetic
/// (`s_i * forfeited_pool / recipient_weight`) has its own, separate overflow
/// characteristics and must independently verify or tighten this bound when
/// that issue lands, the same way `max_total_weight` is already checked
/// against it in `initialize` below.
const MAX_BOND_AMOUNT: i128 = i128::MAX / (MAX_FINALIZE_REWARD_BPS as i128);

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
        if max_total_weight <= 0 || max_total_weight > MAX_BOND_AMOUNT {
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

    /// Read-only lookup of one assertion. Fails with `AssertionNotFound` if
    /// the id doesn't exist.
    pub fn get_assertion(env: Env, id: u64) -> Result<AssertionV2, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::AssertionV2(id))
            .ok_or(Error::AssertionNotFound)
    }

    /// Writes an assertion and extends its persistent storage TTL. Every
    /// write site uses this rather than a bare `.set()`, matching v1's
    /// `set_assertion` so an assertion's ledger entry can't be archived out
    /// from under it while still active.
    fn set_assertion(env: &Env, id: u64, assertion: &AssertionV2) {
        let key = DataKey::AssertionV2(id);
        env.storage().persistent().set(&key, assertion);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
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

    fn set_resolution(env: &Env, id: u64, resolution: &Resolution) {
        let key = DataKey::Resolution(id);
        env.storage().persistent().set(&key, resolution);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    /// Read-only lookup of one address's position on one assertion. Fails
    /// with `AssertionNotFound` if that address has no position there.
    pub fn get_position(env: Env, id: u64, address: Address) -> Result<Position, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Position(id, address))
            .ok_or(Error::AssertionNotFound)
    }

    fn set_position(env: &Env, id: u64, address: &Address, position: &Position) {
        let key = DataKey::Position(id, address.clone());
        env.storage().persistent().set(&key, position);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
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
    /// `asserter` to the contract. Requires `asserter`'s signature. Returns
    /// the new assertion id. Emits `Asserted`.
    pub fn assert_outcome(env: Env, asserter: Address, outcome: bool) -> Result<u64, Error> {
        asserter.require_auth();

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
            },
        );
        Self::set_position(
            &env,
            id,
            &disputer,
            &Position {
                amount: policy.base_bond,
                kind: PositionKind::Fixed(false),
                revealed: false,
            },
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
        };
        Self::set_resolution(&env, id, &resolution);

        token::Client::new(&env, &policy.token).transfer(
            &disputer,
            env.current_contract_address(),
            &policy.base_bond,
        );

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
            },
        );
        resolution.eligible_total = new_total;
        Self::set_resolution(&env, id, &resolution);

        token::Client::new(&env, &assertion.policy.token).transfer(
            &voter,
            env.current_contract_address(),
            &amount,
        );

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
    /// `registration_deadline` has passed. Called from `reveal` itself
    /// rather than as a separate entrypoint, matching V2_RESOLUTION.md's
    /// "a caller may advance an expired phase permissionlessly" principle:
    /// the first `reveal` call after the deadline both closes registration
    /// and reveals in one transaction.
    ///
    /// Counts the asserter's and disputer's fixed positions into the tally
    /// and marks them `revealed`, exactly once, here, since their sides are
    /// already public and they never call `reveal` themselves. `register`
    /// already refuses new deposits once phase leaves `Registration`, so `W`
    /// (`resolution.eligible_total`) is implicitly frozen by this
    /// transition without any separate "freeze" step.
    fn open_reveal_phase(
        env: &Env,
        id: u64,
        mut assertion: AssertionV2,
        mut resolution: Resolution,
    ) -> (AssertionV2, Resolution) {
        let now = env.ledger().timestamp();

        assertion.phase = PhaseV2::Reveal;
        Self::set_assertion(env, id, &assertion);

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
            Self::set_position(env, id, fixed_voter, &position);
        }

        Self::set_resolution(env, id, &resolution);

        RevealOpened {
            id,
            reveal_deadline: resolution.reveal_deadline,
        }
        .publish(env);

        (assertion, resolution)
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
    /// `disagree_weight` otherwise. A client must read the on-chain phase
    /// before submitting a reveal: guessing that registration has closed is
    /// unsafe, a rejected reveal transaction still publishes its
    /// `(choice, salt)` preimage on-chain even though it failed, and a
    /// qualifying late deposit may have extended the deadline. Emits
    /// `Revealed`.
    pub fn reveal(
        env: Env,
        voter: Address,
        id: u64,
        choice: bool,
        salt: BytesN<32>,
    ) -> Result<(), Error> {
        voter.require_auth();

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

        position.revealed = true;
        Self::set_position(&env, id, &voter, &position);

        if choice == assertion.outcome {
            resolution.agree_weight += position.amount;
        } else {
            resolution.disagree_weight += position.amount;
        }
        Self::set_resolution(&env, id, &resolution);

        Revealed { id, voter, choice }.publish(&env);

        Ok(())
    }
}

mod test;
