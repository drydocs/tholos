#![no_std]

//! Tholos v2: stake-weighted resolution. Design in `docs/src/V2_RESOLUTION.md`.
//! Deployed as a wholly separate contract from v1 (`contracts/tholos`), never
//! upgraded in place: see the design doc's "Migration from existing v1
//! deployments" section for why.
//!
//! This crate currently implements #64 (the immutable `PolicySnapshotV2`
//! pinned at assertion creation, and the `AssertionV2` record it lives on)
//! and #65 (bonded assertion posting, and the uncontested-finalize path).
//! Dispute registration, reveal, outcome resolution, settlement, and the
//! freeze/cancel mechanism are separate issues (#66-#71) and land as this
//! crate grows.

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, xdr::ToXdr, Address,
    BytesN, Env,
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
}

mod test;
